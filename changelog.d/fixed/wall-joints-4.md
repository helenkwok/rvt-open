- **More layered wall ends on Revit 2024 to 2027 match Revit's bodies: walls
  whose data copies disagree on their line, L joints whose layer functions
  differ, and T joints whose layers nest.** A wall whose data copies
  disagree on its location line now takes its native record's line, so its
  body is drawn from its centreline and the joins of the walls meeting it are
  read (on the BIM4LCA office, long exterior walls). At an L joint of layered
  walls whose layer functions differ from the outside of the corner (a finish
  1 on one face and a finish 2 on the other, or walls of different layer
  counts), the layers of like function are paired, the cores first, and each
  pair nests; the stopping wall's unpaired outer layers wrap the corner and
  the through wall's stop at the partner's first paired layer. At a T joint,
  a layer outside the core whose face side shows the same functions as the
  other wall's near side nests past that many of its layers. Matched by
  ElementId against Revit's own IFC4 export at 0.001 ft over 6,739 walls:
  5,145 have both body ends within 0.001 ft, from 4,956 (239 go from wrong to
  right, 50 from right to wrong, 24 of those in the advanced architectural
  sample, which cannot be scored at 0.001 ft). The tiny reference models
  still match on every wall.
