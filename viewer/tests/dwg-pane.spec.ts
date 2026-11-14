import { expect, test } from '@playwright/test';
import { drawnBox } from '../src/drawn-box';

/**
 * The DWG / DXF 2D pane (src/drawing.ts): a hand-made R12 ASCII DXF with two
 * layers, a LINE, a CIRCLE and a TEXT opens in the pane, its layers toggle,
 * and a click shows the entity's row in the Details panel.
 */
const DXF = [
  ['0', 'SECTION'], ['2', 'HEADER'],
  ['9', '$ACADVER'], ['1', 'AC1009'],
  ['9', '$HANDLING'], ['70', '1'],
  ['9', '$HANDSEED'], ['5', 'FF'],
  ['0', 'ENDSEC'],
  ['0', 'SECTION'], ['2', 'TABLES'],
  ['0', 'TABLE'], ['2', 'LAYER'], ['70', '2'],
  ['0', 'LAYER'], ['2', 'WALLS'], ['70', '0'], ['62', '1'], ['6', 'CONTINUOUS'],
  ['0', 'LAYER'], ['2', 'NOTES'], ['70', '0'], ['62', '5'], ['6', 'CONTINUOUS'],
  ['0', 'ENDTAB'],
  ['0', 'ENDSEC'],
  ['0', 'SECTION'], ['2', 'ENTITIES'],
  ['0', 'LINE'], ['5', '20'], ['8', 'WALLS'], ['10', '0'], ['20', '0'], ['30', '0'], ['11', '100'], ['21', '60'], ['31', '0'],
  ['0', 'CIRCLE'], ['5', '21'], ['8', 'WALLS'], ['10', '150'], ['20', '40'], ['30', '0'], ['40', '20'],
  ['0', 'TEXT'], ['5', '22'], ['8', 'NOTES'], ['10', '10'], ['20', '80'], ['30', '0'], ['40', '5'], ['1', 'Hello'],
  ['0', 'ENDSEC'],
  ['0', 'EOF'],
]
  .map(([code, value]) => `${code.padStart(3)}\n${value}\n`)
  .join('');

test('opens a DXF in the 2D pane with layers, layouts and entity details', async ({ page }) => {
  await page.goto('/');
  await expect(page.locator('#status')).toHaveText(/Ready/);

  await page.locator('#file-input').setInputFiles({
    name: 'two-layers.dxf',
    mimeType: 'application/dxf',
    buffer: Buffer.from(DXF, 'latin1'),
  });

  const pane = page.locator('#drawing-pane');
  await expect(pane).toBeVisible();
  await expect(page.locator('#status')).toHaveText(/Loaded drawing/);
  await expect(page.locator('#viewport canvas')).toBeHidden();
  await expect(pane.locator('svg [data-handle]')).toHaveCount(3);

  await expect(page.locator('#drawing-layout option')).toContainText(['Model']);
  await expect(page.locator('#export-glb')).toBeDisabled();

  await page.getByRole('checkbox', { name: 'Layer NOTES' }).uncheck();
  await expect(pane.locator('g.layer[data-layer="NOTES"]')).toBeHidden();
  await expect(pane.locator('g.layer[data-layer="WALLS"]')).toBeVisible();

  await pane.locator('line[data-handle]').click();
  await expect(page.locator('#info')).toContainText('LINE');
  await expect(pane.locator('line.drawing-picked')).toHaveCount(1);
});

test('fits a sheet to what its viewports show, not the model clipped away', async ({ page }) => {
  // A sheet as rvt-dwg writes one: the model in <defs>, drawn through a clipped <use>
  // viewport, and a paper-space rectangle. The model's line runs far past the viewport.
  await page.setContent(`<svg id="s" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100" width="400" height="400">
    <defs><g id="model-space"><line x1="-100000" y1="0" x2="100000" y2="0" stroke="#000"/></g>
    <clipPath id="vp0"><rect x="10" y="10" width="20" height="20"/></clipPath></defs>
    <g transform="scale(1 -1)"><g clip-path="url(#vp0)"><use href="#model-space" transform="matrix(0.005 0 0 0.005 20 20)"/></g>
    <rect x="0" y="-50" width="50" height="40"/></g></svg>`);
  const [box, whole] = await page.evaluate((src) => {
    const fit = new Function(`return (${src})`)() as (s: SVGSVGElement) => DOMRect | null;
    const svg = document.getElementById('s') as unknown as SVGSVGElement;
    const b = svg.getBBox();
    return [fit(svg), { x: b.x, y: b.y, width: b.width, height: b.height }];
  }, drawnBox.toString());
  // getBBox counts the clipped line's whole length; the fit is the paper rectangle (0..50 by
  // 10..50) and the line's visible piece (10..30 at y -20).
  expect(whole.width).toBeGreaterThan(900);
  expect(box).not.toBeNull();
  for (const [k, v] of Object.entries({ x: 0, y: -20, width: 50, height: 70 })) {
    expect(box![k as 'x']).toBeCloseTo(v, 3);
  }
});
