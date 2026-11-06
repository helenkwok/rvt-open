//! IFCX (IFC5 alpha) writer: `write_ifcx(&IfcModel)` beside the IFC4 STEP
//!
//! Decisions ("Answers from our own IFCX work"):
//! - header `ifcx_alpha`, imports `ifc@v5a`, `prop@v5a` and `usd@v1`;
//! - a node's path is its IFC GlobalId as a UUID; a mesh child's is the
//!   uuid5 of its element's UUID + `/body` under [`IFCX_NAMESPACE`];
//! - hierarchy and classes through `bsi::ifc::*`; `Name` / `Description` as
//!   `bsi::ifc::prop::*` (prop@v5a); `PredefinedType`, `Tag` and the other
//!   direct attributes as `ifc-src::attr::<Attr>`; a standard `Pset_*` /
//!   `Qto_*` property that prop@v5a defines (IsExternal, Height, ...) as the
//!   flat `bsi::ifc::prop::<Prop>` when all standard sets agree on one value
//!   of the schema's type (the oracle's rule, IFC5-development#72), else
//!   as `ifc-src::pset::<Pset>::<Prop>`; every other property
//!   (Revit's own groups) as `rvtopen::revit::<Group>::<Name>`; every such
//!   key is declared in `schemas`;
//! - metres only, `quantityKind` on length schemas;
//! - geometry: one flat `usd::usdgeom::mesh` per element as its `Body`
//!   child, placement baked into the points, no prototypes, mesh only;
//! - [`LossLedger`] counts what the IFC4 writer emits that this one does not;
//! - exporter diagnostics ([`DIAGNOSTIC_KEYS`]) can be split into a layer of
//!   their own ([`write_ifcx_graph_layers`], `rvt-ifc --ifcx-diagnostics`),
//!   so the main file does not change when only the RVT's layout does;
//!   [`write_ifcx_graph`] is the two composed, one file.
//!
//! Output is a pure function of the model: byte-identical on a rerun.

use super::IfcModel;
use super::entities::PropertyValue;
use super::graph::{GraphOptions, IFC4_PREFIX, IfcxGraph};
use crate::revit_global_ids::expand_ifc_guid;
use serde_json::{Map, Value, json};

/// The fixed namespace of every uuid5 this writer makes.
pub const IFCX_NAMESPACE: [u8; 16] = [
    0x8d, 0x3b, 0x1c, 0x54, 0x5e, 0x0a, 0x4f, 0x6b, 0x9a, 0x37, 0x7c, 0x2d, 0x41, 0xe0, 0xb9, 0xa6,
];

/// Import URIs, as the buildingSMART IFC5-development examples use.
pub const IFC_IMPORT: &str = "https://ifcx.dev/@standards.buildingsmart.org/ifc/core/ifc@v5a.ifcx";
pub const PROP_IMPORT: &str =
    "https://ifcx.dev/@standards.buildingsmart.org/ifc/core/prop@v5a.ifcx";
pub const USD_IMPORT: &str = "https://ifcx.dev/@openusd.org/usd@v1.ifcx";

