//! the IFCX writer (`write_ifcx`) beside the IFC4 STEP writer.
//!
//! Real file: `ARK_NordicLCA_Housing_Terrain-Concrete_BuildingPermit.rvt`
//! (Nordic LCA / BIM4LCA, Revit 2024; VTT, Granlund, Gravicon,
//! Insinööritoimisto Kallinen; CC BY-SA 4.0,
//! <https://cris.vtt.fi/en/publications/bim-files-to-download/>), from
//! `RVT_NORDIC_LCA_DIR`. Skips when absent. The synthetic tests carry the
//! `synthetic_` prefix and are left out of the gate (`skip=` in
//! tools/ci/test-targets.txt), as the repository's directive on unit tests
//! asks; `cargo test --test integration ifcx_writer` runs them.

use rvt::RevitFile;
use rvt::ifc::entities::{Extrusion, IfcEntity, Property, PropertySet, PropertyValue};
use rvt::ifc::ifcx_writer::{IFC_IMPORT, PROP_IMPORT, USD_IMPORT, ifc_class_name, uuid5_in};
use rvt::ifc::{IfcModel, RvtDocExporter, Storey, write_ifcx, write_ifcx_with_ledger, write_step};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::PathBuf;

/// The structure every IFCX file this writer makes must have.
fn check_structure(text: &str) -> Value {
    let doc: Value = serde_json::from_str(text).expect("valid JSON");
    assert_eq!(doc["header"]["ifcxVersion"], "ifcx_alpha");
    assert_eq!(doc["imports"][0]["uri"], IFC_IMPORT);
    assert_eq!(doc["imports"][1]["uri"], PROP_IMPORT);
    assert_eq!(doc["imports"][2]["uri"], USD_IMPORT);
    let data = doc["data"].as_array().expect("data");
    let paths: BTreeSet<&str> = data.iter().map(|n| n["path"].as_str().unwrap()).collect();
    assert_eq!(paths.len(), data.len(), "paths are unique");
    let mut parented = BTreeSet::new();
    for n in data {
        for (name, c) in n["children"].as_object().into_iter().flatten() {
            let c = c.as_str().unwrap();
            assert!(paths.contains(c), "dangling child {name}: {c}");
            assert!(parented.insert(c), "{c} has two parents");
        }
    }
    // Every `inherits` target is a node of this file: a missing one composes
    // silently to an empty node (buildingSMART IFC5-development#142).
    let mut types = BTreeSet::new();
    for n in data {
        for (name, t) in n["inherits"].as_object().into_iter().flatten() {
            let t = t.as_str().unwrap();
            assert!(paths.contains(t), "{name}: {t} is no node of the file");
            types.insert(t);
        }
    }
    // A type hangs from nothing, as in buildingSMART's examples; so does a
    // type nothing places.
    for n in data {
        if n["attributes"][rvt::ifc::graph::UNPLACED_TYPE] == true {
            types.insert(n["path"].as_str().unwrap());
        }
    }
    let roots = paths
        .iter()
        .filter(|p| !parented.contains(*p) && !types.contains(*p))
        .count();
    assert_eq!(roots, 1, "one root");
    let schemas = doc["schemas"].as_object().expect("schemas");
    for n in data {
        let attrs = n["attributes"].as_object();
        for k in attrs.into_iter().flatten().map(|(k, _)| k) {
            if k.starts_with("ifc-src::") || k.starts_with("rvtopen::") {
                assert!(schemas.contains_key(k), "undeclared key {k}");
            }
            // prop@v5a owns these: declared by the import, not here, and a
            // fact is written under one key only.
            assert!(!schemas.contains_key("bsi::ifc::prop::Name"));
            assert!(k != "ifc-src::attr::Name" && k != "ifc-src::attr::Description");
            // `Name` and `Description` are the element's attributes: a standard
            // set's own property of that name (a room's `Pset_ProductRequirements`
            // `Name`) is another fact and keeps its set key.
            if let Some(prop) = k
                .strip_prefix("bsi::ifc::prop::")
                .filter(|prop| !matches!(*prop, "Name" | "Description"))
            {
                let twin = |k2: &String| {
                    k2.starts_with("ifc-src::pset::Pset_") || k2.starts_with("ifc-src::pset::Qto_")
                };
                assert!(
                    !attrs
                        .unwrap()
                        .keys()
                        .any(|k2| twin(k2) && k2.ends_with(&format!("::{prop}"))),
                    "{prop} written under two keys on {}",
                    n["path"]
                );
            }
        }
    }
    doc
}

