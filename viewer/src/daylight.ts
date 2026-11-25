/*
 * The Daylight panel: the instant tier of daylight and sun analysis, in the tab.
 *
 * The open model's scene and sensor grids come from the parse worker
 * (worker.ts, `daylight-prepare`: rvt-wasm's daylightPrepare over the
 * daylight-rt crate); the runs are spread over a pool of workers
 * (daylight-worker.ts), each started from the decoder's compiled wasm.
 *
 * - Sun and shadows: the sun's position for a date and time (Radiance's gensky
 *   formulas) moves the scene's directional light, which casts shadows.
 * - Sun hours over a range of days, sky view factor, and a daylight-factor
 *   preview under the CIE overcast sky, on sensor grids over the rooms (or the
 *   floors), drawn as a coloured grid with a legend, a readout on hover, a
 *   table per room, and JSON / CSV exports.
 *
 * Coordinates: the daylight frame is the model's own, metres, Z up; glTF and the
 * scene are Y up, so a point (x, y, z) is drawn at (x, z, -y).
 */

import * as THREE from 'three';
import DaylightWorker from './daylight-worker.ts?worker&inline';
import type { Bits } from './wasm-build';
import { DETAILS, PVGIS_CREDIT, RADIANCE_BASE, Radiance, parseWeather, pvgisUrl, weatherAt, type Weather } from './radiance';

export interface DaylightHost {
  scene: THREE.Scene;
  renderer: THREE.WebGLRenderer;
  camera: THREE.Camera;
  /** The scene's directional light, which becomes the sun while shadows are on. */
  sun: THREE.DirectionalLight;
  model(): THREE.Object3D | null;
  worker(): Worker | null;
  decoder(): { module: WebAssembly.Module; bits: Bits } | null;
  stem(): string;
  requestRender(): void;
  toast(message: string, kind?: 'info' | 'ok' | 'bad'): void;
  download(filename: string, blob: Blob): void;
}

interface Material {
  id: string;
  label: string;
  kind: 'plastic' | 'glass';
  rgb: Float64Array;
  value: number;
  assumed: boolean;
}

interface Zone {
  kind: 'room' | 'floor';
  name: string;
  number?: string;
  roomName?: string;
  level?: string;
  elementId?: number;
  floorZ: number;
}

interface FileSite {
  latitude: number;
  longitude: number;
  elevation: number;
  timeZone: number;
  trueNorth: number;
  locationName: string;
}

interface Prepared {
  positions: Float64Array;
  modifier: Uint32Array;
  materials: Material[];
  sensors: Float64Array;
  sensorZone: Uint32Array;
  zones: Zone[];
  byCategory: [string, number][];
  site: FileSite | null;
}

type Analysis = 'sun' | 'sunhours' | 'svf' | 'df' | 'rad-df' | 'rad-pit';
type SiteKey = 'latitude' | 'longitude' | 'elevation' | 'timeZone' | 'trueNorth';
type Source = 'file' | 'user' | 'assumed';

const ANALYSES: Record<Analysis, { label: string; unit: string }> = {
  sun: { label: 'Sun position and shadows', unit: '' },
  sunhours: { label: 'Sun hours', unit: 'h' },
  svf: { label: 'Sky view factor', unit: '' },
  df: { label: 'Daylight factor (preview, CIE overcast)', unit: '%' },
  'rad-df': { label: 'Daylight factor (Radiance, CIE overcast)', unit: '%' },
  'rad-pit': { label: 'Illuminance at a time (Radiance, weather file)', unit: 'lux' },
};

const SITE_FIELDS: { key: SiteKey; label: string; unit: string; step: string }[] = [
  { key: 'latitude', label: 'Latitude', unit: '° N', step: '0.0001' },
  { key: 'longitude', label: 'Longitude', unit: '° E', step: '0.0001' },
  { key: 'elevation', label: 'Elevation', unit: 'm', step: '0.1' },
  { key: 'timeZone', label: 'Time zone', unit: 'h from UTC', step: '0.25' },
  { key: 'trueNorth', label: 'Angle to true north', unit: '°', step: '0.1' },
];

/** What a run cannot show, said in the panel and written into every export. */
export const DAYLIGHT_LIMITS = [
  'Instant tier: a preview for design checks, not a compliance calculation.',
  'Daylight factor: Monte Carlo paths without an irradiance cache or sampling toward windows, so each sensor is noisier than Radiance at the same time (about ±0.13 DF points per sensor on the reference building at 4,096 paths, mean within 0.4 %).',
  'Materials: reflectance is the viewer colour of each material (kept between 0.02 and 0.85); glass transmittance is one minus the material’s opacity. Materials marked assumed have a colour rvt-open chose, not the file’s.',
  'No surroundings: neighbouring buildings, trees and terrain are not in the scene.',
  'Sun: positions from Radiance’s gensky formulas for a non-leap year, in local standard time (no daylight saving); the sun is a point, not a disc.',
  'Elevation is shown but not used: no sky generator takes it.',
  'Radiance tier: Radiance 6.0.3 compiled to WebAssembly, on the same scene and sensors. Its Monte Carlo sampling differs from a native build’s random numbers, so a run differs from native Radiance as much as two native runs do (RMSE 0.035 DF points on the reference building, native against native 0.036). Each worker builds its own irradiance cache for its share of the sensors.',
  'Weather: the illuminance at a time uses the hour of the weather file nearest the time asked (PVGIS hours are moved from UTC to the site’s standard time); a Perez sky from that hour’s direct normal and diffuse horizontal irradiance.',
  'Sensors: a grid over each room’s floor, or over each floor’s top face kept 0.35 m from its edge where the model has no rooms.',
];

/** Sequential ramp (low to high), the legend's and the grid's colours. */
const RAMP = ['#30123b', '#4662d7', '#1ac7c2', '#a4fc3c', '#fbb938', '#d23105'];

function rampColour(t: number, out: THREE.Color): THREE.Color {
  const x = Math.min(1, Math.max(0, Number.isFinite(t) ? t : 0)) * (RAMP.length - 1);
  const i = Math.min(RAMP.length - 2, Math.floor(x));
  return out.set(RAMP[i]).lerp(new THREE.Color(RAMP[i + 1]), x - i);
}

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  attrs: Record<string, string> = {},
  text?: string,
): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, v);
  if (text !== undefined) e.textContent = text;
  return e;
}

