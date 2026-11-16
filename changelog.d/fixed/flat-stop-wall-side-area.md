- **A wall that stops flat at an omitted target takes its side area from its
  longer side.** Revit's export writes a wall's `GrossSideArea` as the area of
  the largest vertical face of its solid, and `GrossVolume` as the type's width
  times it. rvt-open wrote the body's volume over its thickness, which is the
  same where the layers' ends step alike at both ends, but not where one end
  stops flat at the partner's near face and the other steps: there the two
  sides differ in length. Such a wall now takes its longer side's area times
  its height, and the end is recorded as `JoinStartFlatStop` or
  `JoinEndFlatStop`. On the BIM4LCA concrete housing, walls 1566546 and 1566548
  match Revit's `GrossSideArea` and `GrossVolume` again at permit and as built.
