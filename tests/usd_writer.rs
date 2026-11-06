//! the OpenUSD writer (`write_usda`), a projection of the same
//! graph as IFCX. Checked against the IFCX of the same model: elements and
//! their classes, every IFCX key per node, and mesh triangles.
//!
//! Real file: `ARK_NordicLCA_Housing_Terrain-Concrete_BuildingPermit.rvt`
//! (Nordic LCA / BIM4LCA, Revit 2024; VTT, Granlund, Gravicon,
//! Insinööritoimisto Kallinen; CC BY-SA 4.0,
//! <https://cris.vtt.fi/en/publications/bim-files-to-download/>), from
//! `RVT_NORDIC_LCA_DIR`. Skips when absent. OpenUSD's own validators run on
//! the whole corpus separately, not here.
//!
//! the binary writer (`write_usdc`) holds the same specs as the
//! text. Read back here with a small Crate reader written from AOUSD Core
//! Specification 1.0.1 section 16.3: the same prim and property paths, and
//! one spec each. OpenUSD's own reader and validators check the corpus in
//! a separate check.

use rvt::RevitFile;
use rvt::ifc::{
    GraphOptions, IfcxGraph, RvtDocExporter, write_ifcx_graph, write_usda_graph, write_usdc_graph,
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// What one USD prim of this writer holds.
#[derive(Default, Debug)]
struct Prim {
    class: Option<String>,
    keys: BTreeSet<String>,
    triangles: usize,
}

/// The first `"..."` literal of a line, unescaped.
fn literal(line: &str) -> Option<String> {
    let start = line.find('"')? + 1;
    let mut out = String::new();
    let mut chars = line[start..].chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => out.push(chars.next()?),
            c => out.push(c),
        }
    }
    None
}

/// Prims by name, read from the writer's own layout: a prim header line,
/// metadata in `( )`, its body in `{ }` on lines of their own, attribute
/// customData in `= { }`.
fn read_usda(text: &str) -> BTreeMap<String, Prim> {
    let mut prims: BTreeMap<String, Prim> = BTreeMap::new();
    let mut stack: Vec<Option<String>> = Vec::new();
    let mut pending: Option<String> = None;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("def ") || t.starts_with("class ") {
            pending = literal(t);
        } else if t == "{" {
            if let Some(name) = &pending {
                prims.entry(name.clone()).or_default();
            }
            stack.push(pending.take());
        } else if t.ends_with("= {") {
            stack.push(None);
        } else if t == "}" {
            stack.pop();
        } else if let Some(Some(name)) = stack.iter().rev().find(|s| s.is_some()) {
            let p = prims.entry(name.clone()).or_default();
            if t.starts_with("string ifcx = ") {
                p.keys.insert(literal(t).expect("key"));
            } else if t.starts_with("string bsi:ifc:class:code = ") {
                p.class = literal(t);
            } else if let Some(counts) = t.strip_prefix("int[] faceVertexCounts = [") {
                p.triangles = counts.split(',').filter(|c| c.trim() != "]").count();
                assert!(counts.trim_end_matches(']').split(", ").all(|c| c == "3"));
            }
        }
    }
    assert!(stack.is_empty(), "balanced braces");
    prims.remove("Types");
    prims
}

#[test]
fn nordic_housing_terrain_usd_holds_what_its_ifcx_holds() {
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
    let graph = IfcxGraph::from_model(&model, GraphOptions::default());
    let usda = write_usda_graph(&graph).expect("usda");
    assert_eq!(
        usda,
        write_usda_graph(&graph).unwrap(),
        "byte-identical rerun"
    );
    let head: String = usda.lines().take(12).collect::<Vec<_>>().join("\n");
    assert!(head.starts_with("#usda 1.0"));
    assert!(head.contains("metersPerUnit = 1") && head.contains("upAxis = \"Z\""));
    assert!(head.contains("defaultPrim = \"_"));

    let doc: Value = serde_json::from_str(&write_ifcx_graph(&graph).expect("ifcx")).unwrap();
    let data = doc["data"].as_array().unwrap();
    let types: BTreeSet<&str> = data
        .iter()
        .flat_map(|n| n["inherits"].as_object().into_iter().flatten())
        .map(|(_, t)| t.as_str().unwrap())
        .collect();
    let prims = read_usda(&usda);
    let name = |path: &str| format!("_{}", path.replace('-', ""));

    let (mut elements, mut triangles_ifcx, mut triangles_usd) = (0, 0, 0);
    for n in data {
        let path = n["path"].as_str().unwrap();
        let attrs = n["attributes"].as_object();
        let prim = prims
            .get(&name(path))
            .unwrap_or_else(|| panic!("no prim for {path}"));
        let keys: BTreeSet<String> = attrs
            .into_iter()
            .flatten()
            .map(|(k, _)| k.clone())
            .filter(|k| k != "usd::usdgeom::mesh")
            .collect();
        assert_eq!(prim.keys, keys, "keys of {path}");
        let class = n["attributes"]["bsi::ifc::class"]["code"].as_str();
        assert_eq!(prim.class.as_deref(), class, "class of {path}");
        if class.is_some() && !types.contains(path) {
            elements += 1;
        }
        if let Some(ix) = n["attributes"]["usd::usdgeom::mesh"]["faceVertexIndices"].as_array() {
            triangles_ifcx += ix.len() / 3;
            triangles_usd += prim.triangles;
        }
    }
    assert_eq!(prims.len(), data.len(), "one prim per IFCX node");
    assert!(elements > 100, "{elements} elements");
    assert!(triangles_ifcx > 0);
    assert_eq!(triangles_usd, triangles_ifcx, "mesh triangles");
}

