//! 3D PDF export: the glTF export's meshes in a PDF 3D annotation.
//!
//! [`model_to_pdf3d`] builds the same document as `rvt-gltf --no-instancing
//! --no-edges` ([`super::gltf::build_gltf_with`]) and writes its scene as
//! U3D ([`super::u3d`]): one U3D mesh per glTF mesh (its triangle primitives,
//! one shading index each, in that primitive's material), one model node per
//! glTF node that draws a mesh, named after its element (`name [ElementId]`),
//! with the ElementId, IFC class and category as node metadata. Nodes whose
//! meshes draw the same accessors in the same materials share one U3D mesh. The world frame is metres, +Z up,
//! centred on the model's bounding box.
//!
//! The PDF ([`super::pdf_container`]) opens on an isometric view framing the
//! model, lit with the CAD lighting scheme. Its poster, drawn where 3D is not
//! shown, is the same view as flat-shaded vector triangles, back to front.

use super::IfcModel;
use super::gltf::{DisplayOptions, GltfDocument, GltfOptions, NodeExtras, build_gltf_with};
use super::pdf_container::{Page3d, View3d, num, write_pdf};
use super::u3d::{self, Scene};
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

/// A4 landscape, points.
const PAGE: [f64; 2] = [842.0, 595.0];
const MARGIN: f64 = 18.0;
const FIELD_OF_VIEW: f64 = 30.0;
/// The poster draws at most this many triangles, the largest on the page.
// ponytail: a cap, not hidden-surface removal; rasterize the poster if large
// models need every triangle in it.
const POSTER_TRIANGLES: usize = 200_000;

/// glTF (+Y up) to the PDF world (+Z up): `(x, y, z)` is `(x, -z, y)`.
const Y_UP_TO_Z_UP: [f64; 16] = [
    1.0, 0.0, 0.0, 0.0, //
    0.0, 0.0, 1.0, 0.0, //
    0.0, -1.0, 0.0, 0.0, //
    0.0, 0.0, 0.0, 1.0,
];

/// A placed mesh: its U3D mesh, world matrix, node name and metadata.
type Placed = (usize, [f64; 16], String, Vec<(String, String)>);

/// A triangle primitive's POSITION and indices accessors and material.
type PrimitiveKey = (Option<usize>, Option<usize>, Option<usize>);

/// A poster triangle: depth, projected area, page points and colour.
type PosterTriangle = (f64, f64, [[f64; 2]; 3], [f64; 3]);

fn mul(a: &[f64; 16], b: &[f64; 16]) -> [f64; 16] {
    let mut m = [0.0; 16];
    for c in 0..4 {
        for r in 0..4 {
            m[c * 4 + r] = (0..4).map(|k| a[k * 4 + r] * b[c * 4 + k]).sum();
        }
    }
    m
}

fn apply(m: &[f64; 16], p: [f32; 3]) -> [f64; 3] {
    let [x, y, z] = p.map(f64::from);
    [0, 1, 2].map(|r| m[r] * x + m[4 + r] * y + m[8 + r] * z + m[12 + r])
}

fn linear_to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

fn accessor_bytes<'a>(
    doc: &GltfDocument,
    bin: &'a [u8],
    accessor: usize,
) -> Option<(&'a [u8], usize, usize)> {
    let a = doc.accessors.get(accessor)?;
    let view = doc.buffer_views.get(a.buffer_view)?;
    let start = view.byte_offset + a.byte_offset;
    let bytes = bin.get(start..view.byte_offset + view.byte_length)?;
    Some((bytes, a.count, view.byte_stride.unwrap_or(0)))
}

fn positions(doc: &GltfDocument, bin: &[u8], accessor: usize) -> Option<Vec<[f32; 3]>> {
    let a = doc.accessors.get(accessor)?;
    if a.component_type != 5126 || a.type_ != "VEC3" {
        return None;
    }
    let (bytes, count, stride) = accessor_bytes(doc, bin, accessor)?;
    let stride = if stride == 0 { 12 } else { stride };
    (0..count)
        .map(|i| {
            let p = bytes.get(i * stride..i * stride + 12)?;
            let f = |k: usize| f32::from_le_bytes([p[k], p[k + 1], p[k + 2], p[k + 3]]);
            Some([f(0), f(4), f(8)])
        })
        .collect()
}

