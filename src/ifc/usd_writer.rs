//! OpenUSD writer: `write_usda(&IfcModel)`, a third projection of the
//! [`IfcxGraph`] beside the IFC4 STEP and IFCX writers.
//!
//! AOUSD has published no AEC/BIM schema (Core Specification 1.0, December
//! 2025, covers the file format and composition; the AECO interest group,
//! formed July 2024, has no schema out), so the BIM data follows IFC5's own
//! USD alignment: the IFCX keys, with `::` as USD's `:` namespace separator.
//!
//! - `.usda` text, `metersPerUnit = 1`, `upAxis = "Z"`, `defaultPrim` = the
//!   IFCX root;
//! - one prim per IFCX node, the tree of IFCX `children` (root, project,
//!   site, building, storey, element, aggregated member); a prim's name is
//!   `_` + its IFCX path (the element's GlobalId as a UUID) without dashes,
//!   so paths are stable; the IFCX child name is the prim's `displayName`;
//! - nodes are `Xform` prims with no xformOps (placement is baked into the
//!   points, as in IFCX); a `usd::usdgeom::mesh` node is a `Mesh` prim:
//!   `points`, `faceVertexIndices`, `faceVertexCounts` (all 3), `extent`,
//!   `subdivisionScheme = "none"`, and the element's colour as constant
//!   `primvars:displayColor` / `displayOpacity`;
//! - IFC types are `class` prims under `<root>/Types`, and an occurrence
//!   `inherits` its type, as an IFCX occurrence inherits it;
//! - every other IFCX attribute is a USD attribute named by its key
//!   (`bsi::ifc::class` -> `bsi:ifc:class:code` / `:uri`,
//!   `rvtopen::revit::Dimensions::Area` -> `rvtopen:revit:Dimensions:Area`),
//!   each namespace segment made an identifier (other characters `_`), with
//!   the IFCX key in its `customData` as `ifcx`; values typed from the
//!   graph's schemas (string, int64, double, bool, and their arrays).
//!
//!   georeference) gets `matrix4d xformOp:transform` and `xformOpOrder`; the
//!   site's georeference ([`IfcxGraph::georeference_metadata`]) is in the
//!   layer's `customLayerData` as `dictionary georeference`.
//!
//! The same walk feeds `.usda` text ([`write_usda`]) and the binary Crate
//! format ([`write_usdc`], [`super::usd_crate`]): the two hold the
//! same specs, fields and values.
//!
//! Output is a pure function of the graph: byte-identical on a rerun.

use super::IfcModel;
use super::graph::{GraphOptions, IfcxGraph};
use super::ifcx_writer::{IfcxError, Ty, v5a_prop};
use super::usd_crate::{self as crate_fmt, Crate};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fmt::Write as _;

/// Serialize an `IfcModel` as USD text. See the module docs.
pub fn write_usda(model: &IfcModel) -> Result<String, IfcxError> {
    write_usda_graph(&IfcxGraph::from_model(model, GraphOptions::default()))
}

/// The graph (built with meshes) as USD text.
pub fn write_usda_graph(graph: &IfcxGraph) -> Result<String, IfcxError> {
    let mut text = Text::default();
    walk(graph, &mut text)?;
    Ok(text.out)
}

/// Serialize an `IfcModel` as a binary USD layer (`.usdc`). See the module docs.
pub fn write_usdc(model: &IfcModel) -> Result<Vec<u8>, IfcxError> {
    write_usdc_graph(&IfcxGraph::from_model(model, GraphOptions::default()))
}

/// The graph (built with meshes) as a binary USD layer: the same specs as
/// [`write_usda_graph`].
pub fn write_usdc_graph(graph: &IfcxGraph) -> Result<Vec<u8>, IfcxError> {
    let mut sink = CrateSink::new();
    walk(graph, &mut sink)?;
    sink.finish().map_err(IfcxError)
}

