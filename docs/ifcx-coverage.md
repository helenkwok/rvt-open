# IFCX coverage of what Revit holds

rvt-open's IFCX file is meant to carry what an IFC4 export of the same Revit
file leaves out. This page says how that is measured and gives the numbers.

## The measure

Each tiny reference model (`fixtures-tiny-1` and `fixtures-tiny-2`, fetched by
`tools/ci/fetch-tiny-models.sh`; 22 recipes saved in every release from 2018
to 2027) comes with `<recipe>.api-dump.json`: every element the Revit API
returns for the file, with all its parameters, read in the release that saved
it. [`tools/ifcx-coverage/coverage.py`](../tools/ifcx-coverage/coverage.py)
sorts those elements into kinds and counts, per kind, how many objects and how
many parameter values an export carries.

| Kind | API elements |
|---|---|
| element instances | placed elements of a model category, not view-specific, with a box or a location (walls, floors, doors, rooms, stair runs, mullions, ...); drawing aids, sketches and markers are left out |
| type objects | types an element instance is typed by |
| unused type objects | every other type (most come from the template); their parameters are not counted |
| levels and grids, site and location, project info, materials, phases, design options, views, schedules, sheets, revisions, worksets, groups, links | by API class |

- **An object is carried** when the export holds an object with its ElementId
  (IFC `Tag`, IFCX `ifc-src::attr::Tag`) or one of its GlobalIds (its
  `IfcGUID` parameter, or the GlobalId Revit's exporter derives from its
  UniqueId; in IFCX that GlobalId as the node's path). A material also counts
  when an IFC4 file holds an `IfcMaterial` of its name.
- **A parameter is carried** when its object is carried and holds a property of
  the same name (case, spaces and punctuation ignored) whose value is the
  parameter's, in any unit Revit or IFC may write it in (feet, metres,
  millimetres, square and cubic metres, degrees, or the number the API
  displays); an ElementId by the name it displays; a Yes/No as a boolean. An
  object also holds its type's properties (IFC4 `IsTypedBy`, IFCX
  `inherits`), and a type's parameter may sit on its elements where the export
  has no type object. An `IfcGUID` parameter is carried as the object's
  GlobalId.
- A property under another name than the parameter's (a standard property set's
  `LoadBearing` for Revit's `Structural`) is not counted, so the measure is a
  lower bound for every export alike.

Both public assets hold the dumps, so CI measures both sets. CI runs it in the "tiny reference models" job on rvt-open's IFC4 and IFCX of
every model that has a dump, and fails when an IFCX total (per set, kind, objects or
parameters) drops below [`tools/ifcx-coverage/floor.tsv`](../tools/ifcx-coverage/floor.tsv),
or below IFC4's. Revit's own IFC4 Reference View exports of the same models are
not public; the column below was measured once with `--revit-ifc`.

```bash
tools/ci/fetch-tiny-models.sh _tiny
python3 tools/ifcx-coverage/coverage.py _tiny out --bin target/ci --rvt-ifc-arg=--ifcx-unused-types --floor tools/ifcx-coverage/floor.tsv
```

## What IFCX carries now

`rvt-ifc --ifcx` writes, beside the building elements, their types and storeys
(which the IFC4 file holds too), every other element the file saves parameters
for: a node whose path is the GlobalId Revit's exporter gives it, with its
ElementId, its record's class (`rvtopen::revit::Class`), its name and its own
parameters under `rvtopen::revit::<Group>::<Name>`, all under one
`RevitElements` node of the project. A phase also gets its order
(`Sequence Number`), a design option its set (`Design Option Set Id`). An
element or type the IFC4 model already holds gets the parameters it lacked.

What Revit's own export leaves out of a parameter it saves is written as saved:
a parameter of the IFC group (`Export to IFC`), an enumeration whose text is
not known as its integer, a real of a spec the export does not list as its
number in Revit's internal units, and an ElementId that names nothing with a
name (no design option, no phase demolished, no template) as its id, `-1`
where it names nothing, as the API shows it.

Beside its parameters, every element IFCX holds gets what its record saves
outside them, as the parameters the API shows (read with our own probes of
the records of the tiny models, the API dumps the answer key):

