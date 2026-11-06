//! exports that do not move when the model does not.
//!
//! - IFCX: the exporter diagnostics (`RvtElementRecordGeometry::SourceStream`,
//!   where in the RVT a record was found) are a separate layer
//!   (`--ifcx-diagnostics`), not part of the main file; main + diagnostics
//!   compose to the graph's full tree.
//! - IFC4: `SOURCE_DATE_EPOCH` or `--ifc4-timestamp` fixes the `FILE_NAME`
//!   time stamp and `IfcOwnerHistory.CreationDate`, so two exports of the same
//!   file are byte-identical.
//!
//! Real file: `ARK_NordicLCA_Office_Terrain_Concrete_BuildingPermit.rvt`
//! (Nordic LCA / BIM4LCA, Revit 2024, CC BY-SA 4.0), from
//! `RVT_NORDIC_LCA_DIR`. Skips when absent.

use rvt::RevitFile;
use rvt::ifc::{GraphOptions, IfcxGraph, write_ifcx_graph};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

const SOURCE_STREAM: &str = "rvtopen::revit::RvtElementRecordGeometry::SourceStream";

fn sample() -> Option<PathBuf> {
    let p = PathBuf::from(std::env::var_os("RVT_NORDIC_LCA_DIR")?)
        .join("ARK_NordicLCA_Office_Terrain_Concrete_BuildingPermit.rvt");
    p.exists().then_some(p)
}

