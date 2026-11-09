- **IFCX carries what an IFC4 export leaves out of a Revit file.** Beside
  the building elements, their types and storeys, `rvt-ifc --ifcx` now
  writes every other element the file saves parameters for, each a node
  whose path is the GlobalId Revit's exporter gives it, under one
  `RevitElements` node of the project: phases (with their name and order),
  design options (with their name and set), views, materials, project
  information and the types no element uses, each with its own parameters
  under `rvtopen::revit::<Group>::<Name>`; an exported element or type also
  gets the parameters it lacked. `--no-ifcx-revit-elements` leaves them out.
  The IFC4 file is unchanged (all 220 tiny-model exports identical but for
  the export time), and the USD and `.frag` files are written before the
  IFCX file gets them. Measured on the tiny
  reference models (both sets, every release from 2018 to 2027) against
  what the Revit API reports (`tools/ifcx-coverage/coverage.py`,
  `docs/ifcx-coverage.md`, which CI runs and fails on a drop): IFCX now
  holds 100% of the phases, design options, materials and project
  information, 92% of the views and 59% of the unused types, none of which
  it held before, and 89% of set 2's placed elements (65% before); Revit's
  own IFC4 Reference View export holds 66%, and none of the others.
