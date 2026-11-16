- **The `Global` streams and embedded documents read as object graphs.**
  `schema_objects::read_graph` reads a single-object stream from its root
  and `content_documents` the records of `Global/ContentDocuments` (a key,
  a size, the document's graph, the size). On `1-hip-roof` (Revit 2024)
  every `Global` stream reads to its trailing word and all 93 embedded
  documents read exactly; with the schema stream read by its grammar, the
  inflated bytes not decoded fall from 3,819,349 to 24,686 (the classes'
  GUIDs, stepped over, are most of them).
