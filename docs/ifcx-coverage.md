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
element or type the IFC4 model already holds gets the parameters it lacked. An
enumeration whose text is not known keeps its integer.
The types no element uses (most come from the project template) are left out
unless `--ifcx-unused-types` is given; CI measures with it, so the unused type
objects below are counted. `--no-ifcx-revit-elements` leaves all of it out;
the IFC4, USD and `.frag` files do not change either way.

## Numbers

Totals over every release (2018 to 2027) of each set. "IFCX before" is
rvt-open's IFCX before the change above; "API" is what the Revit API reports.
Kinds with no element in the models (schedules, sheets, worksets, groups,
links; design options in set 1) are left out.
### Set 1

| Kind | What | API | Revit's IFC4 | rvt-open IFC4 | IFCX before | IFCX after |
|---|---|---:|---:|---:|---:|---:|
| element instances | objects | 240 | 240 (100%) | 240 (100%) | 240 (100%) | 240 (100%) |
| element instances | parameters | 6374 | 188 (3%) | 2442 (38%) | 2442 (38%) | 2442 (38%) |
| type objects | objects | 90 | 82 (91%) | 84 (93%) | 84 (93%) | 90 (100%) |
| type objects | parameters | 1378 | 98 (7%) | 514 (37%) | 514 (37%) | 524 (38%) |
| unused type objects | objects | 44510 | 0 (0%) | 0 (0%) | 0 (0%) | 26205 (59%) |
| levels and grids | objects | 100 | 50 (50%) | 100 (100%) | 100 (100%) | 100 (100%) |
| levels and grids | parameters | 1400 | 50 (4%) | 50 (4%) | 250 (18%) | 250 (18%) |
| site and location | objects | 345 | 0 (0%) | 0 (0%) | 0 (0%) | 0 (0%) |
| site and location | parameters | 1555 | 0 (0%) | 0 (0%) | 0 (0%) | 0 (0%) |
| project info | objects | 50 | 0 (0%) | 0 (0%) | 0 (0%) | 50 (100%) |
| project info | parameters | 450 | 0 (0%) | 0 (0%) | 0 (0%) | 350 (78%) |
| materials | objects | 4745 | 110 (2%) | 4745 (100%) | 0 (0%) | 4745 (100%) |
| materials | parameters | 51480 | 0 (0%) | 0 (0%) | 0 (0%) | 32655 (63%) |
| phases | objects | 100 | 0 (0%) | 0 (0%) | 0 (0%) | 100 (100%) |
| phases | parameters | 400 | 0 (0%) | 0 (0%) | 0 (0%) | 200 (50%) |
| views | objects | 1200 | 0 (0%) | 0 (0%) | 0 (0%) | 1100 (92%) |
| views | parameters | 28985 | 0 (0%) | 0 (0%) | 0 (0%) | 2200 (8%) |
| revisions | objects | 50 | 0 (0%) | 0 (0%) | 0 (0%) | 0 (0%) |
| revisions | parameters | 400 | 0 (0%) | 0 (0%) | 0 (0%) | 0 (0%) |

### Set 2

