//! A floor drawn as its sketch outline whose saved mesh covers another plan
//! area is drawn from the mesh.
//!
//! On the public Snowdon Towers samples (Revit 2024) four floors' outlines are
//! not the floor Revit drew: lawns 1227304 and 1235943 and slab 2238597 of the
//! architectural model are cut by what they join, and slab 727212 of the
//! structural model reaches past its sketch. As outlines they were 838.9,
//! 232.0, 26.8 and 716.0 ft² in plan; Revit's Area, read through its API, is
//! 695.37, 199.97, 24.93 and 804.08 ft², and each saved mesh's top is that
//! within 1%.
//!
//! Runs against `RVT_REVIT2024_SAMPLES_DIR` (`Snowdon Towers Sample
//! <Discipline>.rvt`); a file that is absent is skipped.

use rvt::RevitFile;
use rvt::convert::{ConvertOptions, export};
use rvt::ifc::entities::{IfcEntity, PropertyValue, SolidShape};
use std::path::PathBuf;

/// Each floor's file, ElementId and Revit's Area, ft².
const FLOORS: [(&str, &str, f64); 4] = [
    ("Architectural", "1227304", 695.368),
    ("Architectural", "1235943", 199.973),
    ("Architectural", "2238597", 24.929),
    ("Structural", "727212", 804.080),
];

#[test]
fn snowdon_floors_take_their_saved_mesh_where_the_outline_differs() {
    let Some(dir) = std::env::var_os("RVT_REVIT2024_SAMPLES_DIR").map(PathBuf::from) else {
        eprintln!("skipping: RVT_REVIT2024_SAMPLES_DIR is not set");
        return;
    };
    for discipline in ["Architectural", "Structural"] {
        let path = dir.join(format!("Snowdon Towers Sample {discipline}.rvt"));
        if !path.exists() {
            eprintln!("skipping: {} absent", path.display());
            continue;
        }
        let mut rf = RevitFile::open(&path).expect("open");
        // Every body offered its saved mesh, as `rvt-ifc` and the viewer do.
        let options = ConvertOptions {
            saved_meshes: true,
            ..ConvertOptions::default()
        };
        let model = export(&mut rf, &options).expect("export").model;
        for (_, tag, revit) in FLOORS.iter().filter(|(d, _, _)| *d == discipline) {
            let (shape, source) = model
                .entities
                .iter()
                .find_map(|entity| match entity {
                    IfcEntity::BuildingElement {
                        type_guid: Some(t),
                        solid_shape,
                        property_set,
                        ..
                    } if t == tag => {
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
                .unwrap_or_else(|| panic!("{discipline}: floor {tag} not exported"));
            assert_eq!(
                source.as_deref(),
                Some("saved_graphics_mesh"),
                "{discipline}: {tag}"
            );
            let Some(SolidShape::TriangulatedFaceSet {
                vertices_feet,
                triangles,
                ..
            }) = shape
            else {
                panic!("{discipline}: floor {tag} is not a triangulated face set");
            };
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
                    if length > 0.0 && n[2] / length > 0.1 {
                        length / 2.0
                    } else {
                        0.0
                    }
                })
                .sum();
            assert!(
                (up - revit).abs() <= 0.02 * revit,
                "{discipline}: floor {tag}'s top is {up:.2} ft², Revit's Area {revit}"
            );
        }
    }
}
