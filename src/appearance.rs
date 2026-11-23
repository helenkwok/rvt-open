//! Render appearance assets as stored: each material's appearance asset as a
//! lossless record.
//!
//! A Revit appearance asset is a tree of typed properties. Its schema
//! (`BaseSchema`: `PrismOpaqueSchema`, `HardwoodSchema`, `GenericSchema`, ...)
//! names what it is; each property has a name, a kind (`Boolean`, `Integer`,
//! `Enum`, `Float`, `Double4`, `Distance`, `String`, `Reference`, ...) and a
//! value; a property can have child properties and connected assets, which
//! are the texture slots (a `UnifiedBitmapSchema` image with its real-world
//! scale, offset, rotation and repeat, a `BumpMapSchema`, a procedural such
//! as `NoiseSchema` with its parameters). The public Revit API reads the same
//! tree (`Asset`, `AssetProperty.GetConnectedProperty`), under the same names.
//!
//! [`read`] keeps every property and every field of the stored objects,
//! names and kinds as saved, with values as JSON: nothing is dropped or
//! translated. What it cannot read is listed in the `unread` field of [`Appearance`]. The
//! images a bitmap names are not in the file (they are paths into a material
//! library); only a swatch is.

use crate::graph_value::Fv;
use crate::native_parameters::ObjectGraph;
use crate::{RevitFile, native_document};
use serde::{Serialize, Serializer};
use serde_json::{Map, Value, json};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};

/// The values one record's graph may hold: 16 times the largest record seen
/// (a Snowdon Towers asset of about 110,000 values, its swatch one value per
/// byte). It bounds the decoder's work on any record, as the default budget
/// of 100,000 does for every other reader.
const GRAPH_VALUES: usize = 2_000_000;

/// The objects and values [`read`] visits for one record. A graph whose
/// pointers are shared or cyclic stops here (and the record is listed unread)
/// instead of growing without bound.
const NODE_BUDGET: usize = 1_000_000;

/// The appearance assets of a file and the materials that use them.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Appearance {
    /// Every material, with the appearance asset it names.
    pub materials: Vec<MaterialAppearance>,
    /// Every appearance asset read, by element id.
    pub assets: Vec<AppearanceAsset>,
    /// Material and appearance asset records not read, or read only in part,
    /// and why.
    pub unread: Vec<UnreadAsset>,
}

/// A material and its appearance.
#[derive(Debug, Clone, Serialize)]
pub struct MaterialAppearance {
    /// The material's element id.
    pub material_id: u64,
    /// The material's name.
    pub name: String,
    /// The element id of its appearance asset, where it names one.
    pub appearance_asset_id: Option<i64>,
    /// Whether the material is drawn with its render appearance's colour
    /// ("Use Render Appearance").
    pub use_render_appearance: Option<bool>,
    /// The material's own asset, as saved in the material.
    pub own_asset: Option<Asset>,
    /// The material object's other fields, as saved.
    pub fields: Map<String, Value>,
    /// The material element's other fields, as saved.
    pub element_fields: Map<String, Value>,
}

/// An appearance asset element.
#[derive(Debug, Clone, Serialize)]
pub struct AppearanceAsset {
    /// The element id.
    pub element_id: u64,
    /// The asset's name.
    pub name: Option<String>,
    /// Its colour as saved, `0x00BBGGRR`, where it is one (any other value is
    /// kept in `fields`).
    pub color_packed: Option<u32>,
    /// Its transparency as saved.
    pub transparency: Option<f64>,
    /// The property tree.
    pub asset: Asset,
    /// The swatch image saved with it, where there is one (a thumbnail that
    /// is not one is kept in `element_fields`).
    pub swatch: Option<Swatch>,
    /// The appearance asset object's other fields, as saved.
    pub fields: Map<String, Value>,
    /// The element's other fields (its material path map, ...), as saved.
    pub element_fields: Map<String, Value>,
}

