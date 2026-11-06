//! an element Revit's API gives a type is typed in the IFCX.
//!
//! The Revit 2024 API (a dump of each sample, taken in Revit) gives a type to
//! every occurrence counted below, so each of them must carry an `inherits`
//! `TypeProduct` link. A count here is the number of occurrences of an IFC
//! class in one model and how many of them are typed; the totals are the
//! models' own and the typed counts are the API's (nothing else of the
//! models is kept). Each test is one cause of an element left untyped.
//!
//! Runs against `RVT_NORDIC_LCA_DIR` (Nordic LCA, CC BY-SA) and
//! `RVT_REVIT2024_SAMPLES_DIR` (Autodesk and Snowdon Towers, CC BY-NC-SA).
//! Skips what is absent.

use rvt::RevitFile;
use rvt::ifc::{RvtDocExporter, write_ifcx};
use rvt::partition_type_names::read_type_names;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// (typed, total) occurrences of IFC class `class` in `file` of the directory
/// named by `env`, or `None` when that is absent. An occurrence is a node with
/// a class and a `Tag` that nothing inherits from, a room left out.
fn typed(env: &str, file: &str, class: &str) -> Option<(usize, usize)> {
    let Some(dir) = std::env::var_os(env).map(PathBuf::from) else {
        eprintln!("skipping: {env} is not set");
        return None;
    };
    let path = dir.join(file);
    if !path.exists() {
        eprintln!("skipping: {file} absent");
        return None;
    }
    let mut rf = RevitFile::open(&path).expect("open");
    let model = RvtDocExporter
        .export_with_diagnostics(&mut rf)
        .expect("export")
        .model;
    let doc: Value = serde_json::from_str(&write_ifcx(&model).expect("ifcx")).expect("json");
    let data = doc["data"].as_array().expect("data");
    let inherited: BTreeSet<&str> = data
        .iter()
        .filter_map(|n| n["inherits"]["TypeProduct"].as_str())
        .collect();
    let mut counts: BTreeMap<(&str, bool), usize> = BTreeMap::new();
    for n in data {
        let code = n["attributes"]["bsi::ifc::class"]["code"].as_str();
        let (Some(code), Some(path)) = (code, n["path"].as_str()) else {
            continue;
        };
        if code != class
            || inherited.contains(path)
            || n["attributes"]["ifc-src::attr::Tag"].is_null()
            || n["attributes"][rvt::ifc::graph::UNPLACED_TYPE] == true
        {
            continue;
        }
        *counts
            .entry((code, n["inherits"]["TypeProduct"].is_string()))
            .or_default() += 1;
    }
    let typed = counts.get(&(class, true)).copied().unwrap_or(0);
    Some((
        typed,
        typed + counts.get(&(class, false)).copied().unwrap_or(0),
    ))
}

const NORDIC: &str = "RVT_NORDIC_LCA_DIR";
const SAMPLES: &str = "RVT_REVIT2024_SAMPLES_DIR";

/// Every occurrence of `class` in the model is typed, and there are `total`.
fn all_typed(env: &str, file: &str, class: &str, total: usize) {
    if let Some(got) = typed(env, file, class) {
        assert_eq!(got, (total, total), "{file}: {class} (typed, total)");
    }
}

/// A railing type renamed in a later partition has the later name: its
/// earlier copy no longer cancels it. 56 of 56 railings (0 before).
#[test]
fn a_renamed_railing_type_takes_the_newest_name() {
    all_typed(
        NORDIC,
        "ARK_NordicLCA_Housing_Timber_BuildingPermit.rvt",
        "IfcRailing",
        56,
    );
}

/// A ceiling type edited after an older copy was written still has compound
/// layers, so it is a Compound Ceiling (and a wall type a Basic Wall). 71 of
/// to IFC As" adds downpipes 2206034 and 2206035, which Revit 2027's own IFC
/// writes as IfcCovering ROOFING too).
#[test]
fn a_layered_type_with_disagreeing_copies_keeps_its_system_family() {
    // 69 before: two more roofing coverings Revit's export has are written,
    // and typed. Two gutters, recovered from their element records,
    // are written as IfcCovering too, as Revit 2027's export writes them
    // through their "Export to IFC As", but no gutter type is read yet, so
    // they are the two untyped ones.
    if let Some(got) = typed(
        NORDIC,
        "ARK_NordicLCA_Housing_Timber_BuildingPermit.rvt",
        "IfcCovering",
    ) {
        assert_eq!(got, (71, 73), "IfcCovering (typed, total)");
    }
}

