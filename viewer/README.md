# rvt-open viewer

Zero-upload, client-side Revit viewer powered by
`rvt-open` compiled to WebAssembly.

Hosted at the locally built viewer.

Install and local build paths are documented in
[`docs/install.md`](../docs/install.md).

## Privacy posture

- **Nothing leaves the browser tab.** The dropped `.rvt` / `.rfa` /
  `.rte` / `.rft` bytes are parsed in-process. There are no `fetch`,
  `XMLHttpRequest`, `WebSocket`, `EventSource`, or `sendBeacon`
  imports in the compiled WebAssembly — the CI job
  `.github/workflows/deploy-viewer.yml` fails the build if any show
  up.
- **No telemetry.** The page CSP blocks `connect-src` to everything
  except `self`, which is only needed for the static WASM bundle,
  and `blob:`, which is needed for the local GLB object URL. No
  analytics, no error reporting, no CDN font loads.
- **Browser-tested invariant.** CI runs
  `viewer/tests/no-network.spec.ts` (runtime traffic) and `viewer/tests/wasm-no-network-imports.spec.ts` (the import audit of `scripts/audit-wasm-imports.mjs`): it opens the built viewer,
  loads a public sample RFA, and fails with request URL plus initiator
  if any external or post-open network request occurs. See
  [`docs/viewer-privacy-posture.md`](../docs/viewer-privacy-posture.md).
- **Deterministic builds.** Every JS bundle is built from a pinned
  action SHA; the WASM is built by `tools/ci/wasm-package.sh` with a `wasm-bindgen` pinned in `Cargo.lock`.

## Local dev

```bash
# Run this block from the repository root.
# 1. Build both decoders (32-bit and 64-bit) and the loader into viewer/pkg/.
#    Needs the wasm32 target, wasm-bindgen-cli at the version Cargo.lock pins, Node, and the nightly
#    toolchain tools/ci/wasm64-build.sh names (it adds rust-src itself).
tools/ci/wasm-package.sh viewer/pkg

# 2. Install node deps
cd viewer
pnpm install --frozen-lockfile

# 3. Serve with Vite (localhost:5173)
pnpm run dev
```

Drop a Revit file onto the page, or open a redistributable demo from the
gallery (staged from `docs/viewer-demos.json` via `pnpm run stage:demos`).
Nothing is sent anywhere.

## Supported MVP workflow

1. Open a file locally (drop / picker / demo gallery, or `?demo=<id>` in the address).
2. Read File status: Decode confidence, Export confidence, and IFC bar.
3. Inspect decoded entities in the tree or viewport; a material reads its render appearance (swatch, schema, bitmaps) on request.
4. Export IFC / glTF / 3D PDF / plan only after checking the quality label; IFCX, its appearance layer, Fragments, OpenUSD and the appearance assets are written from the file itself in the worker, as `rvt-ifc` writes them.
5. Download diagnostics for scaffold or partial exports.

## Demo gallery

Each card says in a line what the sample shows. The samples, all redistributable and each staged only when its bytes match the sha256 the catalog pins:

| Sample | Source | Shows |
|---|---|---|
| Einhoven (2023), Core Interior (2024) | magnetar-io/revit-test-datasets, MIT (`_project_corpus/`) | a curtain wall of 24 framed panels and a wall with 2 windows; 15 levels, 1,000 elements, plans and sections |
| Door family (2020) | the same, `Revit/Families/Doors` | a door family in its template's sample wall, the opening cut |
| RAC basic sample family (2024, 2026) | phi-ag/rvt (`_corpus/`) | a family's forms, materials and their render appearances |
| Tiny reference models (2018 to 2026) | this project's release assets fixtures-tiny-1 and -2, MIT (`tools/ci/fetch-tiny-models.sh _tiny`) | stairs and railings, a curtain wall, flipped doors and windows in their host, columns and beams, ducts and pipes, family instances, rooms, a hip roof |
| Tier-1 synthetics | `corpus/tier1/` | file status and a scaffold export (~25% confidence is expected) |

`tests/gallery-examples.spec.ts` opens every example and checks what its card says it shows, and downloads each export the worker writes.

## Progress while a model opens

The loading card shows the decoder's own progress, the stages `rvt-gltf
--progress` prints: the bar fills with a stage's count while it has one
(scanning for elements, decoding elements, reading partition streams, drawing
family instances from saved meshes) and sweeps for the rest, and the card
names the stage and counts it. `src/worker.ts` passes a function to
`openRvtBytesWithDiagnosticsModeAndOpenLimits` and posts each report from
inside the wasm call; the page keeps the list on `#status` as
`data-load-progress`. The "about N s" estimate from the file's size is what
the card says until the first report. `tests/load-progress.spec.ts` watches
the card fill on Core Interior. The stages and what the card cannot show are
in [`docs/viewer-api.md`](../docs/viewer-api.md#decode-progress).

## Large models and resource limits

A file opens under the reader's default limits (256 MiB per stream and per
inflated member). When a stream is over them, File status reports "Part of the
file not read" and offers "Retry with large-model limits", which reads the same
file again with the limits `rvt-ifc --large-model` uses (streams up to 2 GiB).
A browser tab can use at most 4 GiB of memory with the 32-bit decoder and 16 GiB
with the 64-bit one (which the viewer runs where the browser accepts memory64;
a browser can give a tab less than its decoder can address), so a very large
model can still fail to open; `rvt-ifc --large-model` is not held to a tab's limit (its caps are
a 2 GiB stream and, on a 64-bit machine with the memory, 8 GiB inflated, and a
file over 2 GiB is rejected whatever the flags). The button calls
`openRvtBytesWithDiagnosticsModeAndOpenLimits` with `{ largeModel: true }`.

