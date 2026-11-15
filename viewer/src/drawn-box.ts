/**
 * The box an SVG drawing shows, in the SVG's own user space, for fitting it to the 2D pane.
 * `getBBox` ignores clipping, so a sheet viewport (a clipped `<use>` of the whole model) would
 * count the model far outside the viewport. Each outermost clipped element counts only inside
 * its clip path, and the rest is measured with those hidden. Null when nothing is drawn.
 *
 * Self-contained (no imports and no outer names), so the browser test runs it from its source.
 */
export function drawnBox(svg: SVGSVGElement): { x: number; y: number; width: number; height: number } | null {
  const clipped = Array.from(svg.querySelectorAll<SVGGraphicsElement>('[clip-path]')).filter(
    (el) => !el.parentElement?.closest('[clip-path]') && !el.closest('defs, clipPath'),
  );
  let x0 = Infinity;
  let y0 = Infinity;
  let x1 = -Infinity;
  let y1 = -Infinity;
  const add = (b: { x: number; y: number; width: number; height: number }, m: DOMMatrix | null): void => {
    if (!(b.width > 0 || b.height > 0) || !Number.isFinite(b.width + b.height)) return;
    for (const [x, y] of [
      [b.x, b.y],
      [b.x + b.width, b.y],
      [b.x, b.y + b.height],
      [b.x + b.width, b.y + b.height],
    ]) {
      const p = m ? new DOMPoint(x, y).matrixTransform(m) : { x, y };
      x0 = Math.min(x0, p.x);
      y0 = Math.min(y0, p.y);
      x1 = Math.max(x1, p.x);
      y1 = Math.max(y1, p.y);
    }
  };
  const display = clipped.map((el) => el.style.display);
  for (const el of clipped) el.style.display = 'none';
  try {
    add(svg.getBBox(), null);
  } finally {
    clipped.forEach((el, i) => {
      el.style.display = display[i];
    });
  }
  const rootCtm = svg.getCTM();
  const toRoot = rootCtm ? rootCtm.inverse() : null;
  for (const el of clipped) {
    let b = el.getBBox();
    const id = /url\(\s*["']?#([^"')\s]+)/.exec(el.getAttribute('clip-path') ?? '')?.[1];
    const clip = id ? svg.querySelector(`clipPath[id="${id.replace(/["\\]/g, '\\$&')}"]`) : null;
    if (clip) {
      let cx0 = Infinity;
      let cy0 = Infinity;
      let cx1 = -Infinity;
      let cy1 = -Infinity;
      for (const shape of Array.from(clip.children)) {
        if (!(shape instanceof SVGGraphicsElement)) continue;
        const c = shape.getBBox();
        cx0 = Math.min(cx0, c.x);
        cy0 = Math.min(cy0, c.y);
        cx1 = Math.max(cx1, c.x + c.width);
        cy1 = Math.max(cy1, c.y + c.height);
      }
      const ix0 = Math.max(b.x, cx0);
      const iy0 = Math.max(b.y, cy0);
      const ix1 = Math.min(b.x + b.width, cx1);
      const iy1 = Math.min(b.y + b.height, cy1);
      if (ix1 < ix0 || iy1 < iy0) continue;
      b = new DOMRect(ix0, iy0, ix1 - ix0, iy1 - iy0);
    }
    const ctm = el.getCTM();
    add(b, toRoot && ctm ? toRoot.multiply(ctm) : null);
  }
  return x1 > x0 || y1 > y0 ? { x: x0, y: y0, width: x1 - x0, height: y1 - y0 } : null;
}
