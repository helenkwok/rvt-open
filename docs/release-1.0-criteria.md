# 1.0.0 release criteria

Each row states where rvt-open stands now, measured with the Measure workflow and the
tests each row names. A row changes only in the pull request that changes what it measures.

1.0.0 is the first release with a stability commitment
([`RELEASE.md`](../RELEASE.md#versioning)). Its milestone states the goal: a
new user installs rvt-open, opens a model, gets correct elements, geometry and
properties, and can reproduce the measurements. The scope:
1.0 supports every Revit release from 2015 to 2027, and it
ships with no gaps or limitations. Every body is Revit's own shape, every
property Revit's export writes is written, and no release in that range is
experimental. Excluded are the values a project's
own export-time property set file places, which the model does not store
(W7). This page breaks that goal into requirements that can be checked.
1.0.0 is tagged when every requirement has passed.

Each requirement has a bar, the value measured now, a status and its
evidence. A status is one of three values:

- **passed**: measured, and the bar is met.
- **failed**: measured, and the bar is not met.
- **not measured**: no evidence either way, or no bar set yet.

A closed issue or a milestone's completion percentage is not evidence. A bar
marked *proposed* is a draft until it is set. A status changes
only in a pull request that links the new evidence.

## Workflows

| | Requirement | Bar | Measured now | Status |
|---|---|---|---|---|
| W1 | A new user installs it | Every channel (the CLI archives, the WebAssembly package, the viewer, and the Rust and Python builds from source) is checked from what was published, as [`RELEASE.md`](../RELEASE.md#checking-a-release) describes | The CLI archives and the WebAssembly package of v0.1.0 ([`install.md`](install.md)) | **open**; repeated at the 1.0 tag |
| W2 | Every release from 2015 to 2027 is supported | No release in that range is experimental. Each has at least one project with Revit's own IFC export of that release that meets W4, W6 and W7 | 2024 has exact element sets against a same-release export (Core Interior). 2025's same-release evidence is the R2025 sample models (22 with a Revit export, all exact by ElementId, Measure only) and `teste_export_2025` (a Revit 25.4 export, local only); RE1's exports are by Revit 26.2, a cross-release pair. 2023 has 37 of 37 and 37 of 45, the other 8 being window trims nested in families. 2026 has 113 of 115 on one project. 2015 has one: BIMcollab's structural example against Revit 2015's own export, all 241 of its elements by ElementId, in Revit's class or IFC4's for its IFC2X3 one, plus 3 it leaves out. 2018 has one, used up for W5: Vitoria's `ALD26 Before` against Revit's own IFC2X3 export, 1,314 of its 1,321 elements by ElementId (1,136 before conduits, conduit fittings and gutters were recovered), 1,305 of them in Revit's class or IFC4's (9 curtain walls come out as walls), none written that the export leaves out, 7 missing (4 curtain-panel walls and 3 assemblies), so it does not meet W4. 2016, 2017, 2019 to 2022 and 2027 have no same-release export, and are measured only against the same models saved by a neighbouring release; the native readers on 2015 to 2022, and geometry and properties beyond each element's box on 2015 to 2023, 2026 and 2027, are experimental | **failed** |
| W3 | Family and template files are read like projects | A family (`.rfa`) or template (`.rte`, `.rft`) of every supported release yields its types, parameters and geometry, measured against Revit's export of the family placed in a project | Metadata, previews, schema and OmniClass, and a family's types and type parameters as its `PartAtom` stream lists them, Revit's text at save rather than the family's records, and not compared with an export of the family placed in a project ([supported profile](supported-profile.md#by-input)) | **failed** |
| W4 | Correct elements on the models the decoder was developed on | On every reference model, the elements written and their IFC classes equal Revit's export of that model in every category: none missing, none extra | Core Interior (2024): exact in its six instance categories (run in CI). RE1 (2025): Architecture 84, Electrical 49 and Mechanical 74 exact by ElementId; Plumbing writes one element Revit's export leaves out, a water closet tank (442378, pinned in `tests/element_records_2025.rs`). The R2025 sample models: all 22 with an export exact. BIM4LCA office (2024): all 12,974 as built; at building permit one column Revit's export leaves out (1927742), and on both files one fixture in another class (3095244). Snowdon Towers (2024, local only): 5,945 of 5,968 in Revit's export; the other 23 are curtain panels with no resolved type, and the 6 slabs in the Legends phase are no longer written (a Measure run: `IfcSlab` 224 to 218, the only change on any CI model; [evidence page](evidence.md#what-rvt-open-reads-from-real-projects)) | **failed** |
| W5 | Correct on models it was not developed on | At least two projects of every supported release, each with Revit's own IFC export of that release, whose first measurement, made before any fix is made for them, meets W4, W6 and W7. A pair whose first measurement does not is used up for W5: it stays in Measure as a held-out regression check, counts only and never developed on, and its counts may be quoted in reports. Since no model meets W6 and W7 yet, any pair measured now would be used up, so a fresh pair is pinned sealed: no Measure run fetches it until the final 1.0 check, made once when W4, W6 and W7 pass on the reference models. The results are published as measured | Held-out measurements so far: Snowdon Towers Plumbing, HVAC and Electrical (2024). It scored against a VIM of the 2027 edition rather than Revit's IFC export of these files: all 10,385 exported elements in the expected category, storeys and GlobalIds equal, every body a box. The first held-out project with Revit's own export of its release was BIMcollab's Revit 2015 structural example, measured before any fix; later fixes have since used it, so it no longer counts toward this bar, and neither do the holdout pack's models (`research/holdout-models.tsv`), whose first measurements were not recorded against W4, W6 and W7. Since 2026-11-03 Measure also holds out six projects with their release's own export (`research/holdout-archive-models.tsv`), measured once before any fix, counts only: Vitoria's two Revit 2018 saves export no element against Revit's 1,394 and 1,520, so `ALD26 Before` is spent to find why and `ALD26 Project` stays held out; Frigento (2020) has 151 of 151 elements paired by Tag, Gdynia (2021) 199 of 203, Warmond (2021) 710 of 710, and BIM Whale's `AdvancedProject` (2021) 1,698 of 1,714 with none of its 8 spaces, which are Revit Areas (held out, after being measured in full once). All six fail W4 at that first measurement, so they are used up for W5 and stay as regression checks: both Vitoria saves miss every element, Gdynia misses 4 and BIM Whale 16, its 8 Areas among them, and Frigento writes 22 elements and Warmond 6 spaces that Revit's export leaves out. No model meets this bar today, and every release needs fresh pairs. BIMcollab's example fails W4: 3 elements written that Revit's export leaves out, and 16 footings on no storey, whose saved Level is unset. It also fails on identity: the project's, site's, building's and five storeys' GlobalIds are Revit's, from the values the file stores, but none of its 241 elements' GlobalIds are | **failed** |
| W6 | Geometry | Every element's body is Revit's own shape, measured per category against Revit's export of the same model at the tolerance its report states. No body is a bounding box | Below, under [W6](#w6-measured-now) | **failed** |
| W7 | Properties | Every property set, property and quantity Revit's export writes for an element is written, with Revit's value, unit and set, and none Revit's export omits. A value that the export's own user-defined property set file maps into a set is out of scope: the model does not store the file, and the property census counts those values apart per reference model | Below, under [W7](#w7-measured-now) | **failed** |
| W8 | Nothing is approximated without saying so | A supported input gets no approximation. An input 1.0 does not support (Revit 2014 and earlier, a damaged file) is refused or reported as such by the diagnostics, `rvt-inspect` and the viewer | The diagnostics count boxed bodies and unattributed records (`unexported_element_records`). All three say when a file's release is read experimentally. Supported inputs are still approximated: boxed bodies and unread properties (W6, W7). The export diagnostics name a stream over a resource limit, `rvt-ifc` and `rvt-gltf` exit with status 3 instead of writing an empty model, and the viewer offers a retry with the large-model limits; measured by lowering the limits below the reference files' streams, and no model over the default limits has been read with `--large-model` yet | **failed** |
| W9 | The viewer journey | Open, inspect, schedule, export and diagnostics, verified end to end on the reference models | `viewer/tests/journey.spec.ts` runs the whole journey in Chromium on the six redistributable reference models: Einhoven, Core Interior and the four RE1 models (the Deploy viewer workflow). It opens each, inspects an element, and downloads the schedule, IFC, GLB, plan SVG and diagnostics. The inspected element is in the schedule and in the IFC under its ElementId. The schedule lists exactly the elements the diagnostics count by type. On Einhoven and Core Interior, the export has the counts `tests/project_count_fixtures.rs` pins for the CLI's. The local-only reference models ([reference models](reference-models.md)) are not run | **passed** |
| W10 | The measurements reproduce | *Proposed:* every number in the README and the supported profile names its model and says whether anyone can fetch it. Numbers on redistributable models re-run with one documented command | Redistributable models are fetched by `tools/fetch-reference-models.sh` and scored in CI and Measure, and the witness verdicts are checked in CI. Revit's IFC export of Snowdon Towers, the MIT house and the 255ribeiro projects are local only, so the numbers resting on them cannot be re-run by others ([reference models](reference-models.md)). Nothing yet checks that every number names its model | **not measured** |

### W6 measured now

<!-- One sentence or list item per line: a change to one value then touches one line, and pull requests changing different values merge cleanly. -->

Exact where the element's data is decoded, the record's box elsewhere;
the diagnostics count each.
Doors and windows are drawn by default from the mesh Revit saved, where all of it decodes, and on Revit 2024 to 2027 so are the walls that embed a curtain wall where Revit raised their base
(the mesh's lowest vertex is Revit's lowest point on 113 of 113 such walls of the BIM4LCA office);
other loadable families are boxes unless `--saved-meshes` draws theirs, which is Revit's own shape where it is complete (every vertex within 0.001 ft on all 138 of Core Interior's doors and windows and 1,554 of 1,650 bodies scored on the BIM4LCA office as built and 1,638 of 1,738 at building permit, in IfcOpenShell's default tessellation, where a saved mesh is judged; a body both files write as true curves is judged in the fine one) but part of the body elsewhere.
`--family-extrusions` (opt-in) draws an instance from its family's own extrusions where they reproduce its mesh, every arc as an arc whatever its radius: 335 bodies are replaced on Core Interior, RE1 and the BIM4LCA office files (the 0.12 ft rule of replaced 66), 251 of them scored against Revit's export and all 251 holding the bar, 187 judged in the fine tessellation as bodies both files write as true curves and 64 in the default one, 6 of 53 exports moving against `main`'s and none of them held-out.
A face the saved graphics do not decode marks a partial mesh, which keeps its box .
Some partial meshes record nothing, so every class but doors and windows stays opt-in;
all 269 of the office's doors are drawn from their saved meshes, 265 of them exact.
Walls: 360 of 360 with Revit's ends on Core Interior, 510 of 607 on the office as built and 430 of 589 at permit.
Wall tops: a wall whose top join or cut pair names a floor, roof or footing that covers its whole plan is cut at its underside, which is Revit's top on 64 of the office's 65 cut tops as built and 70 of 70 at permit;
a wall such an element covers in part is drawn stepped: 49 walls as built and 46 at permit, and rvt-open's top is Revit's over the whole plan on 583 of 607 walls as built (544 before) and 571 of 589 at permit (530 before). The one wall as built that Revit cuts although 0.0243 ft² of its plan lies outside the floor, inside an abutting wall, keeps its full height, as do the walls whose layers Revit ends at different heights.
Per-category results are in the [supported profile](supported-profile.md#geometry-on-revit-2024-and-2025-projects);
the residuals are open issues, and `tools/re/box_bodies_vs_ifc.py` counts in Measure, per class, the bodies still drawn as their box and how much of it Revit's body fills
Open: family instances other than doors and windows drawn from their saved meshes by default,
and geometry on releases other than 2024 and 2025.

### W7 measured now

<!-- One sentence or list item per line: a change to one value then touches one line, and pull requests changing different values merge cleanly. -->

Names,
types,
storeys,
materials,
layers,
GlobalIds,
`IsExternal`,
curtain mullions' and panels' `IsExternal` by their curtain wall's, as exporters from IFC_v24.2.20 on give it,
the exterior members' checked against the source of Revit's IFC exporter only,
walls' `IsExternal` and the parameters named for a standard property on Revit 2015 to 2023,
curtain mullions' `Span`,
beams' `Span`,
beams' `Slope` 0 where drawn along a level location line,
unrotated beams' `Roll`,
walls' `ExtendToStructure`,
type objects' `FireRating` and `LoadBearing`, with stair types' and flight types' dimensions
(no `IsExternal` on a type object: the office's and the R2025 models' exporters write it, but Revit's newer exporters, those of the held-out models and of RE1 Architecture, write none),
walls' `ExtendToStructure` on Revit 2015 to 2023,
structural columns' and framing's `LoadBearing` from their category, and on Revit 2015 to 2023 their `IsExternal`,
railings' `Height`,
level slabs' `PitchAngle`,
pipes' `InvertElevation`,
storeys' `Reference` and `EntranceLevel`,
stairs' `TreadLengthAtInnerSide` and, with `--revit-set-placement`, their `Pset_StairFlightCommon` from their flights,
stair flights' treads,
tread lengths,
nosing and `Reference`,
landings' `Reference` and parameters named for a standard property set are read.
On the BIM4LCA office the export's own property set file places 2,013 values as built and 1,938 at permit that rvt-open does not write as Revit does (glazing on walls,
curtain walls and coverings,
rooms' `OccupancyType` and the value type of their `OccupancyNumber`,
the `FI_` sets), out of scope;
no other reference model has one.
Quantities are measured by `tools/re/qtos_vs_ifc.py`;
those Revit's export computes from its joined geometry, which the file does not store (walls' side areas and volumes,
and the `NetSurfaceArea` of the mullions whose ends the boxes do not say Revit keeps), stay in W7 and are reached by reproducing Revit's joins: of the 39,388 Revit's export writes on objects rvt-open writes on the office as built, 29,448 are written equal (curtain mullions' `Length`,
`NetSurfaceArea` where the boxes of their wall's mullions say which ends Revit's solid keeps (6,606 of 10,003),
and `NetVolume` where Revit's export draws the mullion as an extrusion of its section,
doors' and windows' `Width`,
`Height` and `Area` where the symbol drawing each and its type agree on its size,
walls' `Width` (on a held-out Revit 2025 model one wall's is counted apart: its export, made hours after the file's last save, gives it a type the saved file does not) and their door openings' `Depth`,
walls' `Length` where their curve cut back at their joins is as long as their curve,
the two lengths Revit's export writes by how it draws a wall (where the two differ, nothing in the model says which, and the census counts the wall's `Length` apart, 461 walls as built and 388 at permit),
walls' `Height` where nothing joins to or cuts them,
and where they are joined to roofs or floors that lie clear of them and get the four quantities below,
walls' `GrossSideArea`,
`NetSideArea`,
`GrossVolume` and `NetVolume` where no step of their record cuts,
embeds or sweeps them and their body is their record's box cut back at their joins,
or they are joined to roofs or floors that lie clear of them and their record confirms their length or their centreline cut back at their joins is as long as their body),
walls' `GrossFootprintArea` where nothing joins to or cuts them,
their body is their record's box and they are more than three times as long as thick,
curtain walls' `Length`,
their curve's,
beams' `Length` where drawn along their location line,
pipe segments' `Length`,
pipes' `GrossCrossSectionArea` and `OuterSurfaceArea`,
stair flights' `Length` and `NetVolume`,
and the `Length` of columns joined to nothing and,
on those drawn as their type's hollow section,
their `CrossSectionArea`,
`OuterSurfaceArea` and `GrossVolume`,
the `CrossSectionArea`,
`OuterSurfaceArea`,
`GrossVolume` and `NetVolume` of columns drawn as their record's box whose type is a plain rectangle,
where no slab,
wall or covering their pairs, clean or cut, name cuts them or crosses an end of them (measured on Revit 2024),
floors' `GrossArea` and,
where a floor's type has one layer as deep as its extrusion,
its `Depth` and `GrossVolume`,
flat roofs written as slabs' `GrossArea`,
`Depth`,
`GrossVolume` and,
where nothing cuts them,
`NetVolume`,
floors' `NetVolume` where their record holds no step that cuts them (what remains of the slabs),
and ceilings' `GrossArea` where they are level and their record holds no step that cuts them (what remains of the coverings),
spaces' `Height` where the room's record holds its solid's faces,
spaces' `NetVolume` where that solid is a prism of its outline (what remains of the spaces),
stair railings' `Length` where the path is one line along the stair's runs,
and the `Length` of each column the rule takes that a clean pair joins, its box's height (186 of 186 as built)), 1 differs (the mullion whose `Span` differs) and 9,990 are missing, none written that Revit's export leaves out, and RE1's 94 IFC2X3 pipe and duct lengths all are written;
Core Interior's and RE1 Architecture's exports were made with Revit's "Export base quantities" off, so the census counts the quantities rvt-open writes on them apart (2,437 and 85,
and RE1 Mechanical's and Plumbing's pipe areas,
12 and 126), and `rvt-ifc --no-base-quantities` leaves them out, base quantities being on by default as in each of Revit's built-in IFC4 setups;
The column cut rule adds, on Revit 2024 files, the round column's `OuterSurfaceArea`, the circumference times its height (one column on each of the two office files, the same column, and the 2 round columns of Snowdon Towers Architectural, whose Revit export holds no base quantities), and at permit the `CrossSectionArea` and `NetVolume` of 17 of the 18 columns a flat slab cuts across their top, `CrossSectionArea` where every edge of the slab inside the section runs parallel to a side of it;
the census scores type objects' own sets too: of the 182 values Revit's export writes on the type objects rvt-open writes on the office as built, 34 are written equal (`FireRating`, `LoadBearing`, stair types' dimensions), and the other 148 are their `IsExternal` (135), which rvt-open writes on no type object because Revit's newer exporters write none, the door types' `FireRating` (6) and `ThermalTransmittance` (1) and the family types' `Manufacturer` (6) (four more,
the mullion types' `IsExternal`,
are counted apart).
Where Revit's export places a property outside the set IFC4's definitions give, W7 is measured with `rvt-ifc --revit-set-placement`, the default output following buildingSMART's definitions;
a value Revit's export writes that is known to be wrong (`WalkingLineOffset`,
curtain mullions' `CrossSectionArea`,
door openings' `Width` and `Height`,
which it swaps,
and,
where it is the floor's own outline read as feet,
a floor's `Perimeter`,
which it writes in feet as though they were the file's unit) is left out and counted apart by both censuses (63,
10,003,
382 and 13 values on the office as built).
So are the curtain mullions' and panels' `IsExternal` in the office's exports and their mullion types', which the exporter that made them (24.0.5.432) sets from its own cache of element types, not from the model: the property census counts them apart, 10,267 values on the office as built and 10,304 at permit.
rvt-open writes the members' by their curtain wall's value, as exporters from IFC_v24.2.20 on give it, and none on the member types.
That differs from the office's on the members the cache turned false (8,579 of 10,003 mullions and 33 of 260 panels as built), and the only model of a newer exporter with curtain members is RE1 Architecture (26.2), whose 12 members, 10 mullions and 2 panels, stand on one interior curtain wall and are false by either rule:
the exterior members' values are checked against Revit's IFC exporter's source only, since no newer exporter's model has an exterior curtain wall.
On the office as built, with `--revit-set-placement`, 289 of the 45,525 property values Revit's export writes on objects rvt-open writes are missing and 1 differs;
without it, 1,386 are missing.
At building permit, 292 of 45,704, and 1,397 without it (1,371 in a run).
rvt-open writes 10 values Revit's export does not as built and 14 at permit. the tags took the `Reference` of 8 and 7 voids written under another line's Tag out of this count: a run, whose census predates the keying by wall, gave 2 and 9 where `main` then gave 10 and 16; the two changes are not measured together in one run:
all but one are on elements Revit's export leaves out (openings, and at permit a column),
and the one, a sanitary terminal's `Pset_SanitaryTerminalTypeCommon.Reference`, follows from fixture 3095244's class, which Revit's export writes as `IfcFlowTerminal`.
rvt-open writes the 30 rectangular wall openings Revit's export holds as built and the 24 at permit, whose values the census scores: 60 and 48 property values and 90 and 72 quantities, all equal, none missing or different. It also writes the 39 fire hose cabinets (38 at permit), 25 of the 39 doors (33 of 43) that cut a second wall and 27 of the 70 curtain walls (13 of 72) that Revit's export gives an opening, each with Revit's box, GlobalId, name, type and `Reference`s, and counts the steps it leaves out by reason (`exported.wall_cut_openings_left_out`). Without the option the census then holds 44,405 equal and 1,386 missing property values of 45,792 as built (44,555 and 1,397 of 45,953 at permit; the missing gain is the door openings' `FireExit`, which rvt-open writes on no door opening), 29,760 equal and 9,990 missing quantities of 39,751 (30,002 and 10,000 of 40,003), and the openings Revit's export writes that rvt-open does not hold 284 property values and 203 quantities as built (316 and 230 at permit), from 595 and 476 (609 and 482) on `main`, which the census now keys by the wall as well as the Tag. The thresholds those rules use (a curtain wall half a foot inside its wall's ends and no other wall's box touching its opening, a door's box within a quarter foot of the second wall, a door's wall twice as long as thick, a height tolerance of 0.05 ft and agreeing record copies to 0.01 ft) were fitted and checked on the office pair alone; the held-out and reference models exercise none of them.
Before this run, with the option, 455 were missing as built and 452 at permit.
The census counts what the export's own property set file places apart, as out of scope: 1,981 values as built and 1,907 at permit, the glazing set on elements that are not doors or windows and the space occupancy set, whose 55 `OccupancyNumber` values, written as `IfcInteger`, were 55 of the 56 that differed;
on the same exports, before that, 2,381 of 47,506 were missing as built with the option and 3,347 without it.
Before the curtain members' `IsExternal` was counted apart, 12,672 of 57,773 were missing with the option and 13,638 without it.
The held-out and BIMcollab runs keep quantity counts only, so no property census of them is published.
Most instance and type parameters are not read.
Open: type objects' values,
the round column's `CrossSectionArea`, `GrossVolume` and `NetVolume`, which are not the circle's, and the one meshed column a cut wall touches,
and the remaining values with no rule yet.

## Interfaces 1.0 stabilizes

| | Requirement | Bar | Measured now | Status |
|---|---|---|---|---|
| S1 | The stable surface is named | `RELEASE.md` names the surfaces 1.0 commits to (the crate, the Python package, the CLIs and their JSON, the output formats, the wasm API) and those it does not | [`RELEASE.md`](../RELEASE.md#what-10-keeps-stable) names all five and what each leaves out. The crate keeps the modules its root's overview lists stable; the rest are under `rvt::research`, behind the `unstable-research` feature, and `rvt::cli` is hidden. CI fails if a stable item hands out a research type (`unnameable_types` on the default-features build) | **passed** |
| S2 | Breaking changes are caught | Each stable surface has a check that fails on an unannounced breaking change | CI's `semver checks` job compares each pull request with its base and fails on a breaking change the title does not mark with `!`: the crate's public API (cargo-semver-checks), the Python type stubs (`tools/ci/python-api-compat.py`; stubtest holds the stubs to the built wheel), the wasm bindings `docs/viewer-api.md` lists (`tools/ci/wasm-api.py`) and the CLIs' JSON schemas (`tools/ci/schema-compat.py`). All twelve CLIs that print JSON have a schema their output is validated against on Linux with the family corpus. The output formats: IfcOpenShell validates the exports, `rvt-gltf` writes a glTF 2.0 container, the schedules begin with the columns the user guide lists, and every writer gives the same bytes twice on the same file (`tests/reproducible_outputs.rs`, `rvt-ifc` with `SOURCE_DATE_EPOCH`) | **passed** |
| S3 | The Rust API is documented with runnable examples | docs.rs shows the user entry points, each with an example CI runs | On `main`, these entry points have an example CI runs as a doctest against `corpus/tier1/` (`cargo test --profile ci --doc`): `RevitFile::open`, `open_with_limits`, `open_bytes` and `open_bytes_with_limits`, `metadata::read_metadata`, `walker::iter_elements`, `RvtDocExporter::export_with_diagnostics`, `write_step`, `model_to_glb`, `elements_csv` and `sheet::render_plan_svg`. Other public items have none yet. `examples/` has three user examples, `basics_inspect`, `basics_elements` and `basics_export`, which CI runs. The docs build from source (`cargo doc`) shows every one of them | **passed** |
| S4 | The Python API is documented with runnable examples | Every Python block in [`python.md`](python.md) runs in CI | The 3 runnable blocks run in CI against `corpus/tier1/` (`tests/python/test_docs_python.py`, in the wheel job); the other 18 show a signature or a result shape and are fenced `text` | **passed** |

## Evidence from outside the project

| | Requirement | Bar | Measured now | Status |
|---|---|---|---|---|
| P1 | Practitioners' reports are part of the supported profile | At least three BIM or AEC practitioners have run the documented workflows on their own models and reported, and each finding is an issue or a line in the supported profile | None yet | **failed** |

## Limitations

None. 1.0 ships with no gaps or limitations, so every row of the [supported profile](supported-profile.md)
has to read as exact before the tag. The work that blocks this is the set of
open issues on geometry and element data.

Out of scope, and so not counted as limitations: Revit 2014 and earlier,
and what [`ROADMAP.md`](../ROADMAP.md#out-of-scope) rules out. That covers
Autodesk SDK internals, a Revit API-compatible surface, writing Revit files,
and cloud worksharing, licensing and linked-model semantics. Also out of
scope: a property value that a project's export-time
user-defined property set file maps into a set. The model stores the
parameter and the export setting that names the file, but not the file, so
which parameter feeds which set on which element is not in the input; the
property census counts those values apart per reference model.

## Keeping this page

A pull request that moves a status updates this page and links the new
evidence. W6's and W7's values are written below the table, one sentence or
list item per line, so that pull requests changing different values do not
conflict. The monthly checklist in [`maintaining.md`](maintaining.md) compares
the page with the 1.0.0 milestone, and the release checklist
([`RELEASE.md`](../RELEASE.md#the-10-gate)) does not tag 1.0.0 while any
requirement is not passed.
