//! glTF 2.0 exporter — dep-free binary GLB emission.
//!
//! Produces a `.glb` file ready to load into Three.js, BabylonJS,
//! Blender, or any glTF 2.0 viewer. Each `BuildingElement` is drawn as the
//! body the STEP writer gives it ([`super::body_geometry`]), at its
//! `location_feet` turned by its `rotation_radians`. A plain rectangular
//! extrusion shares one unit cube, scaled by its node's matrix; every
//! other body (a sketched profile, a steel section, a swept or revolved
//! solid, a brep) has its own mesh. An element with no body gets a node
//! with no mesh. Each element gets a material drawn from
//! `PbrMaterial::from_material_info`.
//!
//! Element nodes are in the model's frame, feet with +Z up. glTF is
//! metres with +Y up, so they hang under one root node whose matrix
//! ([`Z_UP_FEET_TO_Y_UP_METRES`]) converts; without it every model lay on
//! its side, ten times too large, in any glTF viewer.
//!
//! GLB file layout (per the glTF 2.0 spec):
//!
//! ```text
//! [u32 magic=0x46546C67 "glTF"] [u32 version=2] [u32 total_length]
//! [u32 json_chunk_length] [u32 chunk_type=0x4E4F534A "JSON"]
//! [json_chunk_length bytes UTF-8 JSON, padded to 4-byte boundary with 0x20]
//! [u32 bin_chunk_length] [u32 chunk_type=0x004E4942 "BIN\0"]
//! [bin_chunk_length bytes binary buffer, padded to 4-byte with 0x00]
//! ```

use super::IfcModel;
use super::body_geometry::{self, Body, Placement, body_mesh};
use super::entities::{IfcEntity, PropertyValue};
use super::pbr::PbrMaterial;
use serde::{Deserialize, Serialize};

/// Top-level glTF 2.0 JSON document.
///
/// Matches the glTF spec field names exactly (camelCase) so the
/// serialized JSON is a valid glTF file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GltfDocument {
    pub asset: Asset,
    pub scene: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scenes: Vec<Scene>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<Node>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub meshes: Vec<Mesh>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub materials: Vec<Material>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub buffers: Vec<Buffer>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[serde(rename = "bufferViews")]
    pub buffer_views: Vec<BufferView>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accessors: Vec<Accessor>,
    /// `EXT_mesh_gpu_instancing`, `EXT_meshopt_compression`.
    #[serde(
        rename = "extensionsUsed",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub extensions_used: Vec<String>,
    #[serde(
        rename = "extensionsRequired",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub extensions_required: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Asset {
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generator: Option<String>,
}

