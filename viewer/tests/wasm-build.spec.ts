import { expect, test, type Page } from '@playwright/test';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { EXPECTED_BITS } from './build-bits';

// Which decoder build the viewer runs: the 64-bit one (memory64, up to 16 GiB)
// where the browser accepts memory64, the 32-bit one (4 GiB) where it does not,
// `?bits=32|64` to ask for one, and the 32-bit build when the 64-bit one will
// not instantiate. CI runs the whole suite once per build (RVT_VIEWER_BITS at
// `vite build`); this file checks the choice itself.

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const tier1 = path.resolve(
  __dirname,
  '../../corpus/tier1/architectural-2024/architectural-2024.rvt',
);

/** The smallest module with a 64-bit memory: what rvt_open_auto.js validates. */
const MEMORY64_PROBE = [0, 97, 115, 109, 1, 0, 0, 0, 5, 3, 1, 4, 0];

async function browserHasMemory64(page: Page): Promise<boolean> {
  return page.evaluate(
    (bytes) => WebAssembly.validate(new Uint8Array(bytes)),
    MEMORY64_PROBE,
  );
}

async function openTier1(page: Page, query: string): Promise<void> {
  await page.goto(`/${query}`);
  await expect(page.locator('#status')).toHaveText(/Ready/);
  await page.locator('#file-input').setInputFiles(tier1);
}

async function reportedBits(page: Page): Promise<{ bits: number; stats: number; line: string }> {
  await expect(page.locator('#status')).toHaveText(/Loaded/);
  const stats = JSON.parse((await page.locator('#status').getAttribute('data-load-stats')) ?? 'null');
  return {
    bits: Number(await page.locator('#status').getAttribute('data-wasm-bits')),
    stats: stats.bits,
    line: (await page.locator('#wasm-build').textContent()) ?? '',
  };
}

test('the build the suite was told to use is the build that opens the file', async ({ page }) => {
  await page.goto('/');
  const supported = await browserHasMemory64(page);
  // CI's Chromium has memory64: a 64-bit leg that ran the 32-bit build would be a silent fallback.
  if (process.env.CI) expect(supported, 'the CI browser accepts memory64').toBe(true);
  await openTier1(page, '');
  const got = await reportedBits(page);
  const want = EXPECTED_BITS ?? (supported ? 64 : 32);
  expect(got.bits).toBe(want);
  expect(got.stats).toBe(want);
  expect(got.line).toContain(`${want}-bit decoder`);
  expect(got.line).toContain(want === 64 ? '16 GiB' : '4 GiB');
});

test('?bits=32 asks for the 32-bit build whatever the default is', async ({ page }) => {
  await openTier1(page, '?bits=32');
  const got = await reportedBits(page);
  expect(got).toMatchObject({ bits: 32, stats: 32 });
  expect(got.line).toContain('4 GiB');
});

test('?bits=64 asks for the 64-bit build, and opens the file on it', async ({ page }) => {
  await page.goto('/');
  const supported = await browserHasMemory64(page);
  if (process.env.CI) expect(supported, 'the CI browser accepts memory64').toBe(true);
  test.skip(!supported, 'this browser does not accept memory64');
  await openTier1(page, '?bits=64');
  const got = await reportedBits(page);
  expect(got).toMatchObject({ bits: 64, stats: 64 });
  expect(got.line).toContain('16 GiB');
  await expect(page.locator('#file-meta')).toContainText(/[1-9]\d* entities/);
});

test('a browser that does not accept memory64 gets the 32-bit build', async ({ page }) => {
  // The loader asks the engine with WebAssembly.validate: say no, in the page and its workers.
  await page.addInitScript(() => {
    const real = URL.createObjectURL.bind(URL);
    const prelude =
      '(() => { const v = WebAssembly.validate.bind(WebAssembly);' +
      'WebAssembly.validate = (b) => (b instanceof Uint8Array && b.length === 13 && b[8] === 5 && b[11] === 4 ? false : v(b)); })();\n';
    const realValidate = WebAssembly.validate.bind(WebAssembly);
    WebAssembly.validate = (b: BufferSource) =>
      b instanceof Uint8Array && b.length === 13 && b[8] === 5 && b[11] === 4 ? false : realValidate(b);
    URL.createObjectURL = (object: Blob | MediaSource) =>
      object instanceof Blob && /javascript/.test(object.type)
        ? real(new Blob([prelude, object], { type: object.type }))
        : real(object);
  });
  await openTier1(page, '?bits=auto');
  const got = await reportedBits(page);
  expect(got).toMatchObject({ bits: 32, stats: 32 });
  expect(got.line).toContain('4 GiB');
});

/** Makes the first WebAssembly instantiation in every worker fail, as a 64-bit module the engine refuses would. */
async function failFirstInstantiate(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const real = URL.createObjectURL.bind(URL);
    const prelude = `(() => {
      let refused = false;
      for (const name of ['instantiateStreaming', 'instantiate']) {
        const original = WebAssembly[name];
        WebAssembly[name] = async (...args) => {
          if (!refused) { refused = true; throw new WebAssembly.CompileError('refused for the test'); }
          return original.apply(WebAssembly, args);
        };
      }
    })();\n`;
    URL.createObjectURL = (object: Blob | MediaSource) =>
      object instanceof Blob && /javascript/.test(object.type)
        ? real(new Blob([prelude, object], { type: object.type }))
        : real(object);
  });
}

test('the 32-bit build is the fallback when the 64-bit one will not instantiate', async ({ page }) => {
  await page.goto('/');
  test.skip(!(await browserHasMemory64(page)), 'this browser does not accept memory64');
  await failFirstInstantiate(page);
  // The first instantiation is the 64-bit module's: refused, the loader goes on to the 32-bit one.
  await openTier1(page, '?bits=auto');
  const got = await reportedBits(page);
  expect(got).toMatchObject({ bits: 32, stats: 32 });
  await expect(page.locator('#file-meta')).toContainText(/[1-9]\d* entities/);
});

test('?bits=64 is an error, not a fallback, when the 64-bit build will not instantiate', async ({ page }) => {
  await page.goto('/');
  test.skip(!(await browserHasMemory64(page)), 'this browser does not accept memory64');
  await failFirstInstantiate(page);
  await openTier1(page, '?bits=64');
  await expect(page.locator('#status')).toHaveText(/^Error: .*refused for the test/s);
  await expect(page.locator('#status')).not.toHaveAttribute('data-wasm-bits', /.+/);
});
