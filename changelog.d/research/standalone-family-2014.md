- **The research form reader opens standalone families of Revit 2014.**
  `native_form_geometry::open` took releases 2015 to 2027 only; it now takes
  2014 to 2027 (`FAMILY_RELEASES`). On the three Revit 2014 families of
  DynamoRevit's test suite every plain group frames and the 12 extrusions
  they hold all rebuild. Revit 2013 stays refused: its schema does not parse
  within the field budget. The project readers keep `NATIVE_RELEASES`.
