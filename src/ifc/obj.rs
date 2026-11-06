//! Wavefront OBJ export: the glTF export's meshes as `.obj` text with a
//! `.mtl` material library.
//!
//! [`model_to_obj`] draws the same meshes as `rvt-gltf --no-instancing
//! --no-edges` (the 3D PDF export's scene, [`super::pdf3d::scene_from_gltf_at`]),
//! in the model's own coordinates: metres, +Y up, as glTF. Each element is
//! one object and group named after it with its Revit ElementId in brackets
//! (`Basic_Wall_[12345]`, spaces as underscores), its faces in their
//! materials (`usemtl m<n>`), each material's colour and opacity in the
//! `.mtl` (`Kd`, `d`). Vertices are written once per placed element, so a
//! repeated type is repeated.

use super::IfcModel;
use super::gltf::{DisplayOptions, GltfOptions, build_gltf_with};
use std::fmt::Write as _;

/// The model as OBJ text and its MTL text. The OBJ names `mtl_file` (the
/// file name the MTL is written to) in its `mtllib` line.
pub fn model_to_obj(model: &IfcModel, mtl_file: &str) -> (String, String) {
    let opts = GltfOptions::new(false, false, false).with_display(DisplayOptions {
        rooms: false,
        edges: false,
    });
    let (doc, bin) = build_gltf_with(model, &opts);
    let (scene, _) = super::pdf3d::scene_from_gltf_at(&doc, &bin, false);

    let mut mtl = String::from("# rvt-open\n");
    for (i, [r, g, b, a]) in scene.materials.iter().enumerate() {
        let _ = writeln!(mtl, "newmtl m{i}\nKd {r:.4} {g:.4} {b:.4}\nd {a:.4}\n");
    }

    let mut obj = format!("# rvt-open\nmtllib {mtl_file}\n");
    let mut base: u64 = 1;
    for node in &scene.nodes {
        let Some(mesh) = scene.meshes.get(node.mesh) else {
            continue;
        };
        let label: String = node
            .name
            .chars()
            .map(|c| if c.is_whitespace() { '_' } else { c })
            .collect();
        let _ = writeln!(obj, "o {label}\ng {label}");
        let m = node.matrix.map(f64::from);
        for p in &mesh.positions {
            let [x, y, z] = p.map(f64::from);
            // The scene is +Z up; OBJ, like glTF, is +Y up.
            let w = [
                m[0] * x + m[4] * y + m[8] * z + m[12],
                m[1] * x + m[5] * y + m[9] * z + m[13],
                m[2] * x + m[6] * y + m[10] * z + m[14],
            ];
            let _ = writeln!(obj, "v {} {} {}", real(w[0]), real(w[2]), real(-w[1]));
        }
        let mut current: Option<usize> = None;
        for (t, tri) in mesh.triangles.iter().enumerate() {
            let shader = mesh.shading.get(t).copied().unwrap_or(0) as usize;
            let material = mesh.shaders.get(shader).copied().unwrap_or(0);
            if current != Some(material) {
                let _ = writeln!(obj, "usemtl m{material}");
                current = Some(material);
            }
            let [a, b, c] = tri.map(|i| base + u64::from(i));
            let _ = writeln!(obj, "f {a} {b} {c}");
        }
        base += mesh.positions.len() as u64;
    }
    (obj, mtl)
}

/// Up to six decimals (a micrometre), trailing zeros dropped.
fn real(v: f64) -> String {
    let s = format!("{v:.6}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}
