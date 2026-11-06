# IFC comparison tooling (`rvt-ifc-compare`)


Compare an `rvt-open` IFC4 STEP export against a reference IFC (typically a
Revit IFC export of the same model). The tool is a lightweight STEP
summarizer — not a full EXPRESS validator and not a geometry mesh diff.

## Usage

```bash
# Human summary on stdout
./target/release/rvt-ifc-compare rvt-export.ifc revit-reference.ifc

# JSON report for CI / notebooks
./target/release/rvt-ifc-compare rvt-export.ifc revit-reference.ifc \
  --json /tmp/ifc-compare.json

# Fail with exit code 2 when structural deltas exist
./target/release/rvt-ifc-compare a.ifc b.ifc --fail-on-diff
```

Dimensions compared:

| Dimension | Source |
|---|---|
| Entity-type counts | DATA-section `TYPE(...)` histogram |
| Storeys | `IfcBuildingStorey` Name + elevation, in metres by each file's own `LENGTHUNIT` |
| Bounding box | Axis-aligned extents of `IfcCartesianPoint`, in metres the same way (local coordinates, not placed) |
| Objects | Product entities (`IfcWall`, `IfcDoor`, …) Name |
| Materials | Distinct `IfcMaterial` names |
| Property keys | Distinct `IfcPropertySingleValue` Name values |

## Known divergences

When a delta touches a type whose recovery still has a known gap, the
JSON/human report attaches a note naming what is recovered and the open
issue that tracks the remainder:

| IFC type | Recovered | Remaining gap tracked in |
|---|---|---|
| `IFCWALL` | Revit 2024 set exact; run cut back by the joins the record names, the L corners its join lists leave open decided by its joined-wall lists: 360 of 360 world-exact | (2024 ArcWall records) |
| `IFCDOOR` | Revit 2024 set and host binding exact | (opening cut from the wall curve) |
| `IFCSLAB` | Revit 2024 set with sketch-line profiles, 80 of 80; all 80 storey-bound, as are the 20 IFCSHADINGDEVICE plates | nothing slab-specific; plate containment is closed |
| `IFCSPACE` | Revit 2024 room set 116 of 116 with real names, numbers and storeys; body is the record box, equal to the reference envelope on all 116 | (boundary polygon: the record carries no sketch, measured negative) |
| `IFCWINDOW` | Revit 2024 set and host binding exact; all 6 storey-bound | nothing window-specific; window containment is closed |
| `IFCMATERIALLAYERSETUSAGE` | 403, on the floors, shading devices and walls whose types have layers. The walls' single layers take their category's material, "Default Wall" from the document's object styles. The reference export is a `ReferenceView_V1.2` file with no `IfcMaterialLayerSet`, so the sets are scored against the faces its bodies style with each material: 403 of 403 within 0.001 ft | (elements drawn whole get no set) |
| `IFCOPENINGELEMENT` | opening, void and fill chain per host | (true opening cut) |

The table lists exactly the types `catalogued_divergence_notes` in
`src/ifc/compare.rs` carries a note for; `IFCCOLUMN` has no note because
it has no remaining gap — its body is the record prism minus
the walls it names, exact on 256 of 256 against Revit's own export.

**The shipped note strings lag this table.** Those constants still quote
the earlier figures — "31 over-trimmed wall ends" for `IFCWALL`, where
9 remain — and still name an open tracker for `IFCSLAB`
and `IFCWINDOW` storey containment, which is now closed for both.
The table above is the measured state; refreshing the constants is a code
change, not a docs one.

Scaffold-only `rvt-ifc` exports (no typed products) will report large
object/entity gaps versus real Revit IFCs — that is expected until the
linked recall work lands, not a bug in the compare tool.

## Library API

`rvt::ifc::compare::{summarize_ifc_step, compare_summaries, format_human_report}`.
