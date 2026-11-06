# Changelog

All notable changes will be documented here. This project follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and
[semver](https://semver.org/spec/v2.0.0.html). Changes on `main` since the
last release are also in [`changelog.d/`](changelog.d/README.md), as
fragments a release moves into this file.

## [Unreleased]

### Added

- **Where each decoded element was read from, down to the byte, on request.**
  `rvt-ifc --element-provenance` adds a provenance table to the diagnostics:
  for each decoded element, the partition, stream and offset its record was
  read from, and the copy each body, profile or outline reader took its value
  from (value sources for walls, floors drawn along their shape-edited top,
  roofs from their slopes or an extrusion, room outlines, column sections,
  pipe axes, profiles, stair runs, landings, Areas and family instances drawn
  from their families' own extrusions).
- **Why an element, body, opening or outline was left out, one entry per
  element, on request.** `rvt-ifc --export-decisions` adds a decisions log to
  the diagnostics, at most 5,000 entries a reason.
- **The diagnostics of a Revit 2008 to 2023 project count where its element
  records stop** (`decoded.element_record_stages`, and the `skipped` reason
  `element_record_frame_stopped`), so an export that writes nothing says at
  which stage.
- **`rvt-ifc --family-extrusions` draws a family instance from its family's
  own extrusions**, every circle and arc as a true arc, where they reproduce
  the mesh Revit saved (Revit 2024 to 2027 files). It implies `--saved-meshes`.
- **The export phase in Python and the viewer, as `rvt-ifc --phase` has it.**
- **`ifc::run_export`, one function that builds the model `rvt-ifc` writes**,
  which `rvt-ifc`, `rvt-gltf`, the Python package, the viewer and
  `rvt-open-mcp` all call; `rvt::ifc::step_writer::written_names` and
  `type_objects` give the names and type objects the IFC export writes.
- **The MCP server asks for the provenance table and the decisions log**, gains
  `explain_element`, and its tool answers validate against their output
  schemas.

### Changed

- **More Areas of Revit 2015 to 2024 files are drawn**, from area boundary
  lines joined as their records say, and a line off the plan axes refuses only
  the Areas it touches.
- **Exports of Revit 2015 to 2023 files take 0.54 to 0.78 of the time they
  took before**, byte for byte the same.
- **A level beam whose record box is longer than its location line is drawn
  as that box along the line.**

### Fixed

- **The MCP server's element names, type objects, levels and element lists
  are the ones the IFC export writes**, and fields it leaves out when empty are
  optional in its output schema.
- **A floor's void is tagged with the sketch line Revit's export tags it
  with** where the lines run clockwise.
- **A floor whose corners are shape-edited is drawn along its sloped top**, and
  the walls that stand on it start from that top; it keeps its slab
  quantities, and a one-layer wall cut at it its layer colour.
- **Partition streams, and a family's embedded document, are read in the
  order Revit numbers their partitions**, not by name.
- **A room's outline takes off the round columns its stored solid holds.**
- **A plane face split into regions that tile it is drawn whole.**

## [0.1.0] — 2026-11-02

The first release of rvt-open: command-line tools, a Rust library, Python
bindings and a WebAssembly package that read Autodesk Revit project, family
and template files (.rvt, .rfa, .rte, .rft) without Revit and export them to
IFCX, IFC4, ThatOpen Fragments, OpenUSD, glTF, plan SVG and CSV schedules.
CLI archives for Linux (x86_64, arm64), macOS (Apple silicon, Intel) and
Windows (x86_64) and the browser package `rvt-open-wasm-v0.1.0.tar.gz`, each
with `SHA256SUMS` and a build provenance attestation.

### Added

- **A read-and-convert WebAssembly package.** `rvt-wasm` adds
  `convert_ifc`, `convert_ifcx`, `convert_frag`, `convert_usda`,
  `convert_usdc`, `convert_glb`, `schedules_csv`, `elements_json`,
  `diagnostics_json`, `info` and `convert_all` (one open, every output, a
  `progress(step, of, label)` callback), through `rvt::convert`, the pipeline
  `rvt-ifc`, `rvt-elements` and `rvt-info --json` now share.

### Reader and exporter updates

These updates write much more of what Revit's own IFC4 export writes, each value
where the model's records show what Revit writes, and leaves a value out
where Revit's own exporters disagree on it.

Base quantities Revit computes from its joined geometry are written where
the records decide them:
- walls' side areas, volumes and `Height`, and the `GrossFootprintArea` of a
  wall nothing joins to or cuts;
- the areas and volumes of box-drawn columns nothing cuts or crosses;
- floors' `NetVolume` and flat roofs' slab quantities where nothing cuts
  them, and ceilings' `GrossArea`;
- spaces' `NetVolume` where the room's solid is a prism of its outline;
- curtain mullions' `NetSurfaceArea` where their wall's mullions say which
  ends Revit keeps.

A floor's and a covering's `Width` are left out: Revit 2024's exporter writes
them, later ones do not.

These property values are written:
- unrotated beams' `Roll`, level beams' `Slope` and the `Span` of beams drawn
  along their location line (a rotated beam gets no `Roll`, a sloped one no
  `Slope`);
- storeys' `Reference` and `EntranceLevel`;
- structural columns' and framing's `LoadBearing`;
- type objects' common sets, without `IsExternal`, which Revit's newer
  exporters leave out;
- a stair's `TreadLengthAtInnerSide`, and with `--revit-set-placement` its
  flights' `Pset_StairFlightCommon`, measured on 14 and 5 stairs;
- curtain members' `IsExternal` by their curtain wall's value. Exterior
  members' values are checked against the source of Revit's IFC exporter only, since no
  newer exporter's model has an exterior curtain wall.

A pipe's `InvertElevation` is now its height above the model's zero, as
Revit's export writes it.

On the BIM4LCA office as built, counting the objects rvt-open writes:
- 29,262 of the 39,388 base quantities Revit's export writes are equal;
- with `--revit-set-placement`, 45,235 of the 45,525 property values are
  equal;
- 1 differs in each.

What the census counts apart is not in those totals:
- values Revit's export gets wrong (10,390 quantities as built: mullions'
  `CrossSectionArea`, door openings' `Width` and `Height`, floors'
  `Perimeter`);
- openings rvt-open does not write;
- the values the project's own property-set file places;
- the curtain members' values the exporter took from its cache.

No quantity is written that Revit's IFC4 exports leave out. rvt-open writes 10
property values Revit's export does not as built and 14 at permit:
- 9 and 8 on openings and, at permit, 5 on one column, all on elements
  Revit's export does not hold;
- one on a fixture Revit's export writes as another class.

`docs/release-1.0-criteria.md`, W7, has the details.

Native records are read on Revit 2015 to 2022 too: each element's saved
parameters, experimentally, and the phase order, measured against the same
projects saved by Revit 2023. Records that need the extensible-storage
catalogue before Revit 2021, and 2015 to 2017 `VarSketch` records, are
refused. On Revit 2015 to 2023, experimentally, walls' `IsExternal` and
`ExtendToStructure` and structural columns' and framing's `IsExternal` are
written. They are measured against:
- the same projects' later copies;
- modelo_bim's Revit IFC4 export;
- Revit 2015's IFC2X3 export of BIMcollab's example, passed through an
  optimizer and counted apart.

`ExtendToStructure`'s joined value is measured against an export only on
Revit 2024 and 2025.

A model with a partition over 512 MiB no longer loses every value the native
records give. Those readers read under the file's own limits, and name a
partition over the limit. That is not yet measured on a model over
the limits.

Python and the browser viewer draw the doors and windows of Revit 2023 to
2027 files from Revit's saved meshes where the whole mesh decodes, as
`rvt-ifc` already does. A wall's top is cut at the floor that covers its
whole plan. A wall a floor covers only in part keeps its full height where
Revit steps its top: 29 office walls as built and 26 at permit.

Exports of Revit 2015 to 2023 files take longer. The time comes with the
native records now read on those releases, and is not yet broken down by
stage. Timed on one runner against the previous version,
on Autodesk's sample projects:
- 2015 to 2022 projects take 2.1 to 3.9 times as long, 0.9 s to 5.6 s more
  each (2.8 to 3.4 times over each release's samples together);
- 2023 projects take 1.6 to 2.4 times as long;
- peak memory is up to 19% higher.

Revit 2024 to 2027 exports take as long as before in total per release,
within 4%. Single files take 0.93 to 1.15 times as long, and small ones about
0.1 s more.

The public API that nothing used is removed, and other public types change;
"Upgrading" lists each.

#### Upgrading

- Rust code that names a removed public item stops compiling. "Removed"
  below lists each one and what to use instead. `rvt::capability`'s
  relation-domain types and `ifc::NotYetImplemented` are gone.
- `FieldType::ElementIdRef` has no `sub`, and `referenced_tag` no longer keeps
  bit 15.
- Code that builds an `IfcModel` with a struct literal adds
  `storey_properties` and `type_entity_sets` (`Default::default()`), and
  `PropertySet::storey_sets_for` takes the storey properties.
- `rvt-capabilities --json` writes `schema_version` 2, without the
  `relation_domains` section. `rvt-schema --json` writes no `sub` on an
  `ElementIdRef` field. The diagnostics' `parameter_value_count` counts the
  values on type objects too.
- Python's `write_ifc()` and `rvt_to_ifc()` draw doors and windows from their
  saved meshes. Pass `saved_meshes=False` for their boxes, as before.
- The viewer draws them from saved meshes too, with a File status option.
  `?savedMeshes=0` starts it off, and `openRvtBytesWithDiagnosticsModeAndOpenLimits`
  takes `savedMeshes`.
- The native-record readers read partitions under the file's own stream
  limit (no fixed size cap by default), where they had a separate 512 MiB
  cap before.
- A pipe on a raised storey gets a different `InvertElevation`: its height
  above the model's zero, not above its storey.
- An export no longer writes a floor's or a covering's
  `Qto_*BaseQuantities.Width`, which Revit's exporters disagree on.
- Exports of Revit 2015 to 2023 files take 1.6 to 3.9 times as long, 0.9 s
  to 5.6 s more on Autodesk's sample projects, with up to 19% more peak
  memory.

#### Added

- **Unrotated beams' `Pset_BeamCommon.Roll` (W7).** Revit's export
  writes a beam's Cross-Section Rotation, in radians, as its `Roll`. A
  beam's native record holds it in the `m_rotation` of its
  `FamInstCurveDriver`. rvt-open reads it and writes `Roll` 0 for the beams it
  writes an extrusion length for whose rotation is exactly 0: on the BIM4LCA
  office, 8 of 8 beams as built and at building permit, each Revit's value. A
  rotated beam gets none, as no measured beam is rotated.

- **Level beams' `Pset_BeamCommon.Slope` (W7).** For a beam it exports as an
  extrusion, Revit's export takes the slope from a parameter named `Slope`,
  then from the extrusion's own slope, and otherwise as the angle between the
  beam's location line and the horizontal, 0 for a line in the plan. rvt-open
  writes 0 on the beams it writes an extrusion length for whose location line
  stays in the plan: on the BIM4LCA office, 8 of 8 beams as built and at
  building permit, each Revit's value. A sloped beam gets none, as the
  extrusion's slope it would take is not measured.

- **Beams' `Pset_BeamCommon.Span` (W7).** Where it draws a beam as an
  extrusion and no parameter gives the span, Revit's export writes the
  extrusion's length as the beam's `Span`, the length its
  `Qto_BeamBaseQuantities.Length` holds. rvt-open writes it for the beams it
  writes that length for, those drawn along their location line: on
  the BIM4LCA office, 8 of 8 beams as built and at building permit, each
  Revit's value.

- **Ceilings' `GrossArea`.** Revit's export writes a
  compound ceiling's `Qto_CoveringBaseQuantities.GrossArea` as the area of
  the profile its sketch gives, less the cut where Revit cuts it, and a cut
  shows in the ceiling's native record as a `CutPairGStep`. A ceiling drawn
  as one extrusion of its sketch whose record holds none carries that area,
  its holes taken off: 57 of the 90 values Revit's export writes on the
  BIM4LCA office as built and 45 of 75 at building permit, none different.
  The area of a ceiling in pieces (Revit's export gives one piece's), of a
  cut one and of the office's roofing coverings, ventilation units drawn as
  boxes, is not written.

- **A box-drawn column's areas and volumes.** A column
  rvt-open draws as its record's box, whose type is a plain rectangle (its
  length parameters are exactly `b` and `h`, or `b` alone, the box's sides),
  carries the box's `CrossSectionArea`, `OuterSurfaceArea`, `GrossVolume` and
  `NetVolume` when its pair steps are all read and every slab they name lies
  wholly above or below it, with no wall or covering cutting it, whether or
  not a clean pair joins it. On the BIM4LCA office (Revit 2024, the one
  building measured), Revit's export draws a column as a cut mesh, whose
  quantities are not the box's, exactly where a slab its cut pair names lies
  across an end of it; a round column's type holds a `Diameter` and gets
  none. They are Revit's on all 199 such columns of the office as built and
  265 at building permit, none different and none that Revit's export
  leaves out: the office goes from 28,329 to 29,125 of the 39,388 quantities equal
  as built. A joined column's `Length`, the
  round column and the 18 meshed columns stay unwritten.

- **Curtain mullions' and panels' `IsExternal` by their curtain wall's value
  (W7).** Revit's `IsElementExternal` gives a family instance
  with no parameter and no type Function its host's value, which exporters
  from IFC_v24.2.20 on give a curtain wall's members; the exporter that made
  the BIM4LCA office's exports caches types instead and is counted
  apart. rvt-open writes each member's curtain wall's value, and none on the
  member types: 10,262 values on the office as built, 10,299 at permit and
  12 on RE1 Architecture, whose 12 members are equal to its export's (0 of 12
  before). Only those three exports change. The exterior members' values are
  checked against the source of Revit's IFC exporter only: no model of a newer exporter has
  an exterior curtain wall.

- **Floors' `NetVolume`.** Revit's export writes a floor's
  `Qto_SlabBaseQuantities.NetVolume` as the volume of its solid: its
  `GrossArea` times the total thickness of its type's layers, flat or
  sloped, unless something cuts it. A floor whose native record holds
  neither a `CutPairGStep` nor a `WallCutoutGStep` carries that. On the
  BIM4LCA office that is 31 of the 63 `NetVolume` values Revit's export
  writes, as built and at building permit, none different and none that
  Revit's export leaves out. The volumes of the stair landings Revit draws
  as a mesh and of the roofs written as slabs are not written.

- **Curtain mullions' `NetSurfaceArea`.** Revit's export
  sums the faces of a mullion's Revit solid, so an end face is not counted
  where the mullion meets its own continuation end to end, nor where it stops
  inside a crossing mullion, and is counted where it butts against a crossing
  mullion's side face. rvt-open's boxes of a curtain wall's mullions are those
  solids, so it reads which ends are kept from where they meet and writes
  `Qto_MemberBaseQuantities.NetSurfaceArea` as the section's perimeter times
  the length plus that many of the section's areas, on the mullions drawn as
  an extrusion of their section. On the BIM4LCA office that is 6,606
  of 10,003 mullions as built and 6,636 of 10,031 at building permit, each
  Revit's value. Left out are the mullions with an end that meets a mullion
  lying over another, an oblique one, a crossing one that holds the end off
  its axis and faces, one shorter than its section or one left out for
  these reasons, and every mullion on a curved or edited curtain wall.

- **Saved parameters and the phase order are read on Revit 2015 to 2022.** `rvt-elements` and Python's `decoded_elements` list
  each element's saved parameters by name on those releases, experimentally,
  and `rvt-ifc --revit-property-sets` writes them. Against the same project's
  Revit 2023 copy, 932,645 parameters of Autodesk's sample projects read by
  name and value are equal and 243 differ, over 23 projects, each an edit
  between the saves. `native_element::decode_base` took `Identifier` for
  Revit 2024's 8-byte `m_id64` and refused every Element base of 2015 to
  2023, so the project's phase order was not read on Revit 2023 either; it
  takes the 4-byte `m_id` too, and the order reads on all 33 sample projects
  of 2015 to 2023 with no element left out of the last phase, so no export
  changes. A record that needs the extensible-storage catalogue before Revit
  2021, or a 2015 to 2017 `VarSketch` record, is refused, and every other
  native reader (types, stairs, symbol cuts, mullion hosts, ports, saved
  meshes) stays on 2023 to 2027. BIMcollab's 16 footings name no Level, so
  they need another rule.

- **Flat roofs' slab quantities.** Revit's export writes a
  flat roof as a slab, and gives it `Qto_SlabBaseQuantities`: its `GrossArea`
  (its sketch's profile's area), its `Depth` (the layers' total, however
  many there are) and `GrossVolume` (the area times that depth), and its
  `NetVolume` where its record holds no cutting step. rvt-open writes them on
  all 10 of the BIM4LCA office's flat roofs, as built and at building
  permit, none different and none that Revit's export leaves out; the roof a
  hatch cuts keeps Revit's smaller `NetVolume` unwritten, and a roof's
  `Width` (its profile's extent) stays unwritten. A level floor of several
  layers gets no `Depth` or `GrossVolume`: the office's export (Revit
  24.0.5) gives it its last layer's thickness, but Revit 26.4's export of
  the held-out Revit 2026 house writes another value on 2 such slabs, so
  rvt-open leaves it out.

- **Spaces' `NetVolume`.** Revit's export writes a space's
  `Qto_SpaceBaseQuantities.NetVolume` as the volume of the room's closed
  shell. Where the room's stored solid is a prism of its outline, every
  vertical face running from the floor of its record box to the ceiling and
  no face slanted, that volume is the outline's area, its holes taken off,
  times the box's height, and rvt-open writes it. On the BIM4LCA office that
  is 167 of the 278 values Revit's export writes as built and 178 at
  building permit, and both rooms of each of six of the R2025 sample models
  with rooms, none different and none that Revit's export leaves out. The
  volume of a room whose ceiling steps (54 as built), of the mapped
  accessibility volumes and of the rooms whose outline does not close is not
  written; nor is `GrossVolume`, which Revit's export writes on 88 of the
  167 prisms only and which nothing in the file tells from the 79 it leaves
  out.

- **A stair's `TreadLengthAtInnerSide` and flight set (W7).**
  Revit's export writes on each `IfcStair` a `Pset_StairCommon.
  TreadLengthAtInnerSide` and, by the exporter's matching of a set to an entity
  by the start of its name, a `Pset_StairFlightCommon` holding the sums of its
  flights' risers and treads and their common riser height, tread length,
  nosing length and tread lengths at offset and at the inner side. rvt-open
  writes the first always (the stair's tread depth, as a flight's) and the
  second with `rvt-ifc --revit-set-placement`: 111 values on each of the
  BIM4LCA office's two files (14 stairs; the spiral stair's tread count is
  not read) and 8 on each R2025 stair model, 7 on the spiral one, all equal
  to Revit's export's, none different. No other model's export changes. The
  mechanism in the exporter that fills the container's flight set is not read:
  the rule is measured on 14 and 5 stairs of one to three flights.

- **Storeys' `Pset_BuildingStoreyCommon.EntranceLevel` (W7).**
  Revit's IFC exporter's definition of the set gives `EntranceLevel` no calculator, so
  Revit's export reads a yes/no parameter named
  `Pset_BuildingStoreyCommon.EntranceLevel`, then `EntranceLevel`, on the
  Level. rvt-open writes the value the Level's own record holds, where it holds
  one: on the BIM4LCA office, as built and at building permit, 9 storeys (the
  Level_01 yes, 8 more no) and none for Sea Level, which holds no value, each
  Revit's. A file whose Levels declare no such parameter, as the R2025
  models', gets none.

- **Storeys' `Pset_BuildingStoreyCommon.Reference` (W7).** Revit's
  export writes the name of a storey's Level type as its `Reference`. A
  Level's native record names its type in `m_attrId`, and the type's record
  holds the name: `Korkomerkintä` on each of the BIM4LCA office's 10 Levels,
  as built and at building permit, and `Level 1` on the R2025 models'. rvt-open
  writes it on each storey, 20 on the office and 110 on the R2025 models, each
  Revit's value. `IfcModel` gains `storey_properties`, by storey index, and
  `PropertySet::storey_sets_for` takes it.

- **`LoadBearing` on structural columns and framing.**
  Revit's exporter calls a structural column or structural framing element
  load bearing by its category alone. rvt-open now writes that `LoadBearing`
  in its common property set on every release, unless a parameter named for
  it says otherwise, and on Revit 2015 to 2023, experimentally, the
  element's `IsExternal`, not external, as the calculation's last step gives
  it. Against Revit's exports: BIMcollab's Revit 2015 example 91 of 91 beams
  and 15 of 15 columns for each property, and modelo_bim (a Revit 2023 file,
  Revit 26.2's exporter) 8 of 8 beams and columns for `IsExternal` and 9 of 9
  for `LoadBearing` (the ninth on the type objects); on the
  BIM4LCA office as built one column with no such parameter gains Revit's
  value. None different, and no new value that Revit's export leaves out.

- **Type objects' common property sets (W7).** Revit's IFC4
  export gives each type object the common set of its entity, with the
  properties its entries give the type itself. rvt-open now writes them: the
  type's Fire Rating, `LoadBearing` for the structural column and
  framing categories, and a stair type's and flight type's dimensions (a
  stair type's `NosingLength`, `NumberOfRiser` and `NumberOfTreads` at 0, in
  `Pset_StairFlightCommon` too with `--revit-set-placement`). These are equal
  to Revit's export on 34 of the 182 values it writes on the BIM4LCA office's
  type objects as built (29 of 154 at permit), on 34 on the R2025 models and
  on modelo_bim's column and beam types, none different and none that Revit's
  export leaves out. No type object gets `IsExternal`: the office's and the
  R2025 models' exporters write it, Revit's newer ones (the held-out models',
  RE1 Architecture's) do not, and nothing in a file says which exporter will
  read it. `IfcModel` gains `type_entity_sets`, by type and type entity,
  which the export adds to the type object's `HasPropertySets` (code that
  builds an `IfcModel` with a struct literal adds
  `type_entity_sets: Default::default()`), and the diagnostics'
  `parameter_value_count` counts a type object's `Pset_` values. Core
  Interior's `IFCPROPERTYSET` count goes from 6,508 to 6,510.

- **More roof-joined walls' side areas and volumes.** The
  longest vertical face of Revit's solid for a plain wall is as long as the
  centreline cut back where the wall meets the walls joined at its ends,
  which rvt-open reads wherever both ends agree with the record's join reaches.
  A wall joined to roofs, floors or footings that lie clear of it, whose
  body is its record's box, now carries `GrossSideArea`, `NetSideArea`,
  `GrossVolume` and `NetVolume` when that length is its box's long side, not
  only when the record confirms its `Length`. They are Revit's on all 29 such
  walls of the BIM4LCA office as built and 36 at building permit, none
  different and none that Revit's export leaves out: the office goes from
  28,162 to 28,278 of the 39,388 quantities equal as built.

- **A roof-joined wall's side areas and volumes.** A wall
  joined to roofs, floors or footings that rvt-open draws as plain extrusions
  lying wholly above its top or wholly below its base (after the top cut at
  the floor that covers it), with its length confirmed by its record
  (the curve cut back at its joins as long as the curve), and no step in its
  record but those and a clean join, carries the same `GrossSideArea`,
  `NetSideArea`, `GrossVolume` and `NetVolume` as a wall nothing joins to or
  cuts. They are Revit's on all 18 such walls of the BIM4LCA office as built
  and 22 at building permit (2 of each cut at a floor), none different and
  none that Revit's export leaves out: the office goes from 27,948 to 28,020
  of the 39,721 quantities equal as built and from 27,878 to 27,966 at
  permit. A wall a floor passes through, one cut
  by a pair or a cutout, or one joined to a sloped roof (the R2025 models')
  stays unwritten.

- **A wall's side areas and volumes.** A wall whose native
  record holds no step that cuts, embeds or sweeps it and whose body is its
  record's box cut back at its joins, a rectangle, carries
  `Qto_WallBaseQuantities.GrossSideArea`, `NetSideArea`, `GrossVolume` and
  `NetVolume`, as Revit's IFC exporter's `CreateWallBaseQuantities` works them out of
  the wall's solid: the longer side's area, the rectangle times the height,
  and the wall type's width times the side area. They are Revit's on all 49
  of the BIM4LCA office's such walls as built, 48 at building permit and 91
  of the R2025 sample models' walls, none different and none that Revit's export
  leaves out; the office as built goes from 27,752 to 27,948 of the 39,721
  quantities equal, at permit from 27,686 to 27,878 and the R2025 models
  from 411 to 775. Revit writes the four on
  exactly the walls it writes a `Height` on, and a `NetVolume` on the rest;
  the walls with a top join, openings, cut pairs, embedded walls or sweeps
  stay unwritten.

- **Walls' `Pset_WallCommon.ExtendToStructure` on Revit 2015 to 2023,
  experimentally.** Whether a wall's top is joined to a roof
  or floor is read from its native record (a `JoinToRoofGStep` with
  `m_baseOrTop` 1), as on Revit 2024 and 2025. Against the same project
  saved by a later release it is equal on all 1,983 walls both files hold,
  over 24 pairs of Autodesk's sample projects of 2015 to 2022 and their 2023
  copies, with both values present (124 of the 2015 `rac_advanced`'s 180 walls
  are joined). Revit's own exports agree on the 114 walls of BIMcollab's
  Revit 2015 example and the 4 of modelo_bim, which are all not joined; the
  joined value is measured against an export only on 2024 and 2025. A wall's
  height, top cut and side quantities are still read from 2024 on.

- **A wall's `GrossFootprintArea`.** A wall nothing joins
  to or cuts, whose body is its record's box cut back at its joins and that
  is more than three times as long as it is thick, carries `GrossFootprintArea`,
  its length times its width, beside the side areas and volumes it already
  gets. Revit's export writes it on the walls it draws as an extrusion, and
  it is Revit's on all 48 such walls of the BIM4LCA office as built, 47 at
  building permit and 91 of the R2025 models', none different and none that
  Revit's export leaves out (the one wall as long as it is thick, which it
  gives none, is not written): the office goes from 28,162 to 28,210 of the
  39,388 quantities equal as built. Walls joined to roofs
  or floors get one from Revit on 78 of 85 as built and nothing in the
  record tells which, so they stay unwritten.

- **A roof-joined wall's `Height`.** A wall joined to roofs,
  floors or footings that lie clear of it, which gets `GrossSideArea`,
  `NetSideArea`, `GrossVolume` and `NetVolume` from its record's box , now also carries `Qto_WallBaseQuantities.Height`, the body's:
  Revit's export writes it on exactly those walls, and rvt-open's extrusion is
  as tall as Revit's solid on every one. 47 more walls of the BIM4LCA
  office as built and 58 at building permit carry it, Revit's on every
  one, none different and none that Revit's export leaves out: the office
  goes from 28,278 to 28,492 of the 39,388 quantities equal as built.

- **Walls' `IsExternal` and the parameters named for a standard property
  on Revit 2015 to 2023, experimentally.** A wall's type
  Function (`BasicWallType.m_function`) is read from its native records on
  those releases, and the project and shared parameters named `IsExternal`,
  `<set>.IsExternal` or for a property of one of IFC4's sets are read from the
  saved parameters, so a wall's `IsExternal` and those properties are written
  as on 2024 and 2025. The Function is equal to the same project's later
  copy on 1,982 of 1,983 walls over 24 pairs of Autodesk's samples (the other
  is an edit between the saves). Revit's own exports agree on the 114 walls of
  BIMcollab's Revit 2015 example (`IsExternal` and `LoadBearing`, which two
  parameters on its walls give) and the 4 walls of modelo_bim, a Revit 2023
  file (`IsExternal`, from the Function), none different. Doors, windows and
  slabs get none: their Function is not read on these releases.

#### Changed

- **`FieldType::ElementIdRef` is a class reference and nothing more.** A schema field's by-value reference
  is the type word `0e 00 00 00` and a `u16` naming the referenced class,
  six bytes; where its bit 15 is set the class is defined inline after it.
  The variant's `sub` read the two bytes after those six, which belong to
  whatever follows: the next field's name length on about 1,900 fields of a
  2024 to 2026 schema, the pad before an inline definition on about 240,
  and a class's GUID-list count on about 600. `sub` is gone from
  `ElementIdRef` and from `rvt-schema --json`, `referenced_tag` no longer
  keeps bit 15, and every reference to the root `ElementId` class reads as
  `ElementId`, where that used to depend on the next field.
  Code that names `sub` stops
  compiling: drop it. No export changes (every
  reference model, the R2025 models, Autodesk's samples and the family
  corpus); the walker's schema scan finds fewer low-confidence candidates,
  which the default export already left out, so the diagnostics count fewer
  suppressed ones (1,138 to 91 `HostObjAttr` on Autodesk's 2027 MEP sample,
  on 39 files in all).

- **A pipe's `InvertElevation` is the height above the model's zero, and a
  Revit 2024 vertical pipe has one (W7).** Revit's export writes the
  lower end's height in the model's frame, not above the pipe's storey: the
  BIM4LCA office's 12 vertical pipes stand on a storey 55.5 m up and carry
  55.32 m. RE1's pipes are all on the storey at zero, which hid the
  difference. rvt-open no longer takes the storey's elevation off, and a
  vertical pipe's invert, its lower end, no longer waits for an inner
  diameter that Revit 2024 pipes do not have read, so the office's 12 pipes
  as built and at building permit have Revit's value where they had none.

- **The property census counts curtain members' `IsExternal` apart, as the
  exporter's own output (W7).** The exporter that made the BIM4LCA
  office's exports gives a curtain mullion or panel its curtain wall's
  `IsExternal` and then caches each member type as not external, so every
  later wall's members of that type read false: a value the model does not
  hold, which rvt-open does not write.
  `tools/re/psets_vs_ifc.py` no longer counts those values as missing, nor
  the mullion types' own: 10,267 on the office as built (10,003 mullions,
  260 panels, 4 mullion types) and 10,304 at building permit are listed
  apart, with the other values Revit's export gets wrong. It
  counts them only where the export's own type objects hold the value, so
  RE1 Architecture's 12 members, exported by a later exporter, are still
  scored. On the office as built the census goes from 12,672 missing of
  57,773 to 2,405 of 47,506 with `--revit-set-placement`, and from 13,638
  to 3,371 without it; equal (45,045 and 44,079) and different (56) do not
  move, and no export changes.

- **The property census counts what a project's own export-time property set
  file places apart, as out of W7's scope.** `tools/re/psets_vs_ifc.py` now
  lists those values apart instead of as missing: in an IFC4 Reference View export,
  the glazing set on elements that are not doors or windows, and the space
  occupancy set. On the BIM4LCA office as built that is 1,981 values (1,907
  at permit); with `--revit-set-placement` the census's missing values go
  from 2,381 to 455 and its different ones from 56 to 1, and equal is
  unchanged on every model. No export changes.

- **`psets_vs_ifc` scores each opening against the opening it corresponds to
  (W7).** Revit's export writes several openings with one `Tag`, the
  ElementId of the element they belong to: a door drawn as an opening, filled
  by its door, and, in each further wall or door that element cuts, an
  opening with no filling that holds `Pset_OpeningElementCommon.Reference`
  and `FireExit`. Keyed by `Tag` alone, the census scored those values
  against rvt-open's filled opening, or against a curtain wall that shares the
  Tag, and listed them as missing: on the BIM4LCA office as built, 140
  `Reference` values and 31 `FireExit` values that sit on openings rvt-open
  does not write (a curtain wall's 69 cuts, 39 fire hose cabinets' and 31
  door openings' cuts in further walls). An opening is now keyed by whether
  something fills it, and the values on openings rvt-open does not write are
  counted apart, not dropped.

- **Python draws doors and windows from their saved meshes, as `rvt-ifc`
  does.** `rvt-ifc` and `rvt-gltf` draw the doors and windows
  of Revit 2023 to 2027 files from the meshes Revit saved for them, where all
  of a mesh decodes; Python's `write_ifc()` and `rvt_to_ifc()` still drew
  their bounding boxes, so the same file gave a different IFC (found checking
  the previous wheel against the CLI). They now draw the same bodies, and
  `export_diagnostics()`, `export_diagnostics_json()` and
  `rvt_to_ifc_diagnostics()` describe that export. `saved_meshes=False` on
  each keeps the boxes, as `rvt-ifc --no-saved-meshes` does; a
  `RuntimeWarning` says when the saved graphics could not be read.
  `rvt::ifc::saved_meshes::default_scope` gives the default to Rust callers.

- **The quantity census keys an opening by whether something fills it, as
  the property census does.** `tools/re/qtos_vs_ifc.py` keyed every
  opening by its `Tag` alone, so the further cuts Revit's export writes for
  a door or a curtain wall were scored against rvt-open's door opening. Each
  opening now carries the classes of what fills it, so a door's opening is
  still known for the swapped `Width` and `Height` of Revit's export, and the
  quantities on openings rvt-open does not write are counted apart.
  On the BIM4LCA office as built that line lists 527 quantities (some of
  them were hidden before, a later opening of the same `Tag` overwriting an
  earlier one); missing goes from 11,700 to 11,367, the swapped door-opening
  `Width` and `Height` set aside from 191 to 187 each (the 4 others are a
  door's further cuts), and equal and different are unchanged on every
  model. No export changes.

- **The viewer draws doors and windows from their saved meshes, with a
  toggle.** `rvt-ifc` and `rvt-gltf` draw the doors and windows of
  Revit 2023 to 2027 files from the meshes Revit saved for them, where all of a
  mesh decodes; the browser viewer still drew their bounding boxes, so the
  model on screen, its glTF and its IFC differed from the command line's for
  the same file. It now draws the same bodies, and File status has a "Draw
  doors and windows from saved meshes" option, on by default; changing it
  reopens the open file, and `?savedMeshes=0` in the address starts with it
  off. `openRvtBytesWithDiagnosticsModeAndOpenLimits` takes `savedMeshes` for
  it. The cost, measured in Playwright (Chromium on a 4-CPU runner, one open
  each, `tests/saved-meshes-measure.spec.ts`, which Deploy viewer runs and
  uploads): Core Interior (32 MiB, 138 doors and windows, all drawn from
  their meshes) loads in 25.6 s against 24.5 s with boxes, and the wasm
  memory after the load is 324 MiB either way; Einhoven and RE1
  Architecture load 0.1 s and 0.3 s slower with the same memory, and the three
  RE1 models with no doors or windows read nothing extra.

#### Fixed

- **A partition over 512 MiB no longer stops every reader of native records.**
  The readers
  of native records (types, materials, stairs, phases, saved parameters and
  others) kept a stream cap of their own, 512 MiB, that `--large-model` did
  not raise. A model with a larger partition got none of their values, not
  only that partition's, and the diagnostics did not say so. They now read under the file's own
  limits: `--large-model` raises theirs too, and a partition over the limit is
  named in the diagnostics and skipped while the rest of the file is read.
  An element whose current record sits in that partition is left out rather
  than read from an older copy in another partition. At
  the default limits they now stop at 256 MiB, as the rest of the reader
  does, where they read up to 512 MiB before. Not yet measured on a model
  over the limits.

- **A wall's top is cut at the floor that covers it.** A wall
  whose top join or Join Geometry pair names a slab, roof or footing is drawn
  up to that element's underside when the element's outline covers the whole
  of the wall's plan, clear of its holes and openings, as Revit's export draws
  it: on the BIM4LCA office 64 of the 65 tops Revit cuts as built and 70 of 70
  at building permit are now Revit's, no other wall's base or top moves, and
  the exports of Core Interior, RE1 Architecture and the R2025 models are
  byte-identical. A wall that a floor covers in part
  keeps its full height, where Revit steps the top (29 office walls as built
  and 26 at permit), and so does the one wall as built whose plan lies
  0.0243 ft² outside the floor and which Revit cuts all the same. A cut wall
  carries `BodyHeightSource` `wall_top_cut_at_covering_floor` and
  `TopCoverElementId`, the element it is cut at, and no `Height`, which
  Revit's export omits on 2 of them as built and 4 at permit.

#### Removed

- **`rvt-capabilities` no longer lists relation domains, and
  `ifc::NotYetImplemented` is gone.** The
  capability manifest's `relation_domains` section and its
  `relations.typed_domains` row described the typed-relation module that
  an earlier version removed: the row claimed a registry and "SCC/condensation/
  quarantine stubs exist", which was no longer true. The section leaves
  `rvt-capabilities` (text and `--json`) and
  `docs/schemas/capability-manifest.schema.json`, whose `schema_version` is
  now 2 (a consumer that reads version 1 sees the section is gone), and
  `rvt::capability::RelationDomain`, `RelationDomainRegistry`,
  `RelationDomainEntry` and `DomainStatus` leave the API. Nothing replaces
  them: no code read or wrote a relation domain. `ifc::NotYetImplemented`,
  an error no exporter returned, goes with them. Two remaining rows of the
  manifest say in plain words what they meant: nothing reads the compound
  ArcWall variant's openings, and remapping Extensible Storage ElementIds
  needs Revit API output the project does not have.

- **A floor's and a covering's `Width`.** Revit 2024's
  exporter (24.0.5) writes `Qto_SlabBaseQuantities.Width` on a compound
  floor and `Qto_CoveringBaseQuantities.Width` on a ceiling and on a wall
  it exports as a covering; the exporters that wrote two held-out models (a
  Revit 2025 and a Revit 2026 file) write none on them, so rvt-open wrote
  values Revit's export omits there. Where Revit's exporters disagree on a
  value, rvt-open leaves it out, and these are no longer written, including
  the ceilings' `Width` written before: the BIM4LCA office loses 113
  quantities that were equal as built and 89 at building permit (they
  become missing, none different), and the held-out models go from 2 and 3
  values Revit's export omits to none.

- **Public API nothing in the workspace used is removed, as planned before
  1.0.** No exporter, CLI, binding, test,
  example, benchmark or fuzz target called these, so 1.0 doesn't promise
  them. Most were convenience wrappers over fields that stay public:
  - `Floor::is_depressed`, `Door::is_flipped`, `Window::sill_height_inches`,
    `Wall::is_unconnected` and `WallType::width_inches`: read the field
    they wrapped.
  - `WallKind::is_basic` and `WallFunction::to_ifc_predefined`: match on the
    enum.
  - `FloorBoundaryLoop::area_sqft` and `to_curve_loop`: the loop's points
    stay public, so compute the area from them.
  - `Extrusion::t_shape`, `l_shape`, `u_shape`, `rectangle_hollow`,
    `circle_hollow` and `arbitrary_with_voids`: build the `ProfileDef`
    variant directly.
  - `MaterialLayerSet::total_thickness_feet`: sum the layers.
  - `ifc::category_map::is_ifc_member`, `EvidenceTier::meets_verified_gate`,
    and `InflateShortfall::is_complete`.
  - `RevitFile::preview_png_untrimmed`, `preview_raw` and
    `transmission_data_probe`: `preview_png` remains, and
    `transmission_data::probe_transmission_data` is what the last one
    wrapped.
  - `WalkerControl::cancel_token`: keep a clone of the `CancelToken` you
    pass to `with_cancel`.
  - `SchemaTable::tagged_ancestor_map` and
    `ADocumentInstance::elem_table_pointer`.

#### Research

- **Why Revit's export writes `IsExternal` true on 528 curtain members.** The value is not in the model: Revit's IFC exporter's
  `IsElementExternal` gives a curtain mullion or panel its curtain wall's
  value, and the exporter that made the BIM4LCA office's exports (24.0.5.432)
  also caches each member type as not external once the first wall that uses
  it is done, so every later wall's members of that type read false. Replaying
  that cache over the walls in ElementId order is Revit's value on all 10,263
  members and panels of the office as built (498 mullions and 30 panels true)
  and all 10,300 at building permit, none different; the host rule is Revit's
  on 1,651 and 1,691. rvt-open still writes no `IsExternal` on curtain mullions
  and panels: the exporter version is not in the model, and exporters from
  24.2.20 on do not cache types.

- **Reading an ElementId in a 4-byte `Identifier` in every native reader
  moves three Revit 2023 exports no export checks.** Revit
  2015 to 2023 save `Identifier` as a 4-byte `m_id`, 2024 and later as an
  `m_id64`, and every native reader but the Element base and the phase order
  reads only the second. Reading both shapes everywhere leaves 91 of 94
  exports byte-identical on a full measurement run (all 30 samples of 2015
  to 2022, BIMcollab, the held-out models, all 25 samples of 2024 to 2027,
  the R2025 models, the offices, Core Interior and RE1) and moves Einhoven
  and two of Autodesk's 2023 samples: 36 and 30 connector ports, a wall
  layer set and material associations appear that nothing measures on
  Revit 2023. So each reader is
  admitted to 2015 to 2022 on its own measurement, reading the ids it needs
  itself, and the other readers stay silent on 2023.

- **The native record framing holds on Revit 2015 to 2022.**
  On Autodesk's 30 sample projects of those releases and BIMcollab's
  Revit 2015 example, every one of 27,762 record groups frames under the
  layout Revit 2023 uses (a `u32` id, a word, the length, the body and the
  length again), the five global streams parse (the extensible-storage
  catalogue on 22 of 31), and 98.8 to 100 per cent of the declared
  elements' object graphs decode, with the owner id agreeing on every one.
  Before Revit 2018 a group's declared body size counts each record's
  trailing length word, on all 12,005 groups of 2015 to 2017. Against the
  same project's 2023 copy, every ElementId both files hold has the same
  class on 23 pairs, and every saved parameter of 2021 and 2022 has the same
  value. Nothing is admitted yet: `native_element::decode_base` refuses
  `Identifier` as 2015 to 2023 write it (a 4-byte `m_id`), which also keeps
  the phase filter from reading Revit 2023, and the extensible-storage
  catalogue needs `m_specTypeId`, which 2021 introduced.

- **Three BIM4LCA office properties the file does not decide (W7).**
  Columns' `Pset_ColumnCommon.Slope` (73 of 279 as built, 101 of 345 at
  permit, all 0.0) comes from the last geometry object Revit's exporter sees
  for the column, and the file does not fix it: six columns have a native
  record identical in both files and Revit gives them the property only at
  permit, and twelve identical columns on one level split 5 to 7 by whether a
  join is user-set. The building's `Reference` (`Project Information`) is a
  string the file does not hold. The door openings' 140 `Reference` and 31
  `FireExit` values are on the openings Revit writes where a door cuts a
  further wall, which rvt-open does not write. None is written.

- **What Revit's wall areas and volumes are.** In Revit's IFC exporter
  24.1 and 24.2 (the office's exporter) a wall's `GrossSideArea` is the
  largest vertical face's outer boundary and `NetSideArea` the largest face
  with its holes out, the solid's volume is `NetVolume`, and `GrossVolume` is
  the wall type's width times the `GrossSideArea`. Worked out of Revit's own
  bodies they give 435 and 430 of the office's 579 walls as built (grouping
  the faces by normal, as 25.2 does, gives 270 and 276) and all 138 of the
  R2025 models'; `GrossFootprintArea` is `Length` times `Width` on all 279
  walls that carry it. Revit writes the four on exactly the walls it writes
  a `Height` on, and `NetVolume` on every wall, 28 more as built.

- **Revit cuts a wall's top only where the floor covers it.**
  The 5 walls per stage that an earlier measurement found Revit leaves whole are cut over the
  part of their plan the floor covers and left at full height over the rest,
  so the highest point does not move: on the BIM4LCA office the area of
  Revit's body at the wall's full height equals the wall's plan outside the
  floor on 79 of 83 walls as built and 83 of 87 at permit. A rule that cuts a
  top where the floor named by its top join or a cut pair covers the wall's
  whole plan gives 64 of the 65 cut tops as built and 70 of 70 at permit,
  and cuts no wall Revit leaves whole; one wall's 0.085 ft stub past the
  floor's edge is cut by Revit and unexplained, and a part-covered wall
  needs a stepped top, which this export does not draw.
