- **A floor or ceiling whose outline is not what Revit drew is drawn from its
  saved mesh.** A floor drawn as the extrusion of its sketch outline ignored
  what cut it or what it was extended to: on Snowdon Towers (Revit 2024) two
  lawns and a slab were 21%, 16% and 7% over Revit's Area and a deck slab 11%
  under it. Where the mesh Revit saved for a floor or ceiling decodes whole and
  its top covers more than the outline, or less than the outline less every
  opening cut in it, by more than 2%, the mesh is drawn, and
  `ReplacedOutlineArea` keeps the outline's area.
