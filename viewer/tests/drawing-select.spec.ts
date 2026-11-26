import fs from 'node:fs';
import path from 'node:path';
import { expect, test, type Page } from '@playwright/test';

/**
 * Selecting elements in a model's sheets and views (src/drawing-select.ts), as Revit selects them
 * in its drawing area: hover pre-highlights the whole element and names it, a click selects it,
 * Tab cycles what lies under the cursor, Ctrl and Shift add and remove, a window selects what lies
 * inside it (left to right) or what it crosses (right to left), Esc clears and Ctrl+Left brings the
 * selection back; the panel shows the element and the 3D view and tree select it, and the other
 * way round.
 *
 * On the tiny rooms model (MIT, the gallery's), staged in public/demos. The timings run on the
 * Technical School sample's sheet A1 (RVT_SHEETS_MODEL), the largest public sheet the suite opens.
 */
const TINY = path.join(process.cwd(), 'public/demos/tiny-2023-4-rooms.rvt');
const SHEETS = process.env.RVT_SHEETS_MODEL ?? '';

async function openModel(page: Page, file: string): Promise<void> {
  await page.goto('/');
  await expect(page.locator('#status')).toHaveText(/Ready/);
  await page.locator('#file-input').setInputFiles(file);
  await expect(page.locator('#status')).toHaveText(/Loaded/, { timeout: 600_000 });
}

/** Opens a drawing from the list and waits for its elements to be read. */
async function openDrawing(page: Page, key: string): Promise<void> {
  const button = page.locator(`#drawings button[data-key^="${key}"]`).first();
  await expect(button).toBeVisible({ timeout: 120_000 });
  await button.click();
  await expect(button).toHaveAttribute('aria-pressed', 'true', { timeout: 600_000 });
  await expect(page.locator('#drawing-title')).toHaveText((await button.textContent()) ?? '');
  await expect(page.locator('#drawing-sheet > svg')).toHaveAttribute('data-indexed', /\d+/, { timeout: 600_000 });
}

interface Pick {
  id: string;
  x: number;
  y: number;
  nodes: number;
}

/**
 * Points on the screen on an element's line: for each element, the middle of its longest visible
 * line, at least `clear` px from any other element's line ends. Sorted by line length.
 */
async function picks(page: Page): Promise<Pick[]> {
  return page.locator('#drawing-sheet > svg').evaluate((svg) => {
    const pane = svg.getBoundingClientRect();
    const best = new Map<string, { x: number; y: number; len: number }>();
    const counts = new Map<string, number>();
    for (const el of Array.from(svg.querySelectorAll('[data-element]'))) {
      const id = el.getAttribute('data-element')!;
      counts.set(id, (counts.get(id) ?? 0) + 1);
      if (el.localName !== 'line' || el.closest('[data-viewport]')) continue;
      const m = (el as SVGGraphicsElement).getScreenCTM()!;
      const p = (x: number, y: number) => new DOMPoint(x, y).matrixTransform(m);
      const a = p(+el.getAttribute('x1')!, +el.getAttribute('y1')!);
      const b = p(+el.getAttribute('x2')!, +el.getAttribute('y2')!);
      const len = Math.hypot(b.x - a.x, b.y - a.y);
      const mid = { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 };
      if (mid.x < pane.left + 20 || mid.x > pane.right - 20 || mid.y < pane.top + 80 || mid.y > pane.bottom - 20) continue;
      if (len > (best.get(id)?.len ?? 0)) best.set(id, { ...mid, len });
    }
    return [...best]
      .sort((a, b) => b[1].len - a[1].len)
      .map(([id, b]) => ({ id, x: b.x, y: b.y, nodes: counts.get(id) ?? 0 }));
  });
}

