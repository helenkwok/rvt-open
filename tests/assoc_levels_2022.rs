//! the CellList association record is read on Revit 2022 files (u32 layout, as 2023).
//!
//! Runs against `RVT_REVIT_EXTRA_DIR` (`<release>/<label>.rvt`, the Revit API dump samples of
//! the API lists (6,756 of 6,756).

use rvt::RevitFile;
use rvt::ifc::{RvtDocExporter, write_ifcx};
use std::path::PathBuf;

const FILE: &str = "2022/cjk_9480d97c__pf2022.rvt";

fn sample() -> Option<PathBuf> {
    let Some(dir) = std::env::var_os("RVT_REVIT_EXTRA_DIR").map(PathBuf::from) else {
        eprintln!("skipping: RVT_REVIT_EXTRA_DIR is not set");
        return None;
    };
    let path = dir.join(FILE);
    path.exists().then_some(path).or_else(|| {
        eprintln!("skipping: {FILE} absent");
        None
    })
}

#[test]
fn association_records_are_read_on_2022() {
    let Some(path) = sample() else { return };
    let mut rf = RevitFile::open(&path).expect("open");
    let levels =
        rvt::research::partition_schema_mvp::level_element_ids(&mut rf, 2022).expect("levels");
    let assoc = rvt::partition_assoc_levels::assoc_level_ids(&mut rf, 2022, &levels);
    assert_eq!(assoc.len(), 6756);
}

/// The 432 elements with no other Level rule take it from the record (431, plus wall 55853494 that the
/// (revit) export leaves the file's 872 non-primary design-option elements out, wall 55853494 (option "A") among
/// them: 431 remain.
#[test]
fn levelless_2022_elements_are_bound() {
    let Some(path) = sample() else { return };
    let mut rf = RevitFile::open(&path).expect("open");
    let model = RvtDocExporter
        .export_with_diagnostics(&mut rf)
        .expect("export")
        .model;
    let ifcx = write_ifcx(&model).expect("ifcx");
    assert_eq!(ifcx.matches("partition_association_record").count(), 431);
}
