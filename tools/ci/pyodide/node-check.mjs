// node node-check.mjs WHEEL MODEL: loads the wheel in Pyodide (npm package, no network), runs ops.py on MODEL and
// prints its `OPS {...}` line, with the Pyodide start-up and wheel install times.
import fs from 'node:fs';
import { loadPyodide } from 'pyodide';

const [wheel, model] = process.argv.slice(2);
const here = new URL('.', import.meta.url);
const seconds = (t) => Math.round(performance.now() - t) / 1000;
let t = performance.now();
const py = await loadPyodide();
const load = seconds(t);
t = performance.now();
py.unpackArchive(new Uint8Array(fs.readFileSync(wheel)), 'wheel');
const install = seconds(t);
py.FS.writeFile('/tmp/model.rvt', fs.readFileSync(model));
py.FS.writeFile('/tmp/ops.py', fs.readFileSync(new URL('ops.py', here)));
const out = await py.runPythonAsync("import sys; sys.path.insert(0, '/tmp')\nimport ops, json\n'OPS ' + json.dumps(ops.run('/tmp/model.rvt', 'pyodide'))");
const rec = JSON.parse(out.slice(4));
console.log('OPS ' + JSON.stringify({ ...rec, pyodide_start_s: load, wheel_unpack_s: install, wheel_bytes: fs.statSync(wheel).size }));
