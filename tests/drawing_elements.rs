//! Every drawn line, text and fill of a sheet or view names the element it draws, so a viewer can
//! select that element in the drawing (`Entity2d::element`, written to SVG as `data-element`). On
//! the public Technical School sample project saved by Revit 2026 (`Technicalschoolcurrentm.rvt`
//! in `RVT_REVIT_2026_DIR`, see `research/autodesk-sample-projects.tsv`): sheet A1 (a floor plan
//! and its title block), sheet A2 (sections, details and a drafting view), the floor plan on A1 and
//! a section, each drawn as the viewer draws it.
//!
//! - Each element named is one the file holds: a model element (its ElementId, which the IFC
//!   export keeps in `type_guid`), or an annotation, grid, level, title block or viewport the
//!   drawing read from the file's views.
//! - Most of what is drawn names its element, per drawing kind (the shares are printed; what is
//!   left without one is an imported drawing's lines, which are not an element of their own).
//! - A wall drawn in the plan draws its lines as that wall's.
//! - The SVG carries `data-element` on what it draws, with the same ElementIds, and on each
//!   sheet viewport's group, the viewport's.
#![cfg(feature = "dwg")]

use rvt::RevitFile;
use rvt::ifc::entities::IfcEntity;
use rvt::ifc::sheet_views::{DrawingKey, view_drawing};
use rvt::ifc::{Exporter, RvtDocExporter};
use rvt_dwg::Drawing2d;
use std::collections::BTreeSet;
use std::path::PathBuf;

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

/// Entities that draw something (a viewport is a window onto others, not a drawn line).
fn drawn(d: &Drawing2d) -> impl Iterator<Item = &rvt_dwg::Entity2d> {
    d.entities.iter().filter(|e| e.kind != "VIEWPORT")
}

