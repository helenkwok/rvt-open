- **Topography subregions and graded copies are no terrain elements of
  their own.** A topography surface read from its element record was
  written for every toposurface, so `rac_basic_sample_project` (2015 to
  2027) gained its 2 subregions and `rac_advanced_sample_project` its
  graded copy as box-bodied `IfcGeographicElement .TERRAIN.`, which Revit's
  export does not write: a subregion is drawn inside its surface's body and
  a graded copy not at all. Those records are now left out, which brings
  both samples back to their terrain counts before topography records were
  read.
