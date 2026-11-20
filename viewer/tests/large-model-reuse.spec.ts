import fs from 'node:fs';
import { expect, test, type Page } from '@playwright/test';

/**
 * A public project with sheets (Autodesk's metric Technical School sample, which CI fetches into
 * RVT_SHEETS_MODEL). Skipped without it.
 */
const MODEL = process.env.RVT_SHEETS_MODEL ?? '';

// The first drawing of a file draws from the model the open kept, or exports the file again when the open kept
// none. An open under the Large Model limits keeps it when no limit bound during the open (no stream left unread,
// no limit event): its model is then the default limits' model, and the drawing is the same as the re-export's.
// `?maxStreamBytes=200` binds the first open (a stream is left unread), so an open under it keeps nothing; the
// retry under the Large Model limits is what is checked. A mode other than scaffold is never kept (the first
// drawing exports the file again at the defaults), which is how the re-export is forced here (the select is
// hidden until a file is open, so its value is set directly, as the page reads it).

interface Drawn {
  /** Whether the first drawing took the open's kept model (1) or exported the file again (0): `modelKept` of the drawing's steps. */
  kept: number;
  /** The model step of the first drawing, ms (logged, not asserted). */
  modelMs: number;
  /** The drawing's SVG content, as bytes of markup: the elements, not the timing data on the root. */
  svg: string;
}

async function openAndDraw(page: Page, query: string, prepare?: (page: Page) => Promise<void>): Promise<Drawn> {
  await page.setViewportSize({ width: 1600, height: 1000 });
  await page.goto(`/${query}`);
  await expect(page.locator('#status')).toHaveText(/Ready/);
  await prepare?.(page);
  await page.locator('#file-input').setInputFiles(MODEL);
  await expect(page.locator('#status')).toHaveText(/Loaded/, { timeout: 600_000 });
  if (query.includes('maxStreamBytes')) {
    await page.locator('#retry-large-model').click();
    await expect(page.locator('#large-model-note')).toHaveCount(0);
    await expect(page.locator('#status')).toHaveText(/Loaded/, { timeout: 600_000 });
  }
  const sheet = page.locator('#drawings button[data-key^="sheet:"]', { hasText: /A1/ }).first();
  await expect(sheet).toBeVisible({ timeout: 300_000 });
  await sheet.click();
  const svg = page.locator('#drawing-sheet > svg');
  await expect(svg).toHaveAttribute('data-timeline', /painted/, { timeout: 600_000 });
  const times = JSON.parse((await svg.getAttribute('data-times')) ?? '{}') as Record<string, number>;
  return { kept: times.modelKept, modelMs: times.model, svg: await svg.evaluate((el) => el.innerHTML) };
}

test.describe('the first drawing after a Large Model open', () => {
  test.skip(!MODEL || !fs.existsSync(MODEL), 'RVT_SHEETS_MODEL is not set or absent');
  test.setTimeout(900_000);

  test('takes the kept model when no limit bound, and draws what the re-export draws', async ({ browser }) => {
    const draw = async (query: string, prepare?: (page: Page) => Promise<void>) => {
      const page = await (await browser.newContext()).newPage();
      try {
        return await openAndDraw(page, query, prepare);
      } finally {
        await page.context().close();
      }
    };
    const defaults = await draw('');
    const largeModel = await draw('?maxStreamBytes=200');
    const reExport = await draw('', (page) => page.locator('#export-mode').evaluate((el) => { (el as HTMLSelectElement).value = 'typed-no-geometry'; }));
    console.log(`first drawing, model step ms: default ${defaults.modelMs}, large model ${largeModel.modelMs}, re-export ${reExport.modelMs}`);

    // The kept model is taken, not exported again.
    expect(defaults.kept, 'the default open keeps its model').toBe(1);
    expect(largeModel.kept, 'the Large Model open keeps its model').toBe(1);
    expect(reExport.kept, 'a mode other than scaffold keeps nothing').toBe(0);
    // And it is the same drawing, byte for byte.
    expect(largeModel.svg.length).toBeGreaterThan(10_000);
    expect(largeModel.svg === reExport.svg, 'Large Model drawing equals the re-export').toBe(true);
    expect(defaults.svg === reExport.svg, 'default drawing equals the re-export').toBe(true);
  });

  test('an open a limit bound on keeps nothing, and the first drawing exports the file again', async ({ page }) => {
    // The first open under ?maxStreamBytes=200 leaves a stream unread, so its model is incomplete: not kept.
    await page.setViewportSize({ width: 1600, height: 1000 });
    await page.goto('/?maxStreamBytes=200');
    await expect(page.locator('#status')).toHaveText(/Ready/);
    await page.locator('#file-input').setInputFiles(MODEL);
    await expect(page.locator('#status')).toHaveText(/Loaded/, { timeout: 600_000 });
    await expect(page.locator('#large-model-note')).toHaveCount(1);
    const sheet = page.locator('#drawings button[data-key^="sheet:"]', { hasText: /A1/ }).first();
    await expect(sheet).toBeVisible({ timeout: 300_000 });
    await sheet.click();
    const svg = page.locator('#drawing-sheet > svg');
    await expect(svg).toHaveAttribute('data-timeline', /painted/, { timeout: 600_000 });
    const times = JSON.parse((await svg.getAttribute('data-times')) ?? '{}') as Record<string, number>;
    expect(times.modelKept, 'the first drawing exported the file again').toBe(0);
  });
});
