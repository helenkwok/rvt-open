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
import { DETAILS, PVGIS_CREDIT, RADIANCE_BASE, Radiance, RadianceError, parseWeather, pvgisUrl, summarizeLog, weatherAt, type Weather } from './radiance';

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
  /** Points the camera at `box` (scene coordinates), looking along `along` toward it. */
  frame?(box: THREE.Box3, along?: THREE.Vector3): void;
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

/** The file's sun setting a viewer starts from (rvt-wasm `daylightPrepare`'s `sun`). */
export interface FileSun {
  source: 'view' | 'project';
  settingName: string;
  settingId: number;
  sunAndShadowType?: string;
  timeInterval?: string;
  /** The project's clock time as Revit shows it, `YYYY-MM-DDTHH:MM`. */
  start?: string;
  end?: string;
  /** Degrees: the Lighting mode's sun. */
  azimuth?: number;
  altitude?: number;
  relativeToView: boolean;
  viewName?: string;
  viewId?: number;
  castShadows?: boolean;
  ambientShadows?: boolean;
  sunlightIntensity?: number;
  shadowIntensity?: number;
  displayStyle?: string;
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
  sun?: FileSun | null;
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
  'Radiance tier: Radiance 6.0.3 compiled to WebAssembly, on the same scene and sensors. Its Monte Carlo sampling differs from a native build’s random numbers, so a run differs from native Radiance as much as two native runs do (RMSE 0.035 to 0.036 DF points on the reference building, native against native 0.036). Each worker builds its own irradiance cache for its share of the sensors.',
  'Radiance in several workers is faster on some systems than others: on the reference building 18.9 s in one worker and 9.6 s in eight in Chrome on a laptop, but no faster in eight than in one in headless Chromium on Linux in an 8-core container.',
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

/** Date (`YYYY-MM-DD`) and hour of a stored clock time `YYYY-MM-DDTHH:MM`, or null. */
export function clockTime(value: string): { date: string; hour: number } | null {
  const m = /^(\d{4}-\d{2}-\d{2})T(\d{2}):(\d{2})$/.exec(value);
  return m ? { date: m[1], hour: Number(m[2]) + Number(m[3]) / 60 } : null;
}

/**
 * The time step (minutes) of a file's solar-study interval (`SunStudyTimeInterval`), or null for
 * one that is not a step within a day (Day, Week, Month: frames of a multi-day study), seconds, or
 * an interval with no name.
 */
export function stepOf(interval: string | undefined): number | null {
  const m: Record<string, number> = { Minute: 1, Minutes15: 15, Minutes30: 30, Minutes45: 45, Hour: 60 };
  return interval && interval in m ? m[interval] : null;
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

/** Seconds as `m:ss` (or `h:mm:ss`). */
export function clock(seconds: number): string {
  const t = Math.max(0, Math.round(seconds));
  const [h, m, s] = [Math.floor(t / 3600), Math.floor((t % 3600) / 60), t % 60];
  return h ? `${h}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}` : `${m}:${String(s).padStart(2, '0')}`;
}

/**
 * A rough time for a Radiance run before it starts (seconds): rtrace's time per sensor in one
 * worker at the preview parameters grows with the scene, measured on the public samples
 * (0.06 s on 2,000 triangles, 0.17 s on 30,000, 0.8 s on 1.7 million), shared by the workers.
 * The validated parameters trace about six times as many rays.
 */
export function radianceEstimate(sensors: number, triangles: number, workers: number, validated: boolean): number {
  const perSensor = 0.06 * Math.max(1, triangles / 2000) ** 0.38 * (validated ? 6 : 1);
  return (sensors * perSensor) / Math.max(1, workers) + 2;
}

/** Triangles above which the instant tier's tracers are ended before a Radiance run, to free their memory. */
const LARGE_SCENE = 200_000;

/** A run longer than this (seconds, estimated) asks before it starts. */
const ASK_ABOVE_S = 120;

/** One storey's sensors: the level they are on, its elevation, and the top of its grid. */
interface Storey {
  key: string;
  label: string;
  elevation: number;
  top: number;
  sensors: Uint32Array;
}

/** The levels that have sensors, lowest first (a room's level, else the floor's own name). */
export function storeysOf(p: { sensors: Float64Array; sensorZone: Uint32Array; zones: { level?: string; name: string; floorZ: number }[] }): Storey[] {
  const by = new Map<string, { elevation: number; top: number; idx: number[] }>();
  const n = p.sensors.length / 6;
  for (let i = 0; i < n; i++) {
    const z = p.zones[p.sensorZone[i]];
    const key = z?.level || z?.name || '';
    let s = by.get(key);
    if (!s) by.set(key, (s = { elevation: Infinity, top: -Infinity, idx: [] }));
    s.idx.push(i);
    s.top = Math.max(s.top, p.sensors[i * 6 + 2]);
    if (z && Number.isFinite(z.floorZ)) s.elevation = Math.min(s.elevation, z.floorZ);
  }
  return [...by]
    .map(([key, s]) => {
      const elevation = Number.isFinite(s.elevation) ? s.elevation : s.top;
      return { key, label: `${key || 'Unnamed level'} (${elevation.toFixed(2)} m)`, elevation, top: s.top, sensors: Uint32Array.from(s.idx) };
    })
    .sort((a, b) => a.elevation - b.elevation || a.key.localeCompare(b.key));
}

type Decoder = { module: WebAssembly.Module; bits: Bits };

/**
 * The pool: one worker per core up to eight, each holding the scene's tracer.
 * Every message carries a request number its reply repeats, so a reply settles
 * only the call that asked; stopping rejects every call still waiting. A worker
 * that cannot start or throws (its `error` event, which sends no reply) stops
 * the pool the same way, with the reason, so no call waits for ever.
 */
class Pool {
  private workers: Worker[] = [];
  private loaded = -1;
  private ready = false;
  private req = 0;
  private pending = new Map<number, { resolve: (v: unknown) => void; reject: (e: Error) => void }>();
  constructor(private size: number) {}

  get count(): number {
    return this.size;
  }

  private spawn(): void {
    if (this.workers.length) return;
    for (let i = 0; i < this.size; i++) {
      const w = new DaylightWorker({ name: `daylight-${i}` }) as Worker;
      w.addEventListener('message', (ev: MessageEvent) => {
        const m = ev.data as { type: string; req?: number; value?: unknown; message?: string };
        if (m.req === undefined) return;
        const call = this.pending.get(m.req);
        if (!call) return;
        this.pending.delete(m.req);
        if (m.type === 'error') call.reject(new Error(m.message));
        else call.resolve(m.value);
      });
      // a worker that cannot start, throws outside a call, or sends a reply that cannot be read ends every call still waiting
      w.addEventListener('error', (ev: ErrorEvent) => {
        ev.preventDefault();
        this.stop(`a daylight worker failed${ev.message ? `: ${ev.message}` : ' to start'}`);
      });
      w.addEventListener('messageerror', () => this.stop('a daylight worker sent a reply that could not be read'));
      this.workers.push(w);
    }
  }

  /** Posts `msg` to worker `k` and resolves with its reply's `value`. */
  private call<T>(k: number, msg: Record<string, unknown>, transfer: Transferable[] = []): Promise<T> {
    this.spawn();
    const req = ++this.req;
    return new Promise<T>((resolve, reject) => {
      this.pending.set(req, { resolve: (v) => resolve(v as T), reject });
      this.workers[k].postMessage({ ...msg, req }, transfer);
    });
  }

  /** Stops every worker and rejects every call still waiting with `reason`; the next call starts new workers. */
  stop(reason = 'stopped'): void {
    for (const w of this.workers) w.terminate();
    this.workers = [];
    this.loaded = -1;
    this.ready = false;
    const calls = [...this.pending.values()];
    this.pending.clear();
    for (const c of calls) c.reject(new Error(reason));
  }

  /** A sun query, answered by the first worker: it needs the decoder, not a scene. */
  async ask<T>(decoder: Decoder, msg: Record<string, unknown>): Promise<T> {
    if (!this.ready) {
      await this.call(0, { type: 'init', module: decoder.module, bits: decoder.bits });
      this.ready = true;
    }
    return this.call<T>(0, msg);
  }

  /** True when the workers hold scene `id`'s tracer. */
  holds(id: number): boolean {
    return this.loaded === id;
  }

  private async load(id: number, p: Prepared, decoder: Decoder, loading?: (done: number, total: number) => void): Promise<void> {
    if (this.loaded === id) return;
    this.spawn();
    let done = 0;
    loading?.(0, this.workers.length);
    const materials = new Float64Array(p.materials.length * 4);
    p.materials.forEach((m, i) => materials.set([m.kind === 'glass' ? 1 : 0, m.rgb[0], m.rgb[1], m.rgb[2]], i * 4));
    await Promise.all(
      this.workers.map((_, k) =>
        this.call(k, { type: 'load', module: decoder.module, bits: decoder.bits, id, positions: p.positions, modifier: p.modifier, materials }).then(() => loading?.(++done, this.workers.length)),
      ),
    );
    this.loaded = id;
    this.ready = true;
  }

  /** Runs `kind` over every sensor in contiguous shards, each worker taking the next shard as it finishes one. */
  async run(
    id: number,
    p: Prepared,
    decoder: Decoder,
    kind: string,
    params: Float64Array,
    progress: (done: number, total: number, lanes: [number, number][]) => void,
    value?: (first: number, values: Float64Array) => void,
    loading?: (done: number, total: number) => void,
  ): Promise<Float64Array> {
    await this.load(id, p, decoder, loading);
    const n = p.sensors.length / 6;
    const out = new Float64Array(n);
    const shard = Math.min(512, Math.max(8, Math.ceil(n / (this.size * 8))));
    let next = 0;
    let done = 0;
    let failed = false;
    const lanes: [number, number][] = this.workers.map(() => [0, 0]);
    progress(0, n, lanes);
    const lane = async (k: number): Promise<void> => {
      while (next < n && !failed) {
        const first = next;
        next = Math.min(n, next + shard);
        const values = await this.call<Float64Array>(k, { type: 'run', id, sensors: p.sensors.slice(first * 6, next * 6), first, kind, params });
        // a shard that finishes after another lane failed belongs to a run that has ended
        if (failed) return;
        out.set(values, first);
        value?.(first, values);
        done += values.length;
        lanes[k][0] += values.length;
        progress(done, n, lanes);
      }
    };
    try {
      await Promise.all(this.workers.map((_, k) => lane(k)));
    } catch (e) {
      // the other lanes stop taking shards; their calls in flight settle into this finished run
      failed = true;
      throw e;
    }
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
  let preparing: { key: string; promise: Promise<Prepared>; cancel: (e: Error) => void } | null = null;
  let req = 0;
  /** The last completed run: its grid, values and the inputs it used, which hover and the exports read. */
  interface Run {
    analysis: Analysis;
    params: Record<string, unknown>;
    ms: number;
    range: [number, number];
    p: Prepared;
    values: Float64Array;
    site: Record<SiteKey, { value: number; source: Source }>;
    grid: { spacing: number; offset: number; inset: number };
  }
  let lastRun: Run | null = null;
  let runToken = 0;
  let overlay: THREE.InstancedMesh | null = null;
  const radiance = new Radiance(pool.count);
  let weather: Weather | null = null;
  // a weather load that finishes after a newer selection or a model change is dropped
  let weatherGen = 0;
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
      if (analysisSel.value === 'sun' || shadowsBox.checked) void updateSun().catch(() => undefined);
      if (pathBox.checked) void drawSunPath().catch(() => undefined);
      if (f.key === 'latitude' || f.key === 'longitude') void loadKept();
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
    if (whenSourceNow === 'file') applyFileSun(prepared?.sun);
    void updateSun().catch(() => undefined);
    if (pathBox.checked) void drawSunPath().catch(() => undefined);
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
  // no `accept` filter: the system dialog then lists the folder without matching each file against types
  // (.epw has none registered), and parseWeather says plainly when a file is neither format
  const weatherIn = el('input', { type: 'file', id: 'daylight-weather-file', 'aria-label': 'Weather file (PVGIS JSON or EPW)' });
  const NO_WEATHER = 'No weather file loaded. PVGIS does not answer requests from a web page, so the link opens its official service in a new tab: save the file it returns and choose it here, or drop it on this panel. An EPW file of your own works too.';
  const weatherNote = el('p', { class: 'option-hint', id: 'daylight-weather-note' }, NO_WEATHER);
  const weatherSaved = el('p', { class: 'option-hint', id: 'daylight-weather-saved', hidden: '' });
  const weatherForget = el('button', { type: 'button', class: 'side-btn', id: 'daylight-weather-forget', hidden: '' }, 'Forget the kept weather file');
  pvgisLink.title = `Weather data from PVGIS: ${PVGIS_CREDIT}`;
  weatherBox.append(pvgisLink, weatherIn, weatherNote, weatherSaved, weatherForget);
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
  // the run's liveness: elapsed time, the age of the last update, the time left; and per worker
  const live = el('p', { class: 'option-hint daylight-live', id: 'daylight-live', hidden: '' });
  const workersLine = el('p', { class: 'option-hint daylight-live', id: 'daylight-workers', hidden: '' });
  const errorBox = el('details', { class: 'daylight-limits', id: 'daylight-error-details', hidden: '' });
  const runBox = el('div', { class: 'daylight-runbox', id: 'daylight-runbox' });
  const runButtons = el('div', { class: 'daylight-run-buttons' });
  runButtons.append(runBtn, stopBtn);
  runBox.append(runButtons, bar, status, live, workersLine, errorBox);

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
  const whenNote = el('p', { class: 'option-hint', id: 'daylight-when-note' });
  const whenSource = el('span', { class: 'daylight-source', id: 'daylight-when-source' });
  const whenReset = el('button', { type: 'button', class: 'side-btn', id: 'daylight-when-reset', hidden: '' }, 'Use the file’s sun');
  whenBox.append(row('Date', dateIn, whenSource), timeRow, toRow, stepRow, pathsRow, detailRow, whenNote, whenReset);
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
  // which storey's grid shows: All, or one level (its grid only, the cut above it, the camera on it)
  const storeySel = el('select', { class: 'mode-select', id: 'daylight-storey', 'aria-label': 'Storey (Up and Down keys step through the levels)' });
  const storeyDown = el('button', { type: 'button', class: 'side-btn daylight-step', id: 'daylight-storey-down', 'aria-label': 'Lower level', title: 'Lower level (Down key)' }, '↓');
  const storeyUp = el('button', { type: 'button', class: 'side-btn daylight-step', id: 'daylight-storey-up', 'aria-label': 'Higher level', title: 'Higher level (Up key)' }, '↑');
  const sharedBox = el('input', { type: 'checkbox', id: 'daylight-shared-scale' });
  const sharedRow = el('label', {});
  sharedRow.append(sharedBox, document.createTextNode(' Same colour scale on every level'));
  const storeyRow = row('Storey', storeySel, storeyDown, storeyUp);
  const storeyBox = el('div', { class: 'daylight-storeys', id: 'daylight-storeys', hidden: '' });
  storeyBox.append(storeyRow, sharedRow);

  const legend = el('div', { class: 'daylight-legend', id: 'daylight-legend', hidden: '' });
  const readout = el('p', { class: 'option-hint', id: 'daylight-readout', 'aria-live': 'polite' });
  const table = el('table', { class: 'daylight-table', id: 'daylight-table', hidden: '' });
  const tableWrap = el('div', { class: 'daylight-table-wrap' });
  tableWrap.append(table);
  const exportJson = el('button', { type: 'button', class: 'side-btn', id: 'daylight-export-json', disabled: '' }, 'Export results (JSON)');
  const exportCsv = el('button', { type: 'button', class: 'side-btn', id: 'daylight-export-csv', disabled: '' }, 'Export results (CSV)');
  const limits = el('details', { class: 'daylight-limits' });
  limits.append(el('summary', {}, 'Known limits'));
  const ul = el('ul');
  for (const l of DAYLIGHT_LIMITS) ul.append(el('li', {}, l));
  limits.append(ul);

  root.append(intro, siteBox, row('Analysis', analysisSel), whenBox, weatherBox, radNote, gridBox, toggles, cutBoxes, runBox, storeyBox, legend, readout, tableWrap, exportJson, exportCsv, matsBox, limits);

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

  /** The file's site in every field, or (`keepUser`) only in the fields you have not set. */
  function applyFileSite(s: FileSite | null, keepUser = false): void {
    for (const f of SITE_FIELDS) {
      if (keepUser && source[f.key] === 'user') continue;
      site[f.key] = s ? s[f.key] : 0;
      source[f.key] = s ? 'file' : 'assumed';
    }
    showSite();
  }
  applyFileSite(null);

  timeIn.addEventListener('input', () => {
    timeOut.textContent = hhmm(Number(timeIn.value));
    whenFrom('user');
    if (analysisSel.value === 'sun' || shadowsBox.checked) void updateSun().catch(() => undefined);
  });
  dateIn.addEventListener('change', () => {
    whenFrom('user');
    if (analysisSel.value === 'sun' || shadowsBox.checked) void updateSun().catch(() => undefined);
  });
  for (const i of [toIn, stepSel]) i.addEventListener('change', () => whenFrom('user'));
  whenReset.addEventListener('click', () => {
    applyFileSun(prepared?.sun);
    // the file's view options: shadows on or off as its view has them
    const cast = prepared?.sun?.castShadows;
    if (cast === false && shadowsBox.checked) {
      shadowsBox.checked = false;
      sunOff();
    } else if (cast || analysisSel.value === 'sun' || shadowsBox.checked) void updateSun().catch(() => undefined);
  });

  /** Says where the date, time and study range came from. */
  let whenSourceNow: Source = 'assumed';
  function whenFrom(s: Source, note?: string): void {
    whenSourceNow = s;
    whenSource.textContent = s === 'file' ? 'from the file' : s === 'user' ? 'set by you' : 'assumed';
    whenSource.dataset.source = s;
    if (note !== undefined) whenNote.textContent = note;
    else if (s === 'user') whenNote.textContent = 'Date and time set by you.';
    whenReset.hidden = s !== 'user' || !prepared?.sun;
  }
  whenFrom('assumed', 'The file’s sun setting was not read: the date and time are assumed.');

  /**
   * The file's sun setting (the default 3D view's own, else the project's first named preset) in
   * the date, time and range fields, as the clock time Revit shows (the project's local time),
   * and its view's cast-shadows option. A Lighting setting stores a direction, not a date: the
   * fields keep their values and the note says so.
   */
  function applyFileSun(sun: FileSun | null | undefined, keepUser = false): void {
    if (keepUser && whenSourceNow === 'user') {
      // the user's date and time stay; the reset button shows now that the file's sun is known
      whenReset.hidden = !sun;
      return;
    }
    root.dataset.fileSun = sun ? JSON.stringify(sun) : '';
    if (!sun) {
      whenFrom('assumed', 'The file’s sun setting was not read: the date and time are assumed.');
      return;
    }
    const where = sun.source === 'view' && sun.viewName
      ? `the view “${sun.viewName}” (sun setting “${sun.settingName}”)`
      : `the project’s sun setting “${sun.settingName}”`;
    const a = sun.start ? clockTime(sun.start) : null;
    if (sun.sunAndShadowType === 'Lighting' || !a) {
      const dir = sun.azimuth !== undefined && sun.altitude !== undefined
        ? ` (azimuth ${fmt(sun.azimuth, 1)}°, altitude ${fmt(sun.altitude, 1)}°${sun.relativeToView ? ', relative to the view' : ''})`
        : '';
      const what = sun.sunAndShadowType === 'Lighting' ? `is a Lighting sun${dir}: it stores a direction, not a date` : 'holds no date that could be read';
      whenFrom('assumed', `${where[0].toUpperCase()}${where.slice(1)} ${what}, so the date and time are assumed.`);
    } else {
      dateIn.value = a.date;
      // the file's time to the minute (the slider then moves by any amount)
      timeIn.step = 'any';
      timeIn.value = String(a.hour);
      timeOut.textContent = hhmm(Number(timeIn.value));
      const study = sun.sunAndShadowType === 'OneDayStudy' || sun.sunAndShadowType === 'MultiDayStudy';
      const b = sun.end ? clockTime(sun.end) : null;
      const notes: string[] = [];
      if (study && b) {
        toIn.value = b.date;
        const step = stepOf(sun.timeInterval);
        if (step) {
          if (![...stepSel.options].some((o) => o.value === String(step))) stepSel.append(el('option', { value: String(step) }, `every ${step} min`));
          stepSel.value = String(step);
        } else {
          notes.push(`The study’s interval (${sun.timeInterval ?? 'not named'}) is not a step within a day, so the step is kept.`);
        }
        // sun hours counts whole days: the study's clock range is shown, not applied
        notes.push(`The study runs ${hhmm(a.hour)}–${hhmm(b.hour)} each day; sun hours counts whole days, so that range is not applied.`);
      } else {
        toIn.value = a.date;
      }
      const kind = sun.sunAndShadowType ? ` (${sun.sunAndShadowType})` : '';
      whenFrom('file', [`Dates and start time from ${where}${kind}, as Revit shows them (the site’s local time).`, ...notes].join(' '));
    }
  }
  shadowsBox.addEventListener('change', () => {
    if (shadowsBox.checked) void updateSun().catch(() => undefined);
    else sunOff();
  });
  pathBox.addEventListener('change', () => {
    if (pathBox.checked) void drawSunPath().catch(() => undefined);
    else clearHelpers('path');
  });

  // ---------- weather: chosen, dropped, or kept in this browser for the site ----------
  /** The site a kept weather file belongs to: latitude and longitude to 4 decimals (about 10 m). */
  const siteKey = (): string => `${site.latitude.toFixed(4)},${site.longitude.toFixed(4)}`;
  /** The site the loaded weather was loaded for. */
  let weatherSite = '';

  /** Reads a chosen or dropped file: the selection is recorded first, so a slower earlier read cannot replace it. */
  async function readWeather(f: File): Promise<void> {
    const gen = ++weatherGen;
    let text: string;
    try {
      text = await f.text();
    } catch (e) {
      if (gen === weatherGen) weatherNote.textContent = `${f.name} could not be read (${e instanceof Error ? e.message : String(e)}). ${weather ? 'The weather loaded before stays in use.' : 'No weather file is loaded.'}`;
      return;
    }
    if (gen !== weatherGen) return;
    loadWeather(f.name, text, new Date(f.lastModified).toISOString().slice(0, 10), 'file', gen);
  }

  function loadWeather(name: string, text: string, fileDate: string, from: 'file' | 'kept', gen = ++weatherGen): void {
    try {
      const w = parseWeather(name, text, fileDate);
      if (gen !== weatherGen) return;
      if (!w.hours.length) throw new Error(`${name}: no hours of weather could be read`);
      weather = w;
      const pv = weather.provenance;
      weatherNote.textContent = `${name}: ${weather.hours.length.toLocaleString('en')} hours. ${Object.entries(pv)
        .filter(([k]) => k !== 'file')
        .map(([k, v]) => `${k} ${v}`)
        .join('; ')}.${weather.credit ? ` ${weather.credit}.` : ''}`;
      root.dataset.weather = String(weather.hours.length);
      root.dataset.weatherFrom = from;
      weatherSite = siteKey();
      if (from === 'file') void keepWeather(name, text, fileDate, weatherSite, gen);
      else showKept({ name, date: fileDate, saved: '' }, true);
    } catch (e) {
      if (gen !== weatherGen) return;
      weather = null;
      delete root.dataset.weather;
      weatherNote.textContent = `This weather file could not be read: ${e instanceof Error ? e.message : String(e)}`;
    }
  }

  weatherIn.addEventListener('change', async () => {
    const f = weatherIn.files?.[0];
    if (f) await readWeather(f);
  });

  // a weather file dropped on the panel, or anywhere on the page while the analysis that uses one is chosen
  const isWeather = (f: File): boolean => /\.(epw|json)$/i.test(f.name);
  window.addEventListener(
    'drop',
    (ev) => {
      const f = ev.dataTransfer?.files[0];
      if (!f || !isWeather(f) || (!root.contains(ev.target as Node) && analysisSel.value !== 'rad-pit')) return;
      ev.preventDefault();
      ev.stopImmediatePropagation();
      document.getElementById('dropzone')?.classList.remove('drag-over');
      root.classList.remove('daylight-drop');
      if (analysisSel.value !== 'rad-pit') {
        analysisSel.value = 'rad-pit';
        showFields();
      }
      void readWeather(f);
    },
    true,
  );
  root.addEventListener('dragover', (ev) => {
    ev.preventDefault();
    root.classList.add('daylight-drop');
  });
  root.addEventListener('dragleave', () => root.classList.remove('daylight-drop'));

  // the last weather file per site, kept in this browser's IndexedDB (nothing leaves the page)
  const db = (): Promise<IDBDatabase> =>
    new Promise((resolve, reject) => {
      const r = indexedDB.open('rvt-open-daylight', 1);
      r.onupgradeneeded = () => r.result.createObjectStore('weather');
      r.onsuccess = () => resolve(r.result);
      r.onerror = () => reject(r.error ?? new Error('IndexedDB is not available'));
    });
  async function store<T>(mode: IDBTransactionMode, f: (s: IDBObjectStore) => IDBRequest): Promise<T> {
    const d = await db();
    try {
      // settled when the transaction commits (a request can succeed in a transaction that then aborts)
      return await new Promise<T>((resolve, reject) => {
        const tx = d.transaction('weather', mode);
        const r = f(tx.objectStore('weather'));
        tx.oncomplete = () => resolve(r.result as T);
        tx.onabort = tx.onerror = () => reject(tx.error ?? r.error ?? new Error('IndexedDB transaction failed'));
      });
    } finally {
      d.close();
    }
  }
  type Kept = { name: string; text: string; date: string; saved: string };
  function showKept(k: { name: string; date: string; saved: string } | null, loaded = false): void {
    weatherSaved.hidden = weatherForget.hidden = !k;
    if (!k) return;
    weatherSaved.textContent = `${loaded ? 'Loaded from' : 'Kept in'} this browser for the site ${siteKey()}: ${k.name}${k.saved ? `, kept ${k.saved}` : ''}. The next model at this site loads it without the file dialog.`;
  }
  async function keepWeather(name: string, text: string, date: string, key: string, gen: number): Promise<void> {
    try {
      const k: Kept = { name, text, date, saved: new Date().toISOString().slice(0, 10) };
      await store('readwrite', (s) => s.put(k, key));
      if (gen === weatherGen) showKept(k);
    } catch {
      weatherSaved.hidden = false;
      weatherSaved.textContent = 'This browser does not keep files for the page (private window or storage blocked): choose the weather file again next time.';
    }
  }
  /** Loads the weather file kept for the current site, if any; weather kept for another site is dropped first. */
  async function loadKept(): Promise<void> {
    if (weather && root.dataset.weatherFrom === 'kept' && weatherSite !== siteKey()) {
      weatherGen++;
      weather = null;
      delete root.dataset.weather;
      delete root.dataset.weatherFrom;
      showKept(null);
      weatherNote.textContent = NO_WEATHER;
    }
    if (weather) return;
    const gen = weatherGen;
    const key = siteKey();
    try {
      const k = await store<Kept | undefined>('readonly', (s) => s.get(key));
      // a reply for a site the panel has left, or after another file was chosen, is dropped
      if (!k || gen !== weatherGen || weather || key !== siteKey()) return;
      loadWeather(k.name, k.text, k.date, 'kept');
      showKept(k, true);
    } catch {
      // nothing kept, or no IndexedDB: the dialog and the drop still work
    }
  }
  weatherForget.addEventListener('click', async () => {
    try {
      await store('readwrite', (s) => s.delete(siteKey()));
    } catch (e) {
      weatherSaved.textContent = `The kept weather file could not be forgotten: ${e instanceof Error ? e.message : String(e)}`;
      return;
    }
    showKept(null);
    delete root.dataset.weatherFrom;
    weatherNote.textContent = weather ? `${weatherNote.textContent} (no longer kept in this browser)` : NO_WEATHER;
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

  let pathReq = 0;
  async function drawSunPath(): Promise<void> {
    const decoder = host.decoder();
    if (!decoder) return;
    const mine = ++pathReq;
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
    if (mine !== pathReq) return;
    helpers.add(group);
    root.dataset.sunPath = 'on';
    host.requestRender();
  }

  // ---------- the scene and grids ----------
  /** The grid options as the inputs give them: a blank or invalid field takes its default; zero is kept. */
  function gridOptions(): { spacing: number; offset: number; inset: number } {
    const n = (input: HTMLInputElement, d: number): number => (input.value.trim() !== '' && Number.isFinite(Number(input.value)) ? Number(input.value) : d);
    const spacing = n(spacingIn, 0.5);
    return { spacing: spacing > 0 ? spacing : 0.5, offset: n(offsetIn, 0.76), inset: 0.35 };
  }

  /** Stops waiting for a grid (its late reply is ignored). */
  function cancelPrepare(reason: string): void {
    preparing?.cancel(new Error(reason));
    preparing = null;
  }

  function prepare(): Promise<Prepared> {
    const options = gridOptions();
    const key = JSON.stringify(options);
    if (prepared && preparedKey === key) return Promise.resolve(prepared);
    if (preparing?.key === key) return preparing.promise;
    cancelPrepare('the grid options changed');
    const w = host.worker();
    if (!w) return Promise.reject(new Error('no model is open'));
    const mine = ++req;
    if (!running) root.dataset.state = 'preparing';
    if (!running) status.textContent = 'Building the scene and the sensor grids…';
    let cancel: (e: Error) => void = () => undefined;
    const promise = new Promise<Prepared>((resolve, reject) => {
      const on = (ev: MessageEvent): void => {
        const m = ev.data as { type: string; req?: number; prepared?: Prepared; message?: string };
        if (m.req !== mine || (m.type !== 'daylight-prepared' && m.type !== 'daylight-error')) return;
        w.removeEventListener('message', on);
        if (preparing?.promise === promise) preparing = null;
        if (m.type === 'daylight-error' || !m.prepared) {
          reject(new Error(m.message ?? 'no scene'));
          return;
        }
        const firstTime = prepared === null;
        prepared = m.prepared;
        preparedKey = key;
        preparedId++;
        if (firstTime) {
          applyFileSite(prepared.site, true);
          applyFileSun(prepared.sun, true);
          // the file's view casts shadows, or shadows were already on with the assumed inputs
          if (prepared.sun?.castShadows || shadowsBox.checked) void updateSun().catch(() => undefined);
          void loadKept();
        }
        showMaterials(prepared);
        if (root.dataset.state === 'preparing' && !running) {
          root.dataset.state = 'ready';
          status.textContent = `${(prepared.sensors.length / 6).toLocaleString('en')} sensors on ${prepared.zones.length} ${prepared.zones[0]?.kind === 'floor' ? 'floors (the model has no rooms)' : 'rooms'}, ${(prepared.positions.length / 9).toLocaleString('en')} triangles.`;
        }
        resolve(prepared);
      };
      cancel = (e: Error): void => {
        w.removeEventListener('message', on);
        reject(e);
      };
      w.addEventListener('message', on);
      w.postMessage({ type: 'daylight-prepare', options, req: mine });
    });
    preparing = { key, promise, cancel };
    return promise;
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
    cutManual = cutIn.value.trim() !== '';
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

  /** The sensor each drawn square stands for (the grid of one storey, or every sensor). */
  let overlayIdx: Uint32Array | null = null;
  const GREY = new THREE.Color('#9aa0a6');

  /** Draws squares for the sensors `idx` (every sensor when null), coloured by `v` (NaN: not yet known, grey). */
  function drawOverlay(p: Prepared, idx: Uint32Array | null, v: Float64Array, lo: number, hi: number): void {
    clearOverlay();
    const n = idx ? idx.length : p.sensors.length / 6;
    const size = (lastRun?.grid.spacing ?? gridOptions().spacing) * 0.92;
    const geo = new THREE.PlaneGeometry(size, size);
    geo.rotateX(-Math.PI / 2);
    const mesh = new THREE.InstancedMesh(geo, new THREE.MeshBasicMaterial({ side: THREE.DoubleSide, toneMapped: false }), Math.max(1, n));
    mesh.count = n;
    const m = new THREE.Matrix4();
    for (let k = 0; k < n; k++) {
      const i = idx ? idx[k] : k;
      const s = p.sensors.subarray(i * 6, i * 6 + 3);
      const at = toScene(s[0], s[1], s[2]);
      m.makeTranslation(at.x, at.y, at.z);
      mesh.setMatrixAt(k, m);
    }
    mesh.instanceMatrix.needsUpdate = true;
    mesh.name = 'daylight-grid';
    mesh.userData.kind = 'overlay';
    overlay = mesh;
    overlayIdx = idx;
    helpers.add(mesh);
    root.dataset.overlay = String(n);
    paintOverlay(v, lo, hi);
  }

  /** Colours the drawn squares from `v` on the scale lo to hi. */
  function paintOverlay(v: Float64Array, lo: number, hi: number): void {
    if (!overlay) return;
    const c = new THREE.Color();
    for (let k = 0; k < overlay.count; k++) {
      const x = v[overlayIdx ? overlayIdx[k] : k];
      overlay.setColorAt(k, Number.isNaN(x) ? GREY : rampColour(hi > lo ? (x - lo) / (hi - lo) : 0.5, c));
    }
    if (overlay.instanceColor) overlay.instanceColor.needsUpdate = true;
    host.requestRender();
  }

  /** Where the cut goes for the grid shown: just above its highest sensors. */
  function autoCut(top: number): void {
    if (!cutManual) cutIn.value = (top + 0.6).toFixed(2);
    cutPlane.constant = Number(cutIn.value);
    setCut(cutBox.checked && overlay !== null);
  }

  function zoneLabel(z: Zone | undefined): string {
    if (!z) return '–';
    if (z.kind === 'room') {
      const parts = [z.number, z.roomName].filter((x) => x && x.length);
      return parts.length ? parts.join(' ') : z.name || 'Room';
    }
    return z.name || 'Floor';
  }

  function showLegend(a: Analysis, lo: number, hi: number, scope = ''): void {
    legend.hidden = false;
    legend.replaceChildren();
    const unit = ANALYSES[a].unit;
    const barEl = el('div', { class: 'daylight-ramp' });
    barEl.style.background = `linear-gradient(to right, ${RAMP.join(', ')})`;
    const labels = el('div', { class: 'daylight-ramp-labels' });
    for (let k = 0; k <= 4; k++) labels.append(el('span', {}, `${fmt(lo + ((hi - lo) * k) / 4, a === 'svf' ? 3 : a === 'sunhours' ? 1 : a === 'rad-pit' ? 0 : 2)}${unit ? ` ${unit}` : ''}`));
    legend.append(el('div', { class: 'daylight-legend-title' }, `${ANALYSES[a].label}${scope ? ` · ${scope}` : ''}`), barEl, labels);
    root.dataset.legend = `${lo},${hi}`;
  }

  /** Statistics per zone, over the sensors `idx` (every sensor when null). */
  function zoneStats(p: Prepared, v: Float64Array, idx: Uint32Array | null = null): { zone: Zone; label: string; n: number; mean: number; min: number; max: number }[] {
    const acc = p.zones.map((zone) => ({ zone, label: zoneLabel(zone), n: 0, sum: 0, min: Infinity, max: -Infinity }));
    const n = idx ? idx.length : v.length;
    for (let k = 0; k < n; k++) {
      const i = idx ? idx[k] : k;
      const a = acc[p.sensorZone[i]];
      if (!a) continue;
      a.n++;
      a.sum += v[i];
      a.min = Math.min(a.min, v[i]);
      a.max = Math.max(a.max, v[i]);
    }
    return acc.filter((a) => a.n > 0).map((a) => ({ zone: a.zone, label: a.label, n: a.n, mean: a.sum / a.n, min: a.min, max: a.max }));
  }

  function showTable(a: Analysis, p: Prepared, v: Float64Array, idx: Uint32Array | null, scope: string): void {
    const d = a === 'svf' ? 3 : 2;
    table.hidden = false;
    table.replaceChildren();
    const head = el('tr');
    for (const h of [p.zones[0]?.kind === 'floor' ? 'Floor' : 'Room', 'Level', 'Sensors', 'Mean', 'Min', 'Max', 'Min / mean']) head.append(el('th', { scope: 'col' }, h));
    const thead = el('thead');
    thead.append(head);
    const body = el('tbody');
    for (const s of zoneStats(p, v, idx)) {
      const r = el('tr');
      for (const cell of [s.label, s.zone.level ?? '', String(s.n), fmt(s.mean, d), fmt(s.min, d), fmt(s.max, d), s.mean > 0 ? fmt(s.min / s.mean, 2) : '–']) r.append(el('td', {}, cell));
      body.append(r);
    }
    table.append(el('caption', {}, `${ANALYSES[a].label}${ANALYSES[a].unit ? ` (${ANALYSES[a].unit})` : ''} per ${p.zones[0]?.kind === 'floor' ? 'floor' : 'room'}${scope ? `, ${scope}` : ''}`), thead, body);
  }

  /** The legend's range for `a` over the values `v` at `idx` (every value when null). */
  function rangeOf(a: Analysis, v: Float64Array, idx: Uint32Array | null): [number, number] {
    let lo = Infinity;
    let hi = -Infinity;
    const n = idx ? idx.length : v.length;
    for (let k = 0; k < n; k++) {
      const x = v[idx ? idx[k] : k];
      if (Number.isNaN(x)) continue;
      lo = Math.min(lo, x);
      hi = Math.max(hi, x);
    }
    if (!Number.isFinite(lo)) [lo, hi] = [0, 1];
    if (a === 'svf') [lo, hi] = [0, Math.max(hi, 0.05)];
    if (a === 'df' || a === 'rad-df') [lo, hi] = [0, Math.max(5, Math.ceil(hi))];
    if (a === 'rad-pit' || a === 'sunhours') lo = 0;
    return [lo, hi];
  }

  // ---------- storeys ----------
  let storeys: Storey[] = [];
  /** The cut height was typed: it stays until another storey is picked. */
  let cutManual = false;
  function fillStoreys(p: Prepared): void {
    storeys = storeysOf(p);
    const keep = storeySel.value;
    storeySel.replaceChildren(el('option', { value: '' }, `All levels (${storeys.length})`));
    for (const s of storeys) storeySel.append(el('option', { value: s.key }, `${s.label}: ${s.sensors.length.toLocaleString('en')} sensors`));
    storeySel.value = storeys.some((s) => s.key === keep) ? keep : '';
    storeyBox.hidden = storeys.length < 2;
  }
  const storeyNow = (): Storey | null => storeys.find((s) => s.key === storeySel.value && storeySel.value !== '') ?? null;

  /** Shows the last run on the storey picked: its grid, legend, table and cut; `aim` turns the camera to it. */
  function showResult(aim: boolean): void {
    const run = lastRun;
    if (!run) return;
    const st = storeyNow();
    const idx = st ? st.sensors : null;
    const [lo, hi] = sharedBox.checked || !st ? run.range : rangeOf(run.analysis, run.values, idx);
    drawOverlay(run.p, idx, run.values, lo, hi);
    const scope = st ? `${st.label}${sharedBox.checked ? ', scale of every level' : ', scale of this level'}` : storeys.length > 1 ? 'all levels' : '';
    showLegend(run.analysis, lo, hi, scope);
    if (typeof run.params.credit === 'string') legend.append(el('div', { class: 'daylight-credit', id: 'daylight-credit' }, run.params.credit));
    showTable(run.analysis, run.p, run.values, idx, st ? st.label : '');
    root.dataset.storey = st?.key ?? '';
    let top = -Infinity;
    const n = idx ? idx.length : run.values.length;
    for (let k = 0; k < n; k++) top = Math.max(top, run.p.sensors[(idx ? idx[k] : k) * 6 + 2]);
    autoCut(top);
    if (aim && st && host.frame) {
      const box = new THREE.Box3();
      for (const i of st.sensors) box.expandByPoint(toScene(run.p.sensors[i * 6], run.p.sensors[i * 6 + 1], run.p.sensors[i * 6 + 2]));
      // from above and to one side, so the grid and the walls around it both show
      host.frame(box, new THREE.Vector3(0.6, 1.6, 0.6));
    }
  }
  function pickStorey(step: number): void {
    if (!storeys.length) return;
    const k = storeys.findIndex((s) => s.key === storeySel.value && storeySel.value !== '');
    const next = k < 0 ? (step > 0 ? 0 : storeys.length - 1) : Math.min(storeys.length - 1, Math.max(0, k + step));
    storeySel.value = storeys[next].key;
    cutManual = false;
    showResult(true);
  }
  storeySel.addEventListener('change', () => {
    cutManual = false;
    showResult(true);
  });
  storeySel.addEventListener('keydown', (ev) => {
    if (ev.key !== 'ArrowUp' && ev.key !== 'ArrowDown') return;
    ev.preventDefault();
    pickStorey(ev.key === 'ArrowUp' ? 1 : -1);
  });
  storeyUp.addEventListener('click', () => pickStorey(1));
  storeyDown.addEventListener('click', () => pickStorey(-1));
  sharedBox.addEventListener('change', () => showResult(false));

  // ---------- running: stages, progress, liveness ----------
  /** No progress for this long (ms) says so; `?daylight-stall-ms=` sets it (tests use a short one). */
  const STALL_MS = Number(new URLSearchParams(location.search).get('daylight-stall-ms')) || 60_000;
  /** The run's progress as the panel shows it, and the stages it went through. */
  const track = {
    t0: 0,
    last: 0,
    stage: '',
    stageT0: 0,
    text: '',
    done: 0,
    total: 0,
    countT0: 0,
    countDone0: 0,
    timeline: [] as { stage: string; detail: string; startS: number; seconds?: number }[],
    /** Per worker (done, share), when the work is split in fixed shares: the run ends with the slowest. */
    shares: null as [number, number][] | null,
    ticker: 0,
  };
  const STAGES: Record<string, string> = {
    scene: 'Building the scene and the sensor grids',
    suns: 'Working out the sun positions',
    load: 'Loading Radiance',
    radtext: 'Writing the scene as Radiance text',
    sky: 'Making the sky',
    oconv: 'oconv: building the octree',
    'oconv-sky': 'oconv: adding the sky',
    trace: 'Tracing',
    rtrace: 'rtrace: tracing',
    pool: 'Loading the scene into the workers',
    collect: 'Collecting the values and drawing the grid',
  };

  function trackStart(): void {
    const now = performance.now();
    Object.assign(track, { t0: now, last: now, stage: '', text: '', done: 0, total: 0, timeline: [] });
    delete root.dataset.midUpdates;
    live.hidden = false;
    workersLine.hidden = true;
    workersLine.textContent = '';
    errorBox.hidden = true;
    errorBox.replaceChildren();
    delete root.dataset.stalled;
    clearInterval(track.ticker);
    track.ticker = window.setInterval(tick, 500);
    tick();
  }

  /** Enters stage `key`, with what it works on. */
  function stage(key: string, detail = ''): void {
    // a callback of a run that has ended (failed, stopped) changes nothing
    if (!running) return;
    const now = performance.now();
    const prev = track.timeline.at(-1);
    if (prev && prev.seconds === undefined) prev.seconds = (now - track.t0) / 1000 - prev.startS;
    track.timeline.push({ stage: key, detail, startS: (now - track.t0) / 1000 });
    Object.assign(track, { stage: key, stageT0: now, last: now, done: 0, total: 0, countT0: now, countDone0: 0 });
    track.text = `${STAGES[key] ?? key}${detail ? `: ${detail}` : ''}…`;
    root.dataset.stage = key;
    delete root.dataset.done;
    delete root.dataset.total;
    bar.removeAttribute('value');
    status.textContent = track.text;
    tick();
  }

  /** A count within the stage: `done` of `total`, with a line per worker. */
  function count(done: number, total: number, text: string, workers?: string, shares?: [number, number][]): void {
    if (!running) return;
    const now = performance.now();
    if (done > track.done || total !== track.total) track.last = now;
    if (track.done === 0 && done > 0) [track.countT0, track.countDone0] = [now, 0];
    // updates that arrive while tracing is under way: the bar moved between empty and full
    if ((track.stage === 'trace' || track.stage === 'rtrace') && done > 0 && done < total) root.dataset.midUpdates = String(Number(root.dataset.midUpdates ?? 0) + 1);
    track.done = done;
    track.total = total;
    track.shares = shares ?? null;
    root.dataset.done = String(done);
    root.dataset.total = String(total);
    bar.value = total ? done / total : 0;
    status.textContent = text;
    if (workers !== undefined) {
      workersLine.hidden = !workers;
      workersLine.textContent = workers;
    }
    tick();
  }

  /** Elapsed time, the age of the last update, the time left once enough is done, and the stall warning. */
  function tick(): void {
    if (!running) return;
    const now = performance.now();
    const quiet = now - track.last;
    const parts = [`Elapsed ${clock((now - track.t0) / 1000)}`, `last update ${Math.round(quiet / 1000)} s ago`];
    const { done, total } = track;
    if (total && done > 0 && done < total) {
      const spent = (now - track.countT0) / 1000;
      // with fixed shares, the slowest worker's own rate decides the end
      let left = (spent * (total - done)) / done;
      if (track.shares?.every(([d]) => d > 0)) left = Math.max(...track.shares.map(([d, s]) => (spent * (s - d)) / d));
      if (done / total >= 0.03 && spent >= 3) parts.push(`about ${clock(left)} left`);
      else parts.push('time left: shown once 3 % is done');
    }
    let text = parts.join(' · ');
    if (quiet > STALL_MS) {
      text = `No progress for ${Math.round(quiet / 1000)} s (${STAGES[track.stage] ?? track.stage}): still working, or stuck? Stop ends the run. ${text}`;
      root.dataset.stalled = 'true';
    } else delete root.dataset.stalled;
    live.textContent = text;
  }

  function trackEnd(): void {
    clearInterval(track.ticker);
    const prev = track.timeline.at(-1);
    if (prev && prev.seconds === undefined) prev.seconds = (performance.now() - track.t0) / 1000 - prev.startS;
    root.dataset.timeline = JSON.stringify(track.timeline.map((t) => [t.stage, Number((t.seconds ?? 0).toFixed(2))]));
  }

  /** The last failed run, for its JSON export. */
  let lastFailure: Record<string, unknown> | null = null;
  /** The confirmed size of a long run: a second Run with the same inputs starts it. */
  let askedFor = '';
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
    const token = ++runToken;
    const alive = (): void => {
      if (token !== runToken) throw new Error('stopped');
    };
    runBtn.disabled = true;
    stopBtn.hidden = false;
    bar.hidden = false;
    lastFailure = null;
    exportJson.disabled = exportCsv.disabled = true;
    root.dataset.state = 'running';
    trackStart();
    // the status and the bar are beside Run, wherever the panel is scrolled
    runBox.scrollIntoView({ block: 'nearest' });
    let radianceInfo: Record<string, unknown> = {};
    try {
      stage('scene', prepared && preparedKey === JSON.stringify(gridOptions()) ? 'kept from the last run' : '');
      const p = await prepare();
      alive();
      const grid = gridOptions();
      const siteNow = { ...site };
      const siteUsed = Object.fromEntries(SITE_FIELDS.map((f) => [f.key, { value: siteNow[f.key], source: source[f.key] }])) as Run['site'];
      const n = p.sensors.length / 6;
      if (n === 0) throw new Error('the model has no rooms or floors to lay a sensor grid on');
      const tris = p.positions.length / 9;
      const levels = storeysOf(p).length;
      const rooms = p.zones[0]?.kind === 'floor' ? 'floors (the model has no rooms)' : 'rooms';
      status.textContent = `${tris.toLocaleString('en')} surfaces; ${n.toLocaleString('en')} sensors in ${p.zones.length} ${rooms} on ${levels} level${levels === 1 ? '' : 's'}.`;
      root.dataset.sensors = String(n);
      // a long run asks first, with the counts and the estimated time; nothing is capped
      if (a.startsWith('rad-')) {
        const est = radianceEstimate(n, tris, radiance.count, detailSel.value === 'validated');
        const key = `${preparedId}:${a}:${detailSel.value}:${n}`;
        root.dataset.estimate = String(Math.round(est));
        if (est > ASK_ABOVE_S && askedFor !== key) {
          askedFor = key;
          status.textContent = `This run is large: ${n.toLocaleString('en')} sensors and ${tris.toLocaleString('en')} surfaces. Radiance would take roughly ${clock(est)} with ${radiance.count} workers (a rough estimate; the time left is shown while it runs). Press Run again to start it, or pick fewer sensors (a wider grid spacing).`;
          runBtn.textContent = `Run anyway (about ${clock(est)})`;
          root.dataset.state = 'ask';
          live.hidden = true;
          return;
        }
      }
      askedFor = '';
      runBtn.textContent = 'Run';
      root.dataset.state = 'running';
      let kind = a as string;
      let params: Float64Array;
      let scale = 1;
      const extra: Record<string, unknown> = {};
      if (a === 'sunhours') {
        const from = dayOfYear(dateIn.value);
        const to = toIn.value ? dayOfYear(toIn.value) : from;
        const step = Number(stepSel.value) || 15;
        extra.from = dateIn.value.slice(5);
        extra.to = (toIn.value || dateIn.value).slice(5);
        stage('suns', `${extra.from} to ${extra.to}, every ${step} min`);
        params = await pool.ask<Float64Array>(decoder, { type: 'suns', site: { ...siteNow }, from, to, step });
        alive();
        kind = 'sun';
        scale = step / 60;
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
      // the grid fills in as values come back: grey until known, on a scale that grows with them
      // (the storeys are this run's from here on: the result they described is gone)
      fillStoreys(p);
      const v = new Float64Array(n).fill(Number.NaN);
      const unitScale = a === 'rad-df' ? 0.01 : scale;
      lastRun = null;
      storeySel.value = '';
      drawOverlay(p, null, v, 0, 1);
      autoCut(Math.max(...storeys.map((s) => s.top)));
      let painted = 0;
      // every repaint redraws the model as well: a large one is repainted once a second, not four times
      const paintEvery = tris > LARGE_SCENE ? 1000 : 250;
      const paint = (): void => {
        const now = performance.now();
        if (now - painted < paintEvery) return;
        painted = now;
        const [lo, hi] = rangeOf(a, v, null);
        paintOverlay(v, lo, hi);
      };
      const sensorsText = (done: number, total: number): string => `${done.toLocaleString('en')} of ${total.toLocaleString('en')} sensors`;
      if (a === 'rad-df' || a === 'rad-pit') {
        radiance.start();
        if (!(await radiance.available())) throw new Error('the Radiance build is not beside this page (radiance/)');
        const detail = DETAILS[detailSel.value as 'preview' | 'validated'];
        const progs = [a === 'rad-pit' ? 'gendaylit' : 'gensky', 'oconv', 'rtrace'];
        stage('load', `${progs.join(', ')} and the files they read`);
        await radiance.load(progs, (files, of, bytes, totalBytes) =>
          count(files, of, `Loading Radiance: ${files} of ${of} files, ${(bytes / 2 ** 20).toFixed(1)}${totalBytes ? ` of ${(totalBytes / 2 ** 20).toFixed(1)}` : ''} MB`),
        );
        alive();
        let sky: string;
        stage('sky', a === 'rad-pit' ? 'gendaylit, a Perez sky from the weather file' : 'gensky, the CIE overcast sky');
        if (a === 'rad-pit') {
          const w = weather;
          if (!w) throw new Error('choose a weather file first (PVGIS JSON or EPW): the file dialog, or drop it on the panel');
          const doy = dayOfYear(dateIn.value);
          const month = [31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334, 365].findIndex((end) => doy <= end) + 1;
          const day = doy - [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334][month - 1];
          const h = weatherAt(w, month, day, Number(timeIn.value), siteNow.timeZone);
          if (!h) throw new Error(`the weather file has no hour on ${dateIn.value.slice(5)}`);
          sky = await radiance.perezSky(h.month, h.day, h.hour, siteNow, h.dni, h.dhi);
          alive();
          Object.assign(extra, { weatherHour: { month: h.month, day: h.day, hour: h.hour, dni: h.dni, dhi: h.dhi }, sky: 'Perez (gendaylit -W)', weather: w.provenance, credit: w.credit });
        } else {
          sky = await radiance.overcastSky();
          Object.assign(extra, { sky: 'CIE overcast, 10,000 lux horizontal (gensky -ang 45 0 -c -B 55.866)', groundReflectance: 0.2 });
        }
        extra.rtrace = ['-I', '-h', ...detail.args].join(' ');
        const materials = new Float64Array(p.materials.length * 4);
        p.materials.forEach((m, i) => materials.set([m.kind === 'glass' ? 1 : 0, m.rgb[0], m.rgb[1], m.rgb[2]], i * 4));
        // gendaylit's sky has +Y toward true north: the scene and sensors turn by the angle to true north
        // (true = R(-angle) model). The overcast sky is the same all round, so the daylight factor needs no turn.
        const turn = a === 'rad-pit' && siteNow.trueNorth !== 0 ? (-siteNow.trueNorth * Math.PI) / 180 : 0;
        const [ts, tc] = [Math.sin(turn), Math.cos(turn)];
        const rot = (src: Float64Array, stride: number): Float64Array => {
          if (turn === 0) return src;
          const out = src.slice();
          for (let i = 0; i + 1 < out.length; i += stride === 9 ? 3 : stride) {
            const [x, y] = [src[i], src[i + 1]];
            out[i] = x * tc - y * ts;
            out[i + 1] = x * ts + y * tc;
          }
          return out;
        };
        if (turn !== 0) extra.trueNorthTurn = siteNow.trueNorth;
        stage('radtext', `${tris.toLocaleString('en')} surfaces`);
        const [matText, sceneText, degenerate, duplicate] = await pool.ask<string[]>(decoder, { type: 'radtext', positions: rot(p.positions, 9), modifier: p.modifier, materials });
        alive();
        // on a large scene the instant tier's workers each hold a tracer of it: they are ended before
        // Radiance's workers load the octree (the next instant run loads them again), so the tab does not hold both
        if (tris > LARGE_SCENE) pool.stop('freeing memory for Radiance');
        const skipped = { degenerate: Number(degenerate) || 0, duplicate: Number(duplicate) || 0 };
        extra.skippedTriangles = skipped;
        const sens = rot(p.sensors, 6);
        const pts: string[] = [];
        for (let i = 0; i < n; i++) {
          const q = sens.subarray(i * 6, i * 6 + 6);
          pts.push(`${q[0].toFixed(4)} ${q[1].toFixed(4)} ${q[2].toFixed(4)} ${q[3]} ${q[4]} ${q[5]}`);
        }
        const r = await radiance.illuminance(matText, sceneText, `${preparedId}:${turn}`, sky, pts, detail, {
          stage,
          progress: (done, total, workers, final) =>
            count(
              done,
              total,
              `rtrace: ${sensorsText(done, total)} traced`,
              workers.length > 1 ? `Workers: ${workers.map(([d, s], k) => `${k + 1}: ${d.toLocaleString('en')}/${s.toLocaleString('en')}`).join(' · ')}` : '',
              // once every part is taken, the slowest worker decides the end
              final ? workers : undefined,
            ),
          value: (i, x) => {
            v[i] = x * unitScale;
            paint();
          },
        });
        alive();
        radiance.release();
        if (a === 'rad-df') for (let i = 0; i < n; i++) v[i] = r.lux[i] / 100;
        else v.set(r.lux);
        extra.engine = 'Radiance 6.0.3 (WebAssembly)';
        radianceInfo = { workers: r.workers, octreeBytes: r.octreeBytes, sceneOctreeReused: r.sceneReused, octreeResolution: r.resolution, setOverflows: r.setOverflows };
        Object.assign(extra, radianceInfo);
        const logText = radiance.log.map((l) => `${l.prog}:\n${l.text}`).join('\n');
        if (logText) extra.radianceLog = logText;
      } else {
        if (!pool.holds(preparedId)) stage('pool', `${tris.toLocaleString('en')} surfaces, one tracer per worker`);
        let traced = false;
        const out = await pool.run(
          preparedId,
          p,
          decoder,
          kind,
          params,
          (done, total, lanes) => {
            if (!traced) {
              traced = true;
              stage('trace', `${n.toLocaleString('en')} sensors in ${pool.count} workers`);
            }
            count(done, total, `Tracing: ${sensorsText(done, total)}`, lanes.length > 1 ? `Workers: ${lanes.map(([d], k) => `${k + 1}: ${d.toLocaleString('en')}`).join(' · ')}` : '');
          },
          (first, values) => {
            for (let k = 0; k < values.length; k++) v[first + k] = values[k] * scale;
            paint();
          },
          (done, total) => count(done, total, `Loading the scene into the workers: ${done} of ${total} ready`),
        );
        alive();
        for (let i = 0; i < n; i++) v[i] = out[i] * scale;
      }
      stage('collect');
      const ms = performance.now() - track.t0;
      lastRun = { analysis: a, params: extra, ms, range: rangeOf(a, v, null), p, values: v, site: siteUsed, grid };
      trackEnd();
      extra.stages = track.timeline.map((t) => ({ stage: t.stage, detail: t.detail, seconds: Number((t.seconds ?? 0).toFixed(2)) }));
      showResult(false);
      exportJson.disabled = exportCsv.disabled = false;
      let mean = 0;
      for (const x of v) mean += x / v.length;
      root.dataset.state = 'done';
      root.dataset.analysis = a;
      root.dataset.mean = String(mean);
      const sk = extra.skippedTriangles as { degenerate: number; duplicate: number } | undefined;
      const skippedText = sk && (sk.degenerate || sk.duplicate) ? ` Left out of the Radiance scene: ${sk.degenerate.toLocaleString('en')} zero-area and ${sk.duplicate.toLocaleString('en')} duplicate triangles.` : '';
      const workers = (radianceInfo.workers as number | undefined) ?? pool.count;
      const over = Number(radianceInfo.setOverflows) || 0;
      const overText = over ? ` oconv reported ${over.toLocaleString('en')} set overflow${over === 1 ? '' : 's'} (more surfaces in one spot than its octree lists, so some there are missing from the scene).` : '';
      status.textContent = `Done in ${fmt(ms / 1000, 1)} s: ${n.toLocaleString('en')} sensors, mean ${fmt(mean, a === 'svf' ? 3 : 2)}${ANALYSES[a].unit ? ` ${ANALYSES[a].unit}` : ''} (${ANALYSES[a].label}, ${workers} workers).${skippedText}${overText}`;
      live.textContent = `Stages: ${track.timeline.map((t) => `${STAGES[t.stage] ?? t.stage} ${fmt(t.seconds ?? 0, 1)} s`).join(' · ')}`;
    } catch (err) {
      // a failed share leaves the others running: stop them
      if (a.startsWith('rad-')) radiance.stop();
      trackEnd();
      // a run the model change or Stop ended leaves the panel to what came after it
      if (token !== runToken) return;
      const msg = err instanceof Error ? err.message : String(err);
      if (msg === 'stopped') {
        root.dataset.state = 'idle';
        status.textContent = `Stopped after ${clock((performance.now() - track.t0) / 1000)}.`;
        live.hidden = true;
        clearOverlay();
        return;
      }
      root.dataset.state = 'error';
      const where = STAGES[track.stage] ?? track.stage;
      let sentence: string;
      let log = '';
      if (err instanceof RadianceError) {
        const s = summarizeLog(err.prog, err.log, err.message);
        sentence = [s.sentence, ...s.warnings.map((w) => `${w}.`)].join(' ');
        log = err.log;
      } else sentence = `${where} failed: ${msg}${/memory|out of bounds|OOM/i.test(msg) ? ' (the tab ran out of memory: try a wider grid spacing)' : ''}.`;
      status.textContent = `Daylight run failed. ${sentence}`;
      if (log) {
        errorBox.hidden = false;
        errorBox.replaceChildren(el('summary', {}, 'Details'), el('pre', { class: 'daylight-log' }, log));
      }
      clearOverlay();
      // the export is this failed attempt's, not an earlier result's
      lastRun = null;
      legend.hidden = table.hidden = true;
      exportCsv.disabled = true;
      lastFailure = { analysis: a, label: ANALYSES[a].label, failed: true, error: sentence, stage: track.stage, stages: track.timeline, log: log || null, seconds: Number(((performance.now() - track.t0) / 1000).toFixed(2)) };
      exportJson.disabled = false;
      host.toast(`Daylight run failed. ${sentence}`, 'bad');
    } finally {
      // a run Stop or a model change ended has already handed the panel back (maybe to a newer run)
      if (token === runToken) {
        running = false;
        clearInterval(track.ticker);
        runBtn.disabled = false;
        stopBtn.hidden = true;
        bar.hidden = true;
      }
    }
  }
  runBtn.addEventListener('click', () =>
    void run().catch((e) => {
      // nothing ends silently: a failure outside the run's own handling still says so
      status.textContent = `Daylight run failed: ${e instanceof Error ? e.message : String(e)}`;
      root.dataset.state = 'error';
    }),
  );
  // opening the panel reads the site, so it shows before anything runs
  root.closest('details')?.addEventListener('toggle', (ev) => {
    if ((ev.target as HTMLDetailsElement).open && host.worker() && !prepared) void prepare().catch(() => undefined);
  });
  stopBtn.addEventListener('click', () => {
    runToken++;
    cancelPrepare('stopped');
    pool.stop();
    radiance.stop();
    running = false;
    clearInterval(track.ticker);
    // Run is free at once, even while a request of the stopped run is still out on the network
    runBtn.disabled = false;
    stopBtn.hidden = true;
    bar.hidden = true;
    root.dataset.state = 'idle';
    status.textContent = `Stopped after ${clock((performance.now() - track.t0) / 1000)}.`;
    live.hidden = true;
    clearOverlay();
  });

  // ---------- hover readout ----------
  const ray = new THREE.Raycaster();
  const ndc = new THREE.Vector2();
  host.renderer.domElement.addEventListener('pointermove', (ev) => {
    if (!overlay || !lastRun) return;
    const { values, p: grid } = lastRun;
    const r = host.renderer.domElement.getBoundingClientRect();
    ndc.set(((ev.clientX - r.left) / r.width) * 2 - 1, -((ev.clientY - r.top) / r.height) * 2 + 1);
    ray.setFromCamera(ndc, host.camera);
    const hit = ray.intersectObject(overlay, false)[0];
    if (hit?.instanceId === undefined) {
      readout.textContent = '';
      return;
    }
    const i = overlayIdx ? overlayIdx[hit.instanceId] : hit.instanceId;
    const a = lastRun?.analysis ?? 'df';
    const z = grid.zones[grid.sensorZone[i]];
    readout.textContent = `Sensor ${i + 1}: ${fmt(values[i], a === 'svf' ? 3 : 2)}${ANALYSES[a].unit ? ` ${ANALYSES[a].unit}` : ''} · ${zoneLabel(z)}${z?.level ? ` · ${z.level}` : ''}`;
  });

  // ---------- exports ----------
  function result(): Record<string, unknown> | null {
    if (!lastRun) return lastFailure ? { ...lastFailure, limits: DAYLIGHT_LIMITS } : null;
    const run = lastRun;
    const { p, values: v } = run;
    return {
      analysis: run.analysis,
      label: ANALYSES[run.analysis].label,
      unit: ANALYSES[run.analysis].unit,
      engine: String(run.params.engine ?? 'rvt-open daylight-rt, instant tier (preview)'),
      credit: run.params.credit ?? null,
      parameters: run.params,
      frame: 'the model’s own: metres, Z up, +Y the model’s north (true north is at the angle below)',
      site: run.site,
      grid: { ...run.grid, sensors: v.length },
      materials: p.materials.map((m) => ({ label: m.label, kind: m.kind, value: Number(m.value.toFixed(4)), assumed: m.assumed })),
      storeys: storeysOf(p).map((st) => {
        let [sum, min, max] = [0, Infinity, -Infinity];
        for (const i of st.sensors) [sum, min, max] = [sum + v[i], Math.min(min, v[i]), Math.max(max, v[i])];
        return { level: st.key, elevation: st.elevation, sensors: st.sensors.length, mean: sum / st.sensors.length, min, max };
      }),
      zones: zoneStats(p, v).map((s) => ({ label: s.label, kind: s.zone.kind, number: s.zone.number ?? null, name: s.zone.roomName ?? s.zone.name, level: s.zone.level ?? null, sensors: s.n, mean: s.mean, min: s.min, max: s.max })),
      sensors: Array.from({ length: v.length }, (_, i) => ({ x: p.sensors[i * 6], y: p.sensors[i * 6 + 1], z: p.sensors[i * 6 + 2], zone: p.sensorZone[i], value: v[i] })),
      limits: DAYLIGHT_LIMITS,
      seconds: Number((run.ms / 1000).toFixed(2)),
    };
  }
  exportJson.addEventListener('click', () => {
    const r = result();
    if (!r) return;
    host.download(`${host.stem()}.daylight-${lastRun?.analysis ?? `${String(lastFailure?.analysis)}-failed`}.json`, new Blob([`${JSON.stringify(r, null, 1)}\n`], { type: 'application/json' }));
  });
  exportCsv.addEventListener('click', () => {
    if (!lastRun) return;
    const { p: grid, values } = lastRun;
    const q = (s: string): string => (/[",\n]/.test(s) ? `"${s.replaceAll('"', '""')}"` : s);
    const lines = ['x_m,y_m,z_m,zone,room_number,room_name,level,value'];
    for (let i = 0; i < values.length; i++) {
      const z = grid.zones[grid.sensorZone[i]];
      const s = grid.sensors;
      lines.push([s[i * 6].toFixed(4), s[i * 6 + 1].toFixed(4), s[i * 6 + 2].toFixed(4), q(zoneLabel(z)), q(z?.number ?? ''), q(z?.roomName ?? ''), q(z?.level ?? ''), String(values[i])].join(','));
    }
    host.download(`${host.stem()}.daylight-${lastRun.analysis}.csv`, new Blob([`${lines.join('\n')}\n`], { type: 'text/csv' }));
  });

  function clearAll(): void {
    prepared = null;
    preparedKey = '';
    cancelPrepare('the model changed');
    runToken++;
    req++;
    lastRun = null;
    clearOverlay();
    clearHelpers('path');
    sunOff();
    pool.stop();
    radiance.stop();
    // weather belongs to a site: a new model starts without it
    weatherGen++;
    weather = null;
    weatherIn.value = '';
    weatherNote.textContent = NO_WEATHER;
    delete root.dataset.weather;
    legend.hidden = table.hidden = true;
    exportJson.disabled = exportCsv.disabled = true;
    readout.textContent = status.textContent = '';
    // the run the model change ended hands the panel back here
    running = false;
    clearInterval(track.ticker);
    runBtn.disabled = false;
    stopBtn.hidden = bar.hidden = true;
    live.hidden = workersLine.hidden = errorBox.hidden = true;
    lastFailure = null;
    askedFor = '';
    runBtn.textContent = analysisSel.value === 'sun' ? 'Show the sun' : 'Run';
    storeys = [];
    storeySel.replaceChildren();
    storeyBox.hidden = true;
    cutManual = false;
    radiance.forget();
    showKept(null);
    delete root.dataset.weatherFrom;
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
      applyFileSun(null);
    },
  };
}
