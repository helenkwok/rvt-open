// load the 64-bit build (rvt_open64, memory64, up to 16 GiB) where the engine supports
// memory64, else the 32-bit build (rvt_open, 4 GiB). Both packages' files sit next to this one:
//   rvt_open.js, rvt_open_bg.wasm, rvt_open64.js, rvt_open64_bg.wasm, rvt_open_auto.js
//
//   const { module, memory, bits } = await (await import('./rvt_open_auto.js')).load();
//   module.convert_all(bytes, {}, progress);
//
// `load({ bits: 32 })` forces the 32-bit build. `wasm(name)` may return the .wasm bytes, a Response, a URL
// or a compiled WebAssembly.Module for `name` ("rvt_open" or "rvt_open64") where fetching the file next to
// this one does not work (Node, a bundled worker). `glue(name)` may return the already imported glue module
// of `name`, for a page that bundles the two glue files and so cannot import them by URL.

// The smallest module with a 64-bit memory: header, then a memory section holding one memory whose
// limits flag 0x04 means "64-bit address, no maximum", minimum 0 pages. Validating it asks the engine
// whether it accepts memory64 without creating a memory or touching the JS descriptor's key names.
const MEMORY64_PROBE = new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0, 5, 3, 1, 4, 0]);

export function supportsMemory64() {
  try {
    return WebAssembly.validate(MEMORY64_PROBE);
  } catch {
    return false;
  }
}

export async function load({ bits, base, wasm, glue } = {}) {
  const want64 = bits === undefined ? supportsMemory64() : bits === 64;
  // Only resolved when a file is fetched by URL: a bundled worker's own URL (a blob) is no base.
  const here = import.meta.url; // not a literal `new URL('.', import.meta.url)`: a bundler would try to resolve that at build time
  const url = (file) => new URL(file, base ?? new URL('.', here));
  const open = async (name) => {
    const module = glue ? await glue(name) : await import(/* @vite-ignore */ url(`${name}.js`).href);
    const exports = await module.default({ module_or_path: wasm ? await wasm(name) : url(`${name}_bg.wasm`) });
    return { module, memory: exports.memory, bits: name === 'rvt_open64' ? 64 : 32 };
  };
  if (want64) {
    try {
      return await open('rvt_open64');
    } catch (e) {
      if (bits === 64) throw e;
      // Validation passed but the 64-bit build would not load (missing file, engine refusal): use 32.
    }
  }
  return open('rvt_open');
}
