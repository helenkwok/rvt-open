/*
 * Selecting elements in a model's sheet or view (model-drawings.ts), as Revit selects them in its
 * drawing area. Every line, text and fill the drawing draws carries `data-element`, the ElementId
 * of the element it draws (rvt-dwg's SVG writer), so the whole element highlights and selects.
 *
 * - Hover pre-highlights the element nearest the cursor (an edge within a few pixels, a text under
 *   it, and with "Select elements by face" a fill or closed outline under it) and names it in the
 *   status line and a tooltip. Tab cycles through the elements under the cursor, nearest first,
 *   and a wall or line in a chain of joined walls or lines offers the whole chain; Shift+Tab
 *   cycles back.
 * - Click selects the highlighted element, or clears the selection over empty paper; Ctrl (or
 *   Cmd) adds to the selection and Shift removes from it.
 * - A drag draws a window: left to right (solid) selects what lies wholly inside, right to left
 *   (dashed) also what it crosses. Ctrl and Shift add and remove as for a click.
 * - Esc clears the selection; Ctrl+Left Arrow brings back the one cleared.
 * - On a sheet, what a viewport shows belongs to the viewport: it selects as one element until the
 *   viewport is activated (double-click it); double-click outside it deactivates it.
 *
 * Geometry is read once per drawing from the SVG's attributes (no layout), into a uniform grid.
 */

/** What the selection needs from the page. */
export interface DrawingSelectHost {
  /** A one-line description of an element: what the status line and the tooltip show. */
  describe(id: number, layer: string | null): string;
  /** Whether an element chains with joined ones of its kind on Tab (walls, lines). */
  chains(id: number, layer: string | null): boolean;
  /** The selection changed by the user in the drawing. */
  changed(ids: number[]): void;
  setStatus(text: string): void;
}

/** Pointer and key events the SVG view hands over while this selection is in charge. */
export interface DrawingPicker {
  hover(clientX: number, clientY: number): void;
  leave(): void;
  click(clientX: number, clientY: number, ev: MouseEvent): void;
  dblclick(clientX: number, clientY: number): void;
  /** A window from `a` to `b` (client points); `done` when the button is released. */
  box(a: { x: number; y: number }, b: { x: number; y: number }, ev: MouseEvent, done: boolean): void;
  key(ev: KeyboardEvent): boolean;
}

export interface DrawingSelection extends DrawingPicker {
  /** Reads the drawing's elements; returns the milliseconds it took. */
  attach(svg: SVGSVGElement): number;
  detach(): void;
  /** Sets the selection from elsewhere (the 3D view, the tree): no `changed` call. */
  set(ids: Iterable<number>): void;
  selected(): number[];
  /** Whether "Select elements by face" is on. */
  byFace: boolean;
}

/** Pick tolerance, screen pixels. */
const TOLERANCE_PX = 5;

interface Item {
  /** The element it draws, or null when it names none (inside a viewport it is the viewport's). */
  id: number | null;
  /** The viewport it is drawn through (its ElementId), or null on the paper or in a view. */
  vp: number | null;
  layer: string | null;
  el: Element;
  /** Segments, root user space: x0 y0 x1 y1 ... */
  seg: Float64Array;
  /** A region a point inside selects: a text's box, a fill, a closed outline (rings, x y ...). */
  rings: Float64Array[];
  /** Whether a point inside `rings` hits whatever "Select elements by face" says (a text does). */
  solid: boolean;
  box: [number, number, number, number];
}

interface Candidate {
  ids: number[];
  label: string;
}

const matrixOf = (el: Element): DOMMatrix | null => {
  const t = (el as SVGGraphicsElement).transform?.baseVal;
  if (!t || t.numberOfItems === 0) return null;
  const m = t.consolidate()?.matrix;
  return m ? new DOMMatrix([m.a, m.b, m.c, m.d, m.e, m.f]) : null;
};

const num = (el: Element, name: string): number => parseFloat(el.getAttribute(name) ?? '0') || 0;

