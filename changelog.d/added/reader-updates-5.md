- **Revit 2015 to 2023 elements have their `Constraints` Level parameters
  and their `Phasing.Phase Created`,** read from each element's own native
  record, held to the same Autodesk samples' 2024 saves.
- **Revit 2015 to 2023 beams are drawn along their location lines**, and
  walls are joined by their join lists, a wall at an angle following its
  line; held to the same samples' 2024 saves, not scored against a Revit
  export.
- **Pipes have their type, cylinder and ports from their own native record on
  Revit 2015 to 2027,** and Revit 2026 and 2027 pipes, ports, door and window
  cuts and the properties of 2025 are read as on 2025.
- **Furniture Systems are written as `IfcSystemFurnitureElement`,** the class
  of Revit's own IFC4 export of the 2018 `rac_basic` (one project, 3
  elements).
- **A door type's `FireRating` and a family type's `Manufacturer` are
  written**, read from the family type table's entries; a type whose copies
  disagree gets neither, and the diagnostics count it.
