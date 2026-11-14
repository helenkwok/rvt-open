- **An element whose IFC export parameters differ between the partitions of a
  Revit 2024 or later file takes its current copy's value, as Revit's export
  writes it.** The scan took the highest-numbered partition that held the
  key, but the partition the element's stored revision routes to (else the
  latest that holds the element's data, as every other read of a copied record
  does) holds the edited value: the BIM4LCA permit files change a wall's, a
  wall type's or a space's export value after the first save. A current copy that disagrees with itself, or
  that carries no entry for the key, still leaves the key out.
- **A roof that an "Export to IFC As" value writes as another entity than
  `IfcRoof` keeps its saved mesh, as a roof written as `IfcRoof` does.**
  The exporter marks such a roof `RedirectedRoof` and the saved-mesh pass reads
  the mark, so the 8 roofs above keep the bodies they had as `IfcRoof`. The
  mark is on every roof written this way, so the body of 52 roofs in the eight
  BIM4LCA files changes from a sketch outline to the mesh Revit drew: 48
  `IfcSlab` roofs and the 4 `IfcSolarDevice` roofs of the housing files, 50 of
  the 52 equal to Revit's body in all six measures and 2 in five. The viewer
  and the GLB draw these roofs from their mesh and not in layers, as they draw
  a roof written as `IfcRoof`.
