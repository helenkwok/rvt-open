//! The saved-mesh pass reads the big Nordic LCA timber houses.
//!
//! `Global/DocumentIncrementTable` of these two Revit 2024 houses holds about
//! 106,000 and 110,000 values, over the default 100,000-value decode budget, so
//! `saved_meshes::attach` used to fail ("native field value budget exceeded")
//! and every family instance stayed a box. Same fix as.
//!
//! Runs against `RVT_NORDIC_LCA_DIR` (BIM4LCA, CC BY-SA 4.0). Skips when absent.

use rvt::RevitFile;
use rvt::ifc::RvtDocExporter;
use rvt::ifc::saved_meshes;
use std::path::PathBuf;

/// the default export already draws doors and windows from their saved
/// meshes, so fewer bodies are left for this pass (1083 and 1022,
/// decode whole (865 and 866 before) now that the saved-graphics reader walks
/// `GGTag` groups, skips `GBitmap`/`GBiFlipControl` symbols, draws split faces
/// whole and tessellates convex non-rectangular cylinder trims; the
/// candidates are unchanged (walls, floors and roofs get display bodies in a
/// separate pass, not counted here).
/// Three more of each since gutters are recovered from their element records
///: the three gutters of each house are drawn from their saved meshes.
const HOUSES: [(&str, usize, usize); 2] = [
    (
        "ARK_NordicLCA_Housing_Timber_BuildingPermit.rvt",
        1084,
        1019,
    ),
    ("ARK_NordicLCA_Housing_Timber_As-Built.rvt", 1085, 1020),
];

fn timber_houses_read_their_saved_meshes(k: usize, n: usize) {
    let Some(dir) = std::env::var_os("RVT_NORDIC_LCA_DIR").map(PathBuf::from) else {
        eprintln!("skipping: RVT_NORDIC_LCA_DIR is not set");
        return;
    };
    let mut seen = Vec::new();
    for (name, candidates, replaced) in crate::shard(HOUSES, k, n) {
        let path = dir.join(name);
        if !path.exists() {
            eprintln!("skipping {name}: model absent");
            continue;
        }
        let mut rf = RevitFile::open(&path).expect("open");
        let mut model = RvtDocExporter
            .export_with_diagnostics(&mut rf)
            .expect("export")
            .model;
        let report = saved_meshes::attach(&mut rf, &mut model)
            .unwrap_or_else(|e| panic!("{name}: saved graphics not read: {e}"));
        seen.push((
            name,
            (report.candidates, report.replaced),
            (candidates, replaced),
        ));
    }
    // Every house is read before the comparison, so a change shows all of them.
    for (name, got, want) in &seen {
        assert_eq!(got, want, "{name}; all: {seen:?}");
    }
}

split!(timber_houses_read_their_saved_meshes, timber_houses_read_their_saved_meshes, s0 0, s1 1);

/// embedded family documents and Project Information names
/// (runs against `RVT_AUTODESK_SAMPLES_DIR`, skips when absent).
mod family_documents_361 {
    use rvt::RevitFile;
    use rvt::family_documents::{self, Route};
    use rvt::ifc::RvtDocExporter;
    use rvt::ifc::entities::{IfcEntity, PropertyValue};
    use std::path::PathBuf;

    fn rac_basic(year: u32) -> Option<PathBuf> {
        let dir = std::env::var_os("RVT_AUTODESK_SAMPLES_DIR").map(PathBuf::from)?;
        let path = dir.join(format!("{year}/rac_basic_sample_project.rvt"));
        path.exists().then_some(path)
    }

    #[test]
    fn every_content_document_is_found_and_extrusions_match_their_boxes() {
        let Some(path) = rac_basic(2024) else {
            eprintln!("skipping: rac_basic 2024 absent");
            return;
        };
        let mut rf = RevitFile::open(&path).expect("open");
        let docs = family_documents::read(&mut rf).expect("read");
        // 163 keyed sections, each with its own GUID, all named by a host family.
        assert_eq!(docs.stats.sections, 163);
        assert_eq!(docs.docs.len(), 163);
        assert!(
            docs.family_guid.values().all(|g| docs.docs.contains_key(g)),
            "a host family names a document the stream does not hold"
        );
        assert_eq!(docs.stats.extrusions, 435);
        // A solid counts only when its bounding box equals the stored one:
        // the extrusions that match, and the sweeps, revolutions and blends
        // solved from their sketches (each also checked against its box).
        assert_eq!(
            docs.stats.extrusions_bbox_match + docs.stats.solved.values().sum::<usize>(),
            docs.docs.values().map(|d| d.solids.len()).sum::<usize>()
        );
        assert!(docs.stats.extrusions_decoded >= 374);
        assert!(docs.stats.extrusions_bbox_match >= 367);
        // System families (curtain mullions and panels) have no document.
        let none = docs
            .instances
            .keys()
            .filter(|id| docs.route(**id) == Route::NoFamilyDocument)
            .count();
        assert!(none >= 190, "{none}");
    }

    #[test]
    fn site_and_building_take_the_project_information_names() {
        let Some(path) = rac_basic(2024) else {
            eprintln!("skipping: rac_basic 2024 absent");
            return;
        };
        let mut rf = RevitFile::open(&path).expect("open");
        let model = RvtDocExporter
            .export_with_diagnostics(&mut rf)
            .expect("export")
            .model;
        assert_eq!(model.site_name.as_deref(), Some("Sample House"));
        assert_eq!(
            model.building_name.as_deref(),
            Some("Samuel Macalister sample house design")
        );
    }

    #[test]
    fn family_bodies_replace_boxes_and_carry_a_material() {
        let Some(path) = rac_basic(2024) else {
            eprintln!("skipping: rac_basic 2024 absent");
            return;
        };
        let mut rf = RevitFile::open(&path).expect("open");
        let mut model = RvtDocExporter
            .export_with_diagnostics(&mut rf)
            .expect("export")
            .model;
        let before = rvt::convert::count_without_material(&model);
        let report = rvt::ifc::family_bodies::attach(&mut rf, &mut model).expect("attach");
        assert!(report.candidates > 300, "{report:?}");
        assert!(report.replaced > 0, "{report:?}");
        let drawn = model
            .entities
            .iter()
            .filter(|e| {
                matches!(e, IfcEntity::BuildingElement { property_set: Some(set), .. }
                    if set.properties.iter().any(|p| p.name == "BodySource"
                        && matches!(&p.value, PropertyValue::Text(t) if t == "family_document_geometry" || t == "family_document_geometry_scaled")))
            })
            .count();
        assert_eq!(drawn, report.replaced);
        assert!(rvt::convert::count_without_material(&model) <= before);
    }

    #[test]
    fn an_older_release_reads_without_a_document_and_falls_through() {
        let Some(path) = rac_basic(2018) else {
            eprintln!("skipping: rac_basic 2018 absent");
            return;
        };
        let mut rf = RevitFile::open(&path).expect("open");
        let docs = family_documents::read(&mut rf).expect("read");
        // Whatever sections the release has, every document is one of them and
        // an instance without a complete document is not given a body.
        assert_eq!(docs.docs.len(), docs.stats.sections);
        for id in docs.instances.keys() {
            if docs.route(*id) == Route::FamilyDocument {
                assert!(docs.solids_for(*id).is_some_and(|(_, d)| d.complete()));
            }
        }
    }
}
