import { expect, test, type Page } from '@playwright/test';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

// Every Daylight analysis on real public samples, not only the demo building: each run must end in
// "done" with a grid, and the progress bar must move while it traces. The table of models, sensors,
// results and times goes to daylight-shots/samples.md (and samples.json).
//
//   RVT_DAYLIGHT_SAMPLES   models, one per line or separated by ';', each `path` or `path|spacing`; a path starting
//                          with `?` is optional (left out when absent, as CI allows for the CC0 model's host)
//                          (grid spacing in metres; default 0.5). Default: the staged demo building
//                          and Core Interior, and RVT_SHEETS_MODEL (Technical School) at 2 m.
//   RVT_DAYLIGHT_ANALYSES  comma-separated (default sunhours,svf,df,rad-df,rad-pit)
//   RVT_DAYLIGHT_RUN_S     the longest a run may take, seconds (default 1800); a longer one is a failure
//   RVT_DAYLIGHT_QUERY     added to the page's address (for example daylight-workers=4)
//   RVT_DAYLIGHT_SAMPLES_DIR  folders (separated by ';') whose .rvt files are all run, at RVT_DAYLIGHT_SPACING
//                          (default 1 m), in name order; RVT_DAYLIGHT_SHARD=k/n runs every n-th from the k-th (1-based)

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const demos = path.resolve(__dirname, '../public/demos');
const shots = path.resolve(__dirname, '../daylight-shots');
const weatherFile = path.resolve(__dirname, 'data/pvgis-tmy-demo-building-june.json');

const fromDirs = (process.env.RVT_DAYLIGHT_SAMPLES_DIR ?? '')
  .split(';')
  .filter(Boolean)
  .flatMap((d) => fs.readdirSync(d).filter((f) => /\.rvt$/i.test(f)).map((f) => path.join(d, f)))
  .sort()
  .filter((_, i) => {
    const [k, n] = (process.env.RVT_DAYLIGHT_SHARD ?? '1/1').split('/').map(Number);
    return i % n === k - 1;
  })
  .map((f) => `${f}|${process.env.RVT_DAYLIGHT_SPACING ?? '1'}`);
const fromEnv = [...(process.env.RVT_DAYLIGHT_SAMPLES ?? '').split(/[;\n]/).map((s) => s.trim()).filter(Boolean), ...fromDirs];
const defaults = [`${demos}/cc0-2027-demo-building.rvt`, `${demos}/2024_Core_Interior.rvt`, ...(process.env.RVT_SHEETS_MODEL ? [`${process.env.RVT_SHEETS_MODEL}|2`] : [])];
// the default models only when no sample source is set at all (an empty shard runs nothing)
const configured = !!(process.env.RVT_DAYLIGHT_SAMPLES || process.env.RVT_DAYLIGHT_SAMPLES_DIR);
const samples = (configured ? fromEnv : defaults)
  .map((s) => {
    const [name, spacing] = s.split('|');
    const optional = name.startsWith('?');
    return { file: optional ? name.slice(1) : name, spacing: spacing ? Number(spacing) : 0.5, optional };
  })
  // a sample named in RVT_DAYLIGHT_SAMPLES must be there unless marked optional; a default one not staged is left out
  .filter((s) => (configured && !s.optional) || fs.existsSync(s.file));
const analyses = (process.env.RVT_DAYLIGHT_ANALYSES ?? 'sunhours,svf,df,rad-df,rad-pit').split(',').map((s) => s.trim());
const runS = Number(process.env.RVT_DAYLIGHT_RUN_S) || 1800;
const radiance = fs.existsSync(path.resolve(__dirname, '../public/radiance/rtrace.wasm'));

interface Row {
  model: string;
  spacing: number;
  analysis: string;
  sensors: number | null;
  levels: number | null;
  state: string;
  seconds: number;
  barMoved: boolean;
  stages: string;
  mean: number | null;
  reason: string;
}

