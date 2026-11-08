- **`rvt-sheet --view` draws family plan symbols, projection and overhead
  lines, the project's own lines, and site plans turned to true north.** Each
  family instance's plan symbol (door swings, window and fixture lines) is
  drawn through the instance's own transform, so its flips and hand are the
  instance's. Elements wholly below the cut plane are drawn as the edges of
  their bodies seen from above instead of their outlines, and stairs and
  railings the cut plane cuts draw their part above it as overhead lines on the
  `-MBND` layer. Model lines, room separation lines and the view's own detail
  lines (legend views drawn as plans among them) are drawn. A view whose axes
  are turned (a site plan set to true north) is drawn in its own axes, and
  grids are left out where the view hides them. Over the same 35 plan views of
  the public sample projects, linework F1 went from a median of 0.749 to 0.783
  (mean 0.681 to 0.751, 29 views better, 6 worse); the two site plans from
  0.007 and 0.021 to 0.473 and 0.529, and the legend view from 0 to 0.942.
