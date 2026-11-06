//! exact instancing of family-type meshes.
//!
//! Revit saves a family instance's graphics as its symbol's geometry placed
//! by the instance's transform (`GInstance` / `InstanceInfo.m_Trf`). The
//! saved-graphics reader meshes the symbol's faces in the symbol's own frame,
//! so every instance of one symbol yields the same triangles there, bit for
//! bit. [`register`] keeps that symbol-frame mesh once and records, per
//! element, the transform from it into the element's local frame; the shared
//! shapes pass then gives the instances one map placed by that transform
//! (IFC4 `IfcMappedItem.MappingTarget`, a glTF node matrix or instance, a
//! `.frag` sample, an IFCX/USD xformop) instead of matching element-local
//! meshes by a tolerance. Positions are never moved: the element-local mesh
//! the other passes read is the symbol mesh through the transform.
//!
//! A mirrored instance (negative-determinant transform) shares a mirrored
//! copy of the symbol mesh (x negated, winding turned, both exact in floating
//! point), so every transform written is a proper rotation plus translation.

use super::IfcModel;
use super::body_geometry::Placement;
use super::entities::{BrepTriangle, IfcEntity, SolidShape};
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};

/// A 3x4 affine transform, row-major, acting on column vectors (feet).
pub type Frame = [[f64; 4]; 3];

pub const IDENTITY: Frame = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
];

/// A mesh in its family symbol's frame.
#[derive(Debug, Clone, PartialEq)]
pub struct SymbolMesh {
    pub vertices_feet: Vec<[f64; 3]>,
    pub triangles: Vec<BrepTriangle>,
    pub face_materials: Vec<usize>,
}

impl SymbolMesh {
    fn key(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for v in &self.vertices_feet {
            v.map(f64::to_bits).hash(&mut h);
        }
        for t in &self.triangles {
            (t.0, t.1, t.2).hash(&mut h);
        }
        self.face_materials.hash(&mut h);
        h.finish()
    }

    /// The mesh as a triangulated face set.
    pub fn shape(&self) -> SolidShape {
        SolidShape::TriangulatedFaceSet {
            vertices_feet: self.vertices_feet.clone(),
            triangles: self.triangles.clone(),
            face_materials: self.face_materials.clone(),
        }
    }
}

/// The model's symbol meshes and which elements draw them.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Instancing {
    /// Distinct symbol-frame meshes, interned by exact content.
    #[serde(skip)]
    pub meshes: Vec<SymbolMesh>,
    #[serde(skip)]
    index: BTreeMap<u64, Vec<usize>>,
    /// Entity index to its parts: (symbol mesh, transform from the symbol
    /// frame into the element's local frame). Not serialised: the IFC4
    /// derivation sees only the element's own body and its map's target.
    #[serde(skip)]
    pub instances: BTreeMap<usize, Vec<(usize, Frame)>>,
    /// Entity index to its mapped item's target: the transform from its
    /// representation map's frame into the element's local frame. An element
    /// with no entry maps by the identity.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub targets: BTreeMap<usize, Frame>,
}

impl Instancing {
    pub fn is_empty(&self) -> bool {
        self.targets.is_empty() && self.instances.is_empty()
    }

    fn intern(&mut self, mesh: SymbolMesh) -> usize {
        let key = mesh.key();
        let slot = self.index.entry(key).or_default();
        if let Some(&i) = slot.iter().find(|&&i| self.meshes[i] == mesh) {
            return i;
        }
        self.meshes.push(mesh);
        slot.push(self.meshes.len() - 1);
        self.meshes.len() - 1
    }
}

pub fn apply(m: &Frame, v: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2] + m[i][3])
}

pub fn det(m: &Frame) -> f64 {
    m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
}

/// `a · b`.
pub fn compose(a: &Frame, b: &Frame) -> Frame {
    std::array::from_fn(|i| {
        std::array::from_fn(|j| {
            let mut s: f64 = (0..3).map(|k| a[i][k] * b[k][j]).sum();
            if j == 3 {
                s += a[i][3];
            }
            s
        })
    })
}

/// An element's placement (location, rotation about Z) as a frame.
pub fn placement_frame(p: &Placement) -> Frame {
    let (c, s, o) = (p.cos, p.sin, p.origin);
    [[c, -s, 0.0, o[0]], [s, c, 0.0, o[1]], [0.0, 0.0, 1.0, o[2]]]
}

/// The inverse of [`placement_frame`].
pub fn inverse_placement(p: &Placement) -> Frame {
    let (c, s, o) = (p.cos, p.sin, p.origin);
    [
        [c, s, 0.0, -(c * o[0] + s * o[1])],
        [-s, c, 0.0, s * o[0] - c * o[1]],
        [0.0, 0.0, 1.0, -o[2]],
    ]
}

