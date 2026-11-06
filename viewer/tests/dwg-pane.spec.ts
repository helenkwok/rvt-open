import { expect, test } from '@playwright/test';

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
