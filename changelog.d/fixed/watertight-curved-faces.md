- **A curved face and the plane beside it draw the edge they share the same
  way, so they meet.** Where a ruled face with a Hermite spline profile, a
  surface of revolution, or a cone shares an edge with a plane across both of
  the curved face's parameters, both faces now draw it straight between the
  points the file saves for it, and the plane has a point wherever the curved
  face's strips cross the edge. A trimmed curved face is meshed on points its
  triangles share, each strip side holding the grid's `v` steps and the
  loops' own points, and a plane beside a straight edge of its trim takes the
  same points. A ruled face with curved profiles takes the `v` steps its
  twist asks for. On the public models with Revit's export, the ramp flight
  of the office with terrain and the timber office's two stair runs have no
  directed edge without its reverse (552 and 1,088 before), as Revit's bodies,
  their volumes still read Revit's to 2 parts in 100,000, and Revit's
  vertices still lie within 0.0009 ft of the ramp flight, whose own
  vertices now lie within 0.0009 ft of Revit's (0.013 ft before).
