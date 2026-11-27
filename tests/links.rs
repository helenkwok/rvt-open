//! Linked and imported models and files (`rvt::links`), on the public samples CI fetches
//! (tools/ci/fetch-public-samples.sh), checked against what the authoring tool's API reports for
//! the same files and against the `TransmissionData` stream, a second record of the same links.
//!
//! - Snowdon Towers Architectural 2024 (`RVT_REVIT2024_SAMPLES_DIR`): six Revit links, all
//!   Overlay, relative paths, loaded when saved, placed at the internal origin; every link type
//!   is the element its `TransmissionData` reference names; six Manage Links rows.
//! - The Autodesk samples by release (`RVT_AUTODESK_SAMPLES_DIR`): every release's
//!   rst_basic_sample_project links rac_basic_sample_project (Overlay, not loaded) and imports
//!   three drawings into drafting views, each with its layers; Technical school imports
//!   `Roofing Detail.dwg` (90,720 bytes, inches) into one view (CI fetches the 2026 one).
//! - BIM4LCA Housing Concrete As-Built (`RVT_NORDIC_LCA_DIR`): IFC links know the IFC file they
//!   were made from, a link turned about z keeps its axes (x = (0.4904, 0.8715, 0), as the API
//!   reports), and the CAD link is linked, not imported.

use rvt::RevitFile;
use rvt::links::{LinkKind, Links, parse_transmission_data};
use std::path::PathBuf;

fn read(path: &std::path::Path) -> Links {
    let mut rf = RevitFile::open(path).expect("open");
    rvt::links::read(&mut rf).expect("links")
}

fn dir(var: &str) -> Option<PathBuf> {
    let d = std::env::var_os(var).map(PathBuf::from);
    if d.is_none() {
        eprintln!("skipping: {var} is not set");
    }
    d
}

#[test]
fn snowdon_architectural_links_six_overlay_models() {
    let Some(d) = dir("RVT_REVIT2024_SAMPLES_DIR") else {
        return;
    };
    let path = d.join("Snowdon Towers Sample Architectural.rvt");
    if !path.exists() {
        eprintln!("skipping: {} absent", path.display());
        return;
    }
    let links = read(&path);
    let revit: Vec<_> = links
        .types
        .iter()
        .filter(|t| t.kind == LinkKind::Revit)
        .collect();
    let mut names: Vec<&str> = revit.iter().map(|t| t.name.as_str()).collect();
    names.sort();
    assert_eq!(
        names,
        [
            "Snowdon Towers Sample Electrical.rvt",
            "Snowdon Towers Sample Facades.rvt",
            "Snowdon Towers Sample HVAC.rvt",
            "Snowdon Towers Sample Plumbing.rvt",
            "Snowdon Towers Sample Site.rvt",
            "Snowdon Towers Sample Structural.rvt",
        ]
    );
    for t in &revit {
        assert_eq!(t.attachment, Some(false), "{}: Overlay", t.name);
        assert_eq!(t.path_type, "Relative", "{}", t.name);
        assert_eq!(t.status, "Loaded", "{}", t.name);
        let r = links
            .references
            .iter()
            .find(|r| r.element_id == t.id)
            .unwrap_or_else(|| panic!("{} has no TransmissionData reference", t.name));
        assert_eq!(r.kind, "Revit Link");
        assert_eq!(r.saved_path, t.name);
        let placed: Vec<_> = links
            .instances
            .iter()
            .filter(|i| i.type_id == t.id)
            .collect();
        assert_eq!(placed.len(), 1, "{}", t.name);
        assert_eq!(
            placed[0].transform,
            [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.], [0., 0., 0.]]
        );
    }
    let rows = links.manage_links();
    assert_eq!(rows.iter().filter(|r| r.kind == LinkKind::Revit).count(), 6);
    assert!(
        rows.iter()
            .all(|r| r.kind != LinkKind::Revit || r.reference_type == "Overlay")
    );
}

