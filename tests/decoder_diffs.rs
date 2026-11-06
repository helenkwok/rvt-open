//! the names, object types, predefined types and `IsExternal`
//! Revit's own IFC4 export gives, for files whose Revit IFC sits beside them.
//!
//! A renamed type, a renumbered room or an edited element leaves its earlier
//! copy in an older `Partitions/NN` stream, so the copy in the
//! highest-numbered partition is the current one. For each `<name>.rvt` with
//! Revit's `<name>.ifc` or `<name>_Revit.ifc` beside it, matched by
//! `GlobalId`:
//!
//! - an `ObjectType` rvt-open writes is Revit's (`Family:Type`), and a room
//!   with a number or a name is named as Revit names it;
//! - a wall's `PredefinedType` is the wall type's export parameter
//!   (`PARTITIONING`, `SOLIDWALL`, ...) where one is set;
//! - a door's or slab's `IsExternal` is the project parameter
//!   `Pset_<..>Common.IsExternal` where the element or its type sets one;
//! - a duct's `Shape` is its type's true shape on Revit 2024 too, where
//!
//! Runs against `RVT_REVIT2024_SAMPLES_DIR`. Skips what is absent.

use crate::element_names::{decode_step_string, split_args};
use rvt::RevitFile;
use rvt::ifc::{RvtDocExporter, write_step};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// What one IFC element row says about itself.
#[derive(Debug, Clone)]
pub(crate) struct Row {
    pub(crate) entity: String,
    pub(crate) name: String,
    pub(crate) object_type: String,
    /// An `IfcSpace`'s `LongName`; empty for the others.
    pub(crate) long_name: String,
    /// The `PredefinedType` enumerator, where the entity's tail has one.
    pub(crate) predefined: String,
}

fn unquote(raw: &str) -> String {
    if raw == "$" {
        return String::new();
    }
    decode_step_string(
        raw.strip_prefix('\'')
            .and_then(|n| n.strip_suffix('\''))
            .unwrap_or(raw),
    )
}

/// `#id -> (entity, attributes)` for every line of a STEP file.
fn entities(step: &str) -> BTreeMap<u64, (String, Vec<String>)> {
    let mut out = BTreeMap::new();
    for line in step.lines() {
        let Some((id, body)) = line.strip_prefix('#').and_then(|r| r.split_once('=')) else {
            continue;
        };
        let Some((entity, args)) = body.split_once('(') else {
            continue;
        };
        let args = args.trim_end().trim_end_matches(';');
        let args = args.strip_suffix(')').unwrap_or(args);
        if let Ok(id) = id.trim().parse() {
            out.insert(id, (entity.trim().to_string(), split_args(args)));
        }
    }
    out
}

/// `GlobalId` -> row for every element row (Revit's export and rvt-open's give
/// an element the same `GlobalId`).
pub(crate) fn rows(step: &str) -> BTreeMap<String, Row> {
    let mut out = BTreeMap::new();
    for (entity, fields) in entities(step).into_values() {
        if entity.ends_with("TYPE") || entity == "IFCOPENINGELEMENT" {
            continue;
        }
        let Some(global_id) = fields.first().map(|g| g.trim_matches('\'').to_string()) else {
            continue;
        };
        if global_id.len() != 22 || fields.len() < 8 {
            continue;
        }
        let space = entity == "IFCSPACE";
        let at = match entity.as_str() {
            "IFCDOOR" | "IFCWINDOW" => 10,
            "IFCSPACE" => 9,
            _ => 8,
        };
        out.insert(
            global_id,
            Row {
                name: unquote(&fields[2]),
                object_type: unquote(&fields[4]),
                long_name: if space {
                    unquote(&fields[7])
                } else {
                    String::new()
                },
                predefined: fields
                    .get(at)
                    .map(|p| p.trim_matches('.').to_string())
                    .unwrap_or_default(),
                entity,
            },
        );
    }
    out
}

