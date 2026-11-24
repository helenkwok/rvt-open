// The synthetic models' .frag files (written by tests/material_textures.rs
// into public/frag/) drawn with their sidecars: every sample with texture
// coordinates takes them exactly, one run of vertices each.
import { expect, test } from "@playwright/test";

for (const name of ["textures", "big"]) {
  test(`${name}.frag: every textured sample lines up`, async ({ page }) => {
    await page.goto(`http://localhost:5197/?frag=/frag/${name}.frag&zoom=0.8`);
    await page.waitForFunction(() => window.__state?.ready, null, { timeout: 100_000 });
    const state = await page.evaluate(() => window.__state);
    expect(state.sidecar).toBe("applied");
    expect(state.samplesWithUv).toBeGreaterThan(0);
    expect(state.counts.exact).toBe(state.samplesWithUv);
    expect(state.counts.unaligned).toBe(0);
    expect(state.images).toBeGreaterThan(0);
    expect(state.texturedTiles).toBeGreaterThan(0);
  });
}

test("dispose puts every tile's own material back", async ({ page }) => {
  await page.goto("http://localhost:5197/?frag=/frag/textures.frag&dispose=1");
  await page.waitForFunction(() => window.__state?.ready, null, { timeout: 100_000 });
  const state = await page.evaluate(() => window.__state);
  expect(state.texturedTiles).toBeGreaterThan(0);
  expect(state.texturedTilesAfterDispose).toBe(0);
});
