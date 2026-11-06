# Viewer build pipeline

The browser viewer in `viewer/` runs the Rust reader as WebAssembly
and draws its output with three.js. The Rust-side data model it calls
is described in [`docs/viewer-api.md`](viewer-api.md); this document
covers how the pieces are built, wired together and deployed.

## WebAssembly bindings

The bindings live in their own crate, [`rvt-wasm`](../rvt-wasm/src/lib.rs),
a `cdylib` that depends on the core library. The core library builds as a
plain `rlib` with no wasm feature, so native builds, test runs and
`cargo install` link no shared library. On `wasm32-unknown-unknown` the IFC
header's clock comes from `js-sys`.

Every exported function takes bytes or JSON-serialisable values and
returns the same, so the binding layer stays mechanical. The exports use
camelCase names (`#[wasm_bindgen(js_name = ...)]`): `openRvtBytes` and its
`...WithLimits` / `...WithDiagnostics...` variants, `quickSummary`,
`fileMetadata`, `buildSceneGraph`, `distinctIfcTypes`, `elementInfoPanel`,
`buildSchedule`, `scheduleCsv`, `modelToGlb`, `modelToPdf3d`,
`modelToIfcStep`, `renderPlanSvg`, and `memory_stats` / `reset_peak` for
memory diagnostics.

### Build commands

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version <the wasm-bindgen version in Cargo.lock>

cargo build -p rvt-wasm --release --target wasm32-unknown-unknown
wasm-bindgen --target web --out-dir viewer/pkg --out-name rvt \
    target/wasm32-unknown-unknown/release/rvt_wasm.wasm
# -> viewer/pkg/rvt_bg.wasm, rvt.js, rvt.d.ts
```

The `wasm-bindgen` CLI must match the `wasm-bindgen` crate version in
`Cargo.lock`; CI reads that version from the lockfile.

### CI checks

The `wasm` job in `.github/workflows/ci.yml` runs clippy on `rvt-wasm`,
builds the wasm32 package (with `rvt-wasm/js/rvt_open_auto.js` copied
beside it) and a wasm64 package (`tools/ci/wasm64-build.sh`) into one folder
with `tools/ci/wasm-package.sh`, and uploads it as the `wasm-pkg` artifact.
The release ships the same folder.

The no-network invariant of
[`docs/viewer-privacy-posture.md`](viewer-privacy-posture.md) is checked
in the viewer build by two Playwright specs:
`viewer/tests/no-network.spec.ts` runs the built viewer, and
`viewer/tests/wasm-no-network-imports.spec.ts` checks each import of the
compiled `.wasm`, by name and by its shim in the glue, through
`viewer/scripts/audit-wasm-imports.mjs`.

## JavaScript usage

```ts
import init, { openRvtBytesWithDiagnosticsMode, buildSceneGraph, modelToGlb } from '../pkg/rvt.js';

await init();  // load the .wasm

const bytes = new Uint8Array(await file.arrayBuffer());
const { model } = openRvtBytesWithDiagnosticsMode(bytes, 'scaffold');
const scene = buildSceneGraph(model);
const glb = modelToGlb(model);  // Uint8Array
```

## three.js integration

```ts
import { GLTFLoader } from 'three/examples/jsm/loaders/GLTFLoader.js';

const blob = new Blob([modelToGlb(model)], { type: 'model/gltf-binary' });
const url = URL.createObjectURL(blob);

