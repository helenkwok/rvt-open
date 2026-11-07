- **Faster exports, same output.** On the Memory workflow's six public
  models `rvt-ifc` takes 33 to 40% less time and `rvt-gltf` 28 to 44% less
  (Snowdon 2024: 132 to 88 s and 101 to 70 s, one run on a GitHub runner),
  and every IFC, IFCX, .frag and .glb is byte-identical to the previous
  build's. Partitions, family documents and the material set are read once
  per file; record graphs, schema objects and quantities are decoded on
  every core, in model order; the IFCX main file is written as it is built.
- **Lower peak memory.** Saved graphics are extracted 256 elements at a
  time natively too (it was 1,024). The `rvt-ifc` peak is lower or equal on
  all six models (up to 31% lower); the `rvt-gltf` peak is lower on five
  (up to 43%) and 7% higher on `rst_basic` 2024 (350 MiB). Peaks
  vary between runs: an earlier run had `rac_basic` 2024's `rvt-ifc` peak
  6% higher and the BIM4LCA concrete housing model's `rvt-gltf` peak 21%
  higher.