/// `GlobalId` -> (set, property) -> the value as written, for every single
/// and enumerated property of every element.
pub(crate) fn property_values(step: &str) -> BTreeMap<String, BTreeMap<(String, String), String>> {
    let ents = entities(step);
    let refs = |attr: &str| -> Vec<u64> {
        attr.split('#')
            .skip(1)
            .filter_map(|s| {
                s.chars()
                    .take_while(char::is_ascii_digit)
                    .collect::<String>()
                    .parse()
                    .ok()
            })
            .collect()
    };
    let mut out: BTreeMap<String, BTreeMap<(String, String), String>> = BTreeMap::new();
    for (entity, attrs) in ents.values() {
        if entity != "IFCRELDEFINESBYPROPERTIES" || attrs.len() < 6 {
            continue;
        }
        let sets = refs(&attrs[5]);
        let values: Vec<((String, String), String)> = sets
            .iter()
            .filter_map(|set| ents.get(set))
            .flat_map(|(_, pset)| {
                let set_name = unquote(&pset[2]);
                refs(&pset[4])
                    .into_iter()
                    .filter_map(|p| ents.get(&p))
                    .filter(|(e, _)| {
                        matches!(
                            e.as_str(),
                            "IFCPROPERTYSINGLEVALUE" | "IFCPROPERTYENUMERATEDVALUE"
                        )
                    })
                    .map(|(_, a)| ((set_name.clone(), unquote(&a[0])), a[2].clone()))
                    .collect::<Vec<_>>()
            })
            .collect();
        for element in refs(&attrs[4]) {
            let Some(gid) = ents
                .get(&element)
                .and_then(|(_, a)| a.first())
                .map(|g| g.trim_matches('\'').to_string())
            else {
                continue;
            };
            out.entry(gid).or_default().extend(values.iter().cloned());
        }
    }
    out
}

/// A sample model's STEP text from rvt-open's export and from Revit's IFC
/// beside it.
pub(crate) fn export_and_reference(dir: &Path, stem: &str) -> Option<(String, String)> {
    let rvt = dir.join(format!("{stem}.rvt"));
    let reference = [format!("{stem}.ifc"), format!("{stem}_Revit.ifc")]
        .map(|name| dir.join(name))
        .into_iter()
        .find(|path| path.exists());
    let (true, Some(reference)) = (rvt.exists(), reference) else {
        eprintln!(
            "skipping: no {stem} model and Revit export in {}",
            dir.display()
        );
        return None;
    };
    let mut rf = RevitFile::open(&rvt).expect("open");
    let result = RvtDocExporter
        .export_with_diagnostics(&mut rf)
        .expect("export");
    let theirs = std::fs::read_to_string(reference).expect("reference IFC");
    Some((write_step(&result.model), theirs))
}

pub(crate) fn samples() -> Option<PathBuf> {
    let dir = std::env::var_os("RVT_REVIT2024_SAMPLES_DIR").map(PathBuf::from);
    if dir.is_none() {
        eprintln!("skipping: RVT_REVIT2024_SAMPLES_DIR is not set");
    }
    dir
}

/// Housing Concrete, building permit: a Revit 2024 Nordic sample whose
/// types, rooms and parameters were edited while the project was built.
const HOUSING: &str = "ARK_NordicLCA_Housing_Concrete_BuildingPermit";

#[test]
fn object_types_follow_the_newest_name_entry() {
    let Some(dir) = samples() else { return };
    let Some((ours, theirs)) = export_and_reference(&dir, HOUSING) else {
        return;
    };
    let (ours, theirs) = (rows(&ours), rows(&theirs));
    let (mut right, mut wrong) = (0, Vec::new());
    for (id, row) in ours.iter().filter(|(_, r)| !r.object_type.is_empty()) {
        let Some(reference) = theirs.get(id) else {
            continue;
        };
        if row.object_type == reference.object_type {
            right += 1;
        } else {
            wrong.push((&row.entity, &row.object_type, &reference.object_type));
        }
    }
    eprintln!(
        "{HOUSING}: {right} ObjectTypes right, {} wrong",
        wrong.len()
    );
    assert!(wrong.is_empty(), "{:?}", &wrong[..wrong.len().min(5)]);
    // 970 on the base, before the newest copy of a renamed type was read.
    assert!(right >= 1200, "{right} ObjectTypes right");
}

