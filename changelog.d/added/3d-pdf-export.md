- **3D PDF export.** `rvt-gltf --format pdf3d` (default output
  `<input>.pdf`), the library's `rvt::ifc::pdf3d::model_to_pdf3d`, the wasm
  binding `modelToPdf3d` and the viewer's "Export 3D PDF" button write a
  one-page PDF whose 3D annotation holds the glTF export's meshes as U3D
  (ECMA-363, 4th edition): one model node per element, named with its
  ElementId and carrying ElementId, IFC class and category as node metadata,
  instances sharing one mesh, each face in its material. The page opens on an
  isometric view with CAD lighting, and a flat-shaded poster of the same view
  is drawn by viewers that show no 3D (browsers, poppler). The viewer builds
  the file in the tab and sends nothing. The U3D stream is checked by a
  decoder written from the standard and against `rvt-gltf --no-instancing
  --no-edges`, and the PDF by qpdf and a poppler render in CI. Interactive 3D
  needs a viewer that reads U3D (Adobe Acrobat and Reader, Foxit, PDF-XChange).
- **`rvt-gltf` nodes carry ElementIds.** `rvt-gltf` now gives each element
  its `Revit` set (ElementId, category) as the browser's conversion does, so
  the glb's node extras have `elementId` and `category`; before, its nodes
  had none.
