//! ThatOpen Fragments (`.frag`) writer: another projection of the IFCX graph
//! beside IFCX, IFC4 STEP and glTF.
//!
//! Format: the `Model` FlatBuffers table of the public, MIT-licensed
//! `@thatopen/fragments` schema (`packages/fragments/flatbuffers/index.fbs`
//! of github.com/ThatOpen/engine_fragment; unchanged from 3.4.6 to 3.6.0,
//! which is the version targeted), file identifier `0001`, zlib-deflated as
//! the public loader expects (`pako.deflate`). The FlatBuffers encoding is
//! written by a small safe builder here: generated Rust code needs `unsafe`,
//! which this crate forbids.
//!
//! What is written, read from the graph that IFCX is written from:
//! - one item per IFCX node with a `bsi::ifc::class` (project, site,
//!   building, storeys, elements, types); local id = the Revit ElementId
//!   (`Tag`) when it is a free number, else the next free id; GUID = the
//!   IFC GlobalId of the node's path; category = the IFC class in upper case
//!   (a type node: its occurrence class + `TYPE`);
//! - attributes `Name`, `Description` and every `ifc-src::attr::*` as
//!   `[name, value, IFCTYPE]`, the public importer's encoding;
//! - properties as `IFCPROPERTYSET` items (`Name`) holding
//!   `IFCPROPERTYSINGLEVALUE` items (`Name`, `NominalValue`) through
//!   `HasProperties`, linked by `IsDefinedBy` / `DefinesOccurrence`; the set
//!   is the IFCX key's (`ifc-src::pset::<Set>::<Prop>`,
//!   `rvtopen::revit::<Group>::<Name>`, and a flat `bsi::ifc::prop::<Prop>` in
//!   each standard set its `rvtopen::ifc::psetOf::<Prop>` names;
//!   a set named `bsi::ifc::prop` only without one); a type's sets by
//!   `HasPropertySets`;
//! - `metadata.georeference`: the site's georeference;
//! - types by `IsDefinedBy` / `ObjectTypeOf`; the tree by `IsDecomposedBy` /
//!   `Decomposes` (a spatial child, or any child of a non-spatial parent)
//!   and `ContainsElements` / `ContainedInStructure`; the spatial structure
//!   as the public importer builds it;
//! - geometry: each element's IFCX `Body` mesh as one shell (one profile
//!   per triangle), Y-up as three.js and the public importer have it
//!   (IFC `(x, y, z)` is `(x, z, -y)`), metres, one global transform per item
//!   at the mesh's lower corner (doubles) with the points relative to it
//!   (floats), and an RGBA material from the IFCX colour and opacity.
//!
//! Output is a pure function of the graph: byte-identical on a rerun.

use super::graph::IfcxGraph;
use super::ifcx_writer::IfcxError;
use crate::revit_global_ids::compress_ifc_guid;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Write;

/// The public `@thatopen/fragments` version whose schema this writes.
pub const FRAGMENTS_SCHEMA_VERSION: &str = "3.6.0";

/// Counts of what was written, for the loader comparison.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FragStats {
    /// Items made from IFCX nodes (property sets and values excluded).
    pub items: usize,
    /// Property set items.
    pub property_sets: usize,
    /// Property value items: one per IFCX property key on a node.
    pub properties: usize,
    /// Items with a shell.
    pub meshes: usize,
    /// Triangles over all shells.
    pub triangles: usize,
    /// distinct shells (representations); `meshes` samples draw them.
    pub shells: usize,
    /// pipes written as fragments' circle extrusions.
    pub circle_extrusions: usize,
    /// Shells with more than 65,000 points (`BIG`).
    pub big_shells: usize,
    /// Distinct RGBA materials.
    pub materials: usize,
    /// Items whose ElementId was taken by another item, given another id.
    pub tags_reassigned: usize,
}

const SPATIAL: &[&str] = &[
    "IFCPROJECT",
    "IFCSITE",
    "IFCBUILDING",
    "IFCBUILDINGSTOREY",
    "IFCSPACE",
];

/// The graph as a `.frag` file (deflated).
pub fn write_frag_graph(graph: &IfcxGraph) -> Result<(Vec<u8>, FragStats), IfcxError> {
    write_frag_graph_with(graph, &super::gltf::DisplayOptions::default())
}

