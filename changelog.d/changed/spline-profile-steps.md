- **A Hermite spline profile is cut into the steps its curvature asks for.**
  The grid of a surface of revolution or a ruled surface with a spline profile
  took a step count from the profile's node count or turning, which does not
  bound how far a chord falls from the curve. It now takes the equal steps (at
  most 128) that hold the spline's chord error to 0.001 ft. On the public
  samples no body changes from a record box to a drawn one, and no scored
  body moves.
