//! A beam drawn along its line whose record box is more than five times as
//! wide or deep as its own saved mesh is drawn from the mesh. No beam of
//! Autodesk's public structural samples is that far from its mesh (the
//! Snowdon Towers structural model's beams reach 2.37 times its width), so on
//! them every beam drawn along its line without the saved graphics stays so
//! with them. Reads `RVT_REVIT_2023_DIR` and `RVT_REVIT_2024_DIR` (from the
//! download host, see `research/autodesk-sample-projects.tsv`); skips what is
//! absent.

use rvt::RevitFile;
use rvt::ifc::{ExportOptions, SavedMeshChoice, run_export};
use std::path::PathBuf;

fn sample(variable: &str, name: &str) -> Option<PathBuf> {
    let Some(directory) = std::env::var_os(variable) else {
        eprintln!("skipping: {variable} is not set");
        return None;
    };
    let path = PathBuf::from(directory).join(name);
    if !path.is_file() {
        eprintln!("skipping: public sample {name} is unavailable");
        return None;
    }
    Some(path)
}

/// Beams drawn along their line, by the export's body sources.
fn beams_along_their_line(path: &PathBuf, saved_meshes: SavedMeshChoice) -> u64 {
    let mut file = RevitFile::open(path).expect("open public sample");
    let run = run_export(
        &mut file,
        &ExportOptions::default().with_saved_meshes(saved_meshes),
    )
    .expect("export public sample");
    assert!(run.unread.is_empty(), "public sample export is incomplete");
    serde_json::to_value(&run.diagnostics)
        .unwrap()
        .pointer("/exported/body_sources/partition_beam_axis")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0)
}

fn assert_no_beam_leaves_its_line(variable: &str, name: &str) {
    let Some(path) = sample(variable, name) else {
        return;
    };
    let without = beams_along_their_line(&path, SavedMeshChoice::Off);
    assert!(without > 0, "{name}: no beam drawn along its line");
    assert_eq!(
        beams_along_their_line(&path, SavedMeshChoice::Default),
        without,
        "{name}: a beam within five times its mesh left its line body"
    );
}

#[test]
fn structural_basic_2023_beams_keep_their_line_bodies() {
    assert_no_beam_leaves_its_line("RVT_REVIT_2023_DIR", "rstbasicsampleproject.rvt");
}

#[test]
fn snowdon_structural_beams_keep_their_line_bodies() {
    assert_no_beam_leaves_its_line("RVT_REVIT_2024_DIR", "snowdon_towers_sample_structural.rvt");
}