#[test]
fn drawn_lines_texts_and_fills_name_their_elements() {
    let Some(path) = sample() else { return };
    let mut rf = RevitFile::open(&path).expect("open");
    let facts = rvt::plan_views::read(&mut rf).expect("views");
    let model = RvtDocExporter.export(&mut rf).expect("export");
    let symbols = rvt::ifc::plan_maps::instance_plan_symbols(&mut rf, &model).unwrap_or_default();
    let layers = rvt::ifc::dwg_layers::ExportLayers::default();

    // The elements a drawing may name.
    let mut known: BTreeSet<i64> = BTreeSet::new();
    let mut walls: BTreeSet<i64> = BTreeSet::new();
    for e in &model.entities {
        if let IfcEntity::BuildingElement {
            ifc_type,
            type_guid,
            ..
        } = e
        {
            if let Some(id) = type_guid.as_deref().and_then(|t| t.parse::<i64>().ok()) {
                known.insert(id);
                if ifc_type == "IFCWALL" || ifc_type == "IFCWALLSTANDARDCASE" {
                    walls.insert(id);
                }
            }
        }
    }
    known.extend(facts.texts.iter().map(|t| t.id));
    known.extend(facts.dimensions.iter().map(|t| t.id));
    known.extend(facts.room_tags.iter().map(|t| t.id));
    known.extend(facts.curves.iter().map(|t| t.id));
    known.extend(facts.grids.iter().map(|t| t.id));
    known.extend(facts.levels.iter().map(|t| t.id));
    known.extend(facts.title_blocks.iter().map(|t| t.id));
    known.extend(facts.viewports.iter().map(|t| t.id));

    let a1 = facts.find_sheet("A1").expect("sheet A1");
    let a2 = facts.find_sheet("A2").expect("sheet A2");
    // The plan placed on A1, and the first section that has a crop box.
    let plan = facts
        .viewports
        .iter()
        .filter(|v| v.sheet == a1.id)
        .find_map(|v| facts.views.iter().find(|p| p.id == v.view && !p.template))
        .expect("a plan on A1");
    let section = facts
        .sections
        .iter()
        .find(|s| !s.template && rvt::ifc::sheet_views::section_window(&facts, s).is_some())
        .expect("a section");
    let cases = [
        ("sheet A1", DrawingKey::Sheet(a1.id), Some("Layout1"), 0.80),
        ("sheet A2", DrawingKey::Sheet(a2.id), Some("Layout1"), 0.50),
        ("floor plan", DrawingKey::Plan(plan.id), None, 0.80),
        ("section", DrawingKey::Section(section.id), None, 0.80),
    ];
    for (name, key, layout, floor) in cases {
        let mut notes = Vec::new();
        let v = view_drawing(&mut rf, &model, &facts, key, &symbols, &layers, &mut notes)
            .expect("draw");
        let total = drawn(&v.drawing).count();
        let named: Vec<i64> = drawn(&v.drawing).filter_map(|e| e.element).collect();
        let share = named.len() as f64 / total.max(1) as f64;
        let elements: BTreeSet<i64> = named.iter().copied().collect();
        // Per kind of entity, for the report.
        let mut by_kind: std::collections::BTreeMap<&str, (usize, usize)> = Default::default();
        for e in drawn(&v.drawing) {
            let k = by_kind.entry(e.kind.as_str()).or_default();
            k.0 += usize::from(e.element.is_some());
            k.1 += 1;
        }
        eprintln!(
            "{name}: {} of {total} drawn entities name their element ({:.1}%), {} elements; {by_kind:?}",
            named.len(),
            share * 100.0,
            elements.len()
        );
        assert!(total > 0, "{name}: nothing drawn");
        let unknown: Vec<i64> = elements.difference(&known).copied().collect();
        assert!(
            unknown.is_empty(),
            "{name}: ElementIds the file does not hold: {unknown:?}"
        );
        assert!(
            share >= floor,
            "{name}: only {:.1}% of what is drawn names its element",
            share * 100.0
        );
        if let DrawingKey::Plan(_) = key {
            assert!(
                elements.iter().any(|id| walls.contains(id)),
                "{name}: no wall's lines name the wall"
            );
        }
        if let DrawingKey::Sheet(sheet) = key {
            // The viewport entity and the view's title name the viewport.
            let viewports: BTreeSet<i64> = facts
                .viewports
                .iter()
                .filter(|vp| vp.sheet == sheet)
                .map(|vp| vp.id)
                .collect();
            assert!(
                v.drawing
                    .entities
                    .iter()
                    .filter(|e| e.kind == "VIEWPORT")
                    .all(|e| e.element.is_some_and(|id| viewports.contains(&id))),
                "{name}: a viewport does not name its Viewport element"
            );
        }
        let svg = rvt_dwg::svg::render(&v.drawing, layout, &rvt_dwg::Limits::DEFAULT)
            .expect("svg")
            .svg;
        let in_svg: BTreeSet<i64> = svg
            .split(" data-element=\"")
            .skip(1)
            .filter_map(|s| s.split('"').next()?.parse().ok())
            .collect();
        // A sheet's viewports name themselves on their groups, titled or not.
        let mut expected = elements.clone();
        if let DrawingKey::Sheet(sheet) = key {
            expected.extend(
                facts
                    .viewports
                    .iter()
                    .filter(|vp| vp.sheet == sheet)
                    .map(|vp| vp.id),
            );
        }
        assert!(
            !in_svg.is_empty() && in_svg.is_subset(&expected),
            "{name}: the SVG names {} elements, {} not expected",
            in_svg.len(),
            in_svg.difference(&expected).count()
        );
        let tags = svg.matches(" data-element=\"").count();
        let primitives = svg.matches(" data-handle=\"").count();
        eprintln!(
            "{name}: SVG {tags} data-element of {primitives} drawn primitives ({:.1}%)",
            tags as f64 / primitives.max(1) as f64 * 100.0
        );
    }
}
