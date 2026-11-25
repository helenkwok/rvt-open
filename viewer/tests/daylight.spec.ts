import { expect, test, type Page } from '@playwright/test';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

// The Daylight panel on the CC0 demo building (research/cc0-models.tsv):
// the file's site shows, each analysis of the instant tier runs on the pool of
// workers and draws its coloured grid, and the results export. The numbers
// themselves are checked against Radiance in daylight-rt/tests/demo_building.rs;
// here the sky view factor's mean is checked again end to end, through the
// browser's own scene and grid.

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const demo = path.resolve(__dirname, '../public/demos/cc0-2027-demo-building.rvt');
const shots = path.resolve(__dirname, '../daylight-shots');

async function openPanel(page: Page): Promise<void> {
  await page.goto('/?demo=cc0-demo-building-2027');
  await expect(page.locator('#status')).toHaveText(/Loaded/);
  await page.locator('#daylight-details > summary').click();
  await expect(page.locator('#daylight-panel')).toHaveAttribute('data-state', 'ready');
}

async function run(page: Page, analysis: string): Promise<number> {
  const panel = page.locator('#daylight-panel');
  await page.locator('#daylight-analysis').selectOption(analysis);
  await page.locator('#daylight-run').click();
  await expect(panel).toHaveAttribute('data-analysis', analysis);
  await expect(panel).toHaveAttribute('data-state', 'done');
  await expect(panel).toHaveAttribute('data-overlay', '391');
  await expect(page.locator('#daylight-legend')).toBeVisible();
  await expect(page.locator('#daylight-table tbody tr').first()).toBeVisible();
  return Number(await panel.getAttribute('data-mean'));
}

(fs.existsSync(demo) ? test : test.skip)('the Daylight panel runs every analysis on the demo building', async ({ page }) => {
  test.setTimeout(240_000);
  fs.mkdirSync(shots, { recursive: true });
  await openPanel(page);
  const panel = page.locator('#daylight-panel');
  // the file's site: Revit's location (the start location of new projects), shown as such
  await expect(page.locator('#daylight-latitude')).toHaveValue(/^42\.358/);
  await expect(page.locator('#daylight-longitude')).toHaveValue(/^-71\.056/);
  await expect(panel.locator('.daylight-source').first()).toHaveText('from the file');
  await expect(page.locator('#daylight-site-note')).toContainText('Boston');

  await page.locator('#daylight-analysis').selectOption('sunhours');
  await page.locator('#daylight-date').fill('2026-03-20');
  await page.locator('#daylight-date-to').fill('2026-03-20');
  await page.locator('#daylight-step').selectOption('60');
  const sunHours = await run(page, 'sunhours');
  expect(sunHours).toBeGreaterThan(0);
  await page.screenshot({ path: path.join(shots, 'sun-hours.png') });

  const svf = await run(page, 'svf');
  // Radiance's mean sky view factor on these sensors is 0.0125 (daylight-rt/tests/data/demo-building/ref_svf.dat)
  expect(Math.abs(svf - 0.0125)).toBeLessThan(0.002);
  await page.screenshot({ path: path.join(shots, 'sky-view.png') });

  await page.locator('#daylight-paths').selectOption('1024');
  const df = await run(page, 'df');
  // Radiance's mean daylight factor at -ab 2 is 1.09 %; 1,024 paths per sensor is a coarse preview
  expect(Math.abs(df / 1.09 - 1)).toBeLessThan(0.05);
  await page.mouse.move(10, 10);
  await page.screenshot({ path: path.join(shots, 'daylight-factor.png') });

  const [download] = await Promise.all([page.waitForEvent('download'), page.locator('#daylight-export-json').click()]);
  expect(download.suggestedFilename()).toBe('cc0-2027-demo-building.daylight-df.json');
  const result = JSON.parse(fs.readFileSync(await download.path(), 'utf8'));
  expect(result.sensors).toHaveLength(391);
  expect(result.site.latitude.source).toBe('file');
  expect(result.limits.length).toBeGreaterThan(3);
  const [csv] = await Promise.all([page.waitForEvent('download'), page.locator('#daylight-export-csv').click()]);
  expect(fs.readFileSync(await csv.path(), 'utf8').trim().split('\n')).toHaveLength(392);

  // the sun at 21 June, 15:00 standard time, and its shadows
  await page.locator('#daylight-analysis').selectOption('sun');
  await page.locator('#daylight-date').fill('2026-06-21');
  await page.locator('#daylight-time').evaluate((e: HTMLInputElement) => {
    e.value = '15';
    e.dispatchEvent(new Event('input', { bubbles: true }));
  });
  await page.locator('#daylight-run').click();
  await expect(panel).toHaveAttribute('data-shadows', 'on');
  await page.locator('#daylight-sunpath').check();
  await expect(panel).toHaveAttribute('data-sun-path', 'on');
  const sun = JSON.parse((await panel.getAttribute('data-sun')) ?? '{}');
  expect(sun.altitude).toBeGreaterThan(30);
  expect(sun.azimuth).toBeGreaterThan(200);
  expect(sun.azimuth).toBeLessThan(270);
  await page.screenshot({ path: path.join(shots, 'sun-and-shadows.png') });
});

