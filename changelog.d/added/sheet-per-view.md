- **`rvt-sheet --view` draws one plan view, with its annotation.** It reads
  each plan view's name, level, view range and crop box from the file, and
  draws the elements between the view's bottom clip and cut plane, cut at its
  crop, with its room tags, text notes, grid lines and heads, and linear
  dimensions as labels. `--level <name|mm>` with `--cut-height` draws a
  level's band; `--list-views` prints the plan views. Scored with
  `tools/dwg-oracle/score.sh` against the authoring tool's DWG exports of
  35 plan views of Autodesk samples, linework F1 went from a median of 0.536
  to 0.749 (30 of 35 views better; enlarged plans from 0.03-0.23 to
  0.83-0.90), and text F1 from 0 to a median of 0.043 (mean 0.155, up to
  0.66). Dimension F1 stays 0.
- **`rvt-dwg` reads DXF files larger than 512 MiB.** DXF has its own input
  limit (3 GiB; DWG keeps 512 MiB) and is streamed from disk instead of
  loaded first, so a 750 MB DXF re-saved from a 92 MB DWG reads back.
- **`tools/dwg-oracle/score.sh` takes a layer-map path** as a fifth argument,
  draws each plan row's own view with labels on, and aligns its summary with
  awk, so it no longer fails where `column` is missing.
