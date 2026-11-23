//! Render appearance assets read as saved, on Autodesk's public 2024 samples
//! (`RVT_REVIT2024_SAMPLES_DIR`): assets per schema, and property names and
//! kinds as the Revit API documents them (the `Autodesk.Revit.DB.Visual`
//! schema classes name each property: `UnifiedBitmap.TextureRealWorldScaleX`
//! is `texture_RealWorldScaleX`, an `AssetPropertyDistance`).

use rvt::RevitFile;
use rvt::appearance::{Appearance, Asset};
use rvt::research::native_parameters::ObjectGraph;
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

    // the IFCX layer puts the record on a node of its own, once, and makes it
    // the child of each material that names it (material "SH_resin Floor",
    // asset "Resin floor"); two materials of one asset share the node
    let mut model = rvt::ifc::IfcModel {
        project_name: Some("appearance".into()),
        ..Default::default()
    };
    for (name, colour) in [("SH_resin Floor", 0), ("SH_resin Floor", 1), ("no such", 0)] {
        model.materials.push(rvt::ifc::MaterialInfo {
            name: name.into(),
            color_packed: Some(colour),
            transparency: None,
            texture: None,
        });
    }
    let graph = rvt::ifc::IfcxGraph::from_model(&model, rvt::ifc::GraphOptions::default());
    let (layer, matched) =
        rvt::ifc::write_ifcx_graph_appearance_layer(&graph, &a, Some(0)).expect("writes");
    assert_eq!(matched, 2);
    let layer: serde_json::Value = serde_json::from_str(&layer).expect("JSON");
    let data = layer["data"].as_array().expect("data");
    let records: Vec<_> = data
        .iter()
        .filter_map(|n| n["attributes"][rvt::ifc::APPEARANCE_KEY].as_str())
        .collect();
    assert_eq!(records.len(), 1);
    let record: serde_json::Value = serde_json::from_str(records[0]).expect("JSON");
    assert_eq!(record["name"], "Resin floor");
    assert_eq!(record["asset"]["schema"], "CeramicSchema");
    let asset_path = data
        .iter()
        .find(|n| n["attributes"].is_object())
        .expect("asset node")["path"]
        .clone();
    let children: Vec<_> = data
        .iter()
        .filter(|n| n["children"]["appearance"] == asset_path)
        .collect();
    assert_eq!(children.len(), 2);
    assert!(layer["schemas"][rvt::ifc::APPEARANCE_KEY].is_object());
}

/// The decoder's default budget is unchanged: with the default options the
/// 30 Snowdon assets whose swatch takes about 100,000 values or more are
/// still refused; only the appearance reader raises its own budget.
#[test]
fn default_graph_budget_unchanged() {
    let Some(dir) = std::env::var_os("RVT_REVIT2024_SAMPLES_DIR").map(PathBuf::from) else {
        return;
    };
    let path = dir.join("Snowdon Towers Sample Architectural.rvt");
    if !path.exists() {
        return;
    }
    let mut rf = RevitFile::open(&path).expect("opens");
    let options = rvt::research::native_document::Options {
        selected_classes: ["AppearanceAssetElem".to_string()].into(),
        releases: rvt::research::native_document::FRAMED_RELEASES,
        ..Default::default()
    };
    let mut refused = 0;
    rvt::research::native_document::extract(&mut rf, &options, |r| {
        if r.graph.is_none() {
            refused += 1;
            assert!(
                r.diagnostic
                    .as_deref()
                    .is_some_and(|d| d.contains("budget exceeded")),
                "{:?}",
                r.diagnostic
            );
        }
        Ok(())
    })
    .expect("extracts");
    assert_eq!(refused, 30);
}

