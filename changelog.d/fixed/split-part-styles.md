- **IFC4: each part of a textured mesh split by material carries its
  material's style.** A triangle mesh whose faces draw a textured material
  and another was already written as one face set per material, the
  texture map on the textured triangles only and a colour map on the rest,
  but no part named a style, so a reader that styles items drew every part
  in a default material. Each part now has an `IfcStyledItem` with its
  material's `IfcSurfaceStyle`; IfcOpenShell 0.9.0 draws the parts in their
  own colours, and its schema and rule validation finds no issue.
