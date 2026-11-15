- **A door set in a curtain wall is placed on Revit's axes.**
  Revit's export cuts no opening for such a door, so rvt-open binds it to no wall,
  and it kept the default axes. It now takes its stored X axis, reversed where
  its hand and facing flips differ or its curtain wall is flipped, but not
  both. That is Revit's axes on 269 of 269 doors of the BIM4LCA office's four
  files and RE1 Architecture. The doors set in plain walls are unchanged.
