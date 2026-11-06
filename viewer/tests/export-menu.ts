import { expect, type Page } from '@playwright/test';

/**
 * The export controls (IFC bar, IFC, glTF, 3D PDF, plan SVG) live in the one
 * Export menu in the top bar. Opens it if it is closed, so a test can use a
 * control in it the way a person would.
 */
export async function openExportMenu(page: Page): Promise<void> {
  const toggle = page.locator('#export-toggle');
  if ((await toggle.getAttribute('aria-expanded')) !== 'true') await toggle.click();
  await expect(page.locator('#export-menu')).toBeVisible();
}
