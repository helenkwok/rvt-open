- **Faster exports, same output.** On the Memory workflow's six public
  models `rvt-ifc` takes 33 to 40% less time and `rvt-gltf` 28 to 44% less
  (Snowdon 2024: 132 to 88 s and 101 to 70 s, one run on a GitHub runner),
  and every IFC, IFCX, .frag and .glb is byte-identical to the previous
  build's. Partitions, family documents and the material set are read once
  per file; record graphs, schema objects and quantities are decoded on
  every core, in model order; the IFCX main file is written as it is built.
- **Peak memory is higher on some models.** The `rvt-ifc` peak is higher
  on four of the six Memory workflow models and the `rvt-gltf` peak on
  three. Saved graphics are extracted 256 elements at a time natively too
  (it was 1,024). Two runs, before to after, MiB (`rvt-ifc`; `rvt-gltf`):
  Snowdon 2024 3,444 to 2,559 and 3,441 to 2,552 (26% lower); 3,348 to
  2,111 and 3,352 to 2,286 (32 to 37% lower). `rst_basic` 2024 475 to 381
  and 383 (19 to 20% lower); 463 to 455 and 464 to 459 (1 to 2% lower).
  `rac_basic` 2024 1,207 to 1,528 and 1,211 to 1,431 (18 to 27% higher);
  1,222 to 1,099 and 1,209 to 1,171 (3 to 10% lower). BIM4LCA concrete
  office 2,433 to 2,514 and 2,418 to 2,513 (3 to 4% higher); 2,380 to
  2,512 and 2,390 to 2,511 (5 to 6% higher). BIM4LCA timber housing 2,500
  to 3,431 and 2,508 to 3,431 (37% higher); 2,516 to 3,401 and 2,526 to
  3,399 (35% higher). BIM4LCA concrete housing 2,389 to 3,453 and 2,376 to
  3,453 (45% higher); 2,342 to 3,426 and 2,346 to 3,426 (46% higher). On
  the two housing models the new peak is a short rise while Revit
  properties (`rvt-ifc`) or types and materials (`rvt-gltf`) are read.
  The figures first given here were each run's last stage's peak only.