/** Day of a non-leap year from an `<input type="date">` value (29 February counts as the 28th). */
export function dayOfYear(value: string): number {
  const m = /^\d{4}-(\d{2})-(\d{2})$/.exec(value);
  if (!m) return 80;
  const month = Math.min(12, Math.max(1, Number(m[1])));
  const start = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
  const len = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
  return start[month - 1] + Math.min(len[month - 1], Math.max(1, Number(m[2])));
}

function fmt(v: number, digits = 2): string {
  return Number.isFinite(v) ? v.toFixed(digits) : '–';
}

function hhmm(hour: number): string {
  const m = Math.round(hour * 60);
  return `${String(Math.floor(m / 60) % 24).padStart(2, '0')}:${String(m % 60).padStart(2, '0')}`;
}

/** The pool: one worker per core up to eight, each holding the scene's tracer. */
class Pool {
  private workers: Worker[] = [];
  private loaded = -1;
  private req = 0;
  private pending = new Map<number, (v: unknown) => void>();
  constructor(private size: number) {}

  private spawn(): void {
    if (this.workers.length) return;
    for (let i = 0; i < this.size; i++) {
      const w = new DaylightWorker({ name: `daylight-${i}` }) as Worker;
      w.addEventListener('message', (ev: MessageEvent) => {
        const m = ev.data as { type: string; req?: number; value?: unknown; message?: string };
        if ((m.type === 'sun' || m.type === 'suns' || m.type === 'radtext' || m.type === 'error') && m.req !== undefined) {
          const done = this.pending.get(m.req);
          this.pending.delete(m.req);
          done?.(m.type === 'error' ? new Error(m.message) : m.value);
        }
      });
      this.workers.push(w);
    }
  }

  get count(): number {
    return this.size;
  }

  /** Stops every worker; the next call starts new ones. */
  stop(): void {
    for (const w of this.workers) w.terminate();
    this.workers = [];
    this.loaded = -1;
    for (const done of this.pending.values()) done(new Error('stopped'));
    this.pending.clear();
  }

  /** A sun query answered by the first worker (it needs the decoder, not a scene). */
  ask<T>(decoder: { module: WebAssembly.Module; bits: Bits }, msg: Record<string, unknown>): Promise<T> {
    this.spawn();
    const req = ++this.req;
    return new Promise<T>((resolve, reject) => {
      this.pending.set(req, (v) => (v instanceof Error ? reject(v) : resolve(v as T)));
      const w = this.workers[0];
      if (this.loaded < 0) w.postMessage({ type: 'load', module: decoder.module, bits: decoder.bits, id: 0, positions: new Float64Array(0), modifier: new Uint32Array(0), materials: new Float64Array(0) });
      w.postMessage({ ...msg, req });
    });
  }

  private async load(id: number, p: Prepared, decoder: { module: WebAssembly.Module; bits: Bits }): Promise<void> {
    if (this.loaded === id) return;
    this.spawn();
    const materials = new Float64Array(p.materials.length * 4);
    p.materials.forEach((m, i) => materials.set([m.kind === 'glass' ? 1 : 0, m.rgb[0], m.rgb[1], m.rgb[2]], i * 4));
    await Promise.all(
      this.workers.map(
        (w) =>
          new Promise<void>((resolve, reject) => {
            const on = (ev: MessageEvent): void => {
              const m = ev.data as { type: string; id?: number; message?: string };
              if (m.type === 'loaded' && m.id === id) {
                w.removeEventListener('message', on);
                resolve();
              } else if (m.type === 'error' && m.id === undefined) {
                w.removeEventListener('message', on);
                reject(new Error(m.message));
              }
            };
            w.addEventListener('message', on);
            w.postMessage({ type: 'load', module: decoder.module, bits: decoder.bits, id, positions: p.positions, modifier: p.modifier, materials });
          }),
      ),
    );
    this.loaded = id;
  }

  /** Runs `kind` over every sensor in contiguous shards, the workers taking the next shard as they finish. */
  async run(
    id: number,
    p: Prepared,
    decoder: { module: WebAssembly.Module; bits: Bits },
    kind: string,
    params: Float64Array,
    progress: (done: number, total: number) => void,
  ): Promise<Float64Array> {
    await this.load(id, p, decoder);
    const n = p.sensors.length / 6;
    const out = new Float64Array(n);
    const shard = Math.min(512, Math.max(8, Math.ceil(n / (this.size * 8))));
    let next = 0;
    let done = 0;
    let job = 0;
    progress(0, n);
    await Promise.all(
      this.workers.map(
        (w) =>
          new Promise<void>((resolve, reject) => {
            const take = (): void => {
              if (next >= n) {
                w.removeEventListener('message', on);
                resolve();
                return;
              }
              const first = next;
              next = Math.min(n, next + shard);
              w.postMessage({ type: 'run', id, job: ++job, sensors: p.sensors.slice(first * 6, next * 6), first, kind, params });
            };
            const on = (ev: MessageEvent): void => {
              const m = ev.data as { type: string; first?: number; values?: Float64Array; message?: string; job?: number };
              if (m.type === 'done' && m.values && m.first !== undefined) {
                out.set(m.values, m.first);
                done += m.values.length;
                progress(done, n);
                take();
              } else if (m.type === 'error' && m.job !== undefined) {
                w.removeEventListener('message', on);
                reject(new Error(m.message));
              }
            };
            w.addEventListener('message', on);
            take();
          }),
      ),
    );
    return out;
  }
}