const radianceBuild = path.resolve(__dirname, '../public/radiance/rtrace.wasm');
(fs.existsSync(demo) && fs.existsSync(radianceBuild) ? test : test.skip)(
  'the Daylight panel runs Radiance itself: daylight factor and illuminance from a PVGIS year',
  async ({ page }) => {
    test.setTimeout(400_000);
    fs.mkdirSync(shots, { recursive: true });
    await openPanel(page);
    const panel = page.locator('#daylight-panel');
    // the placeholder materials are listed as assumed
    await expect(page.locator('#daylight-materials summary')).toContainText(/assumed/);
    await page.locator('#daylight-analysis').selectOption('rad-df');
    await expect(page.locator('#daylight-radiance-note')).toContainText('Radiance Software License 2.0');
    await page.locator('#daylight-run').click();
    await expect(panel).toHaveAttribute('data-analysis', 'rad-df', { timeout: 300_000 });
    await expect(panel).toHaveAttribute('data-state', 'done');
    await expect(panel).toHaveAttribute('data-overlay', '391');
    const df = Number(await panel.getAttribute('data-mean'));
    // native Radiance's mean at -ab 2 is 1.0900 % (daylight-rt/tests/data/demo-building/df_p1.dat); each worker
    // builds its own irradiance cache, which moves the mean by a fraction of a percent
    expect(Math.abs(df / 1.09 - 1)).toBeLessThan(0.02);
    await page.screenshot({ path: path.join(shots, 'radiance-daylight-factor.png') });

    await page.locator('#daylight-analysis').selectOption('rad-pit');
    await expect(page.locator('#daylight-pvgis')).toHaveAttribute('href', /^https:\/\/re\.jrc\.ec\.europa\.eu\/api\/v5_3\/tmy\?lat=42\.3587&lon=-71\.0567/);
    await page.locator('#daylight-weather-file').setInputFiles(path.resolve(__dirname, 'data/pvgis-tmy-demo-building-june.json'));
    await expect(panel).toHaveAttribute('data-weather', '48');
    await expect(page.locator('#daylight-weather-note')).toContainText('PVGIS-ERA5');
    await page.locator('#daylight-date').fill('2026-06-21');
    await page.locator('#daylight-time').evaluate((e: HTMLInputElement) => {
      e.value = '12';
      e.dispatchEvent(new Event('input', { bubbles: true }));
    });
    await page.locator('#daylight-run').click();
    await expect(panel).toHaveAttribute('data-analysis', 'rad-pit', { timeout: 300_000 });
    await expect(panel).toHaveAttribute('data-state', 'done');
    await expect(page.locator('#daylight-credit')).toHaveText('PVGIS © European Communities, 2001-2026');
    expect(Number(await panel.getAttribute('data-mean'))).toBeGreaterThan(50);
    await page.mouse.move(10, 10);
    await page.screenshot({ path: path.join(shots, 'radiance-illuminance.png') });
    const [download] = await Promise.all([page.waitForEvent('download'), page.locator('#daylight-export-json').click()]);
    const result = JSON.parse(fs.readFileSync(await download.path(), 'utf8'));
    expect(result.engine).toBe('Radiance 6.0.3 (WebAssembly)');
    expect(result.credit).toBe('PVGIS © European Communities, 2001-2026');
    expect(result.parameters.weather.radiationDatabase).toBe('PVGIS-ERA5');
    expect(result.parameters.weather.years).toBe('2005-2023');
    expect(result.parameters.weather.horizon).toBe('DEM-calculated');
    expect(result.parameters.rtrace).toBe('-I -h -ab 2 -aa 0.1 -ad 2048 -ar 64');
  },
);