impl Default for Asset {
    fn default() -> Self {
        Self {
            version: "2.0".into(),
            generator: Some(format!("rvt-open {}", env!("CARGO_PKG_VERSION"))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scene {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub nodes: Vec<usize>,
}

/// What the visible 3D scene draws, as Revit's Shaded 3D view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayOptions {
    /// Room and space volumes in the visible scene. Revit's 3D views never
    /// draw them. Off, glTF keeps them out of the default scene, under a
    /// `rooms` node that only its second scene (`with rooms`) shows; the
    /// `.frag` keeps their items, properties and place in the tree, with no
    /// geometry (its format has no stored visibility).
    pub rooms: bool,
    /// Feature edges as glTF `LINES` (Shaded with Show Edges): where two
    /// faces meet at a hard-shaded angle, and open boundaries. Indices only:
    /// the lines reuse the mesh's points.
    pub edges: bool,
}

impl Default for DisplayOptions {
    fn default() -> Self {
        Self {
            rooms: false,
            edges: true,
        }
    }
}

/// Faces meeting at an angle whose cosine is below this are shaded hard
/// (`push_mesh`'s normal split) and their edge is drawn.
const HARD_EDGE_COS: f64 = 0.906;

/// An edge's face normals and the first face's point pair.
type EdgeFaces = (Vec<[f64; 3]>, (u32, u32));

/// The feature edges of a triangle mesh, as `LINES` index pairs into its
/// points: every edge (points welded by position) used by one triangle, by
/// more than two, or by two whose planes meet at more than
/// 25 degrees (either winding). Zero-area triangles are not faces,
/// and a triangle saved once per side counts once.
pub fn feature_edges(points: &[[f32; 3]], triangles: &[[u32; 3]]) -> Vec<u32> {
    let key = |i: u32| -> Option<[i64; 3]> {
        let p = points.get(i as usize)?;
        Some(p.map(|c| (f64::from(c) * 1e5).round() as i64))
    };
    let mut weld: std::collections::HashMap<[i64; 3], u32> = Default::default();
    // welded edge -> (normals of its faces, the first face's point pair)
    let mut edges: std::collections::BTreeMap<(u32, u32), EdgeFaces> = Default::default();
    let mut faces: std::collections::HashSet<[u32; 3]> = Default::default();
    for t in triangles {
        let (Some(a), Some(b), Some(c)) = (key(t[0]), key(t[1]), key(t[2])) else {
            continue;
        };
        let p = |i: u32| points[i as usize].map(f64::from);
        let (p0, p1, p2) = (p(t[0]), p(t[1]), p(t[2]));
        let u = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
        let w = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];
        let n = [
            u[1] * w[2] - u[2] * w[1],
            u[2] * w[0] - u[0] * w[2],
            u[0] * w[1] - u[1] * w[0],
        ];
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if len.is_nan() || len <= 1e-12 {
            continue;
        }
        let n = n.map(|c| c / len);
        let w3: Vec<u32> = [a, b, c]
            .iter()
            .map(|k| {
                let fresh = weld.len() as u32;
                *weld.entry(*k).or_insert(fresh)
            })
            .collect();
        // A face saved once per side is one face.
        let mut key3 = [w3[0], w3[1], w3[2]];
        key3.sort_unstable();
        if !faces.insert(key3) {
            continue;
        }
        for k in 0..3 {
            let (i, j) = (w3[k], w3[(k + 1) % 3]);
            if i == j {
                continue;
            }
            let e = edges
                .entry((i.min(j), i.max(j)))
                .or_insert_with(|| (Vec::new(), (t[k], t[(k + 1) % 3])));
            e.0.push(n);
        }
    }
    let mut out = Vec::new();
    for (normals, (a, b)) in edges.into_values() {
        let hard = match normals.as_slice() {
            // Either winding: a terrain or saved mesh can mix them.
            [n, m] => (n[0] * m[0] + n[1] * m[1] + n[2] * m[2]).abs() < HARD_EDGE_COS,
            _ => true,
        };
        if hard {
            out.extend([a, b]);
        }
    }
    out
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Node {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mesh: Option<usize>,
    /// 16-element column-major transformation matrix. `None` =
    /// identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matrix: Option<[f32; 16]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<usize>,
    /// Application-specific payload. glTF loaders surface this as
    /// the object's `userData`, which is how the viewer maps a
    /// picked mesh back to its entity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extras: Option<NodeExtras>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<serde_json::Value>,
}

/// Identity the viewer needs on every drawable node: the
/// `model.entities` index behind it and its IFC type. The viewer
/// reads these as `userData.entityIndex` / `userData.ifcType` to
/// drive picking, selection highlight, and category visibility —
/// all three of which were inert while the nodes carried no
/// extras at all.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeExtras {
    #[serde(rename = "entityIndex")]
    pub entity_index: usize,
    #[serde(rename = "ifcType")]
    pub ifc_type: String,
    /// The Revit ElementId, from the element's `Revit` set.
    #[serde(rename = "elementId", default, skip_serializing_if = "Option::is_none")]
    pub element_id: Option<i64>,
    /// The name Revit shows for the element's category.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// The category's `BuiltInCategory` enum name.
    #[serde(
        rename = "builtInCategory",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub builtin_category: Option<String>,
    /// on a node drawing several elements through
    /// `EXT_mesh_gpu_instancing`, each instance's identity, by instance index
    /// (three.js: the raycaster's `instanceId` into `userData.instances`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub instances: Vec<NodeExtras>,
    /// The category's parent, for a subcategory such as `Runs`.
    #[serde(
        rename = "parentCategory",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub parent_category: Option<String>,
}

/// An element's ElementId, category name, `BuiltInCategory` enum name and
/// parent category.
type Identity = (Option<i64>, Option<String>, Option<String>, Option<String>);

/// Each element's extras from its `Revit` set
/// ([`super::revit_identity::SET`]), by entity index.
fn revit_identities(model: &IfcModel) -> std::collections::BTreeMap<usize, Identity> {
    let mut out = std::collections::BTreeMap::new();
    for entity in &model.entities {
        let IfcEntity::ElementPropertySet { element, set } = entity else {
            continue;
        };
        if set.name != super::revit_identity::SET {
            continue;
        }
        let mut id = None;
        let (mut category, mut builtin, mut parent) = (None, None, None);
        for p in &set.properties {
            match (p.name.as_str(), &p.value) {
                ("ElementId", PropertyValue::Integer(v)) => id = Some(*v),
                ("Category", PropertyValue::Label(v)) => category = Some(v.clone()),
                ("BuiltInCategory", PropertyValue::Label(v)) => builtin = Some(v.clone()),
                ("ParentCategory", PropertyValue::Label(v)) => parent = Some(v.clone()),
                _ => {}
            }
        }
        out.insert(*element, (id, category, builtin, parent));
    }
    out
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Mesh {
    pub primitives: Vec<Primitive>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Primitive {
    pub attributes: std::collections::BTreeMap<String, usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indices: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<usize>,
    /// 4 = triangle list (the default per the glTF spec).
    #[serde(default)]
    pub mode: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Material {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(rename = "pbrMetallicRoughness")]
    pub pbr_metallic_roughness: PbrMetallicRoughness,
    #[serde(default, rename = "doubleSided")]
    pub double_sided: bool,
    #[serde(default, rename = "alphaMode", skip_serializing_if = "Option::is_none")]
    pub alpha_mode: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PbrMetallicRoughness {
    /// `[r, g, b, a]` linear 0-1.
    #[serde(rename = "baseColorFactor")]
    pub base_color_factor: [f32; 4],
    #[serde(rename = "metallicFactor")]
    pub metallic_factor: f32,
    #[serde(rename = "roughnessFactor")]
    pub roughness_factor: f32,
}

impl Default for PbrMetallicRoughness {
    fn default() -> Self {
        Self {
            base_color_factor: [0.75, 0.75, 0.75, 1.0],
            metallic_factor: 0.0,
            roughness_factor: 0.6,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Buffer {
    #[serde(rename = "byteLength")]
    pub byte_length: usize,
    /// `None` = embedded in the GLB binary chunk (the common case
    /// for single-file .glb output).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BufferView {
    pub buffer: usize,
    #[serde(rename = "byteOffset", default)]
    pub byte_offset: usize,
    #[serde(rename = "byteLength")]
    pub byte_length: usize,
    /// glTF target: 34962 = ARRAY_BUFFER (vertex attrs),
    /// 34963 = ELEMENT_ARRAY_BUFFER (indices). `None` for
    /// unspecified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<u32>,
    #[serde(
        rename = "byteStride",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub byte_stride: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Accessor {
    #[serde(rename = "bufferView")]
    pub buffer_view: usize,
    #[serde(rename = "byteOffset", default)]
    pub byte_offset: usize,
    /// 5120..5126 per glTF component-type enum:
    /// 5120=i8, 5121=u8, 5122=i16, 5123=u16, 5125=u32, 5126=f32.
    #[serde(rename = "componentType")]
    pub component_type: u32,
    pub count: usize,
    /// `"SCALAR"`, `"VEC2"`, `"VEC3"`, `"VEC4"`, `"MAT4"`, etc.
    #[serde(rename = "type")]
    pub type_: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub max: Vec<f32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub min: Vec<f32>,
}

/// Unit-cube vertex positions ([-0.5, 0.5]³, 24 verts — 4 per
/// face for correct per-face normals). Shared across every
/// element in the first-pass export.
fn unit_cube_vertices() -> [f32; 24 * 3] {
    // 6 faces × 4 verts × 3 coords.
    let h = 0.5_f32;
    [
        // +X face
        h, -h, -h, h, h, -h, h, h, h, h, -h, h, // -X face
        -h, -h, h, -h, h, h, -h, h, -h, -h, -h, -h, // +Y face
        -h, h, -h, -h, h, h, h, h, h, h, h, -h, // -Y face
        h, -h, -h, h, -h, h, -h, -h, h, -h, -h, -h, // +Z face
        -h, -h, h, h, -h, h, h, h, h, -h, h, h, // -Z face
        -h, -h, -h, -h, h, -h, h, h, -h, h, -h, -h,
    ]
}

fn unit_cube_indices() -> [u16; 36] {
    // 2 triangles per face.
    [
        0, 1, 2, 0, 2, 3, // +X
        4, 5, 6, 4, 6, 7, // -X
        8, 9, 10, 8, 10, 11, // +Y
        12, 13, 14, 12, 14, 15, // -Y
        16, 17, 18, 16, 18, 19, // +Z
        20, 21, 22, 20, 22, 23, // -Z
    ]
}

/// Column-major matrix taking the model's frame (feet, +Z up) to glTF's
/// (metres, +Y up): model `(x, y, z)` feet is glTF `(x, z, -y)` metres.
pub const Z_UP_FEET_TO_Y_UP_METRES: [f32; 16] = [
    0.3048, 0.0, 0.0, 0.0, // model X stays X
    0.0, 0.0, -0.3048, 0.0, // model Y is glTF -Z
    0.0, 0.3048, 0.0, 0.0, // model Z (up) is glTF Y (up)
    0.0, 0.0, 0.0, 1.0,
];

/// Build a glTF document + binary buffer from an `IfcModel`.
/// Returns `(document, binary_buffer)` — wire into
/// [`write_glb`] for a single-file `.glb` output.
///
/// Every `BuildingElement` becomes a node carrying its entity index and
/// IFC type, a child of the root node that converts the model's frame to
/// glTF's. Its mesh is its body in element-local feet, and its matrix
/// its placement: the shared unit cube scaled, turned and moved for a
/// plain rectangular extrusion, a per-element mesh turned and moved for
/// any other body, and no mesh for an element with none.
fn node_extras(
    identities: &std::collections::BTreeMap<usize, Identity>,
    entity_index: usize,
    ifc_type: &str,
) -> NodeExtras {
    let (element_id, category, builtin_category, parent_category) =
        identities.get(&entity_index).cloned().unwrap_or_default();
    NodeExtras {
        entity_index,
        ifc_type: ifc_type.to_string(),
        element_id,
        category,
        builtin_category,
        instances: Vec::new(),
        parent_category,
    }
}

pub fn build_gltf(model: &IfcModel) -> (GltfDocument, Vec<u8>) {
    build_gltf_with(model, &GltfOptions::default())
}

/// How [`build_gltf_with`] writes the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct GltfOptions {
    /// Elements drawing one representation map in one material share one
    /// node with `EXT_mesh_gpu_instancing` (three.js GLTFLoader: one
    /// `InstancedMesh`), listed as required. Off: a node per element, the
    /// mesh shared. Default on.
    pub instancing: bool,
    /// Leave NORMAL out of meshes whose faces are all flat (glTF 2.0: the
    /// client computes flat normals; a client that does not draws them
    /// unlit). Default off.
    pub omit_flat_normals: bool,
    /// Compress vertex and index data with `EXT_meshopt_compression`, losslessly
    /// (no filters), listed as required (three.js: `setMeshoptDecoder`).
    /// Default off.
    pub meshopt: bool,
    /// What the visible scene draws: rooms, feature edges.
    pub display: DisplayOptions,
}

impl Default for GltfOptions {
    fn default() -> Self {
        Self {
            instancing: true,
            omit_flat_normals: false,
            meshopt: false,
            display: DisplayOptions::default(),
        }
    }
}

impl GltfOptions {
    pub fn new(instancing: bool, omit_flat_normals: bool, meshopt: bool) -> Self {
        Self {
            instancing,
            omit_flat_normals,
            meshopt,
            display: DisplayOptions::default(),
        }
    }

    /// These options drawing what `display` asks for.
    pub fn with_display(self, display: DisplayOptions) -> Self {
        Self { display, ..self }
    }
}

impl From<DisplayOptions> for GltfOptions {
    fn from(display: DisplayOptions) -> Self {
        Self::default().with_display(display)
    }
}

/// `(POSITION, indices, NORMAL, feature-edge LINES indices)` accessors of
/// one mesh part.
type Accessors = (usize, usize, Option<usize>, Option<usize>);

/// An instanced group: (0 a representation map | 1 a symbol mesh of a part,
/// its index, the element material).
type InstanceKey = (u8, usize, Option<usize>);

/// The parts' triangles, each in its face material else
/// `material`, and their feature edges in black.
fn primitives_of(
    doc: &mut GltfDocument,
    edge_material: &mut Option<usize>,
    parts: &[(Accessors, Option<usize>)],
    material: Option<usize>,
    materials: usize,
) -> Vec<Primitive> {
    let mut out = Vec::new();
    for ((position, indices, normal, edges), face) in parts {
        let mut attributes = std::collections::BTreeMap::new();
        attributes.insert("POSITION".into(), *position);
        if let Some(normal) = normal {
            attributes.insert("NORMAL".into(), *normal);
        }
        out.push(Primitive {
            attributes: attributes.clone(),
            indices: Some(*indices),
            material: face.filter(|m| *m < materials).or(material),
            mode: 4, // TRIANGLES
        });
        if let Some(edges) = edges {
            let m = *edge_material.get_or_insert_with(|| {
                doc.materials
                    .push(colour_material("Edges".into(), 0x000000, 1.0));
                doc.materials.len() - 1
            });
            // The triangles' points (and normals, when they carry them).
            out.push(Primitive {
                attributes,
                indices: Some(*edges),
                material: Some(m),
                mode: 1, // LINES
            });
        }
    }
    out
}

/// A mapped element's node matrix (column-major, model feet): its placement
/// times its map's target.
fn mapped_matrix_f64(placement: &Placement, target: &super::instancing::Frame) -> [f64; 16] {
    let m = super::instancing::compose(&super::instancing::placement_frame(placement), target);
    [
        m[0][0], m[1][0], m[2][0], 0.0, m[0][1], m[1][1], m[2][1], 0.0, m[0][2], m[1][2], m[2][2],
        0.0, m[0][3], m[1][3], m[2][3], 1.0,
    ]
}

fn mapped_matrix(placement: &Placement, target: &super::instancing::Frame) -> [f32; 16] {
    mapped_matrix_f64(placement, target).map(|v| v as f32)
}

/// The unit quaternion (x, y, z, w) of a column-major matrix's rotation.
fn rotation_quaternion(m: &[f64; 16]) -> [f64; 4] {
    let r = |row: usize, col: usize| m[col * 4 + row];
    let trace = r(0, 0) + r(1, 1) + r(2, 2);
    let q = if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0;
        [
            (r(2, 1) - r(1, 2)) / s,
            (r(0, 2) - r(2, 0)) / s,
            (r(1, 0) - r(0, 1)) / s,
            0.25 * s,
        ]
    } else if r(0, 0) > r(1, 1) && r(0, 0) > r(2, 2) {
        let s = (1.0 + r(0, 0) - r(1, 1) - r(2, 2)).sqrt() * 2.0;
        [
            0.25 * s,
            (r(0, 1) + r(1, 0)) / s,
            (r(0, 2) + r(2, 0)) / s,
            (r(2, 1) - r(1, 2)) / s,
        ]
    } else if r(1, 1) > r(2, 2) {
        let s = (1.0 + r(1, 1) - r(0, 0) - r(2, 2)).sqrt() * 2.0;
        [
            (r(0, 1) + r(1, 0)) / s,
            0.25 * s,
            (r(1, 2) + r(2, 1)) / s,
            (r(0, 2) - r(2, 0)) / s,
        ]
    } else {
        let s = (1.0 + r(2, 2) - r(0, 0) - r(1, 1)).sqrt() * 2.0;
        [
            (r(0, 2) + r(2, 0)) / s,
            (r(1, 2) + r(2, 1)) / s,
            0.25 * s,
            (r(1, 0) - r(0, 1)) / s,
        ]
    };
    let l = q.iter().map(|v| v * v).sum::<f64>().sqrt();
    q.map(|v| v / l)
}

/// Append f32 data as a buffer view (no target: instance data) and an
/// accessor of `ty`; returns the accessor.
fn push_floats(doc: &mut GltfDocument, bin: &mut Vec<u8>, data: &[f32], ty: &str) -> usize {
    let off = bin.len();
    bin.extend_from_slice(&bytemuck_cast_f32(data));
    pad_to_4(bin);
    doc.buffer_views.push(BufferView {
        buffer: 0,
        byte_offset: off,
        byte_length: data.len() * 4,
        target: None,
        byte_stride: None,
        extensions: None,
    });
    let width = if ty == "VEC4" { 4 } else { 3 };
    doc.accessors.push(Accessor {
        buffer_view: doc.buffer_views.len() - 1,
        byte_offset: 0,
        component_type: 5126,
        count: data.len() / width,
        type_: ty.into(),
        min: Vec::new(),
        max: Vec::new(),
    });
    doc.accessors.len() - 1
}

/// A mesh's exact content as a key.
fn mesh_hash(mesh: &body_geometry::Mesh) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    mesh.vertices.len().hash(&mut h);
    for v in &mesh.vertices {
        v.map(f64::to_bits).hash(&mut h);
    }
    mesh.triangles.hash(&mut h);
    mesh.face_materials.hash(&mut h);
    h.finish()
}

/// [`build_gltf`] with `opts`.
pub fn build_gltf_with(model: &IfcModel, opts: &GltfOptions) -> (GltfDocument, Vec<u8>) {
    let identities = revit_identities(model);
    let mut bin = Vec::<u8>::new();
    let mut doc = GltfDocument::default();

    // Binary layout:
    //   [positions: 24 * VEC3 f32 = 288 bytes]
    //   [indices: 36 * u16 = 72 bytes → pad to 4-aligned = 72]
    let positions = unit_cube_vertices();
    let pos_bytes = bytemuck_cast_f32(&positions);
    let pos_offset = 0;
    let pos_len = pos_bytes.len();
    bin.extend_from_slice(&pos_bytes);
    pad_to_4(&mut bin);

    let indices = unit_cube_indices();
    let idx_bytes = bytemuck_cast_u16(&indices);
    let idx_offset = bin.len();
    let idx_len = idx_bytes.len();
    bin.extend_from_slice(&idx_bytes);
    pad_to_4(&mut bin);

    // BufferView 0: positions.
    doc.buffer_views.push(BufferView {
        buffer: 0,
        byte_offset: pos_offset,
        byte_length: pos_len,
        target: Some(34962), // ARRAY_BUFFER
        byte_stride: None,
        extensions: None,
    });
    // BufferView 1: indices.
    doc.buffer_views.push(BufferView {
        buffer: 0,
        byte_offset: idx_offset,
        byte_length: idx_len,
        target: Some(34963), // ELEMENT_ARRAY_BUFFER
        byte_stride: None,
        extensions: None,
    });

    // Accessor 0: POSITION (VEC3 f32, 24 verts).
    doc.accessors.push(Accessor {
        buffer_view: 0,
        byte_offset: 0,
        component_type: 5126, // f32
        count: 24,
        type_: "VEC3".into(),
        min: vec![-0.5, -0.5, -0.5],
        max: vec![0.5, 0.5, 0.5],
    });
    // Accessor 1: indices (SCALAR u16, 36 indices).
    doc.accessors.push(Accessor {
        buffer_view: 1,
        byte_offset: 0,
        component_type: 5123, // u16
        count: 36,
        type_: "SCALAR".into(),
        ..Accessor {
            buffer_view: 0,
            byte_offset: 0,
            component_type: 0,
            count: 0,
            type_: String::new(),
            max: Vec::new(),
            min: Vec::new(),
        }
    });
    // Accessor 2: the cube's NORMAL (4 per face, in the face order of the positions).
    {
        let mut nrm = Vec::<f32>::with_capacity(72);
        for n in [
            [1.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -1.0_f32],
        ] {
            for _ in 0..4 {
                nrm.extend_from_slice(&n);
            }
        }
        let off = bin.len();
        bin.extend_from_slice(&bytemuck_cast_f32(&nrm));
        pad_to_4(&mut bin);
        doc.buffer_views.push(BufferView {
            buffer: 0,
            byte_offset: off,
            byte_length: 72 * 4,
            target: Some(34962),
            byte_stride: None,
            extensions: None,
        });
        doc.accessors.push(Accessor {
            buffer_view: doc.buffer_views.len() - 1,
            byte_offset: 0,
            component_type: 5126,
            count: 24,
            type_: "VEC3".into(),
            min: Vec::new(),
            max: Vec::new(),
        });
    }

    // The cube's feature edges: its twelve sides.
    let cube_edges = opts
        .display
        .edges
        .then(|| {
            let points: Vec<[f32; 3]> = positions
                .chunks_exact(3)
                .map(|p| [p[0], p[1], p[2]])
                .collect();
            let tris: Vec<[u32; 3]> = indices
                .chunks_exact(3)
                .map(|t| [u32::from(t[0]), u32::from(t[1]), u32::from(t[2])])
                .collect();
            push_indices(&mut doc, &mut bin, &feature_edges(&points, &tris), true)
        })
        .flatten();
    let mut edge_material: Option<usize> = None;

    // Materials — one per model.materials entry via PbrMaterial.
    for m in &model.materials {
        let pbr = PbrMaterial::from_material_info(m);
        let mat = Material {
            name: Some(m.name.clone()),
            pbr_metallic_roughness: PbrMetallicRoughness {
                base_color_factor: [
                    pbr.base_color_rgb[0],
                    pbr.base_color_rgb[1],
                    pbr.base_color_rgb[2],
                    pbr.alpha,
                ],
                metallic_factor: pbr.metallic,
                roughness_factor: pbr.roughness,
            },
            double_sided: pbr.double_sided,
            alpha_mode: if pbr.alpha < 1.0 {
                Some("BLEND".into())
            } else {
                None
            },
        };
        doc.materials.push(mat);
    }

    // An element with no material of its own takes its category's colour,
    // one material per category, after the model's own.
    let mut category_materials: std::collections::BTreeMap<String, usize> = Default::default();
    // Layer colours, one material per colour and alpha.
    let mut layer_materials: std::collections::BTreeMap<(u32, u32), usize> = Default::default();
    // A family instance whose type draws in one material takes that
    // material's colour, when the colour was read. One drawn in
    // several keeps its category's: a single body cannot show which part is
    // which.
    let single_material: std::collections::BTreeMap<usize, usize> = model
        .material_constituent_sets
        .iter()
        .filter_map(|set| match set.material_indices.as_slice() {
            [material]
                if model
                    .materials
                    .get(*material)
                    .is_some_and(|info| info.color_packed.is_some()) =>
            {
                Some(
                    set.elements
                        .iter()
                        .map(move |&element| (element, *material)),
                )
            }
            _ => None,
        })
        .flatten()
        // A member written with a material profile set of one material
        // takes it the same way.
        .chain(
            model
                .entities
                .iter()
                .enumerate()
                .filter_map(|(index, entity)| match entity {
                    IfcEntity::BuildingElement {
                        material_profile_set_index: Some(set),
                        ..
                    } => match model.material_profile_sets.get(*set)?.profiles.as_slice() {
                        [profile]
                            if model
                                .materials
                                .get(profile.material_index)
                                .is_some_and(|info| info.color_packed.is_some()) =>
                        {
                            Some((index, profile.material_index))
                        }
                        _ => None,
                    },
                    _ => None,
                }),
        )
        .collect();

    // one primitive per face material (`None`: the element's).
    #[allow(clippy::type_complexity)]
    let mut shared_meshes: std::collections::BTreeMap<usize, Vec<(Accessors, Option<usize>)>> =
        Default::default();
    // each symbol mesh's accessors, for the parts of a body.
    let mut part_meshes: std::collections::BTreeMap<usize, Vec<(Accessors, Option<usize>)>> =
        Default::default();
    let usage = super::instancing::part_usage(model);
    // exactly equal meshes of elements without a map share their
    // accessors too, keyed by their content.
    let mut same_meshes: std::collections::HashMap<u64, Vec<(Accessors, Option<usize>)>> =
        Default::default();
    // elements drawing one map in one material, for
    // `EXT_mesh_gpu_instancing`: (map, material) -> (parts, instances as
    // (entity index, name, model-frame matrix)).
    #[allow(clippy::type_complexity)]
    let mut instanced: std::collections::BTreeMap<
        InstanceKey,
        (
            Vec<(Accessors, Option<usize>)>,
            Vec<(usize, String, [f64; 16])>,
        ),
    > = Default::default();
    // Where each instanced group's node goes among the scene's nodes.
    let mut instanced_slot: std::collections::BTreeMap<InstanceKey, usize> = Default::default();
    // Per-element: a node carrying the entity's identity, with the
    // shared cube or its own mesh.
    let mut scene_nodes: Vec<usize> = Vec::new();
    // Rooms and spaces kept out of the default scene.
    let mut room_nodes: Vec<usize> = Vec::new();
    for (entity_index, ent) in model.entities.iter().enumerate() {
        let IfcEntity::BuildingElement {
            ifc_type,
            name,
            type_guid,
            material_index,
            material_layer_set_index,
            representation_map_index,
            location_feet,
            rotation_radians,
            ..
        } = ent
        else {
            continue;
        };
        // An opening is a void in its host, not a solid.
        if ifc_type == "IFCOPENINGELEMENT" {
            continue;
        }
        let hidden_room = !opts.display.rooms && ifc_type == "IFCSPACE";
        let placement = Placement::new(*location_feet, *rotation_radians);
        // a host with its openings cut draws its own mesh.
        let body = body_geometry::drawn_body(model, entity_index, ent);
        let representation_map_index = if model.display_bodies.contains_key(&entity_index) {
            &None
        } else {
            representation_map_index
        };
        let (c, s) = (placement.cos as f32, placement.sin as f32);
        let [tx, ty, tz] = placement.origin.map(|v| v as f32);
        let body_matrix = [
            c,
            s,
            0.0,
            0.0,
            if s == 0.0 { 0.0 } else { -s },
            c,
            0.0,
            0.0,
            0.0,
            0.0,
            1.0,
            0.0,
            tx,
            ty,
            tz,
            1.0,
        ];
        // A layered element is one node per layer, each in its
        // material's colour, all carrying the element's identity.
        let layered = type_guid
            .as_deref()
            .and_then(|tag| tag.parse::<u32>().ok())
            .and_then(|id| model.element_layers.get(&id));
        if let (Some(layers), Some(body)) = (layered, body) {
            let [nx, ny] = layers.exterior_normal;
            let local = [
                nx * placement.cos + ny * placement.sin,
                -nx * placement.sin + ny * placement.cos,
            ];
            let widths: Vec<f64> = layers.layers.iter().map(|l| l.width_feet).collect();
            let meshes = match body {
                Body::Extrusion(extrusion) if layers.stacked => {
                    body_geometry::stacked_extrusion_meshes(extrusion, &widths)
                }
                Body::Extrusion(extrusion) => {
                    body_geometry::layered_extrusion_meshes(extrusion, local, &widths)
                }
                Body::Solid(shape) if !layers.stacked => {
                    body_geometry::layered_stepped_meshes(shape, local, &widths)
                        .or_else(|| body_geometry::one_layer_solid_meshes(shape, local, &widths))
                }
                _ => None,
            };
            if let Some(meshes) = meshes {
                for (mesh, band) in meshes.iter().zip(&layers.layers) {
                    let Some(accessors) = push_mesh(&mut doc, &mut bin, mesh, opts) else {
                        continue;
                    };
                    let material = match band.color_packed {
                        Some(packed) => {
                            let rgb = ((packed & 0xff) << 16)
                                | (packed & 0xff00)
                                | ((packed >> 16) & 0xff);
                            let alpha = (1.0 - band.transparency.clamp(0.0, 1.0)) as f32;
                            *layer_materials
                                .entry((rgb, alpha.to_bits()))
                                .or_insert_with(|| {
                                    doc.materials.push(colour_material(
                                        format!("Layer {rgb:06x}"),
                                        rgb,
                                        alpha,
                                    ));
                                    doc.materials.len() - 1
                                })
                        }
                        None => *category_materials
                            .entry(ifc_type.clone())
                            .or_insert_with(|| {
                                doc.materials.push(category_material(ifc_type));
                                doc.materials.len() - 1
                            }),
                    };
                    let primitives = primitives_of(
                        &mut doc,
                        &mut edge_material,
                        &[(accessors, None)],
                        Some(material),
                        model.materials.len(),
                    );
                    let mesh_idx = doc.meshes.len();
                    doc.meshes.push(Mesh {
                        primitives,
                        name: Some(name.clone()),
                    });
                    let node_idx = doc.nodes.len();
                    doc.nodes.push(Node {
                        name: Some(name.clone()),
                        mesh: Some(mesh_idx),
                        matrix: Some(body_matrix),
                        children: Vec::new(),
                        extras: Some(node_extras(&identities, entity_index, ifc_type)),
                        extensions: None,
                    });
                    if hidden_room {
                        room_nodes.push(node_idx);
                    } else {
                        scene_nodes.push(node_idx);
                    }
                }
                continue;
            }
        }
        // a body of several parts (a railing's balusters): its
        // repeated parts are drawn as instances of their symbol meshes, the
        // rest as the element's own mesh.
        let mut pieces: Vec<(usize, super::instancing::Frame)> = Vec::new();
        let multi = super::instancing::drawn_parts(model, entity_index, &usage)
            .filter(|_| !model.display_bodies.contains_key(&entity_index));
        // (accessor pairs with their face material, matrix)
        #[allow(clippy::type_complexity)]
        let drawn: Option<(Vec<(Accessors, Option<usize>)>, [f32; 16])> =
            if let Some((shared, rest)) = multi {
                pieces = shared;
                let mut parts = Vec::new();
                if let Some(mesh) = rest {
                    for (part, material) in super::graph::split_by_face_material(mesh) {
                        if let Some(a) = push_mesh(&mut doc, &mut bin, &part, opts) {
                            parts.push((a, material));
                        }
                    }
                }
                Some((parts, body_matrix))
            } else {
                match body {
                    Some(Body::Extrusion(ex))
                        if ex.profile_override.is_none()
                            && [ex.width_feet, ex.depth_feet, ex.height_feet]
                                .iter()
                                .all(|v| v.is_finite() && *v > 0.0) =>
                    {
                        let (sx, sy, sz) = (
                            ex.width_feet as f32,
                            ex.depth_feet as f32,
                            ex.height_feet as f32,
                        );
                        // Translate · rotate about Z · scale the unit cube, whose
                        // base sits half its height below its centre.
                        Some((
                            vec![((0, 1, Some(2), cube_edges), None)],
                            [
                                c * sx,
                                s * sx,
                                0.0,
                                0.0,
                                if s == 0.0 { 0.0 } else { -s * sy },
                                c * sy,
                                0.0,
                                0.0,
                                0.0,
                                0.0,
                                sz,
                                0.0,
                                tx,
                                ty,
                                tz + sz * 0.5,
                                1.0,
                            ],
                        ))
                    }
                    Some(body) => {
                        // Instances of one representation map share one mesh.
                        // a mapped element's matrix carries its map's
                        // target (the symbol frame into the element's).
                        let matrix = match representation_map_index {
                            Some(_) => mapped_matrix(
                                &placement,
                                &super::instancing::target_of(model, entity_index),
                            ),
                            None => body_matrix,
                        };
                        let cached =
                            representation_map_index.and_then(|i| shared_meshes.get(&i).cloned());
                        match cached {
                            Some(accessors) => Some((accessors, matrix)),
                            None => body_mesh(body).and_then(|mesh| {
                                let key =
                                    representation_map_index.is_none().then(|| mesh_hash(&mesh));
                                if let Some(parts) = key.and_then(|k| same_meshes.get(&k)) {
                                    return Some((parts.clone(), matrix));
                                }
                                let mut parts = Vec::new();
                                for (part, material) in super::graph::split_by_face_material(mesh) {
                                    parts.push((
                                        push_mesh(&mut doc, &mut bin, &part, opts)?,
                                        material,
                                    ));
                                }
                                if let Some(i) = representation_map_index {
                                    shared_meshes.insert(*i, parts.clone());
                                }
                                if let Some(k) = key {
                                    same_meshes.insert(k, parts.clone());
                                }
                                Some((parts, matrix))
                            }),
                        }
                    }
                    None => None,
                }
            };
        let (mesh, matrix, children) = match drawn {
            Some((parts, matrix)) => {
                let material = material_index
                    .or_else(|| single_material.get(&entity_index).copied())
                    // A wall, floor or roof holds a layer set: its first
                    // layer with a colour.
                    .or_else(|| {
                        material_layer_set_index
                            .and_then(|i| model.material_layer_sets.get(i))
                            .and_then(|set| {
                                set.layers
                                    .iter()
                                    .filter_map(|l| l.material_index)
                                    .find(|m| {
                                        model
                                            .materials
                                            .get(*m)
                                            .is_some_and(|i| i.color_packed.is_some())
                                    })
                            })
                    })
                    .or_else(|| {
                        Some(
                            *category_materials
                                .entry(ifc_type.clone())
                                .or_insert_with(|| {
                                    doc.materials.push(category_material(ifc_type));
                                    doc.materials.len() - 1
                                }),
                        )
                    });
                if let (true, Some(map)) =
                    (opts.instancing && !hidden_room, representation_map_index)
                {
                    let key = (0, *map, material);
                    let group = instanced
                        .entry(key)
                        .or_insert_with(|| (parts.clone(), Vec::new()));
                    group.1.push((
                        entity_index,
                        name.clone(),
                        mapped_matrix_f64(
                            &placement,
                            &super::instancing::target_of(model, entity_index),
                        ),
                    ));
                    if let std::collections::btree_map::Entry::Vacant(v) = instanced_slot.entry(key)
                    {
                        v.insert(scene_nodes.len());
                        scene_nodes.push(usize::MAX);
                    }
                    continue;
                }
                // the repeated parts.
                let mut children = Vec::new();
                for (symbol, target) in pieces.drain(..) {
                    let accessors = match part_meshes.get(&symbol) {
                        Some(a) => a.clone(),
                        None => {
                            let mut a = Vec::new();
                            if let Some(mesh) =
                                super::instancing::body_mesh(&model.instancing.meshes[symbol])
                            {
                                for (part, face) in super::graph::split_by_face_material(mesh) {
                                    if let Some(x) = push_mesh(&mut doc, &mut bin, &part, opts) {
                                        a.push((x, face));
                                    }
                                }
                            }
                            part_meshes.insert(symbol, a.clone());
                            a
                        }
                    };
                    if accessors.is_empty() {
                        continue;
                    }
                    let m = mapped_matrix_f64(&placement, &target);
                    if opts.instancing && !hidden_room {
                        let key = (1, symbol, material);
                        instanced
                            .entry(key)
                            .or_insert_with(|| (accessors.clone(), Vec::new()))
                            .1
                            .push((entity_index, name.clone(), m));
                        if let std::collections::btree_map::Entry::Vacant(v) =
                            instanced_slot.entry(key)
                        {
                            v.insert(scene_nodes.len());
                            scene_nodes.push(usize::MAX);
                        }
                    } else {
                        let primitives = primitives_of(
                            &mut doc,
                            &mut edge_material,
                            &accessors,
                            material,
                            model.materials.len(),
                        );
                        let mesh_idx = doc.meshes.len();
                        doc.meshes.push(Mesh {
                            primitives,
                            name: Some(name.clone()),
                        });
                        children.push(doc.nodes.len());
                        doc.nodes.push(Node {
                            name: Some(name.clone()),
                            mesh: Some(mesh_idx),
                            matrix: Some(m.map(|v| v as f32)),
                            ..Node::default()
                        });
                    }
                }
                if parts.is_empty() {
                    (None, None, children)
                } else {
                    let primitives = primitives_of(
                        &mut doc,
                        &mut edge_material,
                        &parts,
                        material,
                        model.materials.len(),
                    );
                    let mesh_idx = doc.meshes.len();
                    doc.meshes.push(Mesh {
                        primitives,
                        name: Some(name.clone()),
                    });
                    (Some(mesh_idx), Some(matrix), children)
                }
            }
            None => (None, None, Vec::new()),
        };
        // An element drawn only by instanced parts and no own mesh still has
        // its node (its identity, for the tree).
        // With parts as children (placed in the model frame), the element's
        // own mesh is a child too, so the element node stays unplaced.
        let (mesh, matrix, mut children) = (mesh, matrix, children);
        let (mesh, matrix) = if children.is_empty() {
            (mesh, matrix)
        } else {
            if let Some(mesh) = mesh {
                children.push(doc.nodes.len());
                doc.nodes.push(Node {
                    name: Some(name.clone()),
                    mesh: Some(mesh),
                    matrix,
                    ..Node::default()
                });
            }
            (None, None)
        };
        let node_idx = doc.nodes.len();
        doc.nodes.push(Node {
            name: Some(name.clone()),
            mesh,
            matrix,
            children,
            extras: Some(node_extras(&identities, entity_index, ifc_type)),
            extensions: None,
        });
        if hidden_room {
            room_nodes.push(node_idx);
        } else {
            scene_nodes.push(node_idx);
        }
    }

    // each instanced group: one node, with EXT_mesh_gpu_instancing
    // where it draws several elements.
    for (key, (parts, members)) in instanced {
        let (_, _, material) = key;
        let primitives = primitives_of(
            &mut doc,
            &mut edge_material,
            &parts,
            material,
            model.materials.len(),
        );
        let (first, first_name, first_matrix) = &members[0];
        let first_type = match &model.entities[*first] {
            IfcEntity::BuildingElement { ifc_type, .. } => ifc_type.clone(),
            _ => String::new(),
        };
        let mesh_idx = doc.meshes.len();
        doc.meshes.push(Mesh {
            primitives,
            name: Some(first_name.clone()),
        });
        let node = if members.len() == 1 {
            Node {
                name: Some(first_name.clone()),
                mesh: Some(mesh_idx),
                matrix: Some(first_matrix.map(|v| v as f32)),
                children: Vec::new(),
                extras: Some(node_extras(&identities, *first, &first_type)),
                extensions: None,
            }
        } else {
            let mut translation = Vec::with_capacity(members.len() * 3);
            let mut rotation = Vec::with_capacity(members.len() * 4);
            let mut instances = Vec::with_capacity(members.len());
            for (entity, _, m) in &members {
                translation.extend([m[12], m[13], m[14]].map(|v| v as f32));
                rotation.extend(rotation_quaternion(m).map(|v| v as f32));
                let ty = match &model.entities[*entity] {
                    IfcEntity::BuildingElement { ifc_type, .. } => ifc_type.as_str(),
                    _ => "",
                };
                instances.push(node_extras(&identities, *entity, ty));
            }
            let t = push_floats(&mut doc, &mut bin, &translation, "VEC3");
            let r = push_floats(&mut doc, &mut bin, &rotation, "VEC4");
            let mut extras = node_extras(&identities, *first, &first_type);
            extras.instances = instances;
            Node {
                name: Some(first_name.clone()),
                mesh: Some(mesh_idx),
                matrix: None,
                children: Vec::new(),
                extras: Some(extras),
                extensions: Some(serde_json::json!({
                    "EXT_mesh_gpu_instancing": {"attributes": {"TRANSLATION": t, "ROTATION": r}}
                })),
            }
        };
        let node_idx = doc.nodes.len();
        doc.nodes.push(node);
        scene_nodes[instanced_slot[&key]] = node_idx;
    }
    if doc.nodes.iter().any(|n| n.extensions.is_some()) {
        doc.extensions_used.push("EXT_mesh_gpu_instancing".into());
        doc.extensions_required
            .push("EXT_mesh_gpu_instancing".into());
    }

    // Buffer (singular — GLB embeds the whole bin chunk as buffer 0).
    doc.buffers.push(Buffer {
        byte_length: bin.len(),
        uri: None,
        extensions: None,
    });
    let root = doc.nodes.len();
    doc.nodes.push(Node {
        name: Some("model".into()),
        mesh: None,
        matrix: Some(Z_UP_FEET_TO_Y_UP_METRES),
        children: scene_nodes,
        extras: None,
        extensions: None,
    });
    doc.scenes.push(Scene {
        name: None,
        nodes: vec![root],
    });
    if !room_nodes.is_empty() {
        let rooms = doc.nodes.len();
        doc.nodes.push(Node {
            name: Some("rooms".into()),
            mesh: None,
            matrix: Some(Z_UP_FEET_TO_Y_UP_METRES),
            children: room_nodes,
            extras: None,
            extensions: None,
        });
        doc.scenes.push(Scene {
            name: Some("with rooms".into()),
            nodes: vec![root, rooms],
        });
    }
    doc.scene = Some(0);

    (doc, bin)
}

/// The sRGB colour, and the alpha, an element with no material of its own
/// is drawn in: its category's, from the viewer design system's gray, blue,
/// red, green and amber ramps, in the hues of the plan's palette
/// ([`super::sheet`]). Blue-6 is left to the viewer's selection highlight.
/// Glass-like categories are translucent, and spaces faint enough not to
/// hide the building they fill.
pub fn category_colour(ifc_type: &str) -> (u32, f32) {
    match ifc_type {
        "IFCWALL" | "IFCWALLSTANDARDCASE" | "IFCCURTAINWALL" => (0xCDCED0, 1.0),
        "IFCSLAB" | "IFCROOF" | "IFCCOVERING" => (0x909398, 1.0),
        "IFCDOOR" => (0xF59E0B, 1.0),
        "IFCWINDOW" | "IFCPLATE" => (0x8FC1FF, 0.45),
        "IFCCOLUMN" => (0xDC2626, 1.0),
        "IFCBEAM" | "IFCMEMBER" => (0xF87171, 1.0),
        "IFCSTAIR" | "IFCSTAIRFLIGHT" | "IFCRAILING" | "IFCRAMP" | "IFCRAMPFLIGHT" => {
            (0x92400E, 1.0)
        }
        "IFCFURNITURE" | "IFCFURNISHINGELEMENT" | "IFCSYSTEMFURNITUREELEMENT" => (0x16A34A, 1.0),
        "IFCSPACE" => (0x3D94FF, 0.12),
        _ => (0xB2B4B8, 1.0),
    }
}

/// The material a category's colour makes, named for the category.
fn category_material(ifc_type: &str) -> Material {
    let (rgb, alpha) = category_colour(ifc_type);
    colour_material(format!("Category {ifc_type}"), rgb, alpha)
}

/// A material of one sRGB colour (`0xRRGGBB`) and alpha. glTF base colours
/// are linear, so the colour is linearised.
fn colour_material(name: String, rgb: u32, alpha: f32) -> Material {
    let linear = |shift: u32| {
        let c = ((rgb >> shift) & 0xff) as f32 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    Material {
        name: Some(name),
        pbr_metallic_roughness: PbrMetallicRoughness {
            base_color_factor: [linear(16), linear(8), linear(0), alpha],
            metallic_factor: 0.0,
            roughness_factor: 0.8,
        },
        double_sided: alpha < 1.0,
        alpha_mode: (alpha < 1.0).then(|| "BLEND".into()),
    }
}

/// Append `mesh` to the binary buffer as f32 positions and u32 indices,
/// with a buffer view and an accessor for each. Returns the
/// `(POSITION, indices)` accessor indices, or `None` for an empty mesh or
/// one past `u32` indexing.
fn push_mesh(
    doc: &mut GltfDocument,
    bin: &mut Vec<u8>,
    mesh: &body_geometry::Mesh,
    opts: &GltfOptions,
) -> Option<Accessors> {
    // Flat shading: three own vertices per triangle, each with the face normal
    // (a viewer that does not compute normals draws a mesh without them black).
    if mesh.is_empty()
        || mesh
            .triangles
            .len()
            .checked_mul(3)
            .and_then(|n| u32::try_from(n).ok())
            .is_none()
    {
        return None;
    }
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    let mut positions: Vec<f32> = Vec::new();
    let mut normals: Vec<f32> = Vec::new();
    let mut index_list: Vec<u32> = Vec::with_capacity(mesh.triangles.len() * 3);
    // Face normals, in f64 until the end (a tiny triangle's cross product in
    // f32 normalises to a vector that is not unit). A degenerate triangle
    // gets an arbitrary unit normal (glTF requires unit length).
    let mut face_normals: Vec<[f64; 3]> = Vec::with_capacity(mesh.triangles.len());
    for tri in &mesh.triangles {
        let mut p = [[0.0_f64; 3]; 3];
        for (k, &i) in tri.iter().enumerate() {
            p[k] = *mesh.vertices.get(i as usize)?;
        }
        let (u, w) = (
            [p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]],
            [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]],
        );
        let n = [
            u[1] * w[2] - u[2] * w[1],
            u[2] * w[0] - u[0] * w[2],
            u[0] * w[1] - u[1] * w[0],
        ];
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        face_normals.push(if len.is_finite() && len > 1e-30 {
            [n[0] / len, n[1] / len, n[2] / len]
        } else {
            [0.0, 0.0, 1.0]
        });
    }
    // a vertex whose faces all lean within 25 degrees of its first
    // face is smooth and shared (one averaged normal); any other corner keeps
    // its face's own normal, so flat faces and creases stay hard. Roughly 84
    // bytes a triangle before, about 30 now.
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let mut sum = vec![[0.0_f64; 3]; mesh.vertices.len()];
    let mut first: Vec<Option<[f64; 3]>> = vec![None; mesh.vertices.len()];
    let mut smooth = vec![true; mesh.vertices.len()];
    for (tri, fnrm) in mesh.triangles.iter().zip(&face_normals) {
        for &v in tri {
            let v = v as usize;
            match first[v] {
                None => first[v] = Some(*fnrm),
                Some(f) if dot(f, *fnrm) < 0.906 => smooth[v] = false,
                _ => {}
            }
            for k in 0..3 {
                sum[v][k] += fnrm[k];
            }
        }
    }
    // a mesh of flat faces only may leave its normals to the
    // viewer (glTF 2.0: without NORMAL a client computes flat normals; three.js
    // GLTFLoader sets `flatShading`), each position then stored once.
    let omit_normals = opts.omit_flat_normals
        && mesh
            .triangles
            .iter()
            .flatten()
            .all(|&v| !smooth[v as usize]);
    if omit_normals {
        smooth.iter_mut().for_each(|s| *s = true);
    }
    let mut welded: std::collections::HashMap<(u32, [u32; 3]), u32> = Default::default();
    for (tri, fnrm) in mesh.triangles.iter().zip(&face_normals) {
        for &v in tri {
            let vi = v as usize;
            let n = if smooth[vi] {
                let s = sum[vi];
                let l = dot(s, s).sqrt();
                if l > 1e-12 {
                    [s[0] / l, s[1] / l, s[2] / l]
                } else {
                    *fnrm
                }
            } else {
                *fnrm
            };
            let n = n.map(|c| c as f32);
            // Flat corners are keyed by their normal; smooth ones by vertex.
            let key = (
                v,
                if smooth[vi] {
                    [0; 3]
                } else {
                    n.map(f32::to_bits)
                },
            );
            let at = match welded.get(&key) {
                Some(&at) => at,
                None => {
                    let q = mesh.vertices[vi].map(|c| c as f32);
                    for axis in 0..3 {
                        min[axis] = min[axis].min(q[axis]);
                        max[axis] = max[axis].max(q[axis]);
                    }
                    let at = (positions.len() / 3) as u32;
                    positions.extend_from_slice(&q);
                    normals.extend_from_slice(&n);
                    welded.insert(key, at);
                    at
                }
            };
            index_list.push(at);
        }
    }
    if !positions.iter().all(|v| v.is_finite()) {
        return None;
    }
    // 16-bit indices where the vertices allow (lossless).
    let short = positions.len() / 3 <= usize::from(u16::MAX);
    let indices: Vec<u8> = if short {
        index_list
            .iter()
            .flat_map(|i| (*i as u16).to_le_bytes())
            .collect()
    } else {
        index_list.iter().flat_map(|i| i.to_le_bytes()).collect()
    };

    let view = |doc: &mut GltfDocument, bin: &mut Vec<u8>, data: &[u8], target: u32| {
        let off = bin.len();
        bin.extend_from_slice(data);
        doc.buffer_views.push(BufferView {
            buffer: 0,
            byte_offset: off,
            byte_length: data.len(),
            target: Some(target),
            byte_stride: None,
            extensions: None,
        });
        pad_to_4(bin);
        doc.buffer_views.len() - 1
    };
    let position_view = view(doc, bin, &bytemuck_cast_f32(&positions), 34962);
    let normal_view = (!omit_normals).then(|| view(doc, bin, &bytemuck_cast_f32(&normals), 34962));
    let index_view = view(doc, bin, &indices, 34963);

    let acc = |doc: &mut GltfDocument,
               bv: usize,
               ct: u32,
               count: usize,
               ty: &str,
               mn: Vec<f32>,
               mx: Vec<f32>| {
        doc.accessors.push(Accessor {
            buffer_view: bv,
            byte_offset: 0,
            component_type: ct,
            count,
            type_: ty.into(),
            min: mn,
            max: mx,
        });
        doc.accessors.len() - 1
    };
    let vertex_count = positions.len() / 3;
    let position = acc(
        doc,
        position_view,
        5126,
        vertex_count,
        "VEC3",
        min.to_vec(),
        max.to_vec(),
    );
    let normal = normal_view.map(|normal_view| {
        acc(
            doc,
            normal_view,
            5126,
            vertex_count,
            "VEC3",
            Vec::new(),
            Vec::new(),
        )
    });
    let index = acc(
        doc,
        index_view,
        if short { 5123 } else { 5125 },
        index_list.len(),
        "SCALAR",
        Vec::new(),
        Vec::new(),
    );
    // the feature edges, indices into the same points.
    let edges = opts
        .display
        .edges
        .then(|| {
            let points: Vec<[f32; 3]> = positions
                .chunks_exact(3)
                .map(|p| [p[0], p[1], p[2]])
                .collect();
            let tris: Vec<[u32; 3]> = index_list
                .chunks_exact(3)
                .map(|t| [t[0], t[1], t[2]])
                .collect();
            push_indices(doc, bin, &feature_edges(&points, &tris), short)
        })
        .flatten();
    Some((position, index, normal, edges))
}

/// Indices as an element-array accessor, 16-bit when `short` (lossless:
/// every index then fits); `None` when empty.
fn push_indices(
    doc: &mut GltfDocument,
    bin: &mut Vec<u8>,
    list: &[u32],
    short: bool,
) -> Option<usize> {
    if list.is_empty() {
        return None;
    }
    let off = bin.len();
    if short {
        bin.extend(list.iter().flat_map(|i| (*i as u16).to_le_bytes()));
    } else {
        bin.extend(list.iter().flat_map(|i| i.to_le_bytes()));
    }
    let byte_length = bin.len() - off;
    pad_to_4(bin);
    doc.buffer_views.push(BufferView {
        buffer: 0,
        byte_offset: off,
        byte_length,
        target: Some(34963),
        byte_stride: None,
        extensions: None,
    });
    doc.accessors.push(Accessor {
        buffer_view: doc.buffer_views.len() - 1,
        byte_offset: 0,
        component_type: if short { 5123 } else { 5125 },
        count: list.len(),
        type_: "SCALAR".into(),
        min: Vec::new(),
        max: Vec::new(),
    });
    Some(doc.accessors.len() - 1)
}

/// Write a GLB (binary glTF) file to `out` given a JSON document
/// + binary buffer.
///
/// Spec reference: <https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#glb-file-format-specification>
pub fn write_glb(doc: &GltfDocument, bin: &[u8], out: &mut Vec<u8>) {
    let json_text = serde_json::to_string(doc).expect("serialize glTF doc");
    let mut json_bytes = json_text.into_bytes();
    // Pad JSON chunk to 4-byte boundary with ASCII space (per spec).
    while !json_bytes.len().is_multiple_of(4) {
        json_bytes.push(b' ');
    }
    let mut bin_padded = bin.to_vec();
    while !bin_padded.len().is_multiple_of(4) {
        bin_padded.push(0);
    }

    // Header: 12 bytes (magic + version + total length).
    let total_len = 12 + 8 + json_bytes.len() + 8 + bin_padded.len();
    out.extend_from_slice(&0x4654_6C67_u32.to_le_bytes()); // "glTF"
    out.extend_from_slice(&2_u32.to_le_bytes()); // version
    out.extend_from_slice(&(total_len as u32).to_le_bytes());

    // JSON chunk.
    out.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(&0x4E4F_534A_u32.to_le_bytes()); // "JSON"
    out.extend_from_slice(&json_bytes);

    // BIN chunk.
    out.extend_from_slice(&(bin_padded.len() as u32).to_le_bytes());
    out.extend_from_slice(&0x004E_4942_u32.to_le_bytes()); // "BIN\0"
    out.extend_from_slice(&bin_padded);
}

/// One-call convenience: `IfcModel` → GLB bytes.
///
/// # Examples
///
/// ```
/// use rvt::RevitFile;
/// use rvt::ifc::{Exporter, RvtDocExporter, gltf::model_to_glb};
///
/// let mut rf = RevitFile::open("corpus/tier1/architectural-2024/architectural-2024.rvt")?;
/// let glb = model_to_glb(&RvtDocExporter.export(&mut rf)?);
/// assert_eq!(&glb[..4], b"glTF");
/// # Ok::<(), rvt::Error>(())
/// ```
pub fn model_to_glb(model: &IfcModel) -> Vec<u8> {
    model_to_glb_with(model, &GltfOptions::default())
}

/// [`model_to_glb`] with `opts`.
pub fn model_to_glb_with(model: &IfcModel, opts: &GltfOptions) -> Vec<u8> {
    let (mut doc, mut bin) = build_gltf_with(model, opts);
    if opts.meshopt {
        bin = super::meshopt::compress_document(&mut doc, &bin);
    }
    let mut out = Vec::new();
    write_glb(&doc, &bin, &mut out);
    out
}

// ---- Internal: dep-free f32/u16 slice-to-bytes casts ----

fn bytemuck_cast_f32(src: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(src.len() * 4);
    for &v in src {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

fn bytemuck_cast_u16(src: &[u16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(src.len() * 2);
    for &v in src {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

fn pad_to_4(buf: &mut Vec<u8>) {
    while !buf.len().is_multiple_of(4) {
        buf.push(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feature_edges_skip_diagonals_and_keep_creases() {
        // A flat quad: four sides, no diagonal.
        let quad = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ];
        assert_eq!(feature_edges(&quad, &[[0, 1, 2], [0, 2, 3]]).len(), 8);
        // The same quad saved once per side: still four sides.
        assert_eq!(
            feature_edges(&quad, &[[0, 1, 2], [0, 2, 3], [2, 1, 0], [3, 2, 0]]).len(),
            8
        );
        // The unit cube (24 points, 4 per face): its twelve sides.
        let p = unit_cube_vertices();
        let points: Vec<[f32; 3]> = p.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
        let tris: Vec<[u32; 3]> = unit_cube_indices()
            .chunks_exact(3)
            .map(|t| [u32::from(t[0]), u32::from(t[1]), u32::from(t[2])])
            .collect();
        assert_eq!(feature_edges(&points, &tris).len(), 24);
    }

    #[test]
    fn rooms_leave_the_default_scene() {
        let mut model = IfcModel::default();
        model.entities.push(IfcEntity::BuildingElement {
            ifc_type: "IFCSPACE".into(),
            name: "Room".into(),
            type_guid: None,
            predefined_type: None,
            storey_index: None,
            material_index: None,
            property_set: None,
            location_feet: Some([0.0; 3]),
            rotation_radians: None,
            extrusion: Some(super::super::entities::Extrusion::rectangle(
                10.0, 10.0, 8.0,
            )),
            host_element_index: None,
            material_layer_set_index: None,
            material_profile_set_index: None,
            solid_shape: None,
            representation_map_index: None,
        });
        let (doc, _) = build_gltf(&model);
        let root = doc.scenes[0].nodes[0];
        assert!(doc.nodes[root].children.is_empty(), "no room in scene 0");
        assert_eq!(doc.scenes.len(), 2);
        let (doc, _) = build_gltf_with(
            &model,
            &DisplayOptions {
                rooms: true,
                edges: false,
            }
            .into(),
        );
        assert_eq!(doc.scenes.len(), 1);
        assert_eq!(doc.nodes[doc.scenes[0].nodes[0]].children.len(), 1);
    }
}
