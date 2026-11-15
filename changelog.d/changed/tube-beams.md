- **A beam whose type is a rectangular tube is drawn as that tube, with its six
  base quantities.** On Revit 2024, a structural framing type that holds
  the captions `bf`, `d`, `tf`, `tw`, `ri` and `ra` as a hollow rectangle
  (outside 200 by 300 mm, a 4 mm wall, corners rounded 8 mm outside and 4 mm
  inside) is drawn as an extrusion over `IfcArbitraryProfileDefWithVoids`, its
  corners as arcs, where it was a solid box. `CrossSectionArea`,
  `OuterSurfaceArea`, `GrossSurfaceArea`, `NetSurfaceArea`, `GrossVolume` and
  `NetVolume` follow from that profile and the beam's length. The BIM4LCA
  concrete office's 8 tube beams are the one model that holds such a type: on
  its building permit file the six quantities equal Revit's export on all 8 at
  the digits Revit writes (48 values, `tools/re/beam_quantities_vs_ifc.py`),
  and each ring of the profile has Revit's 12 points, lines and arcs
  (`tools/re/beam_tubes_in_ifc.py`). Only Revit 2024 reads it, as no other
  release has a model that holds such a type.
