- **Topography subregions and graded copies are no terrain elements of
  their own.** A topography surface read from its element record was
  written for every toposurface, so `rac_basic_sample_project` (2015 to
  2027) gained 2 and `rac_advanced_sample_project` 1 box-bodied
  `IfcGeographicElement .TERRAIN.` (subregions or graded copies), which Revit's
  export does not write: a subregion is drawn inside its surface's body and
  a graded copy not at all. Those records are now left out, which brings
  both samples back to their terrain counts before topography records were
  read.
- **The IfcSite is placed relative to the survey point.** It stood at the
  shared coordinates of the internal origin, which is Revit's placement only
  where the survey point is at the shared origin (the tiny reference models,
  `rac_advanced_sample_project`, `DACH_sample_project` and the 12 BIM4LCA
  models, all as Revit's own exports place them). Snowdon Towers' export
  puts its site 12 ft higher, by its survey point's -12 ft. Where the survey
  point is not read (Snowdon Towers), the site stays at the internal origin
  instead of 417 km away.
