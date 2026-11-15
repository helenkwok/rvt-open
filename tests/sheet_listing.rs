//! What `rvt-sheet --list-views` and `--list-sheets` list can be drawn, on a public project with
//! many sheets: BIM4LCA's Office Terrain Concrete BuildingPermit (VTT, CC BY-SA 4.0,
//! `RVT_NORDIC_LCA_DIR`, see tools/ci/fetch-public-samples.sh).
//!
//! - Every plan view listed (view templates and loaded families' own plan views left out) is
//!   drawn by its name and by its ElementId, and every section or elevation with a crop box is
//!   drawn: none is refused.
//! - Sheets whose drawings are byte-identical are only sheets with no view placed on them, which
//!   draw nothing but their title block, and the drawing says so.

#![cfg(feature = "dwg")]

use rvt::RevitFile;
use rvt::ifc::sheet_views::{DrawingKey, drawing_bytes, view_drawing};
use rvt::ifc::{Exporter, RvtDocExporter};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn sample() -> Option<PathBuf> {
    let Some(dir) = std::env::var_os("RVT_NORDIC_LCA_DIR").map(PathBuf::from) else {
        eprintln!("skipping: RVT_NORDIC_LCA_DIR is not set");
        return None;
    };
    let path = dir.join("ARK_NordicLCA_Office_Terrain_Concrete_BuildingPermit.rvt");
    if !path.exists() {
        eprintln!("skipping: {} absent", path.display());
        return None;
    }
    Some(path)
}

#[test]
fn every_listed_view_is_drawn_and_only_sheets_without_views_draw_alike() {
    let Some(path) = sample() else { return };
    let mut rf = RevitFile::open(&path).expect("open");
    let facts = rvt::plan_views::read(&mut rf).expect("views");
    let model = RvtDocExporter.export(&mut rf).expect("export");
    let symbols = rvt::ifc::plan_maps::instance_plan_symbols(&mut rf, &model).unwrap_or_default();

    let plans: Vec<_> = facts.views.iter().filter(|v| !v.template).collect();
    // A loaded family's plan views are all on its `Ref. Level`.
    assert!(
        !plans
            .iter()
            .any(|v| v.level.as_deref() == Some("Ref. Level")),
        "a loaded family's plan view is listed"
    );
    assert!(!plans.is_empty());
    for v in &plans {
        assert_eq!(facts.find(&v.id.to_string()).map(|f| f.id), Ok(v.id));
        let by_name = facts.find(&v.name).unwrap_or_else(|e| panic!("{e}"));
        let mut notes = Vec::new();
        view_drawing(
            &mut rf,
            &model,
            &facts,
            DrawingKey::Plan(by_name.id),
            &symbols,
            &mut notes,
        )
        .unwrap_or_else(|e| panic!("plan {:?}: {e}", v.name));
    }
    for v in facts
        .sections
        .iter()
        .filter(|v| !v.template && v.crop.is_some())
    {
        let mut notes = Vec::new();
        view_drawing(
            &mut rf,
            &model,
            &facts,
            DrawingKey::Section(v.id),
            &symbols,
            &mut notes,
        )
        .unwrap_or_else(|e| panic!("section {:?}: {e}", v.name));
    }

    let mut by_bytes: BTreeMap<Vec<u8>, Vec<(String, bool)>> = BTreeMap::new();
    for s in &facts.sheets {
        let mut notes = Vec::new();
        let d = view_drawing(
            &mut rf,
            &model,
            &facts,
            DrawingKey::Sheet(s.id),
            &symbols,
            &mut notes,
        )
        .unwrap_or_else(|e| panic!("sheet {}: {e}", s.number));
        let (bytes, _) = drawing_bytes(&d.drawing, d.layout, d.scale, "dxf", "2018").expect("dxf");
        let empty = notes.iter().any(|n| n.contains("no view is placed on it"));
        by_bytes
            .entry(bytes)
            .or_default()
            .push((s.number.clone(), empty));
    }
    assert_eq!(facts.sheets.len(), 40);
    for sheets in by_bytes.values().filter(|g| g.len() > 1) {
        assert!(
            sheets.iter().all(|(_, empty)| *empty),
            "sheets with a view placed draw alike: {sheets:?}"
        );
    }
}
