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
  where the survey point is at the shared origin. It now stands at the
  internal origin's shared coordinates less the survey point's, as Revit
  2024's own exports place it: `rac_basic_sample_project` at (9.234, -11.277)
  m (it was at (-19.824, -18.808) m), Snowdon Towers Architectural at (0.68,
  0.43, 241.554) m, and `rac_advanced_sample_project`, `DACH_sample_project`
  and the BIM4LCA models unchanged. Where the survey point is not read the
  site stays at the internal origin.
