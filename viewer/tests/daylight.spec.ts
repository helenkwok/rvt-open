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
  // the file's sun: the default 3D view's own setting (or the project's preset), named as the source
  await expect(page.locator('#daylight-when-note')).toContainText(/the view “|the project’s sun setting “/i);
  expect(await panel.getAttribute('data-file-sun')).toMatch(/"settingName"/);

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

// ---------- progress, liveness, failures, storeys, weather ----------

/** Replaces the Radiance workers (radiance-N) with a script of our own, while the page's flag is set. */
async function fakeRadiance(page: Page, script: string): Promise<void> {
  await page.addInitScript((src) => {
    const Real = window.Worker;
    const url = URL.createObjectURL(new Blob([src], { type: 'text/javascript' }));
    window.Worker = class extends Real {
      constructor(u: string | URL, opts?: WorkerOptions) {
        super(opts?.name?.startsWith('radiance-') ? url : u, opts);
      }
    };
  }, script);
}

(fs.existsSync(demo) && fs.existsSync(radianceBuild) ? test : test.skip)('a Radiance run names its stages, moves the bar with the sensors, and ends with a summary', async ({ page }) => {
  test.setTimeout(400_000);
  await openPanel(page);
  const panel = page.locator('#daylight-panel');
  // the stages, the counts and the bar as the run goes
  await page.evaluate(() => {
    const p = document.getElementById('daylight-panel')!;
    const seen: { stage: string; done: number; bar: number; live: string }[] = [];
    (window as unknown as { __seen: typeof seen }).__seen = seen;
    new MutationObserver(() => {
      const bar = document.getElementById('daylight-progress') as HTMLProgressElement;
      seen.push({ stage: p.dataset.stage ?? '', done: Number(p.dataset.done ?? 0), bar: bar.position, live: document.getElementById('daylight-live')!.textContent ?? '' });
    }).observe(p, { attributes: true, subtree: true, characterData: true, childList: true });
  });
  await page.locator('#daylight-analysis').selectOption('rad-df');
  await page.locator('#daylight-run').click();
  // the status and the bar sit beside Run and Stop
  await expect(page.locator('#daylight-runbox #daylight-stop')).toBeVisible();
  await expect(page.locator('#daylight-runbox #daylight-progress')).toBeVisible();
  await expect(page.locator('#daylight-live')).toContainText(/Elapsed \d+:\d\d · last update \d+ s ago/);
  await expect(panel).toHaveAttribute('data-state', 'done', { timeout: 300_000 });
  const seen = await page.evaluate(() => (window as unknown as { __seen: { stage: string; done: number; bar: number; live: string }[] }).__seen);
  const stages = [...new Set(seen.map((s) => s.stage).filter(Boolean))];
  for (const s of ['scene', 'load', 'sky', 'radtext', 'oconv', 'oconv-sky', 'rtrace', 'collect']) expect(stages).toContain(s);
  // the bar moved with the sensors, through values between empty and full
  const traced = seen.filter((s) => s.stage === 'rtrace' && s.done > 0 && s.done < 391);
  expect(traced.length).toBeGreaterThan(0);
  expect(traced.some((s) => s.bar > 0 && s.bar < 1)).toBe(true);
  await expect(page.locator('#daylight-status')).toHaveText(/^Done in \d+\.\d s: 391 sensors, mean \d+\.\d\d %/);
  await expect(page.locator('#daylight-live')).toContainText('Stages:');
  const timeline = JSON.parse((await panel.getAttribute('data-timeline')) ?? '[]') as [string, number][];
  expect(timeline.map(([s]) => s)).toEqual(expect.arrayContaining(['oconv', 'rtrace']));
  // a second run of the same scene reuses its octree: only the sky is added
  await page.locator('#daylight-run').click();
  await expect(panel).toHaveAttribute('data-state', 'done', { timeout: 300_000 });
  const again = JSON.parse((await panel.getAttribute('data-timeline')) ?? '[]') as [string, number][];
  expect(again.map(([s]) => s)).not.toContain('oconv');
  const [download] = await Promise.all([page.waitForEvent('download'), page.locator('#daylight-export-json').click()]);
  const result = JSON.parse(fs.readFileSync(await download.path(), 'utf8'));
  expect(result.parameters.sceneOctreeReused).toBe(true);
  expect(result.parameters.stages.map((s: { stage: string }) => s.stage)).toContain('rtrace');
  expect(result.parameters.skippedTriangles).toEqual({ degenerate: expect.any(Number), duplicate: expect.any(Number) });
});

