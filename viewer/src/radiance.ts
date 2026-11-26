/*
 * The Daylight panel's validated tier: Radiance itself (6.0.3 sources, compiled
 * to WebAssembly by tools/radiance/build-wasm.sh), loaded only when a Radiance
 * analysis runs, from ./radiance/ beside the page.
 *
 * The page compiles each program's wasm once (WebAssembly.compileStreaming) and
 * hands the compiled module to its workers (radiance-worker.ts), which only
 * instantiate it, rather than each worker fetching and compiling its own.
 *
 * The recipes are Radiance's own command lines: gensky (CIE overcast, for the
 * daylight factor) or gendaylit (a Perez sky from direct normal and diffuse
 * horizontal irradiance, for a point in time), oconv, then rtrace -I on the
 * sensors, split into one contiguous share per worker.
 */

import RadianceWorker from './radiance-worker.ts?worker&inline';

/** Base URL of the Radiance build: `radiance/` next to the page. */
export const RADIANCE_BASE = new URL('radiance/', document.baseURI).href;

export interface Detail {
  label: string;
  args: string[];
}

/**
 * rtrace's ambient parameters. `preview` is the daylight-factor recipe common
 * daylighting tools default to; `validated` the higher-accuracy settings their
 * recipes give for final numbers.
 */
export const DETAILS: Record<'preview' | 'validated', Detail> = {
  preview: { label: 'Preview: -ab 2 -aa 0.1 -ad 2048 -ar 64', args: ['-ab', '2', '-aa', '0.1', '-ad', '2048', '-ar', '64'] },
  validated: {
    label: 'Validated: -ab 5 -ad 2048 -as 1024 -ar 256 -aa 0.15 -lr 6 -lw 2e-4',
    args: ['-ab', '5', '-ad', '2048', '-as', '1024', '-ar', '256', '-aa', '0.15', '-lr', '6', '-lw', '2e-4'],
  },
};

/** The sky and ground hemispheres every sky description needs (as gensky's manual shows). */
export const GLOW = `skyfunc glow sky_glow
0
0
4 1 1 1 0
sky_glow source sky
0
0
4 0 0 1 180
skyfunc glow ground_glow
0
0
4 1 1 1 0
ground_glow source ground
0
0
4 0 0 -1 180
`;

const enc = (s: string): Uint8Array => new TextEncoder().encode(s);
const dec = (b: Uint8Array): string => new TextDecoder().decode(b);

/** Radiance's photopic grey value times its 179 lm/W luminous efficacy: lux from an RGB irradiance line. */
function lux(line: string): number {
  const [r, g, b] = line.trim().split(/\s+/).map(Number);
  return 179 * (0.265 * r + 0.67 * g + 0.065 * b);
}

/** A Radiance program that failed: `log` is everything it wrote to standard error. */
export class RadianceError extends Error {
  constructor(
    message: string,
    readonly log: string,
    readonly prog: string,
  ) {
    super(message);
  }
}

/**
 * A Radiance log read for a person: the fatal line as one plain sentence, and
 * the warnings collapsed by kind with a count. Radiance writes
 * `<program>: <kind> - <message>`, a warning's message naming the object.
 */
export function summarizeLog(prog: string, log: string, fallback = ''): { sentence: string; warnings: string[] } {
  const lines = log.split('\n').map((l) => l.trim()).filter(Boolean);
  const kinds = new Map<string, number>();
  let fatal = '';
  // a line Radiance marked as an error outranks later text it did not mark
  let classified = false;
  for (const l of lines) {
    const m = /^[^:]*:\s*(warning|fatal|system|internal|user|consistency)\s*-\s*(.*)$/i.exec(l);
    if (m && m[1].toLowerCase() === 'warning') {
      const text = m[2].replace(/"[^"]*"/g, '').replace(/\([^)]*\)/g, '').replace(/\s+/g, ' ').trim();
      kinds.set(text, (kinds.get(text) ?? 0) + 1);
    } else if (m) {
      fatal = m[2];
      classified = true;
    } else if (!classified && !/^…/.test(l)) fatal = l.replace(/^[^:]*:\s*/, '');
  }
  const n = (k: number): string => k.toLocaleString('en');
  const warnings = [...kinds].map(([text, k]) =>
    /^zero area for polygon/.test(text)
      ? `${n(k)} zero-area polygon${k === 1 ? '' : 's'} skipped`
      : /^set overflow/.test(text)
        ? `${n(k)} set overflow${k === 1 ? '' : 's'} (more surfaces in one spot than the octree can separate)`
        : `${n(k)} × ${text}`,
  );
  const reason = (fatal || fallback || 'it stopped without saying why').replace(/\s+/g, ' ').trim();
  const what = prog === 'oconv' ? 'build the scene' : prog === 'rtrace' ? 'trace the sensors' : prog.startsWith('gen') ? 'make the sky' : `run ${prog}`;
  const oom = /out of memory|memory access out of bounds|cannot enlarge memory|Aborted\(OOM\)/i.test(reason) ? ' (the program ran out of memory: WebAssembly gives each one at most 2 GiB)' : '';
  return { sentence: `Radiance could not ${what}: ${reason}${oom}.`, warnings };
}

