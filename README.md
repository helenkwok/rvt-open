# rvt-open

[![OpenSSF Scorecard](https://api.scorecard.dev/projects/github.com/helenkwok/rvt-open/badge)](https://scorecard.dev/viewer/?uri=github.com/helenkwok/rvt-open)

**Read and convert Revit files without Revit.**

rvt-open is a clean-room toolkit for Autodesk Revit project, family and template files (`.rvt`, `.rfa`, `.rte`, `.rft`). It reads a file's elements, geometry, materials and parameters, and exports them to open formats. It comes as command-line tools, a Rust library, Python bindings and a WebAssembly package that runs in the browser tab, with no upload.

## What you can do with it

| Task | How |
|---|---|
| Read Revit 2008 to 2027 files | Opens every release from 2008 to 2027 and reports its release, worksharing state, previews and embedded schema; elements are decoded from 2010 on. Support per release is in [Support](#support). |
| Export IFCX first | IFCX (buildingSMART IFC5 alpha, following the published `ifcx.tsp`), plus IFC4, ThatOpen Fragments (`.frag`), OpenUSD (`.usda`, `.usdc`), glTF 2.0 binary, plan SVG and CSV schedules, from one command. |
| Get real geometry and materials | Family geometry from the family documents embedded in a project; saved meshes for doors, windows and other elements where the file stores them; saved materials and their colours in every format. |
| Convert in the browser | The `rvt-wasm` package reads a file inside the page and converts it; errors are readable messages, not `unreachable`. |
| Let an agent ask about a file | The [MCP server](docs/mcp.md), `rvt-open-mcp`: local, read-only, over stdio, built from source with `cargo build --release -p rvt-open-mcp`. |

## Quick start

**Command line** (Rust 1.89 or later, edition 2024):

```bash
cargo build --release
./target/release/rvt-info my-project.rvt                       # release, worksharing, previews, schema summary
./target/release/rvt-ifc my-project.rvt -o out.ifc \
    --ifcx out.ifcx --frag out.frag --usda out.usda --usdc out.usdc
./target/release/rvt-ifc my-project.rvt -o out.ifc --guid-namespace "my-company ids"   # namespace of generated GlobalIds
./target/release/rvt-gltf my-project.rvt -o out.glb
./target/release/rvt-sheet my-project.rvt -o plan.svg
./target/release/rvt-schedule my-project.rvt                   # element and room schedules as CSV
```

The GlobalIds the IFC export generates (relations, property sets, anything Revit gives no id) are derived under a namespace you can set: `--guid-namespace <TEXT>` or `RVT_OPEN_GUID_NAMESPACE` on `rvt-ifc`, `guid_namespace` in the wasm convert options, `guid_namespace=` in Python `write_ifc`. The default is `rvt-open ifc GlobalId v1`. Revit's own GlobalIds never change. See [`docs/compatibility.md`](docs/compatibility.md).

**Data outputs**: an element table (JSON Lines, XLSX, Parquet, SQLite), IFC options, OBJ, run reports and batch conversion with an output manifest ([`docs/data-outputs.md`](docs/data-outputs.md)):

```bash
./target/release/rvt-schedule my-project.rvt --format jsonl     # every element with all its parameters, one JSON record per line
./target/release/rvt-schedule my-project.rvt --format parquet   # the same table as Parquet (also xlsx, sqlite)
./target/release/rvt-ifc my-project.rvt -o out.ifc --ifc-profile coordination --ifc-units mm \
    --ifc-origin survey-point --ifc-categories Walls,Doors --phase "New Construction" --report run.json
./target/release/rvt-gltf my-project.rvt --format obj           # my-project.obj + .mtl, one group per element
./target/release/rvt-batch projects/ -o converted/ --formats ifc,table,glb,obj   # one folder per file, one JSON status line each
```

Every export prints diagnostics saying what was approximated or left out; read them before using a file for measurement or coordination. `--help` on any tool lists its options; [`docs/install.md`](docs/install.md) has the details.

**Rust** (package `rvt-open`, library name `rvt`): the [examples](examples/README.md) open a file, list its elements and export it.

**Python** (build the bindings with maturin, see [`docs/python.md`](docs/python.md)):

```bash
maturin build --release --manifest-path rvt-py/Cargo.toml
```

```python
import rvt_open

f = rvt_open.RevitFile("my-project.rvt")
print(f.version)
open("out.ifc", "w").write(f.write_ifc())
```

**Browser / wasm**:

```bash
wasm-pack build rvt-wasm --target web --out-dir ../viewer/pkg --out-name rvt
```

The package exposes the 28 JS-callable wasm-bindgen bindings listed in [`docs/viewer-api.md`](docs/viewer-api.md#the-javascript-bindings): open a file, then `convert_ifc`, `convert_ifcx`, `convert_frag`, `convert_usda`, `convert_usdc`, `convert_glb`, `schedules_csv` or `convert_all`. The privacy checks are in [`docs/viewer-privacy-posture.md`](docs/viewer-privacy-posture.md).

Rust 2024 edition (MSRV 1.89). **Eighteen CLIs ship** (`rvt-analyze`, `rvt-info`, `rvt-inspect`, `rvt-schema`, `rvt-history`, `rvt-diff`, `rvt-corpus`, `rvt-dump`, `rvt-doc`, `rvt-ifc`, `rvt-ifc-compare`, `rvt-gltf`, `rvt-sheet`, `rvt-elem-table`, `rvt-elements`, `rvt-capabilities`, `rvt-schedule`, `rvt-batch`).

## Support

Support depends on the Revit release, the file type and the feature. [`docs/support-matrix.json`](docs/support-matrix.json) records each capability with its evidence and status; `rvt-capabilities --matrix -f text` prints it.

| Revit release | Open, metadata, schema | Elements and geometry |
|---|---|---|
| 2023 to 2027 | yes | typed elements with their saved parameters, storeys, rooms, hosts, family and type names, layer sets and materials; geometry decoded where the file's bytes allow, the element's bounding box elsewhere |
| 2015 to 2022 | yes | typed elements, storeys, hosts and materials; geometry as above |
| 2010 to 2014 | yes | experimental typed elements |
| 2008 to 2009 | yes | opens and reports its release |
| Families (`.rfa`) | yes | family metadata, previews, types and parameters, embedded family geometry |
| Templates (`.rte`, `.rft`) | same container code | not measured |

The tests under `tests/` pin each behaviour on Autodesk's public sample projects and on synthetic fixtures; each export's diagnostics list what a given file did not give.

## Every export is validated

CI checks each format with its official validator ([`tools/ci/validators/`](tools/ci/validators/)):

| Format | Validator |
|---|---|
| IFC4 | IfcOpenShell |
| glTF | Khronos glTF-Validator |
| OpenUSD (`.usda`, `.usdc`) | Pixar UsdValidation |
| IFCX | buildingSMART `ifcx` compose, and the schema generated from `ifcx.tsp` |
| ThatOpen Fragments | `@thatopen/fragments` 3.6.0 |

## Verifying downloads

Each release archive has a build provenance attestation, an entry in `SHA256SUMS`, a CycloneDX SBOM, a Trivy report and a ClamAV scan that must be clean before the release is published. `scans.md` collects the ClamAV result and, when those services are configured, each archive's VirusTotal and MetaDefender Cloud report links; Hybrid Analysis results are in the release workflow's log. To check that an archive was built by this repository's release workflow:

```sh
gh attestation verify rvt-open-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz -R helenkwok/rvt-open
```

[SECURITY.md](SECURITY.md#verifying-a-release) covers every check, and how to report a vulnerability privately.

## Known limits

- Undecoded element families are drawn as boxes (the element's bounding box), and the diagnostics say so.
- In the browser, the 32-bit build has a 4 GiB memory ceiling; files over about 130 MB can exceed it. The browser package also holds a 64-bit build (memory64, up to 16 GiB), which `rvt_open_auto.js` loads where the browser supports it. For the largest files, use the command line.
- Releases before 2023 have less decoded than 2023 to 2027; older releases may lack saved graphics, so their elements are boxes.
- Not affiliated with Autodesk; Revit is a trademark of Autodesk, Inc.

## Documentation

[User guide](docs/user-guide.md), [install](docs/install.md), [Python](docs/python.md), [viewer API](docs/viewer-api.md), [export diagnostics](docs/export-diagnostics.md), [contributing](CONTRIBUTING.md), [security](SECURITY.md), [changelog](CHANGELOG.md).

## Clean room

rvt-open is built from public observations of the file format and public documentation, never from Autodesk code. The rules and the evidence trail are in [`CLEANROOM.md`](CLEANROOM.md).

## License

Apache-2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
