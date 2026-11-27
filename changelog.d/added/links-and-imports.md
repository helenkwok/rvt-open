- **Linked and imported models and files are read: `rvt::links` and `rvt-info --links`.**
  Every Revit link, IFC link, CAD link or import, image, PDF, point cloud and topography
  link, with what the Manage Links dialog shows for each (name, status, Attachment or
  Overlay, saved path and path type, file size, resolution, placement count), each
  placement's transform and view, a CAD import's units, settings and layers (with colour
  and pen), an IFC link's source IFC file, a Revit link's phase map, and the keynote
  table, assembly code table and decal paths the `TransmissionData` stream records. On
  the public samples (Autodesk 2014 to 2027, Snowdon Towers, BIM4LCA; 124 files) the
  link, import and image elements Revit's API lists are found with their types, owner
  views, transforms, linked or imported state, attachment, path type and pixels. Objects
  of deleted elements and what loaded families hold are left out, as the API leaves them.
