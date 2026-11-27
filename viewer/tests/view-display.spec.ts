import { expect, test, type Page } from '@playwright/test';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { backgroundOf, exposureOf, fromView, smoothstepInverse, styleOf, sunOfView, type FileSunAndDisplay } from '../src/view-display';

// The View display panel: a 3D view's Graphic Display Options from the file, applied to the 3D view,
// each value named by where it came from, overridable, and reset to the view's. Technical School
// (RVT_SHEETS_MODEL, Autodesk's public sample) has 3D views in Shaded with edges, Hidden Line, with
// cast shadows and with a sky; the CC0 demo building has no 3D view, so every value is Revit's default.

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SCHOOL = process.env.RVT_SHEETS_MODEL ?? '';
const demo = path.resolve(__dirname, '../public/demos/cc0-2027-demo-building.rvt');
const shots = path.resolve(__dirname, '../view-display-shots');

interface Applied {
  view: string | null;
  values: Record<string, unknown>;
  source: Record<string, string>;
}
interface SceneState {
  materials: Record<string, number>;
  edges: number;
  edgesShown: boolean;
  segments: number;
  hemi: number;
  sun: number;
  shadowIntensity: number;
  toneMapping: number;
  exposure: number;
  fog: boolean;
  background: string | null;
}

const panel = (page: Page) => page.locator('#view-display-panel');
async function applied(page: Page): Promise<Applied> {
  return JSON.parse((await panel(page).getAttribute('data-applied')) ?? '{}') as Applied;
}
async function sceneState(page: Page): Promise<SceneState> {
  return JSON.parse((await panel(page).getAttribute('data-scene')) ?? '{}') as SceneState;
}
async function open(page: Page, file: string): Promise<void> {
  await page.goto('/');
  await page.locator('#file-input').setInputFiles(file);
  await expect(page.locator('#status')).toHaveText(/Loaded/, { timeout: 600_000 });
  await expect(panel(page)).toHaveAttribute('data-state', 'ready', { timeout: 120_000 });
  await page.locator('#view-display-details > summary').click();
}
async function pickView(page: Page, name: string): Promise<void> {
  await page.locator('#vd-view').selectOption({ label: name });
  await expect.poll(async () => (await applied(page)).view).toBe(name);
}

test('a view gives every option it stores; Revit’s defaults fill what was not read', () => {
  const bare = fromView({ id: 1, name: 'Old', is3d: true, isTemplate: false, castShadows: true, ambientShadows: false, displayStyle: 'HLR' });
  expect(bare.values.style).toBe('HLR');
  expect(bare.source.style).toBe('view');
  expect(bare.values.castShadows).toBe(true);
  // nothing of these was read: the defaults, named as such
  for (const k of ['sun', 'ambient', 'shadows', 'sketchy', 'jitter', 'fadeStart', 'background', 'exposure'] as const) expect(bare.source[k]).toBe('default');
  expect(bare.values.sun).toBe(30);
  expect(bare.values.shadows).toBe(50);
  expect(bare.values.exposure).toBe('Automatic');
  expect(fromView(null).source.style).toBe('default');

  const full = fromView({
    id: 2,
    name: 'Full',
    is3d: true,
    isTemplate: false,
    castShadows: false,
    ambientShadows: true,
    displayStyle: 'RealisticWithEdges',
    sunlightIntensity: 80,
    ambientIntensity: 20,
    shadowIntensity: 120,
    sketchyLines: { enabled: true, jitter: 4, extension: 12 },
    depthCueing: { enabled: true, start: 10, end: 90, fadeTo: 30 },
    background: { kind: 'Type258', typeValue: 258, skyColor: '#112233', horizonColor: '#445566', groundColor: '#778899' },
    exposure: { kind: 'Manual', typeValue: 1, value: 10 },
  });
  expect(full.values).toMatchObject({ style: 'RealisticWithEdges', ambientShadows: true, sun: 80, ambient: 20, shadows: 100, sketchy: true, jitter: 4, extension: 10 });
  // depth cueing is read but, as in Revit, not drawn in a 3D view
  expect(full.values).toMatchObject({ depthCueing: false, fadeStart: 10, fadeEnd: 90, fadeLimit: 30 });
  expect(full.notes.depthCueing).toContain('elevations and sections');
  expect(full.values).toMatchObject({ background: 'Gradient', skyColor: '#112233', groundColor: '#778899', exposure: 'Manual', exposureValue: 10 });
  const image = fromView({ id: 3, name: 'I', is3d: true, isTemplate: false, castShadows: false, ambientShadows: false, background: { kind: 'Image', typeValue: 3, imagePath: 'C:/sky.jpg' } });
  expect(image.values.background).toBe('None');
  expect(image.notes.background).toContain('only its path');

  expect(backgroundOf(256)).toBe('None');
  expect(backgroundOf(257)).toBe('Sky');
  expect(styleOf('Rendering')?.style).toBe('Realistic');
  expect(styleOf('Undefined')).toBeNull();
  expect(exposureOf('Automatic', 3)).toBe(1);
  expect(exposureOf('Manual', 14)).toBe(1);
  expect(exposureOf('Manual', 12)).toBe(2);
  const t = smoothstepInverse(0.7);
  expect(t * t * (3 - 2 * t)).toBeCloseTo(0.7, 6);

  const data: FileSunAndDisplay = {
    settings: [
      { id: 10, name: 'Preset', relativeToView: false, start: '2026-06-21T10:00' },
      { id: 11, name: '<In-session, Still>', relativeToView: false, start: '2026-12-21T15:00' },
    ],
    presets: [10],
    views: [],
  };
  expect(sunOfView(data, { id: 5, name: 'V', is3d: true, isTemplate: false, castShadows: true, ambientShadows: false, sunSettingId: 11 })).toMatchObject({ source: 'view', settingId: 11, castShadows: true, viewName: 'V' });
  expect(sunOfView(data, null)).toMatchObject({ source: 'project', settingId: 10 });
});

