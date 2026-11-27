- **Linked and imported models and files are read: `rvt::links` and `rvt-info --links`.**
  Every Revit link, IFC link, CAD link or import, image, PDF, point cloud and topography
  link, with what the Manage Links dialog shows for each (name, status, Attachment or
  Overlay, saved path and path type, file size, resolution, placement count), each
  placement's transform and view, a CAD import's units, settings and layers (with colour
  and pen), an IFC link's source IFC file, a Revit link's phase map, and the keynote
  table, assembly code table and decal paths the `TransmissionData` stream records. On
  the public samples (Autodesk 2014 to 2027, Snowdon Towers, BIM4LCA; 124 files) all 57
  Revit link types and placements, 91 CAD types and 229 CAD placements Revit's API lists
  are found, with their types, owner views, transforms, linked or imported state,
  attachment and path type; of the images, 208 of 210 types and 163 of 165 placements,
  the other two of each being a newer sample file the API was run on. Objects of deleted
  elements and what loaded families hold are left out, as the API leaves them. `--links`
  prints each placement's origin and x axis; `--json` gives the whole transform.
