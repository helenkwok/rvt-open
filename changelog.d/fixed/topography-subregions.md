- **Topography subregions and graded copies are no terrain elements of
  their own.** A topography surface read from its element record was
  written for every toposurface, so `rac_basic_sample_project` (2015 to
  2027) gained 2 and `rac_advanced_sample_project` 1 box-bodied
  `IfcGeographicElement .TERRAIN.` (subregions or graded copies), which Revit's
  export does not write: a subregion is drawn inside its surface's body and
  a graded copy not at all. Those records are now left out, which brings
  both samples back to their terrain counts before topography records were
  read.
