//! three places where rvt-open (or Revit's export) pointed
//! at a class rvt-open got wrong, each re-measured on our files.
//!
//! 1. "Export to IFC As" applies as Revit 2027.1's exporter applies it:
//!    to curtain walls and wall sweeps too, not to a ceiling
//!    naming anything but `IfcCovering` (whose predefined type still
//!    applies), nor to a floor naming `IfcFurniture`. Checked by Tag and by
//!    GlobalId against Revit 2027.1's IFC4 export of the 12 Nordic LCA models
//!    (`RVT_REVIT2027_IFC_DIR`, `<stem>.ifc`; the models are in
//!    against Revit 2024's IFC of DACH (`DACHsampleproject.ifc`).
//! 2. A door or window whose type the value-block scan reads as drawing no
//!    geometry, but whose native `m_geomTag2MaterialId` maps geometry, is a
//!    door or window, not its opening alone: by Tag against Revit's IFC of
//!    and 2026 (`RVT_REVIT_EXTRA_DIR`).
//!
//! Each check skips the files that are absent.

use super::truss_framing::{entities, text};
use rvt::RevitFile;
use rvt::ifc::{RvtDocExporter, write_step};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Each element's classes by `Tag`, openings and type objects left out.
fn classes(step: &str) -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (class, a) in entities(step).into_values() {
        if class == "IFCOPENINGELEMENT" || class.ends_with("TYPE") {
            continue;
        }
        if let Some(tag) = a.get(7).and_then(|t| text(t))
            && !tag.is_empty()
            && tag.bytes().all(|b| b.is_ascii_digit())
        {
            out.entry(tag).or_default().insert(class);
        }
    }
    out
}

fn export(path: &Path) -> String {
    let mut rf = RevitFile::open(path).expect("open");
    let model = RvtDocExporter
        .export_with_diagnostics(&mut rf)
        .expect("export")
        .model;
    write_step(&model)
}

fn samples() -> Option<PathBuf> {
    let dir = std::env::var_os("RVT_REVIT2024_SAMPLES_DIR").map(PathBuf::from);
    if dir.is_none() {
        eprintln!("skipping: RVT_REVIT2024_SAMPLES_DIR is not set");
    }
    dir
}

/// Revit 2024 models with Revit's own IFC: the stem, the IFC's file name
/// (`2027:` = Revit 2027.1's IFC4 export in `RVT_REVIT2027_IFC_DIR`, else
/// Revit 2024's beside the model), and three exact pins measured on a
/// development branch: the elements (by Tag, written by both) whose classes differ
/// from Revit's (Office concrete 121, timber 119, Housing 22 and 21, Housing
/// terrain 52 and 53, Office terrain 17, DACH 65); Revit's doors and windows we
/// write no element of (Office concrete 12, timber 13, Housing 7, others 0);
/// their family names another entity, and the MEP categories it adds export
/// in Revit's classes, so far fewer classes differ (Office concrete 4 and 10,
/// timber 4 and 12, Housing 8 and 7, Housing terrain 27 and 28, Office
/// terrain 6, DACH 34); and 10 office plumbing fixtures Revit writes as
/// `IfcSanitaryTerminal` with no export-as or predefined type rvt-open reads
/// are now `IfcFlowTerminal` (Office concrete 2 + 3, timber 1 + 4 of the
/// `CHECKED` counts).
/// and the elements of the `CHECKED` classes whose class differs (Housing
/// concrete 6, timber 5: roofing coverings and specialty-equipment doors that
/// stay proxies, and roofs Revit splits into `IfcRoof` and `IfcSlab`; each
/// office 1 door Revit writes for a curtain panel, which we write `IfcPlate`,
/// timber offices 1 more roof; Housing terrain timber 1 roof; DACH 34: 16
/// curtain walls Revit writes for curtain panels, which we write `IfcWall`, 15
/// of Revit's slabs, which we write `IfcFooting`, and 3 roofs).
/// Since the door-type fixes (a type's native map with entries is not an
/// opening alone, and the group budget follows the file's limits), the four
/// housing files write every door and window Revit's export holds (7 absent
/// before); DACH has one more element whose class differs from Revit 2024's
/// export since gutters are recovered.
const WITH_REVIT_IFC: &[(&str, &str, usize, usize, usize)] = &[
    (
        "ARK_NordicLCA_Housing_Concrete_As-Built",
        "2027:ARK_NordicLCA_Housing_Concrete_As-Built.ifc",
        4,
        0,
        2,
    ),
    (
        "ARK_NordicLCA_Housing_Concrete_BuildingPermit",
        "2027:ARK_NordicLCA_Housing_Concrete_BuildingPermit.ifc",
        4,
        0,
        2,
    ),
    (
        "ARK_NordicLCA_Housing_Terrain-Concrete_BuildingPermit",
        "2027:ARK_NordicLCA_Housing_Terrain-Concrete_BuildingPermit.ifc",
        18,
        0,
        0,
    ),
    (
        "ARK_NordicLCA_Housing_Terrain-Timber_BuildingPermit",
        "2027:ARK_NordicLCA_Housing_Terrain-Timber_BuildingPermit.ifc",
        19,
        0,
        1,
    ),
    (
        "ARK_NordicLCA_Housing_Timber_As-Built",
        "2027:ARK_NordicLCA_Housing_Timber_As-Built.ifc",
        3,
        0,
        1,
    ),
    (
        "ARK_NordicLCA_Housing_Timber_BuildingPermit",
        "2027:ARK_NordicLCA_Housing_Timber_BuildingPermit.ifc",
        3,
        0,
        1,
    ),
    (
        "ARK_NordicLCA_Office_Concrete_As-Built",
        "2027:ARK_NordicLCA_Office_Concrete_As-Built.ifc",
        1,
        12,
        0,
    ),
    (
        "ARK_NordicLCA_Office_Concrete_BuildingPermit",
        "2027:ARK_NordicLCA_Office_Concrete_BuildingPermit.ifc",
        1,
        12,
        0,
    ),
    (
        "ARK_NordicLCA_Office_Terrain_Concrete_BuildingPermit",
        "2027:ARK_NordicLCA_Office_Terrain_Concrete_BuildingPermit.ifc",
        5,
        0,
        0,
    ),
    (
        "ARK_NordicLCA_Office_Terrain_Timber_BuildingPermit",
        "2027:ARK_NordicLCA_Office_Terrain_Timber_BuildingPermit.ifc",
        5,
        0,
        0,
    ),
    (
        "ARK_NordicLCA_Office_Timber_As-Built",
        "2027:ARK_NordicLCA_Office_Timber_As-Built.ifc",
        2,
        13,
        1,
    ),
    (
        "ARK_NordicLCA_Office_Timber_BuildingPermit",
        "2027:ARK_NordicLCA_Office_Timber_BuildingPermit.ifc",
        2,
        13,
        1,
    ),
    // planting and entourage follow Revit 2027 (IfcGeographicElement);
    // this key is Revit 2024's export, which writes them as proxies (+11),
    // while the Nordic terrain models (2027 keys) lose 9 / 9 / 1 / 1 differences.
    ("DACHsampleproject", "DACHsampleproject.ifc", 46, 0, 34),
];

