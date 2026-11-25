/*
 * One worker of the daylight panel's pool (daylight.ts): runs the instant
 * tier's jobs (daylightRun in rvt-wasm, the daylight-rt crate) on a share of
 * the sensors.
 *
 * Bundled into the page (`?worker&inline`) and started from the decoder's
 * already compiled wasm, so nothing is fetched. Every worker of the pool holds
 * its own copy of the scene's tracer; the page hands each the same compiled
 * module, compiled once.
 *
 * Protocol:
 *   main → worker: { type: 'load', module, bits, id, positions, modifier, materials }
 *                  `materials` is 4 numbers per material (kind 0 plastic / 1 glass, three channels)
 *   worker → main: { type: 'loaded', id, ms }
 *   main → worker: { type: 'run', id, job, sensors, first, kind, params }
 *   worker → main: { type: 'done', job, first, values, ms }
 *   main → worker: { type: 'sun', req, site, day, hour } / { type: 'suns', req, site, from, to, step }
 *   worker → main: { type: 'sun', req, value } (daylightSunAt) / { type: 'suns', req, value } (daylightSunPositions)
 *   Any failure replies { type: 'error', message, job?, req? }.
 */

import * as glue32 from '../pkg/rvt_open.js';
import * as glue64 from '../pkg/rvt_open64.js';
import type { Bits } from './wasm-build';

type Glue = typeof glue32;
let glue: Glue | null = null;

type Msg =
  | {
      type: 'load';
      module: WebAssembly.Module;
      bits: Bits;
      id: number;
      positions: Float64Array;
      modifier: Uint32Array;
      materials: Float64Array;
    }
  | { type: 'run'; id: number; job: number; sensors: Float64Array; first: number; kind: string; params: Float64Array }
  | { type: 'sun'; req: number; site: unknown; day: number; hour: number }
  | { type: 'suns'; req: number; site: unknown; from: number; to: number; step: number };

const send = (msg: unknown, transfer: Transferable[] = []): void => {
  (self as unknown as { postMessage: (m: unknown, t: Transferable[]) => void }).postMessage(msg, transfer);
};

function ready(module?: WebAssembly.Module, bits?: Bits): Glue {
  if (!glue) {
    if (!module) throw new Error('the daylight worker has no decoder module');
    const g = (bits === 64 ? glue64 : glue32) as Glue;
    g.initSync({ module });
    glue = g;
  }
  return glue;
}

self.addEventListener('message', (ev: MessageEvent<Msg>) => {
  const msg = ev.data;
  const t0 = performance.now();
  try {
    switch (msg.type) {
      case 'load':
        ready(msg.module, msg.bits).daylightLoad(msg.id, msg.positions, msg.modifier, msg.materials);
        send({ type: 'loaded', id: msg.id, ms: performance.now() - t0 });
        return;
      case 'run': {
        const values = new Float64Array(ready().daylightRun(msg.id, msg.sensors, msg.first, msg.kind, msg.params));
        send({ type: 'done', job: msg.job, first: msg.first, values, ms: performance.now() - t0 }, [values.buffer]);
        return;
      }
      case 'sun':
        send({ type: 'sun', req: msg.req, value: Array.from(ready().daylightSunAt(msg.site, msg.day, msg.hour)) });
        return;
      case 'suns':
        send({ type: 'suns', req: msg.req, value: new Float64Array(ready().daylightSunPositions(msg.site, msg.from, msg.to, msg.step)) });
        return;
    }
  } catch (err) {
    send({
      type: 'error',
      message: err instanceof Error ? err.message : String(err),
      job: msg.type === 'run' ? msg.job : undefined,
      req: msg.type === 'sun' || msg.type === 'suns' ? msg.req : undefined,
    });
  }
});
