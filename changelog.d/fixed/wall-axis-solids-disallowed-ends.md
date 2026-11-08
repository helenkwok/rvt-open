- **Walls drawn from a solid get their `Axis`, a disallowed end is no
  longer cut back on 2024 to 2027, and a join of three or more walls names
  them all.** A wall whose body is written as a solid (a stepped top, a
  mesh, a compound of extrusions) now gets the same `Axis` representation as
  one drawn as a single extrusion: on its location line, from end to end of
  its body along it, as Revit's export writes it. Where a wall's record
  disallows joins at an end, the cut the record-box join solver took there
  is given back, so the body runs to the end of its location line, as
  Revit's does. Where the record's join at an end holds two or more other
  walls, they are written as `JoinStartWallElementIds` /
  `JoinEndWallElementIds`. On the public Revit 2024 and 2025 sample projects
  with Revit's own IFC4 exports, walls with an `Axis` went from 5,554 to
  5,938 of 6,042 and axes within 0.001 ft of Revit's from 2,885 to 3,112;
  wall bodies whose ends are within 0.001 ft of Revit's went from 3,716 to
  3,719. The tiny reference models are unchanged.
