- **Families of Revit 2008 to 2011 draw their forms and nested families
  instead of boxes.** Those releases give the `ElementId` class schema tag 19
  (tag 20 from Revit 2012 on), so the schema reader read every id as an object
  wrapping it and no extrusion found its sketch: every form fell back to its
  bounding box. The reader now finds `ElementId` by its class name. With ids
  read, four families of Revit 2009 and 2011 measured draw every visible form
  as a solid. Three more causes are fixed with it: such families keep their
  nested families' documents among their own objects, which were drawn as the
  family's own forms at its origin (each element's `m_famId` now names the
  document that holds it, and each nested instance is drawn from its family's
  document by its transform, mirrored where it is flipped); their stored
  bounding boxes are faceted, so an extrusion with arcs may be up to 2.51% of
  its largest radius larger on a side than its box; a sketched circle saved
  as an arc a thousandth of a turn short now closes. Of a record kept in
  several partitions, only the newest copy is read, and the sample host a
  hosted family's template keeps (a door's wall, a face-based family's block,
  named by the family's `m_refs`) is not drawn as one of its forms. Their forms and nested
  instances take the material their record names (by the material's name, as
  the material resolver reads only 2015 and later), so colours and glass
  transparency show.