/// Prim and property paths of this writer's `.usda` layout: prims open with
/// a `def`/`class` line and a `{` line, metadata sits in `( )`, dictionaries
/// in `= { }`.
fn usda_paths(text: &str) -> BTreeSet<String> {
    enum Frame {
        Prim(String),
        Paren,
        Dict,
    }
    let mut paths = BTreeSet::new();
    let mut stack: Vec<Frame> = Vec::new();
    let mut pending = String::new();
    let here = |stack: &[Frame]| -> String {
        stack
            .iter()
            .filter_map(|f| match f {
                Frame::Prim(n) => Some(format!("/{n}")),
                _ => None,
            })
            .collect()
    };
    for line in text.lines().skip(1) {
        let t = line.trim();
        match stack.last() {
            Some(Frame::Paren) | Some(Frame::Dict) => {
                if t == ")" || t == "}" {
                    stack.pop();
                } else if t.ends_with("= {") {
                    stack.push(Frame::Dict);
                }
                continue;
            }
            _ => {}
        }
        if t == "(" {
            stack.push(Frame::Paren);
        } else if t.starts_with("def ") || t.starts_with("class ") {
            pending = literal(t).expect("prim name");
            if t.ends_with('(') {
                stack.push(Frame::Paren);
            }
        } else if t == "{" {
            stack.push(Frame::Prim(pending.clone()));
            paths.insert(here(&stack));
        } else if t == "}" {
            stack.pop();
        } else if let Some((left, _)) = t.split_once(" = ") {
            let name = left.rsplit(' ').next().unwrap();
            paths.insert(format!("{}.{name}", here(&stack)));
            if t.ends_with(" (") {
                stack.push(Frame::Paren);
            }
        }
    }
    assert!(stack.is_empty(), "balanced");
    paths
}

