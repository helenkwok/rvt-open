- **Revit 2023's ArcWall pattern no longer writes a curtain wall's panels and
  grid lines as walls.** A match of the pattern is now kept only where its
  native record is a wall (`ArcWall` or `SWall`); on `Revit_IFC5_Einhoven` its
  24 matches were 15 curtain panels, 7 curtain grid lines and 2 records with no
  ElementId.
