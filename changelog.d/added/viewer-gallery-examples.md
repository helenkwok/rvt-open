- **The viewer's demo gallery shows what the reader does now**, each card
  with a line saying what it shows: a door family standing in its template's
  sample wall with the opening cut, and eight of this project's tiny reference
  models from Revit 2018 to 2026 (stairs with a landing and railings, a
  curtain wall with see-through glazing, flipped doors and windows in their
  host wall, columns and beams, duct and pipe runs, mirrored, turned and
  in-place family instances, rooms, a hip roof). `?demo=<id>` in the viewer's
  address opens a sample. The family samples are labelled as the geometry
  they draw, not as a scaffold.
- **The viewer exports IFCX, an IFCX appearance layer, Fragments, OpenUSD
  and the appearance assets**, written from the file in the worker as
  `rvt-ifc --ifcx`, `--ifcx-appearance`, `--frag`, `--usdc` and
  `--appearance` write them, and Element info reads a material's render
  appearance on request: the swatch Revit saved with it, its schema and the
  bitmap files its texture slots name. The wasm package gains `appearance_json` and
  `convert_ifcx_appearance`.