/** What a run of illuminance() reports while it works. */
export interface RadianceHooks {
  /** A new stage: its name and what it works on. */
  stage(name: string, detail?: string): void;
  /** Files loaded of the total, and their bytes so far of the total (0 when not known). */
  loading?(files: number, of: number, bytes: number, totalBytes: number): void;
  /** Sensors traced of the total, and per worker (done, taken so far); `final` once every part is taken. */
  progress(done: number, total: number, workers: [number, number][], final: boolean): void;
  /** One sensor's value (lux), as soon as its worker writes it. */
  value?(i: number, lux: number): void;
}

/**
 * rtrace's memory per worker: about six to eight times the octree (the scene it reads back, the
 * ambient cache) plus a base, measured on the public samples (76 MiB with a 1.1 MiB octree, 176 MiB
 * with 18 MiB, 991 MiB with 118 MiB, 1,552 MiB with 271 MiB).
 */
const workerBytes = (octree: number): number => 7 * octree + 64 * 2 ** 20;

export class Radiance {
  private modules = new Map<string, Promise<WebAssembly.Module>>();
  private files = new Map<string, Promise<Uint8Array>>();
  private workers: Worker[] = [];
  private req = 0;
  /** Why the workers were stopped, until the next start; a run started meanwhile fails with it. */
  private stopped: string | null = null;
  private pending = new Map<number, { resolve: (out: Uint8Array) => void; reject: (e: Error) => void; progress?: (text: string) => void }>();
  /** The scene's octree from the last run, kept while the scene is the same (a new sky reuses it). */
  private sceneOct: { key: string; oct: Uint8Array; resolution: string; setOverflows: number } | null = null;
  /** `?radiance-compile=each` makes every worker fetch and compile each program itself, to measure against sharing. */
  private compileEach = new URLSearchParams(location.search).get('radiance-compile') === 'each';
  /** Warnings and other standard error of the last run, per program. */
  log: { prog: string; text: string }[] = [];
  constructor(private size: number) {}

  get count(): number {
    return this.size;
  }

  /** True when the build is beside the page (a HEAD request for rtrace's wasm). */
  async available(): Promise<boolean> {
    try {
      const r = await fetch(`${RADIANCE_BASE}rtrace.wasm`, { method: 'HEAD' });
      return r.ok;
    } catch {
      return false;
    }
  }

  private module(prog: string, onBytes?: (n: number, total: number) => void): Promise<WebAssembly.Module> {
    let m = this.modules.get(prog);
    if (!m) {
      m = fetch(`${RADIANCE_BASE}${prog}.wasm`).then((r) => {
        if (!r.ok) throw new Error(`${prog}.wasm: HTTP ${r.status}`);
        const total = Number(r.headers.get('content-length')) || 0;
        if (!r.body || !onBytes) return WebAssembly.compileStreaming(r);
        let n = 0;
        const counted = r.body.pipeThrough(
          new TransformStream<Uint8Array, Uint8Array>({
            transform(c, ctl) {
              n += c.length;
              onBytes(n, total);
              ctl.enqueue(c);
            },
          }),
        );
        return WebAssembly.compileStreaming(new Response(counted, { headers: { 'Content-Type': 'application/wasm' } }));
      });
      this.modules.set(prog, m);
      m.catch(() => this.modules.delete(prog));
    }
    return m;
  }

