# Full decode of one file

No Revit file has been decoded completely yet, by rvt-open or any other reader. `tools/full-decode/full_decode.py` measures how far rvt-open is from that on one small file, the tiny reference model `1-hip-roof` saved in Revit 2024 (release asset `fixtures-tiny-1`, MIT), against that file's Revit API dump (5,601 elements with every parameter and property). CI runs it in the job "full decode of one tiny model" and fails when a total falls below `tools/full-decode/floor.tsv`, so coverage can only rise. When a change raises a total, regenerate the floor with `--write-floor` in the same pull request.

```bash
cargo run --release --example full_decode -- 1-hip-roof.rvt > facts.json
python3 tools/full-decode/full_decode.py 1-hip-roof.rvt 1-hip-roof.api-dump.json facts.json out/ --floor tools/full-decode/floor.tsv
```

`examples/full_decode.rs` writes what the schema reader reads (every partition object with the bytes it read and the bytes between objects, every element object's fields and stored parameters, the element table's rows) without interpreting it; the script does the accounting and the comparison. Needs `olefile` (`tools/ci/requirements-olefile.txt`).

## The three axes

**Bytes.** Every byte is `decoded` (its meaning is read and checked), `known` (the structure around it is known, its content is not read) or `unknown`, counted in three places:

- the file: the CFB header, FAT, directory, mini FAT and free sectors; sector slack past a stream's end and unused directory entries hold stale bytes that belong to no stream (`known`);
- each stream as stored: page parity (every codeword of every page is recomputed), gzip members (inflated, CRC-32 and length checked), the partition framing (segment markers whose stored sizes agree with their members, section headers, terminal), zero padding; the 64-byte signature values of partition sections and the leading word of the `Global/*` streams are `known`;
- each compressed stream inflated: partition objects through the schema (header, fields read, trailing size; bytes an object's read did not reach are `known`), the bytes between partition objects (`unknown`), the element table's rows; the other inflated streams are object graphs not yet accounted byte by byte (`known`).

**Elements.** An API element is located when the file holds an element object whose root `m_id` is its ElementId and `Global/ElemTable` has a row for it. Its class, category, is_type, name, level, host, workset, created and demolished phase, design option and UniqueId are each `match`, `differ` or `not decoded`. A file class decodes the API class when every element of that file class has the same API class. Every element object of the file that is not an API element is counted by where it lies: after the partition's first section header it belongs to an embedded content document.

**Values.** Every parameter the API reports with a value is looked up among the element's stored parameters, by its built-in id or by the id of its parameter element, and compared by storage type and value (lengths in internal feet, as Revit stores them). A value with no stored entry is `not stored`, split by whether the API marks it read-only (likely computed). Properties are compared where a file field is known for them.

## Measured

| Bytes, 1-hip-roof (Revit 2024) | decoded | known, not decoded | unknown |
|---|---|---|---|
| file container (stream sectors counted below) | 17,040 | 20,489 | 0 |
| streams as stored | 5,636,090 | 11,629 | 0 |
| inflated streams, first measure | 26,867,852 | 3,819,349 | 6,145,745 |
| inflated streams, element headers read | 33,013,573 | 3,819,349 | 24 |
| inflated streams, object graphs and schema read | 36,808,260 | 24,686 | 0 |

The bytes between a partition's data objects are element-header records, one per element object, in runs with a start and an end marker (`schema_objects::header_records`): on this file 36,123 records and their markers fill all 6,145,721 such bytes, and every record reads through the schema. Each `Global/*` stream reads as one object graph from its first byte (`schema_objects::read_graph`), followed by a u32 0 (and, in `Global/History`, the episode list); `Global/ContentDocuments` is one record per embedded document, a key object, a u32 size, the document's graph and the size again (`schema_objects::content_documents`: 93 records, every graph read exactly); `Formats/Latest` reads to its end through the schema grammar. What stays `known`: the classes' 1,524 GUIDs in the schema stream (24,384 bytes, stepped over), the `Contents` header and member, 48 bytes of one Extensible Storage payload, the closing 14 bytes of `ContentDocuments`, the 93 partition section signatures, the `Global/*` prefix words, `BasicFileInfo`, `TransmissionData` and `ProjectInformation` (read by rvt-open, not accounted byte by byte), the end bits before each stream's page parity, and the container's stale bytes.

All 5,601 API elements are located; the file's other 30,522 element objects all lie in embedded content documents. Category (the element header's), level, created and demolished phase, design option, workset (the element table's partition field), UniqueId (creation episode and first id) and host match on all 5,601; class on 5,423 (the rest share a file class with another API class); is_type on 5,432: the 169 others derive from `Symbol` and have the API class ElementType, but the dump marks them as no type; name on 5,210 (`rvt::element_names`: the element's own objects, a graphics style's category, a font element's owner; 581 before), 52 differ (32 internal dimension styles named `SecretInternal...` in the file and otherwise by the API, 14 sun settings the API names by their in-session state, 4 tile patterns, ...), 339 not decoded (82 legend components, 36 conceptual construction types, sketch planes, viewers and other elements whose name is not text in their own objects). Of 37,420 parameter values, 2,076 match a stored value (97 differ, 78 differ in storage type). Most of the rest are fields Revit shows as parameters (`rvt::field_parameters`): 16,518 match the element header's category and design option (18 differ: sub-elements whose API category is a subcategory), and 6,117 match an element field through 367 (class, parameter, field) rules measured on this file. 9,397 have no stored entry and no rule yet (1,165 read-only), and 3,119 have a built-in id the catalog does not hold. Of the 119,009 property values, 56,591 match a decoded field (id, level, phases, owner view, name, UniqueId, workset, design option, view-specific, pinned), 20,013 are the API's runtime state (`IsValidObject`, `IsModifiable`, ...), not file data, and 42,405 are not compared yet or have no decoded field.