/// What the IFC4 writer emits for a model that the IFCX writer does not.
/// Counts are of IFC4 things (entities, relationships, values), per model.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize)]
pub struct LossLedger {
    /// Building elements written as IFCX nodes.
    pub elements: usize,
    /// `IfcBuildingElementType`/`IfcTypeObject` entities and typed elements.
    pub type_objects: usize,
    /// Type nodes whose family name an inference rule gave, by
    /// rule: the name is marked `rvtopen::revit::Inferred::FamilyNameRule`.
    pub inferred_type_names: std::collections::BTreeMap<String, usize>,
    /// Elements named after a stair whose family a rule gave (its parts),
    /// marked the same way, by rule.
    pub inferred_element_names: std::collections::BTreeMap<String, usize>,
    /// Elements typed by an IFC type object (`IfcRelDefinesByType`) and not
    pub typed_elements: usize,
    /// Elements with a material association (layer, profile, constituent
    /// set or single material). Their colour is kept, the association not.
    pub material_associations: usize,
    /// Door/window openings, floor/wall-profile openings: `IfcOpeningElement`
    /// with `IfcRelVoidsElement` / `IfcRelFillsElement`.
    pub openings_and_voids: usize,
    /// `IfcDistributionPort`, `IfcRelConnectsPorts`, `IfcRelNests`.
    pub ports_and_connections: usize,
    /// `IfcSystem` with its group assignment.
    pub systems: usize,
    /// Design options (`open`) as `IfcGroup`s with their group
    /// assignments. Each member keeps its set, option and primary flag in
    /// its own `Design Option` property set, which IFCX carries.
    pub design_option_groups: usize,
    /// HVAC zones (`open`) as `IfcZone`s with their group
    /// assignments: IFCX v5a has no zone.
    pub zones: usize,
    /// Classification references.
    pub classifications: usize,
    /// The storey and building property sets the IFC4 writer adds.
    pub storey_building_psets: usize,
    /// Properties with an unknown (`IFCLOGICAL` U) value: IFCX has no way to
    /// say "present but unset".
    pub properties_unset: usize,
    /// Properties dropped because a number was not finite.
    pub properties_non_finite: usize,
    /// Properties whose value type differed between elements and was
    /// coerced to the key's one declared type.
    pub properties_coerced: usize,
    /// Standard-set properties written as a flat `bsi::ifc::prop::*` key;
    /// the name of the set they came from is not kept.
    pub properties_flattened_to_v5a: usize,
    /// Elements with a body the mesher could not draw.
    pub bodies_not_drawn: usize,
    /// Elements that had a body through a shared representation map: now
    /// one flat mesh each (informational, nothing is lost).
    pub instanced_bodies_flattened: usize,
    /// Exporter diagnostic values ([`DIAGNOSTIC_KEYS`]) left out of the main
    /// IFCX file and written to its diagnostics layer.
    pub exporter_diagnostics: usize,
}

/// Why IFCX could not be written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IfcxError(pub String);

impl std::fmt::Display for IfcxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ifcx: {}", self.0)
    }
}

impl std::error::Error for IfcxError {}

/// Serialize an `IfcModel` as IFCX text. See the module docs.
pub fn write_ifcx(model: &IfcModel) -> Result<String, IfcxError> {
    write_ifcx_with_ledger(model).map(|(text, _)| text)
}

/// [`write_ifcx`] and the loss ledger of this model.
pub fn write_ifcx_with_ledger(model: &IfcModel) -> Result<(String, LossLedger), IfcxError> {
    write_ifcx_graph_with_ledger(&IfcxGraph::from_model(model, GraphOptions::default()))
}

/// The graph (built with meshes) as IFCX text.
pub fn write_ifcx_graph(graph: &IfcxGraph) -> Result<String, IfcxError> {
    write_ifcx_graph_with_ledger(graph).map(|(text, _)| text)
}

/// [`write_ifcx_graph`] and the loss ledger. The text is the whole graph in
/// one file, exporter diagnostics included: what composing the two files of
/// [`write_ifcx_graph_layers`] gives.
pub fn write_ifcx_graph_with_ledger(graph: &IfcxGraph) -> Result<(String, LossLedger), IfcxError> {
    write_split(graph, &|_| false, default_timestamp()).map(|(text, _, ledger)| (text, ledger))
}

/// Attributes that say where in the RVT a record was found, not what the
/// model is: their values move when Revit lays the file out again
/// (`SourceStream` alone changed on 12,719 of 27,126 nodes between a
/// separate IFCX layer so the main file is stable across saves.
pub const DIAGNOSTIC_KEYS: &[&str] = &["rvtopen::revit::RvtElementRecordGeometry::SourceStream"];

