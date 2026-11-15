- **A title block's labels are drawn with the sheet's and the project's
  values.** A label in the title block family (`m_tagNoteData` of its
  `FamilySymbol`) names the parameters it shows, its point, box and
  alignment; the values come from the sheet (name, number, drawn and checked
  by, dates), the project information (name, number, client, issue date) and
  the scales of the views placed on the sheet (one scale, or "As indicated").
  On the Technical School sample's sheet A2 all ten labels are drawn, centred
  and right-aligned where the authoring tool's own PDF of the sheet places
  them. `rvt-sheet --sheet` and section views now also write SVG.
