# DWG oracle

The authoring tool's own DWG export of a public sample's sheets and views is the reference for decoding
sheets, views, viewports, title blocks, annotation, dimensions, line styles and 2D symbols, the way its IFC
export is the reference for 3D. `score.sh` scores what rvt-open writes against those exports with
`rvt-dwg compare` (sheets, linework per layer, text, hatches, dimensions; see `rvt-dwg/src/compare.rs`).

```bash
cargo build --release -p rvt-dwg --bin rvt-dwg && cargo build --release --bin rvt-sheet
RVT_BIN=target/release tools/dwg-oracle/score.sh <exports dir> <samples dir> <out dir>
```

- The exports folder is read-only: the script refuses an out dir inside it. Put a `manifest.tsv` beside the
  exports (template: `manifest.example.tsv`) and, optionally, a `layer-map.tsv` (export layer, rvt-open
  layer) when the export used a layer standard rvt-open does not write.
- Today only `plan` rows get a score: the synthetic plan `rvt-sheet` draws is the baseline, so every decoding
  step shows a number moving. `sheet` and `view` rows record the reference's inventory (tables, SVG of each
  layout, `report.json`) until rvt-open writes sheets and views.
- Exports of sample files keep the samples' terms: they stay local, are never committed, and write-ups
  give counts only.
- Ask for exports at the internal origin, in millimetres, true colour, paper-space linetype scaling, with
  views on sheets bound (not external references), and record every export setting with the files.
