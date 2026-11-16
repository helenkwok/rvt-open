- **Element names read from the element's own objects.** `element_names`
  reads a name from the element object or an object it points to (a type's
  `m_symbolInfo`, a category's, material's or pattern's own object, a
  parameter definition, a property set's name parameter), a graphics
  style's from its category (built-in categories from the catalog, with 697
  subcategory names added; a subcategory element's, or its owner's where it
  is empty) and a font element's from its owner. On `1-hip-roof` (Revit
  2024) names match the Revit API on 5,210 of 5,601 elements (from 581).