new GLTFLoader().load(url, (gltf) => {
    scene.add(gltf.scene);
    URL.revokeObjectURL(url);
});
```

Camera: the viewer drives its own three.js `PerspectiveCamera` or
`OrthographicCamera` (`viewer/src/main.ts`); the Rust `CameraState` only
carries a pose inside a shared `ViewerState`.

Picking: `build_gltf` writes `{ entityIndex, ifcType }` as `extras` on
every element node, and `GLTFLoader` surfaces `extras` as `userData`. A
three.js `Raycaster` hit gives `userData.entityIndex`, which is passed to
`elementInfoPanel(model, entityIndex)`. `userData.ifcType` is what the
category toggles and the schedule's per-type highlight match on.

The info panel is rendered, not computed, in TypeScript. The Rust
`element_info_panel` returns display-ready rows: name, IFC type and
predefined type, the ElementId (`type_guid`, which the panel labels
ElementId and pairs with Revit's GlobalId from the model), the storey the
IFC export puts the element in (its `index` addresses the same `IFCBUILDINGSTOREY` scene node, so
the row is clickable), placement and extents in feet, the property set as
unit-carrying rows with booleans as `Yes` / `No`, and `host` / `hosted`
rows for jumping across a door-to-wall relationship. Absent optionals are
omitted; `missing` drives a "not recovered" note only where the export
diagnostics agree the field is a known decode gap. `build_schedule`
likewise returns `Schedule::groups`, the per-IFC-type breakdown (biggest
first) the schedule panel lists and highlights from. The schedule, the tree
and the panel take the storey of each element, and which elements they list,
from the export's own placement decision (`step_writer::placements`,
`storey_of`), as the MCP server and the elements CSV do. Unit formatting and
index resolution stay on the Rust side: the frontend never re-derives a
unit or an index to name.

## Web Worker

Decoding runs off the main thread in `viewer/src/worker.ts`, which
calls `openRvtBytesWithDiagnosticsModeAndOpenLimits` and
`buildSceneGraph` and posts the model and scene back. The main thread
creates it as a module worker:

```ts
const worker = new Worker(new URL('./worker.ts', import.meta.url), { type: 'module' });
worker.postMessage({ bytes });
worker.onmessage = (e) => renderScene(e.data.scene, e.data.model);
```

## File intake

Files arrive by drag and drop or the file picker (`viewer/src/main.ts`).
The picker accepts `.rvt`, `.rfa`, `.rte` and `.rft`, and the bytes go
straight to the worker. Under the client-side-only posture the file never
leaves the browser.

## Memory and decode time

wasm32 commits every page of linear memory it grows and traps at 4 GiB,
where a native allocator backs untouched capacity lazily. So the inflate
path bounds what each gzip member reserves up front (at most 1 MiB) and
trims the retained buffer to its real size. Any chunked or streaming path
must keep the same property: bound what a member reserves, and hold no
more inflated buffers alive than the scene needs.

Each partition stream is inflated once per file:
`RevitFile::inflated_partition` memoises the inflated stream per stream
path, `RevitFile::partition_string_records` memoises the UTF-16
extraction, string candidates are screened on the raw code units before
any decode, and the category sweep uses `memchr::memmem`.

The viewer's loading-card estimate uses `DECODE_BYTES_PER_SECOND`
(11 MB/s) in `viewer/src/main.ts` until the decoder's first progress
report; from then on the card shows the decoder's own stages (see
[`viewer-api.md`](viewer-api.md#decode-progress)). `build_scene_graph`
still returns the whole scene at once, so the model appears when the
decode ends; server-side pre-extraction is excluded by the
client-side-only posture.

## Static site

`.github/workflows/pages.yml` builds the project site: the landing page
in `site/` plus the viewer at `/viewer/`. Pull requests build only; pushes
to `main` and manual runs also deploy to GitHub Pages. The build job:

1. installs the pinned Rust toolchain with the wasm32 target and the
   matching `wasm-bindgen` CLI;
2. builds `rvt-wasm` and runs `wasm-bindgen` into `viewer/pkg`, as above;
3. fetches the demo samples: `magnetar-io/revit-test-datasets` into
   `_project_corpus/` and `phi-ag/rvt` into `_corpus/`, through
   `.github/actions/fetch-corpus`;
4. runs `pnpm install --frozen-lockfile` and `pnpm run build` in
   `viewer/` (the `prebuild` step runs `viewer/scripts/stage-demos.mjs`,
   which stages the hash-verified demos listed in
   [`docs/viewer-demos.json`](viewer-demos.json)), fails if a real-project
   demo was not staged, and runs the no-network specs and the
   real-project demo card regression test;
5. rebuilds the viewer with `RVT_VIEWER_BASE=/rvt-open/viewer/` for its
   Pages path and assembles `site/` and `viewer/dist` into one artifact.

The `deploy` job then publishes that artifact with
`actions/deploy-pages`. The workflow's concurrency group is per ref, so
queued pull-request builds do not cancel each other.

Layout of `viewer/`:

```
viewer/
├── index.html
├── src/            # main.ts, worker.ts, layout.ts, styles
├── scripts/        # stage-demos.mjs, audit-wasm-imports.mjs
├── tests/          # Playwright specs
├── pkg/            # wasm-bindgen output (git-ignored, built in CI)
└── vite.config.ts
```
