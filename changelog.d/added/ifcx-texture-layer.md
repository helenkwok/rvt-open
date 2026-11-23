- **IFCX textures as a layer of their own.** `write_ifcx_texture_layer` (and
  `write_ifcx_graph_texture_layer`) writes an IFCX layer to compose over the
  main file (header id `<id>/textures`): each mesh drawn in a textured
  material gets its texture coordinates under `primvars::st` (one `[s, t]` per
  point of its `usd::usdgeom::mesh`, `t` up, as OpenUSD's `primvars:st` with
  vertex interpolation) and a glTF PBR material under `gltf::material`, as
  the IFC5 examples write one, whose `baseColorTexture` is the image's path
  beside the layer (`textures/<hash>.<ext>`; the images come back to be
  written there). Both keys are declared in the layer's `schemas`. The main
  file is unchanged, so a reader that leaves the layer out still draws the
  material's colour.