#[test]
fn wall_predefined_types_are_the_types_export_parameter() {
    let Some(dir) = samples() else { return };
    let Some((ours, theirs)) = export_and_reference(&dir, HOUSING) else {
        return;
    };
    let (ours, theirs) = (rows(&ours), rows(&theirs));
    let (mut right, mut wrong) = (0, Vec::new());
    for (id, row) in ours.iter().filter(|(_, r)| r.entity == "IFCWALL") {
        let Some(reference) = theirs.get(id) else {
            continue;
        };
        if row.predefined == "NOTDEFINED" {
            continue;
        }
        if row.predefined == reference.predefined {
            right += 1;
        } else {
            wrong.push((&row.predefined, &reference.predefined));
        }
    }
    eprintln!(
        "{HOUSING}: {right} wall PredefinedTypes right, {} wrong",
        wrong.len()
    );
    assert!(wrong.is_empty(), "{:?}", &wrong[..wrong.len().min(5)]);
    // None on the base, which wrote NOTDEFINED on every wall.
    assert!(right >= 300, "{right} wall PredefinedTypes right");
}

#[test]
fn rooms_are_named_and_numbered_as_revit_names_them() {
    let Some(dir) = samples() else { return };
    let Some((ours, theirs)) =
        export_and_reference(&dir, "ARK_NordicLCA_Housing_Concrete_As-Built")
    else {
        return;
    };
    let (ours, theirs) = (rows(&ours), rows(&theirs));
    let (mut right, mut wrong) = (0, Vec::new());
    for (id, row) in ours.iter().filter(|(_, r)| r.entity == "IFCSPACE") {
        let Some(reference) = theirs.get(id) else {
            continue;
        };
        if (&row.name, &row.long_name) == (&reference.name, &reference.long_name) {
            right += 1;
        } else {
            wrong.push((
                &row.name,
                &row.long_name,
                &reference.name,
                &reference.long_name,
            ));
        }
    }
    eprintln!("{} rooms right, {} wrong", right, wrong.len());
    assert!(wrong.len() <= 2, "{:?}", &wrong[..wrong.len().min(5)]);
    // 0 of 82 on the base, which read neither a renumbered room's number nor
    // a blank one, and named every such room `Room-<ElementId>`.
    assert!(right >= 78, "{right} rooms right");
}

#[test]
fn is_external_follows_the_project_parameter() {
    let Some(dir) = samples() else { return };
    let Some((ours, theirs)) = export_and_reference(&dir, HOUSING) else {
        return;
    };
    let (ours, theirs) = (property_values(&ours), property_values(&theirs));
    let (mut right, mut wrong) = (0, Vec::new());
    for set in ["Pset_DoorCommon", "Pset_SlabCommon"] {
        let key = (set.to_string(), "IsExternal".to_string());
        for (id, values) in &ours {
            let (Some(value), Some(reference)) =
                (values.get(&key), theirs.get(id).and_then(|v| v.get(&key)))
            else {
                continue;
            };
            if value == reference {
                right += 1;
            } else {
                wrong.push((set, value, reference));
            }
        }
    }
    eprintln!("{HOUSING}: {right} IsExternal right, {} wrong", wrong.len());
    assert!(wrong.is_empty(), "{:?}", &wrong[..wrong.len().min(5)]);
    assert!(right >= 80, "{right} IsExternal right");
}

/// true shape on Revit 2024, where Revit's IFC4 export writes `OTHER`.
/// Counts of the rigid ducts (Revit API dumps): (file, rectangular, round).
const DUCT_SHAPES: [(&str, usize, usize); 2] = [
    ("rmebasicsampleproject", 715, 9),
    ("rmeadvancedsampleproject", 719, 9),
];