/** A path's data as polylines (an arc as chords) and whether each subpath closes. */
export function pathPolylines(d: string): { pts: number[]; closed: boolean }[] {
  const out: { pts: number[]; closed: boolean }[] = [];
  const tokens = d.match(/[MLAZmlaz]|-?\d*\.?\d+(?:[eE][-+]?\d+)?/g) ?? [];
  let i = 0;
  let cur: { pts: number[]; closed: boolean } | null = null;
  const next = (): number => Number(tokens[i++]);
  while (i < tokens.length) {
    const c = tokens[i++];
    if (c === 'M' || c === 'm') {
      cur = { pts: [next(), next()], closed: false };
      out.push(cur);
    } else if ((c === 'L' || c === 'l') && cur) {
      cur.pts.push(next(), next());
    } else if ((c === 'A' || c === 'a') && cur) {
      const r = next();
      next(); // ry: circles only
      next(); // rotation
      const large = next() !== 0;
      const sweep = next() !== 0;
      const x1 = next();
      const y1 = next();
      const x0 = cur.pts[cur.pts.length - 2];
      const y0 = cur.pts[cur.pts.length - 1];
      // The centre from the end points (SVG's endpoint to centre conversion, circles).
      const hx = (x0 - x1) / 2;
      const hy = (y0 - y1) / 2;
      const h2 = hx * hx + hy * hy;
      if (h2 < 1e-24 || r <= 0) {
        cur.pts.push(x1, y1);
        continue;
      }
      const k = (large !== sweep ? 1 : -1) * Math.sqrt(Math.max(0, (r * r) / h2 - 1));
      const cx = k * hy + (x0 + x1) / 2;
      const cy = -k * hx + (y0 + y1) / 2;
      const rr = Math.hypot(x0 - cx, y0 - cy);
      const a0 = Math.atan2(y0 - cy, x0 - cx);
      let da = Math.atan2(y1 - cy, x1 - cx) - a0;
      if (sweep && da <= 0) da += 2 * Math.PI;
      if (!sweep && da >= 0) da -= 2 * Math.PI;
      const n = Math.max(2, Math.ceil(Math.abs(da) / (Math.PI / 16)));
      for (let s = 1; s <= n; s++) {
        const a = a0 + (da * s) / n;
        cur.pts.push(cx + rr * Math.cos(a), cy + rr * Math.sin(a));
      }
    } else if ((c === 'Z' || c === 'z') && cur) {
      cur.closed = true;
    }
  }
  return out;
}

/** Squared distance from (px, py) to segment (ax, ay)-(bx, by). */
function dist2(px: number, py: number, ax: number, ay: number, bx: number, by: number): number {
  const dx = bx - ax;
  const dy = by - ay;
  const l = dx * dx + dy * dy;
  const t = l > 0 ? Math.max(0, Math.min(1, ((px - ax) * dx + (py - ay) * dy) / l)) : 0;
  const x = ax + t * dx - px;
  const y = ay + t * dy - py;
  return x * x + y * y;
}

/** Even-odd: whether (px, py) lies inside `rings`. */
function inside(px: number, py: number, rings: Float64Array[]): boolean {
  let odd = false;
  for (const r of rings) {
    for (let i = 0, j = r.length - 2; i < r.length; j = i, i += 2) {
      const [xi, yi, xj, yj] = [r[i], r[i + 1], r[j], r[j + 1]];
      if (yi > py !== yj > py && px < ((xj - xi) * (py - yi)) / (yj - yi) + xi) odd = !odd;
    }
  }
  return odd;
}

/** Whether segment a-b meets rectangle r (x0 y0 x1 y1). */
function segmentMeetsRect(ax: number, ay: number, bx: number, by: number, r: number[]): boolean {
  // Liang-Barsky.
  let t0 = 0;
  let t1 = 1;
  const dx = bx - ax;
  const dy = by - ay;
  for (const [p, q] of [
    [-dx, ax - r[0]],
    [dx, r[2] - ax],
    [-dy, ay - r[1]],
    [dy, r[3] - ay],
  ]) {
    if (p === 0) {
      if (q < 0) return false;
    } else {
      const t = q / p;
      if (p < 0) t0 = Math.max(t0, t);
      else t1 = Math.min(t1, t);
      if (t0 > t1) return false;
    }
  }
  return true;
}

