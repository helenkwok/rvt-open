- **`rvt-sheet --sheet` draws a sheet, `--view` draws sections and
  elevations, and every drawing can be written as a vector PDF.** A sheet is
  laid out as the authoring tool's DWG export with views merged lays one
  out: each plan, section and elevation on it in model space, shown through
  a viewport at the view's scale, placed so that the view's internal origin
  lands where the sheet puts it (`Viewport.m_offset`); the title block's
  lines and fixed labels, the sheet's text notes and detail lines and each
  view's title in paper space (`Layout1`). Sections and elevations are their
  elements' edges inside the crop box, projected onto the view's plane (no
  hidden-line removal yet). `--list-sheets` lists the sheets and their
  views. `--format pdf` writes one page: a sheet at full size with model
  space clipped to each viewport, a view at its scale; `rvt-dwg pdf-check`
  scores a PDF's segments against the DXF it came from and names points that
  lie on no linework. The DWG and DXF writer gives each sheet its own
  viewport first: a DWG stores no viewport ids, and without it a reader
  takes the first view's viewport for the sheet's own and drops that view.
  `tools/dwg-oracle/score.sh` scores sheet rows (paper space with model
  space seen through each viewport) in DXF and DWG, and checks the PDF. Over
  the 96 sheets of the public sample projects that have the authoring tool's
  DWG export (2023, 2025 and 2027), sheets went from not drawn to a linework
  F1 median of 0.718 (DXF and DWG alike; text F1 median 0.135 as DXF, 0.148
  as DWG); the 35 plan rows are unchanged (median 0.815, text 0.291), now
  scored as DWG too; all 131 PDFs draw their DXF's linework (precision and
  recall at least 0.99).
