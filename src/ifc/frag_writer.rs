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
    /// Distinct materials: one per colour, and one per textured material
    /// (never merged with another, so the image can be put back on it).
    pub materials: usize,
    /// Materials drawn with an image, each listed in the texture sidecar.
    pub textured_materials: usize,
    /// Samples drawn in a textured material.
    pub textured_samples: usize,
    /// Of those, the samples with texture coordinates for every corner.
    pub textured_samples_with_uvs: usize,
    /// Textured bodies merged into another body of the same item and look,
    /// so that each sample is one run of vertices in a tile.
    pub textured_bodies_merged: usize,
    /// Extra samples made by cutting a textured shell below the public
    /// loader's 65,535-vertex buffer.
    pub textured_shells_split: usize,
    /// Samples with texture coordinates that share an item and a material
    /// with another such sample: a viewer could not tell their runs apart.
    /// 0 by construction; counted to prove it.
    pub textured_duplicate_keys: usize,
    /// Triangle corners whose texture coordinate was missing or not a
    /// number, written as `[0, 0]`.
    pub textured_corners_defaulted: usize,
    /// Textured samples left out of the sidecar because their material had
    /// no image to list.
    pub textured_samples_skipped: usize,
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
    write_frag(graph, display, false).map(|(bytes, stats, _)| (bytes, stats))
}

/// [`write_frag_graph_with`], and the textures the format cannot hold, as
/// a sidecar to write beside the `.frag` (`<name>.frag.textures.json`) and
/// the images it names (paths relative to the sidecar), once each. `None`
/// when no sample is drawn in a textured material; the `.frag` is then
/// byte-identical to [`write_frag_graph_with`]'s. The sidecar's schema is
/// [`TEXTURE_SIDECAR_FORMAT`] version [`TEXTURE_SIDECAR_VERSION`]
/// (`docs/frag-textures.md`).
pub fn write_frag_graph_textured(
    graph: &IfcxGraph,
    display: &super::gltf::DisplayOptions,
) -> Result<(Vec<u8>, FragStats, Option<FragTextures>), IfcxError> {
    write_frag(graph, display, true)
}

/// The `format` of the texture sidecar.
pub const TEXTURE_SIDECAR_FORMAT: &str = "rvt-open/frag-textures";
/// The sidecar schema version. It changes whenever a reader written for
/// the previous version would misread the file, so a reader refuses any
/// version but the ones it knows.
pub const TEXTURE_SIDECAR_VERSION: u32 = 1;

/// Triangles per textured shell at most: 3 vertices each stays under the
/// 65,535-vertex buffer the public loader fills per shell (it starts a new
/// buffer, and so a new run of vertices, past that), and the shell's points
/// under the 65,000 that make it `BIG`.
pub const TEXTURED_SHELL_TRIANGLES: usize = 21_666;

/// A `.frag`'s texture sidecar (`<name>.frag.textures.json`, schema in
/// `docs/frag-textures.md`) and the images it names.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FragTextures {
    /// [`TEXTURE_SIDECAR_FORMAT`].
    pub format: &'static str,
    /// [`TEXTURE_SIDECAR_VERSION`].
    pub version: u32,
    /// The Fragments schema the `.frag` follows.
    pub fragments_schema: &'static str,
    /// The `.frag`'s own model id (its `guid` field: the IFCX header id).
    pub model: String,
    /// SHA-256 of the `.frag` file's bytes, lower-case hex: the file the
    /// sidecar belongs to.
    pub frag_sha256: String,
    /// One entry per textured `.frag` material.
    pub materials: Vec<SidecarMaterial>,
    /// One entry per sample drawn in a textured material.
    pub samples: Vec<SidecarSample>,
    /// The images: path relative to the sidecar, and bytes.
    #[serde(skip)]
    pub images: Vec<super::TextureFile>,
}

impl FragTextures {
    /// The sidecar as JSON, written to `w` as it is made.
    pub fn write_json(&self, w: impl Write) -> serde_json::Result<()> {
        serde_json::to_writer(w, self)
    }
}

