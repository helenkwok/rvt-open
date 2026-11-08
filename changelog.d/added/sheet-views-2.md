- **`rvt-sheet --view` leaves out what the view hides, and draws walls cut at
  their openings, on the export's layers.** It reads each plan view's hidden
  categories (its own, or its view template's) and hidden elements, its
  annotation crop (text, tags and dimensions outside it are left out), and
  whether it is placed on a sheet (of several views with one name, that one is
  drawn). A wall the cut plane cuts is drawn broken at each door, window or
  opening the cut plane crosses. Layers follow the authoring tool's default
  (AIA) export layers by category, with walls on `A-WALL` or `I-WALL` by their
  Function. Over 35 plan views of the public sample projects, text F1 went from a median
  of 0.043 to 0.139 (mean 0.155 to 0.180, 7 views better, none worse), and
  wall layers match better (for example `A-WALL` F1 0.93 to 0.97, and `I-WALL`
  0 to 0.83, on two views). Linework F1 is level (median 0.749, mean 0.677 to
  0.681).
