- **Textures beside a `.frag`.** Fragments 3.6 stores a material as one
  colour, so `rvt-ifc --frag out.frag` now also writes `out.frag.textures.json`
  (and the images under `textures/`) when the model has textured materials:
  per textured material its image, the size one repeat covers and the
  source's own placement record verbatim; per sample its texture coordinates,
  one pair per triangle corner in the order the public loader lays out the
  shell's vertices, or a box projection where it has none. Each textured
  material keeps a `.frag` material of its own instead of merging with one
  of the same colour; models without textures write the same `.frag` as
  before and no sidecar. `examples/frag-textures/` draws a `.frag` with its
  sidecar in `@thatopen/fragments` (`docs/frag-textures.md`).