(fs.existsSync(demo) ? test : test.skip)('a run with no progress for the set time says so, and Stop ends it', async ({ page }) => {
  test.setTimeout(180_000);
  // Radiance workers that never answer
  await fakeRadiance(page, '/* silent */');
  await page.goto('/?demo=cc0-demo-building-2027&daylight-stall-ms=1500');
  await expect(page.locator('#status')).toHaveText(/Loaded/);
  await page.locator('#daylight-details > summary').click();
  const panel = page.locator('#daylight-panel');
  await expect(panel).toHaveAttribute('data-state', 'ready');
  await page.locator('#daylight-analysis').selectOption('rad-df');
  await page.locator('#daylight-run').click();
  await expect(panel).toHaveAttribute('data-stalled', 'true', { timeout: 60_000 });
  await expect(page.locator('#daylight-live')).toContainText(/No progress for \d+ s .*still working, or stuck\? Stop ends the run/);
  await page.locator('#daylight-stop').click();
  await expect(panel).toHaveAttribute('data-state', 'idle');
  await expect(page.locator('#daylight-status')).toHaveText(/^Stopped after \d+:\d\d\.$/);
  await expect(page.locator('#daylight-run')).toBeEnabled();
});

(fs.existsSync(demo) ? test : test.skip)('a Radiance failure shows the fatal line first, the warnings counted, and the log behind Details and in the export', async ({ page }) => {
  test.setTimeout(180_000);
  // every Radiance program fails as oconv did on a large model: warnings, then the fatal line
  await fakeRadiance(
    page,
    `self.onmessage = (e) => self.postMessage({ type: 'error', req: e.data.req, prog: 'oconv', message: 'oconv exited 2',
      log: 'this.program: warning - zero area for polygon "t814734"\\nthis.program: warning - zero area for polygon "t814735"\\nthis.program: warning - zero area for polygon "t9"\\nthis.program: system - out of memory in fullnode: Out of memory\\n' });`,
  );
  await openPanel(page);
  const panel = page.locator('#daylight-panel');
  await page.locator('#daylight-analysis').selectOption('rad-df');
  await page.locator('#daylight-run').click();
  await expect(panel).toHaveAttribute('data-state', 'error', { timeout: 120_000 });
  await expect(page.locator('#daylight-status')).toHaveText(
    'Daylight run failed. Radiance could not build the scene: out of memory in fullnode: Out of memory (the program ran out of memory: WebAssembly gives each one at most 2 GiB). 3 zero-area polygons skipped.',
  );
  const details = page.locator('#daylight-error-details');
  await expect(details).toBeVisible();
  await details.locator('summary').click();
  await expect(details.locator('pre')).toContainText('zero area for polygon "t814734"');
  const [download] = await Promise.all([page.waitForEvent('download'), page.locator('#daylight-export-json').click()]);
  const result = JSON.parse(fs.readFileSync(await download.path(), 'utf8'));
  expect(result.failed).toBe(true);
  expect(result.log).toContain('out of memory in fullnode');
  expect(result.error).toContain('Radiance could not');
});

