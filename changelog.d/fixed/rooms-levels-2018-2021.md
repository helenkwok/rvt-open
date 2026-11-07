- **A project with a single Level now exports it as its storey on every
  release, Revit 2023 and earlier included.** One Level was taken as the
  storey set only on Revit 2024 and later; on older files the storeys came
  from elements' base elevations instead. Einhoven (Revit 2023) wrote four
  empty storeys, `Level 1` and one per row of curtain panels (`Elevation
  6.562 ft`, `Elevation 13.123 ft`, `Roof`), and left its 28 elements outside
  any storey; it now writes one, `Level 1` at 0 ft with the GlobalId of
  Revit's own IFC4 exports, holding the wall, its two windows and the
  curtain wall with its 24 panels, as those exports do. The Level is read
  from its record and name block, and a single Level is still refused
  where any other Level record frames in the file. Of 379 public files
  from Revit 2010 to 2027, three exports change: Einhoven and two Revit
  2016 terrain files of one dataset, which gain their one Level as a
  storey where they had none; the other 376 are byte for byte as before.
- **Revit 2024 to 2027 curtain walls whose one panel is their only part
  are curtain walls, with their type's name.** A curtain wall with no grid
  line and no mullion was written as a wall named `Wall-<id>`: two on the
  `rme_basic` and `rme_advanced` samples of each of 2024 to 2027, now
  `Curtain Wall:Standard` aggregating their panel, 33 of the 33 curtain
  walls of Revit 2024's export of the project, and 14 on the DACH 2024
  sample, which writes 57 of its export's 59 (43 before), none extra. Its
  panel names the wall alone and lies in the wall's record box; a panel
  that is a basic wall beside another wall (Snowdon Towers) does not make
  it a curtain wall. The two DACH curtain walls still missing hold a door
  in place of a panel. Revit 2015 to 2023 are unchanged.
