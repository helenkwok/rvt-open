- **Walls report how each end is joined, read from their own record on
  every release from 2018 to 2027, and have an `Axis`.** A wall's record
  lists the joins at its location curve's ends, the walls in each, the join
  type and whether joins are disallowed at an end. The export now writes
  `JoinStartType` and `JoinEndType` (`Abut`, `Miter`, `SquareOff` or
  `Disallowed`) beside `JoinStartWallElementId` and `JoinEndWallElementId`,
  in the element table and in the diagnostics (`decoded.wall_join_types`),
  and takes the partner from the record: 2018 to 2023 walls now have
  partners, a mitred corner names its partner, and an end with joins
  disallowed is no longer reported joined to the wall it touches, nor cut
  back against it. Each wall's `Length` (`Dimensions` set) is its location
  curve's on every release. Every wall drawn as an extrusion gets an `Axis`
  representation on its location line, from end to end of its body along
  it, as Revit's export writes it, and a 2018 to 2023 corner whose two
  record-box bodies the record joins by a mitre is cut on the corner's
  diagonal. On the tiny reference models of 2018 to 2027 every wall's join
  partners, join types, length and axis now match the Revit API answer keys
  and Revit's own IFC export.
