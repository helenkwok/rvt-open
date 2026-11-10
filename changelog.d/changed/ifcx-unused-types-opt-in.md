- **IFCX leaves out the types no element uses unless asked.** `rvt-ifc
  --ifcx` no longer writes, by default, the types that no element names
  (most come from the project template); `--ifcx-unused-types` writes them
  as before. A type an element takes its type values from, names in a type
  field of its record (a railing's or stair's type) or is exported with
  stays. On the tiny reference models the IFCX files are 51 to 53%
  smaller (sets 1 and 2: 22.9 to 10.6 MB, 81.4 to 39.8 MB) and carry 5%
  of the unused types instead of 59%, every other coverage total
  unchanged; on CI's five public sample projects they are 1 to 3% smaller.
  Conversion time is unchanged within noise, the IFC4 files are
  byte-identical, and with the flag the IFCX files are byte-identical to
  before. CI measures coverage with the flag.