| Kind | What | API | Revit's IFC4 | rvt-open IFC4 | IFCX before | IFCX after |
|---|---|---:|---:|---:|---:|---:|
| element instances | objects | 1497 | 984 (66%) | 984 (66%) | 970 (65%) | 1327 (89%) |
| element instances | parameters | 29604 | 855 (3%) | 6800 (23%) | 6777 (23%) | 7250 (24%) |
| type objects | objects | 341 | 275 (81%) | 259 (76%) | 259 (76%) | 310 (91%) |
| type objects | parameters | 4906 | 249 (5%) | 1332 (27%) | 1332 (27%) | 1633 (33%) |
| unused type objects | objects | 151553 | 35 (0%) | 20 (0%) | 20 (0%) | 89354 (59%) |
| levels and grids | objects | 400 | 175 (44%) | 350 (88%) | 350 (88%) | 350 (88%) |
| levels and grids | parameters | 5364 | 175 (3%) | 175 (3%) | 875 (16%) | 875 (16%) |
| site and location | objects | 1173 | 0 (0%) | 0 (0%) | 0 (0%) | 0 (0%) |
| site and location | parameters | 5322 | 0 (0%) | 0 (0%) | 0 (0%) | 0 (0%) |
| project info | objects | 170 | 0 (0%) | 0 (0%) | 0 (0%) | 170 (100%) |
| project info | parameters | 1537 | 0 (0%) | 0 (0%) | 0 (0%) | 1190 (77%) |
| materials | objects | 16133 | 250 (2%) | 16133 (100%) | 0 (0%) | 16133 (100%) |
| materials | parameters | 175729 | 0 (0%) | 0 (0%) | 0 (0%) | 111027 (63%) |
| phases | objects | 340 | 0 (0%) | 0 (0%) | 0 (0%) | 340 (100%) |
| phases | parameters | 1374 | 0 (0%) | 0 (0%) | 0 (0%) | 680 (49%) |
| design options | objects | 14 | 0 (0%) | 0 (0%) | 0 (0%) | 14 (100%) |
| design options | parameters | 70 | 0 (0%) | 0 (0%) | 0 (0%) | 28 (40%) |
| views | objects | 4105 | 0 (0%) | 0 (0%) | 0 (0%) | 3765 (92%) |
| views | parameters | 99648 | 0 (0%) | 0 (0%) | 0 (0%) | 7530 (8%) |
| revisions | objects | 170 | 0 (0%) | 0 (0%) | 0 (0%) | 0 (0%) |
| revisions | parameters | 1367 | 0 (0%) | 0 (0%) | 0 (0%) | 0 (0%) |

### Set 1 by release

| Release | What | API | Revit's IFC4 | rvt-open IFC4 | IFCX before | IFCX after |
|---|---|---:|---:|---:|---:|---:|
| 2018 | objects | 608 | 42 (7%) | 438 (72%) | 43 (7%) | 563 (93%) |
| 2018 | parameters | 7936 | 0 (0%) | 279 (4%) | 299 (4%) | 3164 (40%) |
| 2019 | objects | 608 | 42 (7%) | 438 (72%) | 43 (7%) | 563 (93%) |
| 2019 | parameters | 7936 | 0 (0%) | 279 (4%) | 299 (4%) | 3164 (40%) |
| 2020 | objects | 693 | 41 (6%) | 509 (73%) | 39 (6%) | 638 (92%) |
| 2020 | parameters | 8573 | 2 (0%) | 221 (3%) | 241 (3%) | 3481 (41%) |
| 2021 | objects | 688 | 43 (6%) | 513 (75%) | 43 (6%) | 638 (93%) |
| 2021 | parameters | 8566 | 21 (0%) | 286 (3%) | 306 (4%) | 3546 (41%) |
| 2022 | objects | 693 | 44 (6%) | 518 (75%) | 43 (6%) | 643 (93%) |
| 2022 | parameters | 8606 | 21 (0%) | 286 (3%) | 306 (4%) | 3571 (41%) |
| 2023 | objects | 698 | 54 (8%) | 521 (75%) | 41 (6%) | 648 (93%) |
| 2023 | parameters | 9674 | 58 (1%) | 303 (3%) | 323 (3%) | 4095 (42%) |
| 2024 | objects | 733 | 54 (7%) | 558 (76%) | 43 (6%) | 683 (93%) |
| 2024 | parameters | 10109 | 58 (1%) | 371 (4%) | 391 (4%) | 4433 (44%) |
| 2025 | objects | 733 | 54 (7%) | 558 (76%) | 43 (6%) | 683 (93%) |
| 2025 | parameters | 10264 | 58 (1%) | 327 (3%) | 347 (3%) | 4389 (43%) |
| 2026 | objects | 733 | 54 (7%) | 558 (76%) | 43 (6%) | 683 (93%) |
| 2026 | parameters | 10370 | 58 (1%) | 327 (3%) | 347 (3%) | 4389 (42%) |
| 2027 | objects | 733 | 54 (7%) | 558 (76%) | 43 (6%) | 683 (93%) |
| 2027 | parameters | 10388 | 60 (1%) | 327 (3%) | 347 (3%) | 4389 (42%) |