  private file(name: string): Promise<Uint8Array> {
    let f = this.files.get(name);
    if (!f) {
      f = fetch(`${RADIANCE_BASE}${name}`).then(async (r) => {
        if (!r.ok) throw new Error(`${name}: HTTP ${r.status}`);
        return new Uint8Array(await r.arrayBuffer());
      });
      this.files.set(name, f);
      f.catch(() => this.files.delete(name));
    }
    return f;
  }

  /** Fetches and compiles `progs` and the files rtrace reads, reporting files and bytes as they come. */
  async load(progs: string[], report?: RadianceHooks['loading']): Promise<void> {
    const cal = ['rayinit.cal', 'skybright.cal', 'perezlum.cal'];
    const of = progs.length + cal.length;
    const bytes = new Map<string, [number, number]>();
    let files = 0;
    const tell = (): void => {
      let a = 0;
      let b = 0;
      for (const [n, t] of bytes.values()) {
        a += n;
        b += t;
      }
      report?.(files, of, a, b);
    };
    tell();
    await Promise.all([
      ...progs.map((p) =>
        (this.compileEach ? Promise.resolve() : this.module(p, (n, t) => (bytes.set(p, [n, t]), tell()))).then(() => {
          files++;
          tell();
        }),
      ),
      ...cal.map((c) =>
        this.file(c).then((d) => {
          bytes.set(c, [d.length, d.length]);
          files++;
          tell();
        }),
      ),
    ]);
  }

  /** Stops every worker and rejects every run still waiting with `reason`. */
  stop(reason = 'stopped'): void {
    this.stopped = reason;
    for (const w of this.workers) w.terminate();
    this.workers = [];
    const calls = [...this.pending.values()];
    this.pending.clear();
    for (const c of calls) c.reject(new Error(reason));
  }

  private worker(k: number): Worker {
    while (this.workers.length <= k) {
      const w = new RadianceWorker({ name: `radiance-${this.workers.length}` }) as Worker;
      w.addEventListener('message', (ev: MessageEvent) => {
        const m = ev.data as { type: string; req: number; out?: Uint8Array; err?: string; message?: string; log?: string; prog?: string; text?: string };
        const call = this.pending.get(m.req);
        if (!call) return;
        if (m.type === 'progress') {
          call.progress?.(m.text ?? '');
          return;
        }
        this.pending.delete(m.req);
        if (m.type === 'done' && m.out) {
          if (m.err?.trim() && m.prog) this.log.push({ prog: m.prog, text: m.err });
          call.resolve(m.out);
        } else call.reject(new RadianceError(m.message ?? 'a Radiance program failed', m.log ?? '', m.prog ?? ''));
      });
      // a worker that cannot start, throws outside a run, or sends a reply that cannot be read ends every run still waiting
      w.addEventListener('error', (ev: ErrorEvent) => {
        ev.preventDefault();
        this.stop(`a Radiance worker failed${ev.message ? `: ${ev.message}` : ' to start'}`);
      });
      w.addEventListener('messageerror', () => this.stop('a Radiance worker sent a reply that could not be read'));
      this.workers.push(w);
    }
    return this.workers[k];
  }

  /** Runs `prog` in worker `k`; resolves with its standard output. `progress` receives each new batch of whole lines. */
  private async run(k: number, prog: string, args: string[], files: Record<string, Uint8Array>, stdin?: Uint8Array, progress?: (text: string) => void): Promise<Uint8Array> {
    const module = this.compileEach ? undefined : await this.module(prog);
    if (this.stopped !== null) throw new Error(this.stopped);
    const w = this.worker(k);
    const req = ++this.req;
    return new Promise<Uint8Array>((resolve, reject) => {
      this.pending.set(req, { resolve, reject, progress });
      w.postMessage({ type: 'run', req, prog, glue: `${RADIANCE_BASE}${prog}.js`, wasm: `${RADIANCE_BASE}${prog}.wasm`, module, args: [...args], files, stdin, stream: !!progress });
    });
  }

