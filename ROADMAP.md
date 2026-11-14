# Roadmap

Last reviewed: 2026-11-13

This roadmap is the public, contributor-facing view of where rvt-open is headed.
The support boundary lives in [`docs/status.md`](docs/status.md) and the
machine-readable [support matrix](docs/support-matrix.json); the statuses below
are its ceilings and this page never claims more. The plan itself is the GitHub
issues; [`docs/maintaining.md`](docs/maintaining.md) says how the tracker is run.

## Product Goal

rvt-open should become a first-class open-source utility for BIM and AEC users who
need to inspect, validate, and exchange Revit files without installing Revit or
uploading private models to a third-party service.

It is not a production Revit-to-IFC converter yet, and the matrix says so
(`converter-grade-rvt-ifc` is unsupported). It opens every Revit file from 2016
to 2026 and reads its metadata, previews and embedded schema, and Revit 2015
and 2027 files too, measured on Autodesk's sample projects. On Revit 2024 and
2025 project files it reads the building: each element's ElementId, category,
name, type, storey, materials, layers and GlobalId, and a body that is exact
where the element's data is decoded and its bounding box where it is not. It
exports that as IFCX, IFC4, ThatOpen Fragments, OpenUSD, glTF, plan SVG and CSV from the CLIs, Python, the
WebAssembly package and the zero-upload browser viewer. Revit 2023 projects get a narrower version of the
same, and Revit 2015 to 2022, 2026 and 2027 projects are experimental. Every
recovery claim is measured against an oracle: Revit's own
IFC export of the same model wherever one is measured, and for 2015 to 2018,
2019 to 2022 and 2027, which have no measured same-release export (except one
2015 project and one 2018 project), the same
sample projects saved by Revit
2019, 2023 and 2026 (but 2015's `rst_basic`, another model than its 2019 copy).
What is not decoded
(most parameters in IFC, family geometry, openings whose symbol cuts no outline, the room phase rule against a Revit export made with the default setup) is listed in [`docs/status.md`](docs/status.md), and
the four levels a file can reach are in the
[supported profile](docs/supported-profile.md).

## Current Position

| Area | Status | Current state | Next decision point |
|---|---|---|---|
| Container, compression, metadata | Verified | MS-CFB container, Revit's truncated-gzip streams and PartAtom open on every release from 2016 to 2027, and a preview is checked on the 2024 family; 2015 project files open on Autodesk's sample projects, where PartAtom and previews are not measured. | Maintain compatibility and bounds checks against hostile input. |
| `Formats/Latest` schema | Verified | The whole schema is read (4,126 classes on Revit 2024, where 395 were read before). A class's tag is its definition ordinal, and element records name their class by it. | Classify the three fields still unclassified (`ClassDefinitionRef::m_ref`, `VarParam::m_pSubst`, `VarSketchDimObj::m_dimValue`: no instance, or only null pointers), only with byte evidence. |
| ADocument walker | Partial | Document-level metadata for triage; the root ADocument walk is validated on 2024 to 2026. | Expand confidence across project releases and older files. |
| Typed project elements | Partial | Elements come from their partition element records, typed by `BuiltInCategory` and matched against Revit's own exports. On Revit 2024 walls, doors, windows, columns, floor slabs and building pads match element for element (verified); 2025 and 2023 are partial; 2026 is experimental (one local model), and so are 2015 to 2022 and 2027, measured against the same sample projects saved by Revit 2019, 2023 and 2026, but 2015's `rst_basic`, another model. Schema-field walls and opening-index doors and windows stay unsupported. | A Revit export of a 2016 to 2022 or 2027 project to measure against, and a second 2015 one, instance and type parameters in IFC, held-out models. |
| IFC export | Partial | IFC4 with storeys, IFC type objects, material layer sets, materials and stable generated GlobalIds. Bodies are exact where the element's data is decoded and the record's bounding box elsewhere, and the diagnostics count each. | The open geometry issues (below). |
| Browser viewer | Partial | Zero-upload WebAssembly viewer over the same decode; its file status shows how much of a model is a stand-in box. | Keep the journey test, which opens, inspects, schedules, exports and checks the diagnostics of the six redistributable reference models, green as the viewer grows. |
| Python and CLI | Partial | The `rvt-open` wheel (import name `rvt_open`) built from source and prebuilt CLI archives on GitHub Releases, sharing the core's honesty bounds. | Stabilize JSON schemas and the one-shot inspect workflow. |

## Milestones

### Released

- **0.1.0.** The first release of rvt-open: CLI
  archives for Linux, macOS and Windows and the WebAssembly package (among it
  read-and-convert: `convert_all` and one call per format), exporting IFCX,
  IFC4, ThatOpen Fragments, OpenUSD, glTF, plan SVG, 3D PDF and CSV
  schedules. Everything is under `[0.1.0]` in [`CHANGELOG.md`](CHANGELOG.md).

### Geometry and element data

Each slice records a baseline, an improvement and a residual against Revit's
own exports on the reference pack, approximations stay labelled, and a slice
ships in the next release that contains it. Geometry open now: wall bodies
thicker or thinner than their type's layers and the tops of walls whose layers Revit ends
at different heights, stair flights, roofs Revit splits into
slab parts, hip roofs over T and U footprints and roof cases with no oracle,
the openings on releases other than 2024 and 2025, room boundaries whose solid
does not close, beams, MEP equipment and routes and ports and pipes outside
Revit 2024 and 2025, the sketch-to-solid pipeline, and the export time on
Revit 2015 to 2023. Element data: instance and type parameters with units and
precedence and the element-record fields that carry them, written to IFC
property sets and shown in the viewer; door and window sizes where the symbol
cuts no outline, measured on one building's families only; and ports' system classification.

### 1.0.0: first-class utility

A dependable open-source Revit data reader: a new user installs it, opens a
model, gets correct elements, geometry and properties, and can reproduce the
measurements. 1.0 supports every Revit release from 2015 to 2027 and ships with
no gaps or limitations: every body is Revit's own shape, every property
Revit's export writes is written, and no release is experimental. The
requirements, each with its bar, measured value and evidence, are in
[`docs/release-1.0-criteria.md`](docs/release-1.0-criteria.md), which is the
release decision; the milestone's completion percentage is not. They include
held-out models measured before fixes, every release with a Revit
export of its own, the surfaces the stability commitment covers,
APIs documented with examples CI runs, and practitioners' reports.
Breadth (more releases, more categories) never lowers measured
correctness.

## Contribution Priorities

The highest leverage work is:

1. Openly licensed models with a Revit export of the same file, and a
   held-out check on them.
2. Partition-stream readers that turn byte observations into falsifiable decoder
   hypotheses: a test that states the fact it checks and how to verify it
   against the corpus, with measured counts.
3. Fixture assertions that prevent decoders from claiming false positives.
4. Documentation that keeps user-facing support boundaries honest.
5. Viewer diagnostics that explain what the tool could and could not decode.

## Out of Scope

rvt-open will not:

- Use Autodesk proprietary SDK internals, leaked documents, or decompiled
  proprietary implementation code. See [`CLEANROOM.md`](CLEANROOM.md).
- Claim production RVT-to-IFC conversion before real project typed elements and
  geometry are corpus-proven.
- Provide a Revit API-compatible surface.
- Write or edit Revit files: rvt-open reads and exports only.
- Resolve cloud-worksharing, licensing, or external linked-model semantics in
  the near-term product.