/** Ids pre-highlighted, and ids selected. */
async function marked(page: Page, cls: 'sel-pre' | 'sel-on'): Promise<string[]> {
  return page
    .locator('#drawing-sheet > svg')
    .evaluate((svg, c) => [...new Set(Array.from(svg.querySelectorAll(`.${c}[data-element]`)).map((e) => e.getAttribute('data-element')!))].sort(), cls);
}

test('selects elements in a floor plan as Revit does, and in step with the 3D view', async ({ page }) => {
  test.skip(!fs.existsSync(TINY), 'the tiny rooms model is not staged');
  test.setTimeout(600_000);
  await openModel(page, TINY);
  await openDrawing(page, 'plan:');
  const sheet = page.locator('#drawing-sheet');
  const svg = page.locator('#drawing-sheet > svg');
  const all = await picks(page);
  expect(all.length).toBeGreaterThan(1);
  const [a, b] = all;

  // Hover: the whole element pre-highlights, the status line names it.
  await page.mouse.move(a.x + 30, a.y + 30);
  await page.mouse.move(a.x, a.y);
  await expect.poll(() => marked(page, 'sel-pre')).toEqual([a.id]);
  await expect(svg.locator(`.sel-pre[data-element="${a.id}"]`)).toHaveCount(a.nodes);
  const label = (await page.locator('#status').textContent()) ?? '';
  expect(label).toMatch(/ : /);
  await expect(sheet).toHaveAttribute('title', /.+/);

  // Click: selected, counted, shown in the panel and in the tree and the 3D view.
  await page.mouse.click(a.x, a.y);
  await expect(svg).toHaveAttribute('data-selected', '1');
  await expect(page.locator('#drawing-selection-count')).toHaveText('1 selected');
  expect(await marked(page, 'sel-on')).toEqual([a.id]);
  await expect(page.locator('#info')).toContainText(a.id);
  await expect(page.locator('.tree-node.selected')).toHaveCount(1);

  // Ctrl adds, Shift removes.
  await page.mouse.move(b.x + 20, b.y + 20);
  await page.mouse.move(b.x, b.y);
  await page.keyboard.down('Control');
  await page.mouse.click(b.x, b.y);
  await page.keyboard.up('Control');
  await expect(svg).toHaveAttribute('data-selected', '2');
  expect(await marked(page, 'sel-on')).toEqual([a.id, b.id].sort());
  await expect(page.locator('#info')).toContainText('2 elements selected');
  await page.keyboard.down('Shift');
  await page.mouse.click(a.x, a.y);
  await page.keyboard.up('Shift');
  expect(await marked(page, 'sel-on')).toEqual([b.id]);

  // Esc clears, in the tree too; Ctrl+Left brings it back.
  await sheet.focus();
  await page.keyboard.press('Escape');
  await expect(svg).toHaveAttribute('data-selected', '0');
  await expect(page.locator('#drawing-selection-count')).toBeHidden();
  await expect(page.locator('.tree-node.selected')).toHaveCount(0);
  await page.keyboard.press('Control+ArrowLeft');
  expect(await marked(page, 'sel-on')).toEqual([b.id]);

  // A click on empty paper clears.
  const box = (await sheet.boundingBox())!;
  await page.mouse.move(box.x + box.width - 5, box.y + box.height - 5);
  await page.mouse.click(box.x + box.width - 5, box.y + box.height - 5);
  await expect(svg).toHaveAttribute('data-selected', '0');

  // Tab: at a point near two elements, cycles through both and back with Shift+Tab.
  const corner = await svg.evaluate((root) => {
    const ends: { id: string; x: number; y: number }[] = [];
    for (const el of Array.from(root.querySelectorAll('line[data-element]'))) {
      const m = (el as SVGGraphicsElement).getScreenCTM()!;
      for (const [x, y] of [
        ['x1', 'y1'],
        ['x2', 'y2'],
      ]) {
        const p = new DOMPoint(+el.getAttribute(x)!, +el.getAttribute(y)!).matrixTransform(m);
        ends.push({ id: el.getAttribute('data-element')!, x: p.x, y: p.y });
      }
    }
    for (const e of ends) {
      if (ends.some((f) => f.id !== e.id && Math.hypot(f.x - e.x, f.y - e.y) < 2)) return e;
    }
    return null;
  });
  expect(corner).not.toBeNull();
  await page.mouse.move(corner!.x + 15, corner!.y + 15);
  await page.mouse.move(corner!.x, corner!.y);
  const first = await marked(page, 'sel-pre');
  expect(first.length).toBeGreaterThan(0);
  await sheet.focus();
  await page.keyboard.press('Tab');
  const second = await marked(page, 'sel-pre');
  expect(second).not.toEqual(first);
  await expect(sheet).toBeFocused();
  await page.keyboard.press('Shift+Tab');
  expect(await marked(page, 'sel-pre')).toEqual(first);
  // Clicking after Tab selects what Tab highlighted.
  await page.keyboard.press('Tab');
  await page.mouse.click(corner!.x, corner!.y);
  expect(await marked(page, 'sel-on')).toEqual(second);
  await page.keyboard.press('Escape');

  // A window left to right selects what lies wholly inside: around the whole drawing, every
  // element; a small one across a single line, nothing. Right to left, the small one crosses it.
  const drawn = await svg.evaluate((root) => {
    const r = (root.querySelector('g') as SVGGraphicsElement).getBoundingClientRect();
    return { x0: r.left - 10, y0: r.top - 10, x1: r.right + 10, y1: r.bottom + 10, ids: new Set(Array.from(root.querySelectorAll('[data-element]')).map((e) => e.getAttribute('data-element'))).size };
  });
  const drag = async (x0: number, y0: number, x1: number, y1: number) => {
    await page.mouse.move(x0, y0);
    await page.mouse.down();
    await page.mouse.move((x0 + x1) / 2, (y0 + y1) / 2, { steps: 4 });
    await page.mouse.move(x1, y1, { steps: 4 });
    await page.mouse.up();
  };
  await drag(Math.max(drawn.x0, box.x + 2), Math.max(drawn.y0, box.y + 60), Math.min(drawn.x1, box.x + box.width - 2), Math.min(drawn.y1, box.y + box.height - 2));
  await expect(svg).toHaveAttribute('data-selected', String(drawn.ids));
  await page.keyboard.press('Escape');
  await drag(a.x - 6, a.y - 6, a.x + 6, a.y + 6);
  await expect(svg).toHaveAttribute('data-selected', '0');
  await drag(a.x + 6, a.y + 6, a.x - 6, a.y - 6);
  expect(await marked(page, 'sel-on')).toContain(a.id);
  await page.keyboard.press('Escape');

  // The other way round: the element selected in the tree, with the drawing in the 3D view's
  // place, is selected in the drawing when it shows again.
  await page.mouse.click(a.x, a.y);
  const index = await page.locator('.tree-node.selected').getAttribute('data-entity-index');
  expect(index).not.toBeNull();
  await page.locator('#drawings button[data-key="model"]').click();
  await expect(page.locator('#drawing-pane')).toBeHidden();
  await page.keyboard.press('Escape');
  await expect(page.locator('.tree-node.selected')).toHaveCount(0);
  expect(await marked(page, 'sel-on')).toEqual([]);
  await page.locator(`.tree-node[data-entity-index="${index}"]`).click();
  await page.locator('#drawings button[data-key^="plan:"]').first().click();
  await expect(page.locator('#drawing-pane')).toBeVisible();
  expect(await marked(page, 'sel-on')).toEqual([a.id]);
});

