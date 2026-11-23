//! A material's image, carried to every format that draws one: glTF
//! (`baseColorTexture` + `TEXCOORD_0`), OBJ (`vt` + `map_Kd`), OpenUSD
//! (`UsdPreviewSurface` + `UsdUVTexture` + `primvars:st`) and IFC4
//! (`IfcBlobTexture` + `IfcIndexedTriangleTextureMap`). Synthetic data only.
//! With `RVT_TEXTURE_OUT` set, the outputs are also written there for
//! outside validators.

use rvt::ifc::entities::{BrepTriangle, IfcEntity, SolidShape};
use rvt::ifc::{IfcModel, MaterialInfo, MaterialTexture};

/// A 2 x 2 RGB PNG.
const RGB_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x08, 0x02, 0x00, 0x00, 0x00, 0xfd, 0xd4, 0x9a,
    0x73, 0x00, 0x00, 0x00, 0x11, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0xf8, 0xcf, 0x00, 0x04,
    0x20, 0xe2, 0x3f, 0x10, 0x00, 0x00, 0x1e, 0xef, 0x05, 0xfb, 0x35, 0x81, 0xa0, 0xc0, 0x00, 0x00,
    0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

/// A 1 x 1 RGBA PNG, half transparent.
const RGBA_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0x60, 0xf8, 0xcf, 0xd0,
    0x00, 0x00, 0x03, 0x82, 0x01, 0x80, 0x2d, 0x96, 0x2d, 0xea, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
    0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

fn material(name: &str, colour: u32, png: Option<&[u8]>) -> MaterialInfo {
    MaterialInfo {
        name: name.into(),
        color_packed: Some(colour),
        transparency: None,
        texture: png.and_then(|p| MaterialTexture::new(p.to_vec(), None, Some([2.0, 2.0]))),
    }
}

fn element(name: &str, material_index: Option<usize>, shape: SolidShape) -> IfcEntity {
    IfcEntity::BuildingElement {
        ifc_type: "IFCBUILDINGELEMENTPROXY".into(),
        name: name.into(),
        type_guid: None,
        predefined_type: Some("NOTDEFINED".into()),
        storey_index: None,
        material_index,
        property_set: None,
        location_feet: None,
        rotation_radians: None,
        extrusion: None,
        host_element_index: None,
        material_layer_set_index: None,
        material_profile_set_index: None,
        solid_shape: Some(shape),
        representation_map_index: None,
    }
}

/// A square of two triangles at height `z`, its texture coordinates across
/// one repeat.
fn square(z: f64) -> (Vec<[f64; 3]>, Vec<BrepTriangle>, Vec<[f64; 2]>) {
    (
        vec![[0.0, 0.0, z], [1.0, 0.0, z], [1.0, 1.0, z], [0.0, 1.0, z]],
        vec![BrepTriangle(0, 1, 2), BrepTriangle(0, 2, 3)],
        vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
    )
}

/// Element 0: a square in textured material 0 (the element's own); element 1:
/// two squares, one in each textured material (face materials); element 2: an
/// untextured square in plain material 2; element 3: a textured square and a
/// plain one; element 4: a square whose faces name no material, the element's
/// being textured and without a colour.
fn model() -> IfcModel {
    let mut m = IfcModel {
        project_name: Some("textures".into()),
        ..Default::default()
    };
    let mut glass = material("Glass", 0x00ff00, Some(RGBA_PNG));
    glass.transparency = Some(0.5);
    let mut bare = material("Bare", 0, Some(RGB_PNG));
    bare.color_packed = None;
    m.materials = vec![
        material("Checker", 0x7f7f7f, Some(RGB_PNG)),
        glass,
        material("Plain", 0x2040c0, None),
        bare,
    ];
    let (v, t, uv) = square(0.0);
    m.entities.push(element(
        "Floor",
        Some(0),
        SolidShape::TriangulatedFaceSet {
            vertices_feet: v,
            triangles: t,
            face_materials: Vec::new(),
            uvs: uv,
        },
    ));
    let (mut v, mut t, mut uv) = square(1.0);
    let (v2, t2, uv2) = square(2.0);
    v.extend(v2);
    t.extend(t2.iter().map(|x| BrepTriangle(x.0 + 4, x.1 + 4, x.2 + 4)));
    uv.extend(uv2);
    m.entities.push(element(
        "Shelf",
        None,
        SolidShape::TriangulatedFaceSet {
            vertices_feet: v,
            triangles: t,
            face_materials: vec![0, 0, 1, 1],
            uvs: uv,
        },
    ));
    let (v, t, _) = square(3.0);
    m.entities.push(element(
        "Lid",
        Some(2),
        SolidShape::TriangulatedFaceSet {
            vertices_feet: v,
            triangles: t,
            face_materials: Vec::new(),
            uvs: Vec::new(),
        },
    ));
    let (mut v, mut t, mut uv) = square(4.0);
    let (v2, t2, uv2) = square(5.0);
    v.extend(v2);
    t.extend(t2.iter().map(|x| BrepTriangle(x.0 + 4, x.1 + 4, x.2 + 4)));
    uv.extend(uv2);
    m.entities.push(element(
        "Mixed",
        None,
        SolidShape::TriangulatedFaceSet {
            vertices_feet: v,
            triangles: t,
            face_materials: vec![0, 0, 2, 2],
            uvs: uv,
        },
    ));
    let (v, t, uv) = square(6.0);
    m.entities.push(element(
        "Bare",
        Some(3),
        SolidShape::TriangulatedFaceSet {
            vertices_feet: v,
            triangles: t,
            face_materials: vec![usize::MAX; 2],
            uvs: uv,
        },
    ));
    m
}