export function initDrawingSelection(host: DrawingSelectHost, sheet: HTMLElement): DrawingSelection {
  let svg: SVGSVGElement | null = null;
  let items: Item[] = [];
  /** Uniform grid over the items' boxes: cell -> item indices. */
  let grid = new Map<number, number[]>();
  let cell = 1;
  let origin = [0, 0];
  /** Each element's DOM nodes (a viewport: its group and its title). */
  let nodes = new Map<number, Element[]>();
  /** Each viewport's clip rectangle, root user space. */
  let vpRects = new Map<number, number[]>();
  let vpGroups = new Map<number, Element>();
  let active: number | null = null;
  let selection = new Set<number>();
  let previous: number[] = [];
  let candidates: Candidate[] = [];
  let cycle = 0;
  let hoverAt: { x: number; y: number } | null = null;
  let pre: number[] = [];
  let boxEl: HTMLDivElement | null = null;
  const hoverMs: number[] = [];

  /** The element an item selects as: the viewport's while that viewport is not active. */
  const effective = (it: Item): number | null => (it.vp !== null && it.vp !== active ? it.vp : it.id);
  /** Items that take part: inside the active viewport only, while one is active. */
  const eligible = (it: Item): boolean => active === null || it.vp === active;

  const toRoot = (clientX: number, clientY: number): { x: number; y: number } | null => {
    const ctm = svg?.getScreenCTM();
    if (!ctm) return null;
    const p = new DOMPoint(clientX, clientY).matrixTransform(ctm.inverse());
    return { x: p.x, y: p.y };
  };
  const perPixel = (): number => {
    const ctm = svg?.getScreenCTM();
    return ctm ? 1 / Math.hypot(ctm.a, ctm.b) : 1;
  };

  const cellKey = (i: number, j: number): number => i * 1_000_003 + j;

  function build(root: SVGSVGElement): void {
    items = [];
    nodes = new Map();
    vpRects = new Map();
    vpGroups = new Map();
    const groupMatrix = new Map<Element, DOMMatrix>();
    const rootMatrix = (el: Element): DOMMatrix => {
      if (el === root || !el.parentElement) return new DOMMatrix();
      const hit = groupMatrix.get(el);
      if (hit) return hit;
      const parent = rootMatrix(el.parentElement);
      const own = matrixOf(el);
      const m = own ? parent.multiply(own) : parent;
      groupMatrix.set(el, m);
      return m;
    };
    const addNode = (id: number, el: Element): void => {
      const list = nodes.get(id);
      if (list) list.push(el);
      else nodes.set(id, [el]);
    };
    // Viewports: their clip rectangles and groups.
    for (const g of Array.from(root.querySelectorAll('g[data-viewport][data-element]'))) {
      const id = Number(g.getAttribute('data-element'));
      vpGroups.set(id, g);
      addNode(id, g);
      const clipId = /url\(#([^)]+)\)/.exec(g.getAttribute('clip-path') ?? '')?.[1];
      const rect = clipId ? root.querySelector(`clipPath[id="${CSS.escape(clipId)}"] rect`) : null;
      if (rect) {
        const m = rootMatrix(g);
        const [x, y, w, h] = ['x', 'y', 'width', 'height'].map((a) => num(rect, a));
        const ps = [
          [x, y],
          [x + w, y],
          [x, y + h],
          [x + w, y + h],
        ].map(([px, py]) => new DOMPoint(px, py).matrixTransform(m));
        vpRects.set(id, [
          Math.min(...ps.map((p) => p.x)),
          Math.min(...ps.map((p) => p.y)),
          Math.max(...ps.map((p) => p.x)),
          Math.max(...ps.map((p) => p.y)),
        ]);
      }
    }
    for (const el of Array.from(root.querySelectorAll('line, path, circle, text'))) {
      if (el.closest('defs, clipPath')) continue;
      const vpGroup = el.closest('g[data-viewport][data-element]');
      const vp = vpGroup ? Number(vpGroup.getAttribute('data-element')) : null;
      const attr = el.getAttribute('data-element');
      const id = attr !== null && /^-?\d+$/.test(attr) ? Number(attr) : null;
      if (id === null && vp === null) continue;
      const parent = el.parentElement ? rootMatrix(el.parentElement) : new DOMMatrix();
      const own = el.localName === 'text' ? matrixOf(el) : null;
      const m = own ? parent.multiply(own) : parent;
      const tx = (x: number, y: number): [number, number] => [
        m.a * x + m.c * y + m.e,
        m.b * x + m.d * y + m.f,
      ];
      const seg: number[] = [];
      const rings: Float64Array[] = [];
      let solid = false;
      const ring = (local: number[]): Float64Array => {
        const r = new Float64Array(local.length);
        for (let i = 0; i < local.length; i += 2) [r[i], r[i + 1]] = tx(local[i], local[i + 1]);
        return r;
      };
      const polyline = (local: number[], closed: boolean): void => {
        const r = ring(local);
        for (let i = 0; i + 3 < r.length; i += 2) seg.push(r[i], r[i + 1], r[i + 2], r[i + 3]);
        if (closed && r.length >= 6) {
          seg.push(r[r.length - 2], r[r.length - 1], r[0], r[1]);
          rings.push(r);
        }
      };
      switch (el.localName) {
        case 'line':
          polyline([num(el, 'x1'), num(el, 'y1'), num(el, 'x2'), num(el, 'y2')], false);
          break;
        case 'circle': {
          const [cx, cy, r] = [num(el, 'cx'), num(el, 'cy'), num(el, 'r')];
          const pts: number[] = [];
          const n = r > 0 ? 32 : 1;
          for (let k = 0; k <= n; k++) pts.push(cx + r * Math.cos((2 * Math.PI * k) / n), cy + r * Math.sin((2 * Math.PI * k) / n));
          polyline(pts, r > 0);
          break;
        }
        case 'path': {
          const fill = el.getAttribute('fill');
          const filled = fill !== null && fill !== 'none';
          for (const p of pathPolylines(el.getAttribute('d') ?? '')) {
            if (filled) rings.push(ring(p.pts));
            else polyline(p.pts, p.closed);
          }
          break;
        }
        case 'text': {
          // Its run: about 0.6 em per character, one em per line (the writer's font size is 1.4
          // cap heights), from the baseline up; local y runs down.
          const fs = num(el, 'font-size') || 1;
          const tspans = Array.from(el.querySelectorAll('tspan'));
          const parts = [el.firstChild?.nodeType === Node.TEXT_NODE ? (el.firstChild.textContent ?? '') : '', ...tspans.map((t) => t.textContent ?? '')];
          const w = 0.6 * fs * Math.max(1, ...parts.map((l) => l.length));
          const top = -0.8 * fs;
          const bottom = 0.25 * fs + tspans.length * 1.25 * fs;
          rings.push(ring([0, top, w, top, w, bottom, 0, bottom]));
          solid = true;
          break;
        }
      }
      if (seg.length === 0 && rings.length === 0) continue;
      let [x0, y0, x1, y1] = [Infinity, Infinity, -Infinity, -Infinity];
      for (const a of [seg, ...rings]) {
        for (let i = 0; i < a.length; i += 2) {
          x0 = Math.min(x0, a[i]);
          x1 = Math.max(x1, a[i]);
          y0 = Math.min(y0, a[i + 1]);
          y1 = Math.max(y1, a[i + 1]);
        }
      }
      const layer = el.closest('g[data-layer]')?.getAttribute('data-layer') ?? null;
      items.push({ id, vp, layer, el, seg: Float64Array.from(seg), rings, solid, box: [x0, y0, x1, y1] });
      if (id !== null) addNode(id, el);
    }
    // The grid: about as many cells as items.
    let [gx0, gy0, gx1, gy1] = [Infinity, Infinity, -Infinity, -Infinity];
    for (const it of items) {
      gx0 = Math.min(gx0, it.box[0]);
      gy0 = Math.min(gy0, it.box[1]);
      gx1 = Math.max(gx1, it.box[2]);
      gy1 = Math.max(gy1, it.box[3]);
    }
    grid = new Map();
    if (items.length === 0) return;
    origin = [gx0, gy0];
    const area = Math.max((gx1 - gx0) * (gy1 - gy0), 1e-12);
    cell = Math.max(Math.sqrt(area / items.length) * 2, (gx1 - gx0 + gy1 - gy0) / 4096, 1e-9);
    items.forEach((it, n) => {
      const [i0, j0, i1, j1] = cellRange(it.box[0], it.box[1], it.box[2], it.box[3]);
      for (let i = i0; i <= i1; i++) {
        for (let j = j0; j <= j1; j++) {
          const k = cellKey(i, j);
          const list = grid.get(k);
          if (list) list.push(n);
          else grid.set(k, [n]);
        }
      }
    });
  }

  function cellRange(x0: number, y0: number, x1: number, y1: number): [number, number, number, number] {
    return [
      Math.floor((x0 - origin[0]) / cell),
      Math.floor((y0 - origin[1]) / cell),
      Math.floor((x1 - origin[0]) / cell),
      Math.floor((y1 - origin[1]) / cell),
    ];
  }

  /** Indices of the items whose cells meet the box. */
  function near(x0: number, y0: number, x1: number, y1: number): Set<number> {
    const out = new Set<number>();
    const [i0, j0, i1, j1] = cellRange(x0, y0, x1, y1);
    // ponytail: a window over a huge area walks every cell; walk the items instead when it does.
    if ((i1 - i0 + 1) * (j1 - j0 + 1) > items.length) {
      items.forEach((it, n) => {
        if (it.box[2] >= x0 && it.box[0] <= x1 && it.box[3] >= y0 && it.box[1] <= y1) out.add(n);
      });
      return out;
    }
    for (let i = i0; i <= i1; i++) {
      for (let j = j0; j <= j1; j++) for (const n of grid.get(cellKey(i, j)) ?? []) out.add(n);
    }
    return out;
  }

  const inVp = (it: Item, x: number, y: number): boolean => {
    const r = it.vp !== null ? vpRects.get(it.vp) : undefined;
    return !r || (x >= r[0] && x <= r[2] && y >= r[1] && y <= r[3]);
  };

  /** Elements under a root point, nearest first, with their distances. */
  function hits(x: number, y: number): number[] {
    const tol = TOLERANCE_PX * perPixel();
    const best = new Map<number, number>();
    for (const n of near(x - tol, y - tol, x + tol, y + tol)) {
      const it = items[n];
      if (!eligible(it) || !inVp(it, x, y)) continue;
      const id = effective(it);
      if (id === null) continue;
      let d = Infinity;
      const s = it.seg;
      for (let i = 0; i < s.length; i += 4) d = Math.min(d, dist2(x, y, s[i], s[i + 1], s[i + 2], s[i + 3]));
      d = Math.sqrt(d);
      if (d > tol && it.rings.length > 0 && (it.solid || api.byFace) && inside(x, y, it.rings)) {
        // A text under the cursor is as near as can be; a face ranks after every edge in reach.
        d = it.solid ? 0 : tol;
      }
      if (d <= tol && d < (best.get(id) ?? Infinity)) best.set(id, d);
    }
    return [...best].sort((a, b) => a[1] - b[1]).map(([id]) => id);
  }

  const layerOf = (id: number): string | null => {
    const el = nodes.get(id)?.find((e) => e.closest('g[data-layer]'));
    return el?.closest('g[data-layer]')?.getAttribute('data-layer') ?? null;
  };

  /** Each chaining element's chain (shared arrays), worked out on first need per drawing. */
  let chains: Map<number, number[]> | null = null;

  /**
   * The chain of joined elements `id` belongs to: the elements that chain (walls, lines) whose
   * drawn lines share a point with it, and theirs with others, and so on.
   */
  function chainOf(id: number): number[] {
    if (!chains) {
      chains = new Map();
      // Points within a ten-thousandth of a grid cell (well under a millimetre of a plan) are one.
      const [x0, y0] = origin;
      const eps = Math.max(cell * 1e-4, 1e-9);
      const key = (x: number, y: number): string => `${Math.round((x - x0) / eps)},${Math.round((y - y0) / eps)}`;
      const parent = new Map<number, number>();
      const find = (e: number): number => {
        let r = e;
        while (parent.get(r) !== r) r = parent.get(r)!;
        parent.set(e, r);
        return r;
      };
      const at = new Map<string, number>();
      for (const it of items) {
        const e = effective(it);
        if (e === null || !eligible(it) || !host.chains(e, it.layer)) continue;
        if (!parent.has(e)) parent.set(e, e);
        for (let i = 0; i < it.seg.length; i += 2) {
          const k = key(it.seg[i], it.seg[i + 1]);
          const other = at.get(k);
          if (other === undefined) at.set(k, e);
          else parent.set(find(other), find(e));
        }
      }
      const groups = new Map<number, number[]>();
      for (const e of parent.keys()) {
        const root = find(e);
        const g = groups.get(root) ?? [];
        g.push(e);
        groups.set(root, g);
      }
      for (const g of groups.values()) for (const e of g) chains.set(e, g);
    }
    return chains.get(id) ?? [id];
  }

  function candidatesAt(x: number, y: number): Candidate[] {
    const ids = hits(x, y);
    const out: Candidate[] = [];
    ids.forEach((id, i) => {
      out.push({ ids: [id], label: host.describe(id, layerOf(id)) });
      if (i === 0) {
        const chain = chainOf(id);
        if (chain.length > 1) out.push({ ids: chain, label: `Chain of ${chain.length} elements` });
      }
    });
    return out;
  }

  const mark = (ids: Iterable<number>, cls: string, on: boolean): void => {
    for (const id of ids) {
      for (const el of nodes.get(id) ?? []) el.classList.toggle(cls, on);
    }
  };

  function setPre(ids: number[]): void {
    mark(pre, 'sel-pre', false);
    pre = ids;
    mark(pre, 'sel-pre', true);
  }

  function showCandidate(): void {
    const c = candidates[cycle];
    setPre(c ? c.ids : []);
    if (c) {
      sheet.title = c.label;
      host.setStatus(candidates.length > 1 ? `${c.label} · Tab for the next of ${candidates.length}` : c.label);
    } else sheet.removeAttribute('title');
  }

  function render(): void {
    for (const el of Array.from(svg?.querySelectorAll('.sel-on') ?? [])) el.classList.remove('sel-on');
    mark(selection, 'sel-on', true);
    svg?.setAttribute('data-selected', String(selection.size));
    const count = document.getElementById('drawing-selection-count');
    if (count) {
      count.hidden = selection.size === 0;
      count.textContent = `${selection.size} selected`;
    }
  }

  function commit(next: Set<number>): void {
    if (next.size === selection.size && [...next].every((id) => selection.has(id))) return;
    if (next.size === 0 && selection.size > 0) previous = [...selection];
    selection = next;
    render();
    host.changed([...selection]);
  }

  function apply(ids: number[], ev: { ctrlKey: boolean; metaKey: boolean; shiftKey: boolean }): void {
    const next = new Set(selection);
    if (ev.shiftKey) for (const id of ids) next.delete(id);
    else if (ev.ctrlKey || ev.metaKey) for (const id of ids) next.add(id);
    else {
      next.clear();
      for (const id of ids) next.add(id);
    }
    commit(next);
  }

  function setActive(vp: number | null): void {
    if (active === vp) return;
    if (active !== null) vpGroups.get(active)?.classList.remove('vp-active');
    active = vp;
    chains = null;
    hoverAt = null;
    candidates = [];
    setPre([]);
    svg?.classList.toggle('has-active-viewport', vp !== null);
    if (vp !== null) vpGroups.get(vp)?.classList.add('vp-active');
    commit(new Set());
    host.setStatus(vp === null ? 'Viewport deactivated' : 'Viewport activated: its elements select; double-click outside it to deactivate');
  }

  const api: DrawingSelection = {
    byFace: false,
    attach(root: SVGSVGElement): number {
      const t0 = performance.now();
      svg = root;
      active = null;
      candidates = [];
      pre = [];
      chains = null;
      build(root);
      render();
      const ms = performance.now() - t0;
      root.dataset.indexMs = ms.toFixed(1);
      root.dataset.indexed = String(items.length);
      hoverMs.length = 0;
      return ms;
    },
    detach(): void {
      svg = null;
      items = [];
      grid = new Map();
      nodes = new Map();
      candidates = [];
      pre = [];
      active = null;
      sheet.removeAttribute('title');
      const count = document.getElementById('drawing-selection-count');
      if (count) count.hidden = true;
    },
    set(ids: Iterable<number>): void {
      // Kept whether or not this drawing draws them, as Revit keeps a selection across views.
      selection = new Set(ids);
      if (svg) render();
    },
    selected: () => [...selection],
    hover(clientX: number, clientY: number): void {
      if (!svg) return;
      // Tab keeps its choice until the cursor moves.
      if (hoverAt && Math.hypot(clientX - hoverAt.x, clientY - hoverAt.y) < 2) return;
      const t0 = performance.now();
      hoverAt = { x: clientX, y: clientY };
      const p = toRoot(clientX, clientY);
      candidates = p ? candidatesAt(p.x, p.y) : [];
      cycle = 0;
      showCandidate();
      hoverMs.push(performance.now() - t0);
      if (hoverMs.length > 1000) hoverMs.shift();
      const sorted = [...hoverMs].sort((a, b) => a - b);
      svg.dataset.hoverMs = JSON.stringify({
        n: sorted.length,
        median: +sorted[sorted.length >> 1].toFixed(2),
        max: +sorted[sorted.length - 1].toFixed(2),
      });
    },
    leave(): void {
      hoverAt = null;
      candidates = [];
      setPre([]);
      sheet.removeAttribute('title');
    },
    click(clientX: number, clientY: number, ev: MouseEvent): void {
      if (!svg) return;
      if (!hoverAt || Math.hypot(clientX - hoverAt.x, clientY - hoverAt.y) >= 2) {
        hoverAt = null;
        api.hover(clientX, clientY);
      }
      const c = candidates[cycle];
      if (c) apply(c.ids, ev);
      else if (!ev.ctrlKey && !ev.metaKey && !ev.shiftKey) commit(new Set());
    },
    dblclick(clientX: number, clientY: number): void {
      const p = toRoot(clientX, clientY);
      if (!p || vpRects.size === 0) return;
      if (active !== null) {
        const r = vpRects.get(active);
        if (r && !(p.x >= r[0] && p.x <= r[2] && p.y >= r[1] && p.y <= r[3])) setActive(null);
        return;
      }
      for (const [id, r] of vpRects) {
        if (p.x >= r[0] && p.x <= r[2] && p.y >= r[1] && p.y <= r[3]) {
          setActive(id);
          return;
        }
      }
    },
    box(a, b, ev, done): void {
      if (!svg) return;
      const frame = sheet.getBoundingClientRect();
      const crossing = b.x < a.x;
      if (!boxEl) {
        boxEl = document.createElement('div');
        boxEl.className = 'drawing-window';
        sheet.appendChild(boxEl);
      }
      boxEl.classList.toggle('is-crossing', crossing);
      Object.assign(boxEl.style, {
        left: `${Math.min(a.x, b.x) - frame.left}px`,
        top: `${Math.min(a.y, b.y) - frame.top}px`,
        width: `${Math.abs(b.x - a.x)}px`,
        height: `${Math.abs(b.y - a.y)}px`,
      });
      if (!done) return;
      boxEl.remove();
      boxEl = null;
      const p = toRoot(a.x, a.y);
      const q = toRoot(b.x, b.y);
      if (!p || !q) return;
      const r = [Math.min(p.x, q.x), Math.min(p.y, q.y), Math.max(p.x, q.x), Math.max(p.y, q.y)];
      // Per element: whether every part of it lies inside, and whether any part meets the window.
      const all = new Map<number, boolean>();
      const meets = new Set<number>();
      const insideRect = (x: number, y: number): boolean => x >= r[0] && x <= r[2] && y >= r[1] && y <= r[3];
      for (const it of items) {
        const id = effective(it);
        if (id === null || !eligible(it)) continue;
        // What a viewport clips away is not seen: its parts outside the clip do not count.
        const clip = it.vp !== null ? vpRects.get(it.vp) : undefined;
        if (clip && (it.box[2] < clip[0] || it.box[0] > clip[2] || it.box[3] < clip[1] || it.box[1] > clip[3])) continue;
        const wholly = insideRect(it.box[0], it.box[1]) && insideRect(it.box[2], it.box[3]);
        all.set(id, (all.get(id) ?? true) && wholly);
        if (!crossing || meets.has(id)) continue;
        let hit = wholly;
        const s = it.seg;
        for (let i = 0; !hit && i < s.length; i += 4) hit = segmentMeetsRect(s[i], s[i + 1], s[i + 2], s[i + 3], r);
        if (!hit) {
          for (const ring of it.rings) {
            for (let i = 0; !hit && i + 3 < ring.length; i += 2) {
              hit = segmentMeetsRect(ring[i], ring[i + 1], ring[i + 2], ring[i + 3], r);
            }
            hit ||= inside((r[0] + r[2]) / 2, (r[1] + r[3]) / 2, [ring]);
          }
        }
        if (hit) meets.add(id);
      }
      const ids = crossing ? [...meets] : [...all].filter(([, w]) => w).map(([id]) => id);
      apply(ids, ev);
      host.setStatus(`${crossing ? 'Crossing' : 'Window'} selection: ${ids.length} element${ids.length === 1 ? '' : 's'}`);
    },
    key(ev: KeyboardEvent): boolean {
      if (!svg) return false;
      if (ev.key === 'Tab' && candidates.length > 0) {
        cycle = (cycle + (ev.shiftKey ? candidates.length - 1 : 1)) % candidates.length;
        showCandidate();
        return true;
      }
      if (ev.key === 'Escape' && selection.size > 0) {
        commit(new Set());
        return true;
      }
      if (ev.key === 'ArrowLeft' && (ev.ctrlKey || ev.metaKey) && previous.length > 0) {
        commit(new Set(previous));
        return true;
      }
      return false;
    },
  };
  return api;
}
