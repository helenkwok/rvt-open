//! A blend whose two profiles have different vertex counts (a rectangle to a
//! circle) is drawn from its family document, not left as its box.
//!
//! Autodesk's RME advanced sample project holds 105 instances of
//! `M_Rectangular to Round Duct Transition - Angle`, a family whose one solid
//! form is a blend from a rectangle to a circle. Before, the blend did not
//! solve (the two profiles must have had the same vertex count) and every
//! instance kept its record box. Another converter draws each of them with
//! about 970 triangles; this one now draws its own tessellation of the
//! same blend.
//!
//! Each document draws the family's default type (600 x 362 x 600 mm: a
//! 600 mm square to a 600 mm circle), while these instances are 250 x 62.5
//! x 250 mm. The body is three blends and two extrusions all running along
//! one axis, and the two cross-axis stretches are equal (0.417), so
//! stretching it along that axis (0.173) is the same ruled form between
//! the scaled profiles over a shorter length.
//!
//! Runs against `RVT_REVIT_EXTRA_DIR` (`2023/rme_advanced_sample_project__pf2023.rvt`)
//! and `RVT_REVIT2024_SAMPLES_DIR` (`rmeadvancedsampleproject.rvt`), each
//! when present. Skips what is absent.

use rvt::RevitFile;
use rvt::family_documents::{self, Route};
use rvt::ifc::RvtDocExporter;
use rvt::ifc::entities::{IfcEntity, PropertyValue, SolidShape};
use std::collections::BTreeMap;
use std::path::PathBuf;

const FAMILY: &str = "M_Rectangular to Round Duct Transition - Angle";
/// An element of another family whose document body would need different
/// stretches across its length (8.693, 1.364, 1.203 on the 2024 sample).
const UNEQUAL_STRETCH: &str = "573615";

/// (directory variable, file, the transitions expected to stay boxes for
/// uneven stretches, where measured).
const FILES: [(&str, &str, Option<&[&str]>); 2] = [
    (
        "RVT_REVIT_EXTRA_DIR",
        "2023/rme_advanced_sample_project__pf2023.rvt",
        None,
    ),
    (
        "RVT_REVIT2024_SAMPLES_DIR",
        "rmeadvancedsampleproject.rvt",
        // 250 x 325 mm at one end: stretches 0.173, 0.417, 0.542.
        Some(&["491931"]),
    ),
];