/// Draw the element at `index` from `mesh` (in its symbol's frame) placed in
/// the document by `world`. Returns false, changing nothing, when `world` is
/// not a rigid transform (a scaled instance) or the element has no placement.
pub fn register(model: &mut IfcModel, index: usize, mesh: SymbolMesh, world: Frame) -> bool {
    register_parts(model, index, vec![(mesh, Some(world))])
}

/// [`register`] for a body of several parts, each a mesh in its own frame
/// placed by its own transform: a railing's balusters and posts are family
/// instances inside the railing's graphics. The element's own
/// body becomes the parts together, in its local frame. A part placed by
/// `None` is already in the element's frame (its own graphics, not a
/// symbol's).
pub fn register_parts(
    model: &mut IfcModel,
    index: usize,
    parts: Vec<(SymbolMesh, Option<Frame>)>,
) -> bool {
    let rigid = |w: &Frame| {
        let d = det(w);
        d.is_finite() && (d.abs() - 1.0).abs() <= 1e-6
    };
    if parts.is_empty()
        || parts
            .iter()
            .any(|(m, w)| w.is_some_and(|w| !rigid(&w)) || m.triangles.is_empty())
    {
        return false;
    }
    let IfcEntity::BuildingElement {
        location_feet: Some(origin),
        rotation_radians,
        ..
    } = &model.entities[index]
    else {
        return false;
    };
    let inverse = inverse_placement(&Placement::new(Some(*origin), *rotation_radians));
    let coloured = parts.iter().any(|(m, _)| !m.face_materials.is_empty());
    let (mut local, mut triangles, mut face_materials) = (Vec::new(), Vec::new(), Vec::new());
    let mut drawn = Vec::with_capacity(parts.len());
    for (mut mesh, world) in parts {
        let Some(mut world) = world else {
            let base = local.len() as u32;
            local.extend_from_slice(&mesh.vertices_feet);
            triangles.extend(
                mesh.triangles
                    .iter()
                    .map(|t| BrepTriangle(base + t.0, base + t.1, base + t.2)),
            );
            if coloured {
                if mesh.face_materials.is_empty() {
                    face_materials.resize(triangles.len(), usize::MAX);
                } else {
                    face_materials.extend_from_slice(&mesh.face_materials);
                }
            }
            drawn.push((model.instancing.intern(mesh), IDENTITY));
            continue;
        };
        if det(&world) < 0.0 {
            // Mirrored: share a mirrored copy, x negated (exact), winding turned.
            for v in &mut mesh.vertices_feet {
                v[0] = -v[0];
            }
            for t in &mut mesh.triangles {
                std::mem::swap(&mut t.1, &mut t.2);
            }
            for row in &mut world {
                row[0] = -row[0];
            }
        }
        let target = compose(&inverse, &world);
        let base = local.len() as u32;
        local.extend(mesh.vertices_feet.iter().map(|v| apply(&target, *v)));
        triangles.extend(
            mesh.triangles
                .iter()
                .map(|t| BrepTriangle(base + t.0, base + t.1, base + t.2)),
        );
        if coloured {
            if mesh.face_materials.is_empty() {
                face_materials.resize(triangles.len(), usize::MAX);
            } else {
                face_materials.extend_from_slice(&mesh.face_materials);
            }
        }
        drawn.push((model.instancing.intern(mesh), target));
    }
    if let IfcEntity::BuildingElement { solid_shape, .. } = &mut model.entities[index] {
        *solid_shape = Some(SolidShape::TriangulatedFaceSet {
            vertices_feet: local,
            triangles,
            face_materials,
        });
    }
    model.instancing.instances.insert(index, drawn);
    true
}

