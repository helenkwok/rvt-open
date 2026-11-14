import fs from 'node:fs';
import path from 'node:path';
import { expect, test, type Page } from '@playwright/test';
import { openExportMenu } from './export-menu';

/**
 * A public project with sheets: BIM4LCA's Office Terrain Concrete BuildingPermit
 * (VTT, CC BY-SA 4.0), which CI fetches into RVT_DRAWINGS_MODEL. Skipped without it.
 */
const MODEL = process.env.RVT_DRAWINGS_MODEL ?? '';

/**
 * A Revit model's sheets and views (src/model-drawings.ts): the Drawings list
 * beside the scene tree, a sheet and a section drawn in the 2D pane on request,
 * switching to the 3D view and back without drawing again, and the shown
 * drawing saved as DXF, DWG and PDF. The counts are the drawing's entities
 * (lines and texts as `rvt-sheet` writes them to DXF), so a change to what is
 * drawn shows here.
 */
const SHOTS = path.join(process.cwd(), 'drawing-shots');

async function shot(page: Page, name: string): Promise<void> {
  fs.mkdirSync(SHOTS, { recursive: true });
  await page.screenshot({ path: path.join(SHOTS, `${name}.png`) });
}

async function drawn(page: Page): Promise<{ lines: number; texts: number; viewports: number }> {
  const svg = page.locator('#drawing-sheet > svg');
  await expect(svg).toBeVisible();
  const d = await svg.evaluate((el) => (el as SVGSVGElement).dataset);
  return { lines: Number(d.lines), texts: Number(d.texts), viewports: Number(d.viewports) };
}

test('lists the sheets and views, draws a sheet and a section, and saves them', async ({ page }) => {
  test.skip(!MODEL || !fs.existsSync(MODEL), 'RVT_DRAWINGS_MODEL is not set or absent');
  // Opening the project and the first drawing (which exports the model in wasm) take minutes in CI.
  test.setTimeout(900_000);
  const t0 = Date.now();
  const at = (what: string): void => console.log(`${what} at ${Math.round((Date.now() - t0) / 1000)} s`);
  await page.goto('/');
  await expect(page.locator('#status')).toHaveText(/Ready/);
  await page.locator('#file-input').setInputFiles(MODEL);
  await expect(page.locator('#status')).toHaveText(/Loaded/);

  at('loaded');
  const list = page.locator('#drawings');
  const sheets = list.locator('button[data-key^="sheet:"]');
  const sections = list.locator('button[data-key^="section:"]');
  await expect(sheets.first()).toBeVisible();
  const counts = {
    sheets: await sheets.count(),
    plans: await list.locator('button[data-key^="plan:"]').count(),
    sections: await sections.count(),
  };
  console.log('drawings listed', JSON.stringify(counts));
  // Templates and loaded families' own plan views are not listed (51 plan views read before).
  expect(counts).toEqual({ sheets: 40, plans: 12, sections: 2 });
  await expect(list.getByRole('button', { name: '3D model' })).toHaveAttribute('aria-pressed', 'true');

  // A sheet with a view placed on it: its paper space, title block and viewports.
  const sheet = list.locator('button[data-key^="sheet:"]:not([title^="0 views"])').first();
  const sheetName = (await sheet.textContent()) ?? '';
  await sheet.click();
  await expect(page.locator('#drawing-pane')).toBeVisible();
  await expect(sheet).toHaveAttribute('aria-pressed', 'true');
  await expect(page.locator('#drawing-title')).toHaveText(sheetName);
  const sheetCounts = await drawn(page);
  console.log('sheet', sheetName, JSON.stringify(sheetCounts));
  console.log(
    'sheet extents',
    await page.locator('#drawing-sheet > svg').evaluate((el) => {
      const svg = el as SVGSVGElement;
      const b = svg.getBBox();
      return JSON.stringify({ viewBox: svg.getAttribute('viewBox'), bbox: [b.x, b.y, b.width, b.height] });
    }),
  );
  // As rvt-sheet draws sheet AR01: its site plan in one viewport, its title block and notes.
  expect(sheetName).toBe('AR01 · ASEMAPIIRUSTUS');
  expect(sheetCounts).toEqual({ lines: 880, texts: 17, viewports: 1 });
  await expect(page.locator('#drawing-sheet svg [data-viewport]')).toHaveCount(sheetCounts.viewports);
  await shot(page, 'sheet');
  at('sheet drawn');

  // The saves are the sheet's, as rvt-sheet writes them. The menu closes after each export.
  for (const [format, magic] of [
    ['dxf', /^\s*0\r?\nSECTION/],
    ['dwg', /^AC10/],
    ['pdf', /^%PDF-/],
  ] as const) {
    const saved = page.waitForEvent('download', { timeout: 120_000 }).catch(async (e: Error) => {
      const status = await page.locator('#status').textContent();
      const toasts = await page.locator('#toast-region').textContent();
      throw new Error(`no ${format} download: status ${JSON.stringify(status)}, toasts ${JSON.stringify(toasts)}; ${e.message}`);
    });
    await openExportMenu(page);
    await page.locator(`#export-view-${format}`).click();
    const download = await saved;
    expect(download.suggestedFilename()).toMatch(new RegExp(`\\.${format}$`));
    const head = fs.readFileSync((await download.path())!).subarray(0, 32).toString('latin1');
    expect(head).toMatch(magic);
    at(`${format} saved`);
  }

  // Back to the 3D view and to the sheet again: nothing is drawn again, and the
  // pan and zoom are kept.
  const svgBefore = await page.locator('#drawing-sheet > svg').elementHandle();
  await page.locator('#drawing-sheet').focus();
  await page.keyboard.press('+');
  const zoomed = await page.locator('#drawing-sheet > svg').getAttribute('viewBox');
  await list.getByRole('button', { name: '3D model' }).click();
  await expect(page.locator('#drawing-pane')).toBeHidden();
  await expect(page.locator('#viewport canvas')).toBeVisible();
  await sheet.click();
  await expect(page.locator('#drawing-pane')).toBeVisible();
  expect(await svgBefore!.evaluate((el) => el.isConnected)).toBe(true);
  await expect(page.locator('#drawing-sheet > svg')).toHaveAttribute('viewBox', zoomed!);

  at('back to the 3D view and the sheet');
  // A section: its elements' edges in its crop box.
  const section = sections.first();
  const sectionName = (await section.textContent()) ?? '';
  await section.click();
  await expect(section).toHaveAttribute('aria-pressed', 'true');
  await expect(page.locator('#drawing-title')).toHaveText(sectionName);
  const sectionCounts = await drawn(page);
  console.log('section', sectionName, JSON.stringify(sectionCounts));
  expect(sectionName).toBe('A');
  expect(sectionCounts).toEqual({ lines: 88, texts: 0, viewports: 0 });
  at('section drawn');
  await shot(page, 'section');

  await list.getByRole('button', { name: '3D model' }).click();
  await shot(page, 'model');
});
