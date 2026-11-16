- **The small streams accounted byte by byte.** The full decode reads each
  `Global/*` stream's u64 format version, the `Contents` header's magic,
  versions and build number, `BasicFileInfo`'s counted strings and text
  lines, `TransmissionData`'s counted XML and `ProjectInformation`'s ZIP
  archive. On `1-hip-roof` (Revit 2024) stored bytes known but not
  decoded fall from 11,629 to 6,120 (93 section signatures are most of
  the rest).
