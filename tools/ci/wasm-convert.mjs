// convert real RVT files with the built wasm package and print one JSON line per file:
//   node tools/ci/wasm-convert.mjs <pkg dir> <file.rvt>...   (RVT_WASM_NAME=rvt_open64 for the wasm64 package)
// Each line is {file, ok, ms, counts?, error?}. `counts` is elements_json({counts:true}), the same text
// `rvt-elements --counts` prints, so a caller can compare it with the native CLI. A panic must arrive as a
// thrown Error with a message (panic hook), never as a bare "unreachable" trap: `error` is checked for that.
// convert_all_reader (the file read by slices through a callback) must give every output of
// convert_all byte for byte; `reader` is "same" or the first key that differs.
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
const [pkg, ...files] = process.argv.slice(2);
const name = process.env.RVT_WASM_NAME || 'rvt_open'; // rvt_open64 for the wasm64 package
const mod = await import(pathToFileURL(path.join(pkg, `${name}.js`)).href);
await mod.default({ module_or_path: fs.readFileSync(path.join(pkg, `${name}_bg.wasm`)) });
for (const f of files) {
  const bytes = new Uint8Array(fs.readFileSync(f));
  const t0 = performance.now();
  const rec = { file: f, ok: false };
  try {
    const opts = { ifc4_timestamp: 0 };
    const r = mod.convert_all(bytes, opts);
    rec.ifc_bytes = r.ifc.length;
    const fd = fs.openSync(f, 'r');
    const s = mod.convert_all_reader(bytes.length, (o, n) => { const b = new Uint8Array(n); return b.subarray(0, fs.readSync(fd, b, 0, n, o)); }, opts);
    fs.closeSync(fd);
    const same = (a, b) => typeof a === 'string' ? a === b : Buffer.compare(Buffer.from(a), Buffer.from(b)) === 0;
    rec.reader = Object.keys(r).filter(k => k !== 'manifest').find(k => !same(r[k], s[k])) ?? 'same';
    if (rec.reader !== 'same') throw new Error(`convert_all_reader differs from convert_all in ${rec.reader}`);
    rec.counts = JSON.parse(mod.elements_json(bytes, { counts: true }));
    rec.ok = true;
  } catch (e) {
    rec.error = String(e && e.message ? e.message : e);
  }
  rec.ms = Math.round(performance.now() - t0);
  console.log(JSON.stringify(rec));
  if (!rec.ok) break; // the wasm instance is unusable after a panic
}
