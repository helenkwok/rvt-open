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
 * Protocol: every message carries `req`, which the reply repeats.
 *   main → worker: { type: 'init', req, module, bits }   starts the decoder's wasm, no scene
 *   main → worker: { type: 'load', req, module, bits, id, positions, modifier, materials }
 *                  `materials` is 4 numbers per material (kind 0 plastic / 1 glass, three channels)
 *   main → worker: { type: 'run', req, id, sensors, first, kind, params }   value: Float64Array, one per sensor
 *   main → worker: { type: 'sun', req, site, day, hour }   value: daylightSunAt's numbers
 *   main → worker: { type: 'suns', req, site, from, to, step }   value: daylightSunPositions' Float64Array
 *   main → worker: { type: 'radtext', req, positions, modifier, materials }   value: [materials, scene,
 *                  degenerate, duplicate]: the materials as Radiance text, the scene as Radiance text encoded
 *                  as UTF-8 bytes (transferred: a large scene's text is hundreds of megabytes, and the page never
 *                  needs it as a string), and the counts of the triangles left out (daylightRadianceText)
 *   worker → main: { type: 'ok', req, value, ms } or { type: 'error', req, message }
 */

import * as glue32 from '../pkg/rvt_open.js';
import * as glue64 from '../pkg/rvt_open64.js';
import type { Bits } from './wasm-build';

type Glue = typeof glue32;
let glue: Glue | null = null;

type Msg = { req: number } & (
  | { type: 'init'; module: WebAssembly.Module; bits: Bits }
  | { type: 'load'; module: WebAssembly.Module; bits: Bits; id: number; positions: Float64Array; modifier: Uint32Array; materials: Float64Array }
  | { type: 'run'; id: number; sensors: Float64Array; first: number; kind: string; params: Float64Array }
  | { type: 'sun'; site: unknown; day: number; hour: number }
  | { type: 'suns'; site: unknown; from: number; to: number; step: number }
  | { type: 'radtext'; positions: Float64Array; modifier: Uint32Array; materials: Float64Array }
);

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
  const ok = (value: unknown, transfer: Transferable[] = []): void => send({ type: 'ok', req: msg.req, value, ms: performance.now() - t0 }, transfer);
  try {
    switch (msg.type) {
      case 'init':
        ready(msg.module, msg.bits);
        ok(null);
        return;
      case 'load':
        ready(msg.module, msg.bits).daylightLoad(msg.id, msg.positions, msg.modifier, msg.materials);
        ok(msg.id);
        return;
      case 'run': {
        const values = new Float64Array(ready().daylightRun(msg.id, msg.sensors, msg.first, msg.kind, msg.params));
        ok(values, [values.buffer]);
        return;
      }
      case 'sun':
        ok(Array.from(ready().daylightSunAt(msg.site, msg.day, msg.hour)));
        return;
      case 'suns':
        ok(new Float64Array(ready().daylightSunPositions(msg.site, msg.from, msg.to, msg.step)));
        return;
      case 'radtext': {
        const [materials, scene, degenerate, duplicate] = ready().daylightRadianceText(msg.positions, msg.modifier, msg.materials);
        const bytes = new TextEncoder().encode(scene);
        ok([materials, bytes, degenerate, duplicate], [bytes.buffer]);
        return;
      }
    }
  } catch (err) {
    send({ type: 'error', req: msg.req, message: err instanceof Error ? err.message : String(err) });
  }
});
