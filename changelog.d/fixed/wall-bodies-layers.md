- **Layered walls on Revit 2024 to 2027 end layer by layer at more joins:
  walls of different heights, corners of three or more walls, and T joints
  beside a wall drawn over another.** At an L joint of two layered walls
  with the same layer functions from the outside of the corner, the layers
  now nest into the corner whatever the walls' heights, as at walls of one
  height; a taller wall that runs through a lower one reaches its far face
  with every layer, since above the lower wall nothing cleans its layers
  back. At a corner of three or more walls, a layered wall that stops
  against one layered wall listed before it nests its layers against that
  wall where that wall runs through the corner, and stops square at its
  near face where that wall is itself cut back by a wall listed before it.
  A wall drawn over another along the same line (a second wall on one
  line) no longer hides the join at the other's end, so its T joint or L
  joint is read. Matched by ElementId against Revit's own IFC4 export at
  0.001 ft, over 6,631 walls of the 2024 to 2027 tiny reference models, the
  R2025 set, Core Interior, RE1, the BIM4LCA models, the public 2024 sample
  projects and Snowdon Towers (2024): 4,848 walls have both body ends
  within 0.001 ft, from 4,683; 173 go from wrong to right and 8 from right
  to wrong. Ends at joins of three or more layered walls: 849 match, from
  794. The tiny reference models still match on every wall.
