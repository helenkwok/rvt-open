/*
 * A Revit model's sheets and views as 2D drawings, beside the 3D view. The
 * Drawings list (left panel) comes from the views the file holds; a sheet or
 * view is drawn only when it is chosen, by the parse worker (worker.ts) as
 * `rvt-sheet --sheet` or `--view` draws it, and
 * shown in the 2D pane as SVG. The 3D view and the drawing stay as they are
 * while the other is shown: switching only hides one.
 */
import type { SvgView } from './drawing';
import { initDrawingSelection, type DrawingSelection, type DrawingSelectHost } from './drawing-select';

export interface ModelDrawingsHost {
  /** The worker that opened the model, or null. */
  worker(): Worker | null;
  /** The file it opened: read again for each request, so no copy is kept. */
  file(): File | null;
  stem(): string;
  /** The 3D frames drawn so far (main.ts): their count and time, the longest, the longest gap between two. */
  frameStats(): FrameStats;
  setStatus(text: string): void;
  toast(message: string, kind?: 'ok' | 'bad' | 'info'): void;
  download(filename: string, blob: Blob): void;
  /** Naming, chaining and reporting the elements selected in a drawing (drawing-select.ts). */
  select: Omit<DrawingSelectHost, 'setStatus'>;
}

type Kind = 'sheet' | 'plan' | 'section';

interface FrameStats {
  frames: number;
  ms: number;
  maxMs: number;
  maxGapMs: number;
}

/** A long-animation-frame entry, where the browser reports them. */
interface LongFrame {
  duration: number;
  blockingDuration?: number;
  scripts?: { duration: number; invoker?: string; sourceFunctionName?: string; sourceURL?: string; sourceCharPosition?: number }[];
}

/** Milliseconds since the epoch, on the clock the worker also reads. */
const clock = (): number => performance.timeOrigin + performance.now();

/**
 * Where a request's time went, from the moment it was made: each stage's time (ms since the
 * request), as the page and the worker marked them, and the 3D frames drawn meanwhile.
 */
class Timeline {
  private readonly start = clock();
  private readonly frames0: FrameStats;
  readonly marks: Record<string, number> = {};
  /** The page's long frames meanwhile (Chromium's long-animation-frame entries), and what ran in them. */
  private readonly long: { ms: number; blocking: number; scripts: { what: string; ms: number }[] }[] = [];
  private observer: PerformanceObserver | null = null;
  constructor(private readonly stats: () => FrameStats) {
    const f = stats();
    this.frames0 = { ...f };
    f.maxMs = 0;
    f.maxGapMs = 0;
    try {
      this.observer = new PerformanceObserver((list) => this.collect(list.getEntries()));
      this.observer.observe({ type: 'long-animation-frame', buffered: false });
    } catch {
      this.observer = null;
    }
  }
  private collect(entries: PerformanceEntryList): void {
    for (const e of entries as unknown as LongFrame[]) {
      this.long.push({
        ms: e.duration,
        blocking: e.blockingDuration ?? 0,
        scripts: (e.scripts ?? []).map((x) => ({
          what: `${x.invoker ?? ''} ${x.sourceFunctionName ?? ''} ${(x.sourceURL ?? '').split('/').pop() ?? ''}:${x.sourceCharPosition ?? ''}`.trim(),
          ms: x.duration,
        })),
      });
    }
  }
  mark(stage: string, at: number = clock()): void {
    this.marks[stage] = Math.round(at - this.start);
  }
  /** The worker's marks (`received`, `wasmStart`, `wasmEnd`, `posted`), on the shared clock. */
  worker(at: Record<string, number> | undefined): void {
    for (const [k, v] of Object.entries(at ?? {})) this.mark(`worker.${k}`, v);
  }
  private frames(): void {
    const f = this.stats();
    this.marks.frames = f.frames - this.frames0.frames;
    this.marks.frameMs = Math.round(f.ms - this.frames0.ms);
    this.marks.maxFrameMs = Math.round(f.maxMs);
    this.marks.maxGapMs = Math.round(f.maxGapMs);
  }
  /** The marks so far, and `epoch`: when the request was made, ms since the epoch. */
  json(): string {
    // Long frames the observer has queued but not yet handed over count too.
    if (this.observer) this.collect(this.observer.takeRecords());
    this.frames();
    const scripts = new Map<string, number>();
    for (const f of this.long) for (const x of f.scripts) scripts.set(x.what, (scripts.get(x.what) ?? 0) + x.ms);
    const top = [...scripts].sort((a, b) => b[1] - a[1]).slice(0, 6).map(([what, ms]) => `${Math.round(ms)} ${what}`);
    return JSON.stringify({
      epoch: Math.round(this.start),
      ...this.marks,
      longFrames: this.long.length,
      longFrameMs: Math.round(this.long.reduce((a, f) => a + f.ms, 0)),
      maxLongFrameMs: Math.round(Math.max(0, ...this.long.map((f) => f.ms))),
      longScripts: top,
    });
  }
  stop(): void {
    this.observer?.disconnect();
    this.observer = null;
  }
}

