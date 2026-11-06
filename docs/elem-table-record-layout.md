# Global/ElemTable record layout

`Global/ElemTable` is Revit's element-id index: it lists every ElementId
in the file, in ascending order, with its owner. `src/elem_table.rs`
reads it; `rvt-elem-table` dumps it (`--raw` prints each record's bytes).

All offsets below are into the inflated stream. The stored stream is
checksum-paged: the page trailers are stripped before inflating, and the
gzip member's CRC32 and `ISIZE` then agree with the inflated length.
Inflating the unstripped bytes gives a longer stream that fails the CRC.

## Header

| Offset | Type | Meaning |
|---|---|---|
| `0x00` | `u16` | The serialization tag of the class `ElemTable` in the file's own schema. The same on every file of a release, project or family: 1174 on 2016, 1370 on 2023, 1411 on 2024, 1451 on 2025, 1481 on 2026. Exposed as `ElemTableHeader::element_count`, but it is not a count of elements. |
| `0x02` | `u32` | The number of records. A table can hold more than a `u16` counts (one office project states 72,798). |
| `0x06` | | Record 0. |

## Records

Two fixed record sizes, chosen by release:

| Releases | Size | Layout |
|---|---|---|
| Revit 2008 to 2023 | 28 B | `[u32 id_primary][u32 id_secondary][16 B][u32 owner]` |
| Revit 2024 and later | 40 B | `[u64 id_primary][12 B][u64 id_secondary][u64 owner][u32]` |

Field offsets within a record:

| Field | 28-byte record | 40-byte record |
|---|---|---|
| `id_primary` | `+0` (`u32`) | `+0` (`u64`, read as its low `u32`) |
| `id_secondary` | `+4` (`u32`) | `+20` (`u64`) |
| owner | `+24` (`u32`) | `+28` (`u64`) |

The records fill the stated count from `0x06` and are followed by a tail:
19 bytes after 28-byte records and 24 bytes after 40-byte records on
project files, and a longer trailer on family files. Two worked examples:

| File | Release | Records | Inflated length |
|---|---|---|---|
| `Revit_IFC5_Einhoven.rvt` | 2023 | 2,615 × 28 B | 73,245 = `0x06` + 73,220 + 19 |
| `2024_Core_Interior.rvt` | 2024 | 26,425 × 40 B | 1,057,030 = `0x06` + 1,057,000 + 24 |

Family files use the same two record sizes as project files (28 bytes
through 2023, 40 bytes from 2024), with record 0 at `0x06`.

### Ids

- The ids rise across the table. The first record is whatever element has
  the lowest surviving id, which depends on the file's history (the
  template it started from, deleted elements): `AllProjectPhases` at 0 on
  some projects, at 1 on Autodesk's sample projects, `DimensionStyle` at 0
  on families. Id 0 is an ordinary id.
- `id_primary` and `id_secondary` are equal on nearly every record. Where
  they differ (seen only on Autodesk's Snowdon Towers samples: 84 records
  on Architectural, 27 on Structural), `id_secondary` is the element's
  current ElementId, the one its partition record carries and Revit's own
  IFC export writes as `Tag`; `id_primary` is the id the element was
  created with, which only its GlobalId still uses.
- `declared_ids` therefore takes `id_secondary` on 40-byte records when it
  is not 0, and `id_primary` otherwise. The table's terminator record has
  id 0, so the declared set can include 0.

### Owner

The field that closes the record names the element this element belongs
to: the curtain wall of a curtain panel, mullion or grid, the sketch of a
sketch line, the model group of a grouped element. Its unset value is all
`0xFF` on project files and `0` on family files. The library reads it as
`ElemRecord::owner_id` (`None` when unset). No record owns itself and no
owner chain loops.

On family files the owner of record 0 is ElementId 17. That is the
`0x0011` at `0x1E` (28-byte) or `0x22` (40-byte) which
`ElemTableHeader::header_flag` reports; it is a record field, not a header
field.

### What the records do not hold

The record payload does not encode a byte offset into `Global/Latest`.
ElemTable is the authoritative set of declared ElementIds, not an offset
index: a `HandleIndex` can be checked against it for coverage, but element
offsets come from walking `Global/Latest` through the schema.

## Layout detection

`detect_layout` picks the record size at which the stated number of
records fits from `0x06`, judged on `id_secondary` (the current ElementId),
in three tiers:

1. **Strict:** the ids rise on every record. When both sizes fit, the one
   leaving the shorter tail wins (a 40-byte table also fits at 28).
2. **Tolerant:** the stream ends with exactly the documented tail (19 or
   24 bytes) and the ids rise on at least 99 % of records. Revit 2023 files
   saved from older releases have a few falls among tens of thousands of
   records.
3. **First id rising:** `id_primary` rises over the whole table. Workshared
   central files saved through a server (Revit 2022 and 2023) have a few
   records where `id_secondary` falls back below its predecessor.

When no tier fits, it falls back to a 12-byte implicit layout from `0x30`
with no owner field (`RecordFraming::Implicit`). No corpus file uses it.

`tests/elem_table_corpus.rs` pins the layout and record counts on a
synthetic fixture, every family release and the two project files above.