- its `Category` and `Design Option`, from the header every record has;
- the type its record names (a view's view type, a level's or grid's type, a
  project location's site location) as `Type Id`, `Type` (the type's name),
  `Family` and `Family and Type` (by the type's ElementId: a system type does
  not save its family's name);
- a view: `View Name`, `View Scale` and `Scale Value 1:`, `View Template`,
  `Parts Visibility`, `Display Model`, `Wall Join Display`, `Underlay
  Orientation`, `Range: Base Level` and `Range: Top Level`, `Default Analysis
  Display Style`, its sketch plane, `Orientation`, `Title on Sheet`, `Show
  Hidden Lines`, `Color Scheme Location`, a plan's `Associated Level`, and its
  viewer's `Annotation Crop` and `Scope Box`;
- a level: `Building Story`, `Computation Height`; a level or grid: `Name`;
- a wall: `Room Bounding`, `Cross-Section`, `Structural`, `Location Line`;
- a revision, a node of its own: `Revision Number`, `Revision Sequence`,
  `Revision Date`, `Revision Description`, `Issued`, `Issued by`, `Issued to`
  and (to Revit 2023) `Numbering`;
- the survey point and project base point, nodes of their own: `E/W`, `N/S`
  and `Elev` in shared coordinates and, the project base point, `Angle to True
  North`, as the georeference reads them; project locations (with their name),
  site locations and the internal origin are nodes too.

A wall also gets Revit's `Volume` and `Area`, which Revit computes and does
not save, from the net volume and net side area of the IFC4 model (the same
values on 451 of the 470 measured walls; other categories are left out, their
net volumes not Revit's).
The types no element uses (most come from the project template) are left out
unless `--ifcx-unused-types` is given; CI measures with it, so the unused type
objects below are counted. `--no-ifcx-revit-elements` leaves all of it out;
the IFC4, USD and `.frag` files do not change either way.

## Numbers

Totals over every release (2018 to 2027) of each set. "IFCX before" is
rvt-open's IFCX before the record fields, categories, design options, site
elements and revisions above were added (main of 2026-11-14), "IFCX after" with
them; "API" is what the Revit API reports. The IFC4 file is byte for byte the
same before and after.
Kinds with no element in the models (schedules, sheets, worksets, groups,
links; design options in set 1) are left out.
### Set 1

| Kind | What | API | Revit's IFC4 | rvt-open IFC4 | IFCX before | IFCX after |
|---|---|---:|---:|---:|---:|---:|
| element instances | objects | 240 | 240 (100%) | 240 (100%) | 240 (100%) | 240 (100%) |
| element instances | parameters | 6374 | 188 (3%) | 2726 (43%) | 2726 (43%) | 4264 (67%) |
| type objects | objects | 90 | 82 (91%) | 86 (96%) | 90 (100%) | 90 (100%) |
| type objects | parameters | 1378 | 98 (7%) | 520 (38%) | 528 (38%) | 633 (46%) |
| unused type objects | objects | 44510 | 0 (0%) | 0 (0%) | 26205 (59%) | 31045 (70%) |
| levels and grids | objects | 100 | 50 (50%) | 100 (100%) | 100 (100%) | 100 (100%) |
| levels and grids | parameters | 1400 | 50 (4%) | 50 (4%) | 250 (18%) | 1150 (82%) |
| site and location | objects | 345 | 0 (0%) | 0 (0%) | 0 (0%) | 345 (100%) |
| site and location | parameters | 1555 | 0 (0%) | 0 (0%) | 0 (0%) | 1390 (89%) |
| project info | objects | 50 | 0 (0%) | 0 (0%) | 50 (100%) | 50 (100%) |
| project info | parameters | 450 | 0 (0%) | 0 (0%) | 350 (78%) | 450 (100%) |
| materials | objects | 4745 | 110 (2%) | 4745 (100%) | 4745 (100%) | 4745 (100%) |
| materials | parameters | 51480 | 0 (0%) | 0 (0%) | 32655 (63%) | 42145 (82%) |
| phases | objects | 100 | 0 (0%) | 0 (0%) | 100 (100%) | 100 (100%) |
| phases | parameters | 400 | 0 (0%) | 0 (0%) | 200 (50%) | 400 (100%) |
| views | objects | 1200 | 0 (0%) | 0 (0%) | 1100 (92%) | 1200 (100%) |
| views | parameters | 28985 | 0 (0%) | 0 (0%) | 2200 (8%) | 19700 (68%) |
| revisions | objects | 50 | 0 (0%) | 0 (0%) | 0 (0%) | 50 (100%) |
| revisions | parameters | 400 | 0 (0%) | 0 (0%) | 0 (0%) | 370 (92%) |

### Set 2

| Kind | What | API | Revit's IFC4 | rvt-open IFC4 | IFCX before | IFCX after |
|---|---|---:|---:|---:|---:|---:|
| element instances | objects | 1497 | 984 (66%) | 990 (66%) | 1327 (89%) | 1327 (89%) |
| element instances | parameters | 29604 | 855 (3%) | 8475 (29%) | 8907 (30%) | 13689 (46%) |
| type objects | objects | 341 | 275 (81%) | 273 (80%) | 312 (91%) | 312 (91%) |
| type objects | parameters | 4906 | 249 (5%) | 1382 (28%) | 1678 (34%) | 2016 (41%) |
| unused type objects | objects | 151553 | 35 (0%) | 20 (0%) | 89354 (59%) | 105810 (70%) |
| levels and grids | objects | 400 | 175 (44%) | 350 (88%) | 350 (88%) | 400 (100%) |
| levels and grids | parameters | 5364 | 175 (3%) | 175 (3%) | 875 (16%) | 4450 (83%) |
| site and location | objects | 1173 | 0 (0%) | 0 (0%) | 0 (0%) | 1173 (100%) |
| site and location | parameters | 5322 | 0 (0%) | 0 (0%) | 0 (0%) | 4726 (89%) |
| project info | objects | 170 | 0 (0%) | 0 (0%) | 170 (100%) | 170 (100%) |
| project info | parameters | 1537 | 0 (0%) | 0 (0%) | 1190 (77%) | 1530 (100%) |
| materials | objects | 16133 | 250 (2%) | 16133 (100%) | 16133 (100%) | 16133 (100%) |
| materials | parameters | 175729 | 0 (0%) | 0 (0%) | 111027 (63%) | 143293 (82%) |
| phases | objects | 340 | 0 (0%) | 0 (0%) | 340 (100%) | 340 (100%) |
| phases | parameters | 1374 | 0 (0%) | 0 (0%) | 680 (49%) | 1360 (99%) |
| design options | objects | 14 | 0 (0%) | 0 (0%) | 14 (100%) | 14 (100%) |
| design options | parameters | 70 | 0 (0%) | 0 (0%) | 28 (40%) | 56 (80%) |
| views | objects | 4105 | 0 (0%) | 0 (0%) | 3765 (92%) | 4105 (100%) |
| views | parameters | 99648 | 0 (0%) | 0 (0%) | 7530 (8%) | 67685 (68%) |
| revisions | objects | 170 | 0 (0%) | 0 (0%) | 0 (0%) | 170 (100%) |
| revisions | parameters | 1367 | 0 (0%) | 0 (0%) | 0 (0%) | 1258 (92%) |

### Set 1 by release

| Release | What | API | Revit's IFC4 | rvt-open IFC4 | IFCX before | IFCX after |
|---|---|---:|---:|---:|---:|---:|
| 2018 | objects | 608 | 42 (7%) | 438 (72%) | 563 (93%) | 608 (100%) |
| 2018 | parameters | 7936 | 0 (0%) | 323 (4%) | 3208 (40%) | 6183 (78%) |
| 2019 | objects | 608 | 42 (7%) | 438 (72%) | 563 (93%) | 608 (100%) |
| 2019 | parameters | 7936 | 0 (0%) | 323 (4%) | 3208 (40%) | 6183 (78%) |
| 2020 | objects | 693 | 41 (6%) | 509 (73%) | 638 (92%) | 693 (100%) |
| 2020 | parameters | 8573 | 2 (0%) | 265 (3%) | 3525 (41%) | 6651 (78%) |
| 2021 | objects | 688 | 43 (6%) | 513 (75%) | 638 (93%) | 688 (100%) |
| 2021 | parameters | 8566 | 21 (0%) | 330 (4%) | 3590 (42%) | 6733 (79%) |
| 2022 | objects | 693 | 44 (6%) | 518 (75%) | 643 (93%) | 693 (100%) |
| 2022 | parameters | 8606 | 21 (0%) | 330 (4%) | 3615 (42%) | 6778 (79%) |
| 2023 | objects | 698 | 54 (8%) | 523 (75%) | 648 (93%) | 698 (100%) |
| 2023 | parameters | 9674 | 58 (1%) | 373 (4%) | 4163 (43%) | 7336 (76%) |
| 2024 | objects | 733 | 54 (7%) | 558 (76%) | 683 (93%) | 733 (100%) |
| 2024 | parameters | 10109 | 58 (1%) | 371 (4%) | 4433 (44%) | 7676 (76%) |
| 2025 | objects | 733 | 54 (7%) | 558 (76%) | 683 (93%) | 733 (100%) |
| 2025 | parameters | 10264 | 58 (1%) | 327 (3%) | 4389 (43%) | 7654 (75%) |
| 2026 | objects | 733 | 54 (7%) | 558 (76%) | 683 (93%) | 733 (100%) |
| 2026 | parameters | 10370 | 58 (1%) | 327 (3%) | 4389 (42%) | 7654 (74%) |
| 2027 | objects | 733 | 54 (7%) | 558 (76%) | 683 (93%) | 733 (100%) |
| 2027 | parameters | 10388 | 60 (1%) | 327 (3%) | 4389 (42%) | 7654 (74%) |

### Set 2 by release

| Release | What | API | Revit's IFC4 | rvt-open IFC4 | IFCX before | IFCX after |
|---|---|---:|---:|---:|---:|---:|
| 2018 | objects | 2159 | 142 (7%) | 1504 (70%) | 1972 (91%) | 2130 (99%) |
| 2018 | parameters | 28000 | 2 (0%) | 928 (3%) | 10779 (38%) | 20968 (75%) |
| 2019 | objects | 2159 | 141 (7%) | 1504 (70%) | 1972 (91%) | 2130 (99%) |
| 2019 | parameters | 27998 | 2 (0%) | 928 (3%) | 10779 (38%) | 20968 (75%) |
| 2020 | objects | 2448 | 144 (6%) | 1752 (72%) | 2227 (91%) | 2419 (99%) |
| 2020 | parameters | 30171 | 13 (0%) | 842 (3%) | 11970 (40%) | 22699 (75%) |
| 2021 | objects | 2436 | 152 (6%) | 1759 (72%) | 2232 (92%) | 2407 (99%) |
| 2021 | parameters | 30296 | 68 (0%) | 939 (3%) | 12080 (40%) | 22843 (75%) |
| 2022 | objects | 2453 | 153 (6%) | 1776 (72%) | 2249 (92%) | 2424 (99%) |
| 2022 | parameters | 30432 | 80 (0%) | 940 (3%) | 12193 (40%) | 23025 (76%) |
| 2023 | objects | 2442 | 188 (8%) | 1794 (73%) | 2256 (92%) | 2431 (100%) |
| 2023 | parameters | 33923 | 223 (1%) | 1121 (3%) | 14123 (42%) | 24974 (74%) |
| 2024 | objects | 2562 | 191 (7%) | 1915 (75%) | 2377 (93%) | 2552 (100%) |
| 2024 | parameters | 35441 | 225 (1%) | 1227 (3%) | 15139 (43%) | 26229 (74%) |
| 2025 | objects | 2562 | 191 (7%) | 1914 (75%) | 2376 (93%) | 2551 (100%) |
| 2025 | parameters | 35976 | 225 (1%) | 1041 (3%) | 14958 (42%) | 26139 (73%) |
| 2026 | objects | 2561 | 191 (7%) | 1914 (75%) | 2375 (93%) | 2550 (100%) |
| 2026 | parameters | 36326 | 217 (1%) | 1033 (3%) | 14948 (41%) | 26110 (72%) |
| 2027 | objects | 2561 | 191 (7%) | 1914 (75%) | 2375 (93%) | 2550 (100%) |
| 2027 | parameters | 36358 | 224 (1%) | 1033 (3%) | 14946 (41%) | 26108 (72%) |

The release tables sum every kind but the unused type objects.

## What is left

- **Revit's own export drops** nearly every parameter (it holds 3% of the
  element parameters and 5 to 7% of the type parameters), all the phases,
  design options, views, project information, revisions, base points and
  locations, the materials no element uses, and a third of set 2's placed
  elements: in the 2019 and 2024 saves those are balusters, the analytical
  model (nodes, columns, beams), treads and risers, top rails, and the wall and
  rooms of the phase and option recipes that its export phase and primary
  option leave out. IFCX holds every object of every kind but those placed
  elements (89% of set 2's: balusters and the left-out wall and rooms
  included) and the unused types (70%, with `--ifcx-unused-types`), with 46%
  to 100% of their parameters: 22 and 16 times as many element parameters as
  Revit's export (sets 1 and 2), 82% of the level and grid parameters, 89% of
  the site and location parameters, 68% of the view parameters and 92% of the
  revision parameters.
- **Not carried yet:** treads and risers, top rails and the analytical model;
  what Revit computes rather than saves (the area and volume of elements other
  than walls, a wall's attachment flags, Related to Mass, a view's sheet and
  detail numbers and Dependency, the view family names); values the file
  leaves at their default and does not save (a level's Story Above and
  Structural, Export to IFC `By Type`, a wall's Top Constraint `Unconnected`);
  the fields of type records (fill patterns, thermal and analytical values,
  Function, wrapping); a view's Sun Path, depth and far clipping and crop
  region (every crop field is off in the tiny models, so no value can be
  checked); an elevation view's type, which its marker holds; a revision's
  Numbering from Revit 2024 (a numbering sequence element replaces it); the
  name a site location shows when it saves none (`Location Data`); and the
  category of a base point as Revit before 2023 names it (`Survey Point`, not
  `Site: Survey Point`).