test('reads a large sheet fast and hovers in well under a frame', async ({ page }) => {
  test.skip(!SHEETS || !fs.existsSync(SHEETS), 'RVT_SHEETS_MODEL is not set or absent');
  test.setTimeout(900_000);
  await openModel(page, SHEETS);
  const sheetButton = page.locator('#drawings button[data-key^="sheet:"]', { hasText: /^A1 / }).first();
  await expect(sheetButton).toBeVisible({ timeout: 120_000 });
  await sheetButton.click();
  const svg = page.locator('#drawing-sheet > svg');
  await expect(svg).toHaveAttribute('data-indexed', /\d+/, { timeout: 600_000 });
  const share = async () =>
    svg.evaluate((root) => {
      const prims = Array.from(root.querySelectorAll('[data-handle]'));
      const named = prims.filter((e) => e.hasAttribute('data-element')).length;
      return { primitives: prims.length, named, share: named / Math.max(1, prims.length) };
    });
  const index = Number(await svg.getAttribute('data-index-ms'));
  const box = (await page.locator('#drawing-sheet').boundingBox())!;
  // The first hover after the drawing is read, then a sweep across the sheet.
  const t0 = Date.now();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await expect(svg).toHaveAttribute('data-hover-ms', /"n":1/);
  const firstHoverWall = Date.now() - t0;
  for (let i = 0; i < 120; i++) {
    await page.mouse.move(box.x + 20 + ((box.width - 40) * i) / 120, box.y + 80 + ((box.height - 100) * ((i * 37) % 120)) / 120);
  }
  const hover = JSON.parse((await svg.getAttribute('data-hover-ms')) ?? '{}') as { n: number; median: number; max: number };
  const a1 = await share();
  console.log(
    'sheet A1 selection timing',
    JSON.stringify({ indexMs: index, items: Number(await svg.getAttribute('data-indexed')), firstHoverWallMs: firstHoverWall, hover }),
  );
  console.log('sheet A1 element ids', JSON.stringify(a1));
  expect(a1.share).toBeGreaterThan(0.5);
  expect(index).toBeLessThan(5000);
  expect(hover.median).toBeLessThan(16);

  // On a sheet a viewport's view selects as the viewport; a double-click activates the viewport,
  // and then its elements select; a double-click outside deactivates it.
  const inside = await svg.evaluate((root) => {
    const vp = root.querySelector('g[data-viewport][data-element]')!;
    const line = Array.from(vp.querySelectorAll('line[data-element]')).find((l) => {
      const r = l.getBoundingClientRect();
      return r.width + r.height > 30;
    })!;
    const m = (line as SVGGraphicsElement).getScreenCTM()!;
    const p = new DOMPoint((+line.getAttribute('x1')! + +line.getAttribute('x2')!) / 2, (+line.getAttribute('y1')! + +line.getAttribute('y2')!) / 2).matrixTransform(m);
    return { vp: vp.getAttribute('data-element')!, element: line.getAttribute('data-element')!, x: p.x, y: p.y };
  });
  await page.mouse.move(inside.x + 10, inside.y + 10);
  await page.mouse.move(inside.x, inside.y);
  await page.mouse.click(inside.x, inside.y);
  expect(await marked(page, 'sel-on')).toEqual([inside.vp]);
  await expect(page.locator('#info')).toContainText('Viewport');
  await page.mouse.dblclick(inside.x, inside.y);
  await expect(svg).toHaveClass(/has-active-viewport/);
  await page.mouse.move(inside.x + 10, inside.y + 10);
  await page.mouse.move(inside.x, inside.y);
  await page.mouse.click(inside.x, inside.y);
  const chosen = await marked(page, 'sel-on');
  expect(chosen).toHaveLength(1);
  expect(chosen).not.toEqual([inside.vp]);
  await page.mouse.dblclick(box.x + 5, box.y + box.height - 5);
  await expect(svg).not.toHaveClass(/has-active-viewport/);

  // The shares in a floor plan and a section, for the report.
  for (const key of ['plan:', 'section:']) {
    await openDrawing(page, key);
    console.log(`${key} element ids`, JSON.stringify(await share()), 'index ms', await svg.getAttribute('data-index-ms'));
  }
});