/// `rvt-ifc --appearance` and `--ifcx-appearance` write the record and the
/// layer; the `--ifcx` file is the same with and without them.
#[test]
fn rvt_ifc_writes_appearance_json_and_layer() {
    let Some(dir) = std::env::var_os("RVT_REVIT2024_SAMPLES_DIR").map(PathBuf::from) else {
        return;
    };
    let rvt = dir.join("rac_basic_sample_project.rvt");
    if !rvt.exists() {
        return;
    }
    let out = std::env::temp_dir().join(format!("appearance-cli-{}", std::process::id()));
    std::fs::create_dir_all(&out).expect("dir");
    let run = |extra: &[&std::path::Path], ifcx: &std::path::Path| {
        let mut args: Vec<std::ffi::OsString> = vec![
            rvt.clone().into(),
            "-o".into(),
            out.join("m.ifc").into(),
            "--ifcx".into(),
            ifcx.into(),
            "--ifc4-timestamp".into(),
            "2026-01-01T00:00:00Z".into(),
        ];
        args.extend(extra.iter().map(|p| p.as_os_str().to_owned()));
        let status = std::process::Command::new(env!("CARGO_BIN_EXE_rvt-ifc"))
            .args(&args)
            .output()
            .expect("runs");
        assert!(
            status.status.success(),
            "{}",
            String::from_utf8_lossy(&status.stderr)
        );
    };
    let json = out.join("m.appearance.json");
    let layer = out.join("m.appearance.ifcx");
    let with = out.join("with.ifcx");
    let without = out.join("without.ifcx");
    run(
        &[
            "--appearance".as_ref(),
            &json,
            "--ifcx-appearance".as_ref(),
            &layer,
        ],
        &with,
    );
    run(&[], &without);
    assert_eq!(
        std::fs::read(&with).expect("reads"),
        std::fs::read(&without).expect("reads")
    );
    let record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&json).expect("reads")).expect("JSON");
    assert_eq!(record["assets"].as_array().map(Vec::len), Some(156));
    assert_eq!(record["unread"].as_array().map(Vec::len), Some(0));
    let layer: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&layer).expect("reads")).expect("JSON");
    assert!(layer["data"].as_array().is_some_and(|d| {
        d.iter()
            .any(|n| n["attributes"][rvt::ifc::APPEARANCE_KEY].is_string())
    }));
    let _ = std::fs::remove_dir_all(&out);
}

/// A hand-made graph: objects (class, fields) and pointers (source object,
/// offset, target object).
fn graph(objects: &[(&str, serde_json::Value)], edges: &[(usize, usize, usize)]) -> ObjectGraph {
    serde_json::from_value(serde_json::json!({
        "consumed_bytes": 0,
        "objects": objects.iter().map(|(c, f)| serde_json::json!({
            "class_tag": 0, "class_name": c, "token": 0, "start": 0, "fields_end": 0, "fields": f,
        })).collect::<Vec<_>>(),
        "edges": edges.iter().map(|(s, o, t)| serde_json::json!({
            "source_object_index": s, "pointer_offset": o, "pointer_token": 1,
            "target_object_index": t, "target_class_tag": 0,
        })).collect::<Vec<_>>(),
    }))
    .expect("a graph")
}

fn ptr(offset: usize) -> serde_json::Value {
    serde_json::json!({"class_tag": null, "offset": offset, "pointer_token": 1})
}

fn element(asset: usize) -> (&'static str, serde_json::Value) {
    (
        "AppearanceAssetElem",
        serde_json::json!({"m_pAppearanceAsset": ptr(asset), "m_materialPathMap": []}),
    )
}

#[test]
fn cyclic_graph_ends_with_a_cycle_marker() {
    // asset -> property -> connected asset -> the same property
    let g = graph(
        &[
            element(1),
            (
                "AppearanceAsset",
                serde_json::json!({"m_Name": "c", "m_color": -1, "m_transparency": 0.0,
                    "m_Asset": {"m_aAProperties": [ptr(10)]}}),
            ),
            (
                "APropertyString",
                serde_json::json!({"m_sName": "p", "m_value": "v", "m_aChild": [], "m_aConnected": [ptr(20)]}),
            ),
            ("Asset", serde_json::json!({"m_aAProperties": [ptr(30)]})),
        ],
        &[(0, 1, 1), (1, 10, 2), (2, 20, 3), (3, 30, 2)],
    );
    let mut a = Appearance::default();
    a.add_graph(7, "AppearanceAssetElem", &g);
    assert!(a.unread.is_empty(), "{:?}", a.unread);
    let asset = &a.assets[0];
    // a colour that is not one is kept as saved
    assert_eq!(asset.color_packed, None);
    assert_eq!(asset.fields["m_color"], -1);
    assert_eq!(
        asset.element_fields["m_materialPathMap"],
        serde_json::json!([])
    );
    let inner = &asset.asset.properties[0].connected[0];
    assert!(inner.properties.is_empty());
    assert_eq!(inner.fields["unread_properties"][0]["cycle"], true);
}

