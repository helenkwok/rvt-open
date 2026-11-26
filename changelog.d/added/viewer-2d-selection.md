- **The viewer selects elements in a model's sheets, plans, ceiling plans,
  sections and elevations as Revit does.** Hovering pre-highlights the whole
  element under the cursor in Revit's pre-selection colour and names it in the
  status line; a click selects it, Ctrl and Shift add and remove, Tab cycles
  the elements under the cursor and offers a chain of joined walls or lines, a
  window drawn left to right selects what lies inside it and one drawn right to
  left also what it crosses, Esc clears and Ctrl+Left Arrow restores. On a
  sheet a viewport selects as one element until a double-click activates it.
  The panel shows the selected element's properties, the selection count shows
  in the drawing's toolbar, and the 3D view and scene tree follow the drawing's
  selection and the other way round. "Select elements by face" makes fills and
  closed outlines select from inside. A left drag now draws the selection
  window in a model's drawing; a middle drag, or a left drag with Space held,
  pans.
- **Sheets and views drawn from a model name the element each line, text, fill
  and dimension draws** (`Entity2d::element`, written to SVG as
  `data-element`): model elements by their ElementId, and text notes, room
  tags, dimensions, grids, levels, detail lines, filled regions, title blocks
  and viewports by theirs.
