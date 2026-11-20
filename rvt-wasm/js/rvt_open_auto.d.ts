/** The two builds `load` chooses between: `rvt_open` is 32-bit (4 GiB), `rvt_open64` is memory64 (16 GiB). */
export type WasmName = 'rvt_open' | 'rvt_open64';

/** The API of either build: the two export the same functions (only their raw wasm exports differ in pointer width). */
export type Decoder = typeof import('./rvt_open.js') | typeof import('./rvt_open64.js');

/** What `load` needs of a glue module: a `default` initialiser that gives the instance's memory. */
export interface GlueModule {
  default: (...args: any[]) => Promise<{ memory: WebAssembly.Memory }>;
}

export function supportsMemory64(): boolean;

/** A page that bundles the two glue files hands them over (`glue`), and chooses the type. */
export function load<M extends GlueModule>(options: {
  bits?: 32 | 64;
  base?: URL | string;
  wasm?: (name: WasmName) => unknown | Promise<unknown>;
  glue: (name: WasmName) => M | Promise<M>;
}): Promise<{ module: M; memory: WebAssembly.Memory; bits: 32 | 64 }>;
/** Imports the build's glue from `base` (default: this file's folder). */
export function load(options?: {
  /** Force a build; absent picks the 64-bit one where the engine accepts memory64, else the 32-bit one. */
  bits?: 32 | 64;
  /** Where the package's files are, as a URL. */
  base?: URL | string;
  wasm?: (name: WasmName) => unknown | Promise<unknown>;
}): Promise<{ module: Decoder; memory: WebAssembly.Memory; bits: 32 | 64 }>;
