- **IFCX carries the category, design option, revisions, site and base
  points, and the view, level and wall fields an IFC4 export drops.** Every
  element IFCX holds gets the category and design option its record's
  header saves; revisions, survey and project base points, project and site
  locations and the internal origin are nodes of their own; a view gets its
  name, scale, template, display settings and type, a level its Building
  Story, Computation Height and type, a wall its Room Bounding, Cross-Section,
  Structural and Location Line, a base point its shared position and Angle to
  True North. Parameters Revit's export leaves out (the IFC group, reals of
  other specs, ElementIds that name nothing) are written as saved. Measured
  against the Revit API dumps of the tiny reference models
  (`docs/ifcx-coverage.md`), IFCX now holds 67% and 46% of the element
  parameters (sets 1 and 2; IFC4 43% and 29%), 46% and 41% of the type
  parameters (38% and 28%), 82% and 83% of the level and grid parameters (4%
  and 3%), 68% of the view parameters, 89% of the site and location
  parameters and 92% of the revision parameters (IFC4 none), where it held 8%
  of the view parameters and none of the last two before. The IFC4 file is
  byte for byte the same; the IFCX files of the tiny models are 54% larger.
