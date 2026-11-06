/*
 * DWG / DXF 2D pane. A drawing opens in the parse worker (worker.ts, the only
 * wasm owner), which keeps it and answers layout, entity, table and save
 * requests. The SVG it renders is untrusted file content: it is parsed as
 * image/svg+xml, stripped of anything active or external, and imported as DOM.
 */

/** What the pane needs from main.ts; passed in so this module has no import cycle. */
export interface DrawingHost {
  resetWorker(): Worker;
  /** Where the wasm is: a worker started from a blob URL cannot resolve it. */
  wasmUrl: string;
  setStatus(text: string): void;
  setStatusBusy(busy: boolean): void;
  toast(message: string, kind?: 'ok' | 'bad' | 'info'): void;
  download(filename: string, blob: Blob): void;
  setDropzoneVisible(visible: boolean): void;
  setExportHint(text: string | null): void;
  formatBytes(n: number): string;
  /** The RVT-only export and download buttons, disabled while a drawing is open. */
  rvtButtons: HTMLButtonElement[];
}

interface DwgSummary {
  version: string;
  units: number;
  layouts: { name: string; isModel: boolean }[];
  layers: { name: string; colour: string | null; on: boolean; entities: number }[];
  notes: string[];
  counts: Record<string, unknown>;
}
interface DwgSvg {
  svg: string;
  layers: string[];
  notDrawn: Record<string, number>;
  truncated: boolean;
}
type DwgReply =
  | { type: 'dwg-ready'; summary: DwgSummary; svg: DwgSvg }
  | { type: 'dwg-svg'; layout: string | null; svg: DwgSvg }
  | { type: 'dwg-entity'; handle: string; row: Record<string, unknown> | null }
  | { type: 'dwg-table'; name: string; text: string }
  | { type: 'dwg-save'; format: string; version: string; bytes: Uint8Array }
  | { type: 'error'; message: string };

const $ = (id: string): HTMLElement => {
  const el = document.getElementById(id);
  if (!el) throw new Error(`missing element: #${id}`);
  return el;
};

export function isDrawingFile(name: string): boolean {
  return /\.(dwg|dxf)$/i.test(name);
}

const SVG_NS = 'http://www.w3.org/2000/svg';
const DXF_UNITS = ['unitless', 'in', 'ft', 'mi', 'mm', 'cm', 'm', 'km', 'µin', 'mil', 'yd', 'Å', 'nm', 'µm', 'dm'];

