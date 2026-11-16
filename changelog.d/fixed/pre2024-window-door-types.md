- **A door or window of a Revit 2015 to 2023 file that the reference-list join
  leaves untyped is typed from its own native record.** The scan names a type
  only through a symbol record with a name entry, and for most windows of a
  Revit 2018 project the list holds only the type's master symbol, while
  Revit's own type is an id the scan knows nothing of. The instance's native
  record names it (`InstanceInfo.m_symbolId`), and its master symbol gives the
  type's name and the family. A member of a model group takes its master symbol
  as the type, as Revit's export does, and is left untyped where its symbol is
  not a copy of that master. Such doors and windows now have Revit's Name,
  `ObjectType` and type object, with the type's `Pset_WindowCommon.Reference`
  and `Pset_DoorCommon.Reference`. Their type objects' GlobalIds are not
  Revit's, and they take no material from the type map yet.
