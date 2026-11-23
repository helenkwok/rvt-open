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
//! translated. The images a bitmap names are not in the file (they are paths
//! into a material library); only a swatch is.

use crate::graph_value::Fv;
use crate::native_parameters::ObjectGraph;
use crate::{RevitFile, native_document};
use serde::{Serialize, Serializer};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// The appearance assets of a file and the materials that use them.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Appearance {
    /// Every material, with the appearance asset it names.
    pub materials: Vec<MaterialAppearance>,
    /// Every appearance asset read, by element id.
    pub assets: Vec<AppearanceAsset>,
    /// Appearance assets whose objects could not be read, and why.
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
}

/// An appearance asset element.
#[derive(Debug, Clone, Serialize)]
pub struct AppearanceAsset {
    /// The element id.
    pub element_id: u64,
    /// The asset's name.
    pub name: Option<String>,
    /// Its colour as saved, `0x00BBGGRR`.
    pub color_packed: Option<u32>,
    /// Its transparency as saved, 0 to 1.
    pub transparency: Option<f64>,
    /// The property tree.
    pub asset: Asset,
    /// The swatch image saved with it, where there is one.
    pub swatch: Option<Swatch>,
    /// The element's material path map, as saved.
    pub material_path_map: Value,
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
    pub width: Option<u64>,
    pub height: Option<u64>,
    /// The image file's bytes (PNG), written as base64 in JSON.
    #[serde(serialize_with = "base64")]
    pub image: Vec<u8>,
    /// The image object's other fields (dpi, size in feet), as saved.
    pub fields: Map<String, Value>,
}

/// An appearance asset not read.
#[derive(Debug, Clone, Serialize)]
pub struct UnreadAsset {
    pub element_id: u64,
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
                out.extend(a.connected());
            }
            stack.extend(p.children.iter().rev());
        }
        out
    }
}

impl Appearance {
    /// The appearance asset of the material named `name`, where it has one.
    pub fn asset_of(&self, name: &str) -> Option<&AppearanceAsset> {
        let id = self
            .materials
            .iter()
            .find(|m| m.name == name)?
            .appearance_asset_id?;
        self.assets.iter().find(|a| a.element_id as i64 == id)
    }
}

