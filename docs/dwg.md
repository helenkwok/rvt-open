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
rvt-sheet model.rvt --view "Level 1" --format pdf  # the same view as a vector PDF, at the view's scale
rvt-sheet model.rvt --list-sheets                  # the sheets and the views on them (JSON)
rvt-sheet model.rvt --sheet A102 --format dwg      # a sheet: its plan views in viewports, title block, notes
rvt-dwg pdf-check sheet.pdf sheet.dxf --layout Layout1   # does the PDF draw the DXF's linework?
```

## Plan views (`rvt-sheet --view`, `--level`)

`--view <name>` draws what a plan view shows: the elements whose height range meets the band from the
view's bottom clip to its cut plane, cut at its crop region when the crop is on, and, as labels, the
annotation the view owns: room tags (the room's name and number), text notes, grid lines with their heads,
and dimensions (an aligned DIMENSION per segment). A name that a file name could not hold may be given
with `_` for the character (`Live_Work` finds `Live/Work`); floor plans win over other plan types of the
same name. `--level <name|mm>` draws a level's band up to `--cut-height` millimetres above it, with no
crop and element names as labels. `--no-labels` leaves every label out.

What the view hides is left out: the categories its visibility settings (or its view template's) turn off
and the elements hidden in it. Of several plan views with one name, the one placed on a sheet is drawn.
With the annotation crop on, text, tags and dimensions outside it (the crop box grown by its offsets, sheet sizes times the view scale)
are left out. A wall the cut plane cuts is drawn broken where a door, window or opening it hosts crosses
the cut plane. Layers follow the authoring tool's default (AIA) export layers by category (`A-WALL`
or `I-WALL` by the wall's Function, `S-STRS`, `A-GLAZ-CURT`, `G-ANNO-TEXT`, ...), and the IFC type where
the category is not known. An element wholly below the cut plane is drawn as the edges of its body seen
from above (projection lines, with no hidden-line removal yet); a stair or railing the cut plane cuts
draws its part above it as overhead lines on its layer's `-MBND` layer; other elements the cut plane cuts
are drawn as their plan outlines. Each family instance's plan symbol (door swings, window and fixture
lines: the curves its type shows in a plan view) is drawn through the instance's transform, on the
instance's layer. The project's own lines are drawn by what owns them: model lines in the view's range and
the view's detail lines on `A-DETL` (legend views drawn as plans are mostly these), room separation lines on
`A-AREA`, property lines on `C-PROP-LINE` in every plan, and the boundary sketches of floors, toposolids and
building pads on their category's layer. Lines that only shape another element are not drawn: a family's
own lines, the sketches of ceilings, roofs, railings, stairs, walls and openings, sweep paths, and area
boundaries (area plans only). Rooms are drawn as the export's room polylines (`A-AREA-BNDY`). Grids are
drawn to their extents in the view, with a head (a circle and the grid's name, `S-GRID-IDEN`) beyond the
bubble end, and beyond the free end too where the view shows both. A roof seen from above hides the
projection lines of what lies under it. A view whose axes are turned
(a site plan set to true north) is drawn in its own axes about the internal origin. Line styles are not
drawn.

## Sheets (`rvt-sheet --sheet`)

`--sheet <number or name>` draws a sheet the way the authoring tool's DWG export with views merged lays
one out. Model space holds each plan, section and elevation placed on the sheet, drawn as `--view` draws
it and moved along x so that the views do not overlap; paper space (layout `Layout1`, millimetres) holds one
viewport per view, at the view's scale, framing what the view draws and centred so that the view's
internal origin lands where the sheet places it (`Viewport.m_offset`, paper feet: the export's
viewport of rac_basic_sample_project's Level 1 on sheet A102 shows model point (4690.2, -5211.3) mm at
its centre, (227.4, 400.7) mm, which is that offset plus the point at 1:100). Paper space also holds
the title block's lines and fixed labels (the curves and rich texts its family symbol's graphics hold,
placed by the instance), the sheet's own text notes and detail lines, and under each view its title:
the view's name over a line of the title's length. Views of other kinds on the sheet (schedules, 3D
views, legends, drafting views, renderings) are reported on stderr and not drawn yet, nor are the title
block's labels that show the sheet's values (number, name, drawn by, ...).

## Sections and elevations

`--view <name>` of a section or elevation (a `DBViewSection`: its right `m_horzDir`, up `m_vertDir`
and toward-the-viewer `m_viewDir` directions, and its crop box from its `Viewer`) draws, in DXF, DWG or
PDF, the sharp edges of every element's body inside the crop box, its depth included, projected onto
the view's plane: x along the view's right and y up, about the internal origin, millimetres, on the
element's layer. There is no hidden-line removal and no cut pattern yet, and no annotation (level
heads, dimensions, tags).

## PDF

`--format pdf` writes the same drawing as a one-page vector PDF (`rvt_dwg::pdf`): a sheet at full
size with model space drawn through each viewport and clipped to it, a view at its scale. Lines,
arcs (as short segments), polylines, filled hatches and text in one of the PDF standard fonts, coloured
by entity or layer; linetypes, lineweights and fonts are not written. `rvt-dwg pdf-check` maps the
PDF's segments back to drawing units and scores them against the drawing's own linework (precision
and recall within a tolerance), so a PDF that drifts from the DWG and DXF fails (CI checks a sheet and a plan view of a public
sample this way, after `qpdf --check` and a render with poppler).

Dimensions read from a DWG or DXF measure their definition points: the authoring tool's DWG export stores each
dimension's measurement times its dimension style's linear factor (914400 for a 3000 mm distance under a
factor of 304.8), so the stored value is used only when the points give none.

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
are not opened. Legends, drafting, 3D and schedule views are not drawn yet. The scorer for the
authoring tool's own DWG exports is `tools/dwg-oracle/`: plan rows against `--view`, sheet rows
against `--sheet` (paper space with model space seen through each viewport), each in DXF and DWG.
