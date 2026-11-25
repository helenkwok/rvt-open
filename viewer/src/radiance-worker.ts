/*
 * Runs one Radiance program (gensky, gendaylit, oconv or rtrace, compiled to
 * WebAssembly by tools/radiance/build-wasm.sh) per request, for the Daylight
 * panel's validated tier (radiance.ts).
 *
 * Each program is an Emscripten ES module with an in-memory file system; a run
 * creates a fresh instance (the runtime exits with the program) from the
 * WebAssembly.Module the page compiled once and hands to every worker, so a
 * worker compiles nothing and fetches only the program's small JS glue.
 *
 * Protocol:
 *   main → worker: { type: 'run', req, prog, glue (URL of <program>.js), wasm (its .wasm), module?, args, files: { name: bytes }, stdin? }
 *                  Without `module` the worker fetches and compiles `wasm` itself (?radiance-compile=each, to measure).
 *   worker → main: { type: 'done', req, out (stdout bytes), err (stderr text), ms }
 *   worker → main: { type: 'error', req, message }   (a non-zero exit included)
 */

interface RunMsg {
  type: 'run';
  req: number;
  prog: string;
  glue: string;
  wasm: string;
  module?: WebAssembly.Module;
  args: string[];
  files: Record<string, Uint8Array>;
  stdin?: Uint8Array;
}

interface EmModule {
  FS: { writeFile(name: string, data: Uint8Array): void };
  callMain(args: string[]): number;
}
type Factory = (opts: Record<string, unknown>) => Promise<EmModule>;

const send = (msg: unknown, transfer: Transferable[] = []): void => {
  (self as unknown as { postMessage: (m: unknown, t: Transferable[]) => void }).postMessage(msg, transfer);
};

const factories = new Map<string, Promise<Factory>>();

self.addEventListener('message', async (ev: MessageEvent<RunMsg>) => {
  const m = ev.data;
  if (m.type !== 'run') return;
  const t0 = performance.now();
  try {
    let f = factories.get(m.glue);
    if (!f) {
      f = import(/* @vite-ignore */ m.glue).then((g: { default: Factory }) => g.default);
      factories.set(m.glue, f);
      // a failed import is not kept: the next run tries again
      f.catch(() => factories.delete(m.glue));
    }
    const factory = await f;
    const out: number[] = [];
    const err: number[] = [];
    let i = 0;
    const stdin = m.stdin ?? null;
    const em = await factory({
      noInitialRun: true,
      // the page compiled the module: instantiate it, nothing is fetched or compiled here
      instantiateWasm: (imports: WebAssembly.Imports, done: (i: WebAssembly.Instance) => void) => {
        const fail = (e: unknown): void => send({ type: 'error', req: m.req, message: `${m.prog}: ${e instanceof Error ? e.message : String(e)}` });
        if (m.module) WebAssembly.instantiate(m.module, imports).then(done, fail);
        else WebAssembly.instantiateStreaming(fetch(m.wasm), imports).then((r) => done(r.instance), fail);
        return {};
      },
      stdin: () => (stdin && i < stdin.length ? stdin[i++] : null),
      stdout: (c: number | null) => {
        if (c !== null) out.push(c & 255);
      },
      stderr: (c: number | null) => {
        if (c !== null) err.push(c & 255);
      },
    });
    for (const [name, data] of Object.entries(m.files)) em.FS.writeFile(name, data);
    let code = 0;
    try {
      // callMain puts the program's name in front of the array it is given: pass a copy
      code = em.callMain([...m.args]);
    } catch (e) {
      const status = (e as { name?: string; status?: number }).name === 'ExitStatus' ? (e as { status: number }).status : null;
      if (status === null) throw e;
      code = status;
    }
    const errText = new TextDecoder().decode(Uint8Array.from(err));
    if (code !== 0) throw new Error(`${m.prog} exited ${code}: ${errText.slice(0, 400)}`);
    const bytes = Uint8Array.from(out);
    send({ type: 'done', req: m.req, out: bytes, err: errText, ms: performance.now() - t0 }, [bytes.buffer]);
  } catch (e) {
    send({ type: 'error', req: m.req, message: e instanceof Error ? e.message : String(e) });
  }
});