async function runOne(page: Page, analysis: string): Promise<Omit<Row, 'model' | 'spacing' | 'analysis'>> {
  const panel = page.locator('#daylight-panel');
  await page.locator('#daylight-analysis').selectOption(analysis);
  if (analysis === 'rad-pit' && !(await panel.getAttribute('data-weather'))) {
    await page.locator('#daylight-weather-file').setInputFiles(weatherFile);
    await expect(panel).toHaveAttribute('data-weather', /\d+/);
    await page.locator('#daylight-date').fill('2026-06-21');
    await page.locator('#daylight-time').evaluate((e: HTMLInputElement) => {
      e.value = '12';
      e.dispatchEvent(new Event('input', { bubbles: true }));
    });
  }
  const t0 = Date.now();
  await panel.evaluate((e) => e.removeAttribute('data-state'));
  await page.locator('#daylight-run').click();
  await expect(panel).toHaveAttribute('data-state', /^(running|ask|done|error)$/, { timeout: 600_000 });
  let barMoved = false;
  let state = '';
  let said = 0;
  while (Date.now() - t0 < runS * 1000) {
    // a line a minute: where a long run is
    if (Date.now() - said > 60_000) {
      said = Date.now();
      const at = await panel.evaluate((e) => ({ stage: e.dataset.stage, done: e.dataset.done, total: e.dataset.total, status: document.getElementById('daylight-status')?.textContent, workers: document.getElementById('daylight-workers')?.textContent, live: document.getElementById('daylight-live')?.textContent }));
      console.log(`PROGRESS ${Math.round((Date.now() - t0) / 1000)} s ${JSON.stringify(at)}`);
    }
    state = (await panel.getAttribute('data-state')) ?? '';
    const done = Number(await panel.getAttribute('data-done'));
    const total = Number(await panel.getAttribute('data-total'));
    const stage = (await panel.getAttribute('data-stage')) ?? '';
    // a large run asks first, once the scene is built; the samples run in full
    if (state === 'ask') {
      await page.locator('#daylight-run').click();
      await expect(panel).not.toHaveAttribute('data-state', 'ask');
      continue;
    }
    if ((stage === 'trace' || stage === 'rtrace') && done > 0 && done < total) barMoved = true;
    if (state === 'done' || state === 'error' || state === 'idle') break;
    await page.waitForTimeout(250);
  }
  const seconds = (Date.now() - t0) / 1000;
  if (state !== 'done' && state !== 'error') {
    await page.locator('#daylight-stop').click();
    state = 'timeout';
  }
  // the panel counts the updates it showed mid-trace, which the polling above can miss while the page is busy
  if (Number(await panel.getAttribute('data-mid-updates')) > 0) barMoved = true;
  const timeline = JSON.parse((await panel.getAttribute('data-timeline')) ?? '[]') as [string, number][];
  const status = (await page.locator('#daylight-status').textContent()) ?? '';
  return {
    sensors: Number(await panel.getAttribute('data-sensors')) || null,
    levels: await page.locator('#daylight-storey option').count().then((n) => (n ? n - 1 : 1)),
    state,
    seconds,
    barMoved,
    stages: timeline.map(([s, t]) => `${s} ${t.toFixed(1)}`).join(', '),
    mean: state === 'done' ? Number(await panel.getAttribute('data-mean')) : null,
    reason: state === 'done' ? '' : state === 'timeout' ? `not done after ${runS} s` : status.replace(/^Daylight run failed\.\s*/, ''),
  };
}

test('every Daylight analysis on real public samples', async ({ page }) => {
  // in CI only the jobs that name their samples run it
  test.skip(!samples.length || (!!process.env.CI && !configured), 'no sample models (RVT_DAYLIGHT_SAMPLES, RVT_DAYLIGHT_SAMPLES_DIR, or the staged demos)');
  test.setTimeout(samples.length * analyses.length * (runS + 120) * 1000 + 600_000);
  fs.mkdirSync(shots, { recursive: true });
  const rows: Row[] = [];
  const missing = samples.filter((s) => !fs.existsSync(s.file)).map((s) => s.file);
  expect(missing, 'samples named in RVT_DAYLIGHT_SAMPLES that are not there').toEqual([]);
  for (const s of samples) {
    const model = path.basename(s.file);
    await page.goto(`/${process.env.RVT_DAYLIGHT_QUERY ? `?${process.env.RVT_DAYLIGHT_QUERY}` : ''}`);
    await page.locator('#file-input').setInputFiles(s.file);
    await expect(page.locator('#status')).toHaveText(/Loaded/, { timeout: 600_000 });
    await page.locator('#daylight-details > summary').click();
    // the grid fields show for the analyses that use a grid
    await page.locator('#daylight-analysis').selectOption('svf');
    await page.locator('#daylight-spacing').fill(String(s.spacing));
    await page.locator('#daylight-spacing').dispatchEvent('change');
    for (const analysis of analyses) {
      if (analysis.startsWith('rad-') && !radiance) continue;
      let r: Omit<Row, 'model' | 'spacing' | 'analysis'>;
      try {
        r = await runOne(page, analysis);
      } catch (e) {
        r = { sensors: null, levels: null, state: 'error', seconds: 0, barMoved: false, stages: '', mean: null, reason: `test: ${e instanceof Error ? e.message.split('\n')[0] : String(e)}` };
      }
      rows.push({ model, spacing: s.spacing, analysis, ...r });
      console.log(JSON.stringify(rows.at(-1)));
      await page.screenshot({ path: path.join(shots, `sample-${model.replace(/\W+/g, '-')}-${analysis}.png`) });
      fs.writeFileSync(path.join(shots, 'samples.json'), `${JSON.stringify(rows, null, 1)}\n`);
    }
  }
  const md = [
    '| Model | Grid (m) | Analysis | Sensors | Levels | Result | Time (s) | Bar moved | Stages (s) | Mean | Reason |',
    '|---|---|---|---|---|---|---|---|---|---|---|',
    ...rows.map((r) =>
      [r.model, r.spacing, r.analysis, r.sensors ?? '–', r.levels ?? '–', r.state === 'done' ? 'passed' : `failed (${r.state})`, r.seconds.toFixed(1), r.barMoved ? 'yes' : 'no', r.stages, r.mean === null ? '–' : r.mean.toFixed(3), r.reason.replaceAll('|', '/')].join(' | ').replace(/^/, '| ').replace(/$/, ' |'),
    ),
  ].join('\n');
  fs.writeFileSync(path.join(shots, 'samples.md'), `${md}\n`);
  console.log(md);
  const failed = rows.filter((r) => r.state !== 'done');
  expect(failed, failed.map((r) => `${r.model} ${r.analysis}: ${r.reason}`).join('\n')).toEqual([]);
  // the bar moves with the sensors on every run whose tracing lasts long enough to report an update before
  // its end (progress comes at most every 250 ms; the test reads it every 250 ms)
  const traceSeconds = (r: Row): number => Number(/(?:^|, )(?:trace|rtrace) ([\d.]+)/.exec(r.stages)?.[1] ?? 0);
  for (const r of rows) if (traceSeconds(r) > 3) expect(r.barMoved, `${r.model} ${r.analysis}: the bar did not move`).toBe(true);
});
