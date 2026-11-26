- **DXF and DWG layers carry the colour numbers of Revit's default DWG
  export, and can be named by any export layer table.** `rvt-sheet` names
  layers by a table in the format of Revit's `exportlayers-*.txt` files
  (category, subcategory, layer, colour, cut layer, cut colour): a cut line
  takes the cut layer, a subcategory with no row its category's, and a wall
  the row of its type's Function, so foundation and retaining walls go on
  `S-FNDN` and `L-SITE-WALL` as Revit's export puts them. The built-in table
  gives each layer the colour number Revit 2023 and 2024 give it in their
  default (AIA) exports of the public samples: on rac_basic's and Snowdon
  Towers' first-floor plans every layer shared with Revit's export had a
  different colour before, and has Revit's now. `--layer-table FILE` takes
  another table, and on Windows the copy a Revit installation keeps is used
  when found.
- **A section's or elevation's levels are drawn on `A-FLOR-LEVL`**, as
  Revit's DWG exports of rac_basic's North elevation and Snowdon Towers'
  Café Kitchen section draw them, no longer on the grid layers.
- **Room polylines are off by default, as in Revit's default DWG export.**
  `rvt-sheet --room-boundaries` (and `roomBoundaries` in the WebAssembly
  `drawingOpen`, `ExportLayers::room_boundaries` in the library) draws
  rooms, spaces and areas as boundary polylines on `A-AREA-BNDY`, as
  Revit's "Export rooms, spaces and areas as polylines" does.
