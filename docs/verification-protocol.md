# Verification protocol: cross-witness agreement on the RVT → IFC edge

This document defines what "verified" means for an rvt-open claim about a
Revit model. It covers one edge: a Revit project (`.rvt`) and the IFC file
Revit's own exporter wrote from it.

## Terms

The Revit format has no public specification, so correctness cannot mean
"matches the spec". It means: **agrees with independent witnesses across a
recorded format boundary**, where the witnesses, the artifacts and the
tolerance are recorded, hashed and re-checked by CI.

- A **node** is a file format (RVT, IFC).
- An **edge** is an export path from one node to another, produced by an
  **authoring witness**: here, a licensed Autodesk Revit session exporting to
  IFC.
- A **reading witness** is an independently implemented parser of a node:
  rvt-open for RVT, IfcOpenShell and IFClite (`ifc-lite`) for IFC.
- **Agreement** is a recorded comparison between what independent witnesses
  report about the same model across an edge.

## Where it is recorded

| Term | Definition | Recorded in |
|---|---|---|
| Golden artifact | One file, identified by SHA-256 and byte length, with licence and provenance | `research/witness-registry.json` → `artifacts` |
| Edge | `from` node → `to` node `via` an authoring witness, binding a source artifact to a derived one | `research/witness-registry.json` → `edges` |
| Witness | A reader or author with a stable identity (repository and version policy) | `research/witness-registry.json` → `witnesses` |
| Agreement | A comparison with a named gate (a test or CI script), its claim scope and its status | `research/witness-registry.json` → `agreements` |
| Claim | Any user-facing capability statement | [`support-matrix.json`](support-matrix.json), [`status.md`](status.md) |

`tests/witness_registry.rs` keeps the registry internally consistent and in
step with the project-count manifests under `tests/fixtures/project-counts/`,
which carry the same artifact hashes.

## The protocol

1. **Record one canonical export.** One Revit session produces the IFC file
   from the `.rvt`. Record the authoring witness and its build (Revit release
   and build, exporter settings).
2. **Hash it.** SHA-256 and byte length for every artifact, source and
   derived. The registry and the project-count manifests carry the same
   hashes; the test fails if they drift.
3. **Commit each witness's parse.** Every reading witness's report on the
   artifact (counts, relations, storeys, diagnostics) is committed, not just
   "it opened".
4. **Require agreement before a semantic claim ships.** A capability may move
   to `verified` in `support-matrix.json` only when at least two independent
   witnesses agree across a recorded edge, within the tolerance the agreement
   states, and the comparison runs in CI. One witness alone yields at most
   `partial`.
5. **Gate regressions.** Every agreement names the test or script that
   enforces it. A decoder change that breaks a recorded agreement fails CI.
   The manifest tolerance is the only place slack is allowed, and changing it
   is a reviewed decision with a written reason.

### Evidence tiers

| Tier | Meaning under this protocol |
|---|---|
| E0–E1 | No independent witness; observed in one environment |
| E2 | Several files or releases, still one witness |
| E3 | Independently reproduced on redistributable or owned fixtures |
| E4 | An authoring witness and an independent reading witness agree, gated in CI |
| E5 | Promoted with a support-matrix row |

"Verified" in the support matrix requires E4.

## The recorded RVT → IFC edges

Both edges use `2024_Core_Interior.rvt` (MIT, `magnetar-io/revit-test-datasets`;
the bytes stay in that dataset and are fetched by hash).

| Edge | Authoring witness | Derived artifact | Manifest |
|---|---|---|---|
| Element fixture | Autodesk Revit 2024 | `2024_Core_Interior.ifc` (20 KB) | `tests/fixtures/project-counts/2024-core-interior.json` |
| Full project | Autodesk Revit 24.0.20.20 | `IFC Exports/2024_Core_Interior_slim.ifc` (19879 entities) | `tests/fixtures/project-counts/2024-core-interior-slim.json` |

Each edge is gated by:

- `tests/project_count_fixtures.rs`: rvt-open's decode against the counts in
  the manifest, which are derived from the export;
- `tools/ci/witness-ifcopenshell.py` and `tools/ci/witness-ifc-lite`: two
  unrelated IFC parsers against the same counts;
- `tools/ci/witness-verdict.py`: the verdict across the three witnesses.

The element fixture is a small element export, so most category expectations
there are zero: a real edge, and a weak one. Revit still writes its complete
fifteen-storey spatial hierarchy into it.