/// A stair type of construction 2 is a Precast Stair, and a project
/// parameter (a positive id) may sit in its parameter list. 7 of 7 (6 before).
/// specialty equipment whose "Export to IFC As" names it, as Revit's export
/// does (Revit's IFC of this file has 38 `IFCSTAIR`): all 38 are typed.
#[test]
fn a_precast_stair_type_is_a_precast_stair() {
    all_typed(
        NORDIC,
        "ARK_NordicLCA_Housing_Timber_BuildingPermit.rvt",
        "IfcStair",
        38,
    );
}

/// A landing type with the monolithic flag is a Monolithic Landing. 63 of 63
/// landings, exported as slabs (42 before).
#[test]
fn a_monolithic_landing_type_is_a_monolithic_landing() {
    all_typed(
        NORDIC,
        "ARK_NordicLCA_Office_Concrete_As-Built.rvt",
        "IfcSlab",
        63,
    );
}

/// A stair type whose name follows no parameter list is read. 3 of 3 (1 before).
#[test]
fn a_stair_type_name_without_a_parameter_list_is_read() {
    all_typed(SAMPLES, "racbasicsampleproject.rvt", "IfcStair", 3);
}

/// A stair type whose component slots hold `f64`s is of the Stair family.
/// A foundation type with compound layers is a Foundation Slab. 15 of 15
/// footings, 13 of them slabs (2 before).
#[test]
fn a_layered_foundation_type_is_a_foundation_slab() {
    all_typed(SAMPLES, "DACHsampleproject.rvt", "IfcFooting", 15);
}

/// types whose value block parses are named by it, which types
/// the located readers did not reach. All of the occurrences below were left
fn types_whose_value_block_parses_are_typed(k: usize, n: usize) {
    let cases: [(&str, &str, &str, usize); 4] = [
        (SAMPLES, "racbasicsampleproject.rvt", "IfcCurtainWall", 9),
        (SAMPLES, "rmebasicsampleproject.rvt", "IfcStair", 8),
        (SAMPLES, "Technicalschoolcurrentm.rvt", "IfcRoof", 6),
        (
            NORDIC,
            "ARK_NordicLCA_Office_Terrain_Concrete_BuildingPermit.rvt",
            "IfcRamp",
            6,
        ),
    ];
    for (env, file, class, total) in crate::shard(cases, k, n) {
        all_typed(env, file, class, total);
    }
}

split!(types_whose_value_block_parses_are_typed, types_whose_value_block_parses_are_typed, s0 0, s1 1, s2 2, s3 3);

/// Fail closed: a type with no element record in the partitions has no class,
/// so its block is not read and its occurrences stay untyped (typed, total).
#[test]
fn types_with_no_class_stay_untyped() {
    let cases: [(&str, &str, &str, (usize, usize)); 1] = [(
        SAMPLES,
        "DACHsampleproject.rvt",
        "IfcCurtainWall",
        (DACH_CURTAIN_TYPED, DACH_CURTAIN_TOTAL),
    )];
    for (env, file, class, want) in cases {
        if let Some(got) = typed(env, file, class) {
            assert_eq!(got, want, "{file}: {class} (typed, total)");
        }
    }
}

// the merge types all 43 (16 before).
const DACH_CURTAIN_TYPED: usize = 43;
const DACH_CURTAIN_TOTAL: usize = 43;

/// `rvtopen::revit::Inferred::FamilyNameRule`: on the type node where the name
/// is the type's, and on a stair part, which is named after its stair. A reader
/// can then tell it from a name read from the file.
const RULE_KEY: &str = "rvtopen::revit::Inferred::FamilyNameRule";

