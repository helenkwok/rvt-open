- **`rvt-sheet --view` draws each project line by what owns it, grids to
  their extents in the view with their heads, and hides what lies under a
  roof.** A line that only shapes another element is no longer drawn: a
  family's own lines, the sketches of ceilings, roofs, railings, stairs, walls
  and openings, sweep paths, and area boundaries (area plans only). Model and
  detail lines go on `A-DETL`, room separation lines on `A-AREA`, property
  lines on `C-PROP-LINE` in every plan, the boundary sketches of floors,
  toposolids and building pads on their category's layer, and rooms as the
  export's room polylines on `A-AREA-BNDY`. Grids run to their extents in the
  view, out to the annotation crop, with a head (a circle and the grid's name,
  `S-GRID-IDEN`) at the ends the view shows one; grids whose plane's x axis
  points up are drawn too. A view's scale is now read the right way round
  (it is stored as 0.01 for 1:100), so text heights and the annotation crop
  are their real sizes. Over the same 35 plan views of the public sample
  projects, linework F1 went from a median of 0.783 to 0.815 (mean 0.751 to
  0.781, 27 views better, 6 worse) and text F1 from a median of 0.139 to 0.291
  (mean 0.179 to 0.336, 29 better, none worse).
