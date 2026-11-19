// A consumer of the browser package, type-checked with the viewer (`tsc --noEmit`, which `pnpm run build` runs): the
// loader's result is usable without saying which build it is, and the declarations of the two builds agree where a
// caller can see them. The raw `__wbindgen_add_to_stack_pointer` is a number in both glue declarations
// (rvt_open.d.ts, rvt_open64.d.ts); the bigint of the 64-bit build is in its `.wasm.d.ts`, which is not the JS API,
// and tools/ci/wasm64-build.sh wraps the pointer in BigInt() at every call site inside the 64-bit glue.
import { load, type Decoder } from '../pkg/rvt_open_auto.js';
import type * as Glue32 from '../pkg/rvt_open.js';
import type * as Glue64 from '../pkg/rvt_open64.js';

export async function consumer(bytes: Uint8Array): Promise<number> {
  const { module, memory, bits } = await load();
  const decoder: Decoder = module;
  const summary: unknown = decoder.quickSummary(bytes);
  const instance = await decoder.default({ module_or_path: new WebAssembly.Module(new Uint8Array(8)) });
  // Callable whichever build it is: the pointer parameter has one type in both.
  instance.__wbindgen_add_to_stack_pointer(0);
  return bits + memory.buffer.byteLength + (summary === undefined ? 0 : 1);
}

// The exported functions a caller uses have one signature in both builds, so the union above hides no difference.
type Same<A, B> = [A] extends [B] ? ([B] extends [A] ? true : never) : never;
export const sameQuickSummary: Same<typeof Glue32.quickSummary, typeof Glue64.quickSummary> = true;
export const sameOpen: Same<
  typeof Glue32.openRvtBytesWithDiagnosticsModeAndOpenLimits,
  typeof Glue64.openRvtBytesWithDiagnosticsModeAndOpenLimits
> = true;
export const sameGlb: Same<typeof Glue32.modelToGlb, typeof Glue64.modelToGlb> = true;