/// One USD value as this writer emits it. Numbers stay `f64` here; the
/// float-typed ones (`point3f[]`, `float3[]`, `color3f[]`, `float[]`) become
/// 32-bit where USD stores them.
#[derive(Debug, Clone)]
enum Val {
    Str(String),
    Tok(&'static str),
    Bool(bool),
    I64(i64),
    F64(f64),
    StrArr(Vec<String>),
    BoolArr(Vec<bool>),
    I64Arr(Vec<i64>),
    F64Arr(Vec<f64>),
    /// `int[]`.
    IntArr(Vec<i64>),
    /// `float[]`.
    FloatArr(Vec<f64>),
    /// `point3f[]`, `float3[]`, `color3f[]`.
    Vec3fArr(Vec<[f64; 3]>),
    /// `matrix4d`, row-major.
    Matrix4d([f64; 16]),
    /// `token[]`.
    TokArr(Vec<&'static str>),
}

/// One attribute spec.
struct Attr<'a> {
    name: &'a str,
    type_name: &'a str,
    uniform: bool,
    value: Val,
    /// The IFCX key, as `customData.ifcx`.
    ifcx: Option<&'a str>,
    interpolation: Option<&'static str>,
}

/// A prim's head: `def <type>` or an untyped `class`.
#[derive(Clone, Copy)]
enum Head {
    Def(&'static str),
    Class,
}

/// What the walk writes to: `.usda` text or a Crate layer.
trait Sink {
    fn layer(&mut self, default_prim: &str, ifcx_id: &str, georeference: Option<&Value>);
    fn prim(&mut self, head: Head, name: &str, display: Option<&str>, inherits: &[String]);
    fn attr(&mut self, a: Attr);
    fn end(&mut self);
}

fn walk(graph: &IfcxGraph, sink: &mut dyn Sink) -> Result<(), IfcxError> {
    if let Some(e) = graph.errors.first() {
        return Err(IfcxError(e.clone()));
    }
    let mut parented = BTreeSet::new();
    let mut types: BTreeSet<&str> = graph.type_nodes.iter().map(String::as_str).collect();
    for node in graph.nodes.values().filter(|n| n.ifcx) {
        parented.extend(node.children.values().map(String::as_str));
        types.extend(node.inherits.iter().map(String::as_str));
    }
    if let Some(p) = graph.nodes.iter().find_map(|(p, n)| {
        (n.ifcx
            && p != &graph.root
            && !parented.contains(p.as_str())
            && !types.contains(p.as_str()))
        .then_some(p)
    }) {
        return Err(IfcxError(format!("node {p} hangs from nothing")));
    }
    let root = prim_name(&graph.root);
    let w = Writer {
        graph,
        types_path: format!("/{root}/Types"),
        types,
    };
    sink.layer(&root, &graph.id, graph.georeference_metadata().as_ref());
    w.prim(sink, &graph.root, Head::Def("Xform"), "rvt-open", None);
    Ok(())
}

struct Writer<'a> {
    graph: &'a IfcxGraph,
    types_path: String,
    /// The type nodes: `class` prims under `types_path`.
    types: BTreeSet<&'a str>,
}

impl Writer<'_> {
    fn shaped_node(&self, path: &str) -> bool {
        self.graph.nodes[path].mesh.is_some()
    }

    /// A node without a mesh of its own that inherits a shared shape.
    fn shaped(&self, path: &str) -> bool {
        let node = &self.graph.nodes[path];
        node.mesh.is_none()
            && node
                .inherits
                .iter()
                .any(|t| self.graph.nodes.get(t).is_some_and(|n| n.mesh.is_some()))
    }