/// An asset: a schema and its properties.
#[derive(Debug, Clone, Serialize)]
pub struct Asset {
    /// The class of the saved object.
    pub class: String,
    /// The value of its `BaseSchema` property.
    pub schema: Option<String>,
    /// Its properties, in saved order.
    pub properties: Vec<AssetProperty>,
    /// The object's other fields (library, scene, asset type, ...), as saved.
    pub fields: Map<String, Value>,
}

/// A property of an asset.
#[derive(Debug, Clone, Serialize)]
pub struct AssetProperty {
    /// Its name (`unifiedbitmap_Bitmap`, `texture_RealWorldScaleX`, ...).
    pub name: String,
    /// Its kind: the saved class without its `AProperty` prefix (`String`,
    /// `Double4`, `Distance`, ...), or the whole class name for any other.
    pub kind: String,
    /// Its value, as saved (`m_value`); `null` where it has none.
    pub value: Value,
    /// Child properties (a list's items).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<AssetProperty>,
    /// Connected assets: the texture or procedural in this slot.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub connected: Vec<Asset>,
    /// Its other fields (a distance's unit, ...), as saved.
    #[serde(skip_serializing_if = "Map::is_empty")]
    pub fields: Map<String, Value>,
}

/// A swatch image saved with an appearance asset.
#[derive(Debug, Clone, Serialize)]
pub struct Swatch {
    /// Width in pixels.
    pub width: Option<u64>,
    /// Height in pixels.
    pub height: Option<u64>,
    /// The image file's bytes (PNG), written as base64 in JSON.
    #[serde(serialize_with = "base64")]
    pub image: Vec<u8>,
    /// The image object's other fields (dpi, size in feet), as saved.
    pub fields: Map<String, Value>,
}

/// A material or appearance asset record not read, or read only in part.
#[derive(Debug, Clone, Serialize)]
pub struct UnreadAsset {
    /// The element id.
    pub element_id: u64,
    /// The record's class.
    pub class: String,
    /// Why.
    pub reason: String,
}

impl Asset {
    /// The property `name`, where the asset has it.
    pub fn property(&self, name: &str) -> Option<&AssetProperty> {
        self.properties.iter().find(|p| p.name == name)
    }

    /// Every asset connected below this one, at any depth.
    pub fn connected(&self) -> Vec<&Asset> {
        let mut out = Vec::new();
        let mut stack: Vec<&AssetProperty> = self.properties.iter().rev().collect();
        while let Some(p) = stack.pop() {
            for a in &p.connected {
                out.push(a);
                stack.extend(a.properties.iter().rev());
            }
            stack.extend(p.children.iter().rev());
        }
        out
    }
}

impl Appearance {
    /// The appearance asset of the material named `name`: none where no
    /// material has the name, or where materials of that name name different
    /// assets.
    pub fn asset_of(&self, name: &str) -> Option<&AppearanceAsset> {
        let mut ids = self
            .materials
            .iter()
            .filter(|m| m.name == name)
            .map(|m| m.appearance_asset_id);
        let id = ids.next()??;
        if ids.any(|other| other != Some(id)) {
            return None;
        }
        self.asset(id)
    }

    /// The appearance asset whose element id is `id`.
    pub fn asset(&self, id: i64) -> Option<&AppearanceAsset> {
        self.assets
            .iter()
            .find(|a| i64::try_from(a.element_id) == Ok(id))
    }

    /// Add one record's graph (a `MaterialElem` or `AppearanceAssetElem`);
    /// what [`read`] does for each record. With `unstable-research`, for
    /// graphs made by hand.
    #[cfg(feature = "unstable-research")]
    pub fn add_graph(&mut self, element_id: u64, class: &str, graph: &ObjectGraph) {
        self.ingest(element_id, class, graph);
    }

    fn unread(&mut self, element_id: u64, class: &str, reason: impl Into<String>) {
        self.unread.push(UnreadAsset {
            element_id,
            class: class.to_string(),
            reason: reason.into(),
        });
    }

