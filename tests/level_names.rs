//! Level names and elevations read from the Levels' name blocks on
//! Revit 2025 files, and the storeys they make.
//!
//! Every RE1 model (Revit 2025, MIT) has the two Levels Revit's own export
//! writes as storeys, with their names and elevations, and each storey
//! rvt-open writes has Revit's GlobalId. Corpus-gated through
//! `RVT_PROJECT_CORPUS_DIR`.

use rvt::RevitFile;
use rvt::ifc::{RvtDocExporter, write_step};
use rvt::research::partition_level_records::recover_partition_levels;
use std::collections::BTreeSet;
use std::path::PathBuf;

/// Every GlobalId of `entity` in a STEP file.
fn global_ids_of(step: &str, entity: &str) -> BTreeSet<String> {
    let needle = format!("={entity}('");
    step.lines()
        .filter_map(|line| {
            let at = line.find(&needle)? + needle.len();
            line.get(at..at + 22).map(str::to_string)
        })
        .collect()
}

#[test]
fn re1_levels_and_storeys_are_revits() {
    let Some(dir) = std::env::var_os("RVT_PROJECT_CORPUS_DIR").map(PathBuf::from) else {
        eprintln!("skipping: RVT_PROJECT_CORPUS_DIR is not set");
        return;
    };
    // Revit's export writes Level 2 at 3,660 mm.
    let level_2 = 3660.0 / 304.8;
    let mut checked = 0;
    for model in ["Architecture", "Mechanical", "Plumbing", "Electrical"] {
        let rvt = dir.join(format!("RE1-{model}.rvt"));
        let reference = dir.join(format!("RE1-{model}.ifc"));
        if !rvt.exists() || !reference.exists() {
            continue;
        }
        let mut rf = RevitFile::open(&rvt).expect("open");
        let levels = recover_partition_levels(&mut rf, 2025).expect("levels");
        let read: Vec<(&str, f64)> = levels
            .iter()
            .map(|level| (level.name.as_str(), level.elevation_feet))
            .collect();
        assert_eq!(read.len(), 2, "RE1 {model}: {read:?}");
        assert_eq!(read[0], ("Level 1", 0.0), "RE1 {model}");
        assert_eq!(read[1].0, "Level 2", "RE1 {model}");
        assert!((read[1].1 - level_2).abs() < 1e-9, "RE1 {model}: {read:?}");

        let ours = write_step(
            &RvtDocExporter
                .export_with_diagnostics(&mut rf)
                .expect("export")
                .model,
        );
        let theirs = std::fs::read_to_string(&reference).expect("reference IFC");
        let revit = global_ids_of(&theirs, "IFCBUILDINGSTOREY");
        assert_eq!(revit.len(), 2, "RE1 {model}");
        assert_eq!(
            global_ids_of(&ours, "IFCBUILDINGSTOREY"),
            revit,
            "RE1 {model}"
        );
        checked += 1;
    }
    if checked == 0 {
        eprintln!("skipping: no RE1 model under RVT_PROJECT_CORPUS_DIR");
    }
}

/// Einhoven (Revit 2023, MIT) has one Level, `Level 1` at 0 ft, which
/// Revit's own IFC4 exports write as their one storey (GlobalId
/// `3V7WWv02fDhAgcYNgjNa6g`), holding the wall, its two windows and the
/// curtain wall. Its storey is that Level, not one per row of curtain
/// panels.
#[test]
fn einhoven_one_level_is_its_one_storey() {
    let Some(dir) = std::env::var_os("RVT_PROJECT_CORPUS_DIR").map(PathBuf::from) else {
        eprintln!("skipping: RVT_PROJECT_CORPUS_DIR is not set");
        return;
    };
    let rvt = dir.join("Revit_IFC5_Einhoven.rvt");
    if !rvt.exists() {
        eprintln!("skipping: Revit_IFC5_Einhoven.rvt absent");
        return;
    }
    let mut rf = RevitFile::open(&rvt).expect("open");
    let levels = recover_partition_levels(&mut rf, 2023).expect("levels");
    let read: Vec<(&str, f64)> = levels
        .iter()
        .map(|level| (level.name.as_str(), level.elevation_feet))
        .collect();
    assert_eq!(read, [("Level 1", 0.0)]);

    let ours = write_step(
        &RvtDocExporter
            .export_with_diagnostics(&mut rf)
            .expect("export")
            .model,
    );
    assert_eq!(
        global_ids_of(&ours, "IFCBUILDINGSTOREY"),
        BTreeSet::from(["3V7WWv02fDhAgcYNgjNa6g".to_string()])
    );
    // The wall, the two windows and the curtain wall, by Tag, in it.
    let storey = ours
        .lines()
        .find(|line| line.contains("=IFCBUILDINGSTOREY("))
        .and_then(|line| line.split('=').next())
        .expect("storey")
        .to_string();
    let contained = ours
        .lines()
        .find(|line| {
            line.contains("=IFCRELCONTAINEDINSPATIALSTRUCTURE(")
                && line.trim_end().ends_with(&format!(",{storey});"))
        })
        .expect("containment in the storey");
    for tag in ["2808", "5317", "5388", "7187"] {
        let product = ours
            .lines()
            .find(|line| line.contains(&format!(",'{tag}',")) && line.starts_with('#'))
            .and_then(|line| line.split('=').next())
            .unwrap_or_else(|| panic!("element {tag}"));
        assert!(
            contained.contains(&format!("{product},"))
                || contained.contains(&format!("{product})")),
            "element {tag} is not in Level 1"
        );
    }
}
