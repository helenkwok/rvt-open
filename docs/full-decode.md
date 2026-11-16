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

## Baseline (2026-11-16)

| Bytes | decoded | known, not decoded | unknown |
|---|---|---|---|
| file container (stream sectors counted below) | 17,040 | 20,489 | 0 |
| streams as stored | 5,636,090 | 11,629 | 0 |
| inflated streams | 26,867,852 | 3,819,349 | 6,145,745 |

All 5,601 API elements are located; the file's other 30,522 element objects all lie in embedded content documents. Level, created and demolished phase and design option match on all 5,601; class on 5,423 (the rest share a file class with another API class); name on the 581 elements with an `m_name`; category, is_type, host, workset and UniqueId are not decoded yet. Of 37,420 parameter values, 2,076 match a stored value, 97 differ, 78 differ in storage type, 32,050 have no stored entry (17,991 read-only) and 3,119 have a built-in id the catalog does not hold.
