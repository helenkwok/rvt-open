# DWG oracle

The authoring tool's own DWG export of a public sample's sheets and views is the reference for decoding
sheets, views, viewports, title blocks, annotation, dimensions, line styles and 2D symbols, the way its IFC
export is the reference for 3D. `score.sh` scores what rvt-open writes against those exports with
`rvt-dwg compare` (sheets, linework per layer, text, hatches, dimensions; see `rvt-dwg/src/compare.rs`).

```bash
cargo build --release -p rvt-dwg --bin rvt-dwg && cargo build --release --bin rvt-sheet
RVT_BIN=target/release tools/dwg-oracle/score.sh <exports dir> <samples dir> <out dir> [manifest.tsv] [layer-map.tsv]
```

- The exports folder is read-only: the script refuses an out dir inside it. Put a `manifest.tsv` beside the
  exports (template: `manifest.example.tsv`) or pass its path, and, optionally, a `layer-map.tsv` (export
  layer, rvt-open layer) when the export used a layer standard rvt-open does not write: beside the exports,
  or passed as the fifth argument.
- `plan` rows are drawn with `rvt-sheet --view <the plan view>` (the view named in the manifest's sixth
  column, else taken from the export's file name), labels on, so linework, text and dimensions are all
  scored. A view `*` draws the whole model.
- `sheet` rows are drawn with `rvt-sheet --sheet <number>` (the export's file name up to ` - `) and
  compared on the layout named in the manifest (the export's is `Layout1`): paper space plus model space
  seen through each viewport, both clipped to the viewport frames, so how each side arranges model space
  does not matter. The summary adds the viewport counts (candidate/reference) and how many reference viewports
  have a candidate viewport at their scale within 10 mm of their centre (the largest such distance in
  brackets). An export may draw a view straight into paper space instead of through a viewport.
- Every candidate is written and scored in each format of `FORMATS` (default `dxf dwg`, one summary row
  per format). With `pdf` in `FORMATS` a PDF is written too and checked against the DXF drawing with
  `rvt-dwg pdf-check`. `view` rows record the reference's inventory (tables, SVG of each layout,
  `report.json`) until rvt-open draws sections, elevations and other views.
- Exports of sample files keep the samples' terms: they stay local, are never committed, and write-ups
  give counts only.
- Ask for exports at the internal origin, in millimetres, true colour, paper-space linetype scaling, with
  views on sheets bound (not external references), and record every export setting with the files.
