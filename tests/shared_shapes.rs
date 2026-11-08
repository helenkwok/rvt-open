//! shared shapes and per-element placements.
//!
//! Each distinct element-local mesh is written once and placed by its
//! element: IFC4 `IfcRepresentationMap` + `IfcMappedItem` for solid-shape
//! bodies two or more elements share, IFCX a `Shape` node every `Body`
//! inherits with the placement as the Body's `usd::xformop`. This checks that
//! the shared form draws every element exactly where the per-element form
//! counts.
//!
//! Real files: `ARK_NordicLCA_Housing_Terrain-Concrete_BuildingPermit.rvt`
//! and `ARK_NordicLCA_Office_Concrete_BuildingPermit.rvt` (Nordic LCA /
//! BIM4LCA, Revit 2024; VTT, Granlund, Gravicon, Insinööritoimisto Kallinen;
//! CC BY-SA 4.0), from `RVT_NORDIC_LCA_DIR`. Skips when absent.

use rvt::RevitFile;
use rvt::ifc::body_geometry::share_shapes;
use rvt::ifc::frag_writer::write_frag_graph_with;
use rvt::ifc::gltf::DisplayOptions;
use rvt::ifc::{GraphOptions, IfcxGraph, RvtDocExporter, write_ifcx_graph, write_step_graph};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Pinned counts of one model.
struct Pins {
    /// Representation maps `share_shapes` adds (= IFC4 `IfcRepresentationMap`).
    maps: usize,
    /// IFC4 `IfcMappedItem`: elements drawn through a map.
    mapped_items: usize,
    /// IFCX elements with a Body.
    bodies: usize,
    /// IFCX `Shape` nodes: distinct element-local meshes.
    shapes: usize,
}

fn nodes(text: &str) -> BTreeMap<String, Value> {
    let doc: Value = serde_json::from_str(text).unwrap();
    doc["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| (n["path"].as_str().unwrap().to_string(), n.clone()))
        .collect()
}

fn check(name: &str, pins: Pins) {
    let Some(dir) = std::env::var_os("RVT_NORDIC_LCA_DIR").map(PathBuf::from) else {
        eprintln!("skipping: RVT_NORDIC_LCA_DIR is not set");
        return;
    };
    let rvt = dir.join(name);
    if !rvt.exists() {
        eprintln!("skipping: {name} absent");
        return;
    }
    let mut rf = RevitFile::open(&rvt).expect("open");
    let old = RvtDocExporter
        .export_with_diagnostics(&mut rf)
        .expect("export")
        .model;
    let mut new = old.clone();
    let maps = share_shapes(&mut new);

    // IFC4: one map per shared shape, one mapped item per element on it.
    let ifc4 = write_step_graph(&IfcxGraph::from_model(
        &new,
        GraphOptions {
            meshes: false,
            shared_shapes: true,
        },
    ));
    let ifc4_maps = ifc4.matches("IFCREPRESENTATIONMAP(").count();
    let mapped_items = ifc4.matches("IFCMAPPEDITEM(").count();
    assert_eq!(ifc4_maps, maps, "one IfcRepresentationMap per map");

    // IFCX: the shared form draws every element where the per-element form did.
    let graph = IfcxGraph::from_model(
        &new,
        GraphOptions {
            meshes: true,
            shared_shapes: true,
        },
    );
    let shared = nodes(&write_ifcx_graph(&graph).expect("ifcx"));
    let flat = nodes(
        &write_ifcx_graph(&IfcxGraph::from_model(
            &old,
            GraphOptions {
                meshes: true,
                shared_shapes: false,
            },
        ))
        .expect("ifcx"),
    );
    let (mut bodies, mut max_dev) = (0, 0.0_f64);
    for (path, n) in &flat {
        let Some(mesh) = n["attributes"].get("usd::usdgeom::mesh") else {
            continue;
        };
        bodies += 1;
        let body = &shared[path];
        assert!(body["attributes"].get("usd::usdgeom::mesh").is_none());
        let shape = body["inherits"]["Shape"]
            .as_str()
            .expect("inherits a Shape");
        let local = &shared[shape]["attributes"]["usd::usdgeom::mesh"];
        assert_eq!(
            local["faceVertexIndices"], mesh["faceVertexIndices"],
            "{path}"
        );
        let m = &body["attributes"]["usd::xformop"]["transform"];
        let f = |r: usize, c: usize| m[r][c].as_f64().unwrap();
        let (a, b) = (
            local["points"].as_array().unwrap(),
            mesh["points"].as_array().unwrap(),
        );
        assert_eq!(a.len(), b.len(), "{path}");
        for (p, q) in a.iter().zip(b) {
            let p = |i: usize| p[i].as_f64().unwrap();
            for k in 0..3 {
                let world = p(0) * f(0, k) + p(1) * f(1, k) + p(2) * f(2, k) + f(3, k);
                max_dev = max_dev.max((world - q[k].as_f64().unwrap()).abs());
            }
        }
    }
    assert!(max_dev <= 1e-6, "{name}: max deviation {max_dev} m");
    let shapes = shared
        .values()
        .filter(|n| {
            n["attributes"].get("usd::usdgeom::mesh").is_some()
                && n["attributes"].get("bsi::ifc::class").is_none()
        })
        .count();
    assert_eq!(
        (maps, mapped_items, bodies, shapes),
        (pins.maps, pins.mapped_items, pins.bodies, pins.shapes),
        "{name}: (maps, mapped items, bodies, shapes)"
    );

    // .frag: one sample per Body (rooms drawn too), byte-identical
    // on a rerun.
    let all = DisplayOptions {
        rooms: true,
        ..Default::default()
    };
    let (bytes, stats) = write_frag_graph_with(&graph, &all).expect("frag");
    assert_eq!(stats.meshes, pins.bodies);
    assert_eq!(
        write_frag_graph_with(&graph, &all).unwrap().0,
        bytes,
        "frag rerun"
    );
}

#[test]
fn nordic_housing_terrain_draws_shared_shapes_in_place() {
    check(
        "ARK_NordicLCA_Housing_Terrain-Concrete_BuildingPermit.rvt",
        Pins {
            maps: 14,
            mapped_items: 433,
            bodies: 507,
            shapes: 69,
        },
    );
}

#[test]
fn nordic_office_concrete_draws_shared_shapes_in_place() {
    check(
        "ARK_NordicLCA_Office_Concrete_BuildingPermit.rvt",
        Pins {
            // 742 -> 740 maps, 10,848 -> 10,844 mapped items and 1,638 ->
            // 1,626 shapes: the walls that embed a curtain wall drawn from
            // their saved mesh, and floors along their shape-edited tops,
            // share no map; the bodies are the same 12,803. 1,626 -> 1,628
            // shapes: walls whose layered ends now nest at their joins, as
            // Revit's bodies do, are drawn as their own outlines.
            maps: 740,
            mapped_items: 10_844,
            bodies: 12_803,
            shapes: 1_628,
        },
    );
}
