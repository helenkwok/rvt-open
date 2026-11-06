- **Design Option (DESIGN_OPTION_ID) is Revit's value.** It is read from the
  element's `ElementHeader` record, not the Element body's saved
  `m_designOptionId`, which holds -4 on many elements in the main model and
  in options alike. Einhoven's 24 curtain panels and 2 materials now read -1
  as in Revit 2023.1, walls and curtain walls (`SWall`, `ArcWall`) carry it
  too, and Snowdon Towers' elements in an option read that option's
  ElementId. With `--revit-property-sets` an element in an option gets
  Revit's `Identity Data.Design Option` label (the option's ElementId); the
  main model gets none, as in Revit's export.
