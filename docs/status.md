# Project status

This page is the short answer to "can rvt-open do what I need?". It describes
`main`. The detail lives in:

- [`supported-profile.md`](supported-profile.md): what rvt-open can be trusted
  with, per input and per level (opened, elements identified, geometry,
  properties).
- [`support-matrix.json`](support-matrix.json): the **executable support matrix**
  (schema:
  [`support-matrix.schema.json`](schemas/support-matrix.schema.json)). Every
  capability, its status ceiling, its evidence and what it means for a user.
  `tests/support_matrix.rs` keeps its statuses honest, and
  `rvt-capabilities --matrix -f text` prints it from any installed build.
- [`release-1.0-criteria.md`](release-1.0-criteria.md): the requirements for
  1.0.0, each with its bar, the value measured now and its status.
- [`compatibility.md`](compatibility.md): release gates, file types, decoded
  element classes and IFC4 export coverage, checked against the source.
- [`reference-models.md`](reference-models.md): the models the measurements
  are made on, and how to fetch them.
- [`verification-protocol.md`](verification-protocol.md): what "verified"
  means for an IFC claim.

Keep that matrix, this page, the README and the viewer support matrix aligned.

## Scope

rvt-open reads Revit project, family and template files (`.rvt`, `.rfa`,
`.rte`, `.rft`) and exports what it reads to open formats. It does not write
or edit Revit files: rvt-open reads and exports only.

It ships as command-line tools, a Rust library, Python bindings (`rvt-open`,
imported as `rvt_open`) and a WebAssembly package that converts a file inside
the browser tab, with no upload.

## What it reads

| Revit release | Opened (container, metadata, previews, schema) | Elements |
|---|---|---|
| 2024, 2025 | yes | ElementId, category, name, type (as an IFC type object), storey, materials, layers, GlobalId, the parameters named for a standard property set, and a body that is exact where the element's data is decoded and its bounding box elsewhere |
| 2023 | yes | ElementId, category and bounding box; walls cut back at their joins and beams at their columns; door and window hosts; room outlines with their numbers and names; Levels with the storey each element sits on; family and type names; layer sets and materials |
| 2026 | yes | experimental: typed elements, storeys, rooms, doors and windows, type names and layer sets, measured on one project against Revit's own export |
| 2027 | yes on Autodesk's sample projects (previews and PartAtom not measured) | experimental: measured against the 2026 copies of the same sample projects |
| 2015 to 2022 | yes (2015 on Autodesk's sample projects, previews and PartAtom not measured) | experimental: measured against the same sample projects saved by a later release, and one 2015 project against Revit 2015's own export |
| 2008 to 2014 | yes, on local files only | experimental and outside the 1.0 range (2015 to 2027); 2011 to 2014 measured on one sample against its 2015 copy, 2008 to 2010 checked only for consistency within each file |
| Families (`.rfa`) | yes | family metadata, OmniClass, and types and type parameters as the `PartAtom` stream lists them |
| Templates (`.rte`, `.rft`) | the same container code; not measured | not measured |

On Revit 2023 to 2027 files, and experimentally on 2015 to 2022 ones, `rvt-elements` and Python also list each
element's saved parameters, named where the file defines them, and
`rvt-ifc --revit-property-sets` writes the named ones in Revit's own property
sets. IFC4 base quantities are written by default; `--no-base-quantities`
leaves them out.

A file of a release read experimentally says so: in the export diagnostics'
`input.release_support`, as a warning, in `rvt-inspect`'s
`experimental_release`, and in the viewer's file status.

## What it exports

One `rvt-ifc` command writes any of:

- IFCX (buildingSMART IFC5 alpha, following the published `ifcx.tsp`)
- IFC4
- ThatOpen Fragments (`.frag`)
- OpenUSD (`.usda`, `.usdc`)

and the other CLIs add glTF 2.0 binary and OBJ (`rvt-gltf`), plan SVG
(`rvt-sheet`), and CSV and JSONL schedules (`rvt-schedule`). `rvt-batch`
converts a folder. The same conversions are available from Python and the
WebAssembly package. `rvt-open-mcp` is a local, read-only MCP server
([`docs/mcp.md`](mcp.md)) that answers an agent's questions from the model
the IFC export writes.

Every format is checked in CI by its official validator: IfcOpenShell for
IFC4, the Khronos glTF-Validator, Pixar's USD validation, buildingSMART's
`ifcx` compose and the schema generated from `ifcx.tsp`, and
`@thatopen/fragments` for Fragments. With `SOURCE_DATE_EPOCH` set, two IFC
exports of the same file are byte-identical
([`tests/reproducible_outputs.rs`](../tests/reproducible_outputs.rs)).

## Main known limits

- rvt-open is not a converter for arbitrary models with Revit-grade fidelity.
- Most parameters are not decoded.
- Family geometry: doors and windows are drawn from the mesh Revit saved for
  their display; other family instances use it only with the experimental
  `--saved-meshes` option, and otherwise their bounding box.
- Not decoded: hip roofs over a T or U footprint, roofs by extrusion outside
  Revit 2025, walls whose layers Revit ends at different heights or cuts at a
  height no join names, the boundaries of the rooms whose stored solid still
  does not close, and opening profiles.
- Each `IfcSpace` says in `RvtElementRecordGeometry.SpaceOutlineSource`
  where its outline came from: the room's stored solid, a saved mesh or a
  family's body, or the record's bounding box.
- Conduits, conduit fittings and gutters export on every release the
  exporter reads, as their record's box; cable trays do not export.
- On 2023 files: no geometry beyond each element's box, wall joins and beam
  cuts; no layered or angled joins, design options or IFC export overrides.
- Extensible Storage ElementIds are not remapped; the default IFC omits
  Extensible Storage edges.
- A stream larger than the resource limits (by default 2 GiB for the file,
  256 MiB for one stream and for one inflated member, 1 GiB inflated per
  partition) is not read. `rvt-ifc` and `rvt-gltf` then name the stream, write
  no model and exit with status 3 unless `--allow-partial`; `--large-model`
  raises the limits.
- A corrupt, encrypted, zero-byte Git LFS placeholder or non-CFB file is
  refused with an error.

Where rvt-open cannot tell something it leaves it out rather than guess: a
body that is a bounding box is a correctly placed, correctly sized box, an
element with an unknown storey sits in the building, and a missing value is
absent, not zero. `rvt-ifc --diagnostics out.json` and the viewer's
diagnostics download list what was exported, approximated and skipped, and
why. `--mode typed-no-geometry`, `geometry` and `strict` refuse a file that
does not meet that bar instead of writing a thinner IFC.

## Releases

Release archives for Linux, macOS and Windows and the WebAssembly package are
published on GitHub Releases with `SHA256SUMS`, a build provenance
attestation, an SBOM and scan results ([`install.md`](install.md)). What
`main` has beyond the latest release is in the fragments under
[`changelog.d/`](../changelog.d/README.md); released changes are in
[`CHANGELOG.md`](../CHANGELOG.md). Open work is tracked as GitHub issues.