/// The classes these checks hold equal to Revit's, by Tag.
const CHECKED: &[&str] = &[
    "IFCCURTAINWALL",
    "IFCWALL",
    "IFCCOVERING",
    "IFCSLAB",
    "IFCSHADINGDEVICE",
    "IFCELECTRICAPPLIANCE",
    "IFCAIRTERMINAL",
    "IFCSANITARYTERMINAL",
    "IFCFURNITURE",
    "IFCSTAIR",
    "IFCTRANSPORTELEMENT",
    "IFCDOOR",
    "IFCWINDOW",
];

fn export_as_and_doors_follow_revits_export(k: usize, n: usize) {
    let Some(dir) = samples() else { return };
    let mut checked = 0usize;
    let mut measured = Vec::new();
    for (stem, revit, max_other, max_absent, max_wrong) in crate::shard(WITH_REVIT_IFC, k, n) {
        let rvt = dir.join(format!("{stem}.rvt"));
        // "2027:" names Revit 2027.1's export of the file.
        let ifc = match revit.strip_prefix("2027:") {
            Some(name) => {
                let Some(dir) = std::env::var_os("RVT_REVIT2027_IFC_DIR").map(PathBuf::from) else {
                    eprintln!("skipping {stem}: RVT_REVIT2027_IFC_DIR is not set");
                    continue;
                };
                dir.join(name)
            }
            None => dir.join(revit),
        };
        if !rvt.exists() || !ifc.exists() {
            eprintln!("skipping {stem}: absent");
            continue;
        }
        let theirs = classes(&std::fs::read_to_string(&ifc).expect("read Revit IFC"));
        let ours = classes(&export(&rvt));
        let mut wrong: BTreeMap<String, usize> = BTreeMap::new();
        let mut other = 0usize;
        for (tag, revit_classes) in &theirs {
            let Some(our_classes) = ours.get(tag) else {
                continue;
            };
            if our_classes != revit_classes {
                other += 1;
            }
            if revit_classes.iter().any(|c| CHECKED.contains(&c.as_str())) {
                checked += 1;
                if our_classes != revit_classes {
                    let key = format!(
                        "{} as {}",
                        revit_classes.iter().cloned().collect::<Vec<_>>().join(","),
                        our_classes.iter().cloned().collect::<Vec<_>>().join(",")
                    );
                    *wrong.entry(key).or_default() += 1;
                }
            }
        }
        // A door or window written as its opening alone has no element of
        // its Tag, so it is absent above: count Revit's we do not write.
        let absent = theirs
            .iter()
            .filter(|(tag, c)| {
                (c.contains("IFCDOOR") || c.contains("IFCWINDOW")) && !ours.contains_key(*tag)
            })
            .count();
        eprintln!("{stem}: {other} differ, {absent} doors/windows absent, {wrong:?}");
        let wrong_total: usize = wrong.values().sum();
        measured.push((
            stem,
            wrong_total,
            *max_wrong,
            absent,
            *max_absent,
            other,
            *max_other,
        ));
    }
    eprintln!("checked {checked}");
    // Exact pins, so a change in either direction is seen.
    let off: Vec<_> = measured
        .iter()
        .filter(|(_, w, mw, a, ma, o, mo)| (w, a, o) != (mw, ma, mo))
        .collect();
    assert!(
        off.is_empty(),
        "(stem, checked classes differ, pin, doors/windows absent, pin, classes differ, pin): {off:?}"
    );
    // The total over all shards, once every shard has recorded its own.
    let record = format!("{} {checked}", measured.len());
    if let Some(all) = crate::all_shards("rvt_open_hints-export_as_and_doors", k, n, &record) {
        let sum = |i: usize| -> usize {
            all.iter()
                .map(|r| r.split(' ').nth(i).unwrap().parse::<usize>().unwrap())
                .sum()
        };
        if sum(0) == WITH_REVIT_IFC.len() {
            assert_eq!(sum(1), MIN_CHECKED, "elements checked");
        }
    }
}

