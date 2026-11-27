- **Imported and linked CAD drawings are drawn on their own layers, as their import.** A
  drafting view's or sheet's imported DWG draws every line and text with the import
  instance's ElementId (sheet A2 of the Technical School sample: 762 lines and 23 texts, where
  408 lines naming no element were drawn before, so every drawn entity of the sheet names
  its element), on
  the drawing's own layers (`Thin`, `Medium`, `Wide`, `Hidden`, `Text`, coloured as their
  line styles are, where Revit's DWG export of the sheet puts them), with polylines, splines
  and texts now drawn and the blocks the drawing places (its `SlaveImportSymbol`s) drawn in
  place. `rvt::links::import_graphics` gives an import's curves and texts by layer. An
  import turned in its view is placed by its own axes (the stored matrix's columns).
