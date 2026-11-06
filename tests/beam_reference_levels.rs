//! a structural framing element's `LevelElementId` is its Reference Level, read from
//! the id lists of its element record; the Level the other rules gave it is kept apart as its
//! containment Level, so building-storey containment is unchanged.
//!
//! Runs against `RVT_REVIT2024_SAMPLES_DIR` (`<name>.rvt`, public Autodesk and Nordic LCA
//! models) and skips what is absent. The counts were measured against the Revit API dump of
//! every beam counted here is bound to the Level the API gives as its Reference Level
//! (parameter -1001383).

use rvt::RevitFile;
use rvt::ifc::{RvtDocExporter, write_ifcx};
use std::path::PathBuf;

const PREFIX: &str = "rvtopen::revit::RvtElementRecordGeometry::";

fn sample(file: &str) -> Option<PathBuf> {
    let Some(dir) = std::env::var_os("RVT_REVIT2024_SAMPLES_DIR").map(PathBuf::from) else {
        eprintln!("skipping: RVT_REVIT2024_SAMPLES_DIR is not set");
        return None;
    };
    let path = dir.join(file);
    path.exists().then_some(path).or_else(|| {
        eprintln!("skipping: {file} absent");
        None
    })
}

/// (beams bound to their Reference Level, of those beams that keep another Level for
/// containment, beams whose `ContainmentLevelBindSource` is the base-constraint rule).
fn counts(file: &str) -> Option<(usize, usize, usize)> {
    let path = sample(file)?;
    let mut rf = RevitFile::open(&path).expect("open");
    let model = RvtDocExporter
        .export_with_diagnostics(&mut rf)
        .expect("export")
        .model;
    let ifcx: serde_json::Value =
        serde_json::from_str(&write_ifcx(&model).expect("ifcx")).expect("json");
    let (mut bound, mut kept, mut base_constraint) = (0, 0, 0);
    for node in ifcx["data"].as_array().expect("data") {
        let attributes = &node["attributes"];
        let text = |key: &str| {
            attributes[format!("{PREFIX}{key}")]
                .as_str()
                .map(str::to_owned)
        };
        if text("LevelBindSource").as_deref() != Some("partition_element_record_reference_level") {
            continue;
        }
        bound += 1;
        if !attributes[format!("{PREFIX}ContainmentLevelElementId")].is_null() {
            kept += 1;
        }
        if text("ContainmentLevelBindSource").as_deref()
            == Some("partition_element_record_base_constraint")
        {
            base_constraint += 1;
        }
    }
    Some((bound, kept, base_constraint))
}

/// Beams take their Reference Level, and keep the Level the other rules gave them.
fn beams_take_their_reference_level(k: usize, n: usize) {
    let cases = [
        ("racadvancedsampleproject.rvt", (37, 16, 2)), // API dump agrees
        ("ARK_NordicLCA_Office_Timber_As-Built.rvt", (71, 71, 47)), // API dump agrees
    ];
    for (file, expected) in crate::shard(cases, k, n) {
        let Some(found) = counts(file) else { continue };
        assert_eq!(found, expected, "{file}");
    }
}

split!(beams_take_their_reference_level, beams_take_their_reference_level, s0 0, s1 1);

/// Count of beams bound to their Reference Level in the 2024 sample.
/// Each of the 37 matches the Revit API dump's "Reference Level" parameter
/// (builtin -1001383) for that element, element id by element id.
const EXPECTED_2024_REF_LEVEL_BEAMS: usize = 37;

#[test]
fn reference_levels_u64_validates_first_reference_list() {
    let Some(path) = sample("racadvancedsampleproject.rvt") else {
        return;
    };
    let mut rf = RevitFile::open(&path).expect("open");
    let version = rf.basic_file_info().expect("bfi").version;
    assert!(version >= 2024, "sample is Revit 2024+");
    let levels = rvt::research::partition_level_records::recover_partition_levels(&mut rf, version)
        .expect("levels");
    let level_ids: std::collections::BTreeSet<u32> = levels.iter().map(|l| l.element_id).collect();
    let elevations: std::collections::BTreeMap<u32, f64> = levels
        .iter()
        .map(|l| (l.element_id, l.elevation_feet))
        .collect();
    let ref_levels =
        rvt::beam_reference_levels::reference_levels(&mut rf, version, &level_ids, &elevations);
    assert_eq!(
        ref_levels.len(),
        EXPECTED_2024_REF_LEVEL_BEAMS,
        "reference_levels count on racadvancedsampleproject.rvt"
    );
}