    fn prim(
        &self,
        sink: &mut dyn Sink,
        path: &str,
        head: Head,
        display: &str,
        colour: Option<&Colour>,
    ) {
        let node = &self.graph.nodes[path];
        let inherits: Vec<String> = node
            .inherits
            .iter()
            .map(|t| format!("{}/{}", self.types_path, prim_name(t)))
            .collect();
        sink.prim(head, &prim_name(path), Some(display), &inherits);
        let mut names = BTreeSet::new();
        for (key, value) in &node.attributes {
            if key.starts_with(super::graph::IFC4_PREFIX) {
                continue;
            }
            if key == "usd::xformop" {
                transform_op(sink, value);
                continue;
            }
            self.attribute(sink, &mut names, key, key, value);
        }
        // a Body of one face material carries its own colour.
        let own = colour_of(node);
        let colour = own.as_ref().or(colour);
        if let Some(mesh) = node.mesh.as_deref() {
            mesh_body(sink, mesh, colour);
        } else if self.shaped(path) {
            // the points come from the inherited shape class.
            colour_attrs(sink, colour);
        }
        for (name, child) in &node.children {
            let head = if self.graph.nodes[child].mesh.is_some() || self.shaped(child) {
                Head::Def("Mesh")
            } else {
                Head::Def("Xform")
            };
            self.prim(sink, child, head, name, own.as_ref());
        }
        if path == self.graph.root && !self.types.is_empty() {
            sink.prim(Head::Def("Scope"), "Types", None, &[]);
            for t in &self.types {
                let name = self.graph.nodes[*t]
                    .attributes
                    .get("bsi::ifc::prop::Name")
                    .and_then(Value::as_str)
                    .unwrap_or(if self.shaped_node(t) { "Shape" } else { "Type" });
                self.prim(sink, t, Head::Class, name, None);
            }
            sink.end();
        }
        sink.end();
    }

    /// One IFCX attribute; an object value is one USD attribute per field.
    fn attribute(
        &self,
        sink: &mut dyn Sink,
        names: &mut BTreeSet<String>,
        key: &str,
        name_key: &str,
        value: &Value,
    ) {
        if let Value::Object(fields) = value {
            for (f, v) in fields {
                self.attribute(sink, names, key, &format!("{name_key}::{f}"), v);
            }
            return;
        }
        let base: Vec<String> = name_key.split("::").map(identifier).collect();
        let base = base.join(":");
        let mut name = base.clone();
        // ponytail: two keys that differ only in characters USD names cannot
        // hold get `_2`, `_3`; the IFCX key in customData keeps them apart.
        let mut n = 1;
        while !names.insert(name.clone()) {
            n += 1;
            name = format!("{base}_{n}");
        }
        let ty = self
            .graph
            .schemas
            .get(key)
            .cloned()
            .or_else(|| key.strip_prefix("bsi::ifc::prop::").and_then(v5a_prop))
            .filter(|_| key == name_key);
        let (type_name, value) = typed(ty.as_ref(), value);
        sink.attr(Attr {
            name: &name,
            type_name,
            uniform: false,
            value,
            ifcx: Some(key),
            interpolation: None,
        });
    }
}

/// An element's `diffuseColor` and `opacity`.
type Colour = (Value, Option<f64>);

fn colour_of(node: &super::graph::Node) -> Option<Colour> {
    let rgb = node
        .attributes
        .get("bsi::ifc::presentation::diffuseColor")?
        .clone();
    let a = node
        .attributes
        .get("bsi::ifc::presentation::opacity")
        .and_then(Value::as_f64);
    Some((rgb, a))
}