(fs.existsSync(demo) ? test : test.skip)('a file with no 3D view draws Revit’s defaults and says they were not read', async ({ page }) => {
  await open(page, demo);
  await expect(page.locator('#vd-view')).toBeDisabled();
  await expect(page.locator('#vd-view')).toContainText('No 3D view');
  const a = await applied(page);
  expect(a.view).toBeNull();
  expect(Object.values(a.source).every((s) => s === 'default')).toBe(true);
  await expect(page.locator('#vd-sun-source')).toHaveText(/not read/);
  await expect(page.locator('#vd-scheme-source')).toHaveText(/not read/);
  const s = await sceneState(page);
  expect(s.edgesShown).toBe(true);
  expect(s.fog).toBe(false);
});

(fs.existsSync(demo) ? test : test.skip)('?view-display=off draws as before, with the panel off', async ({ page }) => {
  await page.goto('/?view-display=off');
  await page.locator('#file-input').setInputFiles(demo);
  await expect(page.locator('#status')).toHaveText(/Loaded/, { timeout: 600_000 });
  await expect(panel(page)).toHaveAttribute('data-state', 'off');
});

(SCHOOL && fs.existsSync(SCHOOL) ? test : test.skip)('Technical School: each 3D view’s options are applied, overridden and reset', async ({ page }) => {
  test.setTimeout(900_000);
  fs.mkdirSync(shots, { recursive: true });
  await open(page, SCHOOL);

  // the default 3D view, {3D}: Shaded with edges, Sun 0, Ambient Light 30, Shadows 50, no shadows
  let a = await applied(page);
  expect(a.view).toBe('{3D}');
  expect(a.values).toMatchObject({ style: 'ShadingWithEdges', sun: 0, ambient: 30, shadows: 50, castShadows: false, background: 'None' });
  expect(a.source.style).toBe('view');
  await expect(page.locator('#vd-style-source')).toHaveText('from the view “{3D}”');
  let s = await sceneState(page);
  expect(s.edgesShown).toBe(true);
  expect(s.hemi).toBeCloseTo(2.6, 3);
  expect(s.sun).toBeCloseTo(0.8, 3);
  await page.screenshot({ path: path.join(shots, 'school-3d.png') });

  // From Parking Area casts shadows on a sky: the Daylight panel's shadows come on with it
  await pickView(page, 'From Parking Area');
  a = await applied(page);
  expect(a.values).toMatchObject({ castShadows: true, background: 'Sky', sun: 30, ambient: 0 });
  await expect(page.locator('#daylight-panel')).toHaveAttribute('data-shadows', 'on', { timeout: 60_000 });
  await expect(page.locator('#daylight-when-note')).toContainText('From Parking Area');
  s = await sceneState(page);
  expect(s.background).toBe('gradient');
  expect(s.shadowIntensity).toBeCloseTo(0.5, 3);
  await page.screenshot({ path: path.join(shots, 'school-parking-area.png') });

  // shadows turned off in the Daylight panel are the view's cast shadows turned off, set by the user
  await page.locator('#daylight-details > summary').click();
  await page.locator('#daylight-shadows').uncheck();
  await expect.poll(async () => (await applied(page)).source.castShadows).toBe('user');
  await expect(page.locator('#vd-castShadows')).not.toBeChecked();
  await page.locator('#daylight-details > summary').click();

  // Balcony View is Hidden Line: white unlit faces under the edges
  await pickView(page, 'Balcony View');
  s = await sceneState(page);
  expect(Object.keys(s.materials)).toContain('MeshBasicMaterial');
  expect(s.materials.MeshStandardMaterial ?? 0).toBe(0);
  expect(s.edgesShown).toBe(true);

  // every style, drawn and shot
  for (const style of ['Wireframe', 'HLR', 'Shading', 'ShadingWithEdges', 'FlatColors', 'Textures', 'Realistic', 'RealisticWithEdges']) {
    await page.locator('#vd-style').selectOption(style);
    await expect.poll(async () => (await applied(page)).values.style).toBe(style);
    s = await sceneState(page);
    expect(s.edgesShown).toBe(['Wireframe', 'HLR', 'ShadingWithEdges', 'FlatColors', 'RealisticWithEdges'].includes(style));
    if (style === 'Wireframe') expect(Object.keys(s.materials)).toEqual(['hidden']);
    expect(s.toneMapping !== 0).toBe(style.startsWith('Realistic'));
    await page.screenshot({ path: path.join(shots, `school-style-${style}.png`) });
  }
  await expect(page.locator('#vd-style-source')).toHaveText('set by you');

  // overrides: lighting, exposure, sketchy lines, depth cueing, background, ambient shadows
  await page.locator('#vd-style').selectOption('Realistic');
  await page.locator('#vd-exposure').selectOption('Manual');
  await page.locator('#vd-exposureValue').fill('12');
  await expect.poll(async () => (await sceneState(page)).exposure).toBe(2);
  await page.locator('#vd-sun').fill('90');
  await page.locator('#vd-ambient').fill('50');
  s = await sceneState(page);
  expect(s.hemi).toBeCloseTo(3, 3);
  await expect(page.locator('#vd-sun-source')).toHaveText('set by you');

  await page.locator('#vd-style').selectOption('HLR');
  const plain = (await sceneState(page)).segments;
  await page.locator('#vd-sketchy').check();
  await page.locator('#vd-jitter').fill('6');
  await page.locator('#vd-extension').fill('4');
  await expect.poll(async () => (await sceneState(page)).segments).toBe(plain * 6);
  await page.screenshot({ path: path.join(shots, 'school-sketchy.png') });

  await page.locator('#vd-depthCueing').check();
  await page.locator('#vd-fadeLimit').fill('20');
  await expect.poll(async () => (await sceneState(page)).fog).toBe(true);
  await page.screenshot({ path: path.join(shots, 'school-depth-cueing.png') });

  await page.locator('#vd-background').selectOption('Gradient');
  await expect.poll(async () => (await sceneState(page)).background).toBe('gradient');

  await page.locator('#vd-style').selectOption('ShadingWithEdges');
  await page.locator('#vd-ambientShadows').check();
  // drawn once the camera is still
  await expect.poll(async () => JSON.parse((await panel(page).getAttribute('data-frame')) ?? '{}').ao, { timeout: 30_000 }).toBe(true);
  await page.screenshot({ path: path.join(shots, 'school-ambient-shadows.png') });

  // back to the view's own settings: Balcony View's
  await page.locator('#vd-reset').click();
  a = await applied(page);
  expect(a.values).toMatchObject({ style: 'HLR', sketchy: false, depthCueing: false, ambientShadows: false, background: 'None' });
  expect(Object.values(a.source).includes('user')).toBe(false);
  s = await sceneState(page);
  expect(s.fog).toBe(false);
  expect(s.toneMapping).toBe(0);
  expect(s.segments).toBe(plain);
});

