- **Views, sheets and placed viewports as a catalog** (research, behind
  `unstable-research`): `rvt::research::views::read` and the `rvt-views` CLI,
  which builds only with that feature, list every view's kind, name, scale,
  plane and 3D camera, every sheet's number and title-block fields, and every
  viewport's sheet, view, offset and box centre. The centre rule was measured
  on one viewport against the Revit API; a turned viewport, an uncropped
  view and a view title's printed position are not read.
