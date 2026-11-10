- **A Revit 2026 or 2027 save is written without an `ObjectType` on any element.**
  Revit's own export of those releases writes none, where rvt-open wrote
  `Family:Type`: of the five held-out pairs saved by 2026 and 2027, 418 typed
  elements of nine classes differed and Revit's export writes no `ObjectType`
  on any of them (counts only), and its export of the same projects saved by
  2025 writes `Family:Type` on every typed element, as rvt-open does for 2025 and
  earlier. A `USERDEFINED` `PredefinedType` states its type in `ObjectType`, so where
  there is none the `PredefinedType` is left unset, as for any element whose
  `ObjectType` was not read. Revit 2025 and earlier are unchanged.