#[test]
fn every_release_of_the_structural_sample_links_and_imports() {
    let Some(d) = dir("RVT_AUTODESK_SAMPLES_DIR") else {
        return;
    };
    let mut checked = 0;
    for year in 2016..=2027 {
        let y = d.join(year.to_string());
        let Some(path) = ["rst_basic_sample_project.rvt", "rstbasicsampleproject.rvt"]
            .iter()
            .map(|n| y.join(n))
            .find(|p| p.exists())
        else {
            continue;
        };
        let links = read(&path);
        let revit: Vec<_> = links
            .types
            .iter()
            .filter(|t| t.kind == LinkKind::Revit)
            .collect();
        assert_eq!(revit.len(), 1, "{year}");
        assert_eq!(revit[0].name, "rac_basic_sample_project.rvt", "{year}");
        assert_eq!(revit[0].attachment, Some(false), "{year}");
        assert_eq!(revit[0].status, "Not Loaded", "{year}");
        let cad: Vec<_> = links
            .types
            .iter()
            .filter(|t| t.kind == LinkKind::Cad)
            .collect();
        assert_eq!(cad.len(), 3, "{year}: {cad:?}");
        for t in &cad {
            assert!(
                !t.linked && t.view_specific == Some(true),
                "{year} {}",
                t.name
            );
            assert!(!t.layers.is_empty(), "{year} {}: no layers", t.name);
            let placed: Vec<_> = links
                .instances
                .iter()
                .filter(|i| i.type_id == t.id)
                .collect();
            assert_eq!(placed.len(), 1, "{year} {}", t.name);
            assert!(placed[0].view.is_some(), "{year} {}: no view", t.name);
        }
        assert!(
            links.references.iter().any(|r| r.kind == "Keynote Table"),
            "{year}"
        );
        checked += 1;
    }
    for year in 2014..=2027 {
        let y = d.join(year.to_string());
        let Some(path) = [
            "Technical_school-current_m.rvt",
            "Technicalschoolcurrentm.rvt",
        ]
        .iter()
        .map(|n| y.join(n))
        .find(|p| p.exists()) else {
            continue;
        };
        let links = read(&path);
        let cad: Vec<_> = links
            .types
            .iter()
            .filter(|t| t.kind == LinkKind::Cad)
            .collect();
        assert_eq!(cad.len(), 1, "{year}");
        assert_eq!(cad[0].name, "Roofing Detail.dwg", "{year}");
        assert_eq!(cad[0].file_size, Some(90_720), "{year}");
        assert!(cad[0].units.contains("inches"), "{year}: {}", cad[0].units);
        let row = links
            .manage_links()
            .into_iter()
            .find(|r| r.id == cad[0].id)
            .expect("row");
        assert_eq!((row.status.as_str(), row.count), ("Imported", 1), "{year}");
        checked += 1;
    }
    eprintln!("{checked} samples checked");
    assert!(
        checked > 0,
        "RVT_AUTODESK_SAMPLES_DIR holds none of the samples this test reads"
    );
}

#[test]
fn ifc_links_rotated_links_and_cad_links() {
    let Some(d) = dir("RVT_NORDIC_LCA_DIR") else {
        return;
    };
    let path = d.join("ARK_NordicLCA_Housing_Concrete_As-Built.rvt");
    if !path.exists() {
        eprintln!("skipping: {} absent", path.display());
        return;
    }
    let links = read(&path);
    let ifc: Vec<_> = links
        .types
        .iter()
        .filter(|t| t.kind == LinkKind::Ifc)
        .collect();
    assert!(!ifc.is_empty());
    for t in &ifc {
        assert!(t.name.to_lowercase().ends_with(".ifc"), "{}", t.name);
        let r = links
            .references
            .iter()
            .find(|r| r.element_id == t.id)
            .expect("reference");
        assert!(
            r.saved_path.ends_with(".ifc.RVT"),
            "{}: the cache Revit links",
            r.saved_path
        );
    }
    let turned = links
        .instances
        .iter()
        .find(|i| i.id == 2_167_925)
        .expect("link instance 2167925");
    let x = turned.transform[0];
    assert!(
        (x[0] - 0.490_420_924_476_449_6).abs() < 1e-12
            && (x[1] - 0.871_485_695_141_156_5).abs() < 1e-12,
        "{x:?}"
    );
    let cad = links
        .types
        .iter()
        .find(|t| t.name == "Assaritesti kotikutonen.dwg")
        .expect("CAD link");
    assert!(cad.linked && cad.kind == LinkKind::Cad);
}

