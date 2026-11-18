/** The two builds `load` chooses between: `rvt_open` is 32-bit (4 GiB), `rvt_open64` is memory64 (16 GiB). */
export type WasmName = 'rvt_open' | 'rvt_open64';

export function supportsMemory64(): boolean;

export function load<M = unknown>(options?: {
  /** Force a build; absent picks the 64-bit one where the engine accepts memory64, else the 32-bit one. */
  bits?: 32 | 64;
  /** Where the package's files are, as a URL; used only when `glue` or `wasm` does not give them. */
  base?: URL | string;
  wasm?: (name: WasmName) => unknown | Promise<unknown>;
  glue?: (name: WasmName) => M | Promise<M>;
}): Promise<{ module: M; memory: WebAssembly.Memory; bits: 32 | 64 }>;
