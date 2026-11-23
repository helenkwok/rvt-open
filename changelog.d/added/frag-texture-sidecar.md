- **Textures beside a `.frag`.** Fragments 3.6 stores a material as one
  colour, so `rvt-ifc --frag out.frag` now also writes `out.frag.textures.json`
  (and the images under `textures/`) when the model has textured materials,
  and removes a stale one when it has none: per textured material its image
  and the size one repeat covers; per sample its texture coordinates, one
  pair per triangle corner in the order the public loader lays out the
  shell's vertices, or a box projection where it has none; and the SHA-256
  of the `.frag` it belongs to. Each textured material keeps a `.frag`
  material of its own, an item's bodies in one textured material are one
  shell, and textured shells are cut below the loader's 65,535-vertex
  buffer, so every sample lines up with one run of vertices. Models without
  textures write the same `.frag` as before. The browser exports give the
  sidecar too. `examples/frag-textures/` draws a `.frag` with its sidecar in
  `@thatopen/fragments` (`docs/frag-textures.md`).
