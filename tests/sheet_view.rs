//! `rvt-sheet --view` on a real plan view: rac_basic_sample_project 2023 (`racbasicsampleproject.rvt`
//! in `RVT_REVIT_2023_DIR`, from the download host, see `research/autodesk-sample-projects.tsv`),
//! plan view "Level 2", drawn as DXF and read back with `rvt-dwg`.
//!
//! - The view placed on a sheet is the one drawn, and its hidden categories (Planting among them)
//!   and its 13 hidden elements are read.
//! - Its dimension strings read back as the distances they measure: the grid strings' ten 3000 mm
//!   bays and one 6000 mm bay, and a stair's 900 mm (the authoring tool's own DWG export of this
//!   view holds the same eleven grid dimensions).
//! - Elements are on the export's layers: interior walls on `I-WALL`, stairs on `S-STRS`, curtain
//!   panels on `A-GLAZ-CURT`, text notes on `G-ANNO-TEXT`.
//! - Plan view "Level 1" draws the doors' plan symbols (their swings, as lines on `A-DOOR`), the
//!   stairs above the cut plane as overhead lines on `S-STRS-MBND`, the project's model lines on
//!   `A-DETL` and its rooms on `A-AREA-BNDY`, as the export does; lines that only shape another
//!   element (a wall's or a stair's sketch) are not drawn.
//! - Plan view "Level 2" draws its grids with their heads (circles and names on `S-GRID-IDEN`).
//! - Plan view "Site" is turned to true north: it is drawn in the view's own axes, so its walls
//!   run at 37 degrees as in the authoring tool's export of it, and the grids it hides are left out.
//!   Its property lines are drawn on `C-PROP-LINE`.
#![cfg(feature = "dwg")]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

fn sample() -> Option<PathBuf> {
    let Some(dir) = std::env::var_os("RVT_REVIT_2023_DIR").map(PathBuf::from) else {
        eprintln!("skipping: RVT_REVIT_2023_DIR is not set");
        return None;
    };
    let path = dir.join("racbasicsampleproject.rvt");
    if !path.exists() {
        eprintln!("skipping: {} absent", path.display());
        return None;
    }
    Some(path)
}

