- **`SavedMeshChoice::DoorsAndWindows` (new) and the MCP server's
  `saved_meshes: "doors_and_windows"` draw doors and windows alone from their
  saved meshes, reading less of the file; `saved_meshes: "default"` (new, the
  default) and `"all"` draw every element that would be a bounding box.**
  `saved_meshes::default_scope` now names every boxed element on Revit 2014 to
  2027, as `rvt-ifc`, `rvt-gltf`, Python, the MCP server and the viewer already
  drew them by default; the docs, the viewer's option and its progress stage
  say so ("Draw family instances from saved meshes"). No default export
  changes. A mesh with a face that does not decode still keeps its box, and
  `--no-saved-meshes`, `saved_meshes=False`, `saved_meshes: "off"`,
  `SavedMeshChoice::Off`, `savedMeshes: false` and the viewer's option keep the
  boxes.
