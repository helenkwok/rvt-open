- **A plain footprint roof is written as its slabs, as Revit's export
  writes it.** Revit's export writes a gable, hip or shed roof as an
  `IfcRoof` with no body that aggregates one `IfcSlab` `.ROOF.` per
  slope-defining edge, each slab the part of the roof where that edge's plane
  is the top, typed by the `IfcRoofType`, with the roof's sub-element
  GlobalIds 2, 3, ... in the edges' order. rvt-open wrote one `IfcRoof` with
  the whole body. It now writes the same structure wherever it draws a roof
  from its slopes without a rafter cut and all its edges share one slope: on
  the tiny reference hip and gable roofs (fixtures-tiny-1) of 2024 to 2027,
  4 and 2 slabs, every slab's GlobalId and volume as in Revit's export (to
  0.0001 m³); on `R2025_Models.zip`, all 13 roofs Revit splits get its slab
  count and GlobalIds, 11 of them every slab's volume too (the two L-shaped
  hips with a raised edge put three slabs in another order), and the 9 roofs
  Revit keeps whole (rafter cuts, differing slopes, by extrusion) stay
  whole. The schedules list a roof once, and its quantities are its slabs'.
- **Hip roofs on Revit 2026 and 2027 are drawn from their slopes.** Their
  sketch lines' record boxes reach past the lines, so the record-box solve
  left them without an outline, and they were drawn from their saved mesh.
  A roof whose lines all carry slope blocks, one of them defining its slope,
  now takes its lines' exact ends on 2026 and 2027 too.
- **Roofs by extrusion are read on Revit 2024, 2026 and 2027.** Only 2025
  read a roof by extrusion's section. Its straight sketch lines are now read
  on every release their reader is measured on: the curved metal roof of
  `rac_basic_sample_project` saved by 2024, 2026 and 2027 is the 2025 save's
  body, vertex for vertex.