/// `rvt-ifc file -o dir/<tag>.ifc` plus `args`, with `env`; the IFC4 text.
fn export(file: &Path, dir: &Path, tag: &str, args: &[&str], env: &[(&str, &str)]) -> String {
    let out = dir.join(format!("{tag}.ifc"));
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rvt-ifc"));
    cmd.arg(file).arg("-o").arg(&out).args(args);
    cmd.env_remove("SOURCE_DATE_EPOCH");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let r = cmd.output().expect("spawn rvt-ifc");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    std::fs::read_to_string(out).unwrap()
}

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rvt-s324-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn ifc4_time_stamp_is_fixed_by_source_date_epoch_or_flag() {
    let Some(path) = sample() else {
        eprintln!("skip: RVT_NORDIC_LCA_DIR not set");
        return;
    };
    let dir = tmp("ts");
    // 1793232000 = 2026-10-29T00:00:00Z.
    let env = [("SOURCE_DATE_EPOCH", "1793232000")];
    let a = export(&path, &dir, "a", &[], &env);
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let b = export(&path, &dir, "b", &[], &env);
    assert!(a == b, "two exports under SOURCE_DATE_EPOCH differ");
    assert!(
        a.contains("FILE_NAME('") && a.contains("','2026-10-29T00:00:00',"),
        "FILE_NAME"
    );
    assert!(
        a.contains(",1793232000);\n"),
        "IfcOwnerHistory.CreationDate"
    );
    // The flag gives the same file, and wins over the variable.
    let f = export(
        &path,
        &dir,
        "f",
        &["--ifc4-timestamp", "2026-10-29T09:30:00+09:30"],
        &[("SOURCE_DATE_EPOCH", "0")],
    );
    assert!(
        a == f,
        "--ifc4-timestamp with an offset differs from SOURCE_DATE_EPOCH"
    );
    // A malformed variable or flag is an error, not the current time.
    for (args, env) in [
        (vec![], vec![("SOURCE_DATE_EPOCH", "yesterday")]),
        (vec![], vec![("SOURCE_DATE_EPOCH", "-5")]),
        (vec!["--ifc4-timestamp", "2026-02-30T00:00:00Z"], vec![]),
        (vec!["--ifc4-timestamp", "2026-10-29T00:00:00"], vec![]),
    ] {
        let r = Command::new(env!("CARGO_BIN_EXE_rvt-ifc"))
            .arg(&path)
            .arg("-o")
            .arg(dir.join("bad.ifc"))
            .args(&args)
            .env_remove("SOURCE_DATE_EPOCH")
            .envs(env.iter().copied())
            .output()
            .unwrap();
        assert!(!r.status.success(), "{args:?} {env:?} accepted");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ifcx_diagnostics_are_a_layer_of_their_own() {
    let Some(path) = sample() else {
        eprintln!("skip: RVT_NORDIC_LCA_DIR not set");
        return;
    };
    let dir = tmp("dx");
    let (main_p, diag_p) = (dir.join("m.ifcx"), dir.join("m.diag.ifcx"));
    export(
        &path,
        &dir,
        "m",
        &[
            "--ifcx",
            main_p.to_str().unwrap(),
            "--ifcx-diagnostics",
            diag_p.to_str().unwrap(),
        ],
        &[],
    );
    let read = |p: &Path| -> Value { serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap() };
    let (main, diag) = (read(&main_p), read(&diag_p));
    let _ = std::fs::remove_dir_all(&dir);
    let text = main.to_string();
    assert!(
        !text.contains(SOURCE_STREAM),
        "main IFCX still has SourceStream"
    );
    assert!(
        text.contains("rvtopen::revit::RvtElementRecordGeometry::FamilyName"),
        "the rest of the record set stays in the main file"
    );
    assert_eq!(
        diag["header"]["id"],
        format!("{}/diagnostics", main["header"]["id"].as_str().unwrap())
    );
    let diag_nodes = diag["data"].as_array().unwrap();
    assert!(
        // 31 since the family instances written as spaces that draw no
        // body are left out, as Revit leaves them out.
        diag_nodes.len() > 25,
        "{} diagnostic nodes",
        diag_nodes.len()
    );
    let main_paths: std::collections::HashSet<&str> = main["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["path"].as_str().unwrap())
        .collect();
    for n in diag_nodes {
        assert!(
            main_paths.contains(n["path"].as_str().unwrap()),
            "diagnostics on a node main lacks"
        );
        assert_eq!(n.as_object().unwrap().len(), 2, "path + attributes only");
        for k in n["attributes"].as_object().unwrap().keys() {
            assert_eq!(k, SOURCE_STREAM);
        }
    }
    let schemas: Vec<&String> = diag["schemas"].as_object().unwrap().keys().collect();
    assert_eq!(schemas, [SOURCE_STREAM]);

    // Main + diagnostics = the graph's whole IFCX tree: every node's
    // attributes, children and inherits, and every schema.
    let mut rf = RevitFile::open(&path).unwrap();
    // As rvt-ifc does by default: real shapes and materials.
    let mut model = rvt::convert::export(
        &mut rf,
        &rvt::convert::ConvertOptions {
            family_geometry: true,
            saved_meshes: true,
            saved_materials: true,
            proxy_shapes: true,
            ..Default::default()
        },
    )
    .unwrap()
    .model;
    // As rvt-ifc does by default.
    rvt::ifc::body_geometry::share_shapes_by_convention(&mut model);
    let graph = IfcxGraph::from_model(
        &model,
        GraphOptions {
            meshes: true,
            shared_shapes: true,
        },
    );
    let mut composed: std::collections::BTreeMap<String, Map<String, Value>> = main["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| {
            (
                n["path"].as_str().unwrap().to_string(),
                n.as_object().unwrap().clone(),
            )
        })
        .collect();
    let mut moved = 0;
    for n in diag_nodes {
        let node = composed.get_mut(n["path"].as_str().unwrap()).unwrap();
        let attrs = node
            .entry("attributes")
            .or_insert_with(|| Value::Object(Map::new()));
        for (k, v) in n["attributes"].as_object().unwrap() {
            assert!(
                attrs
                    .as_object_mut()
                    .unwrap()
                    .insert(k.clone(), v.clone())
                    .is_none(),
                "{k} in both"
            );
            moved += 1;
        }
    }
    assert_eq!(moved, diag_nodes.len(), "one diagnostic per node");
    // The library's single file (write_ifcx_graph) is that composition.
    let mut merged = main.clone();
    merged["data"] = Value::Array(composed.values().cloned().map(Value::Object).collect());
    merged["schemas"]
        .as_object_mut()
        .unwrap()
        .extend(diag["schemas"].as_object().unwrap().clone());
    let whole: Value = serde_json::from_str(&write_ifcx_graph(&graph).unwrap()).unwrap();
    // The CLI stamps the header with the input's modified time (or SOURCE_DATE_EPOCH, or
    // --ifc4-timestamp), the library default does not: both are ISO 8601 (ifcx.tsp).
    for t in [&main["header"]["timestamp"], &whole["header"]["timestamp"]] {
        let t = t.as_str().unwrap();
        let b = t.as_bytes();
        assert!(
            t.len() == 19
                && b[4] == b'-'
                && b[7] == b'-'
                && b[10] == b'T'
                && b[13] == b':'
                && b[16] == b':',
            "header timestamp {t:?} is not ISO 8601"
        );
    }
    merged["header"]["timestamp"] = whole["header"]["timestamp"].clone();
    assert!(
        whole == merged,
        "main + diagnostics differs from the single file"
    );
    let want: std::collections::BTreeMap<&String, Map<String, Value>> = graph
        .nodes
        .iter()
        .filter(|(_, n)| n.ifcx)
        .map(|(p, n)| {
            let attrs: Map<String, Value> = n
                .attributes
                .iter()
                .filter(|(k, _)| !k.starts_with("rvtopen::ifc4::"))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            (p, attrs)
        })
        .collect();
    assert_eq!(want.len(), composed.len());
    for (p, attrs) in want {
        let got = composed[p.as_str()]
            .get("attributes")
            .cloned()
            .unwrap_or(Value::Object(Map::new()));
        // Through text, as the file went: serde_json's float parse is not
        // the writer's float text read back exactly.
        let want: Value = serde_json::from_str(&Value::Object(attrs).to_string()).unwrap();
        assert_eq!(got, want, "{p}");
    }
    let main_schemas = main["schemas"].as_object().unwrap();
    assert!(
        !main_schemas.contains_key(SOURCE_STREAM),
        "main declares a key it does not use"
    );
}