/// The graph as two IFCX layers of one stack, and the loss ledger: the main
/// file without [`DIAGNOSTIC_KEYS`], and a diagnostics layer holding only
/// them (header id `<id>/diagnostics`). Composing the main file with the
/// diagnostics layer on top gives the tree a single file would have.
pub fn write_ifcx_graph_layers(
    graph: &IfcxGraph,
) -> Result<(String, String, LossLedger), IfcxError> {
    write_ifcx_graph_layers_at(graph, None)
}

/// [`write_ifcx_graph_layers`] with the header `timestamp` (Unix seconds,
/// written as ISO 8601 UTC). `None` takes `SOURCE_DATE_EPOCH`, else 0, so the
/// same graph gives the same bytes.
pub fn write_ifcx_graph_layers_at(
    graph: &IfcxGraph,
    timestamp: Option<i64>,
) -> Result<(String, String, LossLedger), IfcxError> {
    write_split(
        graph,
        &|k| DIAGNOSTIC_KEYS.contains(&k),
        timestamp.unwrap_or_else(default_timestamp),
    )
}

/// `SOURCE_DATE_EPOCH` (non-negative Unix seconds) or 0.
fn default_timestamp() -> i64 {
    std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|t| *t >= 0)
        .unwrap_or(0)
}

/// The graph as a main file without the keys `diagnostic` selects, and a
/// layer holding only those.
fn write_split(
    graph: &IfcxGraph,
    diagnostic: &dyn Fn(&str) -> bool,
    timestamp: i64,
) -> Result<(String, String, LossLedger), IfcxError> {
    if let Some(e) = graph.errors.first() {
        return Err(IfcxError(e.clone()));
    }
    let mut data = Vec::with_capacity(graph.nodes.len());
    let mut diag_data = Vec::new();
    let mut ledger = graph.ledger.clone();
    for (path, node) in graph.nodes.iter().filter(|(_, n)| n.ifcx) {
        let mut n = Map::new();
        n.insert("path".into(), json!(path));
        if !node.children.is_empty() {
            n.insert("children".into(), json!(node.children));
        }
        // One inherited node: the type (PCERT's `TypeProduct`), or for a
        // `Body` the shared shape it draws (`Shape`).
        if let [ty] = node.inherits.as_slice() {
            let shape = graph
                .nodes
                .get(ty)
                .is_some_and(|t| t.attributes.contains_key("usd::usdgeom::mesh"));
            let name = if shape { "Shape" } else { "TypeProduct" };
            n.insert("inherits".into(), json!({ name: ty }));
        }
        let (diag, attributes): (Map<String, Value>, Map<String, Value>) = node
            .attributes
            .iter()
            .filter(|(k, _)| !k.starts_with(IFC4_PREFIX))
            .map(|(k, v)| (k.clone(), v.clone()))
            .partition(|(k, _)| diagnostic(k));
        if !attributes.is_empty() {
            n.insert("attributes".into(), Value::Object(attributes));
        }
        data.push(Value::Object(n));
        if !diag.is_empty() {
            ledger.exporter_diagnostics += diag.len();
            diag_data.push(json!({"path": path, "attributes": diag}));
        }
    }
    let schemas = |keep: &dyn Fn(&str) -> bool| -> Map<String, Value> {
        graph
            .schemas
            .iter()
            .filter(|(k, _)| keep(k))
            .map(|(k, t)| (k.clone(), json!({"value": t.schema()})))
            .collect()
    };
    let doc = |id: String, schemas: Map<String, Value>, data: Vec<Value>| {
        let doc = json!({
            "header": {
                "id": id,
                "ifcxVersion": "ifcx_alpha",
                "dataVersion": "1.0.0",
                "author": "rvt-open",
                "timestamp": super::step_writer::iso_timestamp_from(timestamp),
            },
            "imports": [{"uri": IFC_IMPORT}, {"uri": PROP_IMPORT}, {"uri": USD_IMPORT}],
            "schemas": schemas,
            "data": data,
        });
        serde_json::to_string(&doc).expect("a JSON value serializes")
    };
    Ok((
        doc(graph.id.clone(), schemas(&|k| !diagnostic(k)), data),
        doc(
            format!("{}/diagnostics", graph.id),
            schemas(diagnostic),
            diag_data,
        ),
        ledger,
    ))
}

