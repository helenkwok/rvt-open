//! the ThatOpen Fragments (`.frag`) writer against the IFCX of the
//! same graph, on a real file.
//!
//! Real file: `ARK_NordicLCA_Housing_Terrain-Concrete_BuildingPermit.rvt`
//! (Nordic LCA / BIM4LCA, Revit 2024; VTT, Granlund, Gravicon,
//! Insinööritoimisto Kallinen; CC BY-SA 4.0,
//! <https://cris.vtt.fi/en/publications/bim-files-to-download/>), from
//! `RVT_NORDIC_LCA_DIR`. Skips when absent.
//!
//! The `.frag` is inflated and read back with a minimal FlatBuffers reader
//! written from the public schema (`@thatopen/fragments` 3.6.0 `index.fbs`),
//! independent of the writer's builder. The public loader itself is run on
//! the whole corpus by separate scripts.

use rvt::RevitFile;
use rvt::ifc::frag_writer::{write_frag_graph, write_frag_graph_with};
use rvt::ifc::gltf::DisplayOptions;
use rvt::ifc::{GraphOptions, IfcxGraph, RvtDocExporter, write_ifcx_graph};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::PathBuf;

/// A FlatBuffers table or vector position in `b`.
struct Fb<'a>(&'a [u8]);

impl Fb<'_> {
    fn u16(&self, p: usize) -> usize {
        u16::from_le_bytes(self.0[p..p + 2].try_into().unwrap()) as usize
    }
    fn u32(&self, p: usize) -> usize {
        u32::from_le_bytes(self.0[p..p + 4].try_into().unwrap()) as usize
    }
    fn deref(&self, p: usize) -> usize {
        p + self.u32(p)
    }
    /// Where field `id` of the table at `t` is, if set.
    fn field(&self, t: usize, id: usize) -> Option<usize> {
        let soff = i32::from_le_bytes(self.0[t..t + 4].try_into().unwrap());
        let vt = (t as i64 - i64::from(soff)) as usize;
        if 4 + 2 * id >= self.u16(vt) {
            return None;
        }
        match self.u16(vt + 4 + 2 * id) {
            0 => None,
            o => Some(t + o),
        }
    }
    /// The vector a field points to: (first element, length).
    fn vector(&self, t: usize, id: usize) -> (usize, usize) {
        let Some(f) = self.field(t, id) else {
            return (0, 0);
        };
        let v = self.deref(f);
        (v + 4, self.u32(v))
    }
    fn table_at(&self, t: usize, id: usize) -> usize {
        self.deref(self.field(t, id).expect("required table"))
    }
    fn string(&self, p: usize) -> &str {
        let s = self.deref(p);
        std::str::from_utf8(&self.0[s + 4..s + 4 + self.u32(s)]).unwrap()
    }
}

