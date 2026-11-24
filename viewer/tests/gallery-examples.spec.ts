import { expect, test, type Download, type Page } from '@playwright/test';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { openExportMenu } from './export-menu';

// Each example in the demo gallery opens and shows what its card says it
// shows: the IFC classes it draws, and the one feature it is there for.
// The files are staged by scripts/stage-demos.mjs from the pinned public
// corpora and this project's tiny reference models (release assets); a demo
// that was not staged is skipped here, and CI fails the build before this
// runs when one is missing.

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const demos = path.resolve(__dirname, '../public/demos');

interface Example {
  id: string;
  file: string;
  /** IFC classes the Categories panel must list. */
  classes: string[];
  /** The feature the example is there for, checked once the file is open. */
  check?: (page: Page) => Promise<void>;
}

/** Keeps the glb the viewer draws, to read its materials. */
async function keepGlb(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const w = window as unknown as { __glb?: Promise<ArrayBuffer> };
    const create = URL.createObjectURL;
    URL.createObjectURL = function (obj: Blob | MediaSource) {
      if (obj instanceof Blob && obj.type === 'model/gltf-binary') w.__glb = obj.arrayBuffer();
      return create.call(URL, obj);
    };
  });
}

/** The alpha modes of the materials in the glb the viewer drew. */
async function alphaModes(page: Page): Promise<string[]> {
  return page.evaluate(async () => {
    const buf = await (window as unknown as { __glb: Promise<ArrayBuffer> }).__glb;
    const len = new DataView(buf).getUint32(12, true);
    const gltf = JSON.parse(new TextDecoder().decode(new Uint8Array(buf, 20, len)));
    return (gltf.materials ?? []).map((m: { alphaMode?: string }) => m.alphaMode ?? 'OPAQUE');
  });
}

async function selectFirst(page: Page, ifcType: string): Promise<void> {
  const category = page.locator(`.tree-node.tree-category[data-ifc-type="${ifcType}"]`).first();
  await category.click();
  const element = page.locator(`.tree-node.tree-element[data-ifc-type="${ifcType}"]`).first();
  await expect(element).toBeVisible();
  await element.click();
  await expect(page.locator('#info .info-row', { hasText: 'ElementId' }).first()).toBeVisible();
}

/** Asks the selected element's material for its render appearance and waits for it. */
async function showAppearance(page: Page) {
  const material = page.locator('#info .info-group-material');
  await material.getByRole('button', { name: 'Show render appearance' }).click();
  await expect(material.locator('.info-row', { hasText: 'Appearance' })).toBeVisible();
  return material;
}

const examples: Example[] = [
  {
    id: 'door-family-2020',
    file: 'I_Single-Flush.rfa',
    classes: ['IfcDoor', 'IfcWall'],
    // The door family stands in the sample wall its template keeps.
    check: async (page) => {
      await selectFirst(page, 'IFCWALL');
    },
  },
  {
    id: 'rac-basic-2024',
    file: 'rac_basic_sample_family_2024.rfa',
    classes: ['IfcFurniture'],
    // A form's material reads its render appearance asset on request.
    check: async (page) => {
      await selectFirst(page, 'IFCFURNITURE');
      const material = await showAppearance(page);
      await expect(material.locator('.info-row', { hasText: 'Appearance' })).toContainText('Laminate - Ivory, Matte');
    },
  },
  {
    id: 'tiny-stairs-2026',
    file: 'tiny-2026-13-stairs.rvt',
    classes: ['IfcStair', 'IfcStairFlight', 'IfcRailing'],
  },
  {
    id: 'tiny-curtain-wall-2024',
    file: 'tiny-2024-14-curtain-wall.rvt',
    classes: ['IfcCurtainWall', 'IfcPlate', 'IfcMember'],
    // The glazed panels are drawn see-through.
    check: async (page) => {
      expect(await alphaModes(page)).toContain('BLEND');
    },
  },
  {
    id: 'tiny-doors-windows-2024',
    file: 'tiny-2024-16-door-window-flips.rvt',
    classes: ['IfcDoor', 'IfcWindow', 'IfcWall'],
    // A door names the wall that hosts it, and its material shows the swatch Revit saved with its appearance.
    check: async (page) => {
      await selectFirst(page, 'IFCDOOR');
      await expect(page.locator('#info-relations')).toContainText('Hosted by');
      const material = await showAppearance(page);
      await expect(material.locator('.info-row', { hasText: 'Schema' })).toContainText('WallPaintSchema');
      const swatch = material.locator('img.info-swatch');
      await expect(swatch).toBeVisible();
      expect(await swatch.evaluate((img) => (img as HTMLImageElement).naturalWidth)).toBeGreaterThan(0);
    },
  },
  {
    id: 'tiny-columns-beams-2025',
    file: 'tiny-2025-12-columns-beams.rvt',
    classes: ['IfcColumn', 'IfcBeam'],
  },
  {
    id: 'tiny-mep-2024',
    file: 'tiny-2024-21-mep-basics.rvt',
    classes: ['IfcDuctSegment', 'IfcPipeSegment'],
  },
  {
    id: 'tiny-families-2025',
    file: 'tiny-2025-22-families.rvt',
    classes: ['IfcBuildingElementProxy'],
  },
  {
    id: 'tiny-rooms-2023',
    file: 'tiny-2023-4-rooms.rvt',
    classes: ['IfcSpace', 'IfcWall'],
    // Rooms decoded, so the room schedule is offered.
    check: async (page) => {
      await expect(page.locator('#download-rooms')).toBeEnabled();
    },
  },
  {
    id: 'tiny-hip-roof-2018',
    file: 'tiny-2018-1-hip-roof.rvt',
    classes: ['IfcRoof', 'IfcWall'],
  },
];