export function initDaylight(host: DaylightHost, root: HTMLElement): { modelChanged(): void; reset(): void } {
  // `?daylight-workers=N` sets the pool's size (for measuring how the runs scale); else one per core, up to eight
  const asked = Number(new URLSearchParams(location.search).get('daylight-workers'));
  const pool = new Pool(asked >= 1 ? Math.min(32, Math.floor(asked)) : Math.min(8, Math.max(1, (navigator.hardwareConcurrency || 4) - 1)));
  let prepared: Prepared | null = null;
  let preparedKey = '';
  let preparedId = 0;
  let preparing: Promise<Prepared> | null = null;
  let req = 0;
  let values: Float64Array | null = null;
  let lastRun: { analysis: Analysis; params: Record<string, unknown>; ms: number; range: [number, number] } | null = null;
  let overlay: THREE.InstancedMesh | null = null;
  const radiance = new Radiance(pool.count);
  let weather: Weather | null = null;
  const helpers = new THREE.Group();
  helpers.name = 'daylight-helpers';
  host.scene.add(helpers);
  const site: Record<SiteKey, number> = { latitude: 0, longitude: 0, elevation: 0, timeZone: 0, trueNorth: 0 };
  const source: Record<SiteKey, Source> = { latitude: 'assumed', longitude: 'assumed', elevation: 'assumed', timeZone: 'assumed', trueNorth: 'assumed' };
  const sunHome = { position: host.sun.position.clone(), intensity: host.sun.intensity };

  // ---------- the panel ----------
  root.replaceChildren();
  root.dataset.state = 'idle';
  const intro = el('p', { class: 'option-hint' }, 'Sun, shadows, sun hours, sky view and a daylight-factor preview, computed in this tab. Nothing is uploaded.');
  const siteBox = el('fieldset', { class: 'daylight-site' });
  siteBox.append(el('legend', {}, 'Site'));
  const inputs = {} as Record<SiteKey, HTMLInputElement>;
  const tags = {} as Record<SiteKey, HTMLElement>;
  for (const f of SITE_FIELDS) {
    const row = el('label', { class: 'daylight-row' });
    const input = el('input', { type: 'number', step: f.step, id: `daylight-${f.key}`, 'aria-label': `${f.label} (${f.unit})` });
    input.addEventListener('change', () => {
      const v = Number(input.value);
      if (!Number.isFinite(v)) return;
      site[f.key] = v;
      source[f.key] = 'user';
      showSite();
      if (analysisSel.value === 'sun' || shadowsBox.checked) void updateSun();
      if (pathBox.checked) void drawSunPath();
    });
    const tag = el('span', { class: 'daylight-source' });
    row.append(el('span', {}, f.label), input, el('span', { class: 'daylight-unit' }, f.unit), tag);
    inputs[f.key] = input;
    tags[f.key] = tag;
    siteBox.append(row);
  }
  const siteNote = el('p', { class: 'option-hint', id: 'daylight-site-note' });
  const siteReset = el('button', { type: 'button', class: 'side-btn' }, 'Use the file’s site');
  siteReset.addEventListener('click', () => {
    applyFileSite(prepared?.site ?? null);
    void updateSun();
    if (pathBox.checked) void drawSunPath();
  });
  siteBox.append(siteNote, siteReset);

  const analysisSel = el('select', { class: 'mode-select', id: 'daylight-analysis', 'aria-label': 'Analysis' });
  for (const [k, a] of Object.entries(ANALYSES)) analysisSel.append(el('option', { value: k }, a.label));
  const dateIn = el('input', { type: 'date', id: 'daylight-date', value: '2026-03-21', 'aria-label': 'Date' });
  const timeIn = el('input', { type: 'range', id: 'daylight-time', min: '0', max: '24', step: '0.25', value: '12', 'aria-label': 'Time (standard time)' });
  const timeOut = el('output', { for: 'daylight-time' }, '12:00');
  const toIn = el('input', { type: 'date', id: 'daylight-date-to', value: '2026-03-21', 'aria-label': 'Last day' });
  const stepSel = el('select', { class: 'mode-select', id: 'daylight-step', 'aria-label': 'Time step' });
  for (const m of [5, 15, 30, 60]) stepSel.append(el('option', { value: String(m), ...(m === 15 ? { selected: '' } : {}) }, `every ${m} min`));
  const pathsSel = el('select', { class: 'mode-select', id: 'daylight-paths', 'aria-label': 'Paths per sensor' });
  for (const n of [1024, 4096, 16384]) pathsSel.append(el('option', { value: String(n), ...(n === 4096 ? { selected: '' } : {}) }, `${n.toLocaleString('en')} paths per sensor`));
  const spacingIn = el('input', { type: 'number', step: '0.1', min: '0.1', value: '0.5', id: 'daylight-spacing', 'aria-label': 'Grid spacing (m)' });
  const offsetIn = el('input', { type: 'number', step: '0.05', value: '0.76', id: 'daylight-offset', 'aria-label': 'Sensor height above the floor (m)' });
  const detailSel = el('select', { class: 'mode-select', id: 'daylight-detail', 'aria-label': 'Radiance parameters' });
  for (const [k, d] of Object.entries(DETAILS)) detailSel.append(el('option', { value: k }, d.label));
  const weatherBox = el('div', { class: 'daylight-weather', id: 'daylight-weather' });
  const pvgisLink = el('a', { id: 'daylight-pvgis', target: '_blank', rel: 'noopener noreferrer' }, 'Get the typical year from PVGIS (JSON)');
  const weatherIn = el('input', { type: 'file', id: 'daylight-weather-file', accept: '.json,.epw', 'aria-label': 'Weather file (PVGIS JSON or EPW)' });
  const weatherNote = el('p', { class: 'option-hint', id: 'daylight-weather-note' }, 'No weather file loaded. PVGIS does not answer requests from a web page, so the link opens its official service in a new tab: save the file it returns and choose it here. An EPW file of your own works too.');
  pvgisLink.title = `Weather data from PVGIS: ${PVGIS_CREDIT}`;
  weatherBox.append(pvgisLink, weatherIn, weatherNote);
  const radNote = el('p', { class: 'option-hint', id: 'daylight-radiance-note' });
  radNote.append(
    document.createTextNode('Radiance 6.0.3 (WebAssembly), loaded when a Radiance analysis runs. Radiance Software License 2.0: '),
    el('a', { href: `${RADIANCE_BASE}License.txt`, target: '_blank', rel: 'noopener' }, 'License.txt'),
    document.createTextNode(' · '),
    el('a', { href: `${RADIANCE_BASE}SOURCE.md`, target: '_blank', rel: 'noopener' }, 'build and source'),
    document.createTextNode('.'),
  );
  const matsBox = el('details', { class: 'daylight-limits', id: 'daylight-materials' });
  const shadowsBox = el('input', { type: 'checkbox', id: 'daylight-shadows' });
  const pathBox = el('input', { type: 'checkbox', id: 'daylight-sunpath' });
  const runBtn = el('button', { type: 'button', class: 'side-btn', id: 'daylight-run' }, 'Run');
  const stopBtn = el('button', { type: 'button', class: 'side-btn', id: 'daylight-stop', hidden: '' }, 'Stop');
  const bar = el('progress', { id: 'daylight-progress', max: '1', value: '0', hidden: '' });
  const status = el('p', { class: 'option-hint', id: 'daylight-status', 'aria-live': 'polite' });

  const row = (label: string, ...kids: HTMLElement[]): HTMLElement => {
    const r = el('label', { class: 'daylight-row' });
    r.append(el('span', {}, label), ...kids);
    return r;
  };
  const whenBox = el('div', { class: 'daylight-when' });
  const timeRow = row('Time', timeIn, timeOut);
  const toRow = row('To', toIn);
  const stepRow = row('Step', stepSel);
  const pathsRow = row('Quality', pathsSel);
  const detailRow = row('Parameters', detailSel);
  whenBox.append(row('Date', dateIn), timeRow, toRow, stepRow, pathsRow, detailRow);
  const gridBox = el('div', { class: 'daylight-grid' });
  gridBox.append(row('Grid spacing (m)', spacingIn), row('Sensor height (m)', offsetIn));
  const toggles = el('div', { class: 'daylight-toggles' });
  const t1 = el('label', {});
  t1.append(shadowsBox, document.createTextNode(' Shadows'));
  const cutBox = el('input', { type: 'checkbox', id: 'daylight-cut', checked: '' });
  const cutIn = el('input', { type: 'number', step: '0.1', id: 'daylight-cut-height', 'aria-label': 'Cut height (m, the model’s own elevation)' });
  const cutRow = el('label', { class: 'daylight-row' });
  cutRow.append(el('span', {}, 'Cut the model at'), cutIn, el('span', { class: 'daylight-unit' }, 'm'));
  const t3 = el('label', {});
  t3.append(cutBox, document.createTextNode(' Cut the model above the grid'));
  const t2 = el('label', {});
  t2.append(pathBox, document.createTextNode(' Sun path'));
  toggles.append(t1, t2);
  const cutBoxes = el('div', { class: 'daylight-cut' });
  cutBoxes.append(t3, cutRow);

  const legend = el('div', { class: 'daylight-legend', id: 'daylight-legend', hidden: '' });
  const readout = el('p', { class: 'option-hint', id: 'daylight-readout', 'aria-live': 'polite' });
  const table = el('table', { class: 'daylight-table', id: 'daylight-table', hidden: '' });
  const exportJson = el('button', { type: 'button', class: 'side-btn', id: 'daylight-export-json', disabled: '' }, 'Export results (JSON)');
  const exportCsv = el('button', { type: 'button', class: 'side-btn', id: 'daylight-export-csv', disabled: '' }, 'Export results (CSV)');
  const limits = el('details', { class: 'daylight-limits' });
  limits.append(el('summary', {}, 'Known limits'));
  const ul = el('ul');
  for (const l of DAYLIGHT_LIMITS) ul.append(el('li', {}, l));
  limits.append(ul);

  root.append(intro, siteBox, row('Analysis', analysisSel), whenBox, weatherBox, radNote, gridBox, toggles, cutBoxes, runBtn, stopBtn, bar, status, legend, readout, table, exportJson, exportCsv, matsBox, limits);

  const showFields = (): void => {
    const a = analysisSel.value as Analysis;
    timeRow.hidden = a !== 'sun' && a !== 'rad-pit';
    toRow.hidden = stepRow.hidden = a !== 'sunhours';
    pathsRow.hidden = a !== 'svf' && a !== 'df';
    detailRow.hidden = radNote.hidden = !a.startsWith('rad-');
    weatherBox.hidden = a !== 'rad-pit';
    gridBox.hidden = a === 'sun';
    runBtn.textContent = a === 'sun' ? 'Show the sun' : 'Run';
  };
  analysisSel.addEventListener('change', showFields);
  showFields();

  function showSite(): void {
    for (const f of SITE_FIELDS) {
      inputs[f.key].value = String(Number(site[f.key].toFixed(6)));
      const s = source[f.key];
      tags[f.key].textContent = s === 'file' ? 'from the file' : s === 'user' ? 'set by you' : 'assumed';
      tags[f.key].dataset.source = s;
    }
    const notes: string[] = [];
    if (source.latitude === 'assumed') notes.push('The file’s site location was not read: latitude, longitude and time zone are assumed 0. Set them before trusting a sun result.');
    if (Math.abs(site.latitude - 42.3587) < 0.01 && Math.abs(site.longitude + 71.0567) < 0.01 && source.latitude === 'file') {
      notes.push('This is the location new Revit projects start with (Boston, MA): check that the author set the real site.');
    }
    notes.push('Times are local standard time; daylight saving is not applied.');
    pvgisLink.href = pvgisUrl(site.latitude, site.longitude);
    siteNote.textContent = notes.join(' ');
  }

  function applyFileSite(s: FileSite | null): void {
    for (const f of SITE_FIELDS) {
      site[f.key] = s ? s[f.key] : 0;
      source[f.key] = s ? 'file' : 'assumed';
    }
    showSite();
  }
  applyFileSite(null);

  timeIn.addEventListener('input', () => {
    timeOut.textContent = hhmm(Number(timeIn.value));
    if (analysisSel.value === 'sun' || shadowsBox.checked) void updateSun();
  });
  dateIn.addEventListener('change', () => {
    if (analysisSel.value === 'sun' || shadowsBox.checked) void updateSun();
  });
  shadowsBox.addEventListener('change', () => {
    if (shadowsBox.checked) void updateSun();
    else sunOff();
  });
  pathBox.addEventListener('change', () => {
    if (pathBox.checked) void drawSunPath();
    else clearHelpers('path');
  });

  weatherIn.addEventListener('change', async () => {
    const f = weatherIn.files?.[0];
    if (!f) return;
    try {
      weather = parseWeather(f.name, await f.text(), site.timeZone, new Date(f.lastModified).toISOString().slice(0, 10));
      const pv = weather.provenance;
      weatherNote.textContent = `${f.name}: ${weather.hours.length.toLocaleString('en')} hours. ${Object.entries(pv)
        .filter(([k]) => k !== 'file')
        .map(([k, v]) => `${k} ${v}`)
        .join('; ')}.${weather.credit ? ` ${weather.credit}.` : ''}`;
      root.dataset.weather = String(weather.hours.length);
    } catch (e) {
      weather = null;
      delete root.dataset.weather;
      weatherNote.textContent = e instanceof Error ? e.message : String(e);
    }
  });

  function showMaterials(p: Prepared): void {
    matsBox.replaceChildren(el('summary', {}, `Materials (${p.materials.length}; ${p.materials.filter((m) => m.assumed).length} assumed)`));
    const t = el('table', { class: 'daylight-table' });
    const head = el('tr');
    for (const h of ['Material', 'Kind', 'Value', 'Source']) head.append(el('th', { scope: 'col' }, h));
    const body = el('tbody');
    for (const m of p.materials) {
      const r = el('tr');
      for (const c of [m.label, m.kind === 'glass' ? 'glass (transmittance)' : 'reflectance', fmt(m.value, 3), m.assumed ? 'assumed (placeholder)' : 'the file’s colour']) r.append(el('td', {}, c));
      if (m.assumed) r.dataset.assumed = '';
      body.append(r);
    }
    const thead = el('thead');
    thead.append(head);
    t.append(thead, body);
    matsBox.append(t);
  }

  // ---------- the scene's frame ----------
  const toScene = (x: number, y: number, z: number): THREE.Vector3 => new THREE.Vector3(x, z, -y);
  const modelBox = (): THREE.Box3 => {
    const m = host.model();
    return m ? new THREE.Box3().setFromObject(m) : new THREE.Box3(new THREE.Vector3(-10, 0, -10), new THREE.Vector3(10, 10, 10));
  };
  const siteArg = (): Record<string, number> => ({ ...site });

  function clearHelpers(kind: string): void {
    for (const c of [...helpers.children]) {
      if (c.userData.kind !== kind) continue;
      helpers.remove(c);
      c.traverse((o) => {
        const mesh = o as THREE.Mesh;
        mesh.geometry?.dispose();
        const mat = mesh.material as THREE.Material | undefined;
        mat?.dispose();
      });
    }
    host.requestRender();
  }

  // ---------- sun, shadows, sun path ----------
  let sunReq = 0;
  async function updateSun(): Promise<void> {
    const decoder = host.decoder();
    if (!decoder) return;
    const mine = ++sunReq;
    const v = await pool.ask<number[]>(decoder, { type: 'sun', site: siteArg(), day: dayOfYear(dateIn.value), hour: Number(timeIn.value) });
    if (mine !== sunReq) return;
    const [x, y, z, alt, azi] = v;
    const box = modelBox();
    const centre = box.getCenter(new THREE.Vector3());
    const radius = Math.max(5, box.getBoundingSphere(new THREE.Sphere()).radius);
    const dir = toScene(x, y, z).normalize();
    root.dataset.sun = JSON.stringify({ x, y, z, altitude: alt, azimuth: azi });
    status.textContent = alt > 0
      ? `Sun at ${fmt(alt, 1)}° altitude, ${fmt(azi, 1)}° azimuth (clockwise from true north), ${hhmm(Number(timeIn.value))} standard time.`
      : `The sun is below the horizon (${fmt(alt, 1)}°) at ${hhmm(Number(timeIn.value))}.`;
    const r = host.renderer;
    r.shadowMap.enabled = true;
    r.shadowMap.type = THREE.PCFShadowMap;
    const s = host.sun;
    s.position.copy(centre).addScaledVector(dir, radius * 2);
    s.target.position.copy(centre);
    if (!s.target.parent) host.scene.add(s.target);
    s.intensity = alt > 0 ? 2.4 : 0;
    s.castShadow = alt > 0;
    const cam = s.shadow.camera;
    cam.left = cam.bottom = -radius * 1.2;
    cam.right = cam.top = radius * 1.2;
    cam.near = radius * 0.1;
    cam.far = radius * 4;
    cam.updateProjectionMatrix();
    s.shadow.mapSize.set(2048, 2048);
    s.shadow.bias = -0.0005;
    s.shadow.normalBias = 0.02;
    host.model()?.traverse((o) => {
      if ((o as THREE.Mesh).isMesh) {
        o.castShadow = true;
        o.receiveShadow = true;
      }
    });
    clearHelpers('ground');
    const ground = new THREE.Mesh(new THREE.PlaneGeometry(radius * 6, radius * 6), new THREE.ShadowMaterial({ opacity: 0.28 }));
    ground.rotation.x = -Math.PI / 2;
    ground.position.set(centre.x, box.min.y - 0.01, centre.z);
    ground.receiveShadow = true;
    ground.userData.kind = 'ground';
    helpers.add(ground);
    clearHelpers('sun');
    if (alt > 0) {
      const ball = new THREE.Mesh(new THREE.SphereGeometry(radius * 0.04, 16, 12), new THREE.MeshBasicMaterial({ color: 0xf5a623 }));
      ball.position.copy(new THREE.Vector3(centre.x, box.min.y, centre.z)).addScaledVector(dir, radius * 1.5);
      ball.userData.kind = 'sun';
      helpers.add(ball);
    }
    root.dataset.shadows = 'on';
    shadowsBox.checked = true;
    host.requestRender();
  }

  function sunOff(): void {
    sunReq++;
    const s = host.sun;
    s.castShadow = false;
    s.target.position.set(0, 0, 0);
    s.position.copy(sunHome.position);
    s.intensity = sunHome.intensity;
    host.renderer.shadowMap.enabled = false;
    clearHelpers('ground');
    clearHelpers('sun');
    delete root.dataset.shadows;
    host.requestRender();
  }

  async function drawSunPath(): Promise<void> {
    const decoder = host.decoder();
    if (!decoder) return;
    clearHelpers('path');
    const box = modelBox();
    const centre = new THREE.Vector3(box.getCenter(new THREE.Vector3()).x, box.min.y, box.getCenter(new THREE.Vector3()).z);
    const radius = Math.max(5, box.getBoundingSphere(new THREE.Sphere()).radius) * 1.5;
    const group = new THREE.Group();
    group.userData.kind = 'path';
    const lineMat = new THREE.LineBasicMaterial({ color: 0xf5a623, transparent: true, opacity: 0.7 });
    for (let month = 1; month <= 12; month++) {
      const doy = dayOfYear(`2026-${String(month).padStart(2, '0')}-21`);
      const v = await pool.ask<Float64Array>(decoder, { type: 'suns', site: siteArg(), from: doy, to: doy, step: 10 });
      const pts: THREE.Vector3[] = [];
      for (let i = 0; i + 2 < v.length; i += 3) pts.push(centre.clone().addScaledVector(toScene(v[i], v[i + 1], v[i + 2]), radius));
      if (pts.length > 1) group.add(new THREE.Line(new THREE.BufferGeometry().setFromPoints(pts), lineMat));
    }
    // the horizon circle and the north mark (true north turned into the model's frame)
    const ring: THREE.Vector3[] = [];
    for (let a = 0; a <= 64; a++) ring.push(centre.clone().add(new THREE.Vector3(Math.cos((a / 32) * Math.PI) * radius, 0, Math.sin((a / 32) * Math.PI) * radius)));
    group.add(new THREE.Line(new THREE.BufferGeometry().setFromPoints(ring), lineMat));
    const n = (site.trueNorth * Math.PI) / 180;
    const north = toScene(-Math.sin(n), Math.cos(n), 0);
    group.add(new THREE.Line(new THREE.BufferGeometry().setFromPoints([centre, centre.clone().addScaledVector(north, radius * 1.1)]), new THREE.LineBasicMaterial({ color: 0xd23105 })));
    helpers.add(group);
    root.dataset.sunPath = 'on';
    host.requestRender();
  }

  // ---------- the scene and grids ----------
  function prepare(): Promise<Prepared> {
    const options = { spacing: Number(spacingIn.value) || 0.5, offset: Number(offsetIn.value) || 0.76, inset: 0.35 };
    const key = JSON.stringify(options);
    if (prepared && preparedKey === key) return Promise.resolve(prepared);
    if (preparing) return preparing;
    const w = host.worker();
    if (!w) return Promise.reject(new Error('no model is open'));
    const mine = ++req;
    root.dataset.state = 'preparing';
    status.textContent = 'Building the scene and the sensor grids…';
    preparing = new Promise<Prepared>((resolve, reject) => {
      const on = (ev: MessageEvent): void => {
        const m = ev.data as { type: string; req?: number; prepared?: Prepared; message?: string };
        if (m.req !== mine || (m.type !== 'daylight-prepared' && m.type !== 'daylight-error')) return;
        w.removeEventListener('message', on);
        preparing = null;
        if (m.type === 'daylight-error' || !m.prepared) {
          reject(new Error(m.message ?? 'no scene'));
          return;
        }
        const firstTime = prepared === null;
        prepared = m.prepared;
        preparedKey = key;
        preparedId++;
        if (firstTime) applyFileSite(prepared.site);
        showMaterials(prepared);
        if (root.dataset.state === 'preparing') {
          root.dataset.state = 'ready';
          status.textContent = `${(prepared.sensors.length / 6).toLocaleString('en')} sensors on ${prepared.zones.length} ${prepared.zones[0]?.kind === 'floor' ? 'floors (the model has no rooms)' : 'rooms'}, ${(prepared.positions.length / 9).toLocaleString('en')} triangles.`;
        }
        resolve(prepared);
      };
      w.addEventListener('message', on);
      w.postMessage({ type: 'daylight-prepare', options, req: mine });
    });
    return preparing;
  }

  // The cut: the model's meshes are clipped above a height (scene y), so the grid inside shows.
  const cutPlane = new THREE.Plane(new THREE.Vector3(0, -1, 0), 0);
  function setCut(on: boolean): void {
    host.renderer.localClippingEnabled = on;
    host.model()?.traverse((o) => {
      const mats = (o as THREE.Mesh).material;
      for (const m of Array.isArray(mats) ? mats : mats ? [mats] : []) {
        m.clippingPlanes = on ? [cutPlane] : null;
        m.needsUpdate = true;
      }
    });
    if (on) root.dataset.cut = String(cutPlane.constant);
    else delete root.dataset.cut;
    host.requestRender();
  }
  cutBox.addEventListener('change', () => setCut(cutBox.checked && overlay !== null));
  cutIn.addEventListener('change', () => {
    const v = Number(cutIn.value);
    if (!Number.isFinite(v)) return;
    cutPlane.constant = v;
    setCut(cutBox.checked && overlay !== null);
  });

  function clearOverlay(): void {
    if (overlay) setCut(false);
    if (overlay) {
      helpers.remove(overlay);
      overlay.geometry.dispose();
      (overlay.material as THREE.Material).dispose();
      overlay = null;
    }
    delete root.dataset.overlay;
  }

  function drawOverlay(p: Prepared, v: Float64Array, lo: number, hi: number): void {
    clearOverlay();
    const n = v.length;
    const size = (Number(spacingIn.value) || 0.5) * 0.92;
    const geo = new THREE.PlaneGeometry(size, size);
    geo.rotateX(-Math.PI / 2);
    const mesh = new THREE.InstancedMesh(geo, new THREE.MeshBasicMaterial({ side: THREE.DoubleSide, toneMapped: false }), n);
    const m = new THREE.Matrix4();
    const c = new THREE.Color();
    for (let i = 0; i < n; i++) {
      const s = p.sensors.subarray(i * 6, i * 6 + 3);
      const at = toScene(s[0], s[1], s[2]);
      m.makeTranslation(at.x, at.y, at.z);
      mesh.setMatrixAt(i, m);
      mesh.setColorAt(i, rampColour(hi > lo ? (v[i] - lo) / (hi - lo) : 0.5, c));
    }
    mesh.instanceMatrix.needsUpdate = true;
    if (mesh.instanceColor) mesh.instanceColor.needsUpdate = true;
    mesh.name = 'daylight-grid';
    mesh.userData.kind = 'overlay';
    overlay = mesh;
    helpers.add(mesh);
    root.dataset.overlay = String(n);
    // cut half a storey above the highest sensors, unless a height was set
    let top = -Infinity;
    for (let i = 0; i < n; i++) top = Math.max(top, p.sensors[i * 6 + 2]);
    if (cutIn.value === '') cutIn.value = (top + 0.6).toFixed(2);
    cutPlane.constant = Number(cutIn.value);
    setCut(cutBox.checked);
  }

  function zoneLabel(z: Zone | undefined): string {
    if (!z) return '–';
    if (z.kind === 'room') {
      const parts = [z.number, z.roomName].filter((x) => x && x.length);
      return parts.length ? parts.join(' ') : z.name || 'Room';
    }
    return z.name || 'Floor';
  }

  function showLegend(a: Analysis, lo: number, hi: number): void {
    legend.hidden = false;
    legend.replaceChildren();
    const unit = ANALYSES[a].unit;
    const barEl = el('div', { class: 'daylight-ramp' });
    barEl.style.background = `linear-gradient(to right, ${RAMP.join(', ')})`;
    const labels = el('div', { class: 'daylight-ramp-labels' });
    for (let k = 0; k <= 4; k++) labels.append(el('span', {}, `${fmt(lo + ((hi - lo) * k) / 4, a === 'svf' ? 3 : a === 'sunhours' ? 1 : 2)}${unit ? ` ${unit}` : ''}`));
    legend.append(el('div', { class: 'daylight-legend-title' }, ANALYSES[a].label), barEl, labels);
  }

  function zoneStats(p: Prepared, v: Float64Array): { zone: Zone; label: string; n: number; mean: number; min: number; max: number }[] {
    const acc = p.zones.map((zone) => ({ zone, label: zoneLabel(zone), n: 0, sum: 0, min: Infinity, max: -Infinity }));
    for (let i = 0; i < v.length; i++) {
      const a = acc[p.sensorZone[i]];
      if (!a) continue;
      a.n++;
      a.sum += v[i];
      a.min = Math.min(a.min, v[i]);
      a.max = Math.max(a.max, v[i]);
    }
    return acc.filter((a) => a.n > 0).map((a) => ({ zone: a.zone, label: a.label, n: a.n, mean: a.sum / a.n, min: a.min, max: a.max }));
  }

  function showTable(a: Analysis, p: Prepared, v: Float64Array): void {
    const d = a === 'svf' ? 3 : 2;
    table.hidden = false;
    table.replaceChildren();
    const head = el('tr');
    for (const h of [p.zones[0]?.kind === 'floor' ? 'Floor' : 'Room', 'Level', 'Sensors', 'Mean', 'Min', 'Max', 'Min / mean']) head.append(el('th', { scope: 'col' }, h));
    const thead = el('thead');
    thead.append(head);
    const body = el('tbody');
    for (const s of zoneStats(p, v)) {
      const r = el('tr');
      for (const cell of [s.label, s.zone.level ?? '', String(s.n), fmt(s.mean, d), fmt(s.min, d), fmt(s.max, d), s.mean > 0 ? fmt(s.min / s.mean, 2) : '–']) r.append(el('td', {}, cell));
      body.append(r);
    }
    table.append(el('caption', {}, `${ANALYSES[a].label}${ANALYSES[a].unit ? ` (${ANALYSES[a].unit})` : ''} per ${p.zones[0]?.kind === 'floor' ? 'floor' : 'room'}`), thead, body);
  }

  // ---------- running ----------
  let running = false;
  async function run(): Promise<void> {
    const a = analysisSel.value as Analysis;
    const decoder = host.decoder();
    if (!decoder || !host.worker()) {
      host.toast('Open a model first.', 'info');
      return;
    }
    if (a === 'sun') {
      try {
        await prepare();
      } catch {
        // the sun needs only the site; a model without a scene still shows it
      }
      root.dataset.state = 'done';
      shadowsBox.checked = true;
      await updateSun();
      return;
    }
    if (running) return;
    running = true;
    runBtn.disabled = true;
    stopBtn.hidden = false;
    bar.hidden = false;
    const t0 = performance.now();
    try {
      const p = await prepare();
      const n = p.sensors.length / 6;
      if (n === 0) throw new Error('the model has no rooms or floors to lay a sensor grid on');
      root.dataset.sensors = String(n);
      root.dataset.state = 'running';
      let kind = a as string;
      let params: Float64Array;
      let scale = 1;
      const extra: Record<string, unknown> = {};
      if (a === 'sunhours') {
        const from = dayOfYear(dateIn.value);
        const to = toIn.value ? dayOfYear(toIn.value) : from;
        const step = Number(stepSel.value) || 15;
        params = await pool.ask<Float64Array>(decoder, { type: 'suns', site: siteArg(), from, to, step });
        kind = 'sun';
        scale = step / 60;
        extra.from = dateIn.value.slice(5);
        extra.to = toIn.value.slice(5);
        extra.stepMinutes = step;
        extra.sunPositions = params.length / 3;
      } else if (a === 'svf') {
        params = new Float64Array([Number(pathsSel.value)]);
        extra.paths = Number(pathsSel.value);
      } else if (a === 'df') {
        params = new Float64Array([2, Number(pathsSel.value), 0.2]);
        Object.assign(extra, { ab: 2, paths: Number(pathsSel.value), groundReflectance: 0.2, sky: 'CIE overcast (gensky -c)' });
      } else {
        params = new Float64Array(0);
      }
      const progress = (done: number, total: number): void => {
        bar.value = total ? done / total : 0;
      };
      let v: Float64Array;
      if (a === 'rad-df' || a === 'rad-pit') {
        if (!(await radiance.available())) throw new Error('the Radiance build is not beside this page (radiance/)');
        const detail = DETAILS[detailSel.value as 'preview' | 'validated'];
        let sky: string;
        if (a === 'rad-pit') {
          if (!weather) throw new Error('choose a weather file first (PVGIS JSON or EPW)');
          const doy = dayOfYear(dateIn.value);
          const month = [31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334, 365].findIndex((end) => doy <= end) + 1;
          const day = doy - [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334][month - 1];
          const h = weatherAt(weather, month, day, Number(timeIn.value));
          if (!h) throw new Error(`the weather file has no hour on ${dateIn.value.slice(5)}`);
          sky = await radiance.perezSky(h.month, h.day, h.hour, site, h.dni, h.dhi);
          Object.assign(extra, { weatherHour: { month: h.month, day: h.day, hour: h.hour, dni: h.dni, dhi: h.dhi }, sky: 'Perez (gendaylit -W)', weather: weather.provenance, credit: weather.credit });
        } else {
          sky = await radiance.overcastSky();
          Object.assign(extra, { sky: 'CIE overcast, 10,000 lux horizontal (gensky -ang 45 0 -c -B 55.866)', groundReflectance: 0.2 });
        }
        extra.rtrace = ['-I', '-h', ...detail.args].join(' ');
        status.textContent = `Radiance on ${n.toLocaleString('en')} sensors with ${radiance.count} workers…`;
        const materials = new Float64Array(p.materials.length * 4);
        p.materials.forEach((m, i) => materials.set([m.kind === 'glass' ? 1 : 0, m.rgb[0], m.rgb[1], m.rgb[2]], i * 4));
        const [matText, sceneText] = await pool.ask<string[]>(decoder, { type: 'radtext', positions: p.positions, modifier: p.modifier, materials });
        const pts: string[] = [];
        for (let i = 0; i < n; i++) {
          const q = p.sensors.subarray(i * 6, i * 6 + 6);
          pts.push(`${q[0].toFixed(4)} ${q[1].toFixed(4)} ${q[2].toFixed(4)} ${q[3]} ${q[4]} ${q[5]}`);
        }
        const r = await radiance.illuminance(matText, sceneText, sky, pts, detail, progress);
        v = r.lux;
        if (a === 'rad-df') for (let i = 0; i < v.length; i++) v[i] /= 100;
        extra.engine = 'Radiance 6.0.3 (WebAssembly)';
      } else {
        status.textContent = `Running on ${n.toLocaleString('en')} sensors with ${pool.count} workers…`;
        v = await pool.run(preparedId, p, decoder, kind, params, progress);
      }
      if (scale !== 1) for (let i = 0; i < v.length; i++) v[i] *= scale;
      values = v;
      let lo = Infinity;
      let hi = -Infinity;
      for (const x of v) {
        lo = Math.min(lo, x);
        hi = Math.max(hi, x);
      }
      if (a === 'svf') [lo, hi] = [0, Math.max(hi, 0.05)];
      if (a === 'df' || a === 'rad-df') [lo, hi] = [0, Math.max(5, Math.ceil(hi))];
      if (a === 'rad-pit') lo = 0;
      if (a === 'sunhours') lo = 0;
      const ms = performance.now() - t0;
      lastRun = { analysis: a, params: extra, ms, range: [lo, hi] };
      drawOverlay(p, v, lo, hi);
      showLegend(a, lo, hi);
      if (typeof extra.credit === 'string') legend.append(el('div', { class: 'daylight-credit', id: 'daylight-credit' }, extra.credit));
      showTable(a, p, v);
      exportJson.disabled = exportCsv.disabled = false;
      let mean = 0;
      for (const x of v) mean += x / v.length;
      root.dataset.state = 'done';
      root.dataset.analysis = a;
      root.dataset.mean = String(mean);
      status.textContent = `${ANALYSES[a].label}: ${n.toLocaleString('en')} sensors, mean ${fmt(mean, a === 'svf' ? 3 : 2)}${ANALYSES[a].unit ? ` ${ANALYSES[a].unit}` : ''}, ${pool.count} workers, ${fmt(ms / 1000, 1)} s.`;
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      root.dataset.state = msg === 'stopped' ? 'idle' : 'error';
      status.textContent = msg === 'stopped' ? 'Stopped.' : `Daylight run failed: ${msg}`;
      if (msg !== 'stopped') host.toast(`Daylight run failed: ${msg}`, 'bad');
    } finally {
      running = false;
      runBtn.disabled = false;
      stopBtn.hidden = true;
      bar.hidden = true;
    }
  }
  runBtn.addEventListener('click', () => void run());
  // opening the panel reads the site, so it shows before anything runs
  root.closest('details')?.addEventListener('toggle', (ev) => {
    if ((ev.target as HTMLDetailsElement).open && host.worker() && !prepared) void prepare().catch(() => undefined);
  });
  stopBtn.addEventListener('click', () => {
    pool.stop();
    radiance.stop();
  });

  // ---------- hover readout ----------
  const ray = new THREE.Raycaster();
  const ndc = new THREE.Vector2();
  host.renderer.domElement.addEventListener('pointermove', (ev) => {
    if (!overlay || !values || !prepared) return;
    const r = host.renderer.domElement.getBoundingClientRect();
    ndc.set(((ev.clientX - r.left) / r.width) * 2 - 1, -((ev.clientY - r.top) / r.height) * 2 + 1);
    ray.setFromCamera(ndc, host.camera);
    const hit = ray.intersectObject(overlay, false)[0];
    if (hit?.instanceId === undefined) {
      readout.textContent = '';
      return;
    }
    const i = hit.instanceId;
    const a = lastRun?.analysis ?? 'df';
    const z = prepared.zones[prepared.sensorZone[i]];
    readout.textContent = `Sensor ${i + 1}: ${fmt(values[i], a === 'svf' ? 3 : 2)}${ANALYSES[a].unit ? ` ${ANALYSES[a].unit}` : ''} · ${zoneLabel(z)}${z?.level ? ` · ${z.level}` : ''}`;
  });

  // ---------- exports ----------
  function result(): Record<string, unknown> | null {
    if (!prepared || !values || !lastRun) return null;
    const p = prepared;
    const v = values;
    const run = lastRun;
    return {
      analysis: run.analysis,
      label: ANALYSES[run.analysis].label,
      unit: ANALYSES[run.analysis].unit,
      engine: String(run.params.engine ?? 'rvt-open daylight-rt, instant tier (preview)'),
      credit: run.params.credit ?? null,
      parameters: run.params,
      frame: 'the model’s own: metres, Z up, +Y the model’s north (true north is at the angle below)',
      site: Object.fromEntries(SITE_FIELDS.map((f) => [f.key, { value: site[f.key], source: source[f.key] }])),
      grid: { spacing: Number(spacingIn.value), offset: Number(offsetIn.value), sensors: v.length },
      materials: p.materials.map((m) => ({ label: m.label, kind: m.kind, value: Number(m.value.toFixed(4)), assumed: m.assumed })),
      zones: zoneStats(p, v).map((s) => ({ label: s.label, kind: s.zone.kind, number: s.zone.number ?? null, name: s.zone.roomName ?? s.zone.name, level: s.zone.level ?? null, sensors: s.n, mean: s.mean, min: s.min, max: s.max })),
      sensors: Array.from({ length: v.length }, (_, i) => ({ x: p.sensors[i * 6], y: p.sensors[i * 6 + 1], z: p.sensors[i * 6 + 2], zone: p.sensorZone[i], value: v[i] })),
      limits: DAYLIGHT_LIMITS,
      seconds: Number((run.ms / 1000).toFixed(2)),
    };
  }
  exportJson.addEventListener('click', () => {
    const r = result();
    if (!r) return;
    host.download(`${host.stem()}.daylight-${lastRun?.analysis}.json`, new Blob([`${JSON.stringify(r, null, 1)}\n`], { type: 'application/json' }));
  });
  exportCsv.addEventListener('click', () => {
    if (!prepared || !values || !lastRun) return;
    const q = (s: string): string => (/[",\n]/.test(s) ? `"${s.replaceAll('"', '""')}"` : s);
    const lines = ['x_m,y_m,z_m,zone,room_number,room_name,level,value'];
    for (let i = 0; i < values.length; i++) {
      const z = prepared.zones[prepared.sensorZone[i]];
      const s = prepared.sensors;
      lines.push([s[i * 6].toFixed(4), s[i * 6 + 1].toFixed(4), s[i * 6 + 2].toFixed(4), q(zoneLabel(z)), q(z?.number ?? ''), q(z?.roomName ?? ''), q(z?.level ?? ''), String(values[i])].join(','));
    }
    host.download(`${host.stem()}.daylight-${lastRun.analysis}.csv`, new Blob([`${lines.join('\n')}\n`], { type: 'text/csv' }));
  });

  function clearAll(): void {
    prepared = null;
    preparedKey = '';
    preparing = null;
    req++;
    values = null;
    lastRun = null;
    clearOverlay();
    clearHelpers('path');
    sunOff();
    pool.stop();
    legend.hidden = table.hidden = true;
    exportJson.disabled = exportCsv.disabled = true;
    readout.textContent = status.textContent = '';
    root.dataset.state = 'idle';
    delete root.dataset.sensors;
    delete root.dataset.sunPath;
    cutIn.value = '';
  }

  return {
    modelChanged(): void {
      clearAll();
      // the site shows as soon as the model opens, when the panel is open
      if ((root.closest('details') as HTMLDetailsElement | null)?.open) void prepare().catch(() => undefined);
    },
    reset(): void {
      clearAll();
      applyFileSite(null);
    },
  };
}
