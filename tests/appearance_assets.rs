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
    assert_eq!(inner.unread[0].list, "m_aAProperties");
    assert_eq!(inner.unread[0].index, 0);
    assert_eq!(inner.unread[0].entry["cycle"], true);
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
    assert_eq!(asset.unread[0].index, 0);
    assert_eq!(asset.unread[0].entry["offset"], 99);
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

#[test]
fn a_deep_chain_stops_at_the_depth_limit() {
    // 100,000 properties, each the only child of the one before
    let n = 100_000;
    let mut objects = vec![
        element(1),
        (
            "AppearanceAsset",
            serde_json::json!({"m_Name": "d", "m_color": 0, "m_transparency": 0.0,
                "m_Asset": {"m_aAProperties": [ptr(10)]}}),
        ),
    ];
    let mut edges = vec![(0, 1, 1), (1, 10, 2)];
    for k in 0..n {
        objects.push((
            "APropertyList",
            serde_json::json!({"m_sName": "p", "m_aChild": [ptr(1)], "m_aConnected": []}),
        ));
        if k + 1 < n {
            edges.push((2 + k, 1, 3 + k));
        }
    }
    let g = graph(&objects, &edges);
    let started = std::time::Instant::now();
    let mut a = Appearance::default();
    a.add_graph(9, "AppearanceAssetElem", &g);
    assert!(started.elapsed().as_secs() < 60);
    assert_eq!(a.assets.len(), 1);
    assert!(a.unread.iter().any(|u| u.reason.contains("depth limit")));
    let json = serde_json::to_string(&a).expect("serializes");
    assert!(json.contains("\"depth_limit\":true"));
}

#[test]
fn fields_a_typed_read_did_not_take_are_kept() {
    let mut a = Appearance::default();
    a.add_graph(
        10,
        "AppearanceAssetElem",
        &graph(
            &[
                element(1),
                (
                    "AppearanceAsset",
                    serde_json::json!({"m_Name": 5, "m_color": 7, "m_transparency": "x",
                        "m_Asset": {"m_aAProperties": []}}),
                ),
            ],
            &[(0, 1, 1)],
        ),
    );
    let asset = &a.assets[0];
    assert_eq!((asset.name.as_deref(), asset.transparency), (None, None));
    assert_eq!(asset.color_packed, Some(7));
    assert_eq!(asset.fields["m_Name"], 5);
    assert_eq!(asset.fields["m_transparency"], "x");
    assert!(!asset.fields.contains_key("m_color"));
    a.add_graph(
        11,
        "MaterialElem",
        &graph(
            &[
                ("MaterialElem", serde_json::json!({"m_pMaterial": ptr(1)})),
                ("Material", serde_json::json!({"m_name": 3, "m_asset": 4})),
            ],
            &[(0, 1, 1)],
        ),
    );
    let m = &a.materials[0];
    assert!(m.own_asset.is_none());
    assert_eq!(m.fields["m_name"], 3);
    assert_eq!(m.fields["m_asset"], 4);
}

/// When the assets cannot be read (the synthetic tier-1 model has no native
/// records), `rvt-ifc --appearance` still writes the IFC file, warns, writes
/// no JSON and exits 1.
#[test]
fn rvt_ifc_fails_when_the_requested_appearance_cannot_be_read() {
    let input = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("corpus/tier1/architectural-2024/architectural-2024.rvt");
    if !input.exists() {
        return;
    }
    let out = std::env::temp_dir().join(format!("appearance-fail-{}", std::process::id()));
    std::fs::create_dir_all(&out).expect("dir");
    let (ifc, json) = (out.join("m.ifc"), out.join("m.appearance.json"));
    let run = std::process::Command::new(env!("CARGO_BIN_EXE_rvt-ifc"))
        .arg(&input)
        .arg("-o")
        .arg(&ifc)
        .arg("--appearance")
        .arg(&json)
        .output()
        .expect("runs");
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert_eq!(run.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("appearance assets not written"), "{stderr}");
    assert!(ifc.exists() && !json.exists(), "{stderr}");
    let _ = std::fs::remove_dir_all(&out);
}

