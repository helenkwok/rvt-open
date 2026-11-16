- **A sheet's lines are their lineweights on paper in every viewport.** The
  views on a sheet were drawn through one shared copy of model space whose
  line width was set for a single scale, and in the viewer a line inside a
  1:100 viewport came out a hundred times too wide, so a floor plan showed
  as a solid grey shape and sections as thick bars. Model space is now
  drawn into each viewport with its own widths: every line on a sheet, in
  the SVG, the viewer and the DWG pane, is its own or its layer's lineweight
  (0.25 mm where none is set) on paper at any viewport scale, and the PDF
  writes the same widths instead of one hairline.
