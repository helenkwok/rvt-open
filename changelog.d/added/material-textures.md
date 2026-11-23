- **A material can carry an image, and the writers that draw one lay it on its
  faces.** `MaterialInfo::texture` holds a PNG or JPEG file's bytes
  (`MaterialTexture`), and a `TriangulatedFaceSet` per-vertex texture
  coordinates (`uvs`, `v` up). glTF embeds the image and draws it with
  `baseColorTexture` and `TEXCOORD_0`; OBJ writes `vt` and `map_Kd`
  (`model_to_obj_textured`, with the images to write beside the MTL); OpenUSD
  writes `primvars:st` and a `UsdPreviewSurface` reading a `UsdUVTexture`
  (`write_usda_textured`, `write_usdc_textured`, with the images to write under
  `textures/`); IFC4 embeds an `IfcBlobTexture` in the material's surface style
  and maps it with an `IfcIndexedTriangleTextureMap`, a mesh of several textured
  materials written as one face set per material. IFCX (no texture coordinates in
  its mesh schema) and Fragments (colour materials) keep the material's colour,
  as do `model_to_obj`, `write_usda` and `write_usdc`, whose output is
  unchanged. Revit's own render appearances are not read for images yet.
