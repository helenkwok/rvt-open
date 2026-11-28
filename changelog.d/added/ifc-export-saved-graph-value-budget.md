- **IFC export** takes a per-graph saved-graphics limit through
  `ExportOptions::with_saved_mesh_limits` and
  `rvt-ifc --saved-mesh-graph-values <count>`. The default is 1,000,000 field
  values per graph. A graph over the limit keeps its usual fallback.
