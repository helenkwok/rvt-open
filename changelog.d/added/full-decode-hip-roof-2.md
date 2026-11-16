- **Element headers read, and every byte between partition objects
  accounted for.** The bytes between a partition's data objects are runs of
  element-header records, which `schema_objects::header_records` reads
  through the schema (a class reference, kind 0x0a, is now read: 2 bytes).
  On `1-hip-roof` (Revit 2024) its 36,123 records fill all 6,145,721 such
  bytes, so 24 of 36,832,946 inflated bytes stay unknown (from 6,145,745).
  `revit_unique_ids` gives each element its UniqueId (5,601 of 5,601 as the
  Revit API reports them), and the element header's category, the element
  table's partition and the schema's `Symbol` base match the API's category,
  workset and type flag (5,601, 5,601 and 5,432 of 5,601).