/// `points`, triangles, extent, and the element colour.
fn mesh_body(sink: &mut dyn Sink, mesh: &super::graph::UsdMesh, colour: Option<&Colour>) {
    let points: Vec<[f64; 3]> = mesh.finite_points().collect();
    let indices: Vec<i64> = mesh
        .face_vertex_indices
        .iter()
        .map(|&i| i64::from(i))
        .collect();
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for p in &points {
        for i in 0..3 {
            lo[i] = lo[i].min(p[i]);
            hi[i] = hi[i].max(p[i]);
        }
    }
    let plain = |name, type_name, uniform, value| Attr {
        name,
        type_name,
        uniform,
        value,
        ifcx: None,
        interpolation: None,
    };
    // Flat shading: one unit normal per face (a viewer that does not compute
    // normals draws a mesh without them black).
    let normals: Vec<[f64; 3]> = indices
        .chunks_exact(3)
        .map(|t| {
            let at = |k: usize| points.get(t[k] as usize).copied().unwrap_or([0.0; 3]);
            let (a, b, c) = (at(0), at(1), at(2));
            let (u, w) = (
                [b[0] - a[0], b[1] - a[1], b[2] - a[2]],
                [c[0] - a[0], c[1] - a[1], c[2] - a[2]],
            );
            let n = [
                u[1] * w[2] - u[2] * w[1],
                u[2] * w[0] - u[0] * w[2],
                u[0] * w[1] - u[1] * w[0],
            ];
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            if len.is_finite() && len > 1e-30 {
                [n[0] / len, n[1] / len, n[2] / len]
            } else {
                [0.0, 0.0, 1.0]
            }
        })
        .collect();
    sink.attr(plain("subdivisionScheme", "token", true, Val::Tok("none")));
    sink.attr(plain(
        "faceVertexCounts",
        "int[]",
        false,
        Val::IntArr(vec![3; indices.len() / 3]),
    ));
    sink.attr(plain(
        "faceVertexIndices",
        "int[]",
        false,
        Val::IntArr(indices),
    ));
    let empty = points.is_empty();
    sink.attr(plain("points", "point3f[]", false, Val::Vec3fArr(points)));
    if !normals.is_empty() {
        sink.attr(Attr {
            interpolation: Some("uniform"),
            ..plain("normals", "normal3f[]", false, Val::Vec3fArr(normals))
        });
    }
    if !empty {
        sink.attr(plain(
            "extent",
            "float3[]",
            false,
            Val::Vec3fArr(vec![lo, hi]),
        ));
    }
    colour_attrs(sink, colour);
}

/// `primvars:displayColor` (and opacity) of a mesh prim.
fn colour_attrs(sink: &mut dyn Sink, colour: Option<&Colour>) {
    let plain = |name, type_name, uniform, value| Attr {
        name,
        type_name,
        uniform,
        value,
        ifcx: None,
        interpolation: None,
    };
    if let Some((rgb, opacity)) = colour {
        let c = |i: usize| rgb[i].as_f64().unwrap_or(0.0);
        sink.attr(Attr {
            interpolation: Some("constant"),
            ..plain(
                "primvars:displayColor",
                "color3f[]",
                false,
                Val::Vec3fArr(vec![[c(0), c(1), c(2)]]),
            )
        });
        if let Some(a) = opacity {
            sink.attr(Attr {
                interpolation: Some("constant"),
                ..plain(
                    "primvars:displayOpacity",
                    "float[]",
                    false,
                    Val::FloatArr(vec![*a]),
                )
            });
        }
    }
}

/// The USD type and value of one value, of the key's declared type when it
/// has one.
fn typed(ty: Option<&Ty>, value: &Value) -> (&'static str, Val) {
    let inferred = |v: &Value| -> Ty {
        match v {
            Value::Bool(_) => Ty::Bool,
            Value::Number(n) if n.is_i64() || n.is_u64() => Ty::Int,
            Value::Number(_) => Ty::Real(None),
            _ => Ty::Str,
        }
    };
    let ty = ty.cloned().unwrap_or_else(|| match value {
        Value::Array(items) => Ty::List(Box::new(
            items
                .iter()
                .map(inferred)
                .reduce(|a, b| a.merge(&b))
                .unwrap_or(Ty::Str),
        )),
        v => inferred(v),
    });
    if let (Ty::List(inner), Value::Array(items)) = (&ty, value) {
        let arr = match inner.as_ref() {
            Ty::Str => items
                .iter()
                .map(|v| v.as_str().map(String::from))
                .collect::<Option<Vec<_>>>()
                .map(|v| ("string[]", Val::StrArr(v))),
            Ty::Int => items
                .iter()
                .map(Value::as_i64)
                .collect::<Option<Vec<_>>>()
                .map(|v| ("int64[]", Val::I64Arr(v))),
            Ty::Real(_) => items
                .iter()
                .map(Value::as_f64)
                .collect::<Option<Vec<_>>>()
                .map(|v| ("double[]", Val::F64Arr(v))),
            Ty::Bool => items
                .iter()
                .map(Value::as_bool)
                .collect::<Option<Vec<_>>>()
                .map(|v| ("bool[]", Val::BoolArr(v))),
            Ty::List(_) | Ty::Obj(_) => None,
        };
        return arr.unwrap_or_else(|| {
            (
                "string[]",
                Val::StrArr(items.iter().map(json_text).collect()),
            )
        });
    }
    let scalar = match (&ty, value) {
        (Ty::Str, Value::String(s)) => Some(("string", Val::Str(s.clone()))),
        (Ty::Int, Value::Number(n)) => n.as_i64().map(|i| ("int64", Val::I64(i))),
        (Ty::Real(_), Value::Number(n)) => n.as_f64().map(|x| ("double", Val::F64(x))),
        (Ty::Bool, Value::Bool(b)) => Some(("bool", Val::Bool(*b))),
        _ => None,
    };
    scalar.unwrap_or_else(|| ("string", Val::Str(json_text(value))))
}

