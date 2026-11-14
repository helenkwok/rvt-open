- **A plane face whose loop's edges meet a hair apart is drawn.** The saved
  mesh of a face was refused when two edges its loop links met more than
  1e-7 ft apart, and the element fell back to its record's box. On the MEP
  advanced sample (Revit 2024 to 2027) floor 576122, a ring round the
  courtyard, has two edges 0.00019 ft apart on its top and bottom faces, so it
  was drawn as its 188.5 by 146.3 ft box, 27,575.6 ft² in plan over the open
  courtyard. On a plane, edges the loop links now meet within 0.0005 ft, and
  the floor is its saved ring, 4,644.6 ft² with its shafts and pipe holes.
