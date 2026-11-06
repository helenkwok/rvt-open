//! pipes, ducts, curtain mullions and panels the Revit API gives a
//! type are typed in the IFCX on Revit 2023 and 2026, and for names that are
//! not Latin.
//!
//! Runs against `RVT_REVIT_EXTRA_DIR` (a directory holding
//! `<release>/<label>.rvt`, the CC BY-NC-SA samples of the Revit API dump
//! A count is the occurrences of an IFC class in one model, typed and total;
//! every one is typed in the Revit API.

use rvt::RevitFile;
use rvt::ifc::{RvtDocExporter, write_ifcx};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::PathBuf;

const EXTRA: &str = "RVT_REVIT_EXTRA_DIR";
const SAMPLES: &str = "RVT_REVIT2024_SAMPLES_DIR";

/// (typed, total) occurrences of IFC class `class` in `file` of the
/// directory named by `env`, or `None` when that is absent.
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
    let (mut typed, mut total) = (0, 0);
    for n in data {
        let (Some(code), Some(path)) = (
            n["attributes"]["bsi::ifc::class"]["code"].as_str(),
            n["path"].as_str(),
        ) else {
            continue;
        };
        // flex ducts and pipes are segments of the same classes
        // (`.FLEXIBLESEGMENT.`); their types are not read yet, and these
        // counts are of rigid segments.
        if code != class
            || inherited.contains(path)
            || n["attributes"]["ifc-src::attr::Tag"].is_null()
            || n["attributes"][rvt::ifc::graph::UNPLACED_TYPE] == true
            || n["attributes"]["ifc-src::attr::PredefinedType"].as_str() == Some("FLEXIBLESEGMENT")
        {
            continue;
        }
        total += 1;
        typed += usize::from(n["inherits"]["TypeProduct"].is_string());
    }
    Some((typed, total))
}

/// Every occurrence of `class` in the model is typed, and there are `total`.
fn all_typed(env: &str, file: &str, class: &str, total: usize) {
    if let Some(got) = typed(env, file, class) {
        assert_eq!(got, (total, total), "{file}: {class} (typed, total)");
    }
}

/// Revit 2023: 491 of 491 pipes, 724 of 724 ducts (0 typed before). Their
/// types have no name entry, and a 2023 type object holds its ids as `u32`.
fn revit_2023_pipes_and_ducts_are_typed(k: usize, n: usize) {
    const FILE: &str = "2023/rme_basic_sample_project__pf2023.rvt";
    // One shard per (model, class) export.
    let jobs = [
        (EXTRA, FILE, "IfcPipeSegment", 491),
        (EXTRA, FILE, "IfcDuctSegment", 724),
    ];
    for (env, file, class, total) in crate::shard(jobs, k, n) {
        all_typed(env, file, class, total);
    }
}

split!(revit_2023_pipes_and_ducts_are_typed, revit_2023_pipes_and_ducts_are_typed, s0 0, s1 1);

/// Revit 2023: 3,046 of 3,046 mullions and 1,276 of 1,276 panels (0 typed
/// before). A mullion's or panel's type is a system type with no record of
/// its own; its element data names its family.
#[test]
fn revit_2023_mullions_and_panels_are_typed() {
    let file = "2023/Arch_Link_Model__pf2023.rvt";
    all_typed(EXTRA, file, "IfcMember", 3046);
    all_typed(EXTRA, file, "IfcPlate", 1276);
}

/// Revit 2026 ducts (795 of 795 and 158 of 158; 0 typed before) and pipes
/// (1,155 and 2,381; 0 typed before): the profile class tags that mark a
/// type's name are the file's own, not the 27 samples'. The Japanese file
/// names its pipe types in Japanese, which a Latin-only reader dropped (2 of
/// its pipes took a piping system for their type).
fn revit_2026_pipes_and_ducts_are_typed(k: usize, n: usize) {
    const JAPANESE_2026: &str = "2026/cjk_205a4b00__pf2026.rvt";
    const GERMAN_2026: &str = "2026/BIM_Projekt_Golden_Nugget-Gebaeudetechnik__pf2026.rvt";
    // One shard per (model, class) export.
    let jobs = [
        (EXTRA, JAPANESE_2026, "IfcDuctSegment", 795),
        (EXTRA, JAPANESE_2026, "IfcPipeSegment", 1155),
        (EXTRA, GERMAN_2026, "IfcDuctSegment", 158),
        (EXTRA, GERMAN_2026, "IfcPipeSegment", 2381),
    ];
    for (env, file, class, total) in crate::shard(jobs, k, n) {
        all_typed(env, file, class, total);
    }
}

split!(revit_2026_pipes_and_ducts_are_typed, revit_2026_pipes_and_ducts_are_typed, s0 0, s1 1, s2 2, s3 3);

/// The same Japanese file saved by Revit 2024: 1,155 of 1,155 pipes (1,153
/// untyped before, the other 2 typed wrongly).
#[test]
fn revit_2024_pipes_with_japanese_type_names_are_typed() {
    all_typed(
        EXTRA,
        "2024/cjk_602290d6__pf2024.rvt",
        "IfcPipeSegment",
        1155,
    );
}

/// A pipe whose reference list names two pipe types (one a type it was
/// changed from) takes the type its curve object holds: 2,381 of 2,381 pipes
/// (13 untyped before), and 3,056 of 3,056 of Snowdon Towers Plumbing (1).
fn a_pipe_naming_two_types_takes_its_curve_objects(k: usize, n: usize) {
    // One shard per (model, class) export.
    let jobs = [
        (
            EXTRA,
            "2024/BIM_Projekt_Golden_Nugget-Gebaeudetechnik__pf2024.rvt",
            "IfcPipeSegment",
            2381,
        ),
        (
            SAMPLES,
            "Snowdon Towers Sample Plumbing.rvt",
            "IfcPipeSegment",
            3056,
        ),
    ];
    for (env, file, class, total) in crate::shard(jobs, k, n) {
        all_typed(env, file, class, total);
    }
}

split!(a_pipe_naming_two_types_takes_its_curve_objects, a_pipe_naming_two_types_takes_its_curve_objects, s0 0, s1 1);
