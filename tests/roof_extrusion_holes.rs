//! A roof by extrusion is drawn less the holes its hosted skylights cut.
//!
//! The public sample `rac_basic_sample_project.rvt` has one roof by extrusion,
//! 243274, with two `M_Skylight` windows hosted in it (488881 and 489062).
//! Each skylight's symbol cuts a 3.3763 by 3.5469 ft rectangle square through
//! the roof. Revit's IFC4 export of the 2024 sample draws the roof as one
//! body of 1235.098 ft³ with both holes, and the mesh the file saves for it
//! (read on 2023) is the same body to 5e-6 ft. Without the holes the section
//! run along the roof is 1250.972 ft³ (1.3% too big) on 2024 to 2027.
//!
//! Runs against `RVT_AUTODESK_SAMPLES_DIR` (`<year>/rac_basic_sample_project.rvt`);
//! a release whose file is absent is skipped.

use rvt::RevitFile;
use rvt::ifc::RvtDocExporter;
use rvt::ifc::entities::{IfcEntity, PropertyValue, SolidShape};
use std::path::PathBuf;

/// The roof's volume in Revit's 2024 export, ft³.
const REVIT_VOLUME_FT3: f64 = 1235.098;

#[test]
fn rac_basic_extrusion_roof_has_its_skylight_holes() {
    let Some(dir) = std::env::var_os("RVT_AUTODESK_SAMPLES_DIR").map(PathBuf::from) else {
        eprintln!("skipping: RVT_AUTODESK_SAMPLES_DIR is not set");
        return;
    };
    for year in 2024..=2027 {
        let path = dir.join(format!("{year}/rac_basic_sample_project.rvt"));
        if !path.exists() {
            eprintln!("skipping: {} absent", path.display());
            continue;
        }
        let mut rf = RevitFile::open(&path).expect("open");
        let model = RvtDocExporter
            .export_with_diagnostics(&mut rf)
            .expect("export")
            .model;
        let (shape, source) = model
            .entities
            .iter()
            .find_map(|entity| match entity {
                IfcEntity::BuildingElement {
                    type_guid: Some(tag),
                    solid_shape: Some(shape),
                    property_set,
                    ..
                } if tag == "243274" => {
                    let source = property_set.as_ref().and_then(|set| {
                        set.properties.iter().find_map(|p| match &p.value {
                            PropertyValue::Text(t) if p.name == "BodySource" => Some(t.clone()),
                            _ => None,
                        })
                    });
                    Some((shape, source))
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("{year}: roof 243274 has no solid body"));
        assert_eq!(
            source.as_deref(),
            Some("partition_roof_extrusion"),
            "{year}"
        );
        let SolidShape::FacetedBrep {
            vertices_feet,
            triangles,
        } = shape
        else {
            panic!("{year}: roof 243274 is not a faceted brep");
        };
        let volume: f64 = triangles
            .iter()
            .map(|t| {
                let [a, b, c] = [t.0, t.1, t.2].map(|i| vertices_feet[i as usize]);
                (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                    + a[2] * (b[0] * c[1] - b[1] * c[0]))
                    / 6.0
            })
            .sum();
        assert!(
            (volume - REVIT_VOLUME_FT3).abs() < 0.01,
            "{year}: roof 243274 is {volume:.4} ft³, Revit's export {REVIT_VOLUME_FT3}"
        );
    }
}
