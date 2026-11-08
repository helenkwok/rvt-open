//! The butterfly roof 142430 of `rac_advanced_sample_project`, whose two
//! defining edges slope down into it at 15 degrees, on every release the
//! public samples hold (`RVT_AUTODESK_SAMPLES_DIR`, `<release>/<name>.rvt`).
//!
//! Revit's IFC4 export of the 2024 save writes it as an `IfcRoof` with no body
//! that aggregates two `IfcSlab` `.ROOF.`, `0DSJ1rqxHAlwyjXg4lw1M2` and
//! `0DSJ1rqxHAlwyjXg0lw1M2`, each 11.1195 m³, falling from the eaves to a
//! valley. Each file absent is skipped.

use rvt::RevitFile;
use rvt::ifc::RvtDocExporter;
use rvt::ifc::entities::{IfcEntity, SolidShape};
use std::path::PathBuf;

const RELEASES: &[u32] = &[2018, 2019, 2020, 2021, 2022, 2023, 2024, 2025, 2026, 2027];
const SLABS: [&str; 2] = ["0DSJ1rqxHAlwyjXg4lw1M2", "0DSJ1rqxHAlwyjXg0lw1M2"];
const SLAB_VOLUME_M3: f64 = 11.1195;
const FT3_TO_M3: f64 = 0.3048 * 0.3048 * 0.3048;

/// The volume a closed triangulated shell encloses, cubic feet.
fn shell_volume(vertices: &[[f64; 3]], triangles: &[(u32, u32, u32)]) -> f64 {
    let at = |i: u32| vertices[i as usize];
    triangles
        .iter()
        .map(|&(a, b, c)| {
            let (a, b, c) = (at(a), at(b), at(c));
            a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                + a[2] * (b[0] * c[1] - b[1] * c[0])
        })
        .sum::<f64>()
        .abs()
        / 6.0
}

#[test]
fn butterfly_roof_is_revits_two_slabs() {
    let Some(dir) = std::env::var_os("RVT_AUTODESK_SAMPLES_DIR").map(PathBuf::from) else {
        eprintln!("skipping: RVT_AUTODESK_SAMPLES_DIR is not set");
        return;
    };
    for release in RELEASES {
        let path = dir.join(format!("{release}/rac_advanced_sample_project.rvt"));
        if !path.exists() {
            eprintln!("skipping: {release} absent");
            continue;
        }
        let mut rf = RevitFile::open(&path).expect("open");
        let model = RvtDocExporter
            .export_with_diagnostics(&mut rf)
            .expect("export")
            .model;
        let ids = rvt::ifc::step_writer::element_global_ids(&model);
        let mut slabs: Vec<(String, f64)> = Vec::new();
        for (index, entity) in model.entities.iter().enumerate() {
            let IfcEntity::BuildingElement {
                ifc_type,
                type_guid: Some(tag),
                solid_shape,
                ..
            } = entity
            else {
                continue;
            };
            if tag != "142430" || ifc_type != "IFCSLAB" {
                continue;
            }
            let Some(SolidShape::FacetedBrep {
                vertices_feet,
                triangles,
            }) = solid_shape
            else {
                panic!("{release}: a slab of roof 142430 has no faceted body");
            };
            let triangles: Vec<(u32, u32, u32)> =
                triangles.iter().map(|t| (t.0, t.1, t.2)).collect();
            let volume = shell_volume(vertices_feet, &triangles) * FT3_TO_M3;
            slabs.push((ids[index].clone().expect("GlobalId"), volume));
        }
        let names: Vec<&str> = slabs.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(names, SLABS, "{release}: slabs of roof 142430");
        for (id, volume) in &slabs {
            assert!(
                (volume - SLAB_VOLUME_M3).abs() <= 0.001,
                "{release}: slab {id} is {volume:.4} m³, Revit's {SLAB_VOLUME_M3}"
            );
        }
    }
}
