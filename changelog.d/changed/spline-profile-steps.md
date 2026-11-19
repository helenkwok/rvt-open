- **A Hermite spline profile is cut at least as finely as its curvature asks
  for.** The grid of a surface of revolution or a ruled surface with a spline
  profile took a step count from the profile's node count or turning, which
  does not bound how far a chord falls from the curve. It is now never coarser
  than before and takes at least the equal steps (at most 128) that aim at a
  chord error of 0.001 ft, from the spline's greatest second derivative over
  its segments; a spline too curved for 128 steps holds the error only as far
  as they do. On the public samples no body changes from a record box to a
  drawn one, and no scored body moves.