  /**
   * Irradiance-based values per sensor: the scene and `sky` (Radiance text) into
   * an octree, then rtrace -I with `detail` over the sensors, one contiguous
   * share per worker. Returns lux per sensor. The scene's octree is kept for
   * the next run with the same `sceneKey`, which then only adds its sky.
   */
  async illuminance(
    materials: string,
    scene: string,
    sceneKey: string,
    sky: string,
    sensors: string[],
    detail: Detail,
    hooks: RadianceHooks,
  ): Promise<{ lux: Float64Array; octreeBytes: number; workers: number; parts: number; sceneReused: boolean; resolution: string; setOverflows: number }> {
    this.log = [];
    const [rayinit, skybright, perezlum] = await Promise.all(['rayinit.cal', 'skybright.cal', 'perezlum.cal'].map((f) => this.file(f)));
    const sceneReused = this.sceneOct?.key === sceneKey;
    if (!sceneReused) {
      this.sceneOct = null;
      const files = { 'materials.rad': enc(materials), 'scene.rad': enc(scene) };
      const overflows = (): number => (this.log.filter((l) => l.prog === 'oconv').at(-1)?.text.match(/set overflow/g) ?? []).length;
      hooks.stage('oconv', `building the octree of ${(scene.length / 2 ** 20).toFixed(1)} MB of scene text`);
      // -r 1024: the octree stops subdividing at 1/1024 of the scene's size, which keeps it small; where more
      // surfaces than a voxel can list meet at that size ("set overflow", and oconv leaves some out), it is
      // built again at oconv's default resolution
      let oct = await this.run(0, 'oconv', ['-r', '1024', '-f', 'materials.rad', 'scene.rad'], files);
      let resolution = '-r 1024';
      let over = overflows();
      if (over) {
        hooks.stage('oconv', `${over.toLocaleString('en')} set overflows at the coarser resolution: building the octree again at the default one`);
        const fine = await this.run(0, 'oconv', ['-f', 'materials.rad', 'scene.rad'], files);
        // the finer octree is kept only where it places more surfaces: it is larger, so fewer workers fit
        if (overflows() < over) [oct, over, resolution] = [fine, overflows(), 'default (-r 16384)'];
      }
      this.sceneOct = { key: sceneKey, oct, resolution, setOverflows: over };
    }
    hooks.stage('oconv-sky', sceneReused ? 'adding the sky to the scene’s octree from the last run' : 'adding the sky');
    const oct = await this.run(0, 'oconv', ['-f', '-i', 'scene.oct', 'sky.rad', 'glow.rad'], { 'scene.oct': this.sceneOct!.oct, 'sky.rad': enc(sky), 'glow.rad': enc(GLOW) });
    const n = sensors.length;
    // each worker holds the octree and rtrace's own memory: as many as about three quarters of the device's
    // memory allows (browsers report at most 8 GB, so this is a floor on larger machines)
    const budget = ((navigator as { deviceMemory?: number }).deviceMemory ?? 8) * 2 ** 30 * 0.75;
    const fit = Math.max(1, Math.floor(budget / workerBytes(oct.length)));
    const shares = Math.max(1, Math.min(this.size, fit, Math.ceil(n / 16)));
    // contiguous chunks, taken by the next free worker: one per worker, or three per worker when each would still
    // hold 500 sensors or more, so a worker done with an easy part of the building takes on what is left instead of
    // waiting for the slowest share (each chunk is one rtrace run with its own irradiance cache)
    const chunks = n / (shares * 3) >= 500 ? shares * 3 : shares;
    const per = Math.ceil(n / chunks);
    const out = new Float64Array(n);
    // per worker: sensors done, and sensors taken so far
    const workers: [number, number][] = Array.from({ length: shares }, () => [0, 0]);
    let done = 0;
    let next = 0;
    hooks.stage('rtrace', `${n.toLocaleString('en')} sensors in ${shares} worker${shares === 1 ? '' : 's'}${chunks > shares ? `, ${chunks} parts` : ''}`);
    hooks.progress(0, n, workers, next >= chunks);
    await Promise.all(
      workers.map(async (_, k) => {
        while (next < chunks) {
          const first = next++ * per;
          const size = Math.max(0, Math.min(n, first + per) - first);
          if (!size) continue;
          workers[k][1] += size;
          const part = sensors.slice(first, first + size);
          let got = 0;
          const take = (text: string): void => {
            for (const l of text.split('\n')) {
              if (!l.trim() || got >= size) continue;
              const v = lux(l);
              out[first + got] = v;
              hooks.value?.(first + got, v);
              got++;
              done++;
              workers[k][0]++;
            }
            hooks.progress(done, n, workers, next >= chunks);
          };
          const text = dec(
            await this.run(k, 'rtrace', ['-I', '-h', ...detail.args, 'sky.oct'], { 'sky.oct': oct, 'rayinit.cal': rayinit, 'skybright.cal': skybright, 'perezlum.cal': perezlum }, enc(`${part.join('\n')}\n`), take),
          );
          const lines = text.trim().split('\n');
          if (lines.length !== part.length) throw new Error(`rtrace gave ${lines.length} values for ${part.length} sensors`);
          // the whole output is the answer: lines the progress messages did not carry are read here
          take(lines.slice(got).join('\n'));
        }
      }),
    );
    return { lux: out, octreeBytes: oct.length, workers: shares, parts: chunks, sceneReused, resolution: this.sceneOct!.resolution, setOverflows: this.sceneOct!.setOverflows };
  }