fn save(name: &str, bytes: &[u8]) {
    if let Some(dir) = std::env::var_os("RVT_TEXTURE_OUT") {
        let dir = std::path::Path::new(&dir);
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("out dir");
        }
        std::fs::write(path, bytes).expect("write output");
    }
}

#[test]
fn texture_reads_its_image_type() {
    let t = MaterialTexture::new(RGB_PNG.to_vec(), None, None).expect("png");
    assert_eq!((t.mime_type.as_str(), t.extension()), ("image/png", "png"));
    assert!(!t.has_alpha());
    assert!(
        MaterialTexture::new(RGBA_PNG.to_vec(), None, None)
            .expect("png")
            .has_alpha()
    );
    let jpeg = MaterialTexture::new(vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 0], None, None).expect("jpeg");
    assert_eq!(jpeg.extension(), "jpg");
    assert!(MaterialTexture::new(b"BM not an image".to_vec(), None, None).is_none());
    let same = MaterialTexture::new(RGB_PNG.to_vec(), Some("other".into()), None).expect("png");
    assert_eq!(t.file_name(), same.file_name(), "named by its bytes");
    assert!(t.file_name().ends_with(".png") && t.file_name().len() == 20);
}

/// The glTF JSON chunk.
fn gltf_json(glb: &[u8]) -> serde_json::Value {
    let len = u32::from_le_bytes([glb[12], glb[13], glb[14], glb[15]]) as usize;
    serde_json::from_slice(&glb[20..20 + len]).expect("json chunk")
}

#[test]
fn glb_draws_the_images() {
    let glb = rvt::ifc::gltf::model_to_glb(&model());
    save("textures.glb", &glb);
    let doc = gltf_json(&glb);
    assert_eq!(doc["images"].as_array().map(Vec::len), Some(3));
    assert_eq!(doc["textures"].as_array().map(Vec::len), Some(3));
    assert_eq!(doc["images"][0]["mimeType"], "image/png");
    let textured: Vec<&serde_json::Value> = doc["materials"]
        .as_array()
        .expect("materials")
        .iter()
        .filter(|m| m["pbrMetallicRoughness"].get("baseColorTexture").is_some())
        .collect();
    assert_eq!(textured.len(), 3, "one textured copy per textured material");
    assert_eq!(textured[1]["alphaMode"], "BLEND", "the image holds alpha");
    let prims: Vec<&serde_json::Value> = doc["meshes"]
        .as_array()
        .expect("meshes")
        .iter()
        .flat_map(|m| m["primitives"].as_array().expect("primitives"))
        .filter(|p| p["mode"] == 4)
        .collect();
    let with_uv = prims
        .iter()
        .filter(|p| p["attributes"].get("TEXCOORD_0").is_some())
        .count();
    assert_eq!(
        with_uv, 5,
        "floor, the shelf's two parts, the mixed one's textured part, the bare one"
    );
    for p in &prims {
        let material = &doc["materials"][p["material"].as_u64().expect("material") as usize];
        let has_texture = material["pbrMetallicRoughness"]
            .get("baseColorTexture")
            .is_some();
        assert_eq!(has_texture, p["attributes"].get("TEXCOORD_0").is_some());
    }
}

