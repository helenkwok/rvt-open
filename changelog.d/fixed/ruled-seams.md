- **Ruled faces that share an edge draw it through the same points, and a ruled
  face with curved profiles is cut into fewer steps.** A ruled face trimmed to a
  rectangle and one that is not (its trim is cut into strips) now take the points
  each draws along the edge they share, so they meet where each used to put its
  own. A face's grid took one step more than the face beside it where a trim end
  read 1.0000000000000029 instead of 1, which parted a ruled face from its
  neighbour along the whole edge; steps are now counted to 1e-9. The twist bound
  that sets how many `v` steps a ruled face with curved profiles takes is 0.007 ft
  (it was 0.001 ft): the timber office's two stair runs are drawn in 1,720
  triangles instead of 6,892 and keep Revit's volume to 0.005%, the ramp flight
  keeps no directed edge without its reverse, and the duct fittings of the two
  mechanical samples are drawn in 996 triangles instead of 5,532.
