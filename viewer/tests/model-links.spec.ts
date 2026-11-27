import fs from 'node:fs';
import path from 'node:path';
import { expect, test } from '@playwright/test';

/**
 * A Revit model's linked and imported files (src/model-links.ts): the Links list beside the scene
 * tree shows the Manage Links rows, a link whose file is not given stays listed as not loaded with
 * its saved path, and giving the linked file (its button, or dropping it) places it where the model
 * links it, the model kept.
 * Public BIM4LCA projects (VTT, CC BY-SA 4.0), which CI fetches: Housing Concrete BuildingPermit
 * (RVT_LINKS_HOST) links Housing Terrain-Concrete BuildingPermit (RVT_LINKS_CHILD), two IFC links
 * and a DWG. Skipped without them.
 */
const HOST = process.env.RVT_LINKS_HOST ?? '';
const CHILD = process.env.RVT_LINKS_CHILD ?? '';

test('lists the links and places a linked model the user gives', async ({ page }) => {
  test.skip(!HOST || !CHILD || !fs.existsSync(HOST) || !fs.existsSync(CHILD), 'RVT_LINKS_HOST / RVT_LINKS_CHILD not set or absent');
  test.setTimeout(1_200_000);
  await page.goto('/');
  await expect(page.locator('#status')).toHaveText(/Ready/);
  await page.locator('#file-input').setInputFiles(HOST);
  await expect(page.locator('#status')).toHaveText(/Loaded/, { timeout: 900_000 });

  const list = page.locator('#links');
  const rows = list.locator('tr[data-kind]');
  await expect(rows.first()).toBeVisible({ timeout: 300_000 });
  const kinds = await rows.evaluateAll((trs) => trs.map((tr) => (tr as HTMLElement).dataset.kind));
  console.log('links listed', JSON.stringify(kinds));
  expect(kinds.filter((k) => k === 'revit')).toHaveLength(1);
  expect(kinds.filter((k) => k === 'ifc')).toHaveLength(2);
  expect(kinds.filter((k) => k === 'cad')).toHaveLength(1);

  // The Revit link: not loaded when saved, Overlay, its saved path shown and nothing fetched.
  const link = list.locator('tr[data-kind="revit"]');
  await expect(link).toContainText(path.basename(CHILD));
  await expect(link).toContainText('Overlay');
  await expect(link).toHaveAttribute('data-status', 'Not Loaded');
  await expect(link).toContainText('Revit MAASTOT');
  await expect(list.locator('tr.links-instance')).toHaveCount(3);

  // Giving the linked file places it, and the host model stays open.
  const [chooser] = await Promise.all([page.waitForEvent('filechooser'), link.locator('button[data-link-open]').click()]);
  await chooser.setFiles(CHILD);
  await expect(link).toHaveAttribute('data-status', 'Loaded (file given)', { timeout: 900_000 });
  await expect(page.locator('#status')).toHaveText(/Linked/);
  await expect(link.locator('button[data-link-open]')).toHaveText('Remove');
});
