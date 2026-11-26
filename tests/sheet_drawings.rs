//! Sheets drawn whole, on the public Technical School sample project saved by Revit 2026
//! (`Technicalschoolcurrentm.rvt` in `RVT_REVIT_2026_DIR`, from the download host, see
//! `research/autodesk-sample-projects.tsv`): sheet A1 (a floor plan) and A2 (four sections and
//! details and a drafting view).
//!
//! - Every viewport lies inside the sheet, each view drawn about where its viewport's offset places
//!   the view's origin, inside the view's crop: before, a section was placed by the model's origin
//!   instead of its crop box's, which put the 1:10 details 0.4 m left of the sheet and 0.6 m above
//!   it.
//! - The sections and details draw their lines inside their viewports, and the drafting view
//!   draws its imported drawing's lines.
//! - The title block's labels show the sheet's and the project's values (sheet name and number,
//!   project name and number, client, issue date, drawn and checked by, the scale), placed as the
//!   authoring tool's own PDF of the sheets places them.
#![cfg(feature = "dwg")]

use rvt::RevitFile;
use rvt::ifc::sheet_views::{DrawingKey, view_drawing};
use rvt::ifc::{Exporter, RvtDocExporter};
use rvt_dwg::{Drawing2d, Geom};
use std::path::PathBuf;

/// The A0 sheet, millimetres.
const SHEET: [f64; 4] = [0.0, 0.0, 1189.0, 840.0];

fn sample() -> Option<PathBuf> {
    let Some(dir) = std::env::var_os("RVT_REVIT_2026_DIR").map(PathBuf::from) else {
        eprintln!("skipping: RVT_REVIT_2026_DIR is not set");
        return None;
    };
    let path = dir.join("Technicalschoolcurrentm.rvt");
    if !path.exists() {
        eprintln!("skipping: {} absent", path.display());
        return None;
    }
    Some(path)
}

struct Drawn {
    drawing: Drawing2d,
    notes: Vec<String>,
}

fn sheets() -> Option<(Drawn, Drawn)> {
    let path = sample()?;
    let mut rf = RevitFile::open(&path).expect("open");
    let facts = rvt::plan_views::read(&mut rf).expect("views");
    let model = RvtDocExporter.export(&mut rf).expect("export");
    let symbols = rvt::ifc::plan_maps::instance_plan_symbols(&mut rf, &model).unwrap_or_default();
    let mut draw = |number: &str| {
        let sheet = facts.find_sheet(number).expect("sheet");
        let mut notes = Vec::new();
        let drawing = view_drawing(
            &mut rf,
            &model,
            &facts,
            DrawingKey::Sheet(sheet.id),
            &symbols,
            &rvt::ifc::dwg_layers::ExportLayers::default(),
            &mut notes,
        )
        .expect("draw")
        .drawing;
        Drawn { drawing, notes }
    };
    Some((draw("A1"), draw("A2")))
}

/// Each viewport: its frame on the sheet and the model-space region it shows.
fn viewports(d: &Drawing2d) -> Vec<([f64; 4], [f64; 4])> {
    d.entities
        .iter()
        .filter_map(|e| match e.geom {
            Geom::Viewport {
                center,
                width,
                height,
                view_center,
                view_height,
                ..
            } => {
                let k = view_height / height;
                Some((
                    [
                        center[0] - width / 2.0,
                        center[1] - height / 2.0,
                        center[0] + width / 2.0,
                        center[1] + height / 2.0,
                    ],
                    [
                        view_center[0] - width * k / 2.0,
                        view_center[1] - view_height / 2.0,
                        view_center[0] + width * k / 2.0,
                        view_center[1] + view_height / 2.0,
                    ],
                ))
            }
            _ => None,
        })
        .collect()
}

