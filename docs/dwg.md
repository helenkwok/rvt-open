# DWG and DXF

rvt-open reads DWG (R13 to 2018, and R11/12) and DXF (ASCII and binary) and writes DXF (R12 to 2018) and DWG
(2000 to 2018), through the `rvt-dwg` crate. Its engine is acadrust (MPL-2.0), used unmodified (see NOTICE).
Drawings go through one plain 2D model (`rvt_dwg::Drawing2d`): what is read from a file and what rvt-open
draws from a Revit model (`rvt-sheet --format dxf|dwg`) take the same path to every output.

```bash
rvt-dwg plan.dwg                                   # tables (CSV), entities.jsonl, DXF, SVG, report.json
rvt-dwg drawings/ -o out --format csv,svg,report   # every .dwg/.dxf below a folder
rvt-dwg sheet.dwg --all-layouts --format svg       # one SVG per layout
rvt-dwg plan.dwg --format dwg --dwg-version 2004   # re-save as DWG 2004
rvt-dwg compare reference.dwg candidate.dxf --tolerance 5 -o score
rvt-sheet model.rvt --format dwg                   # the whole model's plan as DWG (millimetres, layers by category)
rvt-sheet model.rvt --list-views                   # the plan views: name, level, view range, crop (JSON, feet)
rvt-sheet model.rvt --view "Level 1" --format dxf  # only what that plan view shows, with its annotation
rvt-sheet model.rvt --level "Level 2" --format dxf # a level's band up to --cut-height (default 1200 mm)
```

## Plan views (`rvt-sheet --view`, `--level`)

`--view <name>` draws what a plan view shows: the elements whose height range meets the band from the
view's bottom clip to its cut plane, cut at its crop region when the crop is on, and, as labels, the
annotation the view owns: room tags (the room's name and number), text notes, grid lines with their heads,
and dimensions (an aligned DIMENSION per segment). A name that a file name could not hold may be given
with `_` for the character (`Live_Work` finds `Live/Work`); floor plans win over other plan types of the
same name. `--level <name|mm>` draws a level's band up to `--cut-height` millimetres above it, with no
crop and element names as labels. `--no-labels` leaves every label out. Outlines are still the elements'
plan outlines, not Revit's cut and projection lines, and line styles are not drawn.

Large DXF files read: DXF has its own input limit (`Limits::max_dxf_input_bytes`, 3 GiB; DWG keeps 512 MiB)
and is streamed from disk rather than loaded first.

## Outputs

| File | What |
|---|---|
| `<name>.entities.csv` | One row per entity (columns below). `--explode-blocks` adds a row per entity drawn through each insert, placed where it is drawn, with `insert_path`. |
| `<name>.summary.csv` | Version, units, code page, counts, model-space extents; then counts by type × layer × space (`item`, `type`, `layer`, `space`, `count`, `value`). |
| `<name>.layers.csv` | `name`, `on`, `frozen`, `locked`, `plot`, `colour_aci`, `colour_rgb`, `linetype`, `lineweight`, `entity_count`. |
| `<name>.blocks.csv` | `name`, `base_x/y/z`, `entity_count`, `insert_count`, `is_xref`, `xref_path`, `layout`, `attribute_tags`. |
| `<name>.entities.jsonl` | The Entities table, one JSON object per line, same keys; empty cells left out. |
| `<name>.svg` | Model space, or `--layout`/`--all-layouts` sheets with model space shown through each viewport. Each layer is a `<g class="layer" data-layer>`; elements carry `data-handle`. |
| `<name>.dxf`, `<name>.<version>.dwg` | The file re-saved. When the engine cannot write part of it to the version asked for, the file is written from the drawing model and `report.json` says what was left out. |
| `<name>.report.json` | Input name, size and SHA-256, reader, version, counts, reader notes, what each output left out, entities not drawn in the SVG, outputs, timings. Deterministic apart from `timings_ms`. |

The workbook form (Entities, Summary, Layers, Blocks sheets) takes the same `rvt_dwg::tables::sheets`;
until its writer is in the build, `--format xlsx` is recorded as skipped and the CSV files hold the tables.

Entities columns: `handle`, `type`, `space` (model/paper/block), `layout`, `block` (owner block), `insert_path`,
`layer`, `colour_aci` (256 ByLayer, 0 ByBlock), `colour_rgb` (ByLayer resolved), `linetype`, `lineweight`
(1/100 mm; -1 ByLayer, -2 ByBlock, -3 default), `transparency` (%), `start_x/y/z`, `end_x/y/z`, `centre_x/y`,
`radius`, `length`, `area`, `min_x/y`, `max_x/y`, `text` (formatting codes removed), `insert_block`,
`insert_x/y`, `scale_x/y`, `rotation_deg`, `attributes` (`TAG=value;…`), `measurement` (dimensions; angles in
degrees), `hatch_pattern`, `xdata_apps`.

## Damaged and hostile files

Input size and decompression are capped before decoding; the engine runs inside `catch_unwind`; insert
expansion is bounded by depth and count; a read that skipped records, recovered from errors or came back
empty says so in `report.json` (`notes`). `rvt-dwg` reads each file in a child copy of itself, so a file that
ends the process (allocation failure, stack overflow) costs only that file; `--no-worker` turns that off.

## Not yet

Linetype dashes, hatch patterns (a hatched area is tinted), fonts (one sans-serif face) and twisted viewports
in the SVG; 3D solids, meshes, images, underlays, tables and multileaders are counted, not drawn; xref files
are not opened. Revit sheets, and views other than plans, are not drawn yet. The scorer for the authoring
tool's own DWG exports is `tools/dwg-oracle/`.
