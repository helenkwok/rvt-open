import { expect, test, type Page } from '@playwright/test';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

// Every Daylight analysis on real public samples, not only the demo building: each run must end in
// "done" with a grid, and the progress bar must move while it traces. The table of models, sensors,
// results and times goes to daylight-shots/samples.md (and samples.json).
//
//   RVT_DAYLIGHT_SAMPLES   models, one per line or separated by ';', each `path` or `path|spacing`
//                          (grid spacing in metres; default 0.5). Default: the staged demo building
//                          and Core Interior, and RVT_SHEETS_MODEL (Technical School) at 2 m.
//   RVT_DAYLIGHT_ANALYSES  comma-separated (default sunhours,svf,df,rad-df,rad-pit)
//   RVT_DAYLIGHT_RUN_S     the longest a run may take, seconds (default 1800); a longer one is a failure

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const demos = path.resolve(__dirname, '../public/demos');
const shots = path.resolve(__dirname, '../daylight-shots');
const weatherFile = path.resolve(__dirname, 'data/pvgis-tmy-demo-building-june.json');

const fromEnv = (process.env.RVT_DAYLIGHT_SAMPLES ?? '').split(/[;\n]/).map((s) => s.trim()).filter(Boolean);
const defaults = [`${demos}/cc0-2027-demo-building.rvt`, `${demos}/2024_Core_Interior.rvt`, ...(process.env.RVT_SHEETS_MODEL ? [`${process.env.RVT_SHEETS_MODEL}|2`] : [])];
const samples = (fromEnv.length ? fromEnv : defaults)
  .map((s) => {
    const [file, spacing] = s.split('|');
    return { file, spacing: spacing ? Number(spacing) : 0.5 };
  })
  .filter((s) => fs.existsSync(s.file));
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
  // a large run asks first; the samples run in full
  await expect(panel).toHaveAttribute('data-state', /^(running|ask|done|error)$/, { timeout: 600_000 });
  if ((await panel.getAttribute('data-state')) === 'ask') await page.locator('#daylight-run').click();
  let barMoved = false;
  let state = '';
  while (Date.now() - t0 < runS * 1000) {
    state = (await panel.getAttribute('data-state')) ?? '';
    const done = Number(await panel.getAttribute('data-done'));
    const total = Number(await panel.getAttribute('data-total'));
    const stage = (await panel.getAttribute('data-stage')) ?? '';
    if ((stage === 'trace' || stage === 'rtrace') && done > 0 && done < total) barMoved = true;
    if (state === 'done' || state === 'error' || state === 'idle') break;
    await page.waitForTimeout(250);
  }
  const seconds = (Date.now() - t0) / 1000;
  if (state !== 'done' && state !== 'error') {
    await page.locator('#daylight-stop').click();
    state = 'timeout';
  }
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
  test.skip(!samples.length || (!!process.env.CI && !fromEnv.length), 'no sample models (RVT_DAYLIGHT_SAMPLES, or the staged demos)');
  test.setTimeout(samples.length * analyses.length * (runS + 120) * 1000 + 600_000);
  fs.mkdirSync(shots, { recursive: true });
  const rows: Row[] = [];
  for (const s of samples) {
    const model = path.basename(s.file);
    await page.goto('/');
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
  // the bar moves with the sensors on every run long enough to report one update before the end
  for (const r of rows) if (r.seconds > 5 && (r.analysis.startsWith('rad-') || r.sensors! > 2000)) expect(r.barMoved, `${r.model} ${r.analysis}: the bar did not move`).toBe(true);
});
