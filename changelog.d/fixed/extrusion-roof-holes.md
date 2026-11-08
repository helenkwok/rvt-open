- **A roof by extrusion is drawn less the holes its skylights cut.** A door
  or window hosted in a roof by extrusion whose symbol cuts a rectangle in
  it now cuts that hole square through the roof's body, as Revit draws and
  exports it. The curved metal roof 243274 of `rac_basic_sample_project`,
  with two skylights, was 1250.97 ft³ (35.42 m³) on 2024 to 2027 against
  1235.10 ft³ (34.97 m³) in Revit's export; it is now Revit's volume to
  0.001 ft³, and the mesh the file saves for it lies on the body to 5e-6 ft.
  A roof with a hosted cut that is not a rectangle along its axis, or not
  square to it, keeps its saved mesh. Other roofs are unchanged.
- **A U-shaped roof sloped by arrows is drawn as gables running into one
  another.** Where `20_T_U_Roof`'s U (roof 5941, Revit 2025) slopes its
  long outer walls along the arms only, each pair of facing sloped edges is
  now a gable between them, running along its ridge until the ridge passes
  under another gable, and the highest gable's top and underside win where
  they overlap. The roof was its outline over its record box, 37,357 ft³
  (1,057.8 m³); it is now Revit's body, 1,845.6 ft³ (52.26 m³), every vertex
  on Revit's surface to 4e-6 ft.