split!(export_as_and_doors_follow_revits_export, export_as_and_doors_follow_revits_export, s0 0, s1 1, s2 2, s3 3, s4 4, s5 5, s6 6, s7 7, s8 8, s9 9, s10 10, s11 11, s12 12);

/// Elements of the `CHECKED` classes compared by the test above when every
const MIN_CHECKED: usize = 11_618;

/// The classes whose `PredefinedType` follows "Export to IFC As" on the
/// Nordic models: what a curtain wall, wall sweep, ceiling or floor exports as.
const EXPORT_AS_CLASSES: &[&str] = &[
    "IFCCURTAINWALL",
    "IFCSHADINGDEVICE",
    "IFCWALL",
    "IFCCOVERING",
    "IFCSLAB",
];

/// Per Nordic model, by GlobalId, of the elements Revit 2027.1 writes as one
/// of [`EXPORT_AS_CLASSES`]: those we write with Revit's class and
/// `PredefinedType`, and those we do not (absent or different), measured on
/// (concrete) file (124 shading devices and 21 walls written as curtain walls,
/// 9 sweeps as proxies, and 75 curtain walls `NOTDEFINED` where Revit 2027.1
/// writes `USERDEFINED`), 107 and 105 fewer on the office (timber) files (21
/// walls, 7 sweeps, and 79 and 77 curtain walls) and one fewer: the
/// roof and covering fixes make two more of Revit's
/// elements right on each of the four housing files (as built and permit,
/// concrete and timber).
/// Two more right and two fewer wrong on each housing file since gutters are
/// recovered from their element records: Revit 2027 writes them as
/// roofing coverings or `IfcPipeSegment` `GUTTER`, and so does rvt-open.
const EXPORT_AS_2027: &[(&str, usize, usize)] = &[
    ("ARK_NordicLCA_Housing_Concrete_As-Built", 518, 5),
    ("ARK_NordicLCA_Housing_Concrete_BuildingPermit", 473, 5),
    (
        "ARK_NordicLCA_Housing_Terrain-Concrete_BuildingPermit",
        20,
        2,
    ),
    ("ARK_NordicLCA_Housing_Terrain-Timber_BuildingPermit", 20, 6),
    ("ARK_NordicLCA_Housing_Timber_As-Built", 562, 1),
    ("ARK_NordicLCA_Housing_Timber_BuildingPermit", 492, 1),
    ("ARK_NordicLCA_Office_Concrete_As-Built", 1010, 0),
    ("ARK_NordicLCA_Office_Concrete_BuildingPermit", 956, 0),
    ("ARK_NordicLCA_Office_Terrain_Concrete_BuildingPermit", 2, 0),
    ("ARK_NordicLCA_Office_Terrain_Timber_BuildingPermit", 2, 0),
    ("ARK_NordicLCA_Office_Timber_As-Built", 908, 1),
    ("ARK_NordicLCA_Office_Timber_BuildingPermit", 867, 1),
];

