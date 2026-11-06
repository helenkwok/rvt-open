- **The element table as XLSX, Parquet and SQLite.** `rvt-schedule
  --format xlsx|parquet|sqlite`, `rvt-batch --formats xlsx,parquet,sqlite`,
  the library's `rvt::ifc::table_formats` and the wasm binding
  `element_table` write the JSON Lines element table's columns as a workbook
  (sheets `elements` and `parameters`), an SQLite database (tables
  `elements` and `parameters`, joined on `element_row`) or a Parquet file
  (one row per element, its parameters a list column). SQLite is behind the
  `sqlite` feature (the bundled SQLite; on in the release binaries) and in
  the 32-bit browser package. XLSX and Parquet are in both browser packages.
- **Crash-safe outputs and an output manifest.** The conversion CLIs write
  each output beside its name and rename it into place, so a crash never
  leaves half a file (craft-atomic, vendored whole under
  `vendor/craft-atomic`). Every `rvt-batch` folder ends with `manifest.json`
  (format `rvt-open-output`, schema version, decode limits, each file's size
  and SHA-256), and a folder a killed run left unfinished is converted again.
- **Browser outputs straight to disk.** The wasm bindings
  `convert_to_handles` and `convert_to_handles_reader` write each output to
  a `FileSystemSyncAccessHandle` the page passes (OPFS), one at a time, and
  return the output manifest.
