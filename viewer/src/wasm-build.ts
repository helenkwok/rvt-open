/*
 * The two decoder builds the viewer carries and how the page and its workers
 * choose between them. The package is what tools/ci/wasm-package.sh makes:
 * `rvt_open` (32-bit, at most 4 GiB of wasm memory) and `rvt_open64`
 * (memory64, at most 16 GiB in a browser), loaded by rvt_open_auto.js.
 *
 * The page asks for a build with `?bits=32` or `?bits=64` (`?bits=auto` is the engine's choice; or the build's own
 * default, RVT_VIEWER_BITS at `vite build`, which the browser tests use to run
 * the whole suite on one build). Asked for, a build that does not load is an
 * error; not asked for, the 64-bit build is used where the engine validates
 * memory64 and the 32-bit one where it does not or where the 64-bit one fails
 * to instantiate. The worker says which one it ran (`bits`).
 */
import { supportsMemory64, type WasmName } from '../pkg/rvt_open_auto.js';

export type Bits = 32 | 64;
export type { WasmName };

/** The most wasm memory each build can address: wasm32's 4 GiB, and the 16 GiB a browser caps a 64-bit memory at. */
export const MEMORY_LIMIT_BYTES: Record<Bits, number> = { 32: 4 * 2 ** 30, 64: 16 * 2 ** 30 };

/** Where each build's wasm is, resolved by the page (a worker started from a blob URL has no base). */
export type WasmUrls = Record<WasmName, string>;

export const wasmName = (bits: Bits): WasmName => (bits === 64 ? 'rvt_open64' : 'rvt_open');

/** The build the page asked for, or undefined to let the engine decide: `?bits=` over the build's default. */
export function requestedBits(search: string, buildDefault: string): Bits | undefined {
  for (const value of [new URLSearchParams(search).get('bits'), buildDefault]) {
    if (value === '32') return 32;
    if (value === '64') return 64;
    // `?bits=auto` asks for the engine's choice over a build's default.
    if (value === 'auto') return undefined;
  }
  return undefined;
}

/** The build an open is expected to use before a worker has said: the asked-for one, else what the engine accepts. */
export function plannedBits(requested: Bits | undefined): Bits {
  return requested ?? (supportsMemory64() ? 64 : 32);
}

export function describeBuild(bits: Bits): string {
  return `${bits}-bit decoder · up to ${MEMORY_LIMIT_BYTES[bits] / 2 ** 30} GiB`;
}