/// The parts (symbol mesh, transform into the element's frame) of the element
/// at `index`, when its body is still exactly the one [`register_parts`] gave
/// it (no later pass changed it).
pub fn parts_of(model: &IfcModel, index: usize) -> Option<&[(usize, Frame)]> {
    let parts = model.instancing.instances.get(&index)?;
    let IfcEntity::BuildingElement {
        solid_shape:
            Some(SolidShape::TriangulatedFaceSet {
                vertices_feet,
                triangles,
                face_materials,
            }),
        ..
    } = model.entities.get(index)?
    else {
        return None;
    };
    let coloured = !face_materials.is_empty();
    let (mut v, mut t) = (0usize, 0usize);
    for (symbol, target) in parts {
        let mesh = model.instancing.meshes.get(*symbol)?;
        let n = mesh.vertices_feet.len();
        let base = v as u32;
        let same_vertices = vertices_feet.get(v..v + n).is_some_and(|l| {
            l.iter()
                .zip(&mesh.vertices_feet)
                .all(|(l, s)| apply(target, *s) == *l)
        });
        let k = mesh.triangles.len();
        let same_triangles = triangles.get(t..t + k).is_some_and(|l| {
            l.iter()
                .zip(&mesh.triangles)
                .all(|(l, s)| l.0 == base + s.0 && l.1 == base + s.1 && l.2 == base + s.2)
        });
        let same_materials = !coloured
            || match mesh.face_materials.is_empty() {
                true => face_materials
                    .get(t..t + k)
                    .is_some_and(|f| f.iter().all(|&m| m == usize::MAX)),
                false => face_materials.get(t..t + k) == Some(&mesh.face_materials[..]),
            };
        if !(same_vertices && same_triangles && same_materials) {
            return None;
        }
        v += n;
        t += k;
    }
    (v == vertices_feet.len() && t == triangles.len()).then_some(parts.as_slice())
}

/// The symbol mesh and transform of an element drawn from one symbol mesh
/// ([`parts_of`] with a single part).
pub fn instance_of(model: &IfcModel, index: usize) -> Option<(usize, Frame)> {
    match parts_of(model, index)? {
        [one] => Some(*one),
        _ => None,
    }
}

/// The shared parts of a body and the rest of it ([`drawn_parts`]).
pub type DrawnParts = (Vec<(usize, Frame)>, Option<super::body_geometry::Mesh>);

/// The parts of a body of several parts that can be drawn as their symbol
/// mesh as it is, and the element's body as drawn whole (`body_mesh` of the
/// element's own mesh: each closed shell wound outward). A part is clean when
/// drawing the whole turned none of its triangles; when the whole is drawn
/// two-sided there is none (the element is drawn whole, as before).
fn clean_parts(model: &IfcModel, index: usize) -> Option<(Vec<bool>, super::body_geometry::Mesh)> {
    let parts = parts_of(model, index).filter(|p| p.len() > 1)?;
    let IfcEntity::BuildingElement {
        solid_shape: Some(shape @ SolidShape::TriangulatedFaceSet { triangles, .. }),
        ..
    } = model.entities.get(index)?
    else {
        return None;
    };
    let drawn = super::body_geometry::body_mesh(super::body_geometry::Body::Solid(shape))?;
    if drawn.triangles.len() != triangles.len() {
        return None;
    }
    let mut at = 0;
    let clean = parts
        .iter()
        .map(|(symbol, _)| {
            let n = model.instancing.meshes[*symbol].triangles.len();
            let same = (at..at + n).all(|i| {
                let t = triangles[i];
                drawn.triangles[i] == [t.0, t.1, t.2]
            });
            at += n;
            same
        })
        .collect();
    Some((clean, drawn))
}

/// How many clean parts, over every element of several parts, draw each
/// symbol mesh.
pub fn part_usage(model: &IfcModel) -> Vec<usize> {
    let mut usage = vec![0; model.instancing.meshes.len()];
    for index in model.instancing.instances.keys() {
        let (Some(parts), Some((clean, _))) = (parts_of(model, *index), clean_parts(model, *index))
        else {
            continue;
        };
        for ((symbol, _), clean) in parts.iter().zip(clean) {
            if clean {
                usage[*symbol] += 1;
            }
        }
    }
    usage
}

/// An element of several parts as it is drawn: its clean parts whose symbol
/// mesh `usage` counts more than once (instances), and every other triangle
/// of its body as drawn whole, in the element's frame (`None` when every part
/// is shared). The triangles drawn are exactly those of the whole.
pub fn drawn_parts(model: &IfcModel, index: usize, usage: &[usize]) -> Option<DrawnParts> {
    let parts = parts_of(model, index)?;
    let (clean, drawn) = clean_parts(model, index)?;
    let mut shared = Vec::new();
    let mut keep = vec![true; drawn.triangles.len()];
    let mut at = 0;
    for (&(symbol, target), clean) in parts.iter().zip(clean) {
        let n = model.instancing.meshes[symbol].triangles.len();
        if clean && usage.get(symbol).copied().unwrap_or(0) > 1 {
            shared.push((symbol, target));
            keep[at..at + n].fill(false);
        }
        at += n;
    }
    let mut rest = drawn;
    let coloured = rest.face_materials.len() == rest.triangles.len();
    let mut k = keep.iter();
    rest.triangles
        .retain(|_| *k.next().expect("one per triangle"));
    if coloured {
        let mut k = keep.iter();
        rest.face_materials
            .retain(|_| *k.next().expect("one per triangle"));
    }
    // Only the vertices the rest uses.
    let mut remap = vec![u32::MAX; rest.vertices.len()];
    let mut vertices = Vec::new();
    for t in &mut rest.triangles {
        for v in t.iter_mut() {
            if remap[*v as usize] == u32::MAX {
                remap[*v as usize] = vertices.len() as u32;
                vertices.push(rest.vertices[*v as usize]);
            }
            *v = remap[*v as usize];
        }
    }
    rest.vertices = vertices;
    let rest = (!rest.triangles.is_empty()).then_some(rest);
    Some((shared, rest))
}

