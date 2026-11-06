# Data outputs

What rvt-open writes besides the model formats: an element table for data
tools (JSON Lines, XLSX, Parquet, SQLite), IFC options that shape the IFC
file, an OBJ export, a run report, an output manifest, and a batch mode for
folders of files.

| Output | Command |
|---|---|
| Element table (JSON Lines) | `rvt-schedule model.rvt --format jsonl` |
| Element table (XLSX, Parquet, SQLite) | `rvt-schedule model.rvt --format xlsx\|parquet\|sqlite` |
| IFC profile, units, origin, element filter, phase | `rvt-ifc model.rvt --ifc-profile … --ifc-units … --ifc-origin … --ifc-categories … --phase …` |
| OBJ with materials | `rvt-gltf model.rvt --format obj` |
| Run report | `rvt-ifc model.rvt --report run.json`; every `rvt-batch` file |
| Batch | `rvt-batch in/ -o out/` |
| Output manifest | every `rvt-batch` folder; the wasm `convert_to_handles` |

## Element table

```bash
rvt-schedule model.rvt --format jsonl              # model.elements.jsonl
rvt-schedule model.rvt --format jsonl -o -         # to stdout
```

One JSON object per line, one line per exported element (rooms included, as
`IfcSpace`), in the order the export lists them. It runs the same export as
`rvt-ifc` with no flags, so the GlobalIds are those of the IFC file.

| Field | Meaning |
|---|---|
| `element_id` | Revit ElementId (number), `null` where the element has none |
| `global_id` | The element's IFC GlobalId (22 characters), as the IFC file has it |
| `ifc_class` | IFC4 class (`IfcWall`) |
| `predefined_type` | IFC predefined type, or `null` |
| `name` | The element's name as the IFC file has it |
| `category` | Revit category (`Walls`), from the element's `Other` or `Revit` set |
| `family`, `type` | Revit family and type names |
| `type_id` | The ElementId of the element's type |
| `level`, `level_elevation_m` | The storey it is on and its elevation |
| `area_m2`, `volume_m3`, `length_m` | Revit's own `Area`, `Volume` and `Length` parameters where the file gives them; else volume and length measured from the drawn body |
| `quantity_source` | For each of `area`, `volume`, `length` that has a value: `parameter` or `measured` |
| `quantity_basis` | `measured` (the element's own shape) or `estimated` (still its bounding box), for the measured values |
| `parameters` | Every property the export holds for the element: `{set, name, value, unit, of}`, `of` being `element` or `type` |

Values are in SI units: metres, square metres, cubic metres, degrees,
kilograms, seconds, pascals, watts, volts, amperes, m3/s, W/(m2.K). `unit` is
`null` for text, numbers without a unit, booleans and lists of names. An
unknown value is `null`, never a guess. The CSV schedules (`--format csv`,
the default) are unchanged.

### XLSX, Parquet and SQLite

```bash
rvt-schedule model.rvt --format xlsx       # model.elements.xlsx
rvt-schedule model.rvt --format parquet    # model.elements.parquet
rvt-schedule model.rvt --format sqlite     # model.elements.sqlite
```

The same table, with the same columns, in a form spreadsheets, dataframes and
databases open directly. A flat table needs two changes:

- `quantity_source` becomes three columns, `area_source`, `volume_source`
  and `length_source` (`parameter`, `measured` or empty).
- Each element's `parameters` become rows of their own, with the columns
  `element_row`, `element_id`, `global_id`, `set`, `name`, `value`,
  `value_number`, `unit` and `of`. `value` is the value as text (a number or
  boolean as JSON writes it, a list or record as JSON) and `value_number` the
  same value where it is a number, so each column keeps one type.

`element_row` numbers the elements from 1 in the order of the JSON Lines file
and joins the two tables.

| Format | Elements | Parameters |
|---|---|---|
| XLSX | sheet `elements` | sheet `parameters`; past Excel's 1,048,576 rows it goes on in `parameters 2`, `parameters 3`, ... A cell holds at most 32,767 characters (Excel's limit): longer text is cut there |
| SQLite | table `elements` (`element_row` is the primary key) | table `parameters`, indexed on `element_row` and `name` |
| Parquet | one row per element | the list column `parameters` of the element's row, records of `set`, `name`, `value`, `value_number`, `unit`, `of` |

Parquet is written uncompressed. SQLite needs a build with the `sqlite`
feature (the bundled SQLite C library; the release binaries have it):
`cargo install rvt-open --features sqlite`. In the browser package the
32-bit build has SQLite and Parquet; the 64-bit one has Parquet but not
SQLite (SQLite is compiled for wasm32).

```python
import pandas as pd, sqlite3
pd.read_parquet("model.elements.parquet")
pd.read_excel("model.elements.xlsx", sheet_name="parameters")
sqlite3.connect("model.elements.sqlite").execute(
    "SELECT e.category, p.name, p.value FROM parameters p JOIN elements e USING (element_row)")
```

## IFC options

