- **`rvt-ifc-compare`'s `IFCSPACE` note says how a room's outline is read.**
  It said a room whose stored solid does not close keeps its bounding box.
  Such a room is read again from the solid's faces or edges, then drawn from
  its saved mesh where that is read (Revit 2014 to 2027, by default), and
  only otherwise keeps its box, as `docs/rooms.md` gives.
