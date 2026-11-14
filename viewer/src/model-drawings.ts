/*
 * A Revit model's sheets and views as 2D drawings, beside the 3D view. The
 * Drawings list (left panel) comes from the views the file holds; a sheet or
 * view is drawn only when it is chosen, by the parse worker (worker.ts, which
 * keeps the model it opened) as `rvt-sheet --sheet` or `--view` draws it, and
 * shown in the 2D pane as SVG. The 3D view and the drawing stay as they are
 * while the other is shown: switching only hides one.
 */
import type { SvgView } from './drawing';

export interface ModelDrawingsHost {
  /** The worker that opened the model, or null. */
  worker(): Worker | null;
  /** The file it opened: read again for each request, so no copy is kept. */
  file(): File | null;
  stem(): string;
  setStatus(text: string): void;
  toast(message: string, kind?: 'ok' | 'bad' | 'info'): void;
  download(filename: string, blob: Blob): void;
}

type Kind = 'sheet' | 'plan' | 'section';

interface DrawingList {
  sheets: { id: number; number: string; name: string; views: number[] }[];
  plans: { id: number; name: string; level: string | null; ceiling: boolean; scale: number | null; onSheet: boolean }[];
  sections: { id: number; name: string; scale: number | null }[];
}

interface Drawn {
  svg: string;
  layers: string[];
  truncated: boolean;
  layout: string | null;
  scale: number;
  counts: { lines: number; texts: number; viewports: number; entities: number };
  notes: string[];
}

type Reply =
  | { type: 'drawing-list'; list: DrawingList }
  | { type: 'drawing-open'; kind: Kind; id: number; drawn: Drawn }
  | { type: 'drawing-save'; format: string; bytes: Uint8Array }
  | { type: 'drawing-error'; request: string; message: string };

const $ = (id: string): HTMLElement => {
  const el = document.getElementById(id);
  if (!el) throw new Error(`missing element: #${id}`);
  return el;
};

const scaleText = (scale: number | null): string => (scale && scale > 0 ? ` · 1:${Math.round(scale)}` : '');

export function initModelDrawings(
  host: ModelDrawingsHost,
  view: SvgView,
): { load(): Promise<void>; reset(): void; showModel(): void } {
  const app = $('app');
  const pane = $('drawing-pane');
  const sheetEl = $('drawing-sheet');
  const listEl = $('drawings');
  const titleEl = $('drawing-title');

  let shown: { kind: Kind; id: number; label: string } | null = null;
  let pending: { kind: Kind; id: number; label: string } | null = null;
  let listening: Worker | null = null;

  const bytes = async (): Promise<Uint8Array | null> => {
    const file = host.file();
    return file ? new Uint8Array(await file.arrayBuffer()) : null;
  };

  const setPressed = (key: string): void => {
    for (const b of Array.from(listEl.querySelectorAll<HTMLButtonElement>('button[data-key]'))) {
      b.setAttribute('aria-pressed', String(b.dataset.key === key));
    }
  };

  /** Back to the 3D view; the drawing stays in the pane for the next time. */
  const showModel = (): void => {
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
      case 'drawing-list':
        renderList(msg.list);
        return;
      case 'drawing-open': {
        if (!pending || pending.kind !== msg.kind || pending.id !== msg.id) return;
        shown = pending;
        pending = null;
        listEl.removeAttribute('aria-busy');
        if (!view.show(msg.drawn.svg)) {
          host.toast('The drawing could not be shown: the renderer returned no SVG.', 'bad');
          return;
        }
        const svg = view.svg();
        if (svg) {
          svg.dataset.lines = String(msg.drawn.counts.lines);
          svg.dataset.texts = String(msg.drawn.counts.texts);
          svg.dataset.viewports = String(msg.drawn.counts.viewports);
        }
        titleEl.textContent = shown.label;
        showDrawing();
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
        listEl.removeAttribute('aria-busy');
        if (msg.request === 'drawing-open') pending = null;
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
    // The drawing already drawn shows again at once, as it was left.
    if (shown && `${shown.kind}:${shown.id}` === key) {
      showDrawing();
      return;
    }
    const label = b.textContent ?? key;
    pending = { kind, id: Number(id), label };
    listEl.setAttribute('aria-busy', 'true');
    host.setStatus(`Drawing ${label}…`);
    void bytes().then((data) => {
      if (!data || !post({ type: 'drawing-open', bytes: data, kind, id: Number(id) }, [data.buffer as ArrayBuffer])) {
        pending = null;
        listEl.removeAttribute('aria-busy');
      }
    });
  });

  $('drawing-back').addEventListener('click', showModel);
  for (const btn of Array.from(document.querySelectorAll<HTMLButtonElement>('#export-view-drawing [data-format]'))) {
    btn.addEventListener('click', () => {
      if (shown) post({ type: 'drawing-save', format: btn.dataset.format });
    });
  }

  return {
    /** Lists the open model's sheets and views. */
    async load(): Promise<void> {
      const data = await bytes();
      if (data) post({ type: 'drawing-list', bytes: data }, [data.buffer as ArrayBuffer]);
    },
    /** A new file: no list, no drawing, the 3D view. */
    reset(): void {
      showModel();
      shown = null;
      pending = null;
      listEl.replaceChildren();
      listEl.removeAttribute('aria-busy');
      delete listEl.dataset.count;
      titleEl.textContent = '';
      if (app.dataset.mode !== 'drawing') view.clear();
      listening?.removeEventListener('message', onMessage as EventListener);
      listening = null;
    },
    showModel,
  };
}