The full-project export is the strong edge and the place where the distance
to Revit is measured. It carries 360 `IfcWall`, 132 `IfcDoor`, 6 `IfcWindow`,
256 `IfcColumn`, 116 `IfcSpace`, 80 `IfcSlab`, 20 `IfcShadingDevice` and 15
`IfcBuildingStorey`; rvt-open recovers 360 / 132 / 6 / 256 / 18 / 80 / 20 / 15.
The claimed surface of each verdict is its `semantic_surface` list in
`research/witness/<artifact>/verdict.json`. On the full project it covers the
entity counts of the building element and storey classes, where the
**ElementId sets**, not merely the counts, match the export at tolerance 0;
`relations.IFCRELFILLSELEMENT`, where all 138 `(host wall, filling element)`
pairs match exactly; and `storeys.IFCBUILDINGSTOREY`, where all 15
`(name, elevation)` pairs match exactly.

Two of those fields are stronger than cardinality. The relation field is an
agreement about **topology**: two witnesses can agree on 138 fill
relationships while disagreeing about every wall those openings belong to.
The storey field is an agreement about **labelled position**: two witnesses
can agree on fifteen storeys while putting each at a different height or
under a different name. It is also a surface where the two sides declare
different length units, so each witness resolves its own file's `LENGTHUNIT`
and normalizes the elevation before emitting it.

What the surface excludes is the measured distance between rvt-open and
Revit's own exporter on a real project: spaces, materials and property sets.

A separate check validates rvt-open's own IFC output with IfcOpenShell in CI.
It says the output is valid IFC; it says nothing about decode fidelity, and it
is not an edge.

## The witnesses

| Witness | Node | How it reads |
|---|---|---|
| rvt-open | RVT | `rvt-ifc --observation PATH --artifact-id ID`: splits its own emitted STEP and resolves `IfcProject` → `IfcUnitAssignment` itself |
| IfcOpenShell | IFC | `tools/ci/witness-ifcopenshell.py --observation`: `by_type`, attribute access and `ifcopenshell.util.unit` |
| IFClite | IFC | `tools/ci/witness-ifc-lite --observation`: `EntityScanner`, `parse_entity` and `ifc_lite_core::extract_length_unit_scale` |

All three declare the surface classes `entity_counts`, `relations` and
`storeys`, and each reads the void/fill chain and the storey set with its own
parser.

**Independence.** Two witnesses count as independent only if neither is built
on the other's implementation. IFClite is the crates.io crate
`ifc-lite-core`, pinned exactly, MPL-2.0; it carries its own byte-level STEP
scanner and tokenizer and links no IfcOpenShell code. Comparing a geometry
kernel's results against IfcOpenShell is a comparison, not a lineage.

**Separate processes.** Third-party readers run as separate CI processes and
are never linked into the workspace. `tools/ci/witness-ifc-lite` is its own
workspace root, so no third-party reader enters an Apache-2.0 artifact, and
`tests/witness_registry.rs` fails the build if the Cargo pin, the version the
binary stamps into every observation and the registry entry drift apart.
A witness is never upgraded silently.

**Coverage declaration.** Every adopted reader in the registry carries
`covers`, the list of surface classes it claims. An authoring witness declares
nothing: coverage is a claim about reading, and the Revit exporter emits the
artifact rather than an observation. `tests/witness_registry.rs` enforces
both halves: an adopted reader must declare `covers`, an author must not, and
every committed observation's `semantic_surface_covered` must be a subset of
its witness's declaration, so a witness cannot widen its own claim without a
registry change.

## Observations, verdicts and the CI gate

The observation and verdict shapes are JSON Schema 2020-12 documents:
[`witness-observation.schema.json`](schemas/witness-observation.schema.json)
and [`witness-verdict.schema.json`](schemas/witness-verdict.schema.json). The
committed files under `research/witness/magnetar-2024-core-interior/` and
`research/witness/magnetar-2024-core-interior-slim/` validate against them.

`tools/ci/witness-verdict.py` runs in the `ifcopenshell-validate` CI job. Its
statuses fail closed: `PASS`, `DISAGREE`, `INSUFFICIENT_WITNESSES`,
`INSUFFICIENT_INDEPENDENT_WITNESSES`, `REJECTED_INPUT`, `MANIFEST_ERROR` and
`REPLAY_DRIFT`. Observations and the verdict are published as a build
artifact.

Observation payloads are hashed after canonicalization (sorted keys, no
whitespace, UTF-8). `tests/witness_verdict.rs` recomputes those hashes from
the committed files and asserts the committed verdict is `PASS`, so a decoder
change that alters what rvt-open emits on a golden artifact fails the build
twice: once in replay and once in the verdict.

## Non-claims

- Recording an edge does not make a capability verified; the agreement must
  run in CI within a stated tolerance.
- Agreement between rvt-open's IFC output and IfcOpenShell says the output is
  valid IFC; it says nothing about decode fidelity.
- Nothing here changes any status in `support-matrix.json` by itself.