A file whose size says it may need more than a tab's limit, 4 GiB or 16 GiB by decoder (an open has taken
at least about ten times the file's size, and the warning can fire on a file
that fits) is warned about in the loading card before the decode, and still
tried. When the wasm instance does run out of memory, the viewer says the model
is too large for the browser, how much memory the tab reached, and what reads a
larger file (`rvt-ifc` and `rvt-gltf` with `--large-model`, up to a 2 GiB file;
the Python `RevitFile` with a larger `max_stream_bytes`, `max_inflate_bytes` and,
over 2 GiB, `max_file_bytes`), instead of
`RuntimeError: unreachable`; the engine's own message is in the status panel.
A stop a long way short of the ceiling on a small file says "Ran out of memory"
and that the file may be damaged, since nothing measured says it needs more. How
it tells, the measured estimate and the options for larger models are in
[`docs/viewer-api.md`](../docs/viewer-api.md#models-too-large-for-the-browser).
`?memoryCeilingBytes=` in the address lowers the ceiling the warning compares
with. `tests/memory-ceiling.spec.ts` uses it for the warning, and
`tests/memory-ceiling-capped.spec.ts` for the stop, in a Chromium whose V8 caps
wasm memory (`--js-flags=--wasm-max-mem-pages=256` is 16 MiB), where Einhoven
really stops, because no model of several hundred MiB is in CI.
`tests/worker-failures.spec.ts` covers the other engines' wording of the trap, a
panic that is not memory, and a stop after the open call, by failing one wasm
export in a replacement worker; nothing in it ships.

`?maxStreamBytes=` and `?maxInflateBytes=` in the address open the first file
under those limits, so a small file reaches the same path. The browser test
`tests/large-model-retry.spec.ts` opens the tier-1 fixture at
`/?maxStreamBytes=200`, which stops its 236-byte BasicFileInfo, and retries.

## Limits on what a file can make the page hold

The page bounds what a large or crafted file can make it keep. The scene
tree builds each group's rows (a storey's categories, a category's elements, an
element's parts) a block of 500 at a time: the groups of a model of up to
150,000 rows are all built when it opens, as they always were, and past that a
group builds its first block when it is opened, and a "Show more" row at its end
builds the next. The filter looks at every element, lists the first 2,000 it
finds, builds their rows, and says when there are more; a click on an element in
the 3-D view or a jump from the element panel builds the row it needs. The phase
field lists at most 200 phases (the first 199 and the last, the default), the
page keeps at most 2,000 of the decoder's progress reports and of what the
loading card said, and the saved parameters it remembers are bounded in
characters as well as in elements (64 elements and 8 Mi characters).
`?treeRowsShown=` and `?treeRowsEager=` in the address lower the first two,
`?parametersRemembered=` and `?parametersRememberedCharacters=` the last; none
raises a limit. `tests/tree-rows.spec.ts` opens Einhoven with a block of 3 rows
and nothing built up front, opens every group and every "Show more" row, and
expects the elements the page lists without the limits, in the same order.

## Family instances from saved meshes

Every element of a Revit 2014 to 2027 file that would be a bounding box (doors,
windows, furniture, fixtures, equipment and the rest) is drawn from the mesh
Revit saved for it, where the whole mesh decodes, as `rvt-ifc` and `rvt-gltf`
draw them by default; the others keep their bounding box. File
status has a "Draw family instances from saved meshes" option, on by default.
Changing it reopens the open file with `savedMeshes` in the options of
`openRvtBytesWithDiagnosticsModeAndOpenLimits`, and `?savedMeshes=0` in the
address starts with it off (for every file, until the option is turned on).
The Shapes row says how many bodies came from saved meshes.

Reading the saved graphics is a second pass over the file, so it costs time
and memory. `tests/saved-meshes-measure.spec.ts` opens each reference model
with the option on and off in a fresh browser context and writes the load time,
the wasm memory after the load and the number of bodies drawn from saved meshes
to `measurements/saved-meshes.json`; Deploy viewer runs it and uploads the
file. Run it with `RVT_VIEWER_MEASURE=1 pnpm exec playwright test`; the
reference models are read from `_project_corpus/Revit`.

## Architecture

```text
User drops .rvt
     │
     ▼
main.ts ── postMessage ──▶ worker.ts
                                │
                                ├── init wasm (pkg/rvt_open64.js where memory64 is accepted, else pkg/rvt_open.js)
                                ├── openRvtBytesWithDiagnosticsModeAndOpenLimits()
                                │                         → IfcModel + diagnostics JSON
                                ├── buildSceneGraph()     → SceneNode JSON
                                ├── modelToGlb()          → Uint8Array
                                └── buildSchedule()       → Schedule JSON
                                │
                          postMessage back
                                │
                                ▼
main.ts
  ├── Three.js scene + OrbitControls + GLTFLoader
  ├── scene tree panel
  ├── category toggles
  ├── export-quality label + IFC bar selector
  ├── File status (decode / export confidence)
  ├── element-info panel (raycast picking) + host / hosted jump rows
  └── status line
```

The worker is mandatory — parsing a 100 MB RFA blocks the
main thread for seconds otherwise.

## Build outputs

- `viewer/pkg/` — generated by `tools/ci/wasm-package.sh` (git-ignored): `rvt_open*` and `rvt_open64*` (the two decoder builds) and `rvt_open_auto.js`
- `viewer/node_modules/` — npm (git-ignored)
- `viewer/dist/` — Vite build (git-ignored)

All three are produced by the deploy workflow on every push to
`main`.
