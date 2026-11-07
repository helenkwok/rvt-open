- **Dimensions read from DWG and DXF files measure what they show.** A linear,
  aligned or radial dimension's measurement now comes from its definition
  points: the authoring tool's DWG export stores the measurement times its
  dimension style's linear factor (914400 for a 3000 mm grid spacing, under a
  factor of 304.8), which `rvt-dwg` used to report as it was. Over the same 35
  plan views scored with `tools/dwg-oracle/score.sh`, 412 of the exports' 432
  dimensions are now matched (none were), dimension F1 mean 0 to 0.44.
