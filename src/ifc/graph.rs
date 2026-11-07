//! The IFCX-shaped graph that is rvt-open's model.
//!
//! [`IfcxGraph`] is nodes with a `path`, `children`, `inherits` and an
//! attribute map keyed by namespaced strings, as IFCX has them. Two families
//! of keys live in one map:
//!
//! - the IFCX-facing keys (`bsi::ifc::class`, `bsi::ifc::prop::*`,
//!   `ifc-src::attr::*`, `ifc-src::pset::*`, `rvtopen::revit::*`,
//!   `usd::usdgeom::mesh`, ...), which [`super::ifcx_writer`] serializes;
//! - the IFC4 keys `rvtopen::ifc4::*`, one per field of what the IFC4 STEP
//!   writer needs and IFCX has no home for (IFC types, geometry chains,
//!   relationships, tables). Index-valued fields are node paths. IFCX output
//!   leaves them out.
//!
//! [`IfcxGraph::to_model`] derives the [`IfcModel`] view the IFC4 writer
//! reads, so `write_step_graph` is a function of the graph.

use super::IfcModel;
use super::body_geometry::{Placement, body_mesh};
use super::entities::{IfcEntity, PropertySet, PropertyValue};
use super::export_content::{
    FAMILY_NAME_PROPERTY, ROOM_NAME_PROPERTY, ROOM_NUMBER_PROPERTY, TYPE_NAME_PROPERTY,
};
use super::gltf::category_colour;
use super::ifcx_writer::{
    FT, IfcxError, LossLedger, Ty, as_v5a, child_key, coerce, ifc_guid_uuid, property_key, uuid5,
    v5a_prop, value_of,
};
use super::step_writer::{
    building_element_global_ids, generated_global_id, global_id_seed, project_level_global_id,
    type_object_plan,
};
use crate::georeference::Georeference;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// `rvtopen::ifc::psetOf::<Prop>` names the standard property sets
/// (`Pset_*`, `Qto_*`) a flattened `bsi::ifc::prop::<Prop>` came from.
pub const PSET_OF: &str = "rvtopen::ifc::psetOf::";

/// the site's georeference (Revit's location, shared coordinates,
/// survey and project base points, CRS) as one object on the `IfcSite` node.
pub const GEOREFERENCE: &str = "rvtopen::georeference";
/// set on the IFCX node of a type no element is typed by.
pub const UNPLACED_TYPE: &str = "rvtopen::revit::UnplacedType";

/// a reference point's position in Revit's shared coordinates.
pub const SHARED_COORDINATES: &str = "rvtopen::revit::sharedCoordinates";

/// The prefix of the IFC4-only keys, which IFCX output leaves out.
pub const IFC4_PREFIX: &str = "rvtopen::ifc4::";

/// What to derive when building the graph.
#[derive(Debug, Clone, Copy)]
pub struct GraphOptions {
    /// Draw each element's `usd::usdgeom::mesh` `Body` child. Needed for
    /// IFCX only; the IFC4 derivation does not read it.
    pub meshes: bool,
    /// write each distinct element-local mesh once, as a node the
    /// element's `Body` inherits, with the element placement as the Body's
    /// `usd::xformop`. Off: one world-coordinate mesh per element.
    pub shared_shapes: bool,
}

impl Default for GraphOptions {
    fn default() -> Self {
        Self {
            meshes: true,
            shared_shapes: true,
        }
    }
}

/// A shared shape's points (metres, on a 1e-7 m grid, relative to their
/// lower corner), its key, and that corner (metres, element-local): two
/// element-local meshes share a shape when they are equal on the grid up to
/// a mesh's triangles grouped by face material (first appearance
/// order), each group with only the vertices it uses; a mesh without face
/// materials, or with one, is one part. `None`: the element's own colour.
pub(crate) fn split_by_face_material(
    mesh: super::body_geometry::Mesh,
) -> Vec<(super::body_geometry::Mesh, Option<usize>)> {
    use super::body_geometry::NO_FACE_MATERIAL;
    let some = |m: usize| (m != NO_FACE_MATERIAL).then_some(m);
    let first = mesh.face_materials.first().copied();
    if mesh.face_materials.len() != mesh.triangles.len()
        || mesh.face_materials.iter().all(|m| Some(*m) == first)
    {
        let material = first.and_then(some);
        return vec![(mesh, material)];
    }
    let mut order: Vec<usize> = Vec::new();
    for m in &mesh.face_materials {
        if !order.contains(m) {
            order.push(*m);
        }
    }
    order
        .into_iter()
        .map(|m| {
            let mut remap: HashMap<u32, u32> = HashMap::new();
            let mut part = super::body_geometry::Mesh::default();
            for (t, _) in mesh
                .triangles
                .iter()
                .zip(&mesh.face_materials)
                .filter(|(_, f)| **f == m)
            {
                let t = t.map(|v| {
                    *remap.entry(v).or_insert_with(|| {
                        part.vertices.push(mesh.vertices[v as usize]);
                        (part.vertices.len() - 1) as u32
                    })
                });
                part.triangles.push(t);
            }
            (part, some(m))
        })
        .collect()
}

/// a translation, and the points stay small for float readers.
fn shared_points(mesh: &super::body_geometry::Mesh) -> (Vec<Value>, String, [f64; 3]) {
    let (q, _) = super::body_geometry::mesh_key(mesh, FT);
    let mut lo = [i64::MAX; 3];
    for p in &q {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
        }
    }
    let rel: Vec<[i64; 3]> = q
        .iter()
        .map(|p| [p[0] - lo[0], p[1] - lo[1], p[2] - lo[2]])
        .collect();
    let local = super::body_geometry::Mesh {
        vertices: rel.iter().map(|p| p.map(|c| c as f64)).collect(),
        triangles: mesh.triangles.clone(),
        face_materials: Vec::new(),
    };
    let (_, key) = super::body_geometry::mesh_key(&local, 1e-7);
    let points = rel
        .iter()
        .map(|p| json!(p.map(|c| c as f64 / 1e7)))
        .collect();
    (points, key, lo.map(|c| c as f64 / 1e7))
}

/// One node of the graph.
#[derive(Debug, Clone, Default)]
pub struct Node {
    /// Child name to child path.
    pub children: BTreeMap<String, String>,
    /// Paths of the nodes this one inherits from (IFCX `inherits`).
    pub inherits: Vec<String>,
    /// Namespaced attribute key to value.
    pub attributes: Map<String, Value>,
    /// Part of the IFCX tree. Nodes that exist only for the IFC4 derivation
    /// (relationships, tables) are not written to IFCX.
    pub ifcx: bool,
}

/// Node paths in the order of the `IfcModel` list each came from.
#[derive(Debug, Clone, Default)]
pub struct Order {
    pub entities: Vec<String>,
    pub storeys: Vec<String>,
    pub materials: Vec<String>,
    pub layer_sets: Vec<String>,
    pub profile_sets: Vec<String>,
    pub representation_maps: Vec<String>,
    pub constituent_sets: Vec<String>,
}

