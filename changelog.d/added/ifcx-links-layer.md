- **Links as an IFCX layer: `rvt-ifc --ifcx --ifcx-links <path>`.** A layer over the IFCX
  file holds a node per linked or imported file (`rvtopen::revit::Link`: kind, name,
  linked or imported, saved path and path type, status, Attachment or Overlay, the IFC
  file an IFC link was made from) and, under the project node, a node per link
  placement (`rvtopen::revit::LinkInstance`) with its transform as `usd::xformop` in
  metres, so a linked model's own IFCX composes under it. The main file does not change.
  `rvt::ifc::write_ifcx_graph_links_layer`.
