- **A building element proxy takes its own or its type's "IFC Predefined Type"
  where that is an `IfcBuildingElementProxyTypeEnum` enumerator.** In
  each of the twelve BIM4LCA files (six housing, six office) one proxy carries
  `USERDEFINED` under an "Export to IFC As" of `IfcBuildingElementProxy`, and
  Revit's export writes `USERDEFINED` on it; rvt-open left it unset. A proxy with
  no value keeps `NOTDEFINED`.