fn transitions_are_drawn_from_their_family(
    env: &str,
    file: &str,
    expected_uneven: Option<&[&str]>,
) {
    let Some(dir) = std::env::var_os(env).map(PathBuf::from) else {
        eprintln!("skipping: {env} is not set");
        return;
    };
    let path = dir.join(file);
    if !path.exists() {
        eprintln!("skipping: {file} absent");
        return;
    }
    let mut rf = RevitFile::open(&path).expect("open");
    let mut model = RvtDocExporter
        .export_with_diagnostics(&mut rf)
        .expect("export")
        .model;
    let report = rvt::ifc::family_bodies::attach(&mut rf, &mut model).expect("attach");
    let docs = family_documents::read(&mut rf).expect("family documents");
    let mut found = 0;
    let mut boxed = Vec::new();
    let mut uneven: Vec<(String, Vec<f64>)> = Vec::new();
    // Why each transition left as a box was not drawn, from its document.
    let mut why: BTreeMap<String, usize> = BTreeMap::new();
    let mut doc_detail = None;
    let mut ids = std::collections::BTreeSet::new();
    for e in &model.entities {
        let IfcEntity::BuildingElement {
            name,
            type_guid,
            property_set: Some(set),
            solid_shape,
            ..
        } = e
        else {
            continue;
        };
        if !name.contains(FAMILY) {
            continue;
        }
        found += 1;
        if let Some(t) = type_guid {
            ids.insert(t.clone());
        }
        let source = set.properties.iter().find_map(|p| match &p.value {
            PropertyValue::Text(t) if p.name == "BodySource" => Some(t.as_str()),
            _ => None,
        });
        if !matches!(
            source,
            Some("family_document_geometry" | "family_document_geometry_scaled")
        ) {
            let id = type_guid.as_deref().and_then(|t| t.parse::<i64>().ok());
            // An instance whose ends are not in the document's proportions
            // (no two of its stretches agree within 1%, e.g. a 250 x 325 mm
            // rectangle where the default is square) cannot be drawn by
            // stretching. Drawing it at its own profile sizes is a separate
            // follow-up (a profile-size check for transitions); until then it
            // stays a box and is listed here.
            if let Some(f) = id.and_then(|id| report.size_rejected_factors.get(&id)) {
                let agree = |a: f64, b: f64| (a - b).abs() <= 0.01 * a.max(b);
                if f.len() == 3 && !agree(f[0], f[1]) && !agree(f[0], f[2]) && !agree(f[1], f[2]) {
                    uneven.push((id.unwrap_or_default().to_string(), f.clone()));
                    continue;
                }
            }
            boxed.push(format!("{name}: {source:?}"));
            let reason = match id.map(|id| (docs.route(id), docs.solids_for(id))) {
                None => "no element id".to_string(),
                Some((Route::NoFamilyDocument, _)) => "no family document".to_string(),
                Some((Route::Incomplete(w), _)) => format!("incomplete {w:?}"),
                Some((Route::FamilyDocument, Some((pose, _))))
                    if pose.flipped_x || pose.flipped_y =>
                {
                    "flipped".to_string()
                }
                Some((Route::FamilyDocument, _)) => "complete (size rejected?)".to_string(),
            };
            *why.entry(reason).or_default() += 1;
            if doc_detail.is_none()
                && let Some((pose, doc)) = id.and_then(|id| docs.solids_for(id))
            {
                doc_detail = Some(format!(
                    "{name}: {} solids, undecoded {:?}, solve failures {:?}, matrix {:?}",
                    doc.solids.len(),
                    doc.undecoded,
                    doc.solve_failures,
                    pose.matrix
                ));
            }
            continue;
        }
        let Some(SolidShape::TriangulatedFaceSet { triangles, .. }) = solid_shape else {
            boxed.push(format!("{name}: not a triangulated body"));
            continue;
        };
        // A rectangle-to-circle blend: at least its two caps and its side,
        // and no more than a fine tessellation of the fitting.
        assert!(
            (20..=4000).contains(&triangles.len()),
            "{name}: {} triangles",
            triangles.len()
        );
    }
    eprintln!("{file}: transitions left as boxes, by reason: {why:?}");
    for example in report.size_rejected_examples.iter().filter(|x| {
        let id = x.split(':').next().unwrap_or_default();
        ids.contains(id)
    }) {
        eprintln!("{file}: size rejected: {example}");
    }
    eprintln!("{file}: first such document: {doc_detail:?}");
    eprintln!(
        "{file}: all documents: solved {:?}, solve failures {:?}, other forms {:?}, voids outside solids {}",
        docs.stats.solved,
        docs.stats.solve_failures,
        docs.stats.other_forms,
        docs.stats.voids_outside_solids
    );
    assert!(found >= 100, "{file}: {found} transitions, {report:?}");
    // The transitions are drawn by stretching their document's body along
    // its one ruled axis; a body of another family whose cross-axis
    // stretches differ (element 573615: 8.693, 1.364, 1.203) stays a box.
    eprintln!("{file}: transitions with uneven stretches, left as boxes: {uneven:?}");
    let uneven_ids: Vec<&str> = uneven.iter().map(|(id, _)| id.as_str()).collect();
    match expected_uneven {
        Some(expected) => assert_eq!(
            uneven_ids, expected,
            "{file}: transitions left as boxes for uneven stretches"
        ),
        None => assert!(uneven.len() <= 2, "{file}: {uneven:?}"),
    }
    assert!(
        report.scaled_ruled_along_axis >= found - uneven.len(),
        "{file}: {} stretched along their ruled axis, {found} transitions",
        report.scaled_ruled_along_axis
    );
    for e in &model.entities {
        if let IfcEntity::BuildingElement {
            type_guid: Some(tag),
            property_set: Some(set),
            name,
            ..
        } = e
            && tag == UNEQUAL_STRETCH
        {
            let drawn = set.properties.iter().any(|p| {
                p.name == "BodySource"
                    && matches!(&p.value, PropertyValue::Text(t) if t.starts_with("family_document_geometry"))
            });
            assert!(!drawn, "{file}: {name} stretched unevenly across its axis");
        }
    }
    assert!(
        boxed.is_empty(),
        "{file}: {} of {found} transitions not drawn from their family: {:?}; {report:?}",
        boxed.len(),
        &boxed[..boxed.len().min(5)]
    );
}

#[test]
fn rectangle_to_round_transitions_are_not_boxes() {
    for (env, file, expected_uneven) in FILES {
        transitions_are_drawn_from_their_family(env, file, expected_uneven);
    }
}
