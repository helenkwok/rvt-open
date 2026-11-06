- **Curtain walls in Revit 2023 and earlier files export as Revit's do.** A
  curtain wall is an `IfcCurtainWall` with no box of its own, aggregating
  its panels and mullions; one without mullions is known by the grid lines
  that name it. A loadable curtain panel's own ArcWall record is no longer
  written as a wall: on Einhoven (2023) the export goes from 26 `IfcWall` (a
  box over every panel) and 9 `IfcPlate` to Revit's 1 `IfcWall`, 1
  `IfcCurtainWall` and 24 `IfcPlate`, each panel its family's open frame.