/// A textured `.frag` material in the sidecar.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SidecarMaterial {
    /// The material's local id in the `.frag`.
    pub material: u32,
    /// Its index in `Meshes.materials`.
    pub index: usize,
    /// 0, or the piece of a textured shell cut below the loader's buffer
    /// (each piece has a material of its own; same image).
    pub part: u32,
    /// The source material's name.
    pub name: String,
    /// The image's path, relative to the sidecar.
    pub image: String,
    /// `image/png` or `image/jpeg`.
    pub mime_type: String,
    /// The image's file name in the source, where it names one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_name: Option<String>,
    /// Whether the image can hold transparency.
    pub image_has_alpha: bool,
    /// The `.frag` material's `[r, g, b, a]`.
    pub color: [u8; 4],
    /// As the `.frag` material.
    pub double_sided: bool,
    /// What one repeat of the image covers, feet, as the source gives it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeat_size_feet: Option<[f64; 2]>,
    /// The source's placement record, with absolute paths cut to file names.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mapping: Option<Value>,
}

/// A `.frag` sample drawn in a textured material.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SidecarSample {
    /// The item's local id.
    pub item: u32,
    /// The item's index in `Meshes.meshesItems`.
    pub mesh_item: u32,
    /// The sample's index in `Meshes.samples`.
    pub sample: u32,
    /// The textured material's local id.
    pub material: u32,
    /// Triangles of the sample's shell, where `uv` is given.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub triangles: Option<usize>,
    /// One `u, v` per triangle corner, in profile order, `v` up; single
    /// precision, as the `.frag`'s points.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uv: Option<Vec<f32>>,
    /// Where the sample has no texture coordinates: a box projection.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub projection: Option<Projection>,
}

/// A box projection for a sample without texture coordinates.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Projection {
    /// `box`.
    pub kind: &'static str,
    /// Metres one repeat covers.
    pub repeat_size_metres: [f64; 2],
}