  /** Ends the idle workers, so their WebAssembly memory is returned at once (the next run starts new ones). */
  release(): void {
    if (this.pending.size) return;
    for (const w of this.workers) w.terminate();
    this.workers = [];
  }

  /** Clears an earlier Stop: call when a Radiance analysis starts. */
  start(): void {
    this.stopped = null;
  }

  /** Forgets the kept octree (a new model). */
  forget(): void {
    this.sceneOct = null;
  }

  /** gensky's CIE overcast sky with 10,000 lux of unobstructed horizontal illuminance, so DF % = lux / 100. */
  async overcastSky(): Promise<string> {
    return dec(await this.run(0, 'gensky', ['-ang', '45', '0', '-c', '-B', '55.866'], {}));
  }

  /** gendaylit's Perez sky for local standard time `hour` on `month`/`day`, from DNI and DHI (W/m²). */
  async perezSky(month: number, day: number, hour: number, site: { latitude: number; longitude: number; timeZone: number }, dni: number, dhi: number): Promise<string> {
    const args = [String(month), String(day), hour.toFixed(4), '-a', String(site.latitude), '-o', String(-site.longitude), '-m', String(-15 * site.timeZone), '-W', String(dni), String(dhi)];
    return dec(await this.run(0, 'gendaylit', args, {}));
  }
}

/** One hour of weather, timed by the middle of its interval in UTC (minutes from 1 January 00:00 of a non-leap year). */
export interface WeatherHour {
  utcMinute: number;
  dni: number;
  dhi: number;
}

/** The hour of weather chosen for a moment, in the site's local standard time. */
export interface WeatherAt {
  month: number;
  day: number;
  hour: number;
  dni: number;
  dhi: number;
}

export interface Weather {
  hours: WeatherHour[];
  /** Where the numbers come from, as recorded with every result. */
  provenance: Record<string, string | number | boolean | null>;
  /** The credit the source asks for, shown with the result. */
  credit: string | null;
}

export const PVGIS_CREDIT = 'PVGIS © European Communities, 2001-2026';

/** The official PVGIS 5.3 typical-meteorological-year request for a site (JSON, with the DEM horizon). */
export function pvgisUrl(lat: number, lon: number): string {
  return `https://re.jrc.ec.europa.eu/api/v5_3/tmy?lat=${lat.toFixed(4)}&lon=${lon.toFixed(4)}&usehorizon=1&outputformat=json`;
}

const YEAR_MIN = 365 * 24 * 60;
const MONTH_START = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
const minuteOfYear = (month: number, day: number, hour: number): number => (MONTH_START[month - 1] + day - 1) * 1440 + hour * 60;

/**
 * Reads a PVGIS TMY (JSON, `outputformat=json`) or an EnergyPlus weather file
 * (EPW), keeping every hour in UTC so the site's time zone applies when an
 * hour is used. PVGIS stamps are UTC, plus its stated irradiance time offset;
 * EPW hours are the file's local standard time (its LOCATION line's zone), each
 * the hour ending at its stamp, so its middle is half an hour earlier. A 29
 * February row is skipped, and an EPW row with the missing-value marker
 * (9999) in direct normal or diffuse horizontal irradiance.
 */