/// Read every material and appearance asset of `file`.
pub fn read(file: &mut RevitFile) -> anyhow::Result<Appearance> {
    let mut options = native_document::Options {
        selected_classes: ["AppearanceAssetElem", "MaterialElem"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        releases: native_document::FRAMED_RELEASES,
        // a swatch image is one value per byte; the record's size bounds it.
        max_graph_values: usize::MAX,
        ..Default::default()
    };
    options.channels = BTreeSet::from([102]);
    let mut out = Appearance::default();
    native_document::extract(file, &options, |r| {
        let id = r.identity.element_id;
        let class = r.class_name.as_deref().unwrap_or("");
        let Some(g) = &r.graph else {
            if class == "AppearanceAssetElem" {
                out.unread.push(UnreadAsset {
                    element_id: id,
                    reason: r.diagnostic.clone().unwrap_or_else(|| r.status.clone()),
                });
            }
            return Ok(());
        };
        let reader = Reader::new(g);
        let Some(root) = g.objects.first() else {
            return Ok(());
        };
        match root.class_name.as_str() {
            "MaterialElem" => {
                let Some((i, m)) = reader.target(0, &root.fields["m_pMaterial"]) else {
                    return Ok(());
                };
                out.materials.push(MaterialAppearance {
                    material_id: id,
                    name: m["m_name"].as_str().unwrap_or("").to_string(),
                    appearance_asset_id: crate::native_json::identifier_i64(
                        &m["m_appearanceAssetId"],
                    ),
                    use_render_appearance: m["m_bUseRenderAppearance"].as_bool(),
                    own_asset: m
                        .get("m_asset")
                        .filter(|a| a.get("m_aAProperties").is_some())
                        .map(|a| reader.asset(i, "Asset", a, 0)),
                });
            }
            "AppearanceAssetElem" => {
                let Some((i, a)) = reader.target(0, &root.fields["m_pAppearanceAsset"]) else {
                    out.unread.push(UnreadAsset {
                        element_id: id,
                        reason: "no appearance asset object".into(),
                    });
                    return Ok(());
                };
                let swatch = root
                    .fields
                    .get("m_pThumbnail")
                    .and_then(|p| reader.target(0, p))
                    .and_then(|(_, t)| swatch(t));
                out.assets.push(AppearanceAsset {
                    element_id: id,
                    name: a["m_Name"].as_str().map(str::to_string),
                    color_packed: a["m_color"].as_u64().and_then(|c| u32::try_from(c).ok()),
                    transparency: a["m_transparency"].as_f64(),
                    asset: reader.asset(i, "Asset", &a["m_Asset"], 0),
                    swatch,
                    material_path_map: root.fields["m_materialPathMap"].to_json(),
                });
            }
            _ => {}
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

/// Deep enough for any asset seen (a property, its connected asset, that
/// asset's properties: four levels); a cycle stops here.
const MAX_DEPTH: usize = 64;

struct Reader<'a> {
    g: &'a ObjectGraph,
    edges: BTreeMap<(usize, usize), usize>,
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
        Self { g, edges }
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

    /// An asset whose fields `f` belong to object `owner`.
    fn asset(&self, owner: usize, class: &str, f: &Fv, depth: usize) -> Asset {
        let mut properties = Vec::new();
        let mut fields = Map::new();
        if let Fv::Object(o) = f {
            for (k, v) in o.iter() {
                if k == "m_aAProperties" {
                    for p in v.as_array().into_iter().flatten() {
                        match self.target(owner, p) {
                            Some((j, _)) => properties.push(self.property(j, depth + 1)),
                            None => fields
                                .entry("unresolved_properties")
                                .or_insert_with(|| json!([]))
                                .as_array_mut()
                                .expect("an array")
                                .push(p.to_json()),
                        }
                    }
                } else {
                    fields.insert(k.to_string(), self.value(owner, v, depth + 1));
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

    /// The property object at index `i`.
    fn property(&self, i: usize, depth: usize) -> AssetProperty {
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
        if depth > MAX_DEPTH {
            p.fields.insert("depth_limit".into(), json!(true));
            return p;
        }
        if let Fv::Object(f) = &o.fields {
            for (k, v) in f.iter() {
                match k {
                    "m_sName" => {}
                    "m_value" => p.value = self.value(i, v, depth + 1),
                    "m_aChild" | "m_aConnected" => {
                        for q in v.as_array().into_iter().flatten() {
                            match self.target(i, q) {
                                Some((j, _)) if k == "m_aChild" => {
                                    p.children.push(self.property(j, depth + 1));
                                }
                                Some((j, t)) => p.connected.push(self.asset(
                                    j,
                                    &self.g.objects[j].class_name,
                                    t,
                                    depth + 1,
                                )),
                                None => p
                                    .fields
                                    .entry(format!("unresolved_{k}"))
                                    .or_insert_with(|| json!([]))
                                    .as_array_mut()
                                    .expect("an array")
                                    .push(q.to_json()),
                            }
                        }
                    }
                    _ => {
                        p.fields.insert(k.to_string(), self.value(i, v, depth + 1));
                    }
                }
            }
        }
        p
    }

    /// A field value of object `owner` as JSON, a pointer replaced by the
    /// object it points at (`{"class", "fields"}`), or kept as saved where
    /// it points at none.
    fn value(&self, owner: usize, v: &Fv, depth: usize) -> Value {
        if depth > MAX_DEPTH {
            return json!({"depth_limit": true});
        }
        match v {
            Fv::Array(items) => Value::Array(
                items
                    .iter()
                    .map(|x| self.value(owner, x, depth + 1))
                    .collect(),
            ),
            Fv::Object(_) if Self::is_pointer(v) => match self.target(owner, v) {
                Some((j, t)) => {
                    let class = &self.g.objects[j].class_name;
                    if class.starts_with("AProperty") {
                        serde_json::to_value(self.property(j, depth + 1)).unwrap_or(Value::Null)
                    } else {
                        json!({"class": class, "fields": self.value(j, t, depth + 1)})
                    }
                }
                None => v.to_json(),
            },
            Fv::Object(o) => Value::Object(
                o.iter()
                    .map(|(k, x)| (k.to_string(), self.value(owner, x, depth + 1)))
                    .collect(),
            ),
            _ => v.to_json(),
        }
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
