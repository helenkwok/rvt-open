// load the 64-bit build (rvt_open64, memory64, up to 16 GiB) where the engine supports
// memory64, else the 32-bit build (rvt_open, 4 GiB). Both packages' files sit next to this one:
//   rvt_open.js, rvt_open_bg.wasm, rvt_open64.js, rvt_open64_bg.wasm, rvt_open_auto.js
//
//   const { module, memory, bits } = await (await import('./rvt_open_auto.js')).load();
//   module.convert_all(bytes, {}, progress);
//
// `load({ bits: 32 })` forces the 32-bit build. `wasm(name)` may return the .wasm bytes or a Response
// for `name` ("rvt_open" or "rvt_open64") where fetching a URL does not work (Node).

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

export async function load({ bits, base = new URL('.', import.meta.url), wasm } = {}) {
  const want64 = bits === undefined ? supportsMemory64() : bits === 64;
  const open = async (name) => {
    const module = await import(new URL(`${name}.js`, base).href);
    const exports = await module.default({ module_or_path: wasm ? await wasm(name) : new URL(`${name}_bg.wasm`, base) });
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