fn indices(doc: &GltfDocument, bin: &[u8], accessor: usize) -> Option<Vec<u32>> {
    let a = doc.accessors.get(accessor)?;
    let size = match a.component_type {
        5121 => 1,
        5123 => 2,
        5125 => 4,
        _ => return None,
    };
    let (bytes, count, stride) = accessor_bytes(doc, bin, accessor)?;
    let stride = if stride == 0 { size } else { stride };
    (0..count)
        .map(|i| {
            let b = bytes.get(i * stride..i * stride + size)?;
            Some(match size {
                1 => u32::from(b[0]),
                2 => u32::from(u16::from_le_bytes([b[0], b[1]])),
                _ => u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            })
        })
        .collect()
}

/// The triangle primitives of glTF mesh `mesh` as one U3D mesh; `default`
/// is the material of a primitive with none.
fn mesh_of(doc: &GltfDocument, bin: &[u8], mesh: usize, default: usize) -> Option<u3d::Mesh> {
    let mut out = u3d::Mesh::default();
    let mut bases: HashMap<usize, u32> = HashMap::new();
    for p in &doc.meshes.get(mesh)?.primitives {
        if p.mode != 4 {
            continue;
        }
        let Some(&position) = p.attributes.get("POSITION") else {
            continue;
        };
        let base = match bases.get(&position) {
            Some(&b) => b,
            None => {
                let Some(points) = positions(doc, bin, position) else {
                    continue;
                };
                let b = out.positions.len() as u32;
                out.positions.extend(points);
                bases.insert(position, b);
                b
            }
        };
        let count = out.positions.len() as u32 - base;
        let idx = match p.indices {
            Some(i) => match indices(doc, bin, i) {
                Some(v) => v,
                None => continue,
            },
            None => (0..count).collect(),
        };
        let shading = out.shaders.len() as u32;
        let before = out.triangles.len();
        for t in idx.chunks_exact(3) {
            if t.iter().all(|&i| i < count) {
                out.triangles.push([t[0] + base, t[1] + base, t[2] + base]);
            }
        }
        if out.triangles.len() > before {
            out.shading.resize(out.triangles.len(), shading);
            out.shaders.push(
                p.material
                    .filter(|&m| m < doc.materials.len())
                    .unwrap_or(default),
            );
        }
    }
    (!out.triangles.is_empty()).then_some(out)
}

fn node_label(name: Option<&str>, extras: Option<&NodeExtras>) -> String {
    let name = name.filter(|n| !n.is_empty());
    match extras {
        Some(e) => {
            let name = name.unwrap_or(&e.ifc_type);
            match e.element_id {
                Some(id) => format!("{name} [{id}]"),
                None => format!("{name} #{}", e.entity_index),
            }
        }
        None => name.unwrap_or("element").to_string(),
    }
}

/// The glTF document's default scene as a U3D scene in metres, +Z up,
/// centred on its bounding box; and that box's half diagonal.
pub fn scene_from_gltf(doc: &GltfDocument, bin: &[u8]) -> (Scene, f64) {
    scene_from_gltf_at(doc, bin, true)
}

