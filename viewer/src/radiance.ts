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

export class Radiance {
  private modules = new Map<string, Promise<WebAssembly.Module>>();
  private files = new Map<string, Promise<Uint8Array>>();
  private workers: Worker[] = [];
  private req = 0;
  private stopped = false;
  private pending = new Map<number, { resolve: (out: Uint8Array) => void; reject: (e: Error) => void }>();
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

  private module(prog: string): Promise<WebAssembly.Module> {
    let m = this.modules.get(prog);
    if (!m) {
      m = WebAssembly.compileStreaming(fetch(`${RADIANCE_BASE}${prog}.wasm`));
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

  /** Stops every worker and rejects every run still waiting. */
  stop(): void {
    this.stopped = true;
    for (const w of this.workers) w.terminate();
    this.workers = [];
    const calls = [...this.pending.values()];
    this.pending.clear();
    for (const c of calls) c.reject(new Error('stopped'));
  }

  private worker(k: number): Worker {
    while (this.workers.length <= k) {
      const w = new RadianceWorker({ name: `radiance-${this.workers.length}` }) as Worker;
      w.addEventListener('message', (ev: MessageEvent) => {
        const m = ev.data as { type: string; req: number; out?: Uint8Array; message?: string };
        const call = this.pending.get(m.req);
        if (!call) return;
        this.pending.delete(m.req);
        if (m.type === 'done' && m.out) call.resolve(m.out);
        else call.reject(new Error(m.message ?? 'a Radiance program failed'));
      });
      this.workers.push(w);
    }
    return this.workers[k];
  }

  /** Runs `prog` in worker `k`; resolves with its standard output. */
  private async run(k: number, prog: string, args: string[], files: Record<string, Uint8Array>, stdin?: Uint8Array): Promise<Uint8Array> {
    const module = await this.module(prog);
    if (this.stopped) throw new Error('stopped');
    const w = this.worker(k);
    const req = ++this.req;
    return new Promise<Uint8Array>((resolve, reject) => {
      this.pending.set(req, { resolve, reject });
      w.postMessage({ type: 'run', req, prog, glue: `${RADIANCE_BASE}${prog}.js`, module, args: [...args], files, stdin });
    });
  }

  /**
   * Irradiance-based values per sensor: the scene and `sky` (Radiance text) into
   * an octree, then rtrace -I with `detail` over the sensors, one contiguous
   * share per worker. Returns lux per sensor.
   */
  async illuminance(
    materials: string,
    scene: string,
    sky: string,
    sensors: string[],
    detail: Detail,
    progress: (done: number, total: number) => void,
  ): Promise<{ lux: Float64Array; octreeBytes: number }> {
    this.stopped = false;
    const [rayinit, skybright, perezlum] = await Promise.all(['rayinit.cal', 'skybright.cal', 'perezlum.cal'].map((f) => this.file(f)));
    const sceneOct = await this.run(0, 'oconv', ['-f', 'materials.rad', 'scene.rad'], { 'materials.rad': enc(materials), 'scene.rad': enc(scene) });
    const oct = await this.run(0, 'oconv', ['-f', '-i', 'scene.oct', 'sky.rad', 'glow.rad'], { 'scene.oct': sceneOct, 'sky.rad': enc(sky), 'glow.rad': enc(GLOW) });
    const n = sensors.length;
    const shares = Math.max(1, Math.min(this.size, Math.ceil(n / 16)));
    const per = Math.ceil(n / shares);
    const out = new Float64Array(n);
    let done = 0;
    progress(0, n);
    await Promise.all(
      Array.from({ length: shares }, async (_, k) => {
        const first = k * per;
        const part = sensors.slice(first, Math.min(n, first + per));
        if (!part.length) return;
        const text = dec(
          await this.run(k, 'rtrace', ['-I', '-h', ...detail.args, 'sky.oct'], { 'sky.oct': oct, 'rayinit.cal': rayinit, 'skybright.cal': skybright, 'perezlum.cal': perezlum }, enc(`${part.join('\n')}\n`)),
        );
        const lines = text.trim().split('\n');
        if (lines.length !== part.length) throw new Error(`rtrace gave ${lines.length} values for ${part.length} sensors`);
        lines.forEach((l, i) => (out[first + i] = lux(l)));
        done += part.length;
        progress(done, n);
      }),
    );
    return { lux: out, octreeBytes: oct.length };
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

/** One hour of weather: its local standard time (mid-interval), direct normal and diffuse horizontal irradiance. */
export interface WeatherHour {
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

const isLeapDay = (m: number, d: number): boolean => m === 2 && d === 29;

/**
 * Reads a PVGIS TMY (JSON, `outputformat=json`) or an EnergyPlus weather file
 * (EPW). PVGIS times are UTC hour stamps, moved to local standard time with the
 * site's time zone and its stated irradiance time offset; EPW hours are local
 * standard time, each the hour ending at its stamp, so its middle is half an
 * hour earlier.
 */
export function parseWeather(name: string, text: string, timeZone: number, fileDate: string): Weather {
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
      // a UTC stamp in its (non-leap) year, moved to local standard time
      const utc = Date.UTC(2001, Number(m[2]) - 1, Number(m[3]), Number(m[4]), Number(m[5]));
      const local = new Date(utc + (timeZone + offset) * 3600_000);
      const month = local.getUTCMonth() + 1;
      const day = local.getUTCDate();
      if (isLeapDay(month, day)) continue;
      hours.push({ month, day, hour: local.getUTCHours() + local.getUTCMinutes() / 60, dni: Number(r['Gb(n)']) || 0, dhi: Number(r['Gd(h)']) || 0 });
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
  const hours: WeatherHour[] = [];
  for (const l of lines.slice(8)) {
    const c = l.split(',');
    if (c.length < 16) continue;
    const month = Number(c[1]);
    const day = Number(c[2]);
    if (isLeapDay(month, day)) continue;
    hours.push({ month, day, hour: Number(c[3]) - 0.5, dni: Number(c[14]) || 0, dhi: Number(c[15]) || 0 });
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
      timeZone: Number(head[8]),
      elevation: Number(head[9]),
      loaded: fileDate,
    },
  };
}

/** The weather hour nearest local standard time `hour` on `month`/`day`. */
export function weatherAt(w: Weather, month: number, day: number, hour: number): WeatherHour | null {
  let best: WeatherHour | null = null;
  let gap = Infinity;
  for (const h of w.hours) {
    if (h.month !== month || h.day !== day) continue;
    const g = Math.abs(h.hour - hour);
    if (g < gap) {
      gap = g;
      best = h;
    }
  }
  return best;
}