(fs.existsSync(demo) ? test : test.skip)('a daylight worker that cannot start fails the run instead of hanging it', async ({ page }) => {
  test.setTimeout(180_000);
  // while the flag is set, the pool's workers (named daylight-N) load a script that does not exist
  await page.addInitScript(() => {
    const Real = window.Worker;
    const w = window as unknown as { __breakDaylight?: boolean };
    window.Worker = class extends Real {
      constructor(url: string | URL, opts?: WorkerOptions) {
        super(w.__breakDaylight && opts?.name?.startsWith('daylight-') ? '/no-such-daylight-worker.js' : url, opts);
      }
    };
  });
  await openPanel(page);
  const panel = page.locator('#daylight-panel');
  await page.evaluate(() => {
    (window as unknown as { __breakDaylight?: boolean }).__breakDaylight = true;
  });
  await page.locator('#daylight-analysis').selectOption('svf');
  await page.locator('#daylight-run').click();
  await expect(panel).toHaveAttribute('data-state', 'error', { timeout: 30_000 });
  await expect(page.locator('#daylight-status')).toContainText('a daylight worker failed');
  await expect(page.locator('#daylight-run')).toBeEnabled();
  // the failed pool is gone: the next run starts new workers and finishes
  await page.evaluate(() => {
    (window as unknown as { __breakDaylight?: boolean }).__breakDaylight = false;
  });
  await page.locator('#daylight-run').click();
  await expect(panel).toHaveAttribute('data-state', 'done', { timeout: 120_000 });
  await expect(panel).toHaveAttribute('data-overlay', '391');
});

(fs.existsSync(demo) && fs.existsSync(radianceBuild) ? test : test.skip)('a Radiance worker that cannot start fails the run instead of reading as stopped', async ({ page }) => {
  test.setTimeout(400_000);
  // while the flag is set, the Radiance workers (named radiance-N) load a script that does not exist
  await page.addInitScript(() => {
    const Real = window.Worker;
    const w = window as unknown as { __breakRadiance?: boolean };
    window.Worker = class extends Real {
      constructor(url: string | URL, opts?: WorkerOptions) {
        super(w.__breakRadiance && opts?.name?.startsWith('radiance-') ? '/no-such-radiance-worker.js' : url, opts);
      }
    };
  });
  await openPanel(page);
  const panel = page.locator('#daylight-panel');
  await page.evaluate(() => {
    (window as unknown as { __breakRadiance?: boolean }).__breakRadiance = true;
  });
  await page.locator('#daylight-analysis').selectOption('rad-df');
  await page.locator('#daylight-run').click();
  await expect(panel).toHaveAttribute('data-state', 'error', { timeout: 120_000 });
  await expect(page.locator('#daylight-status')).toContainText('a Radiance worker failed');
  await expect(page.locator('#daylight-run')).toBeEnabled();
  // the next run starts new workers and finishes
  await page.evaluate(() => {
    (window as unknown as { __breakRadiance?: boolean }).__breakRadiance = false;
  });
  await page.locator('#daylight-run').click();
  await expect(panel).toHaveAttribute('data-state', 'done', { timeout: 300_000 });
  await expect(panel).toHaveAttribute('data-analysis', 'rad-df');
  await expect(panel).toHaveAttribute('data-overlay', '391');
});