const coreInterior = path.resolve(__dirname, '../public/demos/2024_Core_Interior.rvt');
(fs.existsSync(coreInterior) ? test : test.skip)('the Storey control shows one level at a time: its grid, cut, table and scale', async ({ page }) => {
  test.setTimeout(400_000);
  await page.goto('/?demo=core-interior-2024');
  await expect(page.locator('#status')).toHaveText(/Loaded/, { timeout: 300_000 });
  await page.locator('#daylight-details > summary').click();
  const panel = page.locator('#daylight-panel');
  await expect(panel).toHaveAttribute('data-state', 'ready', { timeout: 120_000 });
  await page.locator('#daylight-analysis').selectOption('svf');
  await page.locator('#daylight-spacing').fill('1');
  await page.locator('#daylight-spacing').dispatchEvent('change');
  await page.locator('#daylight-paths').selectOption('1024');
  await page.locator('#daylight-run').click();
  await expect(panel).toHaveAttribute('data-state', 'done', { timeout: 300_000 });
  const all = Number(await panel.getAttribute('data-overlay'));
  const storey = page.locator('#daylight-storey');
  await expect(page.locator('#daylight-storeys')).toBeVisible();
  const options = await storey.locator('option').allTextContents();
  expect(options.length).toBeGreaterThan(2);
  expect(options[0]).toMatch(/^All levels/);
  const allLegend = await panel.getAttribute('data-legend');
  const allCut = Number(await page.locator('#daylight-cut-height').inputValue());
  // the lowest level: only its sensors, the cut just above them, the table on that level
  await storey.selectOption({ index: 1 });
  const level = await panel.getAttribute('data-storey');
  expect(level).toBeTruthy();
  const n1 = Number(await panel.getAttribute('data-overlay'));
  expect(n1).toBeGreaterThan(0);
  expect(n1).toBeLessThan(all);
  expect(options[1]).toContain(`${n1.toLocaleString('en')} sensors`);
  const cut1 = Number(await page.locator('#daylight-cut-height').inputValue());
  expect(cut1).toBeLessThan(allCut);
  for (const cell of await page.locator('#daylight-table tbody tr td:nth-child(2)').allTextContents()) expect(cell).toBe(level);
  await expect(page.locator('#daylight-legend')).toContainText('scale of this level');
  // Up steps to the next level up, Down back; the buttons do the same
  await storey.focus();
  await page.keyboard.press('ArrowUp');
  const up = await panel.getAttribute('data-storey');
  expect(up).not.toBe(level);
  expect(Number(await page.locator('#daylight-cut-height').inputValue())).toBeGreaterThanOrEqual(cut1);
  await page.keyboard.press('ArrowDown');
  await expect(panel).toHaveAttribute('data-storey', level!);
  await page.locator('#daylight-storey-up').click();
  await expect(panel).toHaveAttribute('data-storey', up!);
  await page.locator('#daylight-storey-down').click();
  await expect(panel).toHaveAttribute('data-storey', level!);
  // one colour scale for every level
  await page.locator('#daylight-shared-scale').check();
  await expect(panel).toHaveAttribute('data-legend', allLegend!);
  await expect(page.locator('#daylight-legend')).toContainText('scale of every level');
  // a typed cut overrides until another level is picked
  await page.locator('#daylight-cut-height').fill('100');
  await page.locator('#daylight-cut-height').dispatchEvent('change');
  await expect(panel).toHaveAttribute('data-cut', '100');
  await page.locator('#daylight-storey-up').click();
  expect(Number(await page.locator('#daylight-cut-height').inputValue())).toBeLessThan(100);
  await page.mouse.move(10, 10);
  await page.screenshot({ path: path.join(shots, 'storey.png') });
  // the exports hold every level
  const [download] = await Promise.all([page.waitForEvent('download'), page.locator('#daylight-export-json').click()]);
  const result = JSON.parse(fs.readFileSync(await download.path(), 'utf8'));
  expect(result.sensors).toHaveLength(all);
  expect(result.storeys.length).toBe(options.length - 1);
  // All shows every sensor again
  await storey.selectOption('');
  await expect(panel).toHaveAttribute('data-overlay', String(all));
});

(fs.existsSync(demo) ? test : test.skip)('a weather file dropped on the panel loads, is kept for the site, and can be forgotten', async ({ page }) => {
  test.setTimeout(240_000);
  await openPanel(page);
  const panel = page.locator('#daylight-panel');
  await page.locator('#daylight-analysis').selectOption('rad-pit');
  const text = fs.readFileSync(path.resolve(__dirname, 'data/pvgis-tmy-demo-building-june.json'), 'utf8');
  const drop = async (target: string): Promise<void> => {
    const dt = await page.evaluateHandle((t) => {
      const d = new DataTransfer();
      d.items.add(new File([t], 'tmy.json', { type: 'application/json' }));
      return d;
    }, text);
    await page.locator(target).dispatchEvent('drop', { dataTransfer: dt });
  };
  await drop('#daylight-panel');
  await expect(panel).toHaveAttribute('data-weather', '48');
  await expect(page.locator('#daylight-weather-saved')).toContainText('Kept in this browser for the site 42.3587,-71.0567: tmy.json');
  // the same model again: the kept file loads without the dialog
  await page.reload();
  await expect(page.locator('#status')).toHaveText(/Loaded/);
  await page.locator('#daylight-details > summary').click();
  await expect(panel).toHaveAttribute('data-weather', '48', { timeout: 60_000 });
  await expect(panel).toHaveAttribute('data-weather-from', 'kept');
  await page.locator('#daylight-analysis').selectOption('rad-pit');
  await expect(page.locator('#daylight-weather-saved')).toBeVisible();
  await expect(page.locator('#daylight-weather-saved')).toContainText('Loaded from this browser');
  await page.locator('#daylight-weather-forget').click();
  await expect(page.locator('#daylight-weather-saved')).toBeHidden();
  await page.reload();
  await expect(page.locator('#status')).toHaveText(/Loaded/);
  await page.locator('#daylight-details > summary').click();
  await expect(panel).toHaveAttribute('data-state', 'ready');
  await page.waitForTimeout(1000);
  expect(await panel.getAttribute('data-weather')).toBeNull();
  // dropped on the 3D view while the analysis that uses weather is chosen
  await page.locator('#daylight-analysis').selectOption('rad-pit');
  await drop('#viewport');
  await expect(panel).toHaveAttribute('data-weather', '48');
  await expect(page.locator('#status')).not.toContainText('Ignored');
});
