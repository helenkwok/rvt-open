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
  (`docs/ifcx-coverage.md`): IFCX_SUMMARY_PLACEHOLDER