fn lz4_decode(src: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    let len = |i: &mut usize, mut n: usize| {
        if n == 15 {
            loop {
                let b = src[*i];
                *i += 1;
                n += b as usize;
                if b != 255 {
                    break;
                }
            }
        }
        n
    };
    while i < src.len() {
        let token = src[i];
        i += 1;
        let lit = len(&mut i, (token >> 4) as usize);
        out.extend_from_slice(&src[i..i + lit]);
        i += lit;
        if i >= src.len() {
            break;
        }
        let offset = u16::from_le_bytes([src[i], src[i + 1]]) as usize;
        i += 2;
        let m = len(&mut i, (token & 15) as usize) + 4;
        for _ in 0..m {
            out.push(out[out.len() - offset]);
        }
    }
    out
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

/// A buffer with its one-byte chunk count (0: one LZ4 block).
fn buffer(b: &[u8]) -> Vec<u8> {
    assert_eq!(b[0], 0, "one chunk");
    lz4_decode(&b[1..])
}

/// A compressed 32-bit integer array at `at`; returns it and the next offset.
fn ints(b: &[u8], at: usize, count: usize) -> (Vec<i64>, usize) {
    let size = u64_at(b, at) as usize;
    let raw = buffer(&b[at + 8..at + 8 + size]);
    let common = i64::from(i32::from_le_bytes(raw[0..4].try_into().unwrap()));
    let mut p = 4 + (count * 2).div_ceil(8);
    let (mut prev, mut out) = (0i64, Vec::with_capacity(count));
    for k in 0..count {
        let d = match (raw[4 + k / 4] >> (2 * (k % 4))) & 3 {
            0 => common,
            1 => i64::from(raw[p] as i8),
            2 => i64::from(i16::from_le_bytes(raw[p..p + 2].try_into().unwrap())),
            _ => i64::from(i32::from_le_bytes(raw[p..p + 4].try_into().unwrap())),
        };
        p += [0, 1, 2, 4][((raw[4 + k / 4] >> (2 * (k % 4))) & 3) as usize];
        prev = i64::from((prev as i32).wrapping_add(d as i32));
        out.push(prev);
    }
    (out, at + 8 + size)
}

/// Every spec's path and form, with the paths rebuilt from the PATHS section.
pub(crate) fn read_usdc(b: &[u8]) -> Vec<(String, i64)> {
    assert_eq!(&b[..8], b"PXR-USDC");
    assert_eq!(&b[8..11], &[0, 8, 0], "Crate 0.8.0");
    let toc = u64_at(b, 16) as usize;
    let mut sections = BTreeMap::new();
    for k in 0..u64_at(b, toc) as usize {
        let at = toc + 8 + 32 * k;
        let name = String::from_utf8(
            b[at..at + 16]
                .iter()
                .take_while(|c| **c != 0)
                .copied()
                .collect(),
        )
        .unwrap();
        sections.insert(name, u64_at(b, at + 16) as usize);
    }
    let t = sections["TOKENS"];
    let n = u64_at(b, t) as usize;
    let raw = buffer(&b[t + 24..t + 24 + u64_at(b, t + 16) as usize]);
    let tokens: Vec<String> = raw
        .split(|c| *c == 0)
        .take(n)
        .map(|s| String::from_utf8(s.to_vec()).unwrap())
        .collect();
    let p = sections["PATHS"];
    let n = u64_at(b, p + 8) as usize;
    let (ix, at) = ints(b, p + 16, n);
    let (el, at) = ints(b, at, n);
    let (jump, _) = ints(b, at, n);
    let mut paths = vec![String::new(); n];
    // (encoded index, parent path); a sibling shares the parent.
    let mut todo = vec![(0usize, None::<String>)];
    while let Some((mut x, parent)) = todo.pop() {
        let mut parent = parent;
        loop {
            let path = match &parent {
                None => "/".to_string(),
                Some(pp) => {
                    let tok = &tokens[el[x].unsigned_abs() as usize];
                    let sep = if el[x] < 0 { "." } else { "/" };
                    if pp == "/" && sep == "/" {
                        format!("/{tok}")
                    } else {
                        format!("{pp}{sep}{tok}")
                    }
                }
            };
            paths[ix[x] as usize] = path.clone();
            match jump[x] {
                -2 => break,
                -1 => {}
                0 => {
                    x += 1;
                    continue;
                }
                j => todo.push((x + j as usize, parent.clone())),
            }
            parent = Some(path);
            x += 1;
        }
    }
    let s = sections["SPECS"];
    let n = u64_at(b, s) as usize;
    let (sp, at) = ints(b, s + 8, n);
    let (_, at) = ints(b, at, n);
    let (form, _) = ints(b, at, n);
    sp.iter()
        .zip(form)
        .map(|(p, f)| (paths[*p as usize].clone(), f))
        .collect()
}

#[test]
fn nordic_housing_terrain_usdc_holds_the_specs_of_its_usda() {
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
    let graph = IfcxGraph::from_model(&model, GraphOptions::default());
    let usdc = write_usdc_graph(&graph).expect("usdc");
    assert_eq!(
        usdc,
        write_usdc_graph(&graph).unwrap(),
        "byte-identical rerun"
    );
    let specs = read_usdc(&usdc);
    let usda = usda_paths(&write_usda_graph(&graph).unwrap());
    assert_eq!(
        specs.iter().filter(|(_, f)| *f == 7).count(),
        1,
        "one layer spec"
    );
    let prims = specs.iter().filter(|(_, f)| *f == 6).count();
    let props = specs.iter().filter(|(_, f)| *f == 1).count();
    let paths: BTreeSet<String> = specs
        .iter()
        .filter(|(_, f)| *f != 7)
        .map(|(p, _)| p.clone())
        .collect();
    assert_eq!(paths.len(), prims + props, "one spec per path");
    assert_eq!(paths, usda, "same prim and property paths as the .usda");
    assert!(
        prims > 100 && props > prims,
        "{prims} prims, {props} properties"
    );
    assert!(
        usdc.len() * 2 < write_usda_graph(&graph).unwrap().len(),
        "binary is smaller"
    );
}