/// `sidecar` off: the same `.frag` and counts, with no sidecar made (no
/// image copies, coordinates or hash).
fn write_frag(
    graph: &IfcxGraph,
    display: &super::gltf::DisplayOptions,
    sidecar: bool,
) -> Result<(Vec<u8>, FragStats, Option<FragTextures>), IfcxError> {
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
    // (item, mesh, colour and textured material, Some((shared shape path,
    // Body xformop))).
    #[allow(clippy::type_complexity)]
    let mut geometry: Vec<(
        usize,
        &super::graph::UsdMesh,
        Look,
        Option<(&str, Option<&Value>)>,
    )> = Vec::new();
    // bodies that are a circle swept along a straight axis (a
    // pipe), written as fragments' own CIRCLE_EXTRUSION: (item, axis start
    // and end in metres Y-up, radius in metres, colour).
    #[allow(clippy::type_complexity)]
    let mut circles: Vec<(usize, [f64; 3], [f64; 3], f64, Look)> = Vec::new();
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
                // a textured body keeps its own points (never a shared shape).
                let texture = body.mesh.as_deref().and_then(|m| m.texture);
                let colour = (colour, texture.map(|t| (t, 0)));
                if let Some((p1, p2, r)) = circle {
                    if !circles.iter().any(|c| c.0 == index) {
                        // a circle extrusion has no texture coordinates.
                        let colour = (colour.0, colour.1.map(|(t, _)| (t, PROJECTED)));
                        circles.push((index, p1, p2, r, colour));
                    }
                } else if let Some(mesh) = body.mesh.as_deref() {
                    geometry.push((index, mesh, colour, None));
                } else if let Some((shape, mesh)) = body.inherits.iter().find_map(|t| {
                    let m = graph.nodes.get(t)?.mesh.as_deref()?;
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
    // an item's textured bodies of one look become one shell, cut below the
    // public loader's buffer, so each sample is one run of a tile's
    // vertices that its texture coordinates line up with.
    let merged = merge_textured(&mut geometry, &mut stats);
    geometry.extend(
        merged
            .iter()
            .map(|(item, look, mesh)| (*item, mesh, *look, None)),
    );
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
    let mut materials: Vec<Look> = Vec::new();
    // samples drawn in a textured material: (item, mesh item, sample,
    // material, the mesh when its texture coordinates are the sample's).
    let mut textured: Vec<(usize, u32, u32, u32, Option<&super::graph::UsdMesh>)> = Vec::new();
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
            let pts: Vec<[f64; 3]> = mesh.finite_points().map(|c| [c[0], c[2], -c[1]]).collect();
            let idx: Vec<u32> = mesh.face_vertex_indices.clone();
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
        if colour.1.is_some() {
            let own = reuse.is_none() && mesh.st.len() == mesh.points.len();
            textured.push((
                *item,
                sample_item,
                samples.len() as u32,
                material,
                own.then_some(*mesh),
            ));
        }
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
        if colour.1.is_some() {
            textured.push((*item, sample_item, samples.len() as u32, material, None));
        }
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
        .map(|(m, _)| vec![m[0], m[1], m[2], m[3], u8::from(m[3] < 255), 0])
        .collect();
    let mats_vec = fb.vec_structs(&mats, 1);
    let ext_vec = fb.vec_offsets(&circle_tables);
    let items_vec = fb.vec_scalars(
        &meshes_items
            .iter()
            .map(|v| v.to_le_bytes())
            .collect::<Vec<_>>(),
    );
    let mat_id_values: Vec<u32> = (0..materials.len()).map(|_| fresh()).collect();
    let mat_ids = fb.vec_scalars(
        &mat_id_values
            .iter()
            .map(|v| v.to_le_bytes())
            .collect::<Vec<_>>(),
    );
    let mut ids = |fb: &mut Fb, n: usize| {
        let v: Vec<[u8; 4]> = (0..n).map(|_| fresh().to_le_bytes()).collect();
        fb.vec_scalars(&v)
    };
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
    let frag = z.finish().expect("in-memory write");
    let sidecar = texture_sidecar(
        graph,
        &frag,
        &materials,
        &mat_id_values,
        &textured,
        &items,
        &mut stats,
        sidecar,
    );
    Ok((frag, stats, sidecar))
}

/// The texture part of a look for a sample without texture coordinates.
const PROJECTED: u32 = u32::MAX;

/// Takes the unshared textured meshes out of `geometry` and returns them
/// merged per item and look (texture coordinates or none), each merged mesh
/// cut into pieces of at most [`TEXTURED_SHELL_TRIANGLES`] triangles, piece
/// `k` in look part `k` (a material of its own); meshes without texture
/// coordinates in part [`PROJECTED`].
#[allow(clippy::type_complexity)]
fn merge_textured(
    geometry: &mut Vec<(
        usize,
        &super::graph::UsdMesh,
        Look,
        Option<(&str, Option<&Value>)>,
    )>,
    stats: &mut FragStats,
) -> Vec<(usize, Look, super::graph::UsdMesh)> {
    use super::graph::UsdMesh;
    type Key = (usize, Look, bool);
    let mut order: Vec<Key> = Vec::new();
    let mut groups: HashMap<Key, Vec<&UsdMesh>> = HashMap::new();
    geometry.retain(|(item, mesh, look, shared)| {
        if look.1.is_none() || shared.is_some() {
            return true;
        }
        let key = (*item, *look, mesh.st.len() == mesh.points.len());
        let list = groups.entry(key).or_insert_with(|| {
            order.push(key);
            Vec::new()
        });
        list.push(*mesh);
        false
    });
    let mut out = Vec::new();
    for key in order {
        let (item, look, with_st) = key;
        let meshes = &groups[&key];
        stats.textured_bodies_merged += meshes.len() - 1;
        let mut whole = UsdMesh {
            texture: look.1.map(|(t, _)| t),
            ..Default::default()
        };
        for m in meshes {
            let base = whole.points.len() as u32;
            whole.points.extend_from_slice(&m.points);
            if with_st {
                whole.st.extend_from_slice(&m.st);
            }
            whole.face_vertex_indices.extend(
                m.face_vertex_indices
                    .chunks_exact(3)
                    .flatten()
                    .map(|i| i + base),
            );
        }
        if !with_st {
            out.push((item, (look.0, look.1.map(|(t, _)| (t, PROJECTED))), whole));
            continue;
        }
        let triangles = whole.face_vertex_indices.len() / 3;
        if triangles <= TEXTURED_SHELL_TRIANGLES {
            out.push((item, look, whole));
            continue;
        }
        let pieces = whole
            .face_vertex_indices
            .chunks(3 * TEXTURED_SHELL_TRIANGLES)
            .enumerate();
        for (part, chunk) in pieces {
            let mut piece = UsdMesh {
                texture: whole.texture,
                ..Default::default()
            };
            let mut new_index: HashMap<u32, u32> = HashMap::new();
            for &i in chunk {
                let n = *new_index.entry(i).or_insert_with(|| {
                    let p = i as usize;
                    piece
                        .points
                        .push(whole.points.get(p).copied().unwrap_or([0.0; 3]));
                    piece
                        .st
                        .push(whole.st.get(p).copied().unwrap_or([f64::NAN; 2]));
                    (piece.points.len() - 1) as u32
                });
                piece.face_vertex_indices.push(n);
            }
            stats.textured_shells_split += usize::from(part > 0);
            out.push((item, (look.0, look.1.map(|(t, _)| (t, part as u32))), piece));
        }
    }
    out
}

/// `v` with every string that looks like an absolute path (`/...`,
/// `C:\...`, `\\host\...`) cut to its file name: a source's placement
/// record must not carry the author's folders into the sidecar.
fn strip_paths(v: &Value) -> Value {
    match v {
        Value::String(s) => {
            let b = s.as_bytes();
            let absolute = s.starts_with('/')
                || s.starts_with("\\\\")
                || (b.len() > 2
                    && b[0].is_ascii_alphabetic()
                    && b[1] == b':'
                    && matches!(b[2], b'\\' | b'/'));
            if absolute {
                Value::String(s.rsplit(['/', '\\']).next().unwrap_or("").to_string())
            } else {
                v.clone()
            }
        }
        Value::Array(a) => Value::Array(a.iter().map(strip_paths).collect()),
        Value::Object(o) => {
            Value::Object(o.iter().map(|(k, x)| (k.clone(), strip_paths(x))).collect())
        }
        _ => v.clone(),
    }
}

/// A sample's look: its RGBA colour, and for a textured sample the model's
/// index of its material and the piece (see [`merge_textured`]). Samples of
/// one look share a `.frag` material.
type Look = ([u8; 4], Option<(usize, u32)>);

/// The texture sidecar (see [`write_frag_graph_textured`]); `None` without
/// textured samples.
#[allow(clippy::type_complexity)]
fn texture_sidecar(
    graph: &IfcxGraph,
    frag: &[u8],
    materials: &[Look],
    material_ids: &[u32],
    textured: &[(usize, u32, u32, u32, Option<&super::graph::UsdMesh>)],
    items: &[Item],
    stats: &mut FragStats,
    keep: bool,
) -> Option<FragTextures> {
    use sha2::Digest;
    if textured.is_empty() {
        return None;
    }
    let mut images: Vec<super::TextureFile> = Vec::new();
    // the image of each model material, named once: its hash name, or with
    // a suffix where two different images hash alike.
    let mut image_of: HashMap<usize, String> = HashMap::new();
    let mut entries = Vec::new();
    // metres one repeat covers, per listed .frag material.
    let mut repeat: HashMap<u32, Option<[f64; 2]>> = HashMap::new();
    for (index, ((colour, look), id)) in materials.iter().zip(material_ids).enumerate() {
        let Some((m, part)) = *look else {
            continue;
        };
        let Some(info) = graph.material(m) else {
            continue;
        };
        let Some(texture) = info.texture.as_ref() else {
            continue;
        };
        let file = if !keep {
            String::new()
        } else {
            image_of
                .entry(m)
                .or_insert_with(|| {
                    let mut name = format!("textures/{}", texture.file_name());
                    let mut n = 0;
                    loop {
                        match images.iter().find(|(f, _)| *f == name) {
                            None => {
                                images.push((name.clone(), texture.image.clone()));
                                break name;
                            }
                            Some((_, bytes)) if *bytes == texture.image => break name,
                            Some(_) => {
                                n += 1;
                                let stem = texture.file_name();
                                let stem = stem
                                    .rsplit_once('.')
                                    .map_or(stem.as_str(), |(s, _)| s)
                                    .to_string();
                                name = format!("textures/{stem}-{n}.{}", texture.extension());
                            }
                        }
                    }
                })
                .clone()
        };
        repeat.insert(index as u32, texture.size_feet);
        entries.push(SidecarMaterial {
            material: *id,
            index,
            part: if part == PROJECTED { 0 } else { part },
            name: info.name.clone(),
            image: file,
            mime_type: texture.mime_type.clone(),
            image_name: texture
                .name
                .as_deref()
                .map(|n| n.rsplit(['/', '\\']).next().unwrap_or(n).to_string()),
            image_has_alpha: texture.has_alpha(),
            color: *colour,
            double_sided: colour[3] < 255,
            repeat_size_feet: texture.size_feet,
            mapping: texture.mapping.as_ref().map(strip_paths),
        });
    }
    stats.textured_materials = entries.len();
    let mut samples = Vec::new();
    let mut keys = std::collections::HashSet::new();
    let mut counted = 0;
    for (item, mesh_item, sample, material, mesh) in textured {
        if !repeat.contains_key(material) {
            stats.textured_samples_skipped += 1;
            continue;
        }
        // one [u, v] per triangle corner, in the order the shell's profiles
        // list them: the order the public loader lays out a shell's vertices.
        // (counted, not kept, without `keep`)
        let uv: Option<Vec<f32>> = mesh.map(|m| {
            let mut uv = Vec::with_capacity(if keep {
                m.face_vertex_indices.len() / 3 * 6
            } else {
                0
            });
            for i in m.face_vertex_indices.chunks_exact(3).flatten() {
                let c = m.st.get(*i as usize).copied().unwrap_or([f64::NAN; 2]);
                if !c.iter().all(|x| x.is_finite()) {
                    stats.textured_corners_defaulted += 1;
                }
                if keep {
                    uv.extend(c.map(|x| if x.is_finite() { x as f32 } else { 0.0 }));
                }
            }
            uv
        });
        let projection = if uv.is_none() {
            let size = repeat[material].map_or([1.0; 2], |s| s.map(|f| f * 0.3048));
            Some(Projection {
                kind: "box",
                repeat_size_metres: size,
            })
        } else {
            if !keys.insert((*mesh_item, *material)) {
                stats.textured_duplicate_keys += 1;
            }
            stats.textured_samples_with_uvs += 1;
            None
        };
        if !keep {
            counted += 1;
            continue;
        }
        samples.push(SidecarSample {
            item: items[*item].id,
            mesh_item: *mesh_item,
            sample: *sample,
            material: material_ids[*material as usize],
            triangles: uv.as_ref().map(|u| u.len() / 6),
            uv,
            projection,
        });
    }
    stats.textured_samples = samples.len() + counted;
    if !keep {
        return None;
    }
    let digest = sha2::Sha256::digest(frag);
    Some(FragTextures {
        format: TEXTURE_SIDECAR_FORMAT,
        version: TEXTURE_SIDECAR_VERSION,
        fragments_schema: FRAGMENTS_SCHEMA_VERSION,
        model: graph.id.clone(),
        frag_sha256: digest.iter().map(|b| format!("{b:02x}")).collect(),
        materials: entries,
        samples,
        images,
    })
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
    let SolidShape::PlacedExtrusion {
        profile: ProfileDef::Circle { radius_feet },
        origin_feet,
        axis,
        depth_feet,
        ..
    } = node.solid_shape.as_deref()?
    else {
        return None;
    };
    let (radius_feet, origin_feet, axis, depth_feet) =
        (*radius_feet, *origin_feet, *axis, *depth_feet);
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
