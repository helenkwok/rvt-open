- **Render appearance assets as a lossless record.** `rvt::appearance::read`
  reads every material and every render appearance asset of a file: the
  asset's schema (`BaseSchema`), every property with its kind and value as
  saved, child properties, and the connected assets in its texture slots
  (bitmaps with their library path, real-world scale, offset, rotation,
  repeat and tint; bump maps; procedurals such as noise with their
  parameters), plus the swatch image saved with it. Fields not named are
  kept as saved. `rvt-ifc --appearance <PATH>` writes it as JSON, and
  `--ifcx-appearance <PATH>` as an IFCX layer over the `--ifcx` file (one
  node per asset holding the record under `rvtopen::revit::AppearanceAsset`,
  the child `appearance` of each material that names it); other outputs are
  unchanged. Records not read in full are listed, with the reason. Reading
  is bounded: shared or cyclic pointers stop at a marker or a node budget. An asset whose swatch image holds about 100,000 bytes or
  more is read too (it went past a fixed budget of 100,000 values).
