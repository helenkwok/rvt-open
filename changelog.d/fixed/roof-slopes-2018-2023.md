- **Roof slopes are read on Revit 2018 to 2023.** A footprint roof saved by
  2018 to 2023 was drawn from its saved mesh as one body. Its sketch lines'
  slope blocks (a shorter layout than 2024's) and ends are now read, with the
  lines its Sketch lists, so the roof is written as its slabs as on 2024 to
  2027: the tiny reference hip and gable roofs (fixtures-tiny-1) of 2018 to
  2023 get Revit's 4 and 2 slabs, every GlobalId and every volume as in
  Revit's export (24 known gaps closed). A roof whose read slopes do not
  draw it keeps its record box, which its saved mesh replaces, instead of
  standing as a level block of its outline.
- **Butterfly roofs.** A roof whose defining edges slope down into it
  (`rac_advanced_sample_project`'s roof 142430) was not read on any release.
  It is now drawn falling to its valley within its record box and written
  as Revit's two slabs, GlobalIds and volumes as in Revit's 2024 export.
- **Slab order of a raised edge.** A roof's slabs follow its edges by
  sketch-line id with a raised edge after the others, which gives Revit's
  GlobalIds on the L-shaped hips with a 300 mm raised edge of
  `R2025_Models.zip`.