#[test]
fn shared_pointers_stop_at_the_node_budget() {
    // 64 properties, each with two children that are both the next one:
    // 2^64 paths without a budget
    let mut objects = vec![
        element(1),
        (
            "AppearanceAsset",
            serde_json::json!({"m_Name": "s", "m_color": 0, "m_transparency": 0.0,
                "m_Asset": {"m_aAProperties": [ptr(10)]}}),
        ),
    ];
    let mut edges = vec![(0, 1, 1), (1, 10, 2)];
    for k in 0..64 {
        let i = 2 + k;
        objects.push((
            "APropertyList",
            serde_json::json!({"m_sName": format!("p{k}"), "m_aChild": [ptr(1), ptr(2)], "m_aConnected": []}),
        ));
        if k < 63 {
            edges.push((i, 1, i + 1));
            edges.push((i, 2, i + 1));
        }
    }
    let g = graph(&objects, &edges);
    let started = std::time::Instant::now();
    let mut a = Appearance::default();
    a.add_graph(8, "AppearanceAssetElem", &g);
    assert!(started.elapsed().as_secs() < 60);
    assert_eq!(a.assets.len(), 1);
    assert!(a.unread.iter().any(|u| u.reason.contains("node budget")));
}

#[test]
fn records_not_read_are_listed() {
    let mut a = Appearance::default();
    // no objects; first object of another class; no asset object; no m_Asset
    a.add_graph(1, "AppearanceAssetElem", &graph(&[], &[]));
    a.add_graph(
        2,
        "AppearanceAssetElem",
        &graph(&[("Other", serde_json::json!({}))], &[]),
    );
    a.add_graph(3, "AppearanceAssetElem", &graph(&[element(1)], &[]));
    a.add_graph(
        4,
        "AppearanceAssetElem",
        &graph(
            &[
                element(1),
                ("AppearanceAsset", serde_json::json!({"m_Name": "x"})),
            ],
            &[(0, 1, 1)],
        ),
    );
    a.add_graph(
        5,
        "MaterialElem",
        &graph(&[("MaterialElem", serde_json::json!({}))], &[]),
    );
    let ids: Vec<u64> = a.unread.iter().map(|u| u.element_id).collect();
    assert_eq!(ids, [1, 2, 3, 4, 5]);
    assert!(a.assets.is_empty() && a.materials.is_empty());
    // an unresolved property pointer and a non-array property list are kept
    a.add_graph(
        6,
        "AppearanceAssetElem",
        &graph(
            &[
                element(1),
                (
                    "AppearanceAsset",
                    serde_json::json!({"m_Name": "u", "m_color": 0, "m_transparency": 0.0,
                        "m_Asset": {"m_aAProperties": [ptr(99)], "m_sLibrary": "lib"}}),
                ),
                ("APropertyString", serde_json::json!({"m_sName": "q"})),
            ],
            &[(0, 1, 1)],
        ),
    );
    let asset = &a.assets[0].asset;
    assert_eq!(asset.fields["unresolved_properties"][0]["offset"], 99);
    assert_eq!(asset.fields["m_sLibrary"], "lib");
}

#[test]
fn materials_match_assets_by_name_only_when_unambiguous() {
    let material = |name: &str, asset: i64| {
        graph(
            &[
                ("MaterialElem", serde_json::json!({"m_pMaterial": ptr(1)})),
                (
                    "Material",
                    serde_json::json!({"m_name": name, "m_appearanceAssetId": {"m_id": {"m_id64": asset}},
                        "m_bUseRenderAppearance": false}),
                ),
            ],
            &[(0, 1, 1)],
        )
    };
    let asset = |id: i64| {
        graph(
            &[
                element(1),
                (
                    "AppearanceAsset",
                    serde_json::json!({"m_Name": format!("a{id}"), "m_color": 0, "m_transparency": 0.0,
                        "m_Asset": {"m_aAProperties": []}}),
                ),
            ],
            &[(0, 1, 1)],
        )
    };
    let mut a = Appearance::default();
    a.add_graph(100, "AppearanceAssetElem", &asset(100));
    a.add_graph(101, "AppearanceAssetElem", &asset(101));
    for (id, name, target) in [
        (1, "same", 100),
        (2, "same", 100),
        (3, "split", 100),
        (4, "split", 101),
        (5, "", 101),
    ] {
        a.add_graph(id, "MaterialElem", &material(name, target));
    }
    assert_eq!(a.asset_of("same").map(|x| x.element_id), Some(100));
    assert!(a.asset_of("split").is_none());
    assert_eq!(a.asset_of("").map(|x| x.element_id), Some(101));
    assert!(a.asset_of("absent").is_none());
    assert_eq!(a.asset(101).map(|x| x.name.as_deref()), Some(Some("a101")));
}
