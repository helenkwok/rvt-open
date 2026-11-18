- **A plan view's cut edges are drawn in the same order every time.** The edges
  of a body cut by a plan view's range were collected in a hash map and walked
  in its order, which differs from run to run (and between the 32-bit and
  64-bit browser builds, and between the drawing made from the open's kept
  model and the one made from a fresh export), so the same sheet drew its wall
  lines in a different order, and with some of them pointing the other way,
  each time. They are now walked in key order, so the same file draws the same
  SVG, byte for byte, on every run and every build.