    fn ingest(&mut self, id: u64, class: &str, g: &ObjectGraph) {
        let reader = Reader::new(g);
        let Some(root) = g.objects.first() else {
            return self.unread(id, class, "graph has no objects");
        };
        if root.class_name != class {
            return self.unread(id, class, format!("first object is {}", root.class_name));
        }
        reader.enter(0);
        match class {
            "MaterialElem" => {
                let Some((i, m)) = reader.target(0, &root.fields["m_pMaterial"]) else {
                    return self.unread(id, class, "no material object");
                };
                reader.enter(i);
                let own_asset = m
                    .get("m_asset")
                    .filter(|a| a.is_object())
                    .map(|a| reader.asset(i, "Asset", a));
                let material = MaterialAppearance {
                    material_id: id,
                    name: m["m_name"].as_str().unwrap_or("").to_string(),
                    appearance_asset_id: crate::native_json::identifier_i64(
                        &m["m_appearanceAssetId"],
                    ),
                    use_render_appearance: m["m_bUseRenderAppearance"].as_bool(),
                    own_asset,
                    fields: reader.fields(i, m, &["m_name", "m_asset"]),
                    element_fields: reader.fields(0, &root.fields, &["m_pMaterial"]),
                };
                self.materials.push(material);
            }
            "AppearanceAssetElem" => {
                let Some((i, a)) = reader.target(0, &root.fields["m_pAppearanceAsset"]) else {
                    return self.unread(id, class, "no appearance asset object");
                };
                reader.enter(i);
                let Some(tree) = a.get("m_Asset").filter(|t| t.is_object()) else {
                    return self.unread(id, class, "appearance asset has no m_Asset");
                };
                let asset = reader.asset(i, "Asset", tree);
                let swatch = root
                    .fields
                    .get("m_pThumbnail")
                    .and_then(|p| reader.target(0, p))
                    .and_then(|(_, t)| swatch(t));
                let color_packed = a["m_color"].as_u64().and_then(|c| u32::try_from(c).ok());
                let mut fields = reader.fields(i, a, &["m_Name", "m_Asset", "m_transparency"]);
                if color_packed.is_some() {
                    fields.remove("m_color");
                }
                let mut skip = vec!["m_pAppearanceAsset"];
                if swatch.is_some() {
                    skip.push("m_pThumbnail");
                }
                let element_fields = reader.fields(0, &root.fields, &skip);
                self.assets.push(AppearanceAsset {
                    element_id: id,
                    name: a["m_Name"].as_str().map(str::to_string),
                    color_packed,
                    transparency: a["m_transparency"].as_f64(),
                    asset,
                    swatch,
                    fields,
                    element_fields,
                });
            }
            _ => return self.unread(id, class, "not a material or appearance asset"),
        }
        if reader.exhausted.get() {
            self.unread(id, class, "read in part: node budget exceeded");
        }
    }
}

/// Read every material and appearance asset of `file`.
pub fn read(file: &mut RevitFile) -> anyhow::Result<Appearance> {
    let options = native_document::Options {
        selected_classes: ["AppearanceAssetElem", "MaterialElem"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        channels: BTreeSet::from([102]),
        releases: native_document::FRAMED_RELEASES,
        max_graph_values: GRAPH_VALUES,
        ..Default::default()
    };
    let mut out = Appearance::default();
    native_document::extract(file, &options, |r| {
        let class = r.class_name.as_deref().unwrap_or("");
        match &r.graph {
            Some(g) => out.ingest(r.identity.element_id, class, g),
            None => out.unread(
                r.identity.element_id,
                class,
                r.diagnostic.clone().unwrap_or_else(|| r.status.clone()),
            ),
        }
        Ok(())
    })?;
    Ok(out)
}

fn swatch(t: &Fv) -> Option<Swatch> {
    let bytes = t.get("m_compressedImage")?.as_array()?;
    let image = bytes
        .iter()
        .map(|b| b.as_u64().and_then(|b| u8::try_from(b).ok()))
        .collect::<Option<Vec<u8>>>()?;
    let mut fields = Map::new();
    if let Fv::Object(o) = t {
        for (k, v) in o.iter() {
            if !["m_compressedImage", "m_width", "m_height"].contains(&k) {
                fields.insert(k.to_string(), v.to_json());
            }
        }
    }
    Some(Swatch {
        width: t["m_width"].as_u64(),
        height: t["m_height"].as_u64(),
        image,
        fields,
    })
}

/// Turns a graph's objects into records. Every object it follows is on
/// `path` while it is read, so a pointer back to one gives a `cycle` marker;
/// every object and value costs one of [`NODE_BUDGET`], so shared pointers
/// cannot make the work grow past it.
struct Reader<'a> {
    g: &'a ObjectGraph,
    edges: BTreeMap<(usize, usize), usize>,
    path: RefCell<Vec<usize>>,
    nodes: Cell<usize>,
    exhausted: Cell<bool>,
}