interface DrawingList {
  sheets: { id: number; number: string; name: string; views: number[] }[];
  plans: { id: number; name: string; level: string | null; ceiling: boolean; scale: number | null; onSheet: boolean }[];
  sections: { id: number; name: string; scale: number | null }[];
  /** Milliseconds per step of the listing. */
  times?: Record<string, number>;
}

interface Drawn {
  svg: string;
  layers: string[];
  truncated: boolean;
  layout: string | null;
  scale: number;
  counts: { lines: number; texts: number; viewports: number; entities: number };
  notes: string[];
  /** Milliseconds per step of the drawing (`model` and `views` only when the first drawing reads them). */
  times?: Record<string, number>;
}

type Reply =
  | { type: 'drawing-list'; list: DrawingList; at?: Record<string, number> }
  | { type: 'drawing-open'; kind: Kind; id: number; req: number; drawn: Drawn; at?: Record<string, number> }
  | { type: 'drawing-save'; format: string; bytes: Uint8Array }
  | { type: 'drawing-error'; request: string; req?: number; message: string };

/** A drawing asked for: `req` numbers the request, as the worker's reply repeats it. */
interface Request {
  kind: Kind;
  id: number;
  label: string;
  req: number;
  timeline: Timeline;
}

const $ = (id: string): HTMLElement => {
  const el = document.getElementById(id);
  if (!el) throw new Error(`missing element: #${id}`);
  return el;
};

const scaleText = (scale: number | null): string => (scale && scale > 0 ? ` · 1:${Math.round(scale)}` : '');

