# Documentation

The pages under `docs/` are of two kinds. The user pages say what rvt-open does now and are kept current with the code. The contributor pages describe how the project is run.

## For users

- [User guide](user-guide.md): inspect, export and schedule Revit files from the command line.
- [Install](install.md): the binaries, `cargo install`, `pip install` and the container image.
- [Python](python.md): the `rvt-open` package's API (`import rvt_open`), with examples that run in CI.
- [Status](status.md) and [compatibility](compatibility.md): what each Revit release, file type and element class gives, with the evidence. The [support matrix](support-matrix.json) is the machine-readable ceiling.
- [Supported profile](supported-profile.md): the inputs rvt-open is measured on.
- [Evidence](evidence.md): the measured figures behind the README, by input, by element category and by known gap.
- [Library and tools](library.md): the command-line tools, the Rust modules and the browser viewer in more detail.
- [Rooms](rooms.md): what an exported `IfcSpace` holds for each release, and whether to trust its area.
- [MCP server](mcp.md): `rvt-open-mcp`, the local, read-only server an agent asks about a file.
- [Export diagnostics](export-diagnostics.md) and [diagnostic semantics](diagnostic-semantics.md): what an export says it approximated or left out.
- [`rvt-inspect`](inspect.md) and [`rvt-ifc-compare`](ifc-compare.md): checking a file before relying on it, and comparing an export with Revit's.
- [Validation evidence](validation-evidence.md) and the [verification protocol](verification-protocol.md): how an IFC export is checked, and what "verified" means.
- [Demos](demos.md) and [benchmarks](benchmarks.md): reproducible outputs and timings.
- [Viewer API](viewer-api.md) and [viewer privacy posture](viewer-privacy-posture.md): the browser viewer's bindings, and why it uploads nothing.
- [JSON schemas](schemas/README.md): what the CLIs print.
- [1.0 release criteria](release-1.0-criteria.md): what 1.0 requires, and where each requirement stands.

## For contributors and maintainers

[Maintaining](maintaining.md), the [corpus guide](corpus.md), [corpus intake](corpus-intake.md) and [sources](corpus-sources.md), the [reference models](reference-models.md), [extending the element decoders](extending-layer-5b.md), the [viewer build pipeline](viewer-build-pipeline.md), the [element-table record layout](elem-table-record-layout.md), [developing in a Codespace](codespaces.ipynb), and the [supply-chain policy](supply-chain-policy.md).