### Set 2 by release

| Release | What | API | Revit's IFC4 | rvt-open IFC4 | IFCX before | IFCX after |
|---|---|---:|---:|---:|---:|---:|
| 2018 | objects | 2159 | 142 (7%) | 1500 (69%) | 156 (7%) | 1972 (91%) |
| 2018 | parameters | 28000 | 2 (0%) | 679 (2%) | 749 (3%) | 10530 (38%) |
| 2019 | objects | 2159 | 141 (7%) | 1500 (69%) | 156 (7%) | 1972 (91%) |
| 2019 | parameters | 27998 | 2 (0%) | 679 (2%) | 749 (3%) | 10530 (38%) |
| 2020 | objects | 2448 | 144 (6%) | 1748 (71%) | 149 (6%) | 2227 (91%) |
| 2020 | parameters | 30171 | 13 (0%) | 587 (2%) | 657 (2%) | 11715 (39%) |
| 2021 | objects | 2436 | 152 (6%) | 1755 (72%) | 156 (6%) | 2232 (92%) |
| 2021 | parameters | 30296 | 68 (0%) | 685 (2%) | 755 (2%) | 11826 (39%) |
| 2022 | objects | 2453 | 153 (6%) | 1772 (72%) | 156 (6%) | 2249 (92%) |
| 2022 | parameters | 30432 | 80 (0%) | 685 (2%) | 755 (2%) | 11938 (39%) |
| 2023 | objects | 2442 | 188 (8%) | 1790 (73%) | 157 (6%) | 2256 (92%) |
| 2023 | parameters | 33923 | 223 (1%) | 888 (3%) | 958 (3%) | 13893 (41%) |
| 2024 | objects | 2562 | 191 (7%) | 1917 (75%) | 164 (6%) | 2377 (93%) |
| 2024 | parameters | 35441 | 225 (1%) | 1195 (3%) | 1251 (4%) | 15107 (43%) |
| 2025 | objects | 2562 | 191 (7%) | 1916 (75%) | 163 (6%) | 2376 (93%) |
| 2025 | parameters | 35976 | 225 (1%) | 1009 (3%) | 1070 (3%) | 14926 (41%) |
| 2026 | objects | 2561 | 191 (7%) | 1914 (75%) | 161 (6%) | 2374 (93%) |
| 2026 | parameters | 36326 | 217 (1%) | 950 (3%) | 1020 (3%) | 14875 (41%) |
| 2027 | objects | 2561 | 191 (7%) | 1914 (75%) | 161 (6%) | 2374 (93%) |
| 2027 | parameters | 36358 | 224 (1%) | 950 (3%) | 1020 (3%) | 14873 (41%) |

The release tables sum every kind but the unused type objects.

## What is left

- **Revit's own export drops** nearly every parameter (it holds 3% of the
  element parameters and 5 to 7% of the type parameters), all the phases,
  design options, views, project information and revisions, the materials no
  element uses, and a third of set 2's placed elements: in the 2019 and 2024
  saves those are balusters, the analytical model (nodes, columns, beams),
  treads and risers, top rails, and the wall and rooms of the phase and option
  recipes that its export phase and primary option leave out. IFCX holds all
  the materials, phases, design options and project information, 92% of the
  views, 59% of the unused types (with `--ifcx-unused-types`), 89% of set 2's placed elements (balusters
  and the left-out wall and rooms included), and 13 and 8 times as many element
  parameters as Revit's export (sets 1 and 2).
- **Neither carries** the revisions and the survey and project base points as
  objects; treads and risers, top rails and the analytical model; most view
  parameters (92% of the views are there, with 8% of their parameters: scale,
  phase, crop, detail level and the like are fields of the view, not saved
  parameters); the parameters Revit computes rather than saves (an element's
  area and volume, a wall's structural usage, its attachment and room-bounding
  flags); the `Design Option` value `Main Model` of an element in no option;
  and the category of a node only IFCX holds.
