//! Releases whose files exported no building elements.
//!
//! The oracle is the full Revit API dump of the same files: the
//! pinned counts are the elements `rvt-ifc` exports, each measured against
//! that dump (category equal on 99% of them, no type or level link wrong
//! beyond a handful). The files are Autodesk samples and Snowdon Towers
//! (CC BY-NC-SA) and are not redistributed: point `RVT_RELEASE_SAMPLES_DIR`
//! at a directory laid out `<year>/<label>.rvt`; without it the tests skip.
//!
//! - 2018, 2019, 2022: records have the 2023 shape (`u32` ElementId,
//!   category 38 bytes before the marker).
//! - 2023 files saved from older ones: an ElemTable whose ids fall a few
//!   times, and a record header up to hundreds of bytes before the marker.
//! - 2027: the 2026 record shape under its own marker.

use rvt::RevitFile;
use rvt::ifc::RvtDocExporter;
use std::path::PathBuf;

fn sample(year: u32, label: &str) -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("RVT_RELEASE_SAMPLES_DIR")?);
    let path = dir.join(year.to_string()).join(format!("{label}.rvt"));
    path.exists().then_some(path)
}

/// Exported building elements of one sample, or `None` when it is absent.
fn building_elements(year: u32, label: &str) -> Option<usize> {
    let Some(path) = sample(year, label) else {
        eprintln!("skipping: no {year}/{label}.rvt in RVT_RELEASE_SAMPLES_DIR");
        return None;
    };
    let mut rf = RevitFile::open(&path).expect("open");
    let result = RvtDocExporter
        .export_with_diagnostics(&mut rf)
        .expect("export");
    Some(result.diagnostics.exported.building_elements)
}

#[test]
fn releases_2018_2019_2022_export_their_elements() {
    for (year, label, expected) in [
        (2018, "rac_basic_sample_project__pf2018", 464),
        (2019, "rac_basic_sample_project__pf2019", 464),
        (2022, "rac_basic_sample_project__pf2022", 464),
        (2022, "cjk_60930d75__pf2022", 2008),
    ] {
        if let Some(n) = building_elements(year, label) {
            assert_eq!(n, expected, "{year} {label}");
        }
    }
}

#[test]
fn levels_of_2018_2019_2022_are_read() {
    for (year, label) in [
        (2018, "rac_basic_sample_project__pf2018"),
        (2019, "rac_basic_sample_project__pf2019"),
        (2022, "rac_basic_sample_project__pf2022"),
    ] {
        let Some(path) = sample(year, label) else {
            eprintln!("skipping: no {year}/{label}.rvt in RVT_RELEASE_SAMPLES_DIR");
            continue;
        };
        let mut rf = RevitFile::open(&path).expect("open");
        let ids =
            rvt::research::partition_schema_mvp::level_element_ids(&mut rf, year).expect("levels");
        // the six Levels of the API dump; a seventh record of the Levels
        // category names an owner and is not a Level
        assert_eq!(ids.len(), 6, "{year} {label}");
    }
}

#[test]
fn a_2023_elem_table_with_falling_ids_is_read_whole() {
    let Some(path) = sample(2023, "cjk_cd52123f__pf2023") else {
        eprintln!("skipping: no 2023/cjk_cd52123f__pf2023.rvt in RVT_RELEASE_SAMPLES_DIR");
        return;
    };
    let mut rf = RevitFile::open(&path).expect("open");
    let layout = rvt::elem_table::read_layout(&mut rf).expect("layout");
    assert_eq!(
        layout.stride, 28,
        "28-byte records, not the implicit fallback"
    );
    let records = rvt::elem_table::parse_records(&mut rf).expect("records");
    assert_eq!(records.len(), 44_108);
    assert_eq!(rvt::elem_table::declared_ids(&records).len(), 44_108);
}

#[test]
fn the_2023_files_that_exported_nothing_export_their_elements() {
    for (label, expected) in [
        ("cjk_cd52123f__pf2023", 5821),
        ("cjk_e618e87d__pf2023", 4793),
        // a file that exported 1,064 of its elements before: records whose
        // header sits far behind the marker were missed
        ("cjk_b46ce352__pf2023", 2008),
    ] {
        if let Some(n) = building_elements(2023, label) {
            assert_eq!(n, expected, "2023 {label}");
        }
    }
}

#[test]
fn release_2027_exports_its_elements() {
    for (label, expected) in [
        ("Snowdon_Towers_Sample_Site__pf2027", 78),
        ("Pacific_Continental_Residence_Sample__pf2027", 662),
    ] {
        if let Some(n) = building_elements(2027, label) {
            assert_eq!(n, expected, "2027 {label}");
        }
    }
}
