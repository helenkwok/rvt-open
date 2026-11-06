//! the CellList association record is read on Revit 2027 files.
//!
//! Runs against `RVT_REVIT_EXTRA_DIR` (`<release>/<label>.rvt`, the Revit API dump samples of
//! association record read on these files names the Level the API lists.

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

/// Association records read per 2027 file (fitted models).
#[test]
fn association_records_are_read_on_2027() {
    for (file, expected) in [
        ("2027/Pacific_Continental_Residence_Sample__pf2027.rvt", 983),
        ("2027/Snowdon_Towers_Sample_Structural__pf2027.rvt", 332),
    ] {
        let Some(path) = sample(file) else { continue };
        let mut rf = RevitFile::open(&path).expect("open");
        let levels =
            rvt::research::partition_schema_mvp::level_element_ids(&mut rf, 2027).expect("levels");
        let assoc = rvt::partition_assoc_levels::assoc_level_ids(&mut rf, 2027, &levels);
        assert_eq!(assoc.len(), expected, "{file}");
    }
}

/// The two Snowdon Structural elements with no Level take it from the record.
#[test]
fn levelless_2027_elements_are_bound() {
    let Some(path) = sample("2027/Snowdon_Towers_Sample_Structural__pf2027.rvt") else {
        return;
    };
    let mut rf = RevitFile::open(&path).expect("open");
    let model = RvtDocExporter
        .export_with_diagnostics(&mut rf)
        .expect("export")
        .model;
    let ifcx = write_ifcx(&model).expect("ifcx");
    assert_eq!(ifcx.matches("partition_association_record").count(), 2);
}
