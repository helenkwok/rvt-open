import fs from 'node:fs';
import path from 'node:path';
import { expect, test, type Page } from '@playwright/test';

/**
 * A public project with sheets at several scales: Autodesk's metric Technical School sample, which
 * CI fetches into RVT_SHEETS_MODEL. Skipped without it.
 */
const MODEL = process.env.RVT_SHEETS_MODEL ?? '';

/**
 * A sheet's lines on screen are their lineweights on paper, in every viewport whatever its scale:
 * sheet A1 holds a 1:100 floor plan, and sheet A2 sections and details at several scales. Each line's on-screen width is its computed stroke width times the
 * screen scale of the element (the sheet is fitted to the pane). It also reports how long the
 * first drawing and a later one take.
 */
const SHOTS = path.join(process.cwd(), 'drawing-shots');

interface Widths {
  count: number;
  min: number;
  max: number;
  /** Viewport scale (paper per model unit) -> on-screen widths of its default-weight lines. */
  byViewport: Record<string, number[]>;
}

/** On-screen stroke widths, px, of the lines drawn through viewports and of the paper's own. */
async function widths(page: Page): Promise<{ viewports: Widths; paper: Widths }> {
  return page.locator('#drawing-sheet > svg').evaluate((root) => {
    const measure = (els: Element[]): Widths => {
      const out: Widths = { count: 0, min: Infinity, max: -Infinity, byViewport: {} };
      for (const el of els) {
        if (!(el instanceof SVGGeometryElement) || el.getAttribute('stroke') === 'none') continue;
        const ctm = el.getScreenCTM();
        if (!ctm) continue;
        const px = parseFloat(getComputedStyle(el).strokeWidth) * Math.hypot(ctm.a, ctm.b);
        out.count += 1;
        out.min = Math.min(out.min, px);
        out.max = Math.max(out.max, px);
        const group = el.closest('[data-viewport] > g[transform]');
        if (group && !el.hasAttribute('stroke-width')) {
          const k = (group.getAttribute('transform') ?? '').replace(/^matrix\(([^ ]+).*/, '$1');
          (out.byViewport[k] ??= []).push(px);
        }
      }
      return out;
    };
    const all = Array.from(root.querySelectorAll('[data-handle]'));
    return {
      viewports: measure(all.filter((el) => el.closest('[data-viewport]'))),
      paper: measure(all.filter((el) => !el.closest('[data-viewport]'))),
    };
  });
}

async function openSheet(page: Page, name: RegExp): Promise<{ ms: number; label: string; times: string }> {
  const button = page.locator('#drawings button[data-key^="sheet:"]', { hasText: name }).first();
  const label = (await button.textContent()) ?? '';
  const t0 = Date.now();
  await button.click();
  await expect(page.locator('#drawing-title')).toHaveText(label, { timeout: 600_000 });
  const svg = page.locator('#drawing-sheet > svg');
  await expect(svg).toBeVisible();
  const ms = Date.now() - t0;
  return { ms, label, times: (await svg.getAttribute('data-times')) ?? '' };
}

test('sheet lines are their lineweights on paper at every viewport scale', async ({ page }) => {
  test.skip(!MODEL || !fs.existsSync(MODEL), 'RVT_SHEETS_MODEL is not set or absent');
  test.setTimeout(900_000);
  await page.setViewportSize({ width: 1600, height: 1000 });
  await page.goto('/');
  await expect(page.locator('#status')).toHaveText(/Ready/);
  const t0 = Date.now();
  await page.locator('#file-input').setInputFiles(MODEL);
  await expect(page.locator('#status')).toHaveText(/Loaded/, { timeout: 600_000 });
  console.log(`project open: ${Date.now() - t0} ms`);
  const tList = Date.now();
  const sheets = page.locator('#drawings button[data-key^="sheet:"]');
  await expect(sheets.first()).toBeVisible({ timeout: 600_000 });
  console.log(
    `drawings listed: ${Date.now() - tList} ms after the open, steps ${await page.locator('#drawings').getAttribute('data-times')}`,
  );
  console.log('sheets', JSON.stringify(await sheets.allTextContents()));

  fs.mkdirSync(SHOTS, { recursive: true });
  for (const [name, file] of [
    [/^A1 · Floor Plan$/, 'technical-school-a1-floor-plan'],
    [/^A2 · Sections$/, 'technical-school-a2-sections'],
  ] as const) {
    const opened = await openSheet(page, name);
    console.log(`drawing ${opened.label}: ${opened.ms} ms, steps ${opened.times}`);
    await page.screenshot({ path: path.join(SHOTS, `${file}.png`) });
    const w = await widths(page);
    console.log(
      `${opened.label} widths px: viewports ${w.viewports.count} lines ${w.viewports.min.toFixed(3)}..${w.viewports.max.toFixed(3)}, ` +
        `paper ${w.paper.count} lines ${w.paper.min.toFixed(3)}..${w.paper.max.toFixed(3)}, ` +
        `default weight by viewport scale ${JSON.stringify(Object.fromEntries(Object.entries(w.viewports.byViewport).map(([k, v]) => [k, [v.length, Math.min(...v).toFixed(3), Math.max(...v).toFixed(3)]])))}`,
    );
    // Fitted, a sheet is about 1.5 px per paper millimetre here: lineweights of 0.1 to 1 mm.
    expect(w.viewports.count).toBeGreaterThan(100);
    expect(w.viewports.min).toBeGreaterThanOrEqual(0.1);
    expect(w.viewports.max).toBeLessThanOrEqual(2);
    expect(w.paper.min).toBeGreaterThanOrEqual(0.1);
    expect(w.paper.max).toBeLessThanOrEqual(2);
    // A default-weight line is as wide on screen in a 1:20 viewport as in a 1:100 one.
    const defaults = Object.values(w.viewports.byViewport).flat();
    expect(Math.max(...defaults) / Math.min(...defaults)).toBeLessThan(1.01);
    if (file.endsWith('sections')) expect(Object.keys(w.viewports.byViewport).length).toBeGreaterThan(1);
  }
});