/** Parses the renderer's SVG and drops scripts, event handlers and external references. */
export function sanitizeSvg(text: string): SVGSVGElement | null {
  const doc = new DOMParser().parseFromString(text, 'image/svg+xml');
  const root = doc.documentElement;
  if (root.localName !== 'svg' || root.namespaceURI !== SVG_NS || doc.querySelector('parsererror')) return null;
  doc.querySelectorAll('script, foreignObject, style, iframe, object, embed').forEach((n) => n.remove());
  for (const el of [root, ...Array.from(root.querySelectorAll('*'))]) {
    for (const attr of Array.from(el.attributes)) {
      const name = attr.name.toLowerCase();
      const value = attr.value.trim();
      const external =
        ((name === 'href' || name === 'xlink:href' || name === 'src') && !value.startsWith('#')) ||
        (/url\(/i.test(value) && !/^url\(\s*#[^)]*\)$/i.test(value));
      if (name.startsWith('on') || name === 'style' || external) el.removeAttribute(attr.name);
    }
  }
  return document.importNode(root, true) as unknown as SVGSVGElement;
}

export function initDrawing(host: DrawingHost): {
  open(file: File): Promise<void>;
  close(): void;
  isOpen(): boolean;
} {
  const app = $('app');
  const pane = $('drawing-pane');
  const sheet = $('drawing-sheet');
  const layoutSel = $('drawing-layout') as HTMLSelectElement;
  const layersEl = $('drawing-layers');
  const infoEl = $('info');
  const statusPanelEl = $('status-panel');
  const fileMetaEl = $('file-meta');

  let worker: Worker | null = null;
  let stem = 'drawing';
  let summary: DwgSummary | null = null;
  let current: DwgSvg | null = null;
  let svgEl: SVGSVGElement | null = null;
  let fitBox = { x: 0, y: 0, w: 1, h: 1 };
  let box = { ...fitBox };
  const hidden = new Set<string>();
  let pendingHandle: string | null = null;
  let picked: Element | null = null;

  const setViewBox = (): void => {
    svgEl?.setAttribute('viewBox', `${box.x} ${box.y} ${box.w} ${box.h}`);
  };
  const fit = (): void => {
    box = { ...fitBox };
    setViewBox();
  };

  /** Zooms by `f` (< 1 is in) around a client point, or the centre when none. */
  const zoom = (f: number, clientX?: number, clientY?: number): void => {
    if (!svgEl) return;
    let p = { x: box.x + box.w / 2, y: box.y + box.h / 2 };
    const ctm = svgEl.getScreenCTM();
    if (ctm && clientX !== undefined && clientY !== undefined) {
      const pt = new DOMPoint(clientX, clientY).matrixTransform(ctm.inverse());
      p = { x: pt.x, y: pt.y };
    }
    box = { x: p.x - (p.x - box.x) * f, y: p.y - (p.y - box.y) * f, w: box.w * f, h: box.h * f };
    setViewBox();
  };

  const applyLayerVisibility = (): void => {
    if (!svgEl) return;
    for (const g of Array.from(svgEl.querySelectorAll<SVGGElement>('g.layer[data-layer]'))) {
      g.style.display = hidden.has(g.dataset.layer ?? '') ? 'none' : '';
    }
  };

  const showSvg = (s: DwgSvg): void => {
    current = s;
    const el = sanitizeSvg(s.svg);
    sheet.replaceChildren();
    picked = null;
    svgEl = null;
    if (!el) {
      host.toast('The drawing could not be shown: the renderer returned no SVG.', 'bad');
      return;
    }
    el.removeAttribute('width');
    el.removeAttribute('height');
    const vb = el.viewBox.baseVal;
    fitBox = vb && vb.width > 0 && vb.height > 0 ? { x: vb.x, y: vb.y, w: vb.width, h: vb.height } : { x: 0, y: 0, w: 1, h: 1 };
    svgEl = el;
    sheet.appendChild(el);
    fit();
    applyLayerVisibility();
    if (s.truncated) host.toast('The drawing is larger than the render limit; part of it is not drawn.', 'info');
  };

  const renderLayers = (): void => {
    layersEl.replaceChildren();
    const drawn = new Set(current?.layers ?? []);
    for (const layer of summary?.layers ?? []) {
      const row = document.createElement('label');
      row.className = 'category-toggle';
      const cb = document.createElement('input');
      cb.type = 'checkbox';
      cb.checked = !hidden.has(layer.name);
      cb.disabled = !layer.on;
      cb.setAttribute('aria-label', `Layer ${layer.name}`);
      if (!layer.on) row.title = 'Off or frozen in the file, so not drawn';
      cb.addEventListener('change', () => {
        if (cb.checked) hidden.delete(layer.name);
        else hidden.add(layer.name);
        applyLayerVisibility();
      });
      const sw = document.createElement('span');
      sw.className = 'drawing-swatch';
      sw.setAttribute('aria-hidden', 'true');
      if (layer.colour && /^#[0-9a-f]{6}$/i.test(layer.colour)) sw.style.background = layer.colour;
      const name = document.createElement('span');
      name.textContent = layer.name;
      const n = document.createElement('span');
      n.className = 'tree-count';
      n.textContent = String(layer.entities);
      row.append(cb, sw, name, n);
      if (layer.on && !drawn.has(layer.name) && layer.entities > 0) row.title = 'No entities of this layer in this layout';
      layersEl.appendChild(row);
    }
  };

  const setAll = (on: boolean): void => {
    hidden.clear();
    if (!on) for (const l of summary?.layers ?? []) hidden.add(l.name);
    renderLayers();
    applyLayerVisibility();
  };
  $('drawing-layers-on').addEventListener('click', () => setAll(true));
  $('drawing-layers-off').addEventListener('click', () => setAll(false));

  const statusRow = (label: string, value: string): HTMLElement => {
    const row = document.createElement('div');
    row.className = 'status-row';
    const dot = document.createElement('span');
    dot.className = 'status-dot ok';
    const k = document.createElement('div');
    k.className = 'status-label';
    k.textContent = label;
    const v = document.createElement('div');
    v.className = 'status-value';
    v.textContent = value;
    row.append(dot, k, v);
    return row;
  };

  const entityCount = (s: DwgSummary): number =>
    typeof s.counts?.entities === 'number' ? s.counts.entities : s.layers.reduce((a, l) => a + l.entities, 0);

  const renderStatus = (s: DwgSummary): void => {
    statusPanelEl.replaceChildren(
      statusRow('Version', s.version),
      statusRow('Units', DXF_UNITS[s.units] ?? String(s.units)),
      statusRow('Entities', String(entityCount(s))),
      statusRow('Layouts', String(s.layouts.length)),
      statusRow('Layers', String(s.layers.length)),
    );
    if (s.notes.length > 0) {
      const ul = document.createElement('ul');
      ul.className = 'drawing-notes';
      ul.setAttribute('aria-label', 'Reader notes');
      for (const note of s.notes) {
        const li = document.createElement('li');
        li.textContent = note;
        ul.appendChild(li);
      }
      statusPanelEl.appendChild(ul);
    }
  };

  const renderEntity = (row: Record<string, unknown> | null, handle: string): void => {
    infoEl.removeAttribute('aria-busy');
    if (!row) {
      infoEl.textContent = `No entity row for handle ${handle}.`;
      return;
    }
    const dl = document.createElement('dl');
    dl.className = 'info-group';
    for (const [k, v] of Object.entries(row)) {
      if (v === null || v === undefined || v === '') continue;
      const r = document.createElement('div');
      r.className = 'info-row';
      const dt = document.createElement('dt');
      dt.className = 'k';
      dt.textContent = k;
      const dd = document.createElement('dd');
      dd.className = typeof v === 'number' ? 'v num' : 'v';
      dd.textContent = typeof v === 'object' ? JSON.stringify(v) : String(v);
      r.append(dt, dd);
      dl.appendChild(r);
    }
    infoEl.replaceChildren(dl);
  };

  const onMessage = (ev: MessageEvent<DwgReply>): void => {
    const msg = ev.data;
    switch (msg.type) {
      case 'dwg-ready': {
        summary = msg.summary;
        hidden.clear();
        layoutSel.replaceChildren();
        const layouts = [...msg.summary.layouts].sort((a, b) => Number(b.isModel) - Number(a.isModel));
        for (const l of layouts) {
          const o = document.createElement('option');
          o.value = l.isModel ? '' : l.name;
          o.textContent = l.isModel ? 'Model' : l.name;
          layoutSel.appendChild(o);
        }
        if (!layouts.some((l) => l.isModel)) {
          const o = document.createElement('option');
          o.value = '';
          o.textContent = 'Model';
          layoutSel.prepend(o);
        }
        layoutSel.value = '';
        showSvg(msg.svg);
        renderLayers();
        renderStatus(msg.summary);
        fileMetaEl.textContent = `${fileMetaEl.dataset.drawingName ?? stem} · ${msg.summary.version} · ${entityCount(msg.summary)} entities`;
        host.setStatusBusy(false);
        host.setStatus(`Loaded drawing · ${msg.summary.layers.length} layers · ${msg.summary.layouts.length} layouts`);
        host.toast(`Loaded ${fileMetaEl.dataset.drawingName ?? stem}`, 'ok');
        sheet.focus();
        return;
      }
      case 'dwg-svg':
        if ((layoutSel.value || null) !== msg.layout) return;
        showSvg(msg.svg);
        renderLayers();
        host.setStatus(`Layout ${msg.layout ?? 'Model'}`);
        return;
      case 'dwg-entity':
        if (msg.handle === pendingHandle) renderEntity(msg.row, msg.handle);
        return;
      case 'dwg-table': {
        const jsonl = msg.name === 'entities.jsonl';
        const file = jsonl ? `${stem}-entities.jsonl` : `${stem}-${msg.name}.csv`;
        host.download(file, new Blob([msg.text], { type: jsonl ? 'application/x-ndjson' : 'text/csv' }));
        host.toast(`Downloaded ${file}`, 'ok');
        return;
      }
      case 'dwg-save': {
        const file = `${stem}-${msg.version}.${msg.format}`;
        host.download(file, new Blob([msg.bytes as BlobPart], { type: 'application/octet-stream' }));
        host.toast(`Downloaded ${file}`, 'ok');
        return;
      }
      case 'error':
        host.setStatusBusy(false);
        host.setStatus(`Error: ${msg.message}`);
        host.toast(msg.message, 'bad');
        if (!summary) {
          close();
          host.setDropzoneVisible(true);
        }
        return;
    }
  };

  const post = (m: unknown, transfer: Transferable[] = []): void => worker?.postMessage(m, transfer);

  layoutSel.addEventListener('change', () => post({ type: 'dwg-svg', layout: layoutSel.value || null }));
  $('drawing-fit').addEventListener('click', fit);
  for (const btn of Array.from(document.querySelectorAll<HTMLButtonElement>('#export-drawing [data-table]'))) {
    btn.addEventListener('click', () => post({ type: 'dwg-table', name: btn.dataset.table }));
  }
  for (const btn of Array.from(document.querySelectorAll<HTMLButtonElement>('#export-drawing [data-format]'))) {
    btn.addEventListener('click', () => post({ type: 'dwg-save', format: btn.dataset.format, version: '2018' }));
  }
  $('export-dwg-svg').addEventListener('click', () => {
    if (!current) return;
    const file = `${stem}-${layoutSel.value || 'model'}.svg`;
    host.download(file, new Blob([current.svg], { type: 'image/svg+xml' }));
    host.toast(`Downloaded ${file}`, 'ok');
  });

  // Pan by drag; a press that barely moves is a click on an entity.
  let drag: { x: number; y: number; moved: boolean; id: number } | null = null;
  sheet.addEventListener('pointerdown', (ev) => {
    if (ev.button !== 0) return;
    drag = { x: ev.clientX, y: ev.clientY, moved: false, id: ev.pointerId };
  });
  sheet.addEventListener('pointermove', (ev) => {
    if (!drag || ev.pointerId !== drag.id || !svgEl) return;
    const dx = ev.clientX - drag.x;
    const dy = ev.clientY - drag.y;
    if (!drag.moved && Math.hypot(dx, dy) < 4) return;
    if (!drag.moved) {
      drag.moved = true;
      sheet.setPointerCapture(ev.pointerId);
      sheet.classList.add('is-panning');
    }
    const r = sheet.getBoundingClientRect();
    const k = Math.max(box.w / r.width, box.h / r.height);
    box.x -= dx * k;
    box.y -= dy * k;
    drag.x = ev.clientX;
    drag.y = ev.clientY;
    setViewBox();
  });
  const endDrag = (ev: PointerEvent): void => {
    if (!drag || ev.pointerId !== drag.id) return;
    const wasClick = !drag.moved;
    drag = null;
    sheet.classList.remove('is-panning');
    if (!wasClick) return;
    const target = (ev.target as Element | null)?.closest?.('[data-handle]');
    if (!target || !sheet.contains(target)) return;
    picked?.classList.remove('drawing-picked');
    picked = target;
    target.classList.add('drawing-picked');
    pendingHandle = target.getAttribute('data-handle');
    infoEl.setAttribute('aria-busy', 'true');
    post({ type: 'dwg-entity', handle: pendingHandle });
  };
  sheet.addEventListener('pointerup', endDrag);
  sheet.addEventListener('pointercancel', (ev) => {
    if (drag && ev.pointerId === drag.id) drag = null;
    sheet.classList.remove('is-panning');
  });
  sheet.addEventListener(
    'wheel',
    (ev) => {
      ev.preventDefault();
      zoom(Math.exp(ev.deltaY * 0.0015), ev.clientX, ev.clientY);
    },
    { passive: false },
  );
  sheet.addEventListener('keydown', (ev) => {
    if (ev.key === '+' || ev.key === '=') zoom(1 / 1.25);
    else if (ev.key === '-' || ev.key === '_') zoom(1.25);
    else if (ev.key === '0') fit();
    else return;
    ev.preventDefault();
  });

  function close(): void {
    if (app.dataset.mode !== 'drawing') return;
    delete app.dataset.mode;
    pane.hidden = true;
    sheet.replaceChildren();
    layersEl.replaceChildren();
    svgEl = null;
    summary = null;
    current = null;
    picked = null;
    pendingHandle = null;
    worker?.removeEventListener('message', onMessage as EventListener);
    worker = null;
  }

  async function open(file: File): Promise<void> {
    stem = file.name.replace(/\.(dwg|dxf)$/i, '') || 'drawing';
    fileMetaEl.dataset.drawingName = file.name;
    fileMetaEl.textContent = `${file.name} · ${host.formatBytes(file.size)}`;
    host.setStatus(`Reading ${host.formatBytes(file.size)}…`);
    host.setStatusBusy(true);
    let bytes: Uint8Array;
    try {
      bytes = new Uint8Array(await file.arrayBuffer());
    } catch (err) {
      host.setStatusBusy(false);
      host.setStatus(`Error: ${(err as Error).message}`);
      host.toast(`Could not read ${file.name}.`, 'bad');
      return;
    }
    summary = null;
    current = null;
    app.dataset.mode = 'drawing';
    pane.hidden = false;
    sheet.replaceChildren();
    host.setDropzoneVisible(false);
    for (const b of host.rvtButtons) b.disabled = true;
    host.setExportHint(null);
    infoEl.removeAttribute('aria-busy');
    infoEl.textContent = 'Click an entity in the drawing to see its row.';
    statusPanelEl.replaceChildren(statusRow('File', `Reading ${file.name}`));
    worker = host.resetWorker();
    worker.addEventListener('message', onMessage as EventListener);
    host.setStatus('Opening drawing');
    post({ type: 'dwg-open', bytes, wasmUrl: host.wasmUrl }, [bytes.buffer as ArrayBuffer]);
  }

  return { open, close, isOpen: () => app.dataset.mode === 'drawing' };
}
