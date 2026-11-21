- **A railing is not drawn twice where its continuous rail draws the same
  graphics, and a surface of revolution shares its edge points with a ruled
  face beside it.** A railing's top rail or handrail can draw a graphics node
  the railing's own graphics already hold, so that the body held the same
  triangles twice and read up to twice Revit's volume (the road-striping
  railing 2150302 of the Snowdon Towers architectural sample 2.000, the
  railings of the basic architectural sample 1.66 to 1.83); a rail primitive
  that is the same node of the same graph with the same points, triangles and
  placement as one the body already has is now left out once. A surface of
  revolution trimmed to a rectangle and a ruled face across an edge that runs
  along one parameter in both now each take the points the other draws along
  it, where the revolution had kept its own step count beside the ruled face's
  looser one: the trimmed ruled faces beside a revolution that met no vertex
  of it fell from 663 pairs to 0.