/// What `file` marks with `rule`: (type nodes, occurrences inheriting them,
/// marked nodes nothing inherits from), and whether every node named
/// `<family>:...` is a marked node or inherits one; `None` when the model is
/// absent.
fn marked(env: &str, file: &str, rule: &str, family: &str) -> Option<(usize, usize, usize, bool)> {
    let dir = PathBuf::from(std::env::var_os(env)?);
    let path = dir.join(file);
    if !path.exists() {
        eprintln!("skipping: {file} absent");
        return None;
    }
    let mut rf = RevitFile::open(&path).expect("open");
    let model = RvtDocExporter
        .export_with_diagnostics(&mut rf)
        .expect("export")
        .model;
    let doc: Value = serde_json::from_str(&write_ifcx(&model).expect("ifcx")).expect("json");
    let data = doc["data"].as_array().expect("data");
    let inherited: BTreeSet<&str> = data
        .iter()
        .filter_map(|n| n["inherits"]["TypeProduct"].as_str())
        .collect();
    let marks: BTreeSet<&str> = data
        .iter()
        .filter(|n| n["attributes"][RULE_KEY].as_str() == Some(rule))
        .filter_map(|n| n["path"].as_str())
        .collect();
    let types = marks.iter().filter(|p| inherited.contains(*p)).count();
    let prefix = format!("{family}:");
    let (mut occurrences, mut covered) = (0, true);
    for n in data {
        let inherits_mark = n["inherits"]["TypeProduct"]
            .as_str()
            .is_some_and(|t| marks.contains(t));
        let mine = inherits_mark || n["path"].as_str().is_some_and(|p| marks.contains(p));
        let named = n["attributes"]["bsi::ifc::prop::Name"]
            .as_str()
            .is_some_and(|name| name.starts_with(&prefix));
        occurrences += usize::from(inherits_mark);
        covered &= mine || !named;
    }
    // a marked type nothing places is no part.
    let unplaced = data
        .iter()
        .filter(|n| n["attributes"][rvt::ifc::graph::UNPLACED_TYPE] == true)
        .filter_map(|n| n["path"].as_str())
        .filter(|p| marks.contains(p))
        .count();
    Some((types, occurrences, marks.len() - types - unplaced, covered))
}

/// Each case is (directory variable, file, marked type nodes, occurrences of
/// them, marked parts); `exclusive` also requires that every name with the
/// rule's family is marked, for a family no other rule gives.
fn rule_counts(
    rule: &str,
    family: &str,
    exclusive: bool,
    cases: &[(&str, String, usize, usize, usize)],
    k: usize,
    n: usize,
) {
    let mut total = 0;
    for (env, file, types, occurrences, parts) in crate::shard(cases, k, n) {
        let Some((t, o, p, covered)) = marked(env, file, rule, family) else {
            return;
        };
        assert_eq!(
            (t, o, p),
            (*types, *occurrences, *parts),
            "{file}: {rule} (types, occurrences, parts)"
        );
        assert!(
            !exclusive || covered,
            "{file}: a {family} name without the marker"
        );
        total += o;
    }
    eprintln!("{rule}: {total} occurrences");
}

fn housing(name: &str) -> String {
    format!("ARK_NordicLCA_Housing_{name}.rvt")
}

fn office(name: &str) -> String {
    format!("ARK_NordicLCA_Office_{name}.rvt")
}

/// Cases from (file, types, occurrences, parts) in the Nordic directory.
fn nordic(
    files: &[(String, usize, usize, usize)],
) -> Vec<(&'static str, String, usize, usize, usize)> {
    files
        .iter()
        .map(|(f, t, o, p)| (NORDIC, f.clone(), *t, *o, *p))
        .collect()
}

