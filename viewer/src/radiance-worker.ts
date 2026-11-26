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
 *                  `stream` asks for the output as it is written:
 *   worker → main: { type: 'progress', req, text }   the whole lines of standard output written since the last
 *                  one (rtrace: one per sensor), at most every 250 ms while the program runs; reading them changes
 *                  nothing it computes
 *   worker → main: { type: 'done', req, out (stdout bytes), err (stderr text), ms }
 *   worker → main: { type: 'error', req, message, log? }   (a non-zero exit included; `log` is the whole
 *                  standard error, `code` the exit status)
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
  /** Send each new batch of whole output lines while the program runs (rtrace: one line per sensor). */
  stream?: boolean;
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
  // standard error: the first 4 MiB, and always its last 4 KiB (where a fatal error is)
  const errBytes: number[] = [];
  let errTail: number[] = [];
  let errDropped = 0;
  const errText = (): string => {
    const d = new TextDecoder();
    return errDropped ? `${d.decode(Uint8Array.from(errBytes))}\n… ${errDropped.toLocaleString('en')} bytes not kept …\n${d.decode(Uint8Array.from(errTail))}` : d.decode(Uint8Array.from(errBytes));
  };
  try {
    let f = factories.get(m.glue);
    if (!f) {
      f = import(/* @vite-ignore */ m.glue).then((g: { default: Factory }) => g.default);
      factories.set(m.glue, f);
      // a failed import is not kept: the next run tries again
      f.catch(() => factories.delete(m.glue));
    }
    const factory = await f;
    // standard output in 1 MiB chunks: an octree can pass the length a plain array of numbers may reach (2^27)
    const chunks: Uint8Array[] = [];
    let chunk = new Uint8Array(1 << 20);
    let used = 0;
    let i = 0;
    // with `stream`, the bytes since the last progress message
    let fresh: number[] = [];
    let sent = performance.now();
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
        if (c === null) return;
        if (used === chunk.length) {
          chunks.push(chunk);
          chunk = new Uint8Array(1 << 20);
          used = 0;
        }
        chunk[used++] = c & 255;
        if (!m.stream) return;
        fresh.push(c & 255);
        if (c === 10) {
          const now = performance.now();
          if (now - sent >= 250) {
            sent = now;
            send({ type: 'progress', req: m.req, text: new TextDecoder().decode(Uint8Array.from(fresh)) });
            fresh = [];
          }
        }
      },
      stderr: (c: number | null) => {
        if (c === null) return;
        if (errBytes.length < 1 << 22) errBytes.push(c & 255);
        else {
          errDropped++;
          errTail.push(c & 255);
          if (errTail.length > 8192) errTail = errTail.slice(-4096);
        }
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
    if (code !== 0) {
      send({ type: 'error', req: m.req, message: `${m.prog} exited ${code}`, log: errText(), code, prog: m.prog });
      return;
    }
    chunks.push(chunk.subarray(0, used));
    const bytes = new Uint8Array(chunks.reduce((a, c) => a + c.length, 0));
    let at = 0;
    for (const c of chunks) {
      bytes.set(c, at);
      at += c.length;
    }
    send({ type: 'done', req: m.req, out: bytes, err: errText(), ms: performance.now() - t0 }, [bytes.buffer]);
  } catch (e) {
    // a trap (out of memory, out of bounds) or a failed import: what the program wrote so far goes with it
    send({ type: 'error', req: m.req, message: `${m.prog}: ${e instanceof Error ? e.message : String(e)}`, log: errText(), prog: m.prog });
  }
});
