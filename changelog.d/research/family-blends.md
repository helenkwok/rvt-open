- **The research form reader rebuilds a blend of two straight-edged outlines
  from its records.** `FormDocument::blend` reads a `BlendElem`: its two
  sketches (`m_sketchIds`), their planes and the two offsets (`Double`
  parameters -1001804 and -1001805), and gives each section's vertices in the
  family's coordinates, the solid's exact volume (each corner runs straight
  to its counterpart, so Simpson's rule is exact) and its bounding box. On
  the five blends of two Revit 2020 families (one column family, four in a
  DynamoRevit test family) the volume agrees with the saved mesh to 3e-15 of
  it, the bounding box exactly, and every corner lies on a saved mesh vertex.
  Not read: arcs in a section, holes, sections of different vertex counts or
  not in parallel planes, a swept blend, and loops that start at different
  vertices.