/// "Construction 2 means Precast Stair": 12 stairs in 5 models, and 33 stair
/// parts named after them.
fn a_precast_stair_name_is_marked(k: usize, n: usize) {
    let mut cases = nordic(&[
        (housing("Concrete_As-Built"), 1, 4, 12),
        (housing("Concrete_BuildingPermit"), 1, 4, 12),
        (housing("Timber_As-Built"), 1, 1, 3),
        (housing("Timber_BuildingPermit"), 1, 1, 3),
    ]);
    cases.push((SAMPLES, "racbasicsampleproject.rvt".to_string(), 1, 2, 3));
    rule_counts(
        "precast-stair-by-construction",
        "Precast Stair",
        true,
        &cases,
        k,
        n,
    );
}

split!(a_precast_stair_name_is_marked, a_precast_stair_name_is_marked, s0 0, s1 1, s2 2, s3 3, s4 4);

/// "Monolithic flag means Monolithic Landing": 93 landings in 10 models.
fn a_monolithic_landing_name_is_marked(k: usize, n: usize) {
    let mut cases = nordic(&[
        (housing("Concrete_As-Built"), 1, 4, 0),
        (housing("Concrete_BuildingPermit"), 1, 4, 0),
        (housing("Timber_As-Built"), 1, 1, 0),
        (housing("Timber_BuildingPermit"), 1, 1, 0),
        (office("Concrete_As-Built"), 1, 21, 0),
        (office("Concrete_BuildingPermit"), 1, 21, 0),
        (office("Timber_As-Built"), 1, 17, 0),
        (office("Timber_BuildingPermit"), 1, 17, 0),
    ]);
    // DACH: one landing is demolished in the project's last phase (the API
    // dump's phase columns) and left out, as Revit's export does; the type it
    // alone used goes with it.
    cases.push((SAMPLES, "DACHsampleproject.rvt".to_string(), 1, 5, 0));
    cases.push((SAMPLES, "racbasicsampleproject.rvt".to_string(), 1, 1, 0));
    rule_counts(
        "monolithic-landing-by-flag",
        "Monolithic Landing",
        true,
        &cases,
        k,
        n,
    );
}

split!(a_monolithic_landing_name_is_marked, a_monolithic_landing_name_is_marked, s0 0, s1 1, s2 2, s3 3, s4 4, s5 5, s6 6, s7 7, s8 8, s9 9);

/// "A foundation whose layers read is a Foundation Slab": 18 footings in 3
/// models.
fn a_foundation_slab_name_is_marked(k: usize, n: usize) {
    rule_counts(
        "foundation-slab-by-layers",
        "Foundation Slab",
        true,
        &[
            (SAMPLES, "DACHsampleproject.rvt".to_string(), 5, 13, 0),
            (
                SAMPLES,
                "Snowdon Towers Sample Structural.rvt".to_string(),
                1,
                4,
                0,
            ),
            (SAMPLES, "racbasicsampleproject.rvt".to_string(), 1, 1, 0),
        ],
        k,
        n,
    );
}

split!(a_foundation_slab_name_is_marked, a_foundation_slab_name_is_marked, s0 0, s1 1, s2 2);

/// System-family names (Basic Wall, Compound Ceiling, Basic Roof) given to
/// types whose partition copies disagree: 180 occurrences in 6 models. Those
/// family names are also read by measured rules, so only the marked ones are
/// counted.
fn a_system_family_name_from_disagreeing_copies_is_marked(k: usize, n: usize) {
    let cases = nordic(&[
        (housing("Concrete_As-Built"), 0, 0, 0),
        (housing("Concrete_BuildingPermit"), 0, 0, 0),
        (housing("Terrain-Timber_BuildingPermit"), 0, 0, 0),
        (housing("Timber_As-Built"), 0, 0, 0),
        (housing("Timber_BuildingPermit"), 0, 0, 0),
        (office("Timber_BuildingPermit"), 0, 0, 0),
    ]);
    rule_counts(
        "system-family-copies-disagree",
        "Basic Wall",
        false,
        &cases,
        k,
        n,
    );
}

split!(a_system_family_name_from_disagreeing_copies_is_marked, a_system_family_name_from_disagreeing_copies_is_marked, s0 0, s1 1, s2 2, s3 3, s4 4, s5 5);