impl<'a> Reader<'a> {
    fn new(g: &'a ObjectGraph) -> Self {
        let edges = g
            .edges
            .iter()
            .map(|e| {
                (
                    (e.source_object_index, e.pointer_offset),
                    e.target_object_index,
                )
            })
            .collect();
        Self {
            g,
            edges,
            path: RefCell::new(Vec::new()),
            nodes: Cell::new(0),
            exhausted: Cell::new(false),
        }
    }

    /// Put a root object on the path (never left).
    fn enter(&self, i: usize) {
        self.path.borrow_mut().push(i);
    }

    /// Count one node; false once the budget is spent.
    fn spend(&self) -> bool {
        let n = self.nodes.get() + 1;
        self.nodes.set(n);
        if n > NODE_BUDGET {
            self.exhausted.set(true);
        }
        n <= NODE_BUDGET
    }

    /// Read object `j` with `f`, or give a marker where it is already on the
    /// path or the budget is spent.
    fn visit<T>(&self, j: usize, f: impl FnOnce() -> T, marker: impl FnOnce(Value) -> T) -> T {
        if self.path.borrow().contains(&j) {
            return marker(json!({"cycle": true, "object_index": j}));
        }
        if !self.spend() {
            return marker(json!({"budget_exceeded": true, "object_index": j}));
        }
        self.path.borrow_mut().push(j);
        let out = f();
        self.path.borrow_mut().pop();
        out
    }

    /// The object a pointer field of object `owner` points at.
    fn target(&self, owner: usize, p: &Fv) -> Option<(usize, &'a Fv)> {
        let offset = usize::try_from(p.get("offset")?.as_u64()?).ok()?;
        let i = *self.edges.get(&(owner, offset))?;
        Some((i, &self.g.objects.get(i)?.fields))
    }

    fn is_pointer(v: &Fv) -> bool {
        v.get("pointer_token").is_some() && v.get("offset").is_some()
    }

    /// The fields of object `owner` other than `skip`, resolved.
    fn fields(&self, owner: usize, f: &Fv, skip: &[&str]) -> Map<String, Value> {
        let mut out = Map::new();
        if let Fv::Object(o) = f {
            for (k, v) in o.iter() {
                if !skip.contains(&k) {
                    out.insert(k.to_string(), self.value(owner, v));
                }
            }
        }
        out
    }

    /// An asset whose fields `f` belong to object `owner`.
    fn asset(&self, owner: usize, class: &str, f: &Fv) -> Asset {
        let mut properties = Vec::new();
        let mut fields = Map::new();
        if let Fv::Object(o) = f {
            for (k, v) in o.iter() {
                match (k, v.as_array()) {
                    ("m_aAProperties", Some(items)) => {
                        for p in items {
                            match self.target(owner, p) {
                                Some((j, _)) => match self.property(j) {
                                    Ok(p) => properties.push(p),
                                    Err(marker) => push(&mut fields, "unread_properties", marker),
                                },
                                None => push(&mut fields, "unresolved_properties", p.to_json()),
                            }
                        }
                    }
                    _ => {
                        fields.insert(k.to_string(), self.value(owner, v));
                    }
                }
            }
        }
        let schema = properties
            .iter()
            .find(|p| p.name == "BaseSchema")
            .and_then(|p| p.value.as_str())
            .map(str::to_string);
        Asset {
            class: class.to_string(),
            schema,
            properties,
            fields,
        }
    }

