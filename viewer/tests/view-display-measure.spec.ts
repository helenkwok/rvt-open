import { expect, test, type Page } from '@playwright/test';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

// Frame times of the View display panel's styles and effects, against the viewer without the panel
// (`?view-display=off`), on the CC0 demo building, Core Interior and Technical School. Runs only with
// RVT_VIEWER_DISPLAY_MEASURE=1; writes measurements/view-display.json and .md.
//
// Each case orbits the camera with a mouse drag of 90 steps and records the time between animation
// frames (what navigating feels like, GPU included) and the milliseconds the page spent issuing each
// frame; then, the camera still, the cost of the one full frame (ambient shadows are drawn only then).

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const demos = path.resolve(__dirname, '../public/demos');
const out = path.resolve(__dirname, '../measurements');
const models = [
  `${demos}/cc0-2027-demo-building.rvt`,
  `${demos}/2024_Core_Interior.rvt`,
  ...(process.env.RVT_SHEETS_MODEL ? [process.env.RVT_SHEETS_MODEL] : []),
].filter((f) => fs.existsSync(f));

const cases: Array<{ name: string; set?: Record<string, string>; check?: string[] }> = [
  { name: 'Shaded with edges', set: { 'vd-style': 'ShadingWithEdges' } },
  { name: 'Wireframe', set: { 'vd-style': 'Wireframe' } },
  { name: 'Hidden Line', set: { 'vd-style': 'HLR' } },
  { name: 'Shaded', set: { 'vd-style': 'Shading' } },
  { name: 'Consistent Colors', set: { 'vd-style': 'FlatColors' } },
  { name: 'Textures', set: { 'vd-style': 'Textures' } },
  { name: 'Realistic', set: { 'vd-style': 'Realistic' } },
  { name: 'Shaded with edges + ambient shadows', set: { 'vd-style': 'ShadingWithEdges' }, check: ['vd-ambientShadows'] },
  { name: 'Hidden Line + sketchy (jitter 5, extension 5)', set: { 'vd-style': 'HLR', 'vd-jitter': '5', 'vd-extension': '5' }, check: ['vd-sketchy'] },
  { name: 'Shaded with edges + cast shadows', set: { 'vd-style': 'ShadingWithEdges' }, check: ['vd-castShadows'] },
];

interface Row {
  model: string;
  case: string;
  rafMedianMs: number;
  rafP95Ms: number;
  issueMeanMs: number;
  stillMs: number;
  stillAo: boolean;
  applyMs?: number;
}

const pct = (xs: number[], p: number): number => {
  const s = [...xs].sort((a, b) => a - b);
  return s.length ? s[Math.min(s.length - 1, Math.floor(p * s.length))] : 0;
};
const r2 = (x: number): number => Math.round(x * 100) / 100;

async function orbit(page: Page): Promise<{ raf: number[]; issue: number[] }> {
  const canvas = page.locator('#viewport canvas').first();
  const box = (await canvas.boundingBox())!;
  await page.evaluate(() => {
    const w = window as unknown as { __raf: number[]; __rafOn: boolean; __issue: number[] };
    w.__raf = [];
    w.__issue = [];
    w.__rafOn = true;
    let last = performance.now();
    const loop = (t: number): void => {
      w.__raf.push(t - last);
      last = t;
      const f = document.querySelector('#view-display-panel')?.getAttribute('data-frame');
      if (f) w.__issue.push(JSON.parse(f).ms);
      if (w.__rafOn) requestAnimationFrame(loop);
    };
    requestAnimationFrame(loop);
  });
  const cx = box.x + box.width / 2;
  const cy = box.y + box.height / 2;
  await page.mouse.move(cx, cy);
  await page.mouse.down();
  for (let i = 0; i < 90; i++) await page.mouse.move(cx + Math.sin(i / 10) * box.width * 0.3, cy + Math.cos(i / 14) * box.height * 0.1);
  await page.mouse.up();
  return page.evaluate(() => {
    const w = window as unknown as { __raf: number[]; __rafOn: boolean; __issue: number[] };
    w.__rafOn = false;
    return { raf: w.__raf.slice(2), issue: w.__issue.slice(2) };
  });
}

(process.env.RVT_VIEWER_DISPLAY_MEASURE === '1' ? test : test.skip)('frame times of every style and effect', async ({ page }) => {
  test.setTimeout(3_600_000);
  fs.mkdirSync(out, { recursive: true });
  const rows: Row[] = [];
  const gl = await page.evaluate(() => {
    const c = document.createElement('canvas').getContext('webgl2');
    const ext = c?.getExtension('WEBGL_debug_renderer_info');
    return ext ? String(c!.getParameter(ext.UNMASKED_RENDERER_WEBGL)) : 'unknown';
  });
  for (const file of models) {
    const name = path.basename(file);
    // today: the viewer without the panel
    await page.goto('/?view-display=off');
    await page.locator('#file-input').setInputFiles(file);
    await expect(page.locator('#status')).toHaveText(/Loaded/, { timeout: 900_000 });
    await page.waitForTimeout(1500);
    let m = await orbit(page);
    rows.push({ model: name, case: 'today (no panel)', rafMedianMs: r2(pct(m.raf, 0.5)), rafP95Ms: r2(pct(m.raf, 0.95)), issueMeanMs: 0, stillMs: 0, stillAo: false });

    await page.goto('/');
    await page.locator('#file-input').setInputFiles(file);
    await expect(page.locator('#status')).toHaveText(/Loaded/, { timeout: 900_000 });
    await expect(page.locator('#view-display-panel')).toHaveAttribute('data-state', 'ready', { timeout: 120_000 });
    await page.locator('#view-display-details > summary').click();
    for (const c of cases) {
      await page.locator('#vd-reset').click();
      const t0 = Date.now();
      for (const [id, v] of Object.entries(c.set ?? {})) {
        const loc = page.locator(`#${id}`);
        if ((await loc.evaluate((e) => e.tagName)) === 'SELECT') await loc.selectOption(v);
        else await loc.fill(v);
      }
      for (const id of c.check ?? []) await page.locator(`#${id}`).check();
      const applyMs = Date.now() - t0;
      await page.waitForTimeout(1500);
      m = await orbit(page);
      await page.waitForTimeout(800);
      const still = JSON.parse((await page.locator('#view-display-panel').getAttribute('data-frame')) ?? '{}') as { ms: number; ao: boolean };
      const issue = m.issue.length ? m.issue.reduce((a, b) => a + b, 0) / m.issue.length : 0;
      rows.push({ model: name, case: c.name, rafMedianMs: r2(pct(m.raf, 0.5)), rafP95Ms: r2(pct(m.raf, 0.95)), issueMeanMs: r2(issue), stillMs: r2(still.ms), stillAo: still.ao, applyMs });
    }
  }
  fs.writeFileSync(path.join(out, 'view-display.json'), JSON.stringify({ gl, rows }, null, 2));
  const md = [
    `WebGL renderer: ${gl}`,
    '',
    '| Model | Case | Frame interval median (ms) | p95 (ms) | Issue per moving frame (ms) | Still frame (ms) | Ambient shadows in still frame | Apply (ms) |',
    '|---|---|---|---|---|---|---|---|',
    ...rows.map((r) => `| ${r.model} | ${r.case} | ${r.rafMedianMs} | ${r.rafP95Ms} | ${r.issueMeanMs} | ${r.stillMs} | ${r.stillAo ? 'yes' : 'no'} | ${r.applyMs ?? ''} |`),
  ].join('\n');
  fs.writeFileSync(path.join(out, 'view-display.md'), md + '\n');
  expect(rows.length).toBeGreaterThan(0);
});