/// [`write_frag_graph`] drawing what `display` asks for: with
/// `rooms` off, spaces keep their items, attributes and tree place but no
/// shell, since the format stores no visibility. `metadata.display` records
/// the choice. Fragments 3.6 stores no line items (its representations are
/// shells and circle extrusions), so `edges` is not drawn here.
pub fn write_frag_graph_with(
    graph: &IfcxGraph,
    display: &super::gltf::DisplayOptions,
) -> Result<(Vec<u8>, FragStats), IfcxError> {
    if let Some(e) = graph.errors.first() {
        return Err(IfcxError(e.clone()));
    }
    let mut stats = FragStats::default();
    let class_of = |p: &str| -> Option<String> {
        graph.nodes.get(p)?.attributes.get("bsi::ifc::class")?["code"]
            .as_str()
            .map(str::to_uppercase)
    };
    // an unplaced type is a type item too.
    let typed: BTreeSet<&str> = graph
        .nodes
        .values()
        .flat_map(|n| n.inherits.iter().map(String::as_str))
        .chain(graph.type_nodes.iter().map(String::as_str))
        .collect();
    let paths: Vec<&str> = graph
        .nodes
        .iter()
        .filter(|(p, n)| n.ifcx && class_of(p).is_some())
        .map(|(p, _)| p.as_str())
        .collect();

    // Local ids: the ElementId where free, then the next free ids.
    let tag = |p: &str| -> Option<u32> {
        graph.nodes[p]
            .attributes
            .get("ifc-src::attr::Tag")?
            .as_str()?
            .parse()
            .ok()
    };
    let mut id_of: HashMap<&str, u32> = HashMap::new();
    let mut used = BTreeSet::new();
    for p in &paths {
        if let Some(t) = tag(p) {
            if used.insert(t) {
                id_of.insert(p, t);
            } else {
                stats.tags_reassigned += 1;
            }
        }
    }
    let mut next = used.last().map_or(1, |m| m + 1);
    let mut fresh = || {
        next += 1;
        next - 1
    };
    for p in &paths {
        if !id_of.contains_key(p) {
            id_of.insert(p, fresh());
        }
    }

    let mut items: Vec<Item> = Vec::new();
    let mut rels: BTreeMap<u32, BTreeMap<&'static str, Vec<u32>>> = BTreeMap::new();
    let mut rel = |from: u32, name: &'static str, to: u32| {
        rels.entry(from)
            .or_default()
            .entry(name)
            .or_default()
            .push(to);
    };
    // (item, mesh, colour, Some((shared shape path, Body xformop))).
    #[allow(clippy::type_complexity)]
    let mut geometry: Vec<(usize, &Value, [u8; 4], Option<(&str, Option<&Value>)>)> = Vec::new();
    // bodies that are a circle swept along a straight axis (a
    // pipe), written as fragments' own CIRCLE_EXTRUSION: (item, axis start
    // and end in metres Y-up, radius in metres, colour).
    #[allow(clippy::type_complexity)]
    let mut circles: Vec<(usize, [f64; 3], [f64; 3], f64, [u8; 4])> = Vec::new();
    for p in &paths {
        let node = &graph.nodes[*p];
        let id = id_of[p];
        let mut category = class_of(p).expect("filtered");
        if typed.contains(p) {
            category.push_str("TYPE");
        }
        let mut attrs = Vec::new();
        let mut sets: BTreeMap<String, Vec<(String, &Value)>> = BTreeMap::new();
        for (k, v) in &node.attributes {
            if let Some(name) = k.strip_prefix("ifc-src::attr::") {
                attrs.push(attr(name, v));
            } else if let Some(name) = k.strip_prefix("bsi::ifc::prop::") {
                if matches!(name, "Name" | "Description") {
                    attrs.push(attr(name, v));
                } else {
                    // into the standard sets it came from, by name.
                    let from: Vec<String> = node
                        .attributes
                        .get(&format!("{}{name}", super::graph::PSET_OF))
                        .and_then(Value::as_array)
                        .map(|a| {
                            a.iter()
                                .filter_map(|s| s.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();
                    if from.is_empty() {
                        sets.entry("bsi::ifc::prop".into())
                            .or_default()
                            .push((name.into(), v));
                    }
                    for set in from {
                        sets.entry(set).or_default().push((name.into(), v));
                    }
                }
            } else if let Some(rest) = k
                .strip_prefix("ifc-src::pset::")
                .or_else(|| k.strip_prefix("rvtopen::revit::"))
            {
                let (set, name) = rest.split_once("::").unwrap_or(("", rest));
                match v.as_object() {
                    // a layer's quantities, one value per part
                    // (`<layer>.NetVolume`): a property holds one value.
                    Some(parts) => sets.entry(set.into()).or_default().extend(
                        parts
                            .iter()
                            .map(|(part, pv)| (format!("{name}.{part}"), pv)),
                    ),
                    None => sets.entry(set.into()).or_default().push((name.into(), v)),
                }
            }
        }
        attrs.sort();
        let guid = Some(compress_ifc_guid(
            u128::from_str_radix(&p.replace('-', ""), 16)
                .map_err(|_| IfcxError(format!("path {p} is no UUID")))?
                .to_be_bytes(),
        ));
        let index = items.len();
        items.push(Item {
            id,
            category,
            attrs,
            guid,
        });
        for (set, props) in sets {
            let set_id = fresh();
            items.push(Item {
                id: set_id,
                category: "IFCPROPERTYSET".into(),
                attrs: vec![attr("Name", &json!(set))],
                guid: None,
            });
            stats.property_sets += 1;
            if typed.contains(p) {
                rel(id, "HasPropertySets", set_id);
            } else {
                rel(id, "IsDefinedBy", set_id);
                rel(set_id, "DefinesOccurrence", id);
            }
            for (name, v) in props {
                let prop_id = fresh();
                items.push(Item {
                    id: prop_id,
                    category: "IFCPROPERTYSINGLEVALUE".into(),
                    attrs: vec![attr("Name", &json!(name)), attr("NominalValue", v)],
                    guid: None,
                });
                stats.properties += 1;
                rel(set_id, "HasProperties", prop_id);
            }
        }
        for t in &node.inherits {
            if let Some(&t) = id_of.get(t.as_str()) {
                rel(id, "IsDefinedBy", t);
                rel(t, "ObjectTypeOf", id);
            }
        }
        let parent_spatial = SPATIAL.contains(&items[index].category.as_str());
        let circle = circle_axis(node);
        for c in node.children.values() {
            let Some(&cid) = id_of.get(c.as_str()) else {
                let Some(body) = graph.nodes.get(c) else {
                    continue;
                };
                if !display.rooms && items[index].category == "IFCSPACE" {
                    continue;
                }
                // a Body of one face material carries its colour.
                let own = body
                    .attributes
                    .contains_key("bsi::ifc::presentation::diffuseColor");
                let from = if own { body } else { node };
                let colour = rgba(
                    from.attributes.get("bsi::ifc::presentation::diffuseColor"),
                    from.attributes.get("bsi::ifc::presentation::opacity"),
                );
                if let Some((p1, p2, r)) = circle {
                    if !circles.iter().any(|c| c.0 == index) {
                        circles.push((index, p1, p2, r, colour));
                    }
                } else if let Some(mesh) = body.attributes.get("usd::usdgeom::mesh") {
                    geometry.push((index, mesh, colour, None));
                } else if let Some((shape, mesh)) = body.inherits.iter().find_map(|t| {
                    let m = graph.nodes.get(t)?.attributes.get("usd::usdgeom::mesh")?;
                    Some((t.as_str(), m))
                }) {
                    // a shared shape, placed by the Body's xformop.
                    let xf = body.attributes.get("usd::xformop");
                    geometry.push((index, mesh, colour, Some((shape, xf))));
                }
                continue;
            };
            let child_spatial = class_of(c).is_some_and(|k| SPATIAL.contains(&k.as_str()));
            if parent_spatial && !child_spatial {
                rel(id, "ContainsElements", cid);
                rel(cid, "ContainedInStructure", id);
            } else {
                rel(id, "IsDecomposedBy", cid);
                rel(cid, "Decomposes", id);
            }
        }
    }
    stats.items = paths.len();
    let max_local_id = fresh();

    // ---- write, children before parents
    let mut fb = Fb::default();
    let mut unique: BTreeSet<&str> = BTreeSet::new();
    let mut strings: HashMap<String, u32> = HashMap::new();
    let mut shared = |fb: &mut Fb, s: &str| -> u32 {
        if let Some(o) = strings.get(s) {
            return *o;
        }
        let o = fb.string(s);
        strings.insert(s.to_string(), o);
        o
    };

    // Geometry.
    let mut shells = Vec::new();
    let mut reprs: Vec<Vec<u8>> = Vec::new();
    let mut samples: Vec<Vec<u8>> = Vec::new();
    let mut globals: Vec<Vec<u8>> = Vec::new();
    let mut meshes_items: Vec<u32> = Vec::new();
    // an element with several Bodies (one per face material) is
    // one mesh item; its later samples sit at a local transform from the
    // item's global one (the public loader keys an item once).
    let mut item_slot: HashMap<usize, u32> = HashMap::new();
    let mut locals: Vec<Vec<u8>> = vec![transform([0.0; 3])];
    let mut materials: Vec<[u8; 4]> = Vec::new();
    // shape path -> its shell (and representation) index.
    let mut shell_of: HashMap<&str, u32> = HashMap::new();
    // Each shell's lower corner (Y-up): its points are stored relative to it.
    let mut shell_lo: Vec<[f64; 3]> = Vec::new();
    for (item, mesh, colour, shared) in &geometry {
        let reuse = shared.and_then(|(shape, _)| shell_of.get(shape).copied());
        let (shell_index, global) = if let Some(i) = reuse {
            (
                i,
                shared_transform(shared.and_then(|(_, xf)| xf), shell_lo[i as usize]),
            )
        } else {
            let pts: Vec<[f64; 3]> = mesh["points"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|p| {
                    let c = |i: usize| p[i].as_f64().unwrap_or(0.0);
                    [c(0), c(2), -c(1)]
                })
                .collect();
            let idx: Vec<u32> = mesh["faceVertexIndices"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_u64().map(|v| v as u32))
                .collect();
            if pts.is_empty() || idx.len() < 3 {
                continue;
            }
            let mut lo = [f64::INFINITY; 3];
            for p in &pts {
                for k in 0..3 {
                    lo[k] = lo[k].min(p[k]);
                }
            }
            let local: Vec<[f32; 3]> = pts
                .iter()
                .map(|p| {
                    [
                        (p[0] - lo[0]) as f32,
                        (p[1] - lo[1]) as f32,
                        (p[2] - lo[2]) as f32,
                    ]
                })
                .collect();
            let (mut bmin, mut bmax) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
            for p in &local {
                for k in 0..3 {
                    bmin[k] = bmin[k].min(p[k]);
                    bmax[k] = bmax[k].max(p[k]);
                }
            }
            let tris: Vec<&[u32]> = idx.chunks_exact(3).collect();
            let big = local.len() > 65_000;
            let profiles: Vec<u32> = tris
                .iter()
                .map(|t| {
                    let v = if big {
                        fb.vec_scalars(&t.iter().map(|i| i.to_le_bytes()).collect::<Vec<_>>())
                    } else {
                        fb.vec_scalars(
                            &t.iter()
                                .map(|i| (*i as u16).to_le_bytes())
                                .collect::<Vec<_>>(),
                        )
                    };
                    fb.start();
                    fb.add_off(0, v);
                    fb.end()
                })
                .collect();
            let profiles_vec = fb.vec_offsets(&profiles);
            let empty = fb.vec_offsets(&[]);
            // A face id per profile, as the IFCX-to-frag conversion writes them.
            let face_ids: Vec<[u8; 2]> = (0..tris.len())
                .map(|i| ((i % 65_535) as u16).to_le_bytes())
                .collect();
            let face_ids = fb.vec_scalars(&face_ids);
            let points = fb.vec_structs(&local.iter().map(|p| fv(*p)).collect::<Vec<_>>(), 4);
            fb.start();
            fb.add_off(0, if big { empty } else { profiles_vec });
            fb.add_off(1, empty);
            fb.add_off(2, points);
            fb.add_off(3, if big { profiles_vec } else { empty });
            fb.add_off(4, empty);
            fb.add_scalar(5, [u8::from(big)]);
            fb.add_off(6, face_ids);
            let shell_index = shells.len() as u32;
            shells.push(fb.end());
            stats.triangles += tris.len();
            stats.big_shells += usize::from(big);

            let mut r = shell_index.to_le_bytes().to_vec();
            r.extend(fv(bmin));
            r.extend(fv(bmax));
            r.extend([1, 0, 0, 0]); // SHELL + padding
            reprs.push(r);
            shell_lo.push(lo);
            match shared {
                Some((shape, xf)) => {
                    shell_of.insert(shape, shell_index);
                    (shell_index, shared_transform(*xf, lo))
                }
                None => (shell_index, transform(lo)),
            }
        };
        let material = match materials.iter().position(|m| m == colour) {
            Some(m) => m,
            None => {
                materials.push(*colour);
                materials.len() - 1
            }
        } as u32;
        let (sample_item, local) = match item_slot.get(item) {
            Some(&slot) => {
                locals.push(relative_transform(&globals[slot as usize], &global));
                (slot, (locals.len() - 1) as u32)
            }
            None => {
                let slot = meshes_items.len() as u32;
                item_slot.insert(*item, slot);
                meshes_items.push(*item as u32);
                globals.push(global);
                (slot, 0)
            }
        };
        let mut s = Vec::new();
        for v in [sample_item, material, shell_index, local] {
            s.extend(v.to_le_bytes());
        }
        samples.push(s);
        stats.meshes += 1;
    }
    // the circle extrusions, each one representation.
    let mut circle_tables = Vec::with_capacity(circles.len());
    for (item, p1, p2, radius, colour) in &circles {
        let d: [f64; 3] = std::array::from_fn(|k| p2[k] - p1[k]);
        let f = |v: [f64; 3]| [v[0] as f32, v[1] as f32, v[2] as f32];
        let mut wire = fv([0.0; 3]);
        wire.extend(fv(f(d)));
        let wires = fb.vec_structs(&[wire], 4);
        let order = fb.vec_scalars(&[0u32.to_le_bytes()]);
        let parts = fb.vec_scalars(&[[1u8]]); // WIRE
        fb.start();
        fb.add_off(0, wires);
        fb.add_off(1, order);
        fb.add_off(2, parts);
        let axis = fb.end();
        let axes = fb.vec_offsets(&[axis]);
        let radii = fb.vec_scalars(&[radius.to_le_bytes()]);
        fb.start();
        fb.add_off(0, radii);
        fb.add_off(1, axes);
        let index = circle_tables.len() as u32;
        circle_tables.push(fb.end());
        let r = *radius;
        let bmin = f(std::array::from_fn(|k| d[k].min(0.0) - r));
        let bmax = f(std::array::from_fn(|k| d[k].max(0.0) + r));
        let mut repr = index.to_le_bytes().to_vec();
        repr.extend(fv(bmin));
        repr.extend(fv(bmax));
        repr.extend([2, 0, 0, 0]); // CIRCLE_EXTRUSION + padding
        reprs.push(repr);
        let repr_index = (reprs.len() - 1) as u32;
        let global = transform(*p1);
        let material = match materials.iter().position(|m| m == colour) {
            Some(m) => m,
            None => {
                materials.push(*colour);
                materials.len() - 1
            }
        } as u32;
        let (sample_item, local) = match item_slot.get(item) {
            Some(&slot) => {
                locals.push(relative_transform(&globals[slot as usize], &global));
                (slot, (locals.len() - 1) as u32)
            }
            None => {
                let slot = meshes_items.len() as u32;
                item_slot.insert(*item, slot);
                meshes_items.push(*item as u32);
                globals.push(global);
                (slot, 0)
            }
        };
        let mut s = Vec::new();
        for v in [sample_item, material, repr_index, local] {
            s.extend(v.to_le_bytes());
        }
        samples.push(s);
        stats.meshes += 1;
        stats.circle_extrusions += 1;
    }
    stats.materials = materials.len();
    stats.shells = shells.len();
    let shells_vec = fb.vec_offsets(&shells);
    let reprs_vec = fb.vec_structs(&reprs, 4);
    let samples_vec = fb.vec_structs(&samples, 4);
    let globals_vec = fb.vec_structs(&globals, 8);
    let locals_vec = fb.vec_structs(&locals, 8);
    // Two-sided when translucent, as the IFCX-to-frag conversion.
    let mats: Vec<Vec<u8>> = materials
        .iter()
        .map(|m| vec![m[0], m[1], m[2], m[3], u8::from(m[3] < 255), 0])
        .collect();
    let mats_vec = fb.vec_structs(&mats, 1);
    let ext_vec = fb.vec_offsets(&circle_tables);
    let items_vec = fb.vec_scalars(
        &meshes_items
            .iter()
            .map(|v| v.to_le_bytes())
            .collect::<Vec<_>>(),
    );
    let mut ids = |fb: &mut Fb, n: usize| {
        let v: Vec<[u8; 4]> = (0..n).map(|_| fresh().to_le_bytes()).collect();
        fb.vec_scalars(&v)
    };
    let mat_ids = ids(&mut fb, materials.len());
    let repr_ids = ids(&mut fb, reprs.len());
    let sample_ids = ids(&mut fb, samples.len());
    let lt_ids = ids(&mut fb, locals.len());
    let gt_ids = ids(&mut fb, globals.len());
    let max_local_id = fresh().max(max_local_id);
    fb.start();
    fb.add_struct(0, &transform([0.0; 3]), 8);
    fb.add_off(1, items_vec);
    fb.add_off(2, samples_vec);
    fb.add_off(3, reprs_vec);
    fb.add_off(4, mats_vec);
    fb.add_off(5, ext_vec);
    fb.add_off(6, shells_vec);
    fb.add_off(7, locals_vec);
    fb.add_off(8, globals_vec);
    fb.add_off(9, mat_ids);
    fb.add_off(10, repr_ids);
    fb.add_off(11, sample_ids);
    fb.add_off(12, lt_ids);
    fb.add_off(13, gt_ids);
    let meshes = fb.end();

    // Items.
    let mut attr_offs = Vec::new();
    for it in &items {
        let data: Vec<u32> = it.attrs.iter().map(|a| shared(&mut fb, a)).collect();
        unique.extend(it.attrs.iter().map(String::as_str));
        let v = fb.vec_offsets(&data);
        fb.start();
        fb.add_off(0, v);
        attr_offs.push(fb.end());
    }
    let attrs_vec = fb.vec_offsets(&attr_offs);
    let cats: Vec<u32> = items
        .iter()
        .map(|it| shared(&mut fb, &it.category))
        .collect();
    let cats_vec = fb.vec_offsets(&cats);
    let local_ids = fb.vec_scalars(
        &items
            .iter()
            .map(|it| it.id.to_le_bytes())
            .collect::<Vec<_>>(),
    );
    let (guids, guid_items): (Vec<&str>, Vec<u32>) = items
        .iter()
        .filter_map(|it| it.guid.as_deref().map(|g| (g, it.id)))
        .unzip();
    let guid_offs: Vec<u32> = guids.iter().map(|g| fb.string(g)).collect();
    let guids_vec = fb.vec_offsets(&guid_offs);
    let guid_items_vec = fb.vec_scalars(
        &guid_items
            .iter()
            .map(|v| v.to_le_bytes())
            .collect::<Vec<_>>(),
    );
    let mut rel_offs = Vec::new();
    let mut rel_names = BTreeSet::new();
    for r in rels.values() {
        let data: Vec<u32> = r
            .iter()
            .map(|(name, ids)| {
                rel_names.insert(*name);
                let mut a = vec![json!(name)];
                a.extend(ids.iter().map(|i| json!(i)));
                fb.string(&Value::Array(a).to_string())
            })
            .collect();
        let v = fb.vec_offsets(&data);
        fb.start();
        fb.add_off(0, v);
        rel_offs.push(fb.end());
    }
    let rels_vec = fb.vec_offsets(&rel_offs);
    let rel_items = fb.vec_scalars(&rels.keys().map(|v| v.to_le_bytes()).collect::<Vec<_>>());
    let uniq: Vec<u32> = unique.iter().map(|a| shared(&mut fb, a)).collect();
    let uniq_vec = fb.vec_offsets(&uniq);
    let names: Vec<u32> = rel_names.iter().map(|n| fb.string(n)).collect();
    let names_vec = fb.vec_offsets(&names);

    // Spatial structure, as the public importer: a node per item, its
    // decomposition and contents grouped into category nodes.
    let category_of: HashMap<u32, &str> = items
        .iter()
        .map(|it| (it.id, it.category.as_str()))
        .collect();
    let project = paths
        .iter()
        .find(|p| class_of(p).as_deref() == Some("IFCPROJECT"))
        .map(|p| id_of[p]);
    let spatial = |fb: &mut Fb| -> u32 {
        fn node(
            fb: &mut Fb,
            id: u32,
            rels: &BTreeMap<u32, BTreeMap<&'static str, Vec<u32>>>,
            cat: &HashMap<u32, &str>,
            seen: &mut BTreeSet<u32>,
        ) -> u32 {
            let mut groups = Vec::new();
            if seen.insert(id) {
                for r in ["IsDecomposedBy", "ContainsElements"] {
                    let mut by: BTreeMap<&str, Vec<u32>> = BTreeMap::new();
                    for c in rels.get(&id).and_then(|m| m.get(r)).into_iter().flatten() {
                        by.entry(cat[c]).or_default().push(*c);
                    }
                    for (k, ids) in by {
                        let kids: Vec<u32> =
                            ids.iter().map(|c| node(fb, *c, rels, cat, seen)).collect();
                        let kids = fb.vec_offsets(&kids);
                        let name = fb.string(k);
                        fb.start();
                        fb.add_off(1, name);
                        fb.add_off(2, kids);
                        groups.push(fb.end());
                    }
                }
            }
            let kids = fb.vec_offsets(&groups);
            fb.start();
            fb.add_scalar(0, id.to_le_bytes());
            fb.add_off(2, kids);
            fb.end()
        }
        let kids: Vec<u32> = project
            .map(|p| node(fb, p, &rels, &category_of, &mut BTreeSet::new()))
            .into_iter()
            .collect();
        let kids = fb.vec_offsets(&kids);
        let name = fb.string("IFCPROJECT");
        fb.start();
        fb.add_off(1, name);
        fb.add_off(2, kids);
        fb.end()
    };
    let spatial = spatial(&mut fb);
    let mut meta = json!({"schema": "IFC4", "generator": "rvt-open", "fragmentsSchema": FRAGMENTS_SCHEMA_VERSION});
    // the site's georeference (latitude, longitude, elevation and
    // the `rvtopen::georeference` object) as `metadata.georeference`.
    if let Some(g) = graph.georeference_metadata() {
        meta["georeference"] = g;
    }
    meta["display"] = json!({"rooms": display.rooms, "edges": false});
    let metadata = fb.string(&meta.to_string());
    let guid = fb.string(&graph.id);
    fb.start();
    fb.add_off(0, metadata);
    fb.add_off(1, guids_vec);
    fb.add_off(2, guid_items_vec);
    fb.add_scalar(3, max_local_id.to_le_bytes());
    fb.add_off(4, local_ids);
    fb.add_off(5, cats_vec);
    fb.add_off(6, meshes);
    fb.add_off(7, attrs_vec);
    fb.add_off(8, rels_vec);
    fb.add_off(9, rel_items);
    fb.add_off(10, guid);
    fb.add_off(11, spatial);
    fb.add_off(12, uniq_vec);
    fb.add_off(13, names_vec);
    let root = fb.end();
    let raw = fb.finish(root, b"0001");

    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(&raw).expect("in-memory write");
    Ok((z.finish().expect("in-memory write"), stats))
}

struct Item {
    id: u32,
    category: String,
    attrs: Vec<String>,
    guid: Option<String>,
}

/// `[name, value, IFCTYPE]`, the type inferred as the public importer does
/// for an unwrapped value.
fn attr(name: &str, v: &Value) -> String {
    let ty = match v {
        Value::Bool(_) => "IFCBOOLEAN",
        Value::Number(n) if n.is_i64() || n.is_u64() => "IFCINTEGER",
        Value::Number(_) => "IFCREAL",
        Value::String(_) => "IFCLABEL",
        _ => "UNDEFINED",
    };
    json!([name, v, ty]).to_string()
}

fn rgba(rgb: Option<&Value>, opacity: Option<&Value>) -> [u8; 4] {
    let c = |i: usize| {
        rgb.and_then(|v| v[i].as_f64())
            .map_or(200, |x| (x.clamp(0.0, 1.0) * 255.0).round() as u8)
    };
    let a = opacity
        .and_then(Value::as_f64)
        .map_or(255, |x| (x.clamp(0.0, 1.0) * 255.0).round() as u8);
    [c(0), c(1), c(2), a]
}

fn fv(p: [f32; 3]) -> Vec<u8> {
    p.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// the `Transform` of an IFCX `usd::xformop` (row-major 4x4,
/// translation in the last row, Z-up) in the Y-up frame: X from the first
/// row, Y from the third (IFC +Z is +Y here), position the last row plus the
/// shell's lower corner `lo` (shape frame, Y-up) carried through the rotation,
/// so the stored float points stay small.
fn shared_transform(xf: Option<&Value>, lo: [f64; 3]) -> Vec<u8> {
    let row = |i: usize| -> [f64; 3] {
        let r = xf.and_then(|v| v["transform"].get(i));
        let c = |k: usize| r.and_then(|r| r.get(k)).and_then(Value::as_f64);
        let d = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.0; 3]][i];
        [
            c(0).unwrap_or(d[0]),
            c(1).unwrap_or(d[1]),
            c(2).unwrap_or(d[2]),
        ]
    };
    let y_up = |v: [f64; 3]| [v[0], v[2], -v[1]];
    let f = |v: [f64; 3]| [v[0] as f32, v[1] as f32, v[2] as f32];
    let (x, y, t) = (y_up(row(0)), y_up(row(2)), y_up(row(3)));
    let z = [
        x[1] * y[2] - x[2] * y[1],
        x[2] * y[0] - x[0] * y[2],
        x[0] * y[1] - x[1] * y[0],
    ];
    let position: Vec<f64> = (0..3)
        .map(|k| t[k] + lo[0] * x[k] + lo[1] * y[k] + lo[2] * z[k])
        .collect();
    let mut b: Vec<u8> = position.iter().flat_map(|v| v.to_le_bytes()).collect();
    b.extend(fv(f(x)));
    b.extend(fv(f(y)));
    b
}

/// the `Transform` (position f64 x3, X and Y axes f32 x3) that
/// places `body` relative to `item`: `item⁻¹ · body`, the axes orthonormal.
fn relative_transform(item: &[u8], body: &[u8]) -> Vec<u8> {
    let f64s = |b: &[u8], at: usize| f64::from_le_bytes(b[at..at + 8].try_into().unwrap_or([0; 8]));
    let f32s = |b: &[u8], at: usize| {
        f64::from(f32::from_le_bytes(
            b[at..at + 4].try_into().unwrap_or([0; 4]),
        ))
    };
    let read = |b: &[u8]| {
        let p = [f64s(b, 0), f64s(b, 8), f64s(b, 16)];
        let x = [f32s(b, 24), f32s(b, 28), f32s(b, 32)];
        let y = [f32s(b, 36), f32s(b, 40), f32s(b, 44)];
        let z = [
            x[1] * y[2] - x[2] * y[1],
            x[2] * y[0] - x[0] * y[2],
            x[0] * y[1] - x[1] * y[0],
        ];
        (p, x, y, z)
    };
    let (pi, xi, yi, zi) = read(item);
    let (pb, xb, yb, _) = read(body);
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let into = |v: [f64; 3]| [dot(v, xi), dot(v, yi), dot(v, zi)];
    let position = into([pb[0] - pi[0], pb[1] - pi[1], pb[2] - pi[2]]);
    let f = |v: [f64; 3]| [v[0] as f32, v[1] as f32, v[2] as f32];
    let mut b: Vec<u8> = position.iter().flat_map(|v| v.to_le_bytes()).collect();
    b.extend(fv(f(into(xb))));
    b.extend(fv(f(into(yb))));
    b
}

/// `Transform` at `position`, axes X and Y.
/// an element whose body is a circle profile swept along a
/// straight axis (`SolidShape::PlacedExtrusion` with a circle, a pipe), as
/// (axis start, axis end, radius) in metres, Y up.
fn circle_axis(node: &super::graph::Node) -> Option<([f64; 3], [f64; 3], f64)> {
    use super::entities::{ProfileDef, SolidShape};
    let field = |k: &str| {
        node.attributes
            .get(&format!("{}{k}", super::graph::IFC4_PREFIX))
            .filter(|v| !v.is_null())
    };
    if field("representation_map_index").is_some() {
        return None;
    }
    let shape: SolidShape = serde_json::from_value(field("solid_shape")?.clone()).ok()?;
    let SolidShape::PlacedExtrusion {
        profile: ProfileDef::Circle { radius_feet },
        origin_feet,
        axis,
        depth_feet,
        ..
    } = shape
    else {
        return None;
    };
    let length = axis.iter().map(|v| v * v).sum::<f64>().sqrt();
    if !(length > 1e-12 && radius_feet > 0.0 && depth_feet > 0.0) {
        return None;
    }
    let location: Option<[f64; 3]> =
        field("location_feet").and_then(|v| serde_json::from_value(v.clone()).ok());
    let rotation: Option<f64> = field("rotation_radians").and_then(Value::as_f64);
    let p = super::body_geometry::Placement::new(location, rotation);
    let world = |v: [f64; 3]| {
        let q = p.apply(v);
        [q[0] * 0.3048, q[2] * 0.3048, -q[1] * 0.3048]
    };
    let end: [f64; 3] = std::array::from_fn(|k| origin_feet[k] + axis[k] / length * depth_feet);
    Some((world(origin_feet), world(end), radius_feet * 0.3048))
}

fn transform(position: [f64; 3]) -> Vec<u8> {
    let mut b: Vec<u8> = position.iter().flat_map(|x| x.to_le_bytes()).collect();
    b.extend(fv([1.0, 0.0, 0.0]));
    b.extend(fv([0.0, 1.0, 0.0]));
    b
}

// ------------------------------------------------------- FlatBuffers

/// A FlatBuffers builder, back to front like the reference one: `buf` holds
/// the finished buffer's tail reversed, and an offset is a distance from the
/// end. Tables cannot nest while open; vtables are shared.
#[derive(Default)]
struct Fb {
    buf: Vec<u8>,
    minalign: usize,
    fields: Vec<(usize, u32)>,
    table_start: u32,
    vtables: HashMap<Vec<u8>, u32>,
}

impl Fb {
    fn len(&self) -> u32 {
        u32::try_from(self.buf.len()).expect("a .frag under 4 GiB")
    }
    /// Pad so that `extra` more bytes leave the tail aligned to `size`.
    fn align(&mut self, size: usize, extra: usize) {
        self.minalign = self.minalign.max(size);
        let pad = (size - (self.buf.len() + extra) % size) % size;
        self.buf.resize(self.buf.len() + pad, 0);
    }
    /// Bytes in forward order, prepended.
    fn raw(&mut self, forward: &[u8]) {
        self.buf.extend(forward.iter().rev());
    }
    fn uoff(&mut self, target: u32) {
        self.align(4, 0);
        let v = self.len() + 4 - target;
        self.raw(&v.to_le_bytes());
    }
    fn string(&mut self, s: &str) -> u32 {
        self.align(4, s.len() + 1);
        self.buf.push(0);
        self.raw(s.as_bytes());
        self.raw(&(s.len() as u32).to_le_bytes());
        self.len()
    }
    fn vec_scalars<const N: usize>(&mut self, v: &[[u8; N]]) -> u32 {
        self.align(4, v.len() * N);
        self.align(N.max(1), v.len() * N);
        for x in v.iter().rev() {
            self.raw(x);
        }
        self.raw(&(v.len() as u32).to_le_bytes());
        self.len()
    }
    fn vec_structs(&mut self, v: &[Vec<u8>], align: usize) -> u32 {
        let total: usize = v.iter().map(Vec::len).sum();
        self.align(4, total);
        self.align(align, total);
        for x in v.iter().rev() {
            self.raw(x);
        }
        self.raw(&(v.len() as u32).to_le_bytes());
        self.len()
    }
    fn vec_offsets(&mut self, v: &[u32]) -> u32 {
        self.align(4, v.len() * 4);
        for o in v.iter().rev() {
            self.uoff(*o);
        }
        self.raw(&(v.len() as u32).to_le_bytes());
        self.len()
    }
    fn start(&mut self) {
        self.fields.clear();
        self.table_start = self.len();
    }
    fn add_scalar<const N: usize>(&mut self, id: usize, le: [u8; N]) {
        self.align(N, 0);
        self.raw(&le);
        self.fields.push((id, self.len()));
    }
    fn add_off(&mut self, id: usize, target: u32) {
        self.uoff(target);
        self.fields.push((id, self.len()));
    }
    fn add_struct(&mut self, id: usize, bytes: &[u8], align: usize) {
        self.align(align, 0);
        self.raw(bytes);
        self.fields.push((id, self.len()));
    }
    fn end(&mut self) -> u32 {
        self.align(4, 0);
        self.raw(&[0; 4]);
        let obj = self.len();
        let n = self.fields.iter().map(|(id, _)| id + 1).max().unwrap_or(0);
        let mut slots = vec![0u16; n];
        for (id, loc) in &self.fields {
            slots[*id] = (obj - loc) as u16;
        }
        let mut vt: Vec<u8> = Vec::with_capacity(4 + 2 * n);
        vt.extend((4 + 2 * n as u16).to_le_bytes());
        vt.extend(((obj - self.table_start) as u16).to_le_bytes());
        for s in slots {
            vt.extend(s.to_le_bytes());
        }
        let vt_off = match self.vtables.get(&vt) {
            Some(o) => *o,
            None => {
                self.raw(&vt);
                let o = self.len();
                self.vtables.insert(vt, o);
                o
            }
        };
        let soff = (i64::from(vt_off) - i64::from(obj)) as i32;
        let at = obj as usize - 4;
        for (i, b) in soff.to_le_bytes().iter().rev().enumerate() {
            self.buf[at + i] = *b;
        }
        obj
    }
    fn finish(mut self, root: u32, ident: &[u8; 4]) -> Vec<u8> {
        let a = self.minalign.max(4);
        self.align(a, 8);
        self.raw(ident);
        self.uoff(root);
        self.buf.reverse();
        self.buf
    }
}
