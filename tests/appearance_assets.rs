//! Render appearance assets read as saved, on Autodesk's public 2024 samples
//! (`RVT_REVIT2024_SAMPLES_DIR`): assets per schema, and property names and
//! kinds as the Revit API documents them (the `Autodesk.Revit.DB.Visual`
//! schema classes name each property: `UnifiedBitmap.TextureRealWorldScaleX`
//! is `texture_RealWorldScaleX`, an `AssetPropertyDistance`).

use rvt::RevitFile;
use rvt::appearance::{Appearance, Asset};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn read(name: &str) -> Option<Appearance> {
    let Some(dir) = std::env::var_os("RVT_REVIT2024_SAMPLES_DIR").map(PathBuf::from) else {
        eprintln!("skipping: RVT_REVIT2024_SAMPLES_DIR is not set");
        return None;
    };
    let path = dir.join(name);
    if !path.exists() {
        eprintln!("skipping: {} is not there", path.display());
        return None;
    }
    let mut rf = RevitFile::open(&path).expect("opens");
    Some(rvt::appearance::read(&mut rf).expect("reads"))
}

fn schemas(a: &Appearance) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for asset in &a.assets {
        *out.entry(
            asset
                .asset
                .schema
                .clone()
                .unwrap_or_else(|| "(none)".into()),
        )
        .or_default() += 1;
    }
    out
}

/// Every connected asset of every appearance asset, by schema.
fn connected<'a>(a: &'a Appearance, schema: &str) -> Vec<&'a Asset> {
    a.assets
        .iter()
        .flat_map(|x| x.asset.connected())
        .filter(|c| c.schema.as_deref() == Some(schema))
        .collect()
}

/// The kind each documented property has (Revit API: `AssetPropertyString`,
/// `AssetPropertyDistance`, `AssetPropertyDouble`, `AssetPropertyBoolean`,
/// `AssetPropertyDoubleArray4d`), where the asset has the property.
fn check_kinds(asset: &Asset, want: &[(&str, &str)]) {
    for (name, kind) in want {
        if let Some(p) = asset.property(name) {
            assert_eq!(&p.kind, kind, "{name} in {:?}", asset.schema);
        }
    }
}

const BITMAP_KINDS: &[(&str, &str)] = &[
    ("unifiedbitmap_Bitmap", "String"),
    ("texture_RealWorldScaleX", "Distance"),
    ("texture_RealWorldScaleY", "Distance"),
    ("texture_RealWorldOffsetX", "Distance"),
    ("texture_RealWorldOffsetY", "Distance"),
    ("texture_WAngle", "Double1"),
    ("texture_URepeat", "Boolean"),
    ("texture_VRepeat", "Boolean"),
    ("unifiedbitmap_Invert", "Boolean"),
    ("common_Tint_color", "Double4"),
];

