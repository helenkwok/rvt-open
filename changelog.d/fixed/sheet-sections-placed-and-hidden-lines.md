- **A sheet's sections and details are drawn inside their viewports, with
  hidden lines removed and the section plane's cut drawn.** A section was
  placed on the sheet by the model's origin, where a viewport's offset places
  the view's crop box origin: on the Technical School sample's sheet A2 the
  1:10 details landed 0.4 m left of the sheet and 0.6 m above it, the 1:20
  wall section 0.2 m left, and their viewports showed nothing. The viewport
  now frames the view's crop (with its annotation crop when that is on), and
  all four of A2's viewports lie inside the sheet within 25 mm of the centres
  the authoring tool reports. A section draws each element's outline where
  its near crop face cuts it, at a 0.5 mm lineweight, and only the edges no
  nearer face hides, found with a depth buffer at 0.2 mm on the sheet; floors,
  roofs and slabs are drawn too. Its text notes and detail lines, and the
  levels and grids it crosses with their heads, are drawn in it. A drafting
  view draws its notes, detail lines and the lines of a drawing imported into
  it (A2's Roofing Termination Detail), and a schedule on a sheet is named as
  not drawn yet instead of being called a view that is not a plan.
- **A text note's size is its text's, not its box's, and a note that wraps is
  broken into lines.** A note's stored height is its box's, ascent to descent
  and all its lines; the em is that over 1.117, which gives the widths of the
  notes and labels in the authoring tool's PDF of sheet A2 to within 1%. The
  wrapped notes of A2's Detail At Grade were drawn several times too large.
- **SVG drawings hold no control characters and a sheet's SVG frames the
  sheet.** A text note's stream after its terminating zero wrote NULs, which
  XML forbids, so a browser drew none of sheets A1 and A2; and model space
  drawn for the viewports widened a sheet's view box, so the sheet showed as a
  speck beside the model.
- **A plan leaves out the property lines and topography its view hides.**
  Property lines were drawn in every plan and a topography surface, which
  has no category of its own in the export, was never hidden: the Technical
  School sample's floor plan, which hides both, drew a 319 m surface and the
  site's property lines around its building, so sheet A1's viewport reached
  far past the sheet.