/// Model-space line ends inside `r`.
fn lines_in(d: &Drawing2d, r: [f64; 4]) -> usize {
    d.entities
        .iter()
        .filter(|e| e.owner == rvt_dwg::MODEL_SPACE)
        .filter(|e| match e.geom {
            Geom::Line { a, .. } => a[0] >= r[0] && a[0] <= r[2] && a[1] >= r[1] && a[1] <= r[3],
            _ => false,
        })
        .count()
}

fn paper_texts(d: &Drawing2d) -> Vec<(String, [f64; 3])> {
    d.entities
        .iter()
        .filter(|e| e.owner != rvt_dwg::MODEL_SPACE)
        .filter_map(|e| match &e.geom {
            Geom::Text { value, at, .. } => Some((value.clone(), *at)),
            _ => None,
        })
        .collect()
}

#[test]
fn sheets_place_every_view_inside_the_sheet_and_label_the_title_block() {
    let Some((a1, a2)) = sheets() else { return };
    for (name, s, views) in [("A1", &a1, 1), ("A2", &a2, 5)] {
        let vps = viewports(&s.drawing);
        assert_eq!(vps.len(), views, "{name}: {vps:?}");
        for (frame, region) in &vps {
            assert!(
                frame[0] >= SHEET[0] - 1.0
                    && frame[1] >= SHEET[1] - 1.0
                    && frame[2] <= SHEET[2] + 1.0
                    && frame[3] <= SHEET[3] + 1.0,
                "{name}: viewport {frame:?} leaves the sheet"
            );
            let n = lines_in(&s.drawing, *region);
            assert!(n >= 40, "{name}: viewport {frame:?} shows {n} lines");
        }
    }
    // A2's sections and details, by their frames' centres (millimetres), against the viewport
    // centres the authoring tool reports for the same sheet, within the annotation it frames and
    // this drawing does not.
    let centres: Vec<[f64; 2]> = viewports(&a2.drawing)
        .iter()
        .map(|(f, _)| [(f[0] + f[2]) / 2.0, (f[1] + f[3]) / 2.0])
        .collect();
    for expected in [
        [661.1, 696.9], // Section Through Main Stair, 1:100
        [172.4, 435.1], // Typical Wall Section, 1:20
        [457.5, 157.3], // Detail At Grade, 1:10
        [448.6, 375.7], // Detail At Parapet, 1:10
    ] {
        assert!(
            centres
                .iter()
                .any(|c| (c[0] - expected[0]).hypot(c[1] - expected[1]) < 25.0),
            "no viewport near {expected:?}: {centres:?}"
        );
    }
    assert!(
        !a2.notes.iter().any(|n| n.contains("not a plan")),
        "{:?}",
        a2.notes
    );

    let texts = paper_texts(&a2.drawing);
    for value in [
        "Sections",
        "A2",
        "Project Name",
        "Project Number",
        "Owner",
        "Issue Date",
        "Author",
        "Checker",
        "As indicated",
    ] {
        assert!(
            texts.iter().any(|(v, _)| v == value),
            "A2 label {value:?} missing: {texts:?}"
        );
    }
    let a1_texts = paper_texts(&a1.drawing);
    for value in ["Floor Plan", "A1", "1 : 100"] {
        assert!(
            a1_texts.iter().any(|(v, _)| v == value),
            "A1 label {value:?} missing"
        );
    }
    // The sheet number is centred in the title block's last box (1093 mm from the left in the
    // authoring tool's PDF) and the drawn-by value ends at its right edge (1156 mm).
    let starts = |value: &str| -> Vec<f64> {
        texts
            .iter()
            .filter(|(v, at)| v == value && at[0] > 1000.0)
            .map(|(_, at)| at[0])
            .collect()
    };
    assert!(
        starts("A2").iter().any(|x| (1075.0..1093.0).contains(x)),
        "{:?}",
        starts("A2")
    );
    assert!(
        starts("Author")
            .iter()
            .any(|x| (1130.0..1150.0).contains(x)),
        "{:?}",
        starts("Author")
    );
}