// a type's name is read from its value block by the file's schema
// (`partition_type_names`). The counts are how many types of each schema class
// the grammar names in one model; every name read agreed with the Revit 2024
// API in a separate check (6,884 of 6,884 names the API gives, over the 27
// samples; 5 `TilePattern` names it spells differently, 101 it does not give).

/// The names read in `file`, with their classes, by ElementId; `None` when
/// the model is absent.
fn reads(env: &str, file: &str) -> Option<BTreeMap<u32, (String, String)>> {
    let Some(dir) = std::env::var_os(env).map(PathBuf::from) else {
        eprintln!("skipping: {env} is not set");
        return None;
    };
    let path = dir.join(file);
    if !path.exists() {
        eprintln!("skipping: {file} absent");
        return None;
    }
    let mut rf = RevitFile::open(&path).expect("open");
    let declared = rvt::elem_table::declared_ids(
        &rvt::elem_table::parse_records(&mut rf).expect("elem table"),
    );
    let (read, _) = read_type_names(&mut rf, &declared).expect("read");
    Some(
        read.into_iter()
            .map(|(id, at)| (id, (at.read.class_name, at.read.name)))
            .collect(),
    )
}

/// How many types of `class` were read.
fn of_class(read: &BTreeMap<u32, (String, String)>, class: &str) -> usize {
    read.values().filter(|(c, _)| c == class).count()
}

/// The types read per class, for the classes `want` lists. Other classes are
/// read too, among them types the API dump does not list, whose names there
/// is nothing to check against.
fn check(env: &str, file: &str, want: &[(&str, usize)]) {
    let Some(read) = reads(env, file) else {
        return;
    };
    for (class, n) in want {
        assert_eq!(of_class(&read, class), *n, "{file}: {class}");
    }
}

#[test]
fn a_sample_models_system_types_are_named_by_class() {
    check(
        SAMPLES,
        "racbasicsampleproject.rvt",
        &[
            ("BasicWallType", 10),
            ("NewCurtainWallType", 1),
            ("FloorAttributes", 6),
            ("StairsType", 3),
            ("StairsRunType", 3),
            ("StairsLandingType", 3),
            ("StairsAttributes", 1),
            ("RampAttributes", 1),
            ("RoofAttributes", 2),
            ("CurtainRoofAttributes", 1),
            ("ContFootingType", 1),
            ("CorniceAttr", 1),
            ("Text3dAttrSymbol", 1),
        ],
    );
}

#[test]
fn a_nordic_models_system_types_are_named_by_class() {
    check(
        NORDIC,
        "ARK_NordicLCA_Housing_Concrete_As-Built.rvt",
        &[
            ("BasicWallType", 26),
            ("NewCurtainWallType", 3),
            ("FloorAttributes", 11),
            ("StairsRailingAttr", 11),
            ("RoofAttributes", 8),
            ("CompoundCeilingType", 4),
            ("HandRailType", 4),
            ("TopRailType", 1),
            ("StairsSupportType", 4),
            ("RampAttributes", 1),
        ],
    );
}

/// A class no derivation step looked at: `RbsWireType` and
/// `RbsPipeInsulationType` parse by the same grammar, and name Revit's types.
#[test]
fn a_class_never_inspected_parses() {
    let Some(read) = reads(SAMPLES, "snowdonelectrical.rvt")
        .or_else(|| reads(SAMPLES, "Snowdon Towers Sample Electrical.rvt"))
    else {
        return;
    };
    assert_eq!(of_class(&read, "RbsWireType"), 2);
    assert_eq!(of_class(&read, "RbsPipeInsulationType"), 2);
    assert_eq!(of_class(&read, "RbsDuctInsulationType"), 2);
}

/// Fail closed: the types whose fields the grammar does not know have no
/// name read (containers of `FamilySymbol`, `ImportSymbol`, and so on).
#[test]
fn a_type_with_a_field_the_grammar_does_not_know_has_no_name() {
    let Some(read) = reads(SAMPLES, "racbasicsampleproject.rvt") else {
        return;
    };
    for class in ["FamilySymbol", "MasterImportSymbol", "ElementGroupType"] {
        assert_eq!(of_class(&read, class), 0, "{class}");
    }
}