/// [`scene_from_gltf`], centred on the bounding box when `centre`, else in
/// the model's own coordinates (the OBJ export).
pub fn scene_from_gltf_at(doc: &GltfDocument, bin: &[u8], centre: bool) -> (Scene, f64) {
    let mut materials: Vec<[f32; 4]> = doc
        .materials
        .iter()
        .map(|m| {
            let [r, g, b, a] = m.pbr_metallic_roughness.base_color_factor;
            [
                linear_to_srgb(r),
                linear_to_srgb(g),
                linear_to_srgb(b),
                a.clamp(0.0, 1.0),
            ]
        })
        .collect();
    let default = materials.len();
    materials.push([0.75, 0.75, 0.75, 1.0]);
    let mut scene = Scene {
        materials,
        ..Scene::default()
    };

    // U3D meshes by content: the glTF export gives every element its own
    // mesh even when the meshes reuse one type's accessors (with instancing
    // off), so meshes are shared by their triangle primitives' accessors and
    // materials, not by glTF mesh index.
    let mut meshes: HashMap<Vec<PrimitiveKey>, Option<usize>> = HashMap::new();
    let mut names: HashSet<String> = HashSet::new();
    let mut placed: Vec<Placed> = Vec::new();
    let roots = doc
        .scenes
        .get(doc.scene.unwrap_or(0))
        .map(|s| s.nodes.clone())
        .unwrap_or_default();
    // (node, parent world matrix, nearest element extras' node)
    let mut stack: Vec<(usize, [f64; 16], Option<usize>)> = roots
        .into_iter()
        .rev()
        .map(|n| (n, Y_UP_TO_Z_UP, None))
        .collect();
    let mut visited = HashSet::new();
    while let Some((n, parent, owner)) = stack.pop() {
        let Some(node) = doc.nodes.get(n) else {
            continue;
        };
        if !visited.insert(n) {
            continue;
        }
        let local = node.matrix.map_or(
            [
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ],
            |m| m.map(f64::from),
        );
        let world = mul(&parent, &local);
        let owner = if node.extras.is_some() {
            Some(n)
        } else {
            owner
        };
        if let Some(gm) = node.mesh {
            let key: Vec<PrimitiveKey> = doc
                .meshes
                .get(gm)
                .map(|m| {
                    m.primitives
                        .iter()
                        .filter(|p| p.mode == 4)
                        .map(|p| {
                            (
                                p.attributes.get("POSITION").copied(),
                                p.indices,
                                p.material.filter(|&m| m < doc.materials.len()),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            let index = *meshes.entry(key).or_insert_with(|| {
                mesh_of(doc, bin, gm, default).map(|m| {
                    scene.meshes.push(m);
                    scene.meshes.len() - 1
                })
            });
            if let Some(index) = index {
                let owner_node = owner.map(|o| &doc.nodes[o]);
                let extras = owner_node.and_then(|o| o.extras.as_ref());
                let base = node_label(
                    owner_node
                        .and_then(|o| o.name.as_deref())
                        .or(node.name.as_deref()),
                    extras,
                );
                let mut label = base.clone();
                let mut k = 2;
                while !names.insert(label.clone()) {
                    label = format!("{base} ({k})");
                    k += 1;
                }
                let mut metadata = Vec::new();
                if let Some(e) = extras {
                    if let Some(id) = e.element_id {
                        metadata.push(("ElementId".to_string(), id.to_string()));
                    }
                    metadata.push(("IfcClass".to_string(), e.ifc_type.clone()));
                    if let Some(c) = &e.category {
                        metadata.push(("Category".to_string(), c.clone()));
                    }
                }
                placed.push((index, world, label, metadata));
            }
        }
        for &c in node.children.iter().rev() {
            stack.push((c, world, owner));
        }
    }

    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for (mesh, world, _, _) in &placed {
        for &p in &scene.meshes[*mesh].positions {
            for (k, &v) in apply(world, p).iter().enumerate() {
                if v.is_finite() {
                    lo[k] = lo[k].min(v);
                    hi[k] = hi[k].max(v);
                }
            }
        }
    }
    let (centre_point, radius) = if lo[0] <= hi[0] {
        let c = [0, 1, 2].map(|k| (lo[k] + hi[k]) / 2.0);
        let d = [0, 1, 2].map(|k| hi[k] - lo[k]);
        (
            c,
            ((d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() / 2.0).max(1e-3),
        )
    } else {
        ([0.0; 3], 1.0)
    };
    let shift = if centre { centre_point } else { [0.0; 3] };
    for (mesh, mut world, name, metadata) in placed {
        for (k, c) in shift.iter().enumerate() {
            world[12 + k] -= c;
        }
        scene.nodes.push(u3d::Node {
            name,
            mesh,
            matrix: world.map(|v| v as f32),
            metadata,
        });
    }
    (scene, radius)
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn unit(a: [f64; 3]) -> [f64; 3] {
    let l = dot(a, a).sqrt();
    a.map(|x| x / l)
}

/// The camera's right, up and forward axes for the isometric view from the
/// front right, above (from `(1, -1, 1)` towards the origin).
fn camera_axes() -> ([f64; 3], [f64; 3], [f64; 3]) {
    let forward = unit([-1.0, 1.0, -1.0]);
    let right = unit(cross(forward, [0.0, 0.0, 1.0]));
    let up = cross(right, forward);
    (right, up, forward)
}

/// The default view: isometric, framing a sphere of `radius` about the
/// origin. C2W gives the camera's x right, y down and z forward.
pub fn default_view(radius: f64) -> View3d {
    let (right, up, forward) = camera_axes();
    let distance = radius / (FIELD_OF_VIEW.to_radians() / 2.0).sin();
    let position = forward.map(|f| -f * distance);
    let mut c2w = [0.0; 12];
    c2w[..3].copy_from_slice(&right);
    c2w[3..6].copy_from_slice(&up.map(|u| -u));
    c2w[6..9].copy_from_slice(&forward);
    c2w[9..].copy_from_slice(&position);
    View3d {
        name: "Default".into(),
        c2w,
        center_of_orbit: distance,
        field_of_view: FIELD_OF_VIEW,
    }
}

/// The poster: the default view's direction as an orthographic projection,
/// flat-shaded triangles painted back to front on white, `size` points big.
pub fn poster(scene: &Scene, size: [f64; 2]) -> Vec<u8> {
    let (right, up, forward) = camera_axes();
    let mut tris: Vec<PosterTriangle> = Vec::new();
    for node in &scene.nodes {
        let Some(mesh) = scene.meshes.get(node.mesh) else {
            continue;
        };
        let m = node.matrix.map(f64::from);
        let world: Vec<[f64; 3]> = mesh.positions.iter().map(|&p| apply(&m, p)).collect();
        for (f, t) in mesh.triangles.iter().enumerate() {
            let [a, b, c] = t.map(|i| world[i as usize]);
            let n = cross(sub(b, a), sub(c, a));
            let l = dot(n, n).sqrt();
            if l <= 0.0 || !l.is_finite() {
                continue;
            }
            let shade = 0.45 + 0.55 * (dot(n, forward) / l).abs();
            let material = mesh
                .shading
                .get(f)
                .and_then(|&s| mesh.shaders.get(s as usize))
                .and_then(|&m| scene.materials.get(m))
                .copied()
                .unwrap_or([0.75, 0.75, 0.75, 1.0]);
            let rgb = [0, 1, 2].map(|k| (f64::from(material[k]) * shade).clamp(0.0, 1.0));
            let p2 = [a, b, c].map(|p| [dot(p, right), dot(p, up)]);
            let area = ((p2[1][0] - p2[0][0]) * (p2[2][1] - p2[0][1])
                - (p2[2][0] - p2[0][0]) * (p2[1][1] - p2[0][1]))
                .abs();
            let depth = (dot(a, forward) + dot(b, forward) + dot(c, forward)) / 3.0;
            tris.push((depth, area, p2, rgb));
        }
    }
    if tris.len() > POSTER_TRIANGLES {
        tris.sort_by(|x, y| y.1.total_cmp(&x.1));
        tris.truncate(POSTER_TRIANGLES);
    }
    tris.sort_by(|x, y| y.0.total_cmp(&x.0));

    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for (_, _, p2, _) in &tris {
        for p in p2 {
            for (k, &v) in p.iter().enumerate() {
                lo[k] = lo[k].min(v);
                hi[k] = hi[k].max(v);
            }
        }
    }
    let pad = 12.0;
    let scale = if lo[0] < hi[0] || lo[1] < hi[1] {
        ((size[0] - 2.0 * pad) / (hi[0] - lo[0]).max(1e-9))
            .min((size[1] - 2.0 * pad) / (hi[1] - lo[1]).max(1e-9))
    } else {
        1.0
    };
    let (cx, cy) = ((lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0);
    let xy = |p: [f64; 2]| {
        (
            size[0] / 2.0 + (p[0] - cx) * scale,
            size[1] / 2.0 + (p[1] - cy) * scale,
        )
    };

    let mut out = format!("1 1 1 rg 0 0 {} {} re f\n", num(size[0]), num(size[1]));
    let mut last = None;
    for (_, _, p2, rgb) in &tris {
        let rgb = rgb.map(|c| (c * 1000.0).round() / 1000.0);
        if last != Some(rgb) {
            let _ = writeln!(out, "{} {} {} rg", num(rgb[0]), num(rgb[1]), num(rgb[2]));
            last = Some(rgb);
        }
        let [(x0, y0), (x1, y1), (x2, y2)] = p2.map(xy);
        let _ = writeln!(
            out,
            "{x0:.2} {y0:.2} m {x1:.2} {y1:.2} l {x2:.2} {y2:.2} l f"
        );
    }
    out.into_bytes()
}

/// The model as a one-page PDF with a U3D 3D annotation.
pub fn model_to_pdf3d(model: &IfcModel) -> Vec<u8> {
    let opts = GltfOptions::new(false, false, false).with_display(DisplayOptions {
        rooms: false,
        edges: false,
    });
    let (doc, bin) = build_gltf_with(model, &opts);
    let (scene, radius) = scene_from_gltf(&doc, &bin);
    let stream = u3d::write_u3d(&scene);
    let size = [PAGE[0] - 2.0 * MARGIN, PAGE[1] - 2.0 * MARGIN];
    let title = model.project_name.as_deref().unwrap_or("Model");
    write_pdf(&Page3d {
        stream: &stream,
        subtype: "U3D",
        view: default_view(radius),
        poster: &poster(&scene, size),
        page_size: PAGE,
        margin: MARGIN,
        alt: &format!("3D model: {title}"),
    })
}
