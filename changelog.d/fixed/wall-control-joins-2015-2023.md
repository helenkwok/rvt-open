- **A Revit 2023 curtain wall gets its `Length`, and a wall record of Revit 2015
  to 2023 is read with its control joins.** The native wall
  reader took every ElementId in the shape Revit 2024 writes, so a record of
  Revit 2015 to 2023 gave it no control joins, side joins, exposed faces, step
  partners or type. It reads either shape now. On Autodesk's three sample
  projects of Revit 2023 that hold curtain walls (34, 9 and 33 of them) each
  gets `Qto_CurtainWallQuantities.Length`, equal to the same project's 2024
  save's on all of them. One Revit 2021 project (counts only, its export
  through another tool) has 167 of its 390 angled walls' ends within 0.001 ft
  of Revit's, where it had 124. A wall's step ids are still read as before,
  because the wall top cover and the areas they start are not measured before
  Revit 2024. An ordinary wall of any release takes its `Length` only where
  its curve cut back at the reaches its joins give is as long as the curve;
  the side areas, footprint and volumes of 2015 to 2023 walls, and the
  curtain walls of the sample saves before 2023, stay out, as does
  everything that needs the steps.