fn duct_shape_is_the_true_shape_on_revit_2024(k: usize, n: usize) {
    let Some(dir) = samples() else { return };
    for (stem, rect, round) in crate::shard(DUCT_SHAPES, k, n) {
        let Some((ours, theirs)) = export_and_reference(&dir, stem) else {
            return;
        };
        let (ours, theirs) = (property_values(&ours), property_values(&theirs));
        let key = (
            "Pset_DuctSegmentTypeCommon".to_string(),
            "Shape".to_string(),
        );
        let count = |shape: &str| {
            ours.values()
                .filter_map(|v| v.get(&key))
                .filter(|v| v.contains(&format!("'{shape}'")))
                .count()
        };
        let other = ours.values().filter_map(|v| v.get(&key)).count()
            - count("RECTANGULAR")
            - count("ROUND");
        eprintln!(
            "{stem}: {} rectangular, {} round, {other} other",
            count("RECTANGULAR"),
            count("ROUND")
        );
        assert_eq!(
            (count("RECTANGULAR"), count("ROUND")),
            (rect, round),
            "{stem}"
        );
        // Revit's own export still says OTHER on the ducts we call RECTANGULAR/ROUND.
        let revit_other = theirs
            .values()
            .filter_map(|v| v.get(&key))
            .filter(|v| v.contains("'OTHER'"))
            .count();
        assert!(
            revit_other >= rect + round,
            "{stem}: Revit wrote OTHER on {revit_other}"
        );
    }
}

split!(duct_shape_is_the_true_shape_on_revit_2024, duct_shape_is_the_true_shape_on_revit_2024, s0 0, s1 1);

/// Office Concrete, as built: curtain walls whose types frame no name; 124 of
/// them set "Export to IFC As" to `IfcShadingDevice`, which Revit 2024's
/// export does not apply and Revit 2027.1's does.
const OFFICE: &str = "ARK_NordicLCA_Office_Concrete_As-Built";

#[test]
fn curtain_wall_types_are_named_from_their_data_objects() {
    let Some(dir) = samples() else { return };
    let Some((ours, theirs)) = export_and_reference(&dir, OFFICE) else {
        return;
    };
    let (ours, theirs) = (rows(&ours), rows(&theirs));
    let (mut right, mut wrong) = (0, Vec::new());
    // The curtain walls are those Revit's export writes as such: we write some of them as the shading device or wall their "Export
    // to IFC As" names, as Revit 2027.1 does.
    for (id, row) in ours.iter().filter(|(_, r)| !r.object_type.is_empty()) {
        let Some(reference) = theirs.get(id).filter(|r| r.entity == "IFCCURTAINWALL") else {
            continue;
        };
        if row.object_type == reference.object_type {
            right += 1;
        } else {
            wrong.push((&row.object_type, &reference.object_type));
        }
    }
    eprintln!(
        "{OFFICE}: {right} curtain wall ObjectTypes right, {} wrong",
        wrong.len()
    );
    assert!(wrong.is_empty(), "{:?}", &wrong[..wrong.len().min(5)]);
    // None on the base, which read no name from these types.
    assert!(right >= 200, "{right} curtain wall ObjectTypes right");
}

/// Office Timber, as built: 44 basic walls whose "Export to IFC As" names
/// `IfcShadingDevice`, which Revit's export writes as shading devices.
const OFFICE_TIMBER: &str = "ARK_NordicLCA_Office_Timber_As-Built";

