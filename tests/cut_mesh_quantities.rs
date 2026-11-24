//! A mesh closed only up to T-junctions (a face split on one side of an
//! edge and not on the other, as a void cut leaves it) is measured as the
//! closed solid it is, and a void cut's result has no T-junctions left, so
//! it is drawn one-sided. A watertight body keeps its quantities, and an
//! open part of a face set counts for its area only.

use rvt::ifc::entities::{BrepTriangle, IfcEntity, SolidShape};
use rvt::ifc::quantities::{ElementQuantities, measure};
use rvt::ifc::{IfcModel, MaterialInfo, Storey, body_geometry};
use rvt::mesh_csg::difference;
use std::collections::HashMap;

type V3 = [f64; 3];

/// An axis-aligned box from `lo` to `hi`, wound outward.
fn cuboid(lo: V3, hi: V3) -> (Vec<V3>, Vec<[u32; 3]>) {
    let v = (0..8)
        .map(|i| {
            [
                if i & 1 == 0 { lo[0] } else { hi[0] },
                if i & 2 == 0 { lo[1] } else { hi[1] },
                if i & 4 == 0 { lo[2] } else { hi[2] },
            ]
        })
        .collect();
    let t = vec![
        [0, 2, 3],
        [0, 3, 1],
        [4, 5, 7],
        [4, 7, 6],
        [0, 1, 5],
        [0, 5, 4],
        [2, 6, 7],
        [2, 7, 3],
        [0, 4, 6],
        [0, 6, 2],
        [1, 3, 7],
        [1, 7, 5],
    ];
    (v, t)
}

fn signed_volume(v: &[V3], t: &[[u32; 3]]) -> f64 {
    t.iter()
        .map(|t| {
            let [a, b, c] = t.map(|i| v[i as usize]);
            a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                + a[2] * (b[0] * c[1] - b[1] * c[0])
        })
        .sum::<f64>()
        / 6.0
}

/// Directed edges (between welded points) not met by as many reversed.
fn unmatched_edges(v: &[V3], t: &[[u32; 3]]) -> usize {
    let key = |i: u32| v[i as usize].map(|c| (c * 1e6).round() as i64);
    let mut count: HashMap<([i64; 3], [i64; 3]), i64> = HashMap::new();
    for t in t {
        for k in 0..3 {
            *count.entry((key(t[k]), key(t[(k + 1) % 3]))).or_default() += 1;
        }
    }
    count
        .iter()
        .filter(|((a, b), n)| count.get(&(*b, *a)).copied().unwrap_or(0) != **n)
        .count()
}

fn face_set(v: Vec<V3>, t: &[[u32; 3]]) -> SolidShape {
    SolidShape::TriangulatedFaceSet {
        vertices_feet: v,
        triangles: t.iter().map(|t| BrepTriangle(t[0], t[1], t[2])).collect(),
        face_materials: Vec::new(),
        uvs: Vec::new(),
    }
}

/// The net volume (cubic feet) and surface (square feet) the takeoff gives
/// a proxy drawn as `shape`.
fn quantities(shape: SolidShape) -> (Option<f64>, f64) {
    let q = measured(shape, &[]);
    (q.net_volume_cuft, q.surface_sqft)
}

/// What the takeoff gives a proxy drawn as `shape`, in a model with these
/// materials.
fn measured(shape: SolidShape, materials: &[&str]) -> ElementQuantities {
    let mut model = IfcModel::default();
    for name in materials {
        model.materials.push(MaterialInfo {
            name: (*name).into(),
            color_packed: Some(0),
            transparency: None,
            texture: None,
        });
    }
    model.building_storeys.push(Storey {
        name: "L1".into(),
        elevation_feet: 0.0,
    });
    model.entities.push(IfcEntity::BuildingElement {
        ifc_type: "IFCBUILDINGELEMENTPROXY".into(),
        name: "piece".into(),
        type_guid: None,
        predefined_type: None,
        storey_index: Some(0),
        material_index: None,
        property_set: None,
        location_feet: Some([0.0, 0.0, 0.0]),
        rotation_radians: None,
        extrusion: None,
        host_element_index: None,
        material_layer_set_index: None,
        material_profile_set_index: None,
        solid_shape: Some(shape),
        representation_map_index: None,
    });
    let mut q = measure(&model);
    assert_eq!(q.len(), 1);
    q.remove(0)
}

/// A 2 ft cube far from the origin whose top face is split at the middle of
/// its front edge while the front face is not: one T-junction.
fn cube_with_t_junction() -> (Vec<V3>, Vec<[u32; 3]>) {
    let (mut v, mut t) = cuboid([100.0, 100.0, 100.0], [102.0, 102.0, 102.0]);
    // The top face (4, 5, 7, 6) is z = 102; its front edge runs 4 -> 5 at
    // y = 100. Split the top at the edge's middle point, 8.
    v.push([101.0, 100.0, 102.0]);
    t.retain(|t| *t != [4, 5, 7] && *t != [4, 7, 6]);
    t.extend([[4, 8, 6], [8, 7, 6], [8, 5, 7]]);
    (v, t)
}

