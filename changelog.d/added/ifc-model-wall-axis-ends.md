- **`IfcModel` gained the public field `wall_axis_ends`**: for a wall whose
  layers end apart at a layered join, the plan points where the plain join
  lies on its centreline, by entity index, which the STEP writer ends the
  wall's `Axis` at (serialised as `wallAxisEnds` in the graph form). Code
  that builds an `IfcModel` field by field must add it (or use
  `..Default::default()`).
