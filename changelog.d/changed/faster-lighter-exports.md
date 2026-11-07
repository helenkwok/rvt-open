- **Faster exports, same output.** `rvt-ifc` takes 28 to 38% less time
  and `rvt-gltf` 29 to 40% less on the Memory workflow's six public models
  (Snowdon 2024: 102 to 69 s and 76 to 50 s), and every IFC, IFCX, .frag
  and .glb is byte-identical to the previous build's. Partitions, family
  documents and the material set are read once per file; record graphs,
  schema objects and quantities are decoded on every core, in model order;
  the IFCX main file is written as it is built.
- **Lower peak memory natively.** Saved graphics are extracted 256 elements
  at a time natively too (it was 1,024). Peak memory falls on four of the
  six models (`rvt-ifc` by 17 to 28%, `rvt-gltf` by up to 45%); on `rac_basic`
  2024 the `rvt-ifc` peak is 6% higher (1,150 MiB) and on the BIM4LCA
  concrete housing model the `rvt-gltf` peak is 21% higher (2,196 MiB).