/// Class and `PredefinedType` against Revit 2027.1's export, by GlobalId,
/// under both conventions: "Export to IFC As" is the exporter's rule, so the
/// two give the same rows for these elements.
fn export_as_classes_and_predefined_types_follow_revit_2027(k: usize, n: usize) {
    use super::decoder_diffs::rows;
    use rvt::ifc::ExportConvention;
    let Some(dir) = samples() else { return };
    let Some(dir_2027) = std::env::var_os("RVT_REVIT2027_IFC_DIR").map(PathBuf::from) else {
        eprintln!("skipping: RVT_REVIT2027_IFC_DIR is not set");
        return;
    };
    let mut off = Vec::new();
    for (stem, right_pin, wrong_pin) in crate::shard(EXPORT_AS_2027, k, n) {
        let (rvt, ifc) = (
            dir.join(format!("{stem}.rvt")),
            dir_2027.join(format!("{stem}.ifc")),
        );
        if !rvt.exists() || !ifc.exists() {
            eprintln!("skipping {stem}: absent");
            continue;
        }
        let answer = rows(&std::fs::read_to_string(&ifc).expect("read Revit IFC"));
        let mut per_convention = Vec::new();
        for convention in [ExportConvention::Revit, ExportConvention::Open] {
            let mut rf = RevitFile::open(&rvt).expect("open");
            let model = RvtDocExporter
                .export_with_convention(
                    &mut rf,
                    rvt::ifc::ExportQualityMode::Scaffold,
                    rvt::walker::WalkerLimits::default(),
                    convention,
                )
                .expect("export");
            let ours = rows(&write_step(&model));
            let (mut right, mut wrong) = (0usize, BTreeMap::<String, usize>::new());
            let mut seen = Vec::new();
            for (id, reference) in answer
                .iter()
                .filter(|(_, r)| EXPORT_AS_CLASSES.contains(&r.entity.as_str()))
            {
                let got = ours
                    .get(id)
                    .map(|r| (r.entity.clone(), r.predefined.clone()));
                seen.push(got.clone());
                if got.as_ref() == Some(&(reference.entity.clone(), reference.predefined.clone())) {
                    right += 1;
                } else {
                    let (e, p) = got.unwrap_or_default();
                    *wrong
                        .entry(format!(
                            "{}.{} as {e}.{p}",
                            reference.entity, reference.predefined
                        ))
                        .or_default() += 1;
                }
            }
            let wrong_total: usize = wrong.values().sum();
            eprintln!("{stem} {convention:?}: {right} right, {wrong_total} not {wrong:?}");
            if (right, wrong_total) != (*right_pin, *wrong_pin) {
                off.push((stem, convention, right, wrong_total));
            }
            per_convention.push(seen);
        }
        assert_eq!(
            per_convention[0], per_convention[1],
            "{stem}: the conventions differ on these elements"
        );
    }
    assert!(
        off.is_empty(),
        "(stem, convention, right, not right): {off:?}"
    );
}

split!(export_as_classes_and_predefined_types_follow_revit_2027, export_as_classes_and_predefined_types_follow_revit_2027, s0 0, s1 1, s2 2, s3 3, s4 4, s5 5, s6 6, s7 7, s8 8, s9 9, s10 10, s11 11);

/// model and primary options), less the elements that do not exist in the
/// project's last phase: the dump's own phase columns put 6 of its 46
/// windows in Phase 1 and demolish them in Phase 2, the last, and Revit's
/// export leaves them out.
const GOLDEN_NUGGET: &[(&str, usize, usize)] = &[
    (
        "2024/BIM_Projekt_Golden_Nugget-Architektur_und_Ingenieurbau__pf2024.rvt",
        54,
        40,
    ),
    (
        "2026/BIM_Projekt_Golden_Nugget-Architektur_und_Ingenieurbau__pf2026.rvt",
        54,
        40,
    ),
];

fn doors_and_windows_with_a_native_map_are_written(k: usize, n: usize) {
    let Some(dir) = std::env::var_os("RVT_REVIT_EXTRA_DIR").map(PathBuf::from) else {
        eprintln!("skipping: RVT_REVIT_EXTRA_DIR is not set");
        return;
    };
    for (file, doors, windows) in crate::shard(GOLDEN_NUGGET, k, n) {
        let path = dir.join(file);
        if !path.exists() {
            eprintln!("skipping {file}: absent");
            continue;
        }
        let ours = classes(&export(&path));
        let count = |class: &str| ours.values().filter(|c| c.contains(class)).count();
        assert_eq!(
            (count("IFCDOOR"), count("IFCWINDOW")),
            (*doors, *windows),
            "{file}: doors and windows against the API dump"
        );
    }
}

split!(doors_and_windows_with_a_native_map_are_written, doors_and_windows_with_a_native_map_are_written, s0 0, s1 1);
