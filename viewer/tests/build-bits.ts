// The decoder build a test run is on. CI builds the viewer once per build with
// RVT_VIEWER_BITS set (vite.config.ts), so the whole suite runs on the 32-bit
// build and again on the 64-bit one; unset, the page picks the 64-bit build
// where the browser accepts memory64, as a visitor's browser does.

/** The build the viewer under test uses by default, or undefined where the engine decides. */
export const EXPECTED_BITS: 32 | 64 | undefined =
  process.env.RVT_VIEWER_BITS === '32' ? 32 : process.env.RVT_VIEWER_BITS === '64' ? 64 : undefined;

/** The browser tab's memory limit the page names on this build, in GiB: 4 on the 32-bit build, 16 on the 64-bit one. */
export const CEILING_GIB = EXPECTED_BITS === 32 ? 4 : 16;