#[test]
fn a_mesh_closed_up_to_t_junctions_gives_its_true_volume_and_surface() {
    let (v, t) = cube_with_t_junction();
    assert_eq!(unmatched_edges(&v, &t), 3, "the T-junction is there");
    let (volume, surface) = quantities(face_set(v, &t));
    let volume = volume.expect("measured");
    assert!((volume - 8.0).abs() < 1e-9, "volume {volume}");
    assert!((surface - 24.0).abs() < 1e-9, "surface {surface}");
}

#[test]
fn a_watertight_body_keeps_its_quantities() {
    let (v, t) = cuboid([100.0, 100.0, 100.0], [102.0, 103.0, 104.0]);
    let (volume, surface) = quantities(face_set(v, &t));
    assert!((volume.expect("measured") - 24.0).abs() < 1e-9);
    assert!((surface - 52.0).abs() < 1e-9);
}

#[test]
fn an_open_surface_far_from_the_origin_has_no_volume() {
    // One square, drawn two-sided as an open mesh is: no tetrahedra to the
    // origin count as volume.
    let v = vec![
        [100.0, 100.0, 100.0],
        [101.0, 100.0, 100.0],
        [101.0, 101.0, 100.0],
        [100.0, 101.0, 100.0],
    ];
    let (volume, _) = quantities(face_set(v, &[[0, 1, 2], [0, 2, 3]]));
    assert_eq!(volume, None);
}

#[test]
fn a_closed_body_beside_an_open_face_keeps_its_volume() {
    // A 1 ft cube and, apart from it, one square face: the cube is a solid,
    // the face counts for its area only.
    let (mut v, mut t) = cuboid([100.0, 100.0, 100.0], [101.0, 101.0, 101.0]);
    v.extend([
        [110.0, 100.0, 100.0],
        [111.0, 100.0, 100.0],
        [111.0, 101.0, 100.0],
        [110.0, 101.0, 100.0],
    ]);
    t.extend([[8, 9, 10], [8, 10, 11]]);
    let (volume, surface) = quantities(face_set(v, &t));
    assert!((volume.expect("measured") - 1.0).abs() < 1e-9);
    assert!((surface - 7.0).abs() < 1e-9, "surface {surface}");
}

#[test]
fn closed_bodies_touching_an_open_face_keep_their_volume_in_any_order() {
    // Two 1 ft cubes and, between them, an open square sharing an edge with
    // each: the cubes are solids whatever order the triangles come in.
    let (mut v, mut t) = cuboid([100.0, 100.0, 100.0], [101.0, 101.0, 101.0]);
    let (v2, t2) = cuboid([102.0, 100.0, 100.0], [103.0, 101.0, 101.0]);
    t.extend(t2.iter().map(|t| t.map(|i| i + 8)));
    v.extend(v2);
    // The square from x = 101 to 102 at z = 100, its edges the first cube's
    // (1, 3) and the second's (8, 10).
    let sheet = [[1, 8, 10], [1, 10, 3]];
    for sheet_first in [false, true] {
        let tris: Vec<[u32; 3]> = if sheet_first {
            sheet.iter().chain(&t).copied().collect()
        } else {
            t.iter().chain(&sheet).copied().collect()
        };
        let (volume, surface) = quantities(face_set(v.clone(), &tris));
        assert!(
            (volume.expect("measured") - 2.0).abs() < 1e-9,
            "sheet first {sheet_first}"
        );
        assert!((surface - 13.0).abs() < 1e-9, "surface {surface}");
    }
}

#[test]
fn an_open_face_counts_for_the_material_it_is_drawn_in() {
    let v = vec![
        [100.0, 100.0, 100.0],
        [101.0, 100.0, 100.0],
        [101.0, 101.0, 100.0],
        [100.0, 101.0, 100.0],
    ];
    let shape = SolidShape::TriangulatedFaceSet {
        vertices_feet: v,
        triangles: vec![BrepTriangle(0, 1, 2), BrepTriangle(0, 2, 3)],
        face_materials: vec![0, 0],
        uvs: Vec::new(),
    };
    let q = measured(shape, &["Glass"]);
    assert_eq!(q.net_volume_cuft, None);
    assert_eq!(q.materials.len(), 1);
    assert_eq!(q.materials[0].material, "Glass");
    assert!((q.materials[0].area_sqft - 1.0).abs() < 1e-9);
    assert_eq!(q.materials[0].volume_cuft, 0.0);
}

#[test]
fn a_void_cut_leaves_no_t_junctions_and_is_drawn_one_sided() {
    // A pocket cut into the top of a 2 ft cube: the cutter's planes split
    // the top face across its whole width, where the sides are not split.
    let a = cuboid([100.0, 100.0, 100.0], [102.0, 102.0, 102.0]);
    let b = cuboid([100.5, 100.5, 101.5], [101.5, 101.5, 103.0]);
    let (v, t) = difference((&a.0, &a.1), (&b.0, &b.1)).expect("cut");
    assert_eq!(unmatched_edges(&v, &t), 0);
    assert!((signed_volume(&v, &t) - 7.5).abs() < 1e-9);
    let drawn = body_geometry::solid_mesh(&face_set(v.clone(), &t)).expect("mesh");
    assert_eq!(drawn.triangles.len(), t.len(), "not two-sided");
    let (volume, surface) = quantities(face_set(v, &t));
    assert!((volume.expect("measured") - 7.5).abs() < 1e-9);
    // The cube's 24 square feet plus the pocket's four 1 x 0.5 walls.
    assert!((surface - 26.0).abs() < 1e-9, "surface {surface}");
}