/// The model: nodes, the IFCX schemas of their keys, and the order of the
/// lists the IFC4 writer walks.
#[derive(Debug, Clone, Default)]
pub struct IfcxGraph {
    /// The root node's path.
    pub root: String,
    /// The project node's path (it carries the model-wide IFC4 facts).
    pub project: String,
    /// IFCX header id.
    pub id: String,
    pub nodes: BTreeMap<String, Node>,
    /// IFCX key to its one type.
    pub(super) schemas: BTreeMap<String, Ty>,
    pub order: Order,
    pub ledger: LossLedger,
    /// The type nodes: the IFC4 writer's type objects, an
    /// unplaced type, which nothing inherits, included. OpenUSD writes each
    /// as a `class` prim.
    pub type_nodes: BTreeSet<String>,
    /// Why IFCX cannot be written (a GlobalId that is no IFC id, two
    /// elements on one path). The IFC4 derivation
    /// does not need these ids and ignores them.
    pub errors: Vec<String>,
}

/// Tables an index can point into.
#[derive(Clone, Copy)]
enum Tbl {
    Entity,
    Storey,
    Material,
    LayerSet,
    ProfileSet,
    RepMap,
}

type RefSpec = &'static [(&'static [&'static str], Tbl)];

fn entity_refs(kind: &str) -> RefSpec {
    use Tbl::*;
    match kind {
        "BuildingElement" => &[
            (&["storey_index"], Storey),
            (&["material_index"], Material),
            (&["material_layer_set_index"], LayerSet),
            (&["material_profile_set_index"], ProfileSet),
            (&["host_element_index"], Entity),
            (&["representation_map_index"], RepMap),
        ],
        "Aggregate" => &[(&["whole"], Entity), (&["parts"], Entity)],
        "ElementPropertySet" => &[(&["element"], Entity)],
        "PortConnection" => &[(&["a"], Entity), (&["b"], Entity)],
        "Port" => &[(&["element"], Entity)],
        "System" | "DesignOption" | "Zone" => &[(&["members"], Entity)],
        "VoidOpening" | "WallProfileOpening" => &[(&["host"], Entity)],
        "SpaceContainment" => &[(&["space"], Entity), (&["elements"], Entity)],
        _ => &[],
    }
}

const LAYER_SET_REFS: RefSpec = &[(&["layers", "[]", "material_index"], Tbl::Material)];
const PROFILE_SET_REFS: RefSpec = &[(&["profiles", "[]", "material_index"], Tbl::Material)];
const CONSTITUENT_REFS: RefSpec = &[
    (&["material_indices"], Tbl::Material),
    (&["elements"], Tbl::Entity),
];

/// Node paths of every table, and the reverse lookup.
struct Paths {
    entity: Vec<String>,
    storey: Vec<String>,
    material: Vec<String>,
    layer_set: Vec<String>,
    profile_set: Vec<String>,
    rep_map: Vec<String>,
}

impl Paths {
    fn table(&self, t: Tbl) -> &[String] {
        match t {
            Tbl::Entity => &self.entity,
            Tbl::Storey => &self.storey,
            Tbl::Material => &self.material,
            Tbl::LayerSet => &self.layer_set,
            Tbl::ProfileSet => &self.profile_set,
            Tbl::RepMap => &self.rep_map,
        }
    }
}

/// Apply `f` to the values `spec` names inside `v` (`[]` = every element).
fn visit(v: &mut Value, spec: &[&str], f: &mut dyn FnMut(&mut Value)) {
    match spec {
        [] => f(v),
        ["[]", rest @ ..] => {
            if let Value::Array(items) = v {
                for item in items {
                    visit(item, rest, f);
                }
            }
        }
        [key, rest @ ..] => {
            if let Some(child) = v.get_mut(*key) {
                visit(child, rest, f);
            }
        }
    }
}

/// Indices in the named fields become node paths (an index no table holds
/// stays a number).
fn encode_refs(v: &mut Value, spec: RefSpec, paths: &Paths) {
    for (field, table) in spec {
        let list = paths.table(*table);
        visit(v, field, &mut |x| match x {
            Value::Array(items) => items.iter_mut().for_each(|i| one_to_path(i, list)),
            other => one_to_path(other, list),
        });
    }
}

fn one_to_path(v: &mut Value, list: &[String]) {
    if let Some(p) = v.as_u64().and_then(|n| list.get(n as usize)) {
        *v = Value::String(p.clone());
    }
}

/// The inverse of [`encode_refs`].
fn decode_refs(v: &mut Value, spec: RefSpec, index: &HashMap<String, usize>) {
    for (field, _) in spec {
        visit(v, field, &mut |x| match x {
            Value::Array(items) => items.iter_mut().for_each(|i| one_to_index(i, index)),
            other => one_to_index(other, index),
        });
    }
}

fn one_to_index(v: &mut Value, index: &HashMap<String, usize>) {
    if let Some(i) = v.as_str().and_then(|s| index.get(s)) {
        *v = json!(i);
    }
}

fn de<T: serde::de::DeserializeOwned>(v: Value) -> T {
    serde_json::from_value(v).expect("graph values deserialize")
}

fn to_object<T: serde::Serialize>(t: &T) -> Map<String, Value> {
    match serde_json::to_value(t).expect("model values serialize") {
        Value::Object(m) => m,
        other => panic!("expected an object, got {other}"),
    }
}

impl IfcxGraph {
    /// the site's georeference as one object for file-level
    /// metadata (USD `customLayerData`, the `.frag` model metadata): latitude,
    /// longitude (degrees) and elevation (metres) from the site, then the
    /// `rvtopen::georeference` object's fields. `None` without one.
    pub fn georeference_metadata(&self) -> Option<Value> {
        let site = self
            .nodes
            .values()
            .find(|n| n.attributes.contains_key(GEOREFERENCE))?;
        let mut g = Map::new();
        for (k, name) in [
            ("ifc-src::attr::RefLatitude", "latitude"),
            ("ifc-src::attr::RefLongitude", "longitude"),
            ("ifc-src::attr::RefElevation", "elevation"),
        ] {
            if let Some(x) = site.attributes.get(k) {
                g.insert(name.into(), x.clone());
            }
        }
        if let Some(Value::Object(o)) = site.attributes.get(GEOREFERENCE) {
            g.extend(o.clone());
        }
        Some(Value::Object(g))
    }

    /// Build the graph of a model. Errors that only IFCX cares about are
    /// kept in [`IfcxGraph::errors`].
    pub fn from_model(model: &IfcModel, options: GraphOptions) -> IfcxGraph {
        let mut b = Builder::default();
        b.build(model, options);
        b.finish(model)
    }

    /// Fields of node `path` under the IFC4 prefix, as a JSON object.
    fn ifc4_object(&self, path: &str, skip: &[&str]) -> Value {
        let mut m = Map::new();
        if let Some(n) = self.nodes.get(path) {
            for (k, v) in &n.attributes {
                if let Some(f) = k.strip_prefix(IFC4_PREFIX) {
                    if !skip.contains(&f) {
                        m.insert(f.to_string(), v.clone());
                    }
                }
            }
        }
        Value::Object(m)
    }

    fn ifc4<'a>(&'a self, path: &str, key: &str) -> Option<&'a Value> {
        self.nodes
            .get(path)?
            .attributes
            .get(&format!("{IFC4_PREFIX}{key}"))
    }

    /// The model view the IFC4 writer reads, derived from the graph.
    pub fn to_model(&self) -> IfcModel {
        let mut index: HashMap<String, usize> = HashMap::new();
        for list in [
            &self.order.entities,
            &self.order.storeys,
            &self.order.materials,
            &self.order.layer_sets,
            &self.order.profile_sets,
            &self.order.representation_maps,
        ] {
            for (i, p) in list.iter().enumerate() {
                index.insert(p.clone(), i);
            }
        }
        let meta = self
            .ifc4(&self.project, "model")
            .cloned()
            .unwrap_or(Value::Null);
        let from_meta = |k: &str| meta.get(k).cloned().unwrap_or(Value::Null);
        let string_of = |p: &str, k: &str| -> Option<String> {
            self.ifc4(p, k).and_then(|v| v.as_str()).map(str::to_string)
        };
        let mut model = IfcModel {
            project_name: string_of(&self.project, "projectName"),
            description: string_of(&self.project, "description"),
            site_name: string_of(&self.project, "siteName"),
            building_name: string_of(&self.project, "buildingName"),
            units: de(from_meta("units")),
            classifications: de(from_meta("classifications")),
            element_layers: de(from_meta("element_layers")),
            element_type_materials: de(from_meta("element_type_materials")),
            element_type_ids: de(from_meta("element_type_ids")),
            convention: de(from_meta("convention")),
            file_types: de(from_meta("file_types")),
            type_property_sets: de(from_meta("type_property_sets")),
            type_entity_sets: de(from_meta("type_entity_sets")),
            body_pieces: de(from_meta("body_pieces")),
            georeference: de(from_meta("georeference")),
            ..IfcModel::default()
        };
        model.global_ids.document = de(from_meta("global_ids_document"));
        model.global_ids.history = de(from_meta("global_ids_history"));
        model.global_ids.guid_namespace = de(from_meta("global_ids_guid_namespace"));
        model.global_ids.types = de(from_meta("global_ids_types"));
        model.global_ids.element_types = de(from_meta("global_ids_element_types"));
        model.global_ids.revit_version = de(from_meta("global_ids_revit_version"));
        model.global_ids.project = de(from_meta("global_ids_project"));
        model.global_ids.site = de(from_meta("global_ids_site"));
        model.global_ids.building = de(from_meta("global_ids_building"));

        for (i, p) in self.order.entities.iter().enumerate() {
            let mut obj = self.ifc4_object(
                p,
                &[
                    "revitGlobalId",
                    "openingCut",
                    "fillerSize",
                    "wallLocationLine",
                    "layerSetUsage",
                    "mapTarget",
                ],
            );
            let kind = obj
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            decode_refs(&mut obj, entity_refs(&kind), &index);
            model.entities.push(de(obj));
            if let Some(g) = string_of(p, "revitGlobalId") {
                model.global_ids.elements.insert(i, g);
            }
            if let Some(v) = self.ifc4(p, "openingCut") {
                model.opening_cuts.insert(i, de(v.clone()));
            }
            if let Some(v) = self.ifc4(p, "fillerSize") {
                model.filler_sizes.insert(i, de(v.clone()));
            }
            if let Some(v) = self.ifc4(p, "wallLocationLine") {
                model.wall_location_lines.insert(i, de(v.clone()));
            }
            if let Some(v) = self.ifc4(p, "mapTarget") {
                model.instancing.targets.insert(i, de(v.clone()));
            }
            if let Some(v) = self.ifc4(p, "layerSetUsage") {
                model.material_layer_usages.insert(i, de(v.clone()));
            }
        }
        for (i, p) in self.order.storeys.iter().enumerate() {
            model.building_storeys.push(entities_storey(self, p));
            if let Some(g) = string_of(p, "revitGlobalId") {
                model.global_ids.storeys.insert(i, g);
            }
            if let Some(v) = self.ifc4(p, "storeyProperties") {
                model.storey_properties.insert(i, de(v.clone()));
            }
        }
        for p in &self.order.materials {
            model.materials.push(de(self.ifc4_object(p, &[])));
        }
        for p in &self.order.layer_sets {
            let mut o = self.ifc4_object(p, &[]);
            decode_refs(&mut o, LAYER_SET_REFS, &index);
            model.material_layer_sets.push(de(o));
        }
        for p in &self.order.profile_sets {
            let mut o = self.ifc4_object(p, &[]);
            decode_refs(&mut o, PROFILE_SET_REFS, &index);
            model.material_profile_sets.push(de(o));
        }
        for p in &self.order.representation_maps {
            model.representation_maps.push(de(self.ifc4_object(p, &[])));
        }
        for p in &self.order.constituent_sets {
            let mut o = self.ifc4_object(p, &[]);
            decode_refs(&mut o, CONSTITUENT_REFS, &index);
            model.material_constituent_sets.push(de(o));
        }
        model
    }
}

