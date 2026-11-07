- **Footprint roofs slope on Revit 2026 and 2027, and on 2024 where an edge's
  plane is not raised.** A roof sketch line's slope block is the same bytes
  on 2024 to 2027 on the tiny reference hip and gable roofs; 2026 and 2027
  were not read, and 2024 needed −rise/run at +0x3b where these roofs hold
  0.0. The 2024 hip and gable and the 2026 and 2027 gable now rise at 30
  degrees on every defining edge to their 6.2517 m top, as the Revit API
  reports, where they were written flat. The fascia depth and rafter cut stay
  read on 2025 only.
- **Doors and windows have their type's Width and Height on every release.**
  A family type's own slots hold its built-in Width and Height, which no
  definition record names, so they were not read; they are now, from the
  type's slot only. Each door and window of the tiny reference models on
  2018 to 2027 now has the Height its head height is computed from.
- **The `revit` convention writes only the types exported elements use.**
  All 105 type objects of Revit's own 50 IFC4 exports of the tiny reference
  models type an element; rvt-open also wrote every type the file defines
  (170 type objects and 381 property sets on the 2024 hip roof, now 4 and
  49; Revit writes 4 and 33). `--convention open` still writes every type.