#[test]
fn transmission_data_truncated_or_empty_is_no_references() {
    assert!(parse_transmission_data(&[]).is_empty());
    assert!(parse_transmission_data(&[200, 0, 0, 0, b'<', 0]).is_empty());
    let xml = "<TransmissionData><ExternalFileReference><ElementId>7</ElementId>\
        <ExternalFileReferenceType>CAD Link</ExternalFileReferenceType>\
        <LastSavedPath>a&amp;b.dwg</LastSavedPath></ExternalFileReference></TransmissionData>";
    let units: Vec<u16> = xml.encode_utf16().collect();
    let mut bytes = (units.len() as u32).to_le_bytes().to_vec();
    bytes.extend(units.iter().flat_map(|u| u.to_le_bytes()));
    let refs = parse_transmission_data(&bytes);
    assert_eq!(refs.len(), 1);
    assert_eq!(
        (refs[0].element_id, refs[0].saved_path.as_str()),
        (7, "a&b.dwg")
    );
    // Cut inside the reference: nothing half-read.
    assert!(parse_transmission_data(&bytes[..bytes.len() / 2]).is_empty());
}

/// Technical school's drafting view `Roofing Termination Detail` (Revit 2026, `RVT_REVIT_2026_DIR`)
/// draws its imported `Roofing Detail.dwg` as the authoring tool's own DWG export of sheet A2 does:
/// every line and text carries the import instance's ElementId and lies on the drawing's own
/// layers (the export draws its texts on `Text` and its linework on `Thin`, `Medium`, `Wide` and
/// `Hidden`), the blocks the drawing places (its fasteners) drawn in.
#[cfg(feature = "dwg")]
#[test]
fn an_imported_drawing_is_drawn_on_its_layers_as_its_import() {
    use rvt::ifc::sheet_views::{DrawingKey, view_drawing};
    use rvt::ifc::{Exporter, RvtDocExporter};
    let Some(d) = dir("RVT_REVIT_2026_DIR") else {
        return;
    };
    let path = d.join("Technicalschoolcurrentm.rvt");
    if !path.exists() {
        eprintln!("skipping: {} absent", path.display());
        return;
    }
    let mut rf = RevitFile::open(&path).expect("open");
    let links = rvt::links::read(&mut rf).expect("links");
    let import = links
        .instances
        .iter()
        .find(|i| i.class == "ImportInstance")
        .expect("an import");
    let facts = rvt::plan_views::read(&mut rf).expect("views");
    let model = RvtDocExporter.export(&mut rf).expect("export");
    let sheet = facts
        .sheets
        .iter()
        .find(|s| s.number == "A2")
        .expect("sheet A2");
    let mut notes = Vec::new();
    let v = view_drawing(
        &mut rf,
        &model,
        &facts,
        DrawingKey::Sheet(sheet.id),
        &Default::default(),
        &rvt::ifc::dwg_layers::ExportLayers::default(),
        &mut notes,
    )
    .expect("draw");
    let mine: Vec<_> = v
        .drawing
        .entities
        .iter()
        .filter(|e| e.element == Some(import.id))
        .collect();
    let mut by_layer: std::collections::BTreeMap<(&str, &str), usize> = Default::default();
    for e in &mine {
        *by_layer
            .entry((e.layer.as_str(), e.kind.as_str()))
            .or_default() += 1;
    }
    eprintln!(
        "imported drawing on sheet A2: {} entities {by_layer:?}",
        mine.len()
    );
    let layers: std::collections::BTreeSet<&str> = mine.iter().map(|e| e.layer.as_str()).collect();
    for l in ["Thin", "Text", "Medium", "Wide"] {
        assert!(layers.contains(l), "no {l} layer: {layers:?}");
    }
    let texts = mine.iter().filter(|e| e.kind == "TEXT").count();
    assert!(texts > 0, "no texts of the imported drawing");
}