/// An entry of a property, child or connected list that is not read keeps
/// its saved position, outside the saved fields: `[A, ?, B]` and `[?, A, B]`
/// give different records, and a saved field whose name a diagnostic once
/// used is kept as saved.
#[test]
fn unread_list_entries_keep_their_positions() {
    let asset_with = |list: serde_json::Value| {
        let g = graph(
            &[
                element(1),
                (
                    "AppearanceAsset",
                    serde_json::json!({"m_Name": "p", "m_color": 0, "m_transparency": 0.0,
                        "m_Asset": {"m_aAProperties": list, "unresolved_properties": "saved"}}),
                ),
                (
                    "APropertyString",
                    serde_json::json!({"m_sName": "A", "m_value": "a",
                        "m_aChild": [ptr(98), ptr(3)], "m_aConnected": [ptr(97)]}),
                ),
                (
                    "APropertyString",
                    serde_json::json!({"m_sName": "B", "m_value": "b"}),
                ),
            ],
            &[(0, 1, 1), (1, 10, 2), (1, 11, 3), (2, 3, 3)],
        );
        let mut a = Appearance::default();
        a.add_graph(9, "AppearanceAssetElem", &g);
        assert!(a.unread.is_empty(), "{:?}", a.unread);
        a.assets.remove(0).asset
    };
    let first = asset_with(serde_json::json!([ptr(10), ptr(99), ptr(11)]));
    let second = asset_with(serde_json::json!([ptr(99), ptr(10), ptr(11)]));
    for asset in [&first, &second] {
        let names: Vec<&str> = asset.properties.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["A", "B"]);
        assert_eq!(asset.unread.len(), 1);
        assert_eq!(asset.unread[0].list, "m_aAProperties");
        assert_eq!(asset.unread[0].entry["offset"], 99);
        assert_eq!(asset.fields["unresolved_properties"], "saved");
    }
    assert_eq!((first.unread[0].index, second.unread[0].index), (1, 0));
    // the child list [?, B] and the connected list [?]
    let a = &first.properties[0];
    assert_eq!(a.children.len(), 1);
    assert_eq!(a.children[0].name, "B");
    let unread: Vec<(&str, usize, u64)> = a
        .unread
        .iter()
        .map(|u| {
            (
                u.list.as_str(),
                u.index,
                u.entry["offset"].as_u64().unwrap_or(0),
            )
        })
        .collect();
    assert_eq!(unread, [("m_aChild", 0, 98), ("m_aConnected", 0, 97)]);
    assert!(a.fields.is_empty(), "{:?}", a.fields);
    let json = serde_json::to_value(&first).expect("JSON");
    assert_eq!(json["unread"][0]["index"], 1);
}

/// Every list entry counts against the node budget, one that points at no
/// object too: 2,000 references to one property of 2,000 unresolved children
/// stop at the budget (one marker for the rest of the list) instead of
/// recording 4,000,000 entries.
#[test]
fn unresolved_entries_count_against_the_budget() {
    let n = 2000;
    let children: Vec<serde_json::Value> = (0..n).map(|k| ptr(1000 + k)).collect();
    let refs: Vec<serde_json::Value> = (0..n).map(|_| ptr(10)).collect();
    let g = graph(
        &[
            element(1),
            (
                "AppearanceAsset",
                serde_json::json!({"m_Name": "b", "m_color": 0, "m_transparency": 0.0,
                    "m_Asset": {"m_aAProperties": refs}}),
            ),
            (
                "APropertyList",
                serde_json::json!({"m_sName": "shared", "m_aChild": children, "m_aConnected": []}),
            ),
        ],
        &[(0, 1, 1), (1, 10, 2)],
    );
    let mut a = Appearance::default();
    a.add_graph(12, "AppearanceAssetElem", &g);
    assert!(a.unread.iter().any(|u| u.reason.contains("node budget")));
    let asset = &a.assets[0].asset;
    let recorded: usize = asset.unread.len().saturating_add(
        asset
            .properties
            .iter()
            .map(|p| p.unread.len())
            .sum::<usize>(),
    );
    assert!(recorded <= 1_000_001, "{recorded}");
    let last = asset.unread.last().expect("a marker");
    assert_eq!(last.entry["budget_exceeded"], true);
    assert_eq!(
        last.entry["entries_not_read"].as_u64(),
        Some((n - last.index) as u64)
    );
    // properties read under the budget keep their children's saved pointers
    assert_eq!(asset.properties[0].unread.len(), n);
    assert_eq!(asset.properties[0].unread[5].entry["offset"], 1005);
}

