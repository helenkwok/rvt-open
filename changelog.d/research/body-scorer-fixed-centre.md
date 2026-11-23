- **The mesh body scorer takes an open body's volume about a point its own
  mesh does not move.** `mesh_bodies_vs_ifc.py` took each body's volume about
  its own vertex centre, which points added on the same surface of an open
  body move, so a mesh that only gained points on its own surface read a
  different volume. Both volumes are now taken about the centre of Revit's
  vertices: Revit's reads as before and a closed body's does not change.
  Measured on one export of `main` of the 27 public models with a Revit export
  (12 BIM4LCA, 15 Autodesk 2024 samples), old scorer against new, 27,064
  bodies: within 1% of Revit's volume 22,687 against 22,852, within 0.01%
  15,941 against 16,171, the distance columns unchanged; 16 class rows move,
  none down.