fn entities_storey(g: &IfcxGraph, p: &str) -> super::Storey {
    super::Storey {
        name: g
            .ifc4(p, "name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        elevation_feet: g
            .ifc4(p, "elevation_feet")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
    }
}

// ------------------------------------------------------------ builder

/// A node's children (name to path) and attributes.
#[derive(Default)]
struct Builder {
    nodes: BTreeMap<String, Node>,
    /// key -> merged type
    schemas: BTreeMap<String, Ty>,
    /// (path, key) -> (type, value) of properties, typed on the second pass
    pending: Vec<(String, String, Ty, Value)>,
    ledger: LossLedger,
    errors: Vec<String>,
    order: Order,
    root: String,
    project: String,
    /// (occurrence path, type path): each occurrence inherits its type.
    type_links: Vec<(String, String)>,
    /// Every type node, an unplaced one included.
    type_nodes: BTreeSet<String>,
}

impl Builder {
    fn node(&mut self, path: &str) -> &mut Node {
        self.nodes.entry(path.to_string()).or_insert_with(|| Node {
            ifcx: true,
            ..Node::default()
        })
    }

    /// A node that exists for the IFC4 derivation only.
    fn aux(&mut self, path: &str) -> &mut Node {
        self.nodes.entry(path.to_string()).or_default()
    }

    fn attr(&mut self, path: &str, key: &str, value: Value) {
        self.node(path).attributes.insert(key.to_string(), value);
    }

    /// An IFC4 key on a node (made an IFC4-only node if it is new).
    fn attr4(&mut self, path: &str, key: &str, value: Value) {
        self.aux(path)
            .attributes
            .insert(format!("{IFC4_PREFIX}{key}"), value);
    }

    /// Every field of `t` as an IFC4 key of the node.
    fn fields4(&mut self, path: &str, fields: Map<String, Value>) {
        for (k, v) in fields {
            self.attr4(path, &k, v);
        }
    }

    fn declare(&mut self, key: &str, ty: Ty) {
        let merged = match self.schemas.get(key) {
            Some(old) => old.merge(&ty),
            None => ty,
        };
        self.schemas.insert(key.to_string(), merged);
    }

    /// A declared attribute (`ifc-src::attr::*`): typed here, written now.
    fn declared(&mut self, path: &str, key: &str, ty: Ty, value: Value) {
        self.declare(key, ty);
        self.attr(path, key, value);
    }

    fn child(&mut self, parent: &str, name: String, path: &str) {
        let kids = &mut self.node(parent).children;
        match kids.get(&name) {
            // the same element listed twice under one parent
            Some(old) if old == path => {}
            // two different elements whose name and 8 leading id characters agree (a project model
            // hit this): the key stays unique with the whole id
            Some(_) => {
                kids.insert(
                    format!("{name}_{}", path.replace('-', "")),
                    path.to_string(),
                );
            }
            None => {
                kids.insert(name, path.to_string());
            }
        }
    }

    /// An IFC GlobalId as a path; one that is not an IFC id is an IFCX error
    /// and gets a stand-in path.
    fn guid_path(&mut self, global_id: &str) -> String {
        match ifc_guid_uuid(global_id) {
            Ok(p) => p,
            Err(IfcxError(e)) => {
                self.errors.push(e);
                uuid5(&format!("bad-global-id/{global_id}"))
            }
        }
    }

    fn class(&mut self, path: &str, upper: &str) {
        let code = super::ifcx_writer::ifc_class_name(upper);
        let uri =
            format!("https://identifier.buildingsmart.org/uri/buildingsmart/ifc/4.3/class/{code}");
        self.attr(path, "bsi::ifc::class", json!({"code": code, "uri": uri}));
    }

    /// All property sets of one element. A standard-set property that
    /// prop@v5a defines goes out once, as `bsi::ifc::prop::<Prop>`, when every
    /// standard set that has it agrees on one value of the schema's type.
    fn property_sets(&mut self, path: &str, sets: &[&PropertySet]) {
        let standard = |n: &str| n.starts_with("Pset_") || n.starts_with("Qto_");
        let mut seen: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
        let mut flat: BTreeMap<&str, Value> = BTreeMap::new();
        for set in sets.iter().filter(|s| standard(&s.name)) {
            for p in &set.properties {
                let Some(want) = v5a_prop(&p.name) else {
                    continue;
                };
                if let Some((ty, v)) = value_of(&p.value) {
                    seen.entry(&p.name).or_default().insert(v.to_string());
                    if let Some(v) = as_v5a(&want, &ty, &v) {
                        flat.insert(&p.name, v);
                    }
                }
            }
        }
        // Several values, or another kind: stay as pset keys.
        flat.retain(|n, _| seen[n].len() == 1);
        for (name, v) in &flat {
            self.attr(path, &format!("bsi::ifc::prop::{name}"), v.clone());
            // the standard sets the value came from, by name (the
            // value itself stays under the flat key only, IFC5-development#72).
            let from: Vec<&str> = sets
                .iter()
                .filter(|s| standard(&s.name) && s.properties.iter().any(|p| p.name == *name))
                .map(|s| s.name.as_str())
                .collect();
            self.declared(
                path,
                &format!("{PSET_OF}{name}"),
                Ty::List(Box::new(Ty::Str)),
                json!(from),
            );
        }
        for set in sets {
            for p in &set.properties {
                if standard(&set.name) && flat.contains_key(p.name.as_str()) {
                    self.ledger.properties_flattened_to_v5a += 1;
                    continue;
                }
                let key = property_key(&set.name, &p.name);
                match value_of(&p.value) {
                    Some((ty, value)) => {
                        self.declare(&key, ty.clone());
                        self.pending.push((path.to_string(), key, ty, value));
                    }
                    None if matches!(p.value, PropertyValue::Logical(None)) => {
                        self.ledger.properties_unset += 1
                    }
                    None => self.ledger.properties_non_finite += 1,
                }
            }
        }
    }

    /// `RefLatitude` / `RefLongitude` / `RefElevation` on the
    /// site, the Revit georeference object, and two reference points
    /// (`ReferencePoint0` at the internal origin, `ReferencePoint1` 100 m
    /// along internal X) with their shared coordinates and, when the CRS has
    /// an EPSG code, their `epsg<code>` eastings and northings: buildingSMART
    /// IFC5's georeferencing by point correspondence
    /// (examples/Georeferencing).
    fn georeference(&mut self, site: &str, g: &Georeference) {
        let len = || Ty::Real(Some("Length"));
        let xyz = || Ty::List(Box::new(len()));
        self.declared(
            site,
            "ifc-src::attr::RefLatitude",
            Ty::Real(None),
            json!(g.latitude),
        );
        self.declared(
            site,
            "ifc-src::attr::RefLongitude",
            Ty::Real(None),
            json!(g.longitude),
        );
        self.declared(
            site,
            "ifc-src::attr::RefElevation",
            len(),
            json!(g.elevation),
        );
        let mut fields: Vec<(&'static str, Ty)> = vec![
            ("locationName", Ty::Str),
            ("timeZone", Ty::Real(None)),
            ("trueNorth", Ty::Real(None)),
            ("originShared", xyz()),
        ];
        let mut value = json!({
            "locationName": g.location_name,
            "timeZone": g.time_zone,
            "trueNorth": g.true_north,
            "originShared": g.origin_shared,
        });
        let point_ty = || Ty::Obj(vec![("internal", xyz()), ("shared", xyz())]);
        for (k, p) in [
            ("surveyPoint", &g.survey_point),
            ("projectBasePoint", &g.project_base_point),
        ] {
            if let Some(p) = p {
                fields.push((k, point_ty()));
                value[k] = json!({"internal": p.internal, "shared": p.shared});
            }
        }
        if let Some(crs) = &g.crs {
            let mut crs_fields: Vec<(&'static str, Ty)> =
                vec![("name", Ty::Str), ("source", Ty::Str)];
            let mut v = json!({"name": crs.name, "source": crs.source});
            if let Some(e) = crs.epsg {
                crs_fields.push(("epsg", Ty::Int));
                v["epsg"] = json!(e);
            }
            for (k, x) in [
                ("description", &crs.description),
                ("mapUnit", &crs.map_unit),
            ] {
                if let Some(x) = x {
                    crs_fields.push((k, Ty::Str));
                    v[k] = json!(x);
                }
            }
            fields.push(("crs", Ty::Obj(crs_fields)));
            value["crs"] = v;
        }
        self.declared(site, GEOREFERENCE, Ty::Obj(fields), value);

        for (i, internal) in [[0.0, 0.0, 0.0], [100.0, 0.0, 0.0]].into_iter().enumerate() {
            let path = uuid5(&format!("{site}/reference-point/{i}"));
            self.child(site, format!("ReferencePoint{i}"), &path);
            let [x, y, z] = internal;
            self.attr(
                &path,
                "usd::xformop",
                json!({"transform": [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [x, y, z, 1.0]]}),
            );
            let [e, n, h] = g.to_shared(internal);
            self.declared(
                &path,
                SHARED_COORDINATES,
                Ty::Obj(vec![
                    ("eastings", len()),
                    ("northings", len()),
                    ("height", len()),
                ]),
                json!({"eastings": e, "northings": n, "height": h}),
            );
            if let Some(crs) = &g.crs {
                if let Some(code) = crs.epsg {
                    self.declared(
                        &path,
                        &format!("epsg{code}"),
                        Ty::Obj(vec![
                            ("eastings", Ty::Real(None)),
                            ("northings", Ty::Real(None)),
                        ]),
                        json!({"eastings": crs.map_value(e), "northings": crs.map_value(n)}),
                    );
                }
            }
        }
    }

    fn build(&mut self, model: &IfcModel, options: GraphOptions) {
        let gid = |key: &[&str]| generated_global_id(model, key);
        // The default namespace keeps the IFCX ids as they were; a user's
        // namespace joins the root, from which the other ids derive.
        let root = match model.global_ids.guid_namespace.as_deref() {
            Some(ns) if ns != crate::ifc::DEFAULT_GUID_NAMESPACE => {
                uuid5(&format!("{ns}\0{}/root", global_id_seed(model)))
            }
            _ => uuid5(&format!("{}/root", global_id_seed(model))),
        };
        self.root = root.clone();
        let project = self.guid_path(&project_level_global_id(model, "project"));
        self.project = project.clone();
        let site = self.guid_path(&project_level_global_id(model, "site"));
        let building = self.guid_path(&project_level_global_id(model, "building"));
        let project_name = model.project_name.as_deref().unwrap_or("Untitled");

        self.child(&root, child_key(project_name, &project), &project);
        self.class(&project, "IFCPROJECT");
        self.attr(&project, "bsi::ifc::prop::Name", json!(project_name));
        if let Some(d) = &model.description {
            self.attr(&project, "bsi::ifc::prop::Description", json!(d));
        }
        let site_label = model.site_name.as_deref().unwrap_or("Default Site");
        let building_label = model.building_name.as_deref().unwrap_or("Default Building");
        self.child(&project, child_key(site_label, &site), &site);
        self.class(&site, "IFCSITE");
        // The names the IFC4 output gives: Project Information's Project Name
        // and Building Name, else the placeholders; without them a viewer's
        // tree shows the two nodes unnamed.
        self.attr(&site, "bsi::ifc::prop::Name", json!(site_label));
        if let Some(g) = &model.georeference {
            self.georeference(&site, g);
        }
        // under `revit` a model with no storey has no building, and the site
        // holds the elements.
        let site_holds = model
            .convention
            .site_holds_elements(model.building_storeys.is_empty());
        if !site_holds {
            self.child(&site, child_key(building_label, &building), &building);
            self.class(&building, "IFCBUILDING");
            self.attr(&building, "bsi::ifc::prop::Name", json!(building_label));
        }
        let spatial_root = if site_holds {
            site.clone()
        } else {
            building.clone()
        };

        // Storeys, in the order and with the ids the IFC4 writer gives them.
        let mut occurrences: BTreeMap<&str, usize> = BTreeMap::new();
        let mut storey_paths = Vec::new();
        for (i, s) in model.building_storeys.iter().enumerate() {
            let n = occurrences.entry(s.name.as_str()).or_default();
            let g = match model.global_ids.storeys.get(&i) {
                Some(g) => g.clone(),
                None => gid(&["storey", &s.name, &n.to_string()]),
            };
            *n += 1;
            let path = self.guid_path(&g);
            self.child(&building, child_key(&s.name, &path), &path);
            self.class(&path, "IFCBUILDINGSTOREY");
            self.attr(&path, "bsi::ifc::prop::Name", json!(s.name));
            self.declared(
                &path,
                "ifc-src::attr::Elevation",
                Ty::Real(Some("Length")),
                json!(s.elevation_feet * FT),
            );
            storey_paths.push(path);
        }
        self.ledger.storey_building_psets = if model.building_storeys.is_empty() {
            0
        } else {
            3 * model.building_storeys.len() + 1
        };

        // Element ids, and where each element hangs.
        let gids = building_element_global_ids(model);
        let mut paths: Vec<Option<String>> = vec![None; model.entities.len()];
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for (i, g) in gids.iter().enumerate() {
            if let Some(g) = g {
                let p = self.guid_path(g);
                if !seen.insert(p.clone()) {
                    self.errors.push(format!("two elements share the path {p}"));
                }
                paths[i] = Some(p);
            }
        }
        let mut parent_of: BTreeMap<usize, usize> = BTreeMap::new();
        for e in &model.entities {
            match e {
                IfcEntity::Aggregate { whole, parts } => {
                    for &p in parts.iter().filter(|p| **p != *whole) {
                        if paths.get(p).is_some_and(Option::is_some) {
                            parent_of.entry(p).or_insert(*whole);
                        }
                    }
                }
                IfcEntity::SpaceContainment { space, elements } => {
                    for &p in elements.iter().filter(|p| **p != *space) {
                        if paths.get(p).is_some_and(Option::is_some) {
                            parent_of.entry(p).or_insert(*space);
                        }
                    }
                }
                _ => {}
            }
        }

        // an element whose family draws in one material (a
        // constituent set of one) takes its colour, as glTF does.
        let single_material: BTreeMap<usize, usize> = model
            .material_constituent_sets
            .iter()
            .filter_map(|set| match set.material_indices.as_slice() {
                [m] => Some(set.elements.iter().map(move |&e| (e, *m))),
                _ => None,
            })
            .flatten()
            .collect();
        let mut extra_sets: BTreeMap<usize, Vec<&PropertySet>> = BTreeMap::new();
        for e in &model.entities {
            match e {
                IfcEntity::ElementPropertySet { element, set } => {
                    extra_sets.entry(*element).or_default().push(set)
                }
                IfcEntity::BuildingElementType { .. } | IfcEntity::TypeObject { .. } => {
                    self.ledger.type_objects += 1
                }
                IfcEntity::VoidOpening { .. } | IfcEntity::WallProfileOpening { .. } => {
                    self.ledger.openings_and_voids += 1
                }
                IfcEntity::Port { .. } | IfcEntity::PortConnection { .. } => {
                    self.ledger.ports_and_connections += 1
                }
                IfcEntity::System { .. } => self.ledger.systems += 1,
                IfcEntity::DesignOption { .. } => self.ledger.design_option_groups += 1,
                IfcEntity::Zone { .. } => self.ledger.zones += 1,
                _ => {}
            }
        }
        self.ledger.classifications = model.classifications.iter().map(|c| c.items.len()).sum();

        // how many parts draw each symbol mesh.
        let usage = super::instancing::part_usage(model);
        for (i, entity) in model.entities.iter().enumerate() {
            let IfcEntity::BuildingElement {
                ifc_type,
                name,
                type_guid,
                predefined_type,
                storey_index,
                material_index,
                material_layer_set_index,
                material_profile_set_index,
                property_set,
                location_feet,
                rotation_radians,
                host_element_index,
                representation_map_index,
                ..
            } = entity
            else {
                continue;
            };
            let path = paths[i].clone().expect("every building element has a path");
            self.ledger.elements += 1;
            let parent = match parent_of.get(&i).and_then(|p| paths[*p].clone()) {
                Some(p) => p,
                None => match storey_index.and_then(|s| storey_paths.get(s)) {
                    Some(s) => s.clone(),
                    None => spatial_root.clone(),
                },
            };
            self.child(&parent, child_key(name, &path), &path);
            self.class(&path, ifc_type);
            // The direct attributes the STEP writer derives from the element's
            // own properties: a room's number and name, `Family:Type`, a
            // door's or window's opening size.
            let text_of = |key: &str| {
                property_set.as_ref().and_then(|set| {
                    set.properties.iter().find_map(|p| match &p.value {
                        PropertyValue::Text(t) if p.name == key && !t.trim().is_empty() => {
                            Some(t.clone())
                        }
                        _ => None,
                    })
                })
            };
            let is_space = ifc_type.eq_ignore_ascii_case("IFCSPACE");
            let shown_name = if is_space {
                text_of(ROOM_NUMBER_PROPERTY).unwrap_or_else(|| name.clone())
            } else {
                name.clone()
            };
            self.attr(&path, "bsi::ifc::prop::Name", json!(shown_name));
            if is_space {
                if let Some(long) = text_of(ROOM_NAME_PROPERTY) {
                    self.declared(&path, "ifc-src::attr::LongName", Ty::Str, json!(long));
                }
                self.declared(
                    &path,
                    "ifc-src::attr::CompositionType",
                    Ty::Str,
                    json!("ELEMENT"),
                );
            }
            if let (Some(family), Some(kind)) =
                (text_of(FAMILY_NAME_PROPERTY), text_of(TYPE_NAME_PROPERTY))
            {
                let v = json!(format!("{family}:{kind}"));
                self.declared(&path, "ifc-src::attr::ObjectType", Ty::Str, v);
            }
            // The size the exporter worked out ([`IfcModel::filler_sizes`]),
            // as the STEP writer writes it.
            if matches!(ifc_type.as_str(), "IFCDOOR" | "IFCWINDOW") {
                if let Some(size) = model.filler_sizes.get(&i) {
                    for (ft, attr) in [
                        (size.height_feet, "OverallHeight"),
                        (size.width_feet, "OverallWidth"),
                    ] {
                        let ty = Ty::Real(Some("Length"));
                        self.declared(&path, &format!("ifc-src::attr::{attr}"), ty, json!(ft * FT));
                    }
                }
            }
            if let Some(pt) = predefined_type {
                self.declared(&path, "ifc-src::attr::PredefinedType", Ty::Str, json!(pt));
            }
            // `IfcSpace` has no Tag.
            if let Some(tag) = type_guid.as_ref().filter(|_| !is_space) {
                self.declared(&path, "ifc-src::attr::Tag", Ty::Str, json!(tag));
                // A stair part is named after its stair, so its own name
                // carries the stair's inferred family.
                if let Some(rule) = tag
                    .parse::<u32>()
                    .ok()
                    .and_then(|id| model.element_name_rules.get(&id))
                {
                    self.declared(
                        &path,
                        "rvtopen::revit::Inferred::FamilyNameRule",
                        Ty::Str,
                        json!(rule),
                    );
                    *self
                        .ledger
                        .inferred_element_names
                        .entry(rule.clone())
                        .or_default() += 1;
                }
            }
            if material_index.is_some()
                || material_layer_set_index.is_some()
                || material_profile_set_index.is_some()
            {
                self.ledger.material_associations += 1;
            }
            if host_element_index.is_some() {
                self.ledger.openings_and_voids += 1;
            }

            // Colour: the element's material, else the first layer (exterior
            // side) or profile of its layer or profile set that has a colour,
            // else the category's. Walls, roofs, floors and stairs are
            // associated with a layer set, not a single material.
            let coloured = |m: &usize| {
                model
                    .materials
                    .get(*m)
                    .is_some_and(|m| m.color_packed.is_some())
            };
            let colour_material = material_index
                .filter(coloured)
                .or_else(|| {
                    material_layer_set_index
                        .and_then(|i| model.material_layer_sets.get(i))
                        .and_then(|set| {
                            set.layers
                                .iter()
                                .filter_map(|l| l.material_index)
                                .find(coloured)
                        })
                })
                .or_else(|| {
                    material_profile_set_index
                        .and_then(|i| model.material_profile_sets.get(i))
                        .and_then(|set| {
                            set.profiles.iter().map(|p| p.material_index).find(coloured)
                        })
                })
                .or_else(|| single_material.get(&i).copied().filter(coloured));
            let (rgb, opacity) = match colour_material.and_then(|m| model.materials.get(m)) {
                Some(m) if m.color_packed.is_some() => {
                    let p = m.color_packed.unwrap_or(0);
                    (
                        [p & 0xff, (p >> 8) & 0xff, (p >> 16) & 0xff],
                        1.0 - m.transparency.unwrap_or(0.0).clamp(0.0, 1.0),
                    )
                }
                _ => {
                    let (c, a) = category_colour(ifc_type);
                    ([(c >> 16) & 0xff, (c >> 8) & 0xff, c & 0xff], f64::from(a))
                }
            };
            let rgb: Vec<f64> = rgb.iter().map(|c| f64::from(*c) / 255.0).collect();
            self.attr(&path, "bsi::ifc::presentation::diffuseColor", json!(rgb));
            if opacity < 1.0 {
                self.attr(&path, "bsi::ifc::presentation::opacity", json!(opacity));
            }

            let mut sets: Vec<&PropertySet> = property_set.iter().collect();
            sets.extend(extra_sets.remove(&i).unwrap_or_default());
            self.property_sets(&path, &sets);

            // element-local mesh (a node the Body inherits) and the element's
            // placement as the Body's `usd::xformop`; with `shared_shapes`
            // off, one flat mesh with the placement baked in. Metres.
            // a body of several parts (a railing's balusters) is
            // drawn part by part, each repeated part a shared shape.
            let placement = Placement::new(*location_feet, *rotation_radians);
            let parts = (options.meshes && options.shared_shapes)
                .then(|| super::instancing::drawn_parts(model, i, &usage))
                .filter(|_| !model.display_bodies.contains_key(&i))
                .flatten();
            if let Some((shared, rest)) = parts {
                let mut next = 0;
                if let Some(mesh) = rest {
                    next = self.bodies(
                        model,
                        &path,
                        mesh,
                        &placement,
                        &super::instancing::IDENTITY,
                        next,
                        true,
                    );
                }
                for (symbol, target) in shared {
                    if let Some(mesh) =
                        super::instancing::body_mesh(&model.instancing.meshes[symbol])
                    {
                        next = self.bodies(model, &path, mesh, &placement, &target, next, true);
                    }
                }
            } else if let Some(body) =
                super::body_geometry::drawn_body(model, i, entity).filter(|_| options.meshes)
            {
                match body_mesh(body) {
                    Some(mesh) if !mesh.is_empty() => {
                        // A display body is in the element's own frame.
                        let target = if model.display_bodies.contains_key(&i) {
                            super::instancing::IDENTITY
                        } else {
                            super::instancing::target_of(model, i)
                        };
                        self.bodies(
                            model,
                            &path,
                            mesh,
                            &placement,
                            &target,
                            0,
                            options.shared_shapes,
                        );
                        if representation_map_index.is_some() {
                            self.ledger.instanced_bodies_flattened += 1;
                        }
                    }
                    _ => self.ledger.bodies_not_drawn += 1,
                }
            }
        }
        self.types(model, &gids, &paths);
        self.payload(model, &paths, &storey_paths);
    }

    /// One Body per face material of `mesh` under the element at `path`,
    /// named from `start` on (`Body`, `Body1`, ...), placed by `placement`
    /// after `target`; shared shapes when `shared`. Returns the
    /// next free Body number.
    #[allow(clippy::too_many_arguments)]
    fn bodies(
        &mut self,
        model: &IfcModel,
        path: &str,
        mesh: super::body_geometry::Mesh,
        placement: &Placement,
        target: &super::instancing::Frame,
        start: usize,
        shared: bool,
    ) -> usize {
        let mut next = start;
        for (part, (mesh, material)) in (start..).zip(split_by_face_material(mesh)) {
            next = part + 1;
            let indices: Vec<u32> = mesh.triangles.iter().flatten().copied().collect();
            let (body_path, body_name) = if part == 0 {
                (uuid5(&format!("{path}/body")), "Body".to_string())
            } else {
                (uuid5(&format!("{path}/body/{part}")), format!("Body{part}"))
            };
            self.child(path, body_name, &body_path);
            if let Some(m) = material.and_then(|m| model.materials.get(m)) {
                if let Some(p) = m.color_packed {
                    let rgb: Vec<f64> = [p & 0xff, (p >> 8) & 0xff, (p >> 16) & 0xff]
                        .iter()
                        .map(|c| f64::from(*c) / 255.0)
                        .collect();
                    self.attr(
                        &body_path,
                        "bsi::ifc::presentation::diffuseColor",
                        json!(rgb),
                    );
                    let opacity = 1.0 - m.transparency.unwrap_or(0.0).clamp(0.0, 1.0);
                    if opacity < 1.0 {
                        self.attr(
                            &body_path,
                            "bsi::ifc::presentation::opacity",
                            json!(opacity),
                        );
                    }
                }
            }
            if shared {
                let (points, key, lo) = shared_points(&mesh);
                let shape = uuid5(&format!("{}/shape/{key}", self.root));
                if !self.nodes.contains_key(&shape) {
                    self.attr(
                        &shape,
                        "usd::usdgeom::mesh",
                        json!({"points": points, "faceVertexIndices": indices}),
                    );
                }
                self.node(&body_path).inherits.push(shape);
                let r = |v: f64| (v * 1e7).round() / 1e7;
                // The shape's corner goes through the map's
                // target and the placement; the
                // rows are the images of the axes.
                let m = super::instancing::compose(
                    &super::instancing::placement_frame(placement),
                    target,
                );
                let corner = super::instancing::apply(&m, lo.map(|v| v / FT));
                let [x, y, z] = corner.map(|v| r(v * FT));
                let row = |k: usize| json!([m[0][k], m[1][k], m[2][k], 0.0]);
                self.attr(
                    &body_path,
                    "usd::xformop",
                    json!({"transform": [row(0), row(1), row(2), [x, y, z, 1.0]]}),
                );
            } else {
                let r = |v: f64| (v * FT * 1e6).round() / 1e6;
                let points: Vec<Value> = mesh
                    .vertices
                    .iter()
                    .map(|v| {
                        let p = placement.apply(*v);
                        json!([r(p[0]), r(p[1]), r(p[2])])
                    })
                    .collect();
                self.attr(
                    &body_path,
                    "usd::usdgeom::mesh",
                    json!({"points": points, "faceVertexIndices": indices}),
                );
            }
        }
        next
    }

    /// the IFC4 writer's type objects as nodes (the occurrence's
    /// class, as buildingSMART's Hello Wall has it), each occurrence
    /// inheriting its type. Geometry stays on the elements.
    fn types(&mut self, model: &IfcModel, gids: &[Option<String>], paths: &[Option<String>]) {
        for t in type_object_plan(model, gids) {
            let path = self.guid_path(&t.global_id);
            if self.nodes.contains_key(&path) {
                self.errors
                    .push(format!("type {path} shares its path with another node"));
                continue;
            }
            self.class(&path, &t.occurrence);
            self.type_nodes.insert(path.clone());
            // a type nothing places is a type node nothing
            // inherits, which IFCX cannot otherwise tell from an element.
            if t.elements.is_empty() {
                self.declared(&path, UNPLACED_TYPE, Ty::Bool, json!(true));
            }
            self.attr(&path, "bsi::ifc::prop::Name", json!(t.name));
            self.declared(&path, "ifc-src::attr::Tag", Ty::Str, json!(t.tag));
            // NOTDEFINED is the IFC4 writer's filler for "not read", not a
            // value: composed onto an occurrence whose own is absent it would
            // turn an unknown into a claim (Revit has GULLYSUMP there).
            if t.predefined != "NOTDEFINED" {
                self.declared(
                    &path,
                    "ifc-src::attr::PredefinedType",
                    Ty::Str,
                    json!(t.predefined),
                );
            }
            self.property_sets(&path, &t.sets.iter().collect::<Vec<_>>());
            if let Some(rule) = &t.name_rule {
                self.declared(
                    &path,
                    "rvtopen::revit::Inferred::FamilyNameRule",
                    Ty::Str,
                    json!(rule),
                );
                *self
                    .ledger
                    .inferred_type_names
                    .entry(rule.clone())
                    .or_default() += 1;
            }
            for occurrence in t.elements.iter().filter_map(|i| paths[*i].clone()) {
                self.node(&occurrence).inherits.push(path.clone());
                self.type_links.push((occurrence, path.clone()));
            }
        }
    }

    /// The IFC4 keys: every list the IFC4 writer walks becomes nodes, with
    /// indices turned into paths.
    fn payload(&mut self, model: &IfcModel, element_paths: &[Option<String>], storeys: &[String]) {
        let root = self.root.clone();
        let table = |kind: &str, n: usize| -> Vec<String> {
            (0..n)
                .map(|i| uuid5(&format!("{root}/{kind}/{i}")))
                .collect()
        };
        let paths = Paths {
            entity: (0..model.entities.len())
                .map(|i| {
                    element_paths[i]
                        .clone()
                        .unwrap_or_else(|| uuid5(&format!("{root}/entity/{i}")))
                })
                .collect(),
            storey: storeys.to_vec(),
            material: table("material", model.materials.len()),
            layer_set: table("layer-set", model.material_layer_sets.len()),
            profile_set: table("profile-set", model.material_profile_sets.len()),
            rep_map: table("representation-map", model.representation_maps.len()),
        };
        let constituent = table("constituent-set", model.material_constituent_sets.len());

        let project = self.project.clone();
        self.attr4(&project, "projectName", json!(model.project_name));
        self.attr4(&project, "siteName", json!(model.site_name));
        self.attr4(&project, "buildingName", json!(model.building_name));
        self.attr4(&project, "description", json!(model.description));
        let mut project_model = json!({
            "units": model.units,
            "classifications": model.classifications,
            "element_layers": model.element_layers,
            "element_type_materials": model.element_type_materials,
            "element_type_ids": model.element_type_ids,
            "file_types": model.file_types,
            "convention": model.convention,
            "global_ids_document": model.global_ids.document,
            "global_ids_history": model.global_ids.history,
            "global_ids_types": model.global_ids.types,
            "global_ids_element_types": model.global_ids.element_types,
            "global_ids_revit_version": model.global_ids.revit_version,
            "global_ids_project": model.global_ids.project,
            "global_ids_site": model.global_ids.site,
            "global_ids_building": model.global_ids.building,
            "type_property_sets": model.type_property_sets,
            "type_entity_sets": model.type_entity_sets,
            "body_pieces": model.body_pieces,
            "georeference": model.georeference,
        });
        if let Some(ns) = &model.global_ids.guid_namespace {
            project_model["global_ids_guid_namespace"] = json!(ns);
        }
        self.attr4(&project, "model", project_model);

        for (i, e) in model.entities.iter().enumerate() {
            let p = paths.entity[i].clone();
            let mut fields = to_object(e);
            let kind = fields
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let mut obj = Value::Object(std::mem::take(&mut fields));
            encode_refs(&mut obj, entity_refs(&kind), &paths);
            if let Value::Object(m) = obj {
                self.fields4(&p, m);
            }
            if let Some(g) = model.global_ids.elements.get(&i) {
                self.attr4(&p, "revitGlobalId", json!(g));
            }
            if let Some(size) = model.filler_sizes.get(&i) {
                self.attr4(&p, "fillerSize", json!(size));
            }
            if let Some(line) = model.wall_location_lines.get(&i) {
                self.attr4(&p, "wallLocationLine", json!(line));
            }
            if let Some(c) = model.opening_cuts.get(&i) {
                self.attr4(&p, "openingCut", json!(c));
            }
            if let Some(t) = model.instancing.targets.get(&i) {
                self.attr4(&p, "mapTarget", json!(t));
            }
            if let Some(u) = model.material_layer_usages.get(&i) {
                self.attr4(&p, "layerSetUsage", json!(u));
            }
        }
        for (i, s) in model.building_storeys.iter().enumerate() {
            let p = &paths.storey[i];
            self.attr4(p, "name", json!(s.name));
            self.attr4(p, "elevation_feet", json!(s.elevation_feet));
            if let Some(g) = model.global_ids.storeys.get(&i) {
                self.attr4(p, "revitGlobalId", json!(g));
            }
            if let Some(props) = model.storey_properties.get(&i) {
                self.attr4(p, "storeyProperties", json!(props));
            }
        }
        for (i, m) in model.materials.iter().enumerate() {
            self.fields4(&paths.material[i], to_object(m));
        }
        for (i, set) in model.material_layer_sets.iter().enumerate() {
            let mut o = Value::Object(to_object(set));
            encode_refs(&mut o, LAYER_SET_REFS, &paths);
            if let Value::Object(m) = o {
                self.fields4(&paths.layer_set[i], m);
            }
        }
        for (i, set) in model.material_profile_sets.iter().enumerate() {
            let mut o = Value::Object(to_object(set));
            encode_refs(&mut o, PROFILE_SET_REFS, &paths);
            if let Value::Object(m) = o {
                self.fields4(&paths.profile_set[i], m);
            }
        }
        for (i, rm) in model.representation_maps.iter().enumerate() {
            self.fields4(&paths.rep_map[i], to_object(rm));
        }
        for (i, set) in model.material_constituent_sets.iter().enumerate() {
            let mut o = Value::Object(to_object(set));
            encode_refs(&mut o, CONSTITUENT_REFS, &paths);
            if let Value::Object(m) = o {
                self.fields4(&constituent[i], m);
            }
        }
        self.order = Order {
            entities: paths.entity,
            storeys: paths.storey,
            materials: paths.material,
            layer_sets: paths.layer_set,
            profile_sets: paths.profile_set,
            representation_maps: paths.rep_map,
            constituent_sets: constituent,
        };
    }

    fn finish(mut self, model: &IfcModel) -> IfcxGraph {
        // Second pass: every property in its key's one type.
        for (path, key, from, value) in std::mem::take(&mut self.pending) {
            let to = self.schemas[&key].clone();
            if from != to {
                self.ledger.properties_coerced += 1;
            }
            let v = coerce(value, &from, &to);
            self.attr(&path, &key, v);
        }
        // An occurrence repeats no value its type holds (its own class and
        // Name stay, and any value that differs from the type's).
        for (occurrence, ty) in std::mem::take(&mut self.type_links) {
            let held: Vec<(String, Value)> = self.nodes[&ty]
                .attributes
                .iter()
                .filter(|(k, _)| !matches!(k.as_str(), "bsi::ifc::class" | "bsi::ifc::prop::Name"))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            let own = &mut self.node(&occurrence).attributes;
            for (k, v) in held {
                if own.get(&k) == Some(&v) {
                    own.remove(&k);
                }
            }
        }
        IfcxGraph {
            root: self.root,
            project: self.project,
            id: format!(
                "rvt-open/{}",
                model.project_name.as_deref().unwrap_or("Untitled")
            ),
            nodes: self.nodes,
            schemas: self.schemas,
            order: self.order,
            ledger: self.ledger,
            errors: self.errors,
            type_nodes: self.type_nodes,
        }
    }
}