/// an IFCX `usd::xformop` (`transform`, row-major 4x4, translation
/// in the last row, as USD's `matrix4d`) as the prim's `xformOp:transform`.
fn transform_op(sink: &mut dyn Sink, value: &Value) {
    let Some(rows) = value.get("transform").and_then(Value::as_array) else {
        return;
    };
    let mut m = [0.0; 16];
    for (i, x) in rows
        .iter()
        .filter_map(Value::as_array)
        .flatten()
        .take(16)
        .enumerate()
    {
        m[i] = x.as_f64().unwrap_or(0.0);
    }
    sink.attr(Attr {
        name: "xformOp:transform",
        type_name: "matrix4d",
        uniform: false,
        value: Val::Matrix4d(m),
        ifcx: Some("usd::xformop"),
        interpolation: None,
    });
    sink.attr(Attr {
        name: "xformOpOrder",
        type_name: "token[]",
        uniform: true,
        value: Val::TokArr(vec!["xformOp:transform"]),
        ifcx: None,
        interpolation: None,
    });
}

/// A JSON value of layer metadata as `(entry name, USD type, value)` leaves,
/// objects as nested dictionaries.
enum Meta {
    Leaf(&'static str, Val),
    Dict(Vec<(String, Meta)>),
}

fn meta(value: &Value) -> Meta {
    match value {
        Value::Object(fields) => Meta::Dict(
            fields
                .iter()
                .map(|(k, v)| (identifier(k), meta(v)))
                .collect(),
        ),
        v => {
            let (t, v) = typed(None, v);
            Meta::Leaf(t, v)
        }
    }
}

fn val_text(v: &Val) -> String {
    let tuple = |p: &[f64; 3]| format!("({}, {}, {})", p[0], p[1], p[2]);
    let list = |v: Vec<String>| format!("[{}]", v.join(", "));
    match v {
        Val::Str(s) => quote(s),
        Val::Tok(t) => quote(t),
        Val::Bool(b) => b.to_string(),
        Val::I64(i) => i.to_string(),
        Val::F64(x) => real(*x),
        Val::StrArr(v) => list(v.iter().map(|s| quote(s)).collect()),
        Val::BoolArr(v) => list(v.iter().map(bool::to_string).collect()),
        Val::I64Arr(v) | Val::IntArr(v) => list(v.iter().map(i64::to_string).collect()),
        Val::F64Arr(v) => list(v.iter().map(|x| real(*x)).collect()),
        Val::FloatArr(v) => list(v.iter().map(f64::to_string).collect()),
        Val::Vec3fArr(v) => list(v.iter().map(tuple).collect()),
        Val::Matrix4d(m) => format!(
            "({})",
            m.chunks(4)
                .map(|r| format!(
                    "({})",
                    r.iter().map(|x| real(*x)).collect::<Vec<_>>().join(", ")
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Val::TokArr(v) => list(v.iter().map(|s| quote(s)).collect()),
    }
}

fn meta_text(out: &mut String, pad: &str, name: &str, m: &Meta) {
    match m {
        Meta::Dict(fields) => {
            let _ = writeln!(out, "{pad}dictionary {name} = {{");
            for (k, v) in fields {
                meta_text(out, &format!("{pad}    "), k, v);
            }
            let _ = writeln!(out, "{pad}}}");
        }
        Meta::Leaf(t, v) => {
            let _ = writeln!(out, "{pad}{t} {name} = {}", val_text(v));
        }
    }
}

fn json_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

// ------------------------------------------------------------------ text

#[derive(Default)]
struct Text {
    out: String,
    depth: usize,
}

impl Text {
    fn pad(&self) -> String {
        "    ".repeat(self.depth)
    }
}

impl Sink for Text {
    fn layer(&mut self, default_prim: &str, ifcx_id: &str, georeference: Option<&Value>) {
        let mut geo = String::new();
        if let Some(g) = georeference {
            meta_text(&mut geo, "        ", "georeference", &meta(g));
        }
        let _ = write!(
            self.out,
            "#usda 1.0\n(\n    defaultPrim = \"{default_prim}\"\n    doc = \"rvt-open\"\n    metersPerUnit = 1\n    upAxis = \"Z\"\n    customLayerData = {{\n        string ifcxId = {}\n{geo}    }}\n)\n\n",
            quote(ifcx_id)
        );
    }

    fn prim(&mut self, head: Head, name: &str, display: Option<&str>, inherits: &[String]) {
        let pad = self.pad();
        let head = match head {
            Head::Def(t) => format!("def {t}"),
            Head::Class => "class".into(),
        };
        match display {
            Some(d) => {
                let _ = write!(
                    self.out,
                    "{pad}{head} \"{name}\" (\n{pad}    displayName = {}\n",
                    quote(d)
                );
                if !inherits.is_empty() {
                    let targets: Vec<String> = inherits.iter().map(|t| format!("<{t}>")).collect();
                    let _ = writeln!(
                        self.out,
                        "{pad}    prepend inherits = [{}]",
                        targets.join(", ")
                    );
                }
                let _ = writeln!(self.out, "{pad})\n{pad}{{");
            }
            None => {
                let _ = writeln!(self.out, "{pad}{head} \"{name}\"\n{pad}{{");
            }
        }
        self.depth += 1;
    }

    fn attr(&mut self, a: Attr) {
        let pad = self.pad();
        let text = val_text(&a.value);
        let uniform = if a.uniform { "uniform " } else { "" };
        let _ = write!(
            self.out,
            "{pad}{uniform}{} {} = {text}",
            a.type_name, a.name
        );
        if let Some(k) = a.ifcx {
            let _ = write!(
                self.out,
                " (\n{pad}    customData = {{\n{pad}        string ifcx = {}\n{pad}    }}\n{pad})",
                quote(k)
            );
        } else if let Some(i) = a.interpolation {
            let _ = write!(self.out, " (\n{pad}    interpolation = \"{i}\"\n{pad})");
        }
        self.out.push('\n');
    }

    fn end(&mut self) {
        self.depth -= 1;
        let pad = self.pad();
        let _ = writeln!(self.out, "{pad}}}");
    }
}

// ----------------------------------------------------------------- crate

/// An open prim: its path, fields so far, and the names of its children and
/// properties (written as `primChildren` / `properties` when it closes).
struct OpenPrim {
    path: u32,
    fields: Vec<(&'static str, u64)>,
    children: Vec<u32>,
    properties: Vec<u32>,
    inherits: Vec<String>,
}

/// A spec's `(field name, value)` list.
type Fields = Vec<(&'static str, u64)>;

struct CrateSink {
    c: Crate,
    stack: Vec<OpenPrim>,
    /// Absolute prim path text to path index, for `inherits` targets.
    path_ix: std::collections::HashMap<String, u32>,
    /// Absolute path text of each open prim.
    texts: Vec<String>,
    /// Inherit targets (a type prim may close after its occurrences), resolved
    /// in `finish`: (spec index, absolute target paths).
    pending: Vec<(usize, Vec<String>)>,
    /// Specs held until the inherit targets resolve: (path, form, fields).
    specs: Vec<(u32, u32, Fields)>,
}

impl CrateSink {
    fn value(&mut self, v: &Val) -> u64 {
        let c = &mut self.c;
        match v {
            Val::Str(s) => c.string_value(s),
            Val::Tok(t) => c.token_value(t),
            Val::Bool(b) => Crate::bool(*b),
            Val::I64(i) => c.int64(*i),
            Val::F64(x) => c.double(*x),
            Val::StrArr(v) => c.string_array(v),
            Val::BoolArr(v) => c.bool_array(v),
            Val::I64Arr(v) => c.int_array(crate_fmt::ty::INT64, v),
            Val::IntArr(v) => c.int_array(crate_fmt::ty::INT, v),
            Val::F64Arr(v) => c.double_array(v),
            Val::FloatArr(v) => c.float_array(v),
            Val::Vec3fArr(v) => c.float3_array(v),
            Val::Matrix4d(m) => c.matrix4d(m),
            Val::TokArr(v) => c.token_array(v),
        }
    }

    /// layer metadata, objects as nested dictionaries.
    fn meta_value(&mut self, m: &Meta) -> u64 {
        match m {
            Meta::Leaf(_, v) => self.value(v),
            Meta::Dict(fields) => {
                let reps: Vec<(String, u64)> = fields
                    .iter()
                    .map(|(k, v)| (k.clone(), self.meta_value(v)))
                    .collect();
                let entries: Vec<(&str, u64)> =
                    reps.iter().map(|(k, v)| (k.as_str(), *v)).collect();
                self.c.dictionary(&entries)
            }
        }
    }

    fn new() -> Self {
        CrateSink {
            c: Crate::new(),
            stack: vec![OpenPrim {
                path: 0,
                fields: Vec::new(),
                children: Vec::new(),
                properties: Vec::new(),
                inherits: Vec::new(),
            }],
            path_ix: Default::default(),
            texts: vec![String::new()],
            pending: Vec::new(),
            specs: Vec::new(),
        }
    }

    fn finish(mut self) -> Result<Vec<u8>, String> {
        // The layer spec: the bottom of the stack, never closed by `end`.
        let mut layer = self.stack.pop().ok_or("usdc: unbalanced prims")?;
        if !self.stack.is_empty() {
            return Err("usdc: unbalanced prims".into());
        }
        let kids = self.c.token_vector(&layer.children);
        layer.fields.push(("primChildren", kids));
        self.specs.push((0, crate_fmt::FORM_LAYER, layer.fields));
        for (spec, targets) in std::mem::take(&mut self.pending) {
            let ix = targets
                .iter()
                .map(|t| {
                    self.path_ix
                        .get(t)
                        .copied()
                        .ok_or_else(|| format!("usdc: inherits target {t} is no prim"))
                })
                .collect::<Result<Vec<u32>, String>>()?;
            let rep = self.c.prepend_paths(&ix);
            self.specs[spec].2.push(("inheritPaths", rep));
        }
        for (path, form, fields) in std::mem::take(&mut self.specs) {
            self.c.spec(path, form, &fields);
        }
        self.c.finish()
    }
}

impl Sink for CrateSink {
    fn layer(&mut self, default_prim: &str, ifcx_id: &str, georeference: Option<&Value>) {
        let geo = georeference.map(|g| self.meta_value(&meta(g)));
        let c = &mut self.c;
        let id = c.string_value(ifcx_id);
        let fields = vec![
            ("defaultPrim", c.token_value(default_prim)),
            ("documentation", c.string_value("rvt-open")),
            ("metersPerUnit", c.double(1.0)),
            ("upAxis", c.token_value("Z")),
            ("customLayerData", {
                let mut entries = vec![("ifcxId", id)];
                if let Some(g) = geo {
                    entries.push(("georeference", g));
                }
                c.dictionary(&entries)
            }),
        ];
        self.stack[0].fields = fields;
    }

    fn prim(&mut self, head: Head, name: &str, display: Option<&str>, inherits: &[String]) {
        let parent = self.stack.last().map_or(0, |p| p.path);
        let path = self.c.path(parent, name, false);
        let tok = self.c.token(name);
        if let Some(p) = self.stack.last_mut() {
            p.children.push(tok);
        }
        let text = format!("{}/{name}", self.texts.last().map_or("", String::as_str));
        self.path_ix.insert(text.clone(), path);
        self.texts.push(text);
        let mut fields = Vec::new();
        match head {
            Head::Def(t) => {
                fields.push(("specifier", Crate::specifier(0)));
                fields.push(("typeName", self.c.token_value(t)));
            }
            Head::Class => fields.push(("specifier", Crate::specifier(2))),
        }
        if let Some(d) = display {
            fields.push(("displayName", self.c.string_value(d)));
        }
        self.stack.push(OpenPrim {
            path,
            fields,
            children: Vec::new(),
            properties: Vec::new(),
            inherits: inherits.to_vec(),
        });
    }

    fn attr(&mut self, a: Attr) {
        let parent = self.stack.last().map_or(0, |p| p.path);
        let path = self.c.path(parent, a.name, true);
        let tok = self.c.token(a.name);
        if let Some(p) = self.stack.last_mut() {
            p.properties.push(tok);
        }
        let value = self.value(&a.value);
        let c = &mut self.c;
        let mut fields = vec![
            ("custom", Crate::bool(false)),
            ("typeName", c.token_value(a.type_name)),
            ("variability", Crate::variability(i32::from(a.uniform))),
            ("default", value),
        ];
        if let Some(k) = a.ifcx {
            let v = c.string_value(k);
            fields.push(("customData", c.dictionary(&[("ifcx", v)])));
        }
        if let Some(i) = a.interpolation {
            fields.push(("interpolation", c.token_value(i)));
        }
        self.specs.push((path, crate_fmt::FORM_ATTRIBUTE, fields));
    }

    fn end(&mut self) {
        let Some(mut p) = self.stack.pop() else {
            return;
        };
        self.texts.pop();
        if !p.children.is_empty() {
            let v = self.c.token_vector(&p.children);
            p.fields.push(("primChildren", v));
        }
        if !p.properties.is_empty() {
            let v = self.c.token_vector(&p.properties);
            p.fields.push(("properties", v));
        }
        if !p.inherits.is_empty() {
            self.pending.push((self.specs.len(), p.inherits));
        }
        self.specs.push((p.path, crate_fmt::FORM_PRIM, p.fields));
    }
}

fn real(x: f64) -> String {
    let s = format!("{x}");
    if s.contains(['.', 'e', 'E']) || s.contains("inf") || s.contains("NaN") {
        s
    } else {
        format!("{s}.0")
    }
}

/// `_` + the IFCX path with only its letters, digits and `_`.
fn prim_name(path: &str) -> String {
    let body: String = path
        .chars()
        .filter(|c| *c != '-')
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("_{body}")
}

/// A USD identifier: letters, digits and `_`, not starting with a digit.
fn identifier(segment: &str) -> String {
    let s: String = segment
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    match s.chars().next() {
        None => "_".into(),
        Some(c) if c.is_ascii_digit() => format!("_{s}"),
        _ => s,
    }
}

/// A `.usda` string literal.
fn quote(s: &str) -> String {
    let mut q = String::with_capacity(s.len() + 2);
    q.push('"');
    for c in s.chars() {
        match c {
            '"' => q.push_str("\\\""),
            '\\' => q.push_str("\\\\"),
            '\n' => q.push_str("\\n"),
            '\t' => q.push_str("\\t"),
            '\r' => q.push_str("\\r"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                let _ = write!(q, "\\x{:02x}", c as u32);
            }
            c => q.push(c),
        }
    }
    q.push('"');
    q
}
