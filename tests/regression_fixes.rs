//! Regression checks for fixed bugs, each measured on real files.
//!
//! Runs against `RVT_REVIT_EXTRA_DIR` (`<release>/<label>.rvt`, Revit sample projects).

use rvt::RevitFile;
use rvt::ifc::RvtDocExporter;
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

/// A file with no PartAtom title is named by the file name in its original
/// path, never by the path with its folders (an author's account or a client folder).
fn project_name_is_the_file_name_not_the_path(k: usize, n: usize) {
    let files = [
        "2019/rac_basic_sample_project__pf2019.rvt",
        "2023/rac_basic_sample_project__pf2023.rvt",
        "2027/Pacific_Continental_Residence_Sample__pf2027.rvt",
    ];
    for file in crate::shard(files, k, n) {
        let Some(path) = sample(file) else { continue };
        let mut rf = RevitFile::open(&path).expect("open");
        let original = rf
            .basic_file_info()
            .ok()
            .and_then(|info| info.original_path)
            .expect("original path");
        let model = RvtDocExporter
            .export_with_diagnostics(&mut rf)
            .expect("export")
            .model;
        let name = model.project_name.expect("project name");
        assert!(
            !name.contains('\\') && !name.contains('/'),
            "{file}: {name}"
        );
        let last = original.rsplit(['\\', '/']).next().unwrap();
        assert!(
            last.starts_with(&name),
            "{file}: {name} is not the start of {last}"
        );
    }
}

split!(project_name_is_the_file_name_not_the_path, project_name_is_the_file_name_not_the_path, s0 0, s1 1, s2 2);

/// The header's record count is the `u32` the table states. Three of our
/// tables state more than 65,535 records (81,034 to 81,291 here), where the `u16` kept the
/// low 16 bits (15,498 to 15,755).
#[test]
fn elem_table_record_count_is_the_stated_u32() {
    for (file, stated) in [
        ("2022/cjk_60930d75__pf2022.rvt", 81_034),
        ("2023/cjk_b46ce352__pf2023.rvt", 81_139),
        ("2027/cjk_89244e25__pf2027.rvt", 81_291),
    ] {
        let Some(path) = sample(file) else { continue };
        let mut rf = RevitFile::open(&path).expect("open");
        let header = rvt::elem_table::parse_header(&mut rf).expect("header");
        let records = rvt::elem_table::parse_records(&mut rf).expect("records");
        assert_eq!(header.record_count, stated, "{file}");
        assert_eq!(records.len(), stated as usize, "{file}");
    }
}

/// Building Story: on the 32-bit releases the setting follows `ff ff ff ff 00 01`,
/// an `f64` and a `u32` ElementId. Counts are the API dump's Building Story parameter
/// (rac_basic: 6 Levels, all stories; rst_basic: 9 Levels, none a story).
#[test]
fn building_story_is_read_in_its_32_bit_form() {
    for (file, version, levels, stories) in [
        ("2018/rac_basic_sample_project__pf2018.rvt", 2018, 6, 6),
        ("2023/rac_basic_sample_project__pf2023.rvt", 2023, 6, 6),
        ("2023/rst_basic_sample_project__pf2023.rvt", 2023, 9, 0),
    ] {
        let Some(path) = sample(file) else { continue };
        let mut rf = RevitFile::open(&path).expect("open");
        let found =
            rvt::research::partition_level_records::recover_partition_levels(&mut rf, version)
                .expect("levels");
        assert_eq!(found.len(), levels, "{file}");
        assert!(found.iter().all(|l| l.building_story.is_some()), "{file}");
        let read = found
            .iter()
            .filter(|l| l.building_story == Some(true))
            .count();
        assert_eq!(read, stories, "{file}");
    }
}

/// Container: the reference is the `u32` 22 bytes before the marker. Joined on
/// ElementId with the 2024 copy of the same model, it gave 2024's container on every shared
/// record of 34 file pairs; the `u64` at 2024's place gave none of the 2024 container members.
/// Container members of the 2023 files, as the 2024 copies have them.
#[test]
fn container_members_of_2023_records_match_the_2024_copy() {
    for (file, members) in [
        ("2023/rac_basic_sample_project__pf2023.rvt", 3),
        ("2023/rac_advanced_sample_project__pf2023.rvt", 32),
        ("2023/rst_basic_sample_project__pf2023.rvt", 0),
    ] {
        let Some(path) = sample(file) else { continue };
        let mut rf = RevitFile::open(&path).expect("open");
        let found = rvt::research::partition_element_records_2023::scan_records(&mut rf, 2023);
        let read = found.iter().filter(|r| r.is_container_member()).count();
        assert_eq!(read, members, "{file}");
    }
}
