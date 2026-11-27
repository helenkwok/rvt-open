- **A Rust caller can set how many values one saved-graphics graph may
  hold** with `saved_meshes::attach_scoped_with_limits_and_control` and
  `SavedMeshLimits::with_max_graph_values`. A graph over the limit is left
  out of the attach pass and its elements keep their bodies with the usual
  refusal; the other attach calls keep the one-million-value default.
