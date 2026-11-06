- **Lower peak memory in the command-line exports, with identical output.**
  rvt-ifc and rvt-gltf now extract saved graphics 1,024 elements at a
  time, as the browser build already did with 256, and read the saved
  materials once for all batches instead of once per batch. rvt-ifc frees
  the file before building the graph and the model after it; rvt-gltf
  frees the file before writing the glb. Every IFC, IFCX, .frag and .glb
  output is byte-identical to before on the public samples measured.
  `RVT_STAGE_MEMORY=1` prints each stage's time and peak resident memory
  (Linux), and the Memory workflow measures both on the largest public
  samples.