#[test]
fn rac_basic_assets_schemas_and_bitmaps() {
    let Some(a) = read("rac_basic_sample_project.rvt") else {
        return;
    };
    assert!(a.unread.is_empty(), "{:?}", a.unread);
    assert_eq!(a.assets.len(), 156);
    let s = schemas(&a);
    for (schema, n) in [
        ("GenericSchema", 52),
        ("MetalSchema", 26),
        ("(none)", 19),
        ("HardwoodSchema", 11),
        ("WallPaintSchema", 9),
        ("PlasticVinylSchema", 9),
        ("GlazingSchema", 8),
        ("MetallicPaintSchema", 8),
        ("SolidGlassSchema", 6),
        ("CeramicSchema", 3),
        ("ConcreteSchema", 2),
        ("MasonryCMUSchema", 2),
        ("WaterSchema", 1),
    ] {
        assert_eq!(s.get(schema), Some(&n), "{schema}: {s:?}");
    }
    assert_eq!(a.materials.len(), 174);
    let bitmaps = connected(&a, "UnifiedBitmapSchema");
    assert_eq!(bitmaps.len(), 146);
    for b in &bitmaps {
        check_kinds(b, BITMAP_KINDS);
        assert!(b.property("unifiedbitmap_Bitmap").is_some());
    }
    // procedurals are kept as their parameters: "Resin floor"'s ceramic
    // pattern is a noise (Revit API: `Noise.NoiseSize` is `noise_Size`, an
    // `AssetPropertyDouble`; `NoiseType` an `AssetPropertyInteger`;
    // `NoiseColor1` an `AssetPropertyDoubleArray4d`)
    let noise = connected(&a, "NoiseSchema");
    assert_eq!(noise.len(), 1);
    let n = noise[0];
    let value = |p: &str| n.property(p).map(|p| (p.kind.as_str(), p.value.clone()));
    assert_eq!(
        value("noise_Size"),
        Some(("Double1", serde_json::json!(50.0)))
    );
    assert_eq!(
        value("noise_Levels"),
        Some(("Double1", serde_json::json!(3.0)))
    );
    assert_eq!(value("noise_Type"), Some(("Integer", serde_json::json!(0))));
    assert_eq!(
        value("noise_Color2"),
        Some(("Double4", serde_json::json!([1.0, 1.0, 1.0, 1.0])))
    );
    // the swatches saved with the assets, PNG files
    let swatches = a.assets.iter().filter_map(|x| x.swatch.as_ref()).count();
    assert_eq!(swatches, 137);
    for s in a.assets.iter().filter_map(|x| x.swatch.as_ref()) {
        assert!(s.image.starts_with(b"\x89PNG\r\n\x1a\n"));
    }
    let json = serde_json::to_value(&a).expect("serializes");
    let first = json["assets"]
        .as_array()
        .and_then(|v| v.iter().find_map(|x| x["swatch"]["image"].as_str()));
    assert!(first.is_some_and(|b| b.starts_with("iVBORw0KGgo")));
    // a material that names an asset finds it
    let named = a
        .materials
        .iter()
        .filter(|m| a.asset_of(&m.name).is_some())
        .count();
    assert_eq!(named, 174);

    // the IFCX layer puts the record on the node of the material that names it
    // (material "SH_resin Floor", asset "Resin floor"), as JSON text
    let mut model = rvt::ifc::IfcModel {
        project_name: Some("appearance".into()),
        ..Default::default()
    };
    model.materials.push(rvt::ifc::MaterialInfo {
        name: "SH_resin Floor".into(),
        color_packed: Some(0),
        transparency: None,
        texture: None,
    });
    let graph = rvt::ifc::IfcxGraph::from_model(&model, rvt::ifc::GraphOptions::default());
    let layer = rvt::ifc::write_ifcx_graph_appearance_layer(&graph, &a, Some(0)).expect("writes");
    let layer: serde_json::Value = serde_json::from_str(&layer).expect("JSON");
    let data = layer["data"].as_array().expect("data");
    assert_eq!(data.len(), 1);
    let text = data[0]["attributes"][rvt::ifc::APPEARANCE_KEY]
        .as_str()
        .expect("text");
    let record: serde_json::Value = serde_json::from_str(text).expect("JSON");
    assert_eq!(record["name"], "Resin floor");
    assert_eq!(record["asset"]["schema"], "CeramicSchema");
    assert!(layer["schemas"][rvt::ifc::APPEARANCE_KEY].is_object());
}

#[test]
fn snowdon_architectural_reads_every_asset() {
    let Some(a) = read("Snowdon Towers Sample Architectural.rvt") else {
        return;
    };
    // the 30 assets the old 100,000-value budget refused are read too: 29
    // swatches of more than 100,000 bytes, one of 99,799 in a larger asset
    assert!(a.unread.is_empty(), "{:?}", a.unread);
    assert_eq!(a.assets.len(), 200);
    let big = a
        .assets
        .iter()
        .filter(|x| x.swatch.as_ref().is_some_and(|s| s.image.len() > 100_000))
        .count();
    assert_eq!(big, 29);
    assert_eq!(schemas(&a).get("PrismOpaqueSchema"), Some(&75));
    let bitmaps = connected(&a, "UnifiedBitmapSchema");
    assert_eq!(bitmaps.len(), 217);
    for b in bitmaps {
        check_kinds(b, BITMAP_KINDS);
    }
    assert_eq!(connected(&a, "BumpMapSchema").len(), 62);
    assert_eq!(connected(&a, "NoiseSchema").len(), 3);
}
