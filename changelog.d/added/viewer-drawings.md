- **The browser viewer shows a model's sheets and views as drawings.** A
  Drawings list beside the scene tree holds the file's sheets, floor plans,
  ceiling plans and sections and elevations; choosing one draws it in the 2D
  pane as `rvt-sheet --sheet` or `--view` draws it, from the model the viewer
  opened (its phase), with pan and zoom: a sheet's title block, notes and its
  views in viewports, a view's elements, annotation and crop. Each drawing is
  made when it is chosen, not at load. The Export menu saves the drawing shown
  as DXF 2018, DWG 2018 or PDF, and the 3D view and the drawing each keep
  their state while the other is shown. New bindings `drawingList`,
  `drawingOpen` and `drawingSave` (docs/viewer-api.md).
