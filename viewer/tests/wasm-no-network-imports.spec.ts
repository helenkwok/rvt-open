import { expect, test } from '@playwright/test';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { BUILDS, auditWasmImports, canaryFailure, moduleImporting } from '../scripts/audit-wasm-imports.mjs';

/**
 * Compile-time invariant: the packaged WASM must not import
 * network primitives. Complements the browser traffic test in
 * `no-network.spec.ts` (which needs a sample file) — this check always
 * runs against both builds' wasm and glue in `viewer/pkg` (`rvt_open` and `rvt_open64`),
 * through `scripts/audit-wasm-imports.mjs`.
 */

const __dirname = path.dirname(fileURLToPath(import.meta.url));

// Both builds of the package are audited: the 32-bit one and the memory64 one.
for (const name of BUILDS) {
  test(`compiled WASM (${name}) imports no network primitives`, () => {
    const wasmPath = path.resolve(__dirname, `../pkg/${name}_bg.wasm`);
    const gluePath = path.resolve(__dirname, `../pkg/${name}.js`);
    expect(fs.existsSync(wasmPath), `missing ${wasmPath} — build viewer/pkg first`).toBe(true);
    expect(fs.existsSync(gluePath), `missing ${gluePath} — build viewer/pkg first`).toBe(true);

    const { imports, hits, unread } = auditWasmImports(
      fs.readFileSync(wasmPath),
      fs.readFileSync(gluePath, 'utf8'),
    );
    expect(imports, 'the wasm has no imports to read').toBeGreaterThan(0);
    expect(unread, `imports with no shim the audit can read:\n${unread.join('\n')}`).toEqual([]);
    expect(hits, `network imports:\n${hits.join('\n')}`).toEqual([]);
  });
}

// wasm-bindgen names a binding of `fetch` `__wbg_fetch_<hash>`. An audit that
// does not flag a module importing one cannot fail on the real wasm either.
test('the import audit flags a module that imports fetch (canary)', () => {
  const { hits } = auditWasmImports(
    moduleImporting(['__wbg_fetch_0']),
    '__wbg_fetch_0: function(arg0) { return fetch(arg0); }',
  );
  expect(hits, "the audit missed the canary's fetch import").not.toEqual([]);
  expect(canaryFailure(), 'the audit script misread its own canary').toBeNull();
});