/// Our curtain walls on [`OFFICE`] have the class and `PredefinedType` Revit
/// 2027.1's export gives them (`RVT_REVIT2027_IFC_DIR`, `<stem>.ifc`): the
/// curtain walls are those Revit 2024's export writes as `IfcCurtainWall`
/// (220), matched by GlobalId.
///
/// Revit 2027.1 writes the 124 set to `IfcShadingDevice` as shading devices,
/// the 21 set to `IfcWall` as walls and the other 75 as curtain walls, all
/// with their type's `USERDEFINED`. This replaced, before that,
/// `curtain_walls_exported_as_shading_devices_are_notdefined`; its intent
/// (Revit's `PredefinedType` on these elements) is kept here and in
/// [`shading_devices_have_revits_predefined_type`].
#[test]
fn curtain_walls_have_revits_class_and_predefined_type() {
    let Some(dir) = samples() else { return };
    let Some(dir_2027) = std::env::var_os("RVT_REVIT2027_IFC_DIR").map(PathBuf::from) else {
        eprintln!("skipping: RVT_REVIT2027_IFC_DIR is not set");
        return;
    };
    let Ok(answer) = std::fs::read_to_string(dir_2027.join(format!("{OFFICE}.ifc"))) else {
        eprintln!("skipping: no Revit 2027 export of {OFFICE}");
        return;
    };
    let Some((ours, revit_2024)) = export_and_reference(&dir, OFFICE) else {
        return;
    };
    let (ours, revit_2024, answer) = (rows(&ours), rows(&revit_2024), rows(&answer));
    let (mut right, mut wrong) = (0, Vec::new());
    for id in revit_2024
        .iter()
        .filter(|(_, r)| r.entity == "IFCCURTAINWALL")
        .map(|(id, _)| id)
    {
        let (Some(row), Some(reference)) = (ours.get(id), answer.get(id)) else {
            wrong.push((id.clone(), String::new(), String::new()));
            continue;
        };
        if row.entity == reference.entity && row.predefined == reference.predefined {
            right += 1;
        } else {
            wrong.push((id.clone(), row.entity.clone(), reference.entity.clone()));
        }
    }
    eprintln!(
        "{OFFICE}: {right} curtain walls with Revit 2027's class and PredefinedType, {} not",
        wrong.len()
    );
    assert!(wrong.is_empty(), "{:?}", &wrong[..wrong.len().min(5)]);
    // 220 measured; an earlier version wrote all 220 as IfcCurtainWall
    // NOTDEFINED.
    assert_eq!(right, 220, "curtain walls right");
}

/// Shading devices keep Revit's `PredefinedType` (the original intent of
/// `curtain_walls_exported_as_shading_devices_are_notdefined`), on the file
/// whose shading devices Revit's export still writes as such.
#[test]
fn shading_devices_have_revits_predefined_type() {
    let Some(dir) = samples() else { return };
    let Some((ours, theirs)) = export_and_reference(&dir, OFFICE_TIMBER) else {
        return;
    };
    let (ours, theirs) = (rows(&ours), rows(&theirs));
    let (mut right, mut wrong) = (0, Vec::new());
    for (id, row) in ours.iter().filter(|(_, r)| r.entity == "IFCSHADINGDEVICE") {
        let Some(reference) = theirs.get(id) else {
            continue;
        };
        if row.entity == reference.entity && row.predefined == reference.predefined {
            right += 1;
        } else {
            wrong.push((&reference.entity, &row.predefined, &reference.predefined));
        }
    }
    eprintln!(
        "{OFFICE_TIMBER}: {right} shading device PredefinedTypes right, {} wrong",
        wrong.len()
    );
    assert!(wrong.is_empty(), "{:?}", &wrong[..wrong.len().min(5)]);
    // 44 measured, all of the file's shading devices.
    assert!(right >= 44, "{right} shading device PredefinedTypes right");
}

/// Ducts, duct fittings, pipe fittings and air terminals: Revit's export
/// writes `NOTDEFINED` on every one in the 27 public models, and the base
/// wrote no `PredefinedType` for them.
#[test]
fn flow_elements_default_to_notdefined_as_revit_writes_them() {
    let Some(dir) = samples() else { return };
    let Some((ours, theirs)) = export_and_reference(&dir, "rmebasicsampleproject") else {
        return;
    };
    let (ours, theirs) = (rows(&ours), rows(&theirs));
    let classes = [
        "IFCDUCTSEGMENT",
        "IFCDUCTFITTING",
        "IFCPIPEFITTING",
        "IFCAIRTERMINAL",
    ];
    let (mut right, mut wrong) = (0, Vec::new());
    for (id, row) in ours
        .iter()
        .filter(|(_, r)| classes.contains(&r.entity.as_str()))
    {
        let Some(reference) = theirs.get(id).filter(|r| r.entity == row.entity) else {
            continue;
        };
        if row.predefined == reference.predefined {
            right += 1;
        } else {
            wrong.push((&row.entity, &row.predefined, &reference.predefined));
        }
    }
    eprintln!(
        "{right} flow element PredefinedTypes right, {} wrong",
        wrong.len()
    );
    assert!(wrong.is_empty(), "{:?}", &wrong[..wrong.len().min(5)]);
    assert!(right >= 1000, "{right} flow element PredefinedTypes right");
}