/// A symbol mesh as it is (no reorientation), for a clean part.
pub fn body_mesh(mesh: &SymbolMesh) -> Option<super::body_geometry::Mesh> {
    let out = super::body_geometry::Mesh {
        vertices: mesh.vertices_feet.clone(),
        triangles: mesh.triangles.iter().map(|t| [t.0, t.1, t.2]).collect(),
        face_materials: if mesh.face_materials.len() == mesh.triangles.len() {
            mesh.face_materials.clone()
        } else {
            Vec::new()
        },
    };
    (!out.triangles.is_empty()).then_some(out)
}

/// The target of the element at `index` (identity when none).
pub fn target_of(model: &IfcModel, index: usize) -> Frame {
    model
        .instancing
        .targets
        .get(&index)
        .copied()
        .unwrap_or(IDENTITY)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirrored_instances_share_one_mirrored_mesh_with_proper_targets() {
        let mesh = SymbolMesh {
            vertices_feet: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 2.0, 0.5]],
            triangles: vec![BrepTriangle(0, 1, 2)],
            face_materials: Vec::new(),
        };
        let mut model = IfcModel::default();
        let element = |x: f64, rot: f64| IfcEntity::BuildingElement {
            ifc_type: "IFCFURNITURE".into(),
            name: "chair".into(),
            type_guid: None,
            predefined_type: None,
            storey_index: None,
            material_index: None,
            material_layer_set_index: None,
            material_profile_set_index: None,
            property_set: None,
            location_feet: Some([x, 1.0, 0.0]),
            rotation_radians: Some(rot),
            extrusion: None,
            host_element_index: None,
            solid_shape: None,
            representation_map_index: None,
        };
        model.entities.push(element(3.0, 0.3));
        model.entities.push(element(-2.0, 0.0));
        model.entities.push(element(5.0, 1.0));
        let (c, s) = (0.3_f64.cos(), 0.3_f64.sin());
        let w0 = [[c, -s, 0.0, 3.0], [s, c, 0.0, 1.0], [0.0, 0.0, 1.0, 0.0]];
        let w1 = [
            [-1.0, 0.0, 0.0, -2.0],
            [0.0, 1.0, 0.0, 1.0],
            [0.0, 0.0, 1.0, 0.0],
        ];
        let w2 = [
            [-1.0, 0.0, 0.0, 5.0],
            [0.0, 1.0, 0.0, 1.0],
            [0.0, 0.0, 1.0, 0.0],
        ];
        assert!(register(&mut model, 0, mesh.clone(), w0));
        assert!(register(&mut model, 1, mesh.clone(), w1));
        assert!(register(&mut model, 2, mesh.clone(), w2));
        // One plain and one mirrored copy; the two mirrored instances share.
        assert_eq!(model.instancing.meshes.len(), 2);
        assert_eq!(
            instance_of(&model, 1).unwrap().0,
            instance_of(&model, 2).unwrap().0
        );
        for i in 0..3 {
            let (sym, target) = instance_of(&model, i).expect("consistent");
            assert!(det(&target) > 0.0);
            // World positions equal the mesh placed by its own transform.
            let IfcEntity::BuildingElement {
                location_feet,
                rotation_radians,
                ..
            } = &model.entities[i]
            else {
                unreachable!()
            };
            let p = placement_frame(&Placement::new(*location_feet, *rotation_radians));
            let world = [w0, w1, w2][i];
            let m = &model.instancing.meshes[sym];
            for (k, v) in mesh.vertices_feet.iter().enumerate() {
                let want = apply(&world, *v);
                // The shared vertex k of the (possibly mirrored) copy.
                let got = apply(&compose(&p, &target), m.vertices_feet[k]);
                assert!(
                    (0..3).all(|j| (want[j] - got[j]).abs() < 1e-12),
                    "{i} {k} {want:?} {got:?}"
                );
            }
        }
    }
}
