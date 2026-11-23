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
//! repeated type is repeated. [`model_to_obj_textured`] also writes texture
//! coordinates (`vt`) and each textured material's image (`map_Kd`), the
//! image files coming back to be written beside the MTL.

use super::gltf::{DisplayOptions, GltfDocument, GltfOptions, build_gltf_with};
use super::{IfcModel, MaterialTexture};
use std::fmt::Write as _;

/// The model as OBJ text and its MTL text. The OBJ names `mtl_file` (the
/// file name the MTL is written to) in its `mtllib` line. A textured
/// material is drawn in its average colour.
pub fn model_to_obj(model: &IfcModel, mtl_file: &str) -> (String, String) {
    let (obj, mtl, _) = obj_and_mtl(model, mtl_file, false);
    (obj, mtl)
}

/// [`model_to_obj`] with textures: texture coordinates on the meshes that
/// have them, and `map_Kd` naming each textured material's image, by a name
/// its bytes give ([`MaterialTexture::file_name`]). Returns the OBJ, the MTL
/// and the images (file name, bytes) to write beside the MTL.
pub fn model_to_obj_textured(
    model: &IfcModel,
    mtl_file: &str,
) -> (String, String, Vec<(String, Vec<u8>)>) {
    obj_and_mtl(model, mtl_file, true)
}

/// The image material `m` of `doc` draws.
fn material_image(doc: &GltfDocument, bin: &[u8], m: usize) -> Option<MaterialTexture> {
    let t = doc
        .materials
        .get(m)?
        .pbr_metallic_roughness
        .base_color_texture
        .as_ref()?;
    let image = doc.images.get(doc.textures.get(t.index)?.source)?;
    let view = doc.buffer_views.get(image.buffer_view)?;
    let bytes = bin.get(view.byte_offset..view.byte_offset + view.byte_length)?;
    MaterialTexture::new(bytes.to_vec(), None, None)
}

fn obj_and_mtl(
    model: &IfcModel,
    mtl_file: &str,
    textures: bool,
) -> (String, String, Vec<(String, Vec<u8>)>) {
    let opts = GltfOptions::new(false, false, false).with_display(DisplayOptions {
        rooms: false,
        edges: false,
    });
    let (doc, bin) = build_gltf_with(model, &opts);
    let (scene, _) = super::pdf3d::scene_from_gltf_at(&doc, &bin, false);

    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut mtl = String::from("# rvt-open\n");
    for (i, [r, g, b, a]) in scene.materials.iter().enumerate() {
        match textures.then(|| material_image(&doc, &bin, i)).flatten() {
            Some(texture) => {
                let file = texture.file_name();
                // An image with transparency is also the opacity map (its alpha).
                let alpha = if texture.has_alpha() {
                    format!("map_d -imfchan m {file}\n")
                } else {
                    String::new()
                };
                let _ = writeln!(
                    mtl,
                    "newmtl m{i}\nKd 1 1 1\nd {a:.4}\nmap_Kd {file}\n{alpha}"
                );
                if !files.iter().any(|(f, _)| *f == file) {
                    files.push((file, texture.image));
                }
            }
            None => {
                let _ = writeln!(mtl, "newmtl m{i}\nKd {r:.4} {g:.4} {b:.4}\nd {a:.4}\n");
            }
        }
    }

    let mut obj = format!("# rvt-open\nmtllib {mtl_file}\n");
    let mut base: u64 = 1;
    let mut uv_base: u64 = 1;
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
        let uvs = textures && mesh.uvs.len() == mesh.positions.len() && !mesh.uvs.is_empty();
        if uvs {
            for [u, v] in &mesh.uvs {
                // glTF's v runs down the image; OBJ's runs up.
                let _ = writeln!(
                    obj,
                    "vt {} {}",
                    real(f64::from(*u)),
                    real(1.0 - f64::from(*v))
                );
            }
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
            if uvs {
                let [ta, tb, tc] = tri.map(|i| uv_base + u64::from(i));
                let _ = writeln!(obj, "f {a}/{ta} {b}/{tb} {c}/{tc}");
            } else {
                let _ = writeln!(obj, "f {a} {b} {c}");
            }
        }
        base += mesh.positions.len() as u64;
        if uvs {
            uv_base += mesh.uvs.len() as u64;
        }
    }
    (obj, mtl, files)
}

/// Up to six decimals (a micrometre), trailing zeros dropped.
fn real(v: f64) -> String {
    let s = format!("{v:.6}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}