/// A swatch width or height that is not a whole number is kept as saved,
/// with the image.
#[test]
fn swatch_dimensions_not_read_are_kept() {
    let g = graph(
        &[
            (
                "AppearanceAssetElem",
                serde_json::json!({"m_pAppearanceAsset": ptr(1), "m_pThumbnail": ptr(2)}),
            ),
            (
                "AppearanceAsset",
                serde_json::json!({"m_Name": "s", "m_color": 0, "m_transparency": 0.0,
                    "m_Asset": {"m_aAProperties": []}}),
            ),
            (
                "Thumbnail",
                serde_json::json!({"m_compressedImage": [137, 80], "m_width": -1, "m_height": 4}),
            ),
        ],
        &[(0, 1, 1), (0, 2, 2)],
    );
    let mut a = Appearance::default();
    a.add_graph(13, "AppearanceAssetElem", &g);
    let swatch = a.assets[0].swatch.as_ref().expect("swatch");
    assert_eq!((swatch.width, swatch.height), (None, Some(4)));
    assert_eq!(swatch.image, [137, 80]);
    assert_eq!(swatch.fields["m_width"], -1);
    assert!(!swatch.fields.contains_key("m_height"));
    assert!(!a.assets[0].element_fields.contains_key("m_pThumbnail"));
}

/// `--appearance` or `--ifcx-appearance` naming the input file is refused
/// before anything is read, and the input is left as it was.
#[test]
fn rvt_ifc_refuses_an_appearance_path_that_is_the_input() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("corpus/tier1/architectural-2024/architectural-2024.rvt");
    if !source.exists() {
        return;
    }
    let out = std::env::temp_dir().join(format!("appearance-input-{}", std::process::id()));
    std::fs::create_dir_all(&out).expect("dir");
    let input = out.join("m.rvt");
    std::fs::copy(&source, &input).expect("copies");
    let before = std::fs::read(&input).expect("reads");
    for flags in [
        vec!["--appearance".to_string()],
        vec![
            "--ifcx".to_string(),
            out.join("m.ifcx").display().to_string(),
            "--ifcx-appearance".to_string(),
        ],
    ] {
        let flag = flags.last().cloned().unwrap_or_default();
        let run = std::process::Command::new(env!("CARGO_BIN_EXE_rvt-ifc"))
            .arg(&input)
            .arg("-o")
            .arg(out.join("m.ifc"))
            .args(&flags)
            .arg(&input)
            .output()
            .expect("runs");
        let stderr = String::from_utf8_lossy(&run.stderr);
        assert!(!run.status.success(), "{stderr}");
        assert!(
            stderr.contains("refusing to overwrite the input"),
            "{stderr}"
        );
        assert!(stderr.contains(&flag), "{stderr}");
        assert!(!out.join("m.ifc").exists(), "{stderr}");
        assert_eq!(std::fs::read(&input).expect("reads"), before);
    }
    let _ = std::fs::remove_dir_all(&out);
}