export function parseWeather(name: string, text: string, fileDate: string): Weather {
  const t = text.trimStart();
  if (t.startsWith('{')) {
    const j = JSON.parse(t) as {
      inputs?: { location?: Record<string, number>; meteo_data?: Record<string, unknown> };
      outputs?: { tmy_hourly?: Record<string, unknown>[] };
    };
    const rows = j.outputs?.tmy_hourly;
    if (!Array.isArray(rows)) throw new Error(`${name}: not a PVGIS TMY (no outputs.tmy_hourly)`);
    const loc = j.inputs?.location ?? {};
    const meteo = j.inputs?.meteo_data ?? {};
    const offset = Number(loc.irradiance_time_offset ?? 0) || 0;
    const hours: WeatherHour[] = [];
    for (const r of rows) {
      const m = /^(\d{4})(\d{2})(\d{2}):(\d{2})(\d{2})$/.exec(String(r['time(UTC)'] ?? ''));
      if (!m) continue;
      const [month, day] = [Number(m[2]), Number(m[3])];
      if (month === 2 && day === 29) continue;
      const utcMinute = (minuteOfYear(month, day, Number(m[4]) + Number(m[5]) / 60 + offset) + YEAR_MIN) % YEAR_MIN;
      hours.push({ utcMinute, dni: Number(r['Gb(n)']) || 0, dhi: Number(r['Gd(h)']) || 0 });
    }
    return {
      hours,
      credit: PVGIS_CREDIT,
      provenance: {
        source: 'PVGIS 5.3 typical meteorological year (re.jrc.ec.europa.eu)',
        file: name,
        radiationDatabase: String(meteo.radiation_db ?? ''),
        meteoDatabase: String(meteo.meteo_db ?? ''),
        latitude: loc.latitude ?? null,
        longitude: loc.longitude ?? null,
        elevation: loc.elevation ?? null,
        years: meteo.year_min !== undefined ? `${meteo.year_min}-${meteo.year_max}` : null,
        horizon: meteo.use_horizon ? String(meteo.horizon_db ?? 'yes') : 'none',
        irradianceTimeOffsetHours: offset,
        downloaded: fileDate,
      },
    };
  }
  const lines = t.split(/\r?\n/);
  const head = (lines[0] ?? '').split(',');
  if (head[0] !== 'LOCATION') throw new Error(`${name}: neither a PVGIS JSON TMY nor an EPW file`);
  const fileZone = Number(head[8]) || 0;
  const hours: WeatherHour[] = [];
  let missing = 0;
  for (const l of lines.slice(8)) {
    const c = l.split(',');
    if (c.length < 16) continue;
    const [month, day] = [Number(c[1]), Number(c[2])];
    if (!(month >= 1 && month <= 12) || (month === 2 && day === 29)) continue;
    const [dni, dhi] = [Number(c[14]), Number(c[15])];
    if (!(dni >= 0 && dni < 9999 && dhi >= 0 && dhi < 9999)) {
      missing++;
      continue;
    }
    hours.push({ utcMinute: (minuteOfYear(month, day, Number(c[3]) - 0.5 - fileZone) + YEAR_MIN) % YEAR_MIN, dni, dhi });
  }
  return {
    hours,
    credit: null,
    provenance: {
      source: 'EnergyPlus weather file (EPW) chosen by you',
      file: name,
      location: head.slice(1, 4).join(', '),
      dataSource: head[4] ?? null,
      latitude: Number(head[6]),
      longitude: Number(head[7]),
      timeZone: fileZone,
      elevation: Number(head[9]),
      hoursMissing: missing,
      loaded: fileDate,
    },
  };
}

/** The weather hour nearest local standard time `hour` on `month`/`day` at a site `timeZone` hours from UTC, within 90 minutes. */
export function weatherAt(w: Weather, month: number, day: number, hour: number, timeZone: number): WeatherAt | null {
  const want = (minuteOfYear(month, day, hour - timeZone) + YEAR_MIN) % YEAR_MIN;
  let best: WeatherHour | null = null;
  let gap = Infinity;
  for (const h of w.hours) {
    const d = Math.abs(h.utcMinute - want);
    const g = Math.min(d, YEAR_MIN - d);
    if (g < gap) {
      gap = g;
      best = h;
    }
  }
  if (!best || gap > 90) return null;
  const local = (best.utcMinute + timeZone * 60 + YEAR_MIN) % YEAR_MIN;
  const doy = Math.floor(local / 1440);
  let m = 11;
  while (MONTH_START[m] > doy) m--;
  return { month: m + 1, day: doy - MONTH_START[m] + 1, hour: (local % 1440) / 60, dni: best.dni, dhi: best.dhi };
}