    /// The property object at index `i`, or a marker for a cycle or a spent
    /// budget.
    fn property(&self, i: usize) -> Result<AssetProperty, Value> {
        self.visit(i, || Ok(self.property_body(i)), Err)
    }

    fn property_body(&self, i: usize) -> AssetProperty {
        let o = &self.g.objects[i];
        let kind = o
            .class_name
            .strip_prefix("AProperty")
            .filter(|k| !k.is_empty())
            .unwrap_or(&o.class_name)
            .to_string();
        let mut p = AssetProperty {
            name: o.fields["m_sName"].as_str().unwrap_or("").to_string(),
            kind,
            value: Value::Null,
            children: Vec::new(),
            connected: Vec::new(),
            fields: Map::new(),
        };
        if let Fv::Object(f) = &o.fields {
            for (k, v) in f.iter() {
                match (k, v.as_array()) {
                    ("m_sName", _) if v.is_string() => {}
                    ("m_value", _) => p.value = self.value(i, v),
                    ("m_aChild" | "m_aConnected", Some(items)) => {
                        for q in items {
                            let Some((j, t)) = self.target(i, q) else {
                                push(&mut p.fields, &format!("unresolved_{k}"), q.to_json());
                                continue;
                            };
                            if k == "m_aChild" {
                                match self.property(j) {
                                    Ok(c) => p.children.push(c),
                                    Err(marker) => push(&mut p.fields, "unread_m_aChild", marker),
                                }
                            } else {
                                let class = &self.g.objects[j].class_name;
                                match self.visit(j, || Ok(self.asset(j, class, t)), Err) {
                                    Ok(a) => p.connected.push(a),
                                    Err(marker) => {
                                        push(&mut p.fields, "unread_m_aConnected", marker)
                                    }
                                }
                            }
                        }
                    }
                    _ => {
                        p.fields.insert(k.to_string(), self.value(i, v));
                    }
                }
            }
        }
        p
    }

    /// A field value of object `owner` as JSON, a pointer replaced by the
    /// object it points at (`{"class", "fields"}`, a property as a property),
    /// or kept as saved where it points at none.
    fn value(&self, owner: usize, v: &Fv) -> Value {
        if !self.spend() {
            return json!({"budget_exceeded": true});
        }
        match v {
            Fv::Array(items) => Value::Array(items.iter().map(|x| self.value(owner, x)).collect()),
            Fv::Object(_) if Self::is_pointer(v) => match self.target(owner, v) {
                Some((j, t)) => self.visit(
                    j,
                    || {
                        let class = &self.g.objects[j].class_name;
                        if class.starts_with("AProperty") {
                            serde_json::to_value(self.property_body(j)).unwrap_or(Value::Null)
                        } else {
                            json!({"class": class, "fields": self.fields(j, t, &[])})
                        }
                    },
                    |marker| marker,
                ),
                None => v.to_json(),
            },
            Fv::Object(o) => Value::Object(
                o.iter()
                    .map(|(k, x)| (k.to_string(), self.value(owner, x)))
                    .collect(),
            ),
            _ => v.to_json(),
        }
    }
}

/// Append `v` to the array `fields[key]`.
fn push(fields: &mut Map<String, Value>, key: &str, v: Value) {
    if let Value::Array(items) = fields.entry(key).or_insert_with(|| json!([])) {
        items.push(v);
    }
}

fn base64<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        for k in 0..4 {
            if k <= c.len() {
                out.push(T[((n >> (18 - 6 * k)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    s.serialize_str(&out)
}