#[test]
fn nordic_housing_terrain_frag_matches_its_ifcx() {
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
    let graph = IfcxGraph::from_model(
        &model,
        GraphOptions {
            meshes: true,
            shared_shapes: false,
        },
    );
    let ifcx: Value = serde_json::from_str(&write_ifcx_graph(&graph).expect("ifcx")).unwrap();
    // Every IFCX mesh, rooms included (by default they have no
    // shell, the format storing no visibility).
    let all = DisplayOptions {
        rooms: true,
        ..Default::default()
    };
    let (deflated, stats) = write_frag_graph_with(&graph, &all).expect("frag");
    let default_meshes = write_frag_graph(&graph).unwrap().1.meshes;
    assert!(
        default_meshes <= stats.meshes,
        "rooms only ever leave meshes out"
    );
    assert_eq!(
        write_frag_graph_with(&graph, &all).unwrap().0,
        deflated,
        "byte-identical rerun"
    );

    // What IFCX holds.
    let data = ifcx["data"].as_array().unwrap();
    let by_path: BTreeMap<&str, &Value> = data
        .iter()
        .map(|n| (n["path"].as_str().unwrap(), n))
        .collect();
    let is_prop = |k: &str| {
        (k.starts_with("bsi::ifc::prop::")
            && k != "bsi::ifc::prop::Name"
            && k != "bsi::ifc::prop::Description")
            || k.starts_with("ifc-src::pset::")
            || k.starts_with("rvtopen::revit::")
    };
    let (mut items, mut props, mut meshes, mut tris) = (0, 0, 0, 0);
    let mut tags = BTreeSet::new();
    let mut classes: BTreeMap<String, usize> = BTreeMap::new();
    // Type nodes: those an occurrence inherits, and the
    // unplaced types, which nothing inherits.
    let typed: BTreeSet<&str> = data
        .iter()
        .flat_map(|n| {
            n["inherits"]
                .as_object()
                .into_iter()
                .flatten()
                .map(|(_, t)| t.as_str().unwrap())
        })
        .chain(
            data.iter()
                .filter(|n| n["attributes"][rvt::ifc::graph::UNPLACED_TYPE] == true)
                .map(|n| n["path"].as_str().unwrap()),
        )
        .collect();
    for n in data {
        let a = n["attributes"].as_object();
        let Some(code) = a
            .and_then(|a| a.get("bsi::ifc::class"))
            .and_then(|c| c["code"].as_str())
        else {
            continue;
        };
        items += 1;
        let suffix = if typed.contains(n["path"].as_str().unwrap()) {
            "TYPE"
        } else {
            ""
        };
        *classes
            .entry(format!("{}{suffix}", code.to_uppercase()))
            .or_default() += 1;
        let a = a.unwrap();
        // a flat standard property sits in every set its
        // `rvtopen::ifc::psetOf::<P>` names.
        props += a
            .keys()
            .filter(|k| is_prop(k))
            .map(|k| {
                k.strip_prefix("bsi::ifc::prop::")
                    .and_then(|p| a.get(&format!("rvtopen::ifc::psetOf::{p}")))
                    .and_then(Value::as_array)
                    .map_or(1, |sets| sets.len().max(1))
            })
            .sum::<usize>();
        if let Some(t) = a
            .get("ifc-src::attr::Tag")
            .and_then(Value::as_str)
            .and_then(|t| t.parse::<u32>().ok())
        {
            tags.insert(t);
        }
        for c in n["children"].as_object().into_iter().flatten() {
            let m = &by_path[c.1.as_str().unwrap()]["attributes"]["usd::usdgeom::mesh"];
            if let Some(ix) = m["faceVertexIndices"].as_array().filter(|ix| ix.len() >= 3) {
                meshes += 1;
                tris += ix.len() / 3;
            }
        }
    }
    assert!(
        items > 100 && props > 1_000 && tris > 1_000,
        "a real model: {items} {props} {tris}"
    );

    // What the .frag holds, read back from its bytes.
    let mut raw = Vec::new();
    flate2::read::ZlibDecoder::new(&deflated[..])
        .read_to_end(&mut raw)
        .expect("zlib");
    let b = Fb(&raw);
    assert_eq!(&raw[4..8], b"0001", "file identifier");
    let root = b.deref(0);
    let (ids_at, n_ids) = b.vector(root, 4);
    let (cats_at, n_cats) = b.vector(root, 5);
    let (_, n_attrs) = b.vector(root, 7);
    assert_eq!(
        (n_cats, n_attrs),
        (n_ids, n_ids),
        "one category and attribute row per item"
    );
    let local_ids: BTreeSet<usize> = (0..n_ids).map(|i| b.u32(ids_at + 4 * i)).collect();
    assert_eq!(local_ids.len(), n_ids, "local ids are unique");
    let mut frag_classes: BTreeMap<String, usize> = BTreeMap::new();
    for i in 0..n_cats {
        *frag_classes
            .entry(b.string(cats_at + 4 * i).to_string())
            .or_default() += 1;
    }
    let values = frag_classes.remove("IFCPROPERTYSINGLEVALUE").unwrap_or(0);
    let sets = frag_classes.remove("IFCPROPERTYSET").unwrap_or(0);
    assert_eq!(frag_classes, classes, "an item per IFCX node, same class");
    assert_eq!(values, props, "a property value per IFCX property key");
    assert_eq!(
        (sets, values, items),
        (stats.property_sets, stats.properties, stats.items)
    );
    assert!(
        tags.iter().all(|t| local_ids.contains(&(*t as usize))),
        "every Revit ElementId is a local id"
    );

    let mesh = b.table_at(root, 6);
    let (_, n_items) = b.vector(mesh, 1);
    let (_, n_samples) = b.vector(mesh, 2);
    let (shells_at, n_shells) = b.vector(mesh, 6);
    assert_eq!(
        (n_items, n_samples, n_shells),
        (meshes, meshes, meshes),
        "one sample per IFCX mesh"
    );
    let frag_tris: usize = (0..n_shells)
        .map(|i| {
            let s = b.deref(shells_at + 4 * i);
            b.vector(s, 0).1 + b.vector(s, 3).1
        })
        .sum();
    assert_eq!(frag_tris, tris, "a shell profile per IFCX triangle");
    assert_eq!(stats.triangles, tris);
}