fn sheet(args: &[&std::ffi::OsStr]) -> std::process::Output {
    let out = Command::new(env!("CARGO_BIN_EXE_rvt-sheet"))
        .args(args)
        .output()
        .expect("spawn rvt-sheet");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

#[test]
fn level_2_reads_its_visibility_and_draws_its_dimensions_on_the_export_layers() {
    let Some(rvt) = sample() else { return };
    let views: Vec<serde_json::Value> =
        serde_json::from_slice(&sheet(&[rvt.as_os_str(), "--list-views".as_ref()]).stdout)
            .expect("views JSON");
    let level_2: Vec<&serde_json::Value> =
        views.iter().filter(|v| v["name"] == "Level 2").collect();
    let placed: Vec<&&serde_json::Value> =
        level_2.iter().filter(|v| v["on_sheet"] == true).collect();
    assert_eq!(placed.len(), 1, "one Level 2 on a sheet: {level_2:?}");
    let hidden = placed[0]["hidden_categories"].as_array().expect("hidden");
    // BuiltInCategory OST_Planting.
    assert!(hidden.iter().any(|c| c == -2_001_360), "{hidden:?}");
    assert_eq!(
        placed[0]["hidden_elements"].as_array().map(Vec::len),
        Some(13)
    );

    let dir = std::env::temp_dir().join(format!("rvt-sheet-view-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dxf = dir.join("level-2.dxf");
    sheet(&[
        rvt.as_os_str(),
        "--view".as_ref(),
        "Level 2".as_ref(),
        "--format".as_ref(),
        "dxf".as_ref(),
        "-o".as_ref(),
        dxf.as_os_str(),
    ]);
    let read = rvt_dwg::read_file(&dxf, rvt_dwg::Limits::DEFAULT).expect("read back");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dimensions: Vec<i64> = Vec::new();
    let mut layers: BTreeMap<String, usize> = BTreeMap::new();
    for e in &read.drawing.entities {
        *layers.entry(e.layer.clone()).or_default() += 1;
        if let rvt_dwg::Geom::Dimension { measurement, .. } = e.geom {
            dimensions.push(measurement.round() as i64);
        }
    }
    dimensions.sort();
    let mut want = vec![900, 6000];
    want.extend([3000; 10]);
    want.sort();
    assert_eq!(dimensions, want);
    for layer in [
        "I-WALL",
        "A-WALL",
        "S-STRS",
        "A-GLAZ-CURT",
        "G-ANNO-TEXT",
        "S-GRID",
        "S-GRID-IDEN",
    ] {
        assert!(layers.contains_key(layer), "{layer}: {layers:?}");
    }
    for old in ["A-FLOR-STRS", "A-ANNO-NOTE"] {
        assert!(!layers.contains_key(old), "{old}: {layers:?}");
    }
}

fn draw(rvt: &std::path::Path, view: &str) -> rvt_dwg::Drawing2d {
    let dir = std::env::temp_dir().join(format!(
        "rvt-sheet-view-{}-{}",
        std::process::id(),
        view.replace(' ', "-")
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let dxf = dir.join("view.dxf");
    sheet(&[
        rvt.as_os_str(),
        "--view".as_ref(),
        view.as_ref(),
        "--format".as_ref(),
        "dxf".as_ref(),
        "-o".as_ref(),
        dxf.as_os_str(),
    ]);
    let read = rvt_dwg::read_file(&dxf, rvt_dwg::Limits::DEFAULT).expect("read back");
    let _ = std::fs::remove_dir_all(&dir);
    read.drawing
}

#[test]
fn level_1_draws_door_symbols_overhead_stairs_model_lines_and_rooms() {
    let Some(rvt) = sample() else { return };
    let d = draw(&rvt, "Level 1");
    let count = |layer: &str, kind: &str| {
        d.entities
            .iter()
            .filter(|e| e.layer == layer && e.kind == kind)
            .count()
    };
    // Door outlines are polylines; their plan symbols are lines.
    assert!(count("A-DOOR", "LINE") > 0, "no door symbol lines");
    assert!(count("S-STRS-MBND", "LINE") > 0, "no overhead stair lines");
    assert!(count("A-DETL", "LINE") > 0, "no model lines");
    assert!(count("A-AREA-BNDY", "LWPOLYLINE") > 0, "no room polylines");
    // Model lines go on the export's `Lines` layer, not the detail items' one.
    assert_eq!(count("A-DETL-GENF", "LINE"), 0);
}

#[test]
fn site_is_drawn_turned_to_true_north_with_its_property_lines_without_its_hidden_grids() {
    let Some(rvt) = sample() else { return };
    let d = draw(&rvt, "Site");
    assert!(!d.entities.iter().any(|e| e.layer == "S-GRID"));
    assert!(
        d.entities.iter().any(|e| e.layer == "C-PROP-LINE"),
        "no property lines"
    );
    // Every wall edge's direction, modulo a quarter turn, weighted by length.
    let mut by_angle: BTreeMap<i64, f64> = BTreeMap::new();
    for e in d.entities.iter().filter(|e| e.layer == "A-WALL") {
        // Walls below the cut plane are their edges (lines), cut ones their outlines.
        let pts: Vec<[f64; 3]> = match &e.geom {
            rvt_dwg::Geom::Line { a, b } => vec![*a, *b],
            rvt_dwg::Geom::Polyline { pts, closed, .. } => {
                let mut v = pts.clone();
                if *closed && !pts.is_empty() {
                    v.push(pts[0]);
                }
                v
            }
            _ => continue,
        };
        for w in pts.windows(2) {
            let (dx, dy) = (w[1][0] - w[0][0], w[1][1] - w[0][1]);
            let degrees = dy.atan2(dx).to_degrees().rem_euclid(90.0);
            *by_angle.entry(degrees.round() as i64 % 90).or_default() += dx.hypot(dy);
        }
    }
    let main = by_angle
        .iter()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map(|(a, _)| *a);
    assert_eq!(main, Some(37), "{by_angle:?}");
}
