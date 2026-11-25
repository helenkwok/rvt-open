// The Radiance WebAssembly build against native Radiance 6.0.3 on the CC0 demo building.
//
//   node tools/radiance/check-demo.mjs WASM_DIR [DATA_DIR]
//
// Runs gensky -> oconv -> oconv -i -> rtrace (the daylight factor at -ab 2 -aa 0.1 -ad 2048 -ar 64, one process) on
// the reference scene and sensors in daylight-rt/tests/data/demo-building, then checks:
// - the daylight factor against native Radiance (df_p1.dat): RMSE within the noise between two native runs
//   (df_p1_n8.dat against df_p1.dat) plus 0.01 DF points, means within 0.5 %. The runs are not bit-identical:
//   the Monte Carlo samples come from the C library's generator, which differs between glibc and Emscripten's
//   libc, so a WebAssembly run differs from a native one as much as two native runs do.
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

const [wasmDir, dataArg] = process.argv.slice(2);
if (!wasmDir) throw new Error('usage: check-demo.mjs WASM_DIR [DATA_DIR]');
const data = dataArg ?? path.resolve(path.dirname(new URL(import.meta.url).pathname), '../../daylight-rt/tests/data/demo-building');
const rd = (f) => readFileSync(path.join(data, f));
const lib = (f) => readFileSync(path.join(wasmDir, f));

async function run(prog, args, files = {}, stdin = null) {
  const factory = (await import(pathToFileURL(path.join(wasmDir, `${prog}.js`)).href)).default;
  const inb = stdin == null ? null : typeof stdin === 'string' ? new TextEncoder().encode(stdin) : stdin;
  let i = 0;
  const out = [];
  const err = [];
  const M = await factory({
    noInitialRun: true,
    stdin: () => (inb && i < inb.length ? inb[i++] : null),
    stdout: (c) => c !== null && out.push(c & 255),
    stderr: (c) => c !== null && err.push(c & 255),
  });
  for (const [n, d] of Object.entries(files)) M.FS.writeFile(n, d);
  let code = 0;
  try {
    code = M.callMain([...args]);
  } catch (e) {
    if (e && e.name === 'ExitStatus') code = e.status;
    else throw e;
  }
  if (code) throw new Error(`${prog} exited ${code}: ${new TextDecoder().decode(Uint8Array.from(err))}`);
  return Uint8Array.from(out);
}

const df = (text) =>
  text
    .trim()
    .split('\n')
    .map((l) => l.trim().split(/\s+/).map(Number))
    .map(([r, g, b]) => (100 * 179 * (0.265 * r + 0.67 * g + 0.065 * b)) / 10000);
const mean = (v) => v.reduce((a, b) => a + b, 0) / v.length;
const rmse = (a, b) => Math.sqrt(a.reduce((s, x, k) => s + (x - b[k]) ** 2, 0) / a.length);

const t0 = performance.now();
const sky = await run('gensky', ['-ang', '45', '0', '-c', '-B', '55.866']);
const scene = await run('oconv', ['-f', 'materials.rad', 'scene.rad'], { 'materials.rad': rd('materials.rad'), 'scene.rad': rd('scene.rad') });
const oct = await run('oconv', ['-f', '-i', 'scene.oct', 'sky.rad', 'glow.rad'], { 'scene.oct': scene, 'sky.rad': sky, 'glow.rad': rd('glow.rad') });
const t1 = performance.now();
const out = await run('rtrace', ['-I', '-h', '-ab', '2', '-aa', '0.1', '-ad', '2048', '-ar', '64', 'df.oct'],
  { 'df.oct': oct, 'skybright.cal': lib('skybright.cal'), 'rayinit.cal': lib('rayinit.cal') }, rd('grid.pts'));
const t2 = performance.now();
const ours = df(new TextDecoder().decode(out));
if (process.env.RADIANCE_CHECK_OUT) (await import('node:fs')).writeFileSync(process.env.RADIANCE_CHECK_OUT, out);
const native = df(rd('df_p1.dat').toString());
const native8 = df(rd('df_p1_n8.dat').toString());
const floor = rmse(native8, native);
const e = rmse(ours, native);
console.log(`sensors ${ours.length}; scene ${((t1 - t0) / 1000).toFixed(2)} s, rtrace ${((t2 - t1) / 1000).toFixed(1)} s`);
console.log(`daylight factor: means ${mean(ours).toFixed(4)} (wasm) / ${mean(native).toFixed(4)} (native) %; RMSE ${e.toFixed(4)}; native against native ${floor.toFixed(4)}`);
let failed = false;
const check = (ok, what) => { if (!ok) { console.error(`FAIL: ${what}`); failed = true; } };
check(ours.length === 391, 'one value per sensor');
check(e <= floor + 0.01, `RMSE ${e.toFixed(4)} above the noise floor ${floor.toFixed(4)} + 0.01`);
check(Math.abs(mean(ours) / mean(native) - 1) <= 0.005, 'means differ by more than 0.5 %');
process.exit(failed ? 1 : 0);