All five apply to the IFC4 file of `rvt-ifc` and `rvt-batch`. `--phase` also
applies to every other output, because it decides which elements are read;
the other four change only the IFC4 file (IFCX, USD, `.frag` and glTF stay as
they were). Revit's own IFC export is the reference for what each means.

### `--ifc-profile full|coordination|geometry`

| Profile | Holds | Leaves out |
|---|---|---|
| `full` (default) | Everything `rvt-ifc` writes | Nothing |
| `coordination` | Geometry, materials, types, spatial structure, classification, identity (GlobalId, Name, Tag = ElementId, the `Revit` set with ElementId and category) and the IFC common property sets (`Pset_*`) | Revit's own parameter sets (one per parameter group, `Other`, `Identity Data`, `Phasing` and the rest) and the base quantities (`Qto_*`), as Revit's coordination setups do ("Export IFC common property sets" on, "Export Revit property sets" and "Export base quantities" off) |
| `geometry` | Geometry, materials, types, spatial structure, classification and identity | Every property set and quantity set except the `Revit` set |

Every element stays in every profile; only property and quantity sets go,
with anything only they used.

### `--ifc-units m|mm|ft|in`

The project length unit of the IFC file. Every length is rescaled:
coordinates, extrusion depths, profile dimensions, storey and building
elevations, door and window sizes, stair riser and tread sizes, material
layer thicknesses and offsets, length quantities and length property values
without their own unit. `IfcMapConversion.Scale` takes the new unit to the
map's, so georeferencing is unchanged. With `m` and `mm` areas stay in square
metres and volumes in cubic metres (as Revit's metric exports write them);
with `ft` and `in` areas and volumes are in square and cubic feet or inches.

### `--ifc-origin internal|project-base|survey-point`

| Origin | IFC origin | Axes |
|---|---|---|
| `internal` (default) | Revit's internal origin | Revit's internal axes (project north) |
| `project-base` | The project base point | Unchanged (project north) |
| `survey-point` | The survey point | Turned to true north (the shared coordinate axes) |

These are Revit's export coordinate bases "Internal Coordinates", "Project
Base Point" and "Survey Point". The move is written as the `IfcSite`
placement, which every other placement is relative to, and the
`IfcMapConversion`, where the file names a coordinate reference system, is
rewritten so every point keeps its map coordinates. The base points are read
from the file; where the one asked for was not read, `rvt-ifc` writes
nothing and says which point is missing.

### `--ifc-categories <list>`

Keeps only the elements whose Revit category or IFC class is on the
comma-separated list: `--ifc-categories Walls,Doors,IfcSlab`. Names are case
insensitive; an entry beginning with `Ifc` is a class (it also keeps that
class's type objects), any other a category. An element's openings, ports and
aggregated parts go with it; a type no kept element uses goes, and so does
anything only the removed elements used. An element contained in a room that
is left out moves to the room's storey. Entries that match nothing are named
in a warning, so a misspelt category shows.

### `--phase <name|id>`

Shows the model as it is in the named phase (by its name, or its ElementId
where no phase has that name), as Revit's export "Phase to export" does: an
element is written when it was created in that phase or an earlier one and
was not demolished in it or an earlier one, and a room is written when its own
phase (`ROOM_PHASE`) is that phase; a room left out is counted in the
diagnostics as `room_not_in_export_phase`. The default is the project's last
phase, which is what Revit exports by default. A phase that is not one of the
project's is an error that lists them.

## OBJ export

```bash
rvt-gltf model.rvt --format obj              # model.obj and model.mtl
```

The meshes of `rvt-gltf --no-instancing --no-edges`, in metres with +Y up (as
glTF), in the model's own coordinates. Each element is one object and group
named after it with its ElementId in brackets, spaces as underscores
(`Basic_Wall_Generic_-_200mm_[12345]`); faces use `usemtl m<n>`, and the
`.mtl` beside the `.obj` gives each material's colour (`Kd`) and opacity
(`d`). Vertices are written for every placed element, so the file is larger
than the instanced glb.

## Run report

`rvt-ifc --report run.json` writes it for one conversion; `rvt-batch` writes
`report.json` into every file's output folder.

| Field | Meaning |
|---|---|
| `schema_version` | `1`; additive fields keep it |
| `tool`, `tool_version` | The command and its version |
| `input` | The input path as given |
| `status` | `ok`, `partial` (a resource limit kept part of the input from being read), `failed` or `skipped` |
| `error` | Why it failed, else `null` |
| `seconds` | Wall-clock time of the conversion |
| `outputs` | `{format, path, bytes}` of every file written |
| `elements_by_category` | Exported elements by Revit category (by IFC class where an element has none) |
| `ifc_options` | What `--ifc-profile` and `--ifc-categories` left out (`property_sets_left_out`, `elements_kept`, `elements_left_out`, `instances_dropped`, `unmatched_categories`), else `null` |
| `notes` | What each pass drew, read or left out, as printed on stderr |
| `diagnostics` | The export diagnostics, unchanged, as `rvt-ifc --diagnostics` writes them ([export-diagnostics.md](export-diagnostics.md)): what was read, what was skipped and why, warnings, unread streams |

With `rvt-ifc --diagnostics … --anonymize`, the report's `diagnostics` are
anonymized too.

## Batch

```bash
rvt-batch projects/ -o converted/                                   # IFC and element table
rvt-batch projects/ -o converted/ --formats ifc,table,glb,obj --ifc-units mm --ifc-profile coordination
rvt-batch inbox/ -o converted/ --watch --interval 30                # keep converting new files
```

`rvt-batch` searches the input folder and its subfolders for `.rvt`, `.rfa`,
`.rte` and `.rft` files (any letter case; symbolic links to folders are not
followed, and the output folder is skipped if it is inside the input). Each
file gets its own folder at the same relative path under the output folder,
named after the file: `projects/a/b/model.rvt` gives `converted/a/b/model.rvt/`,
holding `model.ifc`, `model.elements.jsonl`, `model.elements.xlsx`,
`model.elements.parquet`, `model.elements.sqlite`, `model.glb`, `model.obj`
and `model.mtl` as `--formats` asks (`ifc`, `table`, `xlsx`, `parquet`,
`sqlite`, `glb`, `obj`; default `ifc,table`), and always `report.json` and
`manifest.json`. It takes the IFC options above and `--large-model` and
`--allow-partial` as `rvt-ifc` does. Files are converted one at a time; a
file that fails or stops on an internal error does not stop the others.

**An existing output folder is never written into.** Its file is skipped, so
running the same command again converts only new files; to convert a file
again, delete its folder. A file that fails keeps its folder with its
`report.json`, so delete the folder to retry it.

**A crash never leaves half a file or an unfinished folder that looks
done.** Every file is written beside its name, flushed to disk and then
renamed into place, so it is there whole or not at all. A new folder is
marked `.incomplete` until its `manifest.json` is written, last; a folder that
still holds `.incomplete` (the run was killed) is deleted and converted again
on the next run.

One JSON line per file goes to stdout:

```json
{"input":"projects/a/b/model.rvt","output":"converted/a/b/model.rvt","status":"ok","seconds":12.408,"elements":1843,"outputs":["converted/a/b/model.rvt/model.elements.jsonl","converted/a/b/model.rvt/model.ifc"],"error":null}
```

| Exit status | Meaning |
|---|---|
| 0 | Every file converted or skipped |
| 1 | The run could not start (no input, output folder not creatable) |
| 2 | At least one file failed |
| 3 | At least one file was read only in part (a resource limit; nothing written for it unless `--allow-partial`) and none failed |

`--watch` keeps running after the first pass: every `--interval` seconds
(default 10) it looks again and converts each new file once its size and
modified time are the same on two looks in a row, so a file still being
copied is not read half-written. Stop it with Ctrl-C.

## Output manifest

`manifest.json`, in every `rvt-batch` folder and returned by the wasm
`convert_to_handles`, is the folder's envelope:

| Field | Meaning |
|---|---|
| `format` | `rvt-open-output` |
| `schema_version` | `1`; additive fields keep it, and a reader refuses a newer one (`rvt::ifc::run_report::OutputManifest::read`) |
| `tool`, `tool_version`, `input` | What wrote it, from which file |
| `status` | As in the run report |
| `limits` | The decode limits the file was read under: `max_file_bytes`, `max_stream_bytes`, `max_inflate_output_bytes`, `max_inflate_total_bytes`, `max_inflate_ratio`, `max_scan_bytes`, `max_candidates`, `max_trial_offsets`, `max_record_decode_bytes`, `max_container_records`; `null` is unlimited |
| `outputs` | `{format, file, bytes, sha256}` of every file in the folder, the run report included |
| `report` | The run report's file name (`report.json`), or `null` |

## In the browser: one output at a time, straight to disk

The wasm package's `convert_to_handles(bytes, options, handles, progress)`
(and `convert_to_handles_reader(size, readAt, ...)`, which reads the file by
slices) makes only the outputs named in `handles` and writes each to its
`FileSystemSyncAccessHandle` as soon as it is made, before the next one, so
outputs are never all in memory together. Open the handles in a worker from
the page's private disk (OPFS):

```js
const dir = await navigator.storage.getDirectory();
const open = async (name) => (await dir.getFileHandle(name, { create: true })).createSyncAccessHandle();
const handles = { ifc: await open("model.ifc"), elements_parquet: await open("model.elements.parquet"),
                  manifest: await open("manifest.json") };
const manifest = convert_to_handles(bytes, {}, handles, null);
for (const h of Object.values(handles)) h.close();
```

Output names are those of `convert_all` plus `elements_jsonl`,
`elements_xlsx`, `elements_parquet` and `elements_sqlite` (32-bit build only)
and `manifest`. `element_table(bytes, options, format)` returns one element table
as bytes.