for (const example of examples) {
  const staged = fs.existsSync(path.join(demos, example.file));
  (staged ? test : test.skip)(`gallery example ${example.id} opens and shows its feature`, async ({ page }) => {
    await keepGlb(page);
    await page.goto('/');
    await expect(page.locator('#status')).toHaveText(/Ready/);
    const card = page.locator(`[data-demo-id="${example.id}"]`);
    await expect(card).toBeEnabled();
    await expect(card.locator('.demo-shows')).not.toBeEmpty();
    await expect(card).toContainText(/expected:\s*Geometry/i);
    await card.click();
    await expect(page.locator('#status')).toHaveText(/Loaded/);
    await expect(page.locator('#status')).not.toContainText(/error|failed/i);
    await expect(page.locator('#file-meta')).toContainText(example.file);
    await expect(page.locator('#export-quality')).toContainText(/Geometry/);
    // Each class is a row of the Categories panel, with its count.
    const categories = page.locator('#categories label');
    for (const ifcClass of example.classes) {
      await expect(categories.filter({ hasText: new RegExp(`^\\s*${ifcClass}\\s*\\d+\\s*$`) })).toHaveCount(1);
    }
    await example.check?.(page);
  });
}

async function downloadOf(page: Page, selector: string): Promise<Download> {
  await openExportMenu(page);
  const [download] = await Promise.all([
    page.waitForEvent('download', { timeout: 120_000 }),
    page.locator(selector).click(),
  ]);
  return download;
}

async function bytesOf(download: Download): Promise<Buffer> {
  const file = await download.path();
  return fs.readFileSync(file);
}

const exportsDemo = path.join(demos, 'rac_basic_sample_family_2024.rfa');
(fs.existsSync(exportsDemo) ? test : test.skip)(
  'IFCX, its appearance layer, Fragments, OpenUSD and the appearance assets download from the open file',
  async ({ page }) => {
    await page.goto('/');
    await expect(page.locator('#status')).toHaveText(/Ready/);
    for (const id of ['#export-ifcx', '#export-ifcx-appearance', '#export-frag', '#export-usdc', '#export-appearance']) {
      await expect(page.locator(id)).toBeDisabled();
    }
    await page.locator('[data-demo-id="rac-basic-2024"]').click();
    await expect(page.locator('#status')).toHaveText(/Loaded/);

    const ifcxDownload = await downloadOf(page, '#export-ifcx');
    expect(ifcxDownload.suggestedFilename()).toBe('rac_basic_sample_family_2024.ifcx');
    const ifcx = JSON.parse((await bytesOf(ifcxDownload)).toString('utf8'));
    expect(typeof ifcx.header?.id).toBe('string');
    expect(Array.isArray(ifcx.data) && ifcx.data.length).toBeGreaterThan(0);

    const layerDownload = await downloadOf(page, '#export-ifcx-appearance');
    expect(layerDownload.suggestedFilename()).toBe('rac_basic_sample_family_2024.appearance.ifcx');
    const layer = JSON.parse((await bytesOf(layerDownload)).toString('utf8'));
    expect(layer.header.id).toBe(`${ifcx.header.id}/appearance`);
    expect(JSON.stringify(layer.data)).toContain('rvtopen::revit::AppearanceAsset');

    const frag = await bytesOf(await downloadOf(page, '#export-frag'));
    // zlib-deflated FlatBuffers.
    expect(frag[0]).toBe(0x78);
    expect(frag.length).toBeGreaterThan(100);

    const usdc = await bytesOf(await downloadOf(page, '#export-usdc'));
    expect(usdc.subarray(0, 8).toString('latin1')).toBe('PXR-USDC');

    const appearance = JSON.parse((await bytesOf(await downloadOf(page, '#export-appearance'))).toString('utf8'));
    expect(appearance.materials.length).toBeGreaterThan(0);
    expect(appearance.assets.length).toBeGreaterThan(0);
    expect(appearance.unread).toEqual([]);
    await expect(page.locator('#status')).toHaveText(/Exported rac_basic_sample_family_2024\.appearance\.json/);
  },
);

const stairs = path.join(demos, 'tiny-2026-13-stairs.rvt');
(fs.existsSync(stairs) ? test : test.skip)('?demo=<id> in the address opens that sample', async ({ page }) => {
  await page.goto('/?demo=tiny-stairs-2026');
  await expect(page.locator('#status')).toHaveText(/Loaded/);
  await expect(page.locator('#file-meta')).toContainText('tiny-2026-13-stairs.rvt');
});
