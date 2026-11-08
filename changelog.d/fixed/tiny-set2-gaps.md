- **The IfcSite stands at the project position.** Under `--convention
  revit` (the default) the site is placed at the shared coordinates of the
  internal origin and turned by the angle to True North, with the context's
  True North the shared Y axis, as Revit's IFC4 export writes it: all 10
  exports of the tiny reference models set 2 recipe 20 (2018 to 2027) put the
  site at E/W 10 m, N/S 20 m, turned 30 degrees, and rvt-open now does too.
  `--ifc-origin` still moves the origin from the internal frame.
- **Areas carry their category and area.** An Area's record has no box, so
  the record scan gave it no `Category`; it is now `Areas`, and an Area drawn
  from straight boundary lines has its `Area` (the tiny areas of 2018 to 2023
  and 2025 to 2027 within 0.01 m² of Revit's).
- **Shaft openings, rectangular wall openings, toposolids and topography
  surfaces.** A shaft opening is written as the `IfcOpeningElement` it cuts in
  each slab its record box crosses in a volume, named for the slab's type and
  tagged with the shaft, as Revit's export does (the Level 2 floor, not the
  Level 1 floor the shaft stands on); an element that only cuts openings is
  listed once in the element table under its category. A toposolid (2024 on)
  and a topography surface are `IfcGeographicElement .TERRAIN.` with their
  record box, which a topography surface's saved mesh replaces where site
  content is read.
- **A footprint roof with no slope-defining edge is its one slab on Revit
  2018 to 2023**, with Revit's GlobalId (the flat roof of the tiny reference
  models set 2 recipe 7, 2018 to 2020).
- **Grids** are read from their records on Revit 2024 to 2027 and written as
  one `IfcGrid` of named `IfcGridAxis` lines on the first storey, a row per
  grid in the element table.
- **A wall with an edited elevation profile** takes the location curve its
  record's driver names, so it has its `Axis` and length on 2024 to 2027.