/// The IFCX appearance layer writes each material it counts as a node of
/// its own (IfcMaterial, its name, its `appearance`), and links it from the
/// elements of the main file associated with it, directly or through a
/// layer set; a material with no asset is not written or counted.
#[test]
fn ifcx_appearance_layer_writes_the_materials_it_links() {
    use rvt::ifc::entities::{IfcEntity, MaterialLayer, MaterialLayerSet};
    use rvt::ifc::{IfcModel, MaterialInfo};
    let mut a = Appearance::default();
    a.add_graph(
        100,
        "AppearanceAssetElem",
        &graph(
            &[
                element(1),
                (
                    "AppearanceAsset",
                    serde_json::json!({"m_Name": "Oak", "m_color": 0, "m_transparency": 0.0,
                        "m_Asset": {"m_aAProperties": []}}),
                ),
            ],
            &[(0, 1, 1)],
        ),
    );
    for (id, name) in [(1, "wood"), (2, "paint")] {
        a.add_graph(
            id,
            "MaterialElem",
            &graph(
                &[
                    ("MaterialElem", serde_json::json!({"m_pMaterial": ptr(1)})),
                    (
                        "Material",
                        serde_json::json!({"m_name": name, "m_appearanceAssetId": {"m_id": {"m_id64": 100}}}),
                    ),
                ],
                &[(0, 1, 1)],
            ),
        );
    }
    let wall =
        |name: &str, material: Option<usize>, layers: Option<usize>| IfcEntity::BuildingElement {
            ifc_type: "IfcWall".into(),
            name: name.into(),
            type_guid: None,
            predefined_type: None,
            storey_index: Some(0),
            material_index: material,
            property_set: None,
            location_feet: Some([0.0, 0.0, 0.0]),
            rotation_radians: None,
            extrusion: None,
            host_element_index: None,
            material_layer_set_index: layers,
            material_profile_set_index: None,
            solid_shape: None,
            representation_map_index: None,
        };
    let mut model = IfcModel {
        project_name: Some("appearance".into()),
        ..Default::default()
    };
    model.building_storeys.push(rvt::ifc::Storey {
        name: "L1".into(),
        elevation_feet: 0.0,
    });
    for name in ["wood", "paint", "no asset"] {
        model.materials.push(MaterialInfo {
            name: name.into(),
            color_packed: Some(0),
            transparency: None,
            texture: None,
        });
    }
    model.material_layer_sets.push(MaterialLayerSet {
        name: "set".into(),
        layers: [1, 2, 0]
            .iter()
            .map(|m| MaterialLayer {
                material_index: Some(*m),
                thickness_feet: 0.1,
                name: None,
            })
            .collect(),
        description: None,
    });
    model.entities.push(wall("w0", Some(0), None));
    model.entities.push(wall("w1", None, Some(0)));
    model.entities.push(wall("w2", Some(2), None));
    let graph = rvt::ifc::IfcxGraph::from_model(&model, rvt::ifc::GraphOptions::default());
    let (layer, written) =
        rvt::ifc::write_ifcx_graph_appearance_layer(&graph, &a, Some(0)).expect("writes");
    let layer: serde_json::Value = serde_json::from_str(&layer).expect("JSON");
    let data = layer["data"].as_array().expect("data");
    let materials: Vec<&serde_json::Value> = data
        .iter()
        .filter(|n| n["attributes"]["bsi::ifc::class"]["code"] == "IfcMaterial")
        .collect();
    assert_eq!(written, 2);
    assert_eq!(materials.len(), written);
    let asset = data
        .iter()
        .find(|n| n["attributes"][rvt::ifc::APPEARANCE_KEY].is_string())
        .expect("asset node");
    let path_of = |name: &str| {
        let m = materials
            .iter()
            .find(|m| m["attributes"]["bsi::ifc::prop::Name"] == name)
            .expect("material node");
        assert_eq!(m["children"]["appearance"], asset["path"]);
        m["path"].clone()
    };
    let (wood, paint) = (path_of("wood"), path_of("paint"));
    // every path a link names is a node of the main file or of the layer
    let main: serde_json::Value = serde_json::from_str(
        &rvt::ifc::write_ifcx_graph_layers_at(&graph, Some(0))
            .expect("main")
            .0,
    )
    .expect("JSON");
    let mut paths: std::collections::BTreeSet<String> = main["data"]
        .as_array()
        .into_iter()
        .chain(Some(data))
        .flatten()
        .filter_map(|n| n["path"].as_str().map(str::to_string))
        .collect();
    paths.insert(String::new());
    let links: Vec<&serde_json::Value> =
        data.iter().filter(|n| n["attributes"].is_null()).collect();
    assert_eq!(links.len(), 2);
    let mut kids: Vec<Vec<serde_json::Value>> = Vec::new();
    for l in &links {
        assert!(paths.contains(l["path"].as_str().unwrap_or("")));
        let c = l["children"].as_object().expect("children");
        kids.push(
            ["Material", "Material1", "Material2"]
                .iter()
                .filter_map(|k| c.get(*k).cloned())
                .collect(),
        );
    }
    kids.sort_by_key(Vec::len);
    // w0: its own material; w1: its layers that have an asset, in order
    assert_eq!(kids, [vec![wood.clone()], vec![paint, wood]]);
}