#[test]
fn obj_writes_texture_coordinates_and_images() {
    let m = model();
    let (obj, mtl, files) = rvt::ifc::obj::model_to_obj_textured(&m, "textures.mtl");
    save("obj/textures.obj", obj.as_bytes());
    save("obj/textures.mtl", mtl.as_bytes());
    for (name, bytes) in &files {
        save(&format!("obj/{name}"), bytes);
    }
    assert_eq!(files.len(), 2, "one file per distinct image");
    assert_eq!(mtl.matches("map_Kd ").count(), 3);
    assert_eq!(
        mtl.matches("map_d -imfchan m ").count(),
        1,
        "the image with alpha"
    );
    let name = MaterialTexture::new(RGB_PNG.to_vec(), None, None)
        .expect("png")
        .file_name();
    assert!(mtl.contains(&format!("map_Kd {name}")));
    // each open square drawn from both sides: its points once per side; the
    // mixed element's plain square carries coordinates beside its textured one.
    assert_eq!(obj.lines().filter(|l| l.starts_with("vt ")).count(), 6 * 8);
    assert!(obj.lines().any(|l| l.starts_with("f ") && l.contains('/')));
    assert!(
        obj.lines().any(|l| l.starts_with("f ") && !l.contains('/')),
        "the lid"
    );
    // v up, as the coordinates were given: the first vertex's (0, 0).
    assert!(obj.lines().any(|l| l == "vt 0 0"));
    let (plain_obj, plain_mtl) = rvt::ifc::obj::model_to_obj(&m, "textures.mtl");
    assert!(!plain_obj.contains("\nvt ") && !plain_mtl.contains("map_Kd"));
}

#[test]
fn ifc4_embeds_and_maps_the_images() {
    let step = rvt::ifc::write_step(&model());
    save("textures.ifc", step.as_bytes());
    // the material without a colour keeps its image.
    let blobs = step.matches("IFCBLOBTEXTURE(.T.,.T.,$,$,$,'PNG',\"089504E4");
    assert_eq!(blobs.count(), 3);
    assert_eq!(step.matches("IFCSURFACESTYLEWITHTEXTURES(").count(), 3);
    // the floor; the shelf split into one face set per textured material; the
    // mixed element split into its textured and its plain square; the bare one.
    assert_eq!(step.matches("IFCINDEXEDTRIANGLETEXTUREMAP(").count(), 5);
    assert_eq!(step.matches("IFCTEXTUREVERTEXLIST(").count(), 5);
    assert_eq!(step.matches("IFCTRIANGULATEDFACESET(").count(), 7);
    // the plain square split off keeps its colour.
    assert_eq!(step.matches("IFCINDEXEDCOLOURMAP(").count(), 1);
}

#[test]
fn usd_binds_textured_materials() {
    let m = model();
    let (text, files) = rvt::ifc::write_usda_textured(&m).expect("usda");
    save("usd/textures.usda", text.as_bytes());
    for (name, bytes) in &files {
        save(&format!("usd/{name}"), bytes);
    }
    assert_eq!(files.len(), 2, "one file per distinct image");
    assert!(files.iter().all(|(f, _)| f.starts_with("textures/")));
    let count = |needle: &str| text.matches(needle).count();
    assert_eq!(count("def Material \"M"), 3);
    assert_eq!(count("\"UsdPreviewSurface\""), 3);
    assert_eq!(count("\"UsdUVTexture\""), 3);
    assert_eq!(count("texCoord2f[] primvars:st = "), 5);
    assert_eq!(count("rel material:binding = <"), 5);
    assert_eq!(count("prepend apiSchemas = [\"MaterialBindingAPI\"]"), 5);
    assert!(text.contains(&format!("asset inputs:file = @{}@", files[0].0)));
    assert!(
        text.contains("inputs:opacity.connect = <"),
        "the image with alpha"
    );
    assert!(
        text.contains("float4 inputs:scale = (1, 1, 1, 0.5)"),
        "its alpha times the material's opacity"
    );
    let (binary, binary_files) = rvt::ifc::write_usdc_textured(&m).expect("usdc");
    save("usd/textures.usdc", &binary);
    assert_eq!(binary_files, files);
    assert!(binary.starts_with(b"PXR-USDC"));
    let plain = rvt::ifc::write_usda(&m).expect("usda");
    assert!(!plain.contains("def Material") && !plain.contains("primvars:st"));
}
