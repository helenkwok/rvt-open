- **The viewer no longer calls a Revit 2008 to 2014 file an unsupported
  version.** Its file status warned "Unsupported Revit version" for any file
  before Revit 2015, though rvt-open reads 2008 to 2014 element records
  experimentally; it now warns only
  outside 2008 to 2027, as `rvt-inspect` decides. `rvt-inspect`'s
  `unsupported_revit_version` message, and the docs that quote it, name the
  same 2008 to 2027 range.