// ---------------------------------------------------------------- ids

/// SHA-1 (RFC 3174), for uuid5 only.
fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for block in msg.chunks_exact(64) {
        let mut w = [0u32; 80];
        for (i, word) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes(word.try_into().expect("4 bytes"));
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let t = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 20];
    for (i, x) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&x.to_be_bytes());
    }
    out
}

pub(super) fn uuid_text(b: [u8; 16]) -> String {
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// uuid5 of `name` under [`IFCX_NAMESPACE`].
pub fn uuid5(name: &str) -> String {
    uuid5_in(IFCX_NAMESPACE, name)
}

/// uuid5 (RFC 4122, SHA-1) of `name` under `namespace`.
pub fn uuid5_in(namespace: [u8; 16], name: &str) -> String {
    let mut data = namespace.to_vec();
    data.extend_from_slice(name.as_bytes());
    let d = sha1(&data);
    let mut b: [u8; 16] = d[..16].try_into().expect("16 bytes");
    b[6] = (b[6] & 0x0f) | 0x50;
    b[8] = (b[8] & 0x3f) | 0x80;
    uuid_text(b)
}

/// An IFC GlobalId (22 characters) as a UUID text.
pub(super) fn ifc_guid_uuid(global_id: &str) -> Result<String, IfcxError> {
    expand_ifc_guid(global_id).map(uuid_text).ok_or_else(|| {
        IfcxError(format!(
            "GlobalId {global_id:?} is not a 22-character IFC id"
        ))
    })
}

/// `IFCBUILDINGELEMENTPROXY` as `IfcBuildingElementProxy`.
pub fn ifc_class_name(upper: &str) -> String {
    const WORDS: &[&str] = &[
        "Air",
        "Alarm",
        "Appliance",
        "Assembly",
        "Bar",
        "Beam",
        "Building",
        "Case",
        "Ceiling",
        "Column",
        "Control",
        "Controller",
        "Covering",
        "Curtain",
        "Device",
        "Distribution",
        "Door",
        "Duct",
        "Electric",
        "Element",
        "Fitting",
        "Fixture",
        "Flight",
        "Flow",
        "Footing",
        "Furnishing",
        "Furniture",
        "Grid",
        "Light",
        "Material",
        "Member",
        "Opening",
        "Pipe",
        "Plate",
        "Proxy",
        "Railing",
        "Ramp",
        "Reinforcing",
        "Roof",
        "Sanitary",
        "Segment",
        "Shading",
        "Slab",
        "Space",
        "Stair",
        "Standard",
        "Storey",
        "Terminal",
        "Transport",
        "Wall",
        "Window",
        "Project",
        "Site",
        "Type",
    ];
    let body = upper.strip_prefix("IFC").unwrap_or(upper);
    // best[i]: a split of body[..i] into words, if there is one.
    let mut best: Vec<Option<Vec<&str>>> = vec![None; body.len() + 1];
    best[0] = Some(Vec::new());
    for i in 1..=body.len() {
        for w in WORDS {
            let n = w.len();
            if n <= i
                && body[i - n..i].eq_ignore_ascii_case(w)
                && let Some(prev) = &best[i - n]
            {
                let mut cand = prev.clone();
                cand.push(w);
                if best[i].as_ref().is_none_or(|b| cand.len() < b.len()) {
                    best[i] = Some(cand);
                }
            }
        }
    }
    match &best[body.len()] {
        Some(words) => format!("Ifc{}", words.concat()),
        None => {
            let lower = body.to_ascii_lowercase();
            let mut c = lower.chars();
            let first = c.next().map(|f| f.to_ascii_uppercase());
            format!("Ifc{}{}", first.unwrap_or_default(), c.as_str())
        }
    }
}

/// A child key: the name made of letters, digits and `_`, then the first
/// eight characters of the node's UUID, so siblings of one name stay apart.
pub(super) fn child_key(name: &str, uuid: &str) -> String {
    let clean: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("{}_{}", clean.trim_matches('_'), &uuid[..8])
}

// -------------------------------------------------------- value types

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Ty {
    Str,
    Int,
    Real(Option<&'static str>),
    Bool,
    List(Box<Ty>),
    /// An object with these fields (georeference values).
    Obj(Vec<(&'static str, Ty)>),
}

impl Ty {
    pub(super) fn schema(&self) -> Value {
        match self {
            Ty::Str => json!({"dataType": "String"}),
            Ty::Int => json!({"dataType": "Integer"}),
            Ty::Bool => json!({"dataType": "Boolean"}),
            Ty::Real(None) => json!({"dataType": "Real"}),
            Ty::Real(Some(q)) => json!({"dataType": "Real", "quantityKind": q}),
            Ty::List(inner) => {
                json!({"dataType": "Array", "arrayRestrictions": {"value": inner.schema()}})
            }
            Ty::Obj(fields) => {
                let values: Map<String, Value> = fields
                    .iter()
                    .map(|(k, t)| ((*k).to_string(), t.schema()))
                    .collect();
                json!({"dataType": "Object", "objectRestrictions": {"values": values}})
            }
        }
    }

    /// The one type that holds values of both, falling back to String.
    pub(super) fn merge(&self, other: &Ty) -> Ty {
        match (self, other) {
            (a, b) if a == b => a.clone(),
            (Ty::Int, Ty::Real(q)) | (Ty::Real(q), Ty::Int) => Ty::Real(*q),
            (Ty::Real(_), Ty::Real(_)) => Ty::Real(None),
            (Ty::List(a), Ty::List(b)) => Ty::List(Box::new(a.merge(b))),
            _ => Ty::Str,
        }
    }
}

/// `value` converted to `to`, which `from` merged into.
pub(super) fn coerce(value: Value, from: &Ty, to: &Ty) -> Value {
    if from == to {
        return value;
    }
    match (to, value) {
        (Ty::Real(_), Value::Number(n)) => json!(n.as_f64()),
        (Ty::List(inner), Value::Array(items)) => {
            let from_inner = match from {
                Ty::List(f) => (**f).clone(),
                other => other.clone(),
            };
            Value::Array(
                items
                    .into_iter()
                    .map(|v| coerce(v, &from_inner, inner))
                    .collect(),
            )
        }
        (Ty::Str, Value::String(s)) => Value::String(s),
        (_, v) => Value::String(match v {
            Value::String(s) => s,
            other => other.to_string(),
        }),
    }
}

pub(super) const FT: f64 = 0.3048;

/// A property value in metres/kilograms, or `None` when IFCX cannot hold it.
pub(super) fn value_of(v: &PropertyValue) -> Option<(Ty, Value)> {
    let real = |x: f64, q: Option<&'static str>| x.is_finite().then(|| (Ty::Real(q), json!(x)));
    match v {
        PropertyValue::Text(s) | PropertyValue::Label(s) | PropertyValue::Identifier(s) => {
            Some((Ty::Str, json!(s)))
        }
        PropertyValue::Integer(n) | PropertyValue::CountValue(n) => Some((Ty::Int, json!(n))),
        PropertyValue::Real(x) => real(*x, None),
        PropertyValue::Boolean(b) => Some((Ty::Bool, json!(b))),
        PropertyValue::Logical(b) => b.map(|b| (Ty::Bool, json!(b))),
        PropertyValue::LengthFeet(x) | PropertyValue::PositiveLengthFeet(x) => {
            real(x * FT, Some("Length"))
        }
        PropertyValue::AngleRadians(x) => real(*x, Some("Plane angle")),
        PropertyValue::AreaSquareFeet(x) => real(x * 0.09290304, Some("Area")),
        PropertyValue::VolumeCubicFeet(x) => real(x * 0.028316846592, Some("Volume")),
        PropertyValue::TimeSeconds(x) => real(*x, Some("Time")),
        PropertyValue::MassPounds(x) => real(x * 0.45359237, Some("Mass")),
        // Already in SI units (W/(m2.K), m3/s, Pa, W, V, A).
        PropertyValue::ThermalTransmittance(x)
        | PropertyValue::VolumetricFlowRate(x)
        | PropertyValue::Pressure(x)
        | PropertyValue::Power(x)
        | PropertyValue::ElectricVoltage(x)
        | PropertyValue::ElectricCurrent(x) => real(*x, None),
        PropertyValue::Enumerated(labels) => Some((Ty::List(Box::new(Ty::Str)), json!(labels))),
        // a layer's quantities, as an object of its parts in SI
        // units and its discrimination.
        PropertyValue::PhysicalComplex {
            discrimination,
            quantities,
        } => {
            let mut fields: Vec<(&'static str, Ty)> = vec![("Discrimination", Ty::Str)];
            let mut value = Map::new();
            value.insert("Discrimination".into(), json!(discrimination));
            for part in quantities {
                let name: &'static str = match part.name.as_str() {
                    "Width" => "Width",
                    "NetArea" => "NetArea",
                    "NetVolume" => "NetVolume",
                    _ => continue,
                };
                if let Some((ty, v)) = value_of(&part.value) {
                    fields.push((name, ty));
                    value.insert(name.into(), v);
                }
            }
            Some((Ty::Obj(fields), Value::Object(value)))
        }
        PropertyValue::List(items) => {
            let parts: Vec<(Ty, Value)> = items.iter().filter_map(value_of).collect();
            let ty = parts
                .iter()
                .map(|(t, _)| t.clone())
                .reduce(|a, b| a.merge(&b))
                .unwrap_or(Ty::Str);
            let values = parts
                .into_iter()
                .map(|(t, v)| coerce(v, &t, &ty))
                .collect::<Vec<_>>();
            Some((Ty::List(Box::new(ty)), Value::Array(values)))
        }
    }
}

/// The attribute key of one property of one set.
pub(super) fn property_key(set: &str, name: &str) -> String {
    if set.starts_with("Pset_") || set.starts_with("Qto_") {
        format!("ifc-src::pset::{set}::{name}")
    } else {
        format!("rvtopen::revit::{set}::{name}")
    }
}

/// The kind of value prop@v5a gives a property (Name and Description
/// excepted), fetched from ifcx.dev; `None` if it defines none.
pub(super) fn v5a_prop(name: &str) -> Option<Ty> {
    Some(match name {
        "UsageType" | "TypeName" => Ty::Str,
        "RefElevation"
        | "ElevationOfRefHeight"
        | "ElevationOfTerrain"
        | "Height"
        | "Width"
        | "Length"
        | "Depth" => Ty::Real(Some("Length")),
        "NumberOfStoreys" => Ty::Int,
        "IsExternal" => Ty::Bool,
        "Volume" | "NetVolume" => Ty::Real(Some("Volume")),
        "NetArea" | "NetSideArea" | "CrossSectionArea" => Ty::Real(Some("Area")),
        "Station" => Ty::Real(None),
        _ => return None,
    })
}

/// `value` as prop@v5a's `want` kind, or `None` when it is another kind.
pub(super) fn as_v5a(want: &Ty, got: &Ty, value: &Value) -> Option<Value> {
    match (want, got) {
        (Ty::Real(_), Ty::Int) => value.as_f64().map(|x| json!(x)),
        (Ty::Real(_), Ty::Real(_))
        | (Ty::Str, Ty::Str)
        | (Ty::Int, Ty::Int)
        | (Ty::Bool, Ty::Bool) => Some(value.clone()),
        _ => None,
    }
}
