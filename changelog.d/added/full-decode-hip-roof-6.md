- **Built-in parameter names the catalog missed.** `builtin_parameters::ALIASES`
  lists 13 `BuiltInParameter` names that share an id with a catalog member
  (names the Revit 2026 API renamed, such as `OMNICLASS_CODE` and
  `UNIFORMAT_CODE`, and synonyms such as `SYMBOL_NAME_PARAM`), and
  `parameter_by_enum` resolves them. On `1-hip-roof` (Revit 2024) every
  parameter the Revit API reports now has a known id (3,119 values had
  none).
