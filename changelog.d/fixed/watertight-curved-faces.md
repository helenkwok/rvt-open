- **A curved face and the plane beside it draw the edge they share the same
  way, so they meet.** Where a ruled face with a spline profile, or a surface
  of revolution, shares an edge with a plane and the edge crosses both of the
  curved face's parameters, both faces now draw it straight between the
  points the file saves for it, and the plane has a point wherever the curved
  face's strips cross the edge. The faces had parted by the edge's bow
  (0.013 ft on the office with terrain's ruled ramp flight, whose vertices
  lay 0.013 ft from Revit's), and their trimmed strips did not share points:
  a trimmed face is now meshed on points shared between its triangles, each
  strip side holding the grid's `v` values, and a plane beside a trimmed
  face's straight edge takes the same values. A ruled face with curved
  profiles also takes the `v` steps its twist asks for. The ramp flight
  and the timber office's two stair runs now have no directed edge without
  its reverse, as Revit's bodies, and their volumes still read Revit's to
  4 parts in a million.
