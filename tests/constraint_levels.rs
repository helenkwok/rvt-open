//! Levels bound from the CellList association record, and 2023
//! Level records with a 42-byte header.
//!
//! Runs against `RVT_REVIT_EXTRA_DIR` (a directory holding
//! `<release>/<label>.rvt`, the CC BY-NC-SA samples of the Revit API dump of
//! that dump (no Level disagreement).

use rvt::RevitFile;
use rvt::ifc::{RvtDocExporter, write_ifcx};
use std::path::PathBuf;

fn sample(file: &str) -> Option<PathBuf> {
    let Some(dir) = std::env::var_os("RVT_REVIT_EXTRA_DIR").map(PathBuf::from) else {
        eprintln!("skipping: RVT_REVIT_EXTRA_DIR is not set");
        return None;
    };
    let path = dir.join(file);
    path.exists().then_some(path).or_else(|| {
        eprintln!("skipping: {file} absent");
        None
    })
}

/// Elements whose Level came from the association record, per file. The
/// 2023 file's 162 are now bound by the base constraint and the
/// reference list, which find the file's 9 Levels once the 11 records that
/// are not Levels are gone: the association record is the last
/// resolve, and the default (revit) export leaves their 872 non-primary
/// design-option elements out, wall 55853494 (option "A") among them: 432
/// remain.
#[test]
fn association_record_binds_levels() {
    for (file, expected) in [
        ("2024/cjk_138949ac__pf2024.rvt", 432),
        ("2026/cjk_1706fd8d__pf2026.rvt", 432),
        ("2023/cjk_b46ce352__pf2023.rvt", 0),
    ] {
        let Some(path) = sample(file) else { continue };
        let mut rf = RevitFile::open(&path).expect("open");
        let model = RvtDocExporter
            .export_with_diagnostics(&mut rf)
            .expect("export")
            .model;
        let ifcx = write_ifcx(&model).expect("ifcx");
        let n = ifcx.matches("partition_association_record").count();
        assert_eq!(n, expected, "{file}");
    }
}

/// The Level records of 2022/2023 files with longer headers:
/// the header length is the field count the record states, so the 11th Level
/// of a file whose other ten carry two fields (three fields: 56 bytes) is
/// found, and the 11 records of the Levels category that belong to another
/// element (an owner id where a Level has none) are not Levels. Each count is
/// the number of Levels the Revit API lists for the file.
#[test]
fn level_record_set_is_the_files_levels() {
    for (file, version, expected) in [
        ("2023/cjk_b46ce352__pf2023.rvt", 2023, 9),
        ("2023/cjk_e618e87d__pf2023.rvt", 2023, 11),
        ("2022/cjk_cf74146d__pf2022.rvt", 2022, 11),
        ("2023/cjk_cd52123f__pf2023.rvt", 2023, 14),
        ("2022/cjk_9480d97c__pf2022.rvt", 2022, 14),
    ] {
        let Some(path) = sample(file) else { continue };
        let mut rf = RevitFile::open(&path).expect("open");
        let ids = rvt::research::partition_schema_mvp::level_element_ids(&mut rf, version)
            .expect("levels");
        assert_eq!(ids.len(), expected, "{file}");
    }
}

/// With every Level found, no element is placed by a neighbour's Level
/// (`same_class_host`) or a base elevation against a Level set that lacked
/// one: on these two files 22 elements were bound to a wrong Level (18 pipes,
/// 4 light fixtures) while the 11th Level was missing.
#[test]
fn no_wrong_level_from_an_incomplete_level_set() {
    for (file, host, elevation) in [
        ("2023/cjk_e618e87d__pf2023.rvt", 0, 4),
        ("2022/cjk_cf74146d__pf2022.rvt", 0, 4),
    ] {
        let Some(path) = sample(file) else { continue };
        let mut rf = RevitFile::open(&path).expect("open");
        let model = RvtDocExporter
            .export_with_diagnostics(&mut rf)
            .expect("export")
            .model;
        let ifcx = write_ifcx(&model).expect("ifcx");
        assert_eq!(
            ifcx.matches("partition_element_record_same_class_host")
                .count(),
            host,
            "{file}"
        );
        assert_eq!(
            ifcx.matches("partition_element_record_base_elevation")
                .count(),
            elevation,
            "{file}"
        );
    }
}
