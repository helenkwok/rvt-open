// The decoder build a test run is on. CI builds the viewer once per build with
// RVT_VIEWER_BITS set (vite.config.ts), so the whole suite runs on the 32-bit
// build and again on the 64-bit one; unset, the page picks the 64-bit build
// where the browser accepts memory64, as a visitor's browser does.

/** The build the viewer under test uses by default, or undefined where the engine decides. */
export const EXPECTED_BITS: 32 | 64 | undefined =
  process.env.RVT_VIEWER_BITS === '32' ? 32 : process.env.RVT_VIEWER_BITS === '64' ? 64 : undefined;

/**
 * The memory limit in GiB the page names, from the build that ran (`data-wasm-bits` on #status, which the
 * worker's report sets) and not from the text under test: 4 for the 32-bit build, 16 for the 64-bit one.
 */
export async function ceilingGiB(page: import('@playwright/test').Page): Promise<number> {
  const bits = await page.locator('#status').getAttribute('data-wasm-bits');
  if (bits !== '32' && bits !== '64') throw new Error(`no decoder build reported: data-wasm-bits is ${bits}`);
  return bits === '32' ? 4 : 16;
}
