- **More parameter values from where Revit keeps them.**
  `SchemaObject::nested_parameters` reads the parameter sets an element's
  other objects own (a property set's `m_oParamSet`); `field_parameters`
  follows pointers, with 26 more rules (a type's compound structure, a
  material's own object); the full decode measures a type's name and its
  family's name as Type Name and Family Name, and integer parameters kept
  in the Double set. On `1-hip-roof` (Revit 2024) parameter values with no
  source fall from 12,480 to 7,929 and 78 storage-type differences to 0.