fn nodes_of_class<'a>(doc: &'a Value, code: &str) -> Vec<&'a Value> {
    doc["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| n["attributes"]["bsi::ifc::class"]["code"] == code)
        .collect()
}

#[test]
fn nordic_housing_terrain_ifcx_has_the_shape_of_its_ifc4() {
    const NAME: &str = "ARK_NordicLCA_Housing_Terrain-Concrete_BuildingPermit.rvt";
    let Some(dir) = std::env::var_os("RVT_NORDIC_LCA_DIR").map(PathBuf::from) else {
        eprintln!("skipping: RVT_NORDIC_LCA_DIR is not set");
        return;
    };
    let rvt = dir.join(NAME);
    if !rvt.exists() {
        eprintln!("skipping: {NAME} absent");
        return;
    }
    let mut rf = RevitFile::open(&rvt).expect("open");
    let model = RvtDocExporter
        .export_with_diagnostics(&mut rf)
        .expect("export")
        .model;
    let (text, ledger) = write_ifcx_with_ledger(&model).expect("ifcx");
    assert_eq!(text, write_ifcx(&model).unwrap(), "byte-identical rerun");
    let doc = check_structure(&text);
    assert!(ledger.elements > 100, "{ledger:?}");

    // The IFC4 file names the same walls by the same GlobalIds.
    let step = write_step(&model);
    let mut step_walls = 0;
    let paths: BTreeSet<&str> = doc["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["path"].as_str().unwrap())
        .collect();
    for line in step.lines().filter(|l| l.contains("=IFCWALL(")) {
        let gid = line.split('\'').nth(1).expect("GlobalId");
        let b = rvt::research::revit_global_ids::expand_ifc_guid(gid).expect("22-character id");
        let uuid = {
            let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
            format!(
                "{}-{}-{}-{}-{}",
                &h[0..8],
                &h[8..12],
                &h[12..16],
                &h[16..20],
                &h[20..32]
            )
        };
        assert!(paths.contains(uuid.as_str()), "IFC4 wall {gid} not in IFCX");
        step_walls += 1;
    }
    assert!(step_walls > 0);
    // A type node carries its occurrences' class too.
    let walls = nodes_of_class(&doc, "IfcWall");
    let types: BTreeSet<&str> = doc["data"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|n| n["inherits"].as_object().into_iter().flatten())
        .map(|(_, t)| t.as_str().unwrap())
        // an unplaced wall type is a type node too.
        .chain(
            doc["data"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|n| n["attributes"][rvt::ifc::graph::UNPLACED_TYPE] == true)
                .map(|n| n["path"].as_str().unwrap()),
        )
        .collect();
    assert_eq!(
        walls
            .iter()
            .filter(|n| !types.contains(n["path"].as_str().unwrap()))
            .count(),
        step_walls
    );
    // A mesh child per drawn element, all Body, none empty.
    let meshes = doc["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| n["attributes"]["usd::usdgeom::mesh"].is_object())
        .count();
    assert!(meshes > 0 && meshes <= ledger.elements);
}

#[test]
fn synthetic_uuid5_matches_the_rfc_vector_and_classes_are_cased() {
    // RFC 4122 namespace DNS + "python.org" (Python's uuid.uuid5).
    let dns = [
        0x6b, 0xa7, 0xb8, 0x10, 0x9d, 0xad, 0x11, 0xd1, 0x80, 0xb4, 0x00, 0xc0, 0x4f, 0xd4, 0x30,
        0xc8,
    ];
    assert_eq!(
        uuid5_in(dns, "python.org"),
        "886313e1-3b8a-5372-9b90-0c9aee199e5d"
    );
    assert_eq!(
        ifc_class_name("IFCBUILDINGELEMENTPROXY"),
        "IfcBuildingElementProxy"
    );
    assert_eq!(ifc_class_name("IFCWALLSTANDARDCASE"), "IfcWallStandardCase");
    assert_eq!(ifc_class_name("IFCAIRTERMINAL"), "IfcAirTerminal");
    assert_eq!(ifc_class_name("IFCSPACE"), "IfcSpace");
}

fn wall(name: &str, tag: &str, props: Vec<Property>) -> IfcEntity {
    IfcEntity::BuildingElement {
        ifc_type: "IFCWALL".into(),
        name: name.into(),
        type_guid: Some(tag.into()),
        predefined_type: Some("STANDARD".into()),
        storey_index: Some(0),
        material_index: None,
        property_set: Some(PropertySet {
            name: "Other".into(),
            properties: props,
        }),
        location_feet: Some([10.0, 20.0, 0.0]),
        rotation_radians: None,
        extrusion: Some(Extrusion::rectangle(10.0, 1.0, 8.0)),
        host_element_index: None,
        material_layer_set_index: None,
        material_profile_set_index: None,
        solid_shape: None,
        representation_map_index: None,
    }
}

fn prop(name: &str, value: PropertyValue) -> Property {
    Property {
        name: name.into(),
        value,
    }
}

#[test]
fn synthetic_two_walls_one_storey() {
    let model = IfcModel {
        project_name: Some("Demo".into()),
        building_storeys: vec![Storey {
            name: "Level 1".into(),
            elevation_feet: 0.0,
        }],
        entities: vec![
            wall(
                "Wall A",
                "100",
                vec![
                    prop("Category", PropertyValue::Label("Walls".into())),
                    prop("Base Offset", PropertyValue::LengthFeet(1.0)),
                    prop("Unset", PropertyValue::Logical(None)),
                ],
            ),
            // Same name, same key, another value type: coerced, not lost.
            wall(
                "Wall A",
                "101",
                vec![prop("Base Offset", PropertyValue::Integer(2))],
            ),
        ],
        ..Default::default()
    };
    let (text, ledger) = write_ifcx_with_ledger(&model).unwrap();
    assert_eq!(text, write_ifcx(&model).unwrap(), "byte-identical rerun");
    let doc = check_structure(&text);
    // root, project, site, building, storey, 2 walls, 2 bodies placing the
    // one Shape both identical walls share
    assert_eq!(doc["data"].as_array().unwrap().len(), 10);
    let offset = &doc["schemas"]["rvtopen::revit::Other::Base Offset"]["value"];
    assert_eq!(offset["dataType"], "Real");
    assert_eq!(offset["quantityKind"], "Length");
    assert_eq!(ledger.elements, 2);
    assert_eq!(ledger.properties_unset, 1);
    assert_eq!(ledger.properties_coerced, 1);
    // Metres, placement baked in: the wall is 10 ft long about x = 10 ft.
    let xs: Vec<f64> = doc["data"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|n| n["attributes"]["usd::usdgeom::mesh"]["points"].as_array())
        .unwrap()
        .iter()
        .map(|p| p[0].as_f64().unwrap())
        .collect();
    let min = xs.iter().cloned().fold(f64::MAX, f64::min);
    assert!((min - 5.0 * 0.3048).abs() < 1e-5, "min x {min}");
}

#[test]
fn synthetic_v5a_props_are_written_once_as_flat_keys() {
    let set = |name: &str, p: Vec<Property>| IfcEntity::ElementPropertySet {
        element: 0,
        set: PropertySet {
            name: name.into(),
            properties: p,
        },
    };
    let model = IfcModel {
        project_name: Some("Demo".into()),
        building_storeys: vec![Storey {
            name: "Level 1".into(),
            elevation_feet: 0.0,
        }],
        entities: vec![
            wall("Wall A", "100", vec![]),
            set(
                "Pset_WallCommon",
                vec![
                    prop("IsExternal", PropertyValue::Boolean(true)),
                    prop("Height", PropertyValue::LengthFeet(10.0)),
                    prop("Reference", PropertyValue::Label("W1".into())),
                ],
            ),
            // Height disagrees between standard sets: both stay pset keys.
            set(
                "Qto_WallBaseQuantities",
                vec![prop("Height", PropertyValue::LengthFeet(11.0))],
            ),
        ],
        ..Default::default()
    };
    let (text, ledger) = write_ifcx_with_ledger(&model).unwrap();
    let doc = check_structure(&text);
    let a = &nodes_of_class(&doc, "IfcWall")[0]["attributes"];
    assert_eq!(a["bsi::ifc::prop::IsExternal"], true);
    assert_eq!(a["bsi::ifc::prop::Name"], "Wall A");
    assert!(
        a.get("ifc-src::pset::Pset_WallCommon::IsExternal")
            .is_none()
    );
    assert!(a.get("bsi::ifc::prop::Height").is_none());
    assert!(a.get("ifc-src::pset::Pset_WallCommon::Height").is_some());
    assert!(
        a.get("ifc-src::pset::Qto_WallBaseQuantities::Height")
            .is_some()
    );
    assert!(a.get("ifc-src::pset::Pset_WallCommon::Reference").is_some());
    assert_eq!(ledger.properties_flattened_to_v5a, 1);
}

use rvt::ifc::entities::MaterialLayerSet;
use rvt::ifc::step_writer::{StepOptions, write_step_graph_with_options, write_step_with_options};
use rvt::ifc::{GraphOptions, IfcxGraph, MaterialInfo, write_ifcx_graph};

/// The graph gives back the model it was made from, writes the same IFC4 and
/// the same IFCX, and keeps its IFC4-only keys out of the IFCX.
fn check_graph(model: &IfcModel) {
    let graph = IfcxGraph::from_model(model, GraphOptions::default());
    let back = graph.to_model();
    assert_eq!(
        serde_json::to_value(&back).unwrap(),
        serde_json::to_value(model).unwrap(),
        "graph -> model is the identity"
    );
    let at = StepOptions {
        timestamp: Some(1_700_000_000),
    };
    assert_eq!(
        write_step_with_options(model, &at),
        write_step_graph_with_options(&graph, &at),
        "IFC4 from the graph equals IFC4 from the model"
    );
    let ifcx = write_ifcx_graph(&graph).expect("ifcx");
    assert_eq!(ifcx, write_ifcx(model).unwrap());
    assert!(
        !ifcx.contains("rvtopen::ifc4::"),
        "IFC4 keys stay out of IFCX"
    );
    // Without meshes the IFC4 is unchanged.
    let bare = IfcxGraph::from_model(
        model,
        GraphOptions {
            meshes: false,
            shared_shapes: false,
        },
    );
    assert_eq!(
        write_step_with_options(model, &at),
        write_step_graph_with_options(&bare, &at)
    );
}

#[test]
fn nordic_housing_terrain_graph_derives_ifc4_and_ifcx() {
    const NAME: &str = "ARK_NordicLCA_Housing_Terrain-Concrete_BuildingPermit.rvt";
    let Some(dir) = std::env::var_os("RVT_NORDIC_LCA_DIR").map(PathBuf::from) else {
        eprintln!("skipping: RVT_NORDIC_LCA_DIR is not set");
        return;
    };
    let rvt = dir.join(NAME);
    if !rvt.exists() {
        eprintln!("skipping: {NAME} absent");
        return;
    }
    let mut rf = RevitFile::open(&rvt).expect("open");
    let model = RvtDocExporter
        .export_with_diagnostics(&mut rf)
        .expect("export")
        .model;
    check_graph(&model);
}

#[test]
fn synthetic_graph_turns_indices_into_paths_and_back() {
    let mut walls = vec![wall("Wall A", "100", vec![]), wall("Wall B", "101", vec![])];
    if let IfcEntity::BuildingElement {
        material_index,
        material_layer_set_index,
        ..
    } = &mut walls[0]
    {
        *material_index = Some(0);
        *material_layer_set_index = Some(0);
    }
    let mut entities = walls;
    entities.push(IfcEntity::Aggregate {
        whole: 0,
        parts: vec![1],
    });
    let model = IfcModel {
        project_name: Some("Demo".into()),
        building_storeys: vec![Storey {
            name: "Level 1".into(),
            elevation_feet: 3.5,
        }],
        materials: vec![MaterialInfo {
            name: "Concrete".into(),
            color_packed: Some(0x808080),
            transparency: None,
        }],
        material_layer_sets: vec![MaterialLayerSet {
            name: "Wall".into(),
            layers: vec![rvt::ifc::entities::MaterialLayer {
                material_index: Some(0),
                thickness_feet: 0.5,
                name: None,
            }],
            description: None,
        }],
        entities,
        ..Default::default()
    };
    let graph = IfcxGraph::from_model(&model, GraphOptions::default());
    let agg = &graph.nodes[&graph.order.entities[2]];
    let parts = &agg.attributes["rvtopen::ifc4::parts"];
    assert_eq!(parts[0], graph.order.entities[1].as_str());
    let a = &graph.nodes[&graph.order.entities[0]];
    assert_eq!(
        a.attributes["rvtopen::ifc4::material_index"],
        graph.order.materials[0].as_str()
    );
    assert_eq!(
        a.attributes["rvtopen::ifc4::storey_index"],
        graph.order.storeys[0].as_str()
    );
    check_graph(&model);
    // An index no table holds stays a number, and comes back.
    let mut dangling = model.clone();
    dangling.entities[2] = IfcEntity::Aggregate {
        whole: 0,
        parts: vec![1, 7],
    };
    let g = IfcxGraph::from_model(&dangling, GraphOptions::default());
    assert_eq!(
        g.nodes[&g.order.entities[2]].attributes["rvtopen::ifc4::parts"][1],
        7
    );
    assert_eq!(
        serde_json::to_value(g.to_model()).unwrap(),
        serde_json::to_value(&dangling).unwrap()
    );
}

/// the type of an occurrence. The composed value of a type
/// property on an occurrence (type values first, then the occurrence's own, as
/// buildingSMART's compose.ts does) is what the model says of that element.
#[test]
fn nordic_housing_terrain_types_compose_to_the_models_values() {
    const NAME: &str = "ARK_NordicLCA_Housing_Terrain-Concrete_BuildingPermit.rvt";
    let Some(dir) = std::env::var_os("RVT_NORDIC_LCA_DIR").map(PathBuf::from) else {
        eprintln!("skipping: RVT_NORDIC_LCA_DIR is not set");
        return;
    };
    let rvt = dir.join(NAME);
    if !rvt.exists() {
        eprintln!("skipping: {NAME} absent");
        return;
    }
    let mut rf = RevitFile::open(&rvt).expect("open");
    let model = RvtDocExporter
        .export_with_diagnostics(&mut rf)
        .expect("export")
        .model;
    let doc = check_structure(&write_ifcx(&model).expect("ifcx"));
    let data = doc["data"].as_array().unwrap();
    let node: std::collections::BTreeMap<&str, &Value> = data
        .iter()
        .map(|n| (n["path"].as_str().unwrap(), n))
        .collect();
    let compose = |n: &Value, key: &str| -> Option<Value> {
        let own = &n["attributes"][key];
        if !own.is_null() {
            return Some(own.clone());
        }
        let ty = node[n["inherits"]["TypeProduct"].as_str()?];
        Some(ty["attributes"][key].clone()).filter(|v| !v.is_null())
    };

    // What the model says, by the element's Tag (the ElementId): its Category
    // and PredefinedType.
    let mut tag_of = std::collections::BTreeMap::new();
    let mut predefined: std::collections::BTreeMap<String, BTreeSet<String>> = Default::default();
    for (i, e) in model.entities.iter().enumerate() {
        if let IfcEntity::BuildingElement {
            type_guid: Some(tag),
            predefined_type: Some(pt),
            ..
        } = e
        {
            tag_of.insert(i, tag.clone());
            predefined
                .entry(tag.clone())
                .or_default()
                .insert(pt.to_ascii_uppercase());
        }
    }
    let mut category: std::collections::BTreeMap<String, BTreeSet<String>> = Default::default();
    for e in &model.entities {
        if let IfcEntity::ElementPropertySet { element, set } = e {
            let c = set.properties.iter().find_map(|p| match &p.value {
                PropertyValue::Label(l) if set.name == "Other" && p.name == "Category" => {
                    Some(l.clone())
                }
                _ => None,
            });
            if let (Some(c), Some(tag)) = (c, tag_of.get(element)) {
                category.entry(tag.clone()).or_default().insert(c);
            }
        }
    }

    let (mut typed, mut categories, mut predefs, mut repeated) = (0, 0, 0, 0);
    // Occurrences of a type (a Body inherits its shared Shape).
    for n in data
        .iter()
        .filter(|n| n["inherits"]["TypeProduct"].is_string())
    {
        typed += 1;
        let tag = n["attributes"]["ifc-src::attr::Tag"].as_str().unwrap_or("");
        if let Some(want) = category.get(tag) {
            // The occurrence's own Category, else its type's.
            let got = compose(n, "rvtopen::revit::Other::Category").expect("composed Category");
            assert!(
                want.contains(got.as_str().unwrap()),
                "{tag}: {got} not in {want:?}"
            );
            categories += 1;
        }
        if let Some(want) = predefined.get(tag) {
            let got = compose(n, "ifc-src::attr::PredefinedType").expect("composed PredefinedType");
            assert!(
                want.contains(got.as_str().unwrap()),
                "{tag}: {got} not in {want:?}"
            );
            predefs += 1;
        }
        // A value the type holds is written on the type only.
        let ty = node[n["inherits"]["TypeProduct"].as_str().unwrap()];
        for (k, v) in ty["attributes"].as_object().unwrap() {
            if k != "bsi::ifc::class" && k != "bsi::ifc::prop::Name" && n["attributes"][k] == *v {
                repeated += 1;
            }
        }
        // The occurrence keeps its class and its mesh stays on it.
        assert!(n["attributes"]["bsi::ifc::class"].is_object());
        assert!(ty["children"].is_null(), "no geometry on a type");
    }
    assert!(
        typed > 100 && categories > 0 && predefs > 0,
        "{typed} {categories} {predefs}"
    );
    assert_eq!(repeated, 0, "values repeated on occurrences");
}