export function initModelDrawings(
  host: ModelDrawingsHost,
  view: SvgView,
): { load(): Promise<void>; reset(): void; showModel(): void; selection: DrawingSelection } {
  const app = $('app');
  const pane = $('drawing-pane');
  const sheetEl = $('drawing-sheet');
  const listEl = $('drawings');
  const titleEl = $('drawing-title');
  const selection = initDrawingSelection({ ...host.select, setStatus: host.setStatus }, sheetEl);
  const byFace = document.getElementById('drawing-by-face') as HTMLInputElement | null;
  byFace?.addEventListener('change', () => {
    selection.byFace = byFace.checked;
  });

  let shown: Request | null = null;
  let pending: Request | null = null;
  let nextReq = 0;
  // The worker draws one drawing at a time, in the order asked, keeps the last and saves what it
  // keeps: `posted` is the last request sent to it, `held` the one it keeps after its last
  // drawing reply (null when that failed, as a failed open has dropped the drawing before it).
  let posted: number | null = null;
  let held: number | null = null;
  let listening: Worker | null = null;
  let listTimeline: Timeline | null = null;

  /** Whether the worker keeps the drawing shown, with no other drawing asked for after it. */
  const holdsShown = (): boolean => shown !== null && posted === shown.req && held === shown.req;

  /** A request still waiting is not shown when it comes. */
  const cancelPending = (): void => {
    if (!pending) return;
    pending.timeline.stop();
    pending = null;
    listEl.removeAttribute('aria-busy');
  };

  const setPressed = (key: string): void => {
    for (const b of Array.from(listEl.querySelectorAll<HTMLButtonElement>('button[data-key]'))) {
      b.setAttribute('aria-pressed', String(b.dataset.key === key));
    }
  };

  /** Back to the 3D view; the drawing stays in the pane for the next time. */
  const showModel = (): void => {
    cancelPending();
    if (app.dataset.view !== 'drawing') return;
    delete app.dataset.view;
    pane.hidden = true;
    setPressed('model');
  };

  const showDrawing = (): void => {
    app.dataset.view = 'drawing';
    pane.hidden = false;
    setPressed(shown ? `${shown.kind}:${shown.id}` : 'model');
  };

  const onMessage = (ev: MessageEvent<Reply>): void => {
    const msg = ev.data;
    switch (msg.type) {
      case 'drawing-list': {
        const t = listTimeline;
        t?.mark('received');
        t?.worker(msg.at);
        renderList(msg.list);
        if (t) {
          t.mark('listed');
          listEl.dataset.timeline = t.json();
        }
        return;
      }
      case 'drawing-open': {
        held = msg.req;
        if (!pending || pending.req !== msg.req) return;
        const timeline = pending.timeline;
        timeline.mark('received');
        timeline.worker(msg.at);
        shown = pending;
        pending = null;
        listEl.removeAttribute('aria-busy');
        // Shown before the SVG goes in, so that it is laid out when it is fitted to what it draws.
        titleEl.textContent = shown.label;
        showDrawing();
        if (!view.show(msg.drawn.svg, true)) {
          timeline.stop();
          host.toast('The drawing could not be shown: the renderer returned no SVG.', 'bad');
          return;
        }
        const svg = view.svg();
        if (svg) {
          svg.dataset.lines = String(msg.drawn.counts.lines);
          svg.dataset.texts = String(msg.drawn.counts.texts);
          svg.dataset.viewports = String(msg.drawn.counts.viewports);
          svg.dataset.times = JSON.stringify(msg.drawn.times ?? {});
          timeline.mark('inserted');
          // The drawing's elements, for selecting them in it (the selection shown stays).
          const kept = selection.selected();
          selection.attach(svg);
          selection.set(kept);
          view.setPicker(selection);
          timeline.mark('indexed');
          svg.dataset.timeline = timeline.json();
          // Painted: the frame after the one that lays the drawing out.
          requestAnimationFrame(() =>
            requestAnimationFrame(() => {
              timeline.mark('painted');
              svg.dataset.timeline = timeline.json();
              timeline.stop();
            }),
          );
        }
        const c = msg.drawn.counts;
        host.setStatus(
          `${shown.label} · ${c.lines} lines · ${c.texts} texts${c.viewports > 0 ? ` · ${c.viewports} viewport${c.viewports === 1 ? '' : 's'}` : ''}`,
        );
        if (msg.drawn.truncated) host.toast('The drawing is larger than the render limit; part of it is not drawn.', 'info');
        for (const note of msg.drawn.notes) host.toast(note, 'info');
        sheetEl.focus();
        return;
      }
      case 'drawing-save': {
        const file = `${host.stem()}-${(shown?.label ?? 'drawing').replace(/[^\w.-]+/g, '_')}.${msg.format}`;
        const type = msg.format === 'pdf' ? 'application/pdf' : 'application/octet-stream';
        host.download(file, new Blob([msg.bytes as BlobPart], { type }));
        host.toast(`Downloaded ${file}`, 'ok');
        return;
      }
      case 'drawing-error':
        if (msg.request === 'drawing-open') {
          held = null;
          // A request replaced by another fails silently: the one that replaced it reports itself.
          if (!pending || pending.req !== msg.req) return;
          pending.timeline.stop();
          pending = null;
        }
        listEl.removeAttribute('aria-busy');
        host.setStatus(`Drawing: ${msg.message}`);
        host.toast(msg.message, 'bad');
        return;
    }
  };

  const post = (m: unknown, transfer: Transferable[] = []): boolean => {
    const w = host.worker();
    if (!w) return false;
    if (listening !== w) {
      listening?.removeEventListener('message', onMessage as EventListener);
      w.addEventListener('message', onMessage as EventListener);
      listening = w;
    }
    w.postMessage(m, transfer);
    return true;
  };

  /**
   * Reads the open file and posts what `make` builds from it, unless the file or the worker
   * changed, or `current()` turned false, while it was read.
   */
  const postWithBytes = async (
    make: (data: Uint8Array) => unknown,
    current: () => boolean,
    timeline?: Timeline,
  ): Promise<boolean> => {
    const file = host.file();
    const worker = host.worker();
    if (!file || !worker) return false;
    const data = new Uint8Array(await file.arrayBuffer());
    timeline?.mark('bytes');
    if (host.file() !== file || host.worker() !== worker || !current()) return false;
    const sent = post(make(data), [data.buffer as ArrayBuffer]);
    timeline?.mark('posted');
    return sent;
  };

  const item = (key: string, label: string, title?: string): HTMLButtonElement => {
    const b = document.createElement('button');
    b.type = 'button';
    b.className = 'drawing-item';
    b.dataset.key = key;
    b.setAttribute('aria-pressed', String(key === 'model'));
    b.textContent = label;
    if (title) b.title = title;
    return b;
  };

  const group = (label: string, items: HTMLButtonElement[]): HTMLDetailsElement | null => {
    if (items.length === 0) return null;
    const d = document.createElement('details');
    d.className = 'drawing-group';
    d.open = true;
    const s = document.createElement('summary');
    s.textContent = `${label} (${items.length})`;
    d.append(s, ...items);
    return d;
  };

  function renderList(list: DrawingList): void {
    const byName = (a: { name: string }, b: { name: string }): number => a.name.localeCompare(b.name);
    const plans = [...list.plans].sort(byName);
    const groups = [
      group(
        'Sheets',
        [...list.sheets]
          .sort((a, b) => a.number.localeCompare(b.number, undefined, { numeric: true }))
          .map((s) =>
            item(`sheet:${s.id}`, `${s.number} · ${s.name}`, `${s.views.length} view${s.views.length === 1 ? '' : 's'} placed`),
          ),
      ),
      group(
        'Floor plans',
        plans.filter((p) => !p.ceiling).map((p) => item(`plan:${p.id}`, p.name, `${p.level ?? 'No level'}${scaleText(p.scale)}`)),
      ),
      group(
        'Ceiling plans',
        plans.filter((p) => p.ceiling).map((p) => item(`plan:${p.id}`, p.name, `${p.level ?? 'No level'}${scaleText(p.scale)}`)),
      ),
      group(
        'Sections and elevations',
        [...list.sections].sort(byName).map((s) => item(`section:${s.id}`, s.name, scaleText(s.scale).slice(3) || undefined)),
      ),
    ].filter((g): g is HTMLDetailsElement => g !== null);
    listEl.replaceChildren(item('model', '3D model'), ...groups);
    listEl.dataset.count = String(list.sheets.length + list.plans.length + list.sections.length);
    listEl.dataset.times = JSON.stringify(list.times ?? {});
    if (groups.length === 0) {
      const p = document.createElement('p');
      p.className = 'drawings-empty';
      p.textContent = 'No sheets or views found in this file.';
      listEl.appendChild(p);
    }
  }

  listEl.addEventListener('click', (ev) => {
    const b = (ev.target as Element).closest<HTMLButtonElement>('button[data-key]');
    if (!b) return;
    const key = b.dataset.key ?? 'model';
    if (key === 'model') {
      showModel();
      return;
    }
    const [kind, id] = key.split(':') as [Kind, string];
    // The drawing already drawn shows again at once, as it was left, while the worker keeps it.
    if (shown && `${shown.kind}:${shown.id}` === key && holdsShown()) {
      cancelPending();
      showDrawing();
      return;
    }
    // The list's timeline runs on to the first drawing asked for: what the page did meanwhile.
    if (listTimeline) {
      listTimeline.mark('firstClick');
      listEl.dataset.untilClick = listTimeline.json();
      listTimeline.stop();
      listTimeline = null;
    }
    const request: Request = {
      kind,
      id: Number(id),
      label: b.textContent ?? key,
      req: ++nextReq,
      timeline: new Timeline(host.frameStats),
    };
    // A request still waiting is replaced: its reply is not shown, and its timeline ends here.
    pending?.timeline.stop();
    pending = request;
    listEl.setAttribute('aria-busy', 'true');
    host.setStatus(`Drawing ${request.label}…`);
    void (async () => {
      try {
        const sent = await postWithBytes(
          (data) => ({ type: 'drawing-open', bytes: data, kind, id: request.id, req: request.req }),
          () => pending === request,
          request.timeline,
        );
        if (sent) posted = request.req;
        else if (pending === request) cancelPending();
      } catch (err) {
        // The file could not be read again (moved, or no longer readable): the request ends here.
        request.timeline.stop();
        if (pending !== request) return;
        cancelPending();
        const message = `The file could not be read: ${err instanceof Error ? err.message : String(err)}`;
        host.setStatus(`Drawing: ${message}`);
        host.toast(message, 'bad');
      }
    })();
  });

  $('drawing-back').addEventListener('click', showModel);
  for (const btn of Array.from(document.querySelectorAll<HTMLButtonElement>('#export-view-drawing [data-format]'))) {
    btn.addEventListener('click', () => {
      if (!shown) return;
      if (!holdsShown()) {
        host.toast('The drawing shown is not the one kept for saving: choose it again to save it.', 'info');
        return;
      }
      post({ type: 'drawing-save', format: btn.dataset.format });
    });
  }

  return {
    /** Lists the open model's sheets and views. */
    async load(): Promise<void> {
      listTimeline?.stop();
      listTimeline = new Timeline(host.frameStats);
      await postWithBytes((data) => ({ type: 'drawing-list', bytes: data }), () => true, listTimeline);
    },
    /** A new file: no list, no drawing, the 3D view. */
    reset(): void {
      showModel();
      shown = null;
      posted = null;
      held = null;
      listEl.replaceChildren();
      listEl.removeAttribute('aria-busy');
      delete listEl.dataset.count;
      delete listEl.dataset.times;
      delete listEl.dataset.timeline;
      delete listEl.dataset.untilClick;
      listTimeline?.stop();
      listTimeline = null;
      titleEl.textContent = '';
      selection.detach();
      selection.set([]);
      if (app.dataset.mode !== 'drawing') view.clear();
      listening?.removeEventListener('message', onMessage as EventListener);
      listening = null;
    },
    showModel,
    selection,
  };
}
