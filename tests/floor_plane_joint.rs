//! A floor whose saved faces meet a hair apart is drawn from its saved mesh,
//! not as its record's box.
//!
//! The public sample `rmeadvancedsampleproject.rvt` (Revit 2024 to 2027) has
//! floor 576122, `FB 15.0 - Fliesen 50 x 50` on Level 3: a 0.15 m ring round
//! the courtyard with three shafts and twelve pipe holes through it. Its record
//! box is 188.484 by 146.302 ft. Two edges of the outer loop of its top face,
//! and of its bottom face, meet 0.00019 ft apart in the plane, so the face
//! decoder refused both faces and the floor was drawn as the whole box,
//! covering the courtyard: 27,575.6 ft² in plan. Its saved top face is the
//! outer loop of 5,377.42 ft² less the shafts and holes, 4,644.65 ft².
//!
//! Runs against `RVT_AUTODESK_SAMPLES_DIR` (`<year>/rmeadvancedsampleproject.rvt`);
//! a release whose file is absent is skipped.

use rvt::RevitFile;
use rvt::convert::{ConvertOptions, export};
use rvt::ifc::entities::{IfcEntity, PropertyValue, SolidShape};
use std::path::PathBuf;

/// The plan area of the floor's saved top face, ft².
const TOP_FACE_FT2: f64 = 4644.647;

#[test]
fn rme_advanced_floor_576122_is_its_saved_ring_not_its_box() {
    let Some(dir) = std::env::var_os("RVT_AUTODESK_SAMPLES_DIR").map(PathBuf::from) else {
        eprintln!("skipping: RVT_AUTODESK_SAMPLES_DIR is not set");
        return;
    };
    for year in 2024..=2027 {
        let path = dir.join(format!("{year}/rmeadvancedsampleproject.rvt"));
        if !path.exists() {
            eprintln!("skipping: {} absent", path.display());
            continue;
        }
        let mut rf = RevitFile::open(&path).expect("open");
        // Every box body offered its saved mesh, as `rvt-ifc` and the viewer
        // do by default.
        let options = ConvertOptions {
            saved_meshes: true,
            ..ConvertOptions::default()
        };
        let model = export(&mut rf, &options).expect("export").model;
        let (shape, source) = model
            .entities
            .iter()
            .find_map(|entity| match entity {
                IfcEntity::BuildingElement {
                    type_guid: Some(tag),
                    solid_shape,
                    property_set,
                    ..
                } if tag == "576122" => {
                    let source = property_set.as_ref().and_then(|set| {
                        set.properties.iter().find_map(|p| match &p.value {
                            PropertyValue::Text(t) if p.name == "BodySource" => Some(t.clone()),
                            _ => None,
                        })
                    });
                    Some((solid_shape.clone(), source))
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("{year}: floor 576122 not exported"));
        assert_eq!(source.as_deref(), Some("saved_graphics_mesh"), "{year}");
        let Some(SolidShape::TriangulatedFaceSet {
            vertices_feet,
            triangles,
            ..
        }) = shape
        else {
            panic!("{year}: floor 576122 is not a triangulated face set");
        };
        // The plan area of the faces that look up.
        let up: f64 = triangles
            .iter()
            .map(|t| {
                let [a, b, c] = [t.0, t.1, t.2].map(|i| vertices_feet[i as usize]);
                let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
                let n = [
                    u[1] * v[2] - u[2] * v[1],
                    u[2] * v[0] - u[0] * v[2],
                    u[0] * v[1] - u[1] * v[0],
                ];
                let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                if length > 0.0 && n[2] / length > 0.9 {
                    length / 2.0
                } else {
                    0.0
                }
            })
            .sum();
        assert!(
            (up - TOP_FACE_FT2).abs() < 0.5,
            "{year}: floor 576122's top is {up:.3} ft² in plan, its saved face {TOP_FACE_FT2}"
        );
    }
}
