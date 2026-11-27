//! Linked and imported models and files: every Revit link, IFC link, CAD link or import, image,
//! PDF, point cloud and other external file the model refers to or embeds, with what the
//! authoring tool's Manage Links dialog shows for each and where each is placed.
//!
//! Two places in the file hold them:
//!
//! - The `TransmissionData` stream: a 32-bit count of UTF-16 code units, then that many units of
//!   XML (`<TransmissionData version="5">`, the same in every release from 2014 to 2027 read so
//!   far). One `ExternalFileReference` per referenced file: the element that owns it
//!   (`ElementId`), its kind (`ExternalFileReferenceType`: Revit Link, CAD Link, Keynote Table,
//!   Assembly Code Table, Decal, ...), the path as saved (`LastSavedPath`, and resolved,
//!   `LastSavedAbsolutePath`), its path type and load state, and the path, path type and load
//!   state wanted on the next open (`Desired*`). Imported files are not in it.
//! - The model's own objects, read through the schema:
//!   - `RvtLinkSymbol` (a link type): the name (`m_symbolInfo.m_name`), the saved path
//!     (`m_otherDocModelLocation.m_lastKnownModelPath.m_path`), relative or absolute
//!     (`m_relative`), Attachment or Overlay (`m_isAttachment`), room bounding, the phase map
//!     (`m_phaseMap`: host phase, linked phase), a parent type when nested (`m_parentSymbolId`),
//!     and for an IFC link the IFC file it was converted from (`m_linkOriginFileType` 1,
//!     `m_oRvtLinkConversionData.m_path`).
//!   - `RvtLinkInstance`, `ImportInstance` (any `LinkInstance`): the type
//!     (`m_pInstanceInfo.m_symbolId`) and placement (`m_pInstanceInfo.m_Trf`: a 3x3 rotation, row
//!     per axis, and origin, feet), the owner view of a view-specific import
//!     (`m_ownerDBViewId`), its draw layer (`m_background`), a link instance's name (`m_name`)
//!     and parent instance when nested (`m_parentInstanceId`).
//!   - `MasterImportSymbol` (a CAD import or link): the file name, the file size (`m_size`), the
//!     drawing units (`m_unitTypeId`), the import settings (`m_importSettings`: link or import,
//!     current view only, colour mode), the import's category (`m_categoryId`) and its layers
//!     (`m_layerCategoryIdSet`: `CategoryElem`s named after the layers, each with a `GStyleElem`
//!     giving colour, pen and line pattern). `SlaveImportSymbol`s are its blocks.
//!   - `ImageSymbol` (an image or PDF page) with its `ImageHolder`, whose `m_pRasterImage` keeps
//!     an embedded image's encoded bytes (`m_compressedImage`, PNG or the source format), and
//!     `ImageInstance`s placing it in a view (`m_pImageInfo`: base and up vectors, placement,
//!     width and height or their scales).
//!   - `PointCloudType`/`PointCloudInstance`, `TopographyLinkType`/`SiteLink` and other
//!     `LinkType`s: name, saved path (an `ExternalResourceReferenceCell`) and placement.
//!
//! Objects that belong to a loaded family's own document (a title block's logo image, a
//! family's imported detail) are left out: they are the family's, not the model's.

use crate::schema_objects::{Fields, SchemaObject, Value, for_each_object_where};
use crate::streams::TRANSMISSION_DATA;
use crate::{Result, RevitFile};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// One `ExternalFileReference` of the `TransmissionData` stream.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ExternalFileReference {
    pub element_id: i64,
    /// As written: `Revit Link`, `CAD Link`, `Keynote Table`, `Assembly Code Table`, `Decal`, ...
    pub kind: String,
    pub saved_path: String,
    pub saved_absolute_path: String,
    /// `Relative to Central Model`, `Absolute`, `Relative to Library Locations`, ...
    pub saved_path_type: String,
    /// `Loaded`, `Unloaded`, `Not Found`, ...
    pub saved_load_state: String,
    pub desired_path: String,
    pub desired_path_type: String,
    pub desired_load_state: String,
}

/// The kinds of linked or imported things, as the Manage Links dialog groups them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    Revit,
    Ifc,
    Cad,
    PointCloud,
    Topography,
    Image,
    Pdf,
    /// A keynote table, assembly code table, decal image or other file the model names.
    Other,
}

/// A link or import type: one row of the Manage Links dialog.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LinkType {
    pub id: i64,
    /// The schema class (`RvtLinkSymbol`, `MasterImportSymbol`, `ImageSymbol`, ...).
    pub class: String,
    pub kind: LinkKind,
    /// The file name the dialog shows.
    pub name: String,
    /// Linked (a reference to a file) rather than imported (embedded in the model).
    pub linked: bool,
    /// The saved path, as written.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub saved_path: String,
    /// `Relative`, `Absolute`, or the stored path type text.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub path_type: String,
    /// The saved status: `Loaded`, `Not Loaded`, `Not Found` as the file was last saved (links
    /// only). The authoring tool shows the status it finds when it opens the file, which can differ.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub status: String,
    /// Revit and IFC links: Attachment (`true`) or Overlay.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachment: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room_bounding: Option<bool>,
    /// Host phase id to linked phase id (Revit links).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub phase_map: Vec<(i64, i64)>,
    /// The link type this one is nested in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<i64>,
    /// IFC link: the IFC file the link was made from.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub source_path: String,
    /// CAD: the file's size in bytes, as saved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_size: Option<i64>,
    /// CAD: the drawing's length unit (`autodesk.unit.unit:millimeters-1.0.1`, ...).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub units: String,
    /// CAD: imported or linked into one view only (Current View Only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view_specific: Option<bool>,
    /// CAD: the import's own category, whose subcategories are its layers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<i64>,
    /// CAD: the layers, with their colour (0xBBGGRR as stored) and pen.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<Layer>,
    /// Images and PDFs: size in pixels and the PDF page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pixels: Option<[i64; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<i64>,
    /// Images and PDFs: the encoded image is held in the model (its holder element).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_holder: Option<i64>,
}

/// A CAD layer: a subcategory of the import's category.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Layer {
    pub id: i64,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pen: Option<i64>,
    /// The line styles (`GStyleElem`s) of the layer: the import's graphics name these.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub styles: Vec<i64>,
}

/// One placed link, import, image or point cloud.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LinkInstance {
    pub id: i64,
    pub class: String,
    pub type_id: i64,
    /// Revit link instances: the instance name.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// x axis, y axis, z axis, origin; feet in the host's internal coordinates.
    pub transform: [[f64; 3]; 4],
    /// The view a view-specific import, image or PDF is drawn in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view: Option<i64>,
    /// Draw layer of a view-specific import or image: behind model elements.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<bool>,
    /// Images: width and height on the view, feet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<[f64; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<i64>,
    pub pinned: bool,
}

/// Everything [`read`] finds.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Links {
    pub references: Vec<ExternalFileReference>,
    pub types: Vec<LinkType>,
    pub instances: Vec<LinkInstance>,
}

/// One row of the Manage Links dialog (see [`Links::manage_links`]).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ManageLinksRow {
    pub id: i64,
    pub kind: LinkKind,
    pub link_name: String,
    /// The saved status: `Loaded`, `Not Loaded`, `Not Found` as the file was last saved, `Unknown`
    /// when it records none, `Imported` for an import. The authoring tool's dialog shows the status
    /// it finds when it opens the file, which can differ.
    pub status: String,
    /// `Attachment`, `Overlay` (Revit, IFC); `Linked`, `Imported` (CAD, images, PDFs).
    pub reference_type: String,
    pub saved_path: String,
    pub path_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_size: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolution: Option<[i64; 2]>,
    /// Placements of this type.
    pub count: usize,
}

/// Parse the `TransmissionData` stream: a u32 count of UTF-16 code units, then the XML.
pub fn parse_transmission_data(bytes: &[u8]) -> Vec<ExternalFileReference> {
    if bytes.len() < 4 {
        return Vec::new();
    }
    let n = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    let body = &bytes[4..];
    let units: Vec<u16> = body
        .chunks_exact(2)
        .take(n.min(body.len() / 2))
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let text = String::from_utf16_lossy(&units);
    let mut out = Vec::new();
    let mut rest = text.as_str();
    while let Some(start) = rest.find("<ExternalFileReference>") {
        let after = &rest[start + "<ExternalFileReference>".len()..];
        let Some(end) = after.find("</ExternalFileReference>") else {
            break;
        };
        let item = &after[..end];
        let tag = |name: &str| -> String {
            let open = format!("<{name}>");
            let close = format!("</{name}>");
            item.find(&open)
                .and_then(|a| {
                    let v = &item[a + open.len()..];
                    v.find(&close).map(|b| unescape(&v[..b]))
                })
                .unwrap_or_default()
        };
        out.push(ExternalFileReference {
            element_id: tag("ElementId").trim().parse().unwrap_or(-1),
            kind: tag("ExternalFileReferenceType"),
            saved_path: tag("LastSavedPath"),
            saved_absolute_path: tag("LastSavedAbsolutePath"),
            saved_path_type: tag("LastSavedPathType"),
            saved_load_state: tag("LastSavedLoadState"),
            desired_path: tag("DesiredPath"),
            desired_path_type: tag("DesiredPathType"),
            desired_load_state: tag("DesiredLoadState"),
        });
        rest = &after[end..];
    }
    out
}

/// XML text: the five named entities and numeric character references (`&#38;`, `&#x26;`).
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        // A reference is at most 12 bytes after its `&`: look no further for its `;`.
        let Some(end) = tail.bytes().take(13).position(|b| b == b';') else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let name = &tail[1..end];
        let c = match name {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "amp" => Some('&'),
            _ => name
                .strip_prefix("#x")
                .or_else(|| name.strip_prefix("#X"))
                .map(|h| u32::from_str_radix(h, 16))
                .or_else(|| name.strip_prefix('#').map(|d| d.parse::<u32>()))
                .and_then(|n| n.ok())
                .and_then(char::from_u32),
        };
        match c {
            Some(c) => {
                out.push(c);
                rest = &tail[end + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn id(f: &Fields, name: &str) -> Option<i64> {
    match f.get(name)? {
        Value::Id(i) if *i >= 0 => Some(*i),
        _ => None,
    }
}

fn int(v: &Value) -> Option<i64> {
    match v {
        Value::Int(i) | Value::Id(i) => Some(*i),
        Value::Object(o) => o.values.iter().find_map(|(_, v)| int(v)),
        _ => None,
    }
}

fn text(f: &Fields, name: &str) -> Option<String> {
    match f.get(name)? {
        Value::Text(s) => Some(s.clone()),
        _ => None,
    }
}

fn flag(f: &Fields, name: &str) -> Option<bool> {
    match f.get(name)? {
        Value::Bool(b) => Some(*b),
        Value::Int(i) => Some(*i != 0),
        _ => None,
    }
}

fn float(f: &Fields, name: &str) -> Option<f64> {
    match f.get(name)? {
        Value::Float(v) if v.is_finite() => Some(*v),
        _ => None,
    }
}

fn floats(v: &Value) -> Vec<f64> {
    match v {
        Value::Float(x) => vec![*x],
        Value::Int(i) => vec![*i as f64],
        Value::List(items) => items.iter().flat_map(floats).collect(),
        Value::Object(o) => o.values.iter().flat_map(|(_, v)| floats(v)).collect(),
        _ => Vec::new(),
    }
}

fn obj<'a>(o: &'a SchemaObject, f: &'a Fields, name: &str) -> Option<&'a Fields> {
    match f.get(name)? {
        Value::New { index, .. } => o.objects.get(*index),
        Value::Object(f) => Some(f),
        _ => None,
    }
}

/// A symbol's name: `m_symbolInfo.m_name`.
fn symbol_name(o: &SchemaObject, root: &Fields) -> String {
    obj(o, root, "m_symbolInfo")
        .and_then(|s| text(s, "m_name"))
        .unwrap_or_default()
}

/// `m_Trf` of an instance info: the 3x3 is stored row by row, so its columns are the x, y and z
/// axes (a link turned 60.6 degrees holds x = (0.490, 0.871, 0) down its first column); `m_or`
/// is the origin.
pub(crate) fn transform(info: &Fields) -> Option<[[f64; 3]; 4]> {
    let t = floats(info.get("m_Trf")?);
    (t.len() == 12 && t.iter().all(|v| v.is_finite())).then(|| {
        [
            [t[0], t[3], t[6]],
            [t[1], t[4], t[7]],
            [t[2], t[5], t[8]],
            [t[9], t[10], t[11]],
        ]
    })
}

/// The first saved path and path type an `ExternalResourceReferenceCell` holds: its
/// `m_referenceInformation` entries `Path` and `PathType`, else `m_displayPath`.
fn resource_path(o: &SchemaObject) -> Option<(String, String)> {
    fn pairs(v: &Value, out: &mut BTreeMap<String, String>, display: &mut Option<String>) {
        match v {
            Value::Object(f) => {
                if let (Some(Value::Text(k)), Some(Value::Text(val))) =
                    (f.get("first"), f.get("second"))
                {
                    out.entry(k.clone()).or_insert_with(|| val.clone());
                }
                if let Some(Value::Text(d)) = f.get("m_displayPath") {
                    display.get_or_insert_with(|| d.clone());
                }
                f.values.iter().for_each(|(_, v)| pairs(v, out, display));
            }
            Value::List(items) => items.iter().for_each(|v| pairs(v, out, display)),
            _ => {}
        }
    }
    let f = o
        .objects
        .iter()
        .find(|f| f.get("m_externalResourceReferences").is_some())?;
    let (mut map, mut display) = (BTreeMap::new(), None);
    pairs(
        f.get("m_externalResourceReferences")?,
        &mut map,
        &mut display,
    );
    let path = map.remove("Path").or(display)?;
    Some((path, map.remove("PathType").unwrap_or_default()))
}

/// A `CategoryElem`'s name and parent, and a `GStyleElem`'s colour and pen, by element id.
#[derive(Default)]
struct Styles {
    categories: BTreeMap<i64, (String, Option<i64>, Vec<i64>)>,
    gstyles: BTreeMap<i64, (Option<i64>, Option<i64>)>,
}

const CLASSES: &[&str] = &[
    "RvtLinkSymbol",
    "MasterImportSymbol",
    "SlaveImportSymbol",
    "ImageSymbol",
    "PointCloudType",
    "TopographyLinkType",
    "LinkType",
    "LinkInstance",
    "ImageInstance",
    "CategoryElem",
    "GStyleElem",
];

/// Read every link, import, image and external file reference of the model.
pub fn read(rf: &mut RevitFile) -> Result<Links> {
    let mut out = Links {
        references: match rf.read_stream(TRANSMISSION_DATA) {
            Ok(bytes) => parse_transmission_data(&bytes),
            Err(crate::Error::StreamNotFound(_)) => Vec::new(),
            Err(e) => return Err(e),
        },
        ..Default::default()
    };
    let layouts = rf.schema_layouts()?;
    // Each class's nearest wanted base, so an RvtLinkSymbol is one before it is a LinkType.
    let mut wanted: BTreeMap<u16, (&'static str, String)> = BTreeMap::new();
    for tag in layouts.classes.keys() {
        let Some(chain) = layouts.chain(*tag) else {
            continue;
        };
        if let Some(name) = chain
            .iter()
            .rev()
            .find_map(|c| CLASSES.iter().find(|n| **n == c.name))
        {
            let class = chain.last().map(|c| c.name.clone()).unwrap_or_default();
            wanted.insert(*tag, (*name, class));
        }
    }
    let mut sections: BTreeMap<String, usize> = BTreeMap::new();
    for name in rf.partition_stream_names() {
        if let Ok(inflated) = rf.inflated_partition(&name) {
            if let Some((at, _)) = inflated.sections().first() {
                sections.insert(name, *at);
            }
        }
    }
    let in_family = |o: &SchemaObject| {
        sections
            .get(&o.stream)
            .is_some_and(|first| *first <= o.frame.span.start)
    };
    // Objects of deleted elements can stay in a partition; the element table lists the live ones.
    let live: BTreeSet<u32> = crate::elem_table::declared_element_ids(rf)
        .map(|ids| ids.into_iter().collect())
        .unwrap_or_default();
    let mut styles = Styles::default();
    let mut seen = BTreeSet::new();
    let mut layer_ids: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
    let mut image_size: BTreeMap<i64, [f64; 2]> = BTreeMap::new();
    for_each_object_where(
        rf,
        |f, _| wanted.contains_key(&f.header.tag),
        |o| {
            let (Some(element), Some(root)) = (o.element(), o.root()) else {
                return;
            };
            // The element table lists 32-bit ids only: an id it cannot hold is not one of its.
            if in_family(o)
                || (!live.is_empty() && u32::try_from(element).map_or(true, |e| !live.contains(&e)))
            {
                return;
            }
            let (base, class) = wanted[&o.frame.header.tag].clone();
            // A CAD import's blocks are not types of their own.
            if base == "SlaveImportSymbol" {
                return;
            }
            match base {
                "CategoryElem" => {
                    if let Some(c) = obj(o, root, "m_pCategory") {
                        let gstyles = match root.get("m_gstyleIds") {
                            Some(Value::List(l)) => l.iter().filter_map(int).collect(),
                            _ => Vec::new(),
                        };
                        styles.categories.insert(
                            element,
                            (
                                text(c, "m_name").unwrap_or_default(),
                                id(c, "m_parentCategoryId"),
                                gstyles,
                            ),
                        );
                    }
                    return;
                }
                "GStyleElem" => {
                    if let Some(g) = obj(o, root, "m_pGStyle") {
                        styles.gstyles.insert(
                            element,
                            (
                                g.get("m_color").and_then(int),
                                g.get("m_penNumber").and_then(int),
                            ),
                        );
                    }
                    return;
                }
                _ => {}
            }
            // What a loaded family holds (a title block's logo image, a family's imported
            // drawing) names that family in `m_famId`: it is the family's, not the model's.
            // An element is taken from its first record that reads whole (`seen` is set when it is).
            if seen.contains(&element) || id(root, "m_famId").is_some() {
                return;
            }
            let pinned = flag(root, "m_locked").unwrap_or(false);
            match base {
                "LinkInstance" | "ImageInstance" => {
                    let info =
                        obj(o, root, "m_pInstanceInfo").or_else(|| obj(o, root, "m_pImageInfo"));
                    let Some(info) = info else { return };
                    let (Some(type_id), Some(transform)) =
                        (id(info, "m_symbolId"), transform(info))
                    else {
                        return;
                    };
                    let size = match (float(info, "m_dWidth"), float(info, "m_dHeight")) {
                        (Some(w), Some(h)) => Some([w, h]),
                        _ => match (float(info, "m_dWidthScale"), float(info, "m_dHeightScale")) {
                            (Some(w), Some(h)) => Some([-w, -h]),
                            _ => None,
                        },
                    };
                    seen.insert(element);
                    out.instances.push(LinkInstance {
                        id: element,
                        class,
                        type_id,
                        name: text(root, "m_name").unwrap_or_default(),
                        transform,
                        view: id(root, "m_ownerDBViewId"),
                        background: flag(root, "m_background"),
                        size,
                        parent: id(root, "m_parentInstanceId"),
                        pinned,
                    });
                }
                _ => {
                    let name = symbol_name(o, root);
                    let mut t = LinkType {
                        id: element,
                        class: class.clone(),
                        kind: LinkKind::Other,
                        name,
                        linked: true,
                        saved_path: String::new(),
                        path_type: String::new(),
                        status: String::new(),
                        attachment: None,
                        room_bounding: None,
                        phase_map: Vec::new(),
                        parent: None,
                        source_path: String::new(),
                        file_size: None,
                        units: String::new(),
                        view_specific: None,
                        category: None,
                        layers: Vec::new(),
                        pixels: None,
                        page: None,
                        image_holder: None,
                    };
                    if let Some((path, kind)) = resource_path(o) {
                        t.saved_path = path;
                        t.path_type = kind;
                    }
                    match base {
                        "RvtLinkSymbol" => {
                            let ifc = root.get("m_linkOriginFileType").and_then(int) == Some(1)
                                || t.name.to_ascii_lowercase().ends_with(".ifc");
                            t.kind = if ifc { LinkKind::Ifc } else { LinkKind::Revit };
                            if let Some(p) = obj(o, root, "m_otherDocModelLocation")
                                .and_then(|l| obj(o, l, "m_lastKnownModelPath"))
                                .and_then(|p| text(p, "m_path"))
                                .filter(|p| !p.is_empty())
                            {
                                t.saved_path = p;
                            }
                            if let Some(rel) = flag(root, "m_relative") {
                                t.path_type = if rel { "Relative" } else { "Absolute" }.to_string();
                            }
                            t.attachment = flag(root, "m_isAttachment");
                            t.room_bounding = flag(root, "m_isRoomBounding");
                            t.parent = id(root, "m_parentSymbolId");
                            if let Some(Value::List(items)) = root.get("m_phaseMap") {
                                for item in items {
                                    if let Value::Object(p) = item {
                                        if let (Some(a), Some(b)) = (
                                            p.get("first").and_then(int),
                                            p.get("second").and_then(int),
                                        ) {
                                            t.phase_map.push((a, b));
                                        }
                                    }
                                }
                            }
                            if let Some(c) = obj(o, root, "m_oRvtLinkConversionData") {
                                t.source_path = text(c, "m_path").unwrap_or_default();
                            }
                        }
                        "MasterImportSymbol" => {
                            t.kind = LinkKind::Cad;
                            let settings = obj(o, root, "m_importSettings");
                            t.linked = settings.and_then(|s| flag(s, "m_link")).unwrap_or(false);
                            t.view_specific = settings.and_then(|s| flag(s, "m_viewSpecific"));
                            t.file_size = root.get("m_size").and_then(int);
                            t.units = root
                                .get("m_unitTypeId")
                                .and_then(|v| match v {
                                    Value::Object(f) => text(f, "m_typeId"),
                                    _ => None,
                                })
                                .or_else(|| {
                                    // Before the unit ids: the API's DisplayUnitType numbers.
                                    let unit = match root.get("m_displayUnitType").and_then(int)? {
                                        0 => "meters",
                                        1 => "centimeters",
                                        2 => "millimeters",
                                        3 | 4 => "feet",
                                        5 | 6 => "inches",
                                        _ => return None,
                                    };
                                    Some(format!("autodesk.unit.unit:{unit}-1.0.1"))
                                })
                                .unwrap_or_default();
                            t.category = id(root, "m_categoryId");
                            if let Some(Value::List(l)) = root.get("m_layerCategoryIdSet") {
                                layer_ids.insert(element, l.iter().filter_map(int).collect());
                            }
                        }
                        "ImageSymbol" => {
                            let pdf = t.name.to_ascii_lowercase().ends_with(".pdf");
                            t.kind = if pdf { LinkKind::Pdf } else { LinkKind::Image };
                            if t.saved_path.is_empty() {
                                t.saved_path = text(root, "m_strFile").unwrap_or_default();
                            }
                            if let (Some(w), Some(h)) = (
                                root.get("m_widthInPixels").and_then(int),
                                root.get("m_heightInPixels").and_then(int),
                            ) {
                                t.pixels = Some([w, h]);
                            }
                            t.page = root.get("m_pageNumber").and_then(int);
                            t.image_holder = id(root, "m_idImageHolder");
                            if let (Some(w), Some(h)) =
                                (float(root, "m_widthInFeet"), float(root, "m_heightInFeet"))
                            {
                                image_size.insert(element, [w, h]);
                            }
                        }
                        "PointCloudType" => t.kind = LinkKind::PointCloud,
                        "TopographyLinkType" => t.kind = LinkKind::Topography,
                        _ => {}
                    }
                    seen.insert(element);
                    out.types.push(t);
                }
            }
        },
    )?;
    let refs: BTreeMap<i64, &ExternalFileReference> =
        out.references.iter().map(|r| (r.element_id, r)).collect();
    for t in &mut out.types {
        if let Some(r) = refs.get(&t.id) {
            t.status = match r.saved_load_state.as_str() {
                "Unloaded" => "Not Loaded".into(),
                s => s.to_string(),
            };
            if t.saved_path.is_empty() {
                t.saved_path = r.saved_path.clone();
            }
            if t.kind == LinkKind::Cad {
                t.linked = true;
            }
        } else if matches!(t.kind, LinkKind::Image | LinkKind::Pdf) {
            // An image no reference names is held in the model (imported).
            t.linked = false;
        }
        if let Some(ids) = layer_ids.get(&t.id) {
            for l in ids {
                if let Some((name, _, gs)) = styles.categories.get(l) {
                    let g = gs.first().and_then(|g| styles.gstyles.get(g));
                    t.layers.push(Layer {
                        id: *l,
                        name: name.clone(),
                        color: g.and_then(|g| g.0),
                        pen: g.and_then(|g| g.1),
                        styles: gs.clone(),
                    });
                }
            }
        }
    }
    // Later releases keep an image's size as scales of its type's size; make them feet.
    for i in &mut out.instances {
        if let Some([w, h]) = i.size.filter(|s| s[0] < 0.0) {
            i.size = image_size.get(&i.type_id).map(|s| [s[0] * -w, s[1] * -h]);
        }
    }
    Ok(out)
}

impl Links {
    /// The rows of the Manage Links dialog: one per link or import type the model holds (nested
    /// link types are not rows, as in the dialog), plus a row for each link reference whose type
    /// element was not read. Keynote tables, assembly code tables and decals are not rows of the
    /// dialog: they are in [`Links::references`]. A link whose saved load state the file does not
    /// record has status `Unknown`.
    pub fn manage_links(&self) -> Vec<ManageLinksRow> {
        let refs: BTreeMap<i64, &ExternalFileReference> =
            self.references.iter().map(|r| (r.element_id, r)).collect();
        let mut counts: BTreeMap<i64, usize> = BTreeMap::new();
        for i in &self.instances {
            *counts.entry(i.type_id).or_default() += 1;
        }
        let mut rows: Vec<ManageLinksRow> = self
            .types
            .iter()
            .filter(|t| t.parent.is_none())
            .map(|t| {
                let r = refs.get(&t.id);
                ManageLinksRow {
                    id: t.id,
                    kind: t.kind,
                    link_name: t.name.clone(),
                    status: if t.linked {
                        if t.status.is_empty() {
                            "Unknown".into()
                        } else {
                            t.status.clone()
                        }
                    } else {
                        "Imported".into()
                    },
                    reference_type: match (t.kind, t.attachment) {
                        (LinkKind::Revit | LinkKind::Ifc, Some(true)) => "Attachment".into(),
                        (LinkKind::Revit | LinkKind::Ifc, _) => "Overlay".into(),
                        _ if t.linked => "Linked".into(),
                        _ => "Imported".into(),
                    },
                    saved_path: r
                        .map(|r| r.saved_path.clone())
                        .filter(|p| !p.is_empty())
                        .unwrap_or_else(|| t.saved_path.clone()),
                    path_type: r
                        .map(|r| r.saved_path_type.clone())
                        .filter(|p| !p.is_empty())
                        .unwrap_or_else(|| t.path_type.clone()),
                    file_size: t.file_size,
                    resolution: t.pixels,
                    count: counts.get(&t.id).copied().unwrap_or(0),
                }
            })
            .collect();
        let typed: BTreeSet<i64> = self.types.iter().map(|t| t.id).collect();
        // Keynote tables, assembly code tables and decals are not rows of the dialog.
        let link_kinds = [
            "Revit Link",
            "CAD Link",
            "DWF Markup",
            "Point Cloud",
            "Revit Topography Link",
        ];
        for r in self
            .references
            .iter()
            .filter(|r| !typed.contains(&r.element_id) && link_kinds.contains(&r.kind.as_str()))
        {
            rows.push(ManageLinksRow {
                id: r.element_id,
                kind: match r.kind.as_str() {
                    "Revit Link" => LinkKind::Revit,
                    "CAD Link" => LinkKind::Cad,
                    "Point Cloud" => LinkKind::PointCloud,
                    "Revit Topography Link" => LinkKind::Topography,
                    _ => LinkKind::Other,
                },
                link_name: r
                    .saved_path
                    .rsplit(['\\', '/'])
                    .next()
                    .unwrap_or_default()
                    .to_string(),
                status: match r.saved_load_state.as_str() {
                    "Unloaded" => "Not Loaded".into(),
                    "" => "Unknown".into(),
                    s => s.to_string(),
                },
                reference_type: r.kind.clone(),
                saved_path: r.saved_path.clone(),
                path_type: r.saved_path_type.clone(),
                file_size: None,
                resolution: None,
                count: counts.get(&r.element_id).copied().unwrap_or(0),
            });
        }
        rows
    }
}

/// What an imported or linked CAD drawing draws, in its symbol's coordinates (feet): the curves
/// and texts of its graphics, with nested blocks placed and drawn in, each with the layer
/// (`CategoryElem` id) it is drawn on. See [`import_graphics`].
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ImportGraphics {
    /// Polylines (two points for a line; arcs and splines as chords), with their layer.
    pub curves: Vec<(Vec<[f64; 3]>, i64)>,
    /// Texts: insertion point, height, rotation (radians), value, layer.
    pub texts: Vec<ImportText>,
}

/// A text of an imported drawing.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ImportText {
    pub at: [f64; 3],
    pub height: f64,
    pub rotation: f64,
    pub value: String,
    pub layer: i64,
}

type M4 = [[f64; 4]; 4];
const IDENTITY4: M4 = [
    [1., 0., 0., 0.],
    [0., 1., 0., 0.],
    [0., 0., 1., 0.],
    [0., 0., 0., 1.],
];

fn mul(a: &M4, b: &M4) -> M4 {
    let mut m = [[0.0; 4]; 4];
    for (i, row) in m.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = (0..4).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    m
}

fn apply(m: &M4, p: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|i| m[i][0] * p[0] + m[i][1] * p[1] + m[i][2] * p[2] + m[i][3])
}

fn fv_point(v: &crate::graph_value::Fv) -> Option<[f64; 3]> {
    let a = v.as_array()?;
    let x: Vec<f64> = a.iter().filter_map(|c| c.as_f64()).collect();
    match x.len() {
        3 => Some([x[0], x[1], x[2]]),
        2 => Some([x[0], x[1], 0.0]),
        _ => None,
    }
}

fn fv_id(v: &crate::graph_value::Fv) -> Option<i64> {
    v["m_id"]["m_id64"]
        .as_i64()
        .or_else(|| v["m_id"]["m_id"].as_i64())
        .or_else(|| v["m_id64"].as_i64())
        .or_else(|| v["m_id"].as_i64())
}

/// A NURBS curve's points at `n` even steps of its parameter (de Boor, with weights).
fn nurbs_points(degree: usize, knots: &[f64], ctrl: &[([f64; 3], f64)], n: usize) -> Vec<[f64; 3]> {
    if ctrl.len() <= degree || knots.len() != ctrl.len() + degree + 1 {
        return ctrl.iter().map(|c| c.0).collect();
    }
    let (t0, t1) = (knots[degree], knots[ctrl.len()]);
    (0..=n)
        .map(|s| {
            let t = if s == n {
                t1
            } else {
                t0 + (t1 - t0) * s as f64 / n as f64
            };
            // The span holding t; the end of the domain belongs to the last span that is not empty.
            let k = (degree..ctrl.len())
                .rfind(|&k| {
                    if t >= t1 {
                        knots[k] < t1
                    } else {
                        knots[k] <= t
                    }
                })
                .unwrap_or(degree);
            let mut d: Vec<[f64; 4]> = (0..=degree)
                .map(|j| {
                    let (p, w) = ctrl[k - degree + j];
                    [p[0] * w, p[1] * w, p[2] * w, w]
                })
                .collect();
            for r in 1..=degree {
                for j in (r..=degree).rev() {
                    let i = k - degree + j;
                    let den = knots[i + degree + 1 - r] - knots[i];
                    let a = if den.abs() < 1e-15 {
                        0.0
                    } else {
                        (t - knots[i]) / den
                    };
                    let prev = d[j - 1];
                    d[j] = [0, 1, 2, 3].map(|c| (1.0 - a) * prev[c] + a * d[j][c]);
                }
            }
            let p = d[degree];
            let w = if p[3].abs() < 1e-15 { 1.0 } else { p[3] };
            [p[0] / w, p[1] / w, p[2] / w]
        })
        .collect()
}

/// The curves and texts imported or linked CAD drawings draw, by import symbol id, with the
/// blocks they place (their `SlaveImportSymbol`s, `GInstance` nodes naming an `InstanceInfo`)
/// drawn in at their placement. Lines, arcs, polylines, splines and texts are read; fills and
/// hatches (`Geometry` faces) are not.
pub fn import_graphics(
    rf: &mut RevitFile,
    symbols: &BTreeSet<u64>,
) -> anyhow::Result<BTreeMap<i64, ImportGraphics>> {
    use crate::graph_value::Fv;
    use crate::native_parameters::ObjectGraph;
    // Read each wanted symbol's graph, then the blocks those place, until none is new.
    let mut graphs: BTreeMap<u64, ObjectGraph> = BTreeMap::new();
    let mut want: BTreeSet<u64> = symbols.clone();
    for _ in 0..8 {
        want.retain(|s| !graphs.contains_key(s));
        if want.is_empty() {
            break;
        }
        let mut opts = crate::native_document::Options {
            selected_ids: want.clone(),
            max_graph_values: 4_000_000,
            max_graph_objects: 4_000_000,
            ..Default::default()
        };
        opts.channels = BTreeSet::from([103]);
        crate::native_document::extract(rf, &opts, |r| {
            if let Some(g) = r.graph {
                graphs.entry(r.identity.element_id).or_insert(g);
            }
            Ok(())
        })?;
        let mut next = BTreeSet::new();
        for g in graphs.values() {
            for o in g.objects.iter().filter(|o| o.class_name == "InstanceInfo") {
                if let Some(s) = fv_id(&o.fields["m_symbolId"]).and_then(|s| u64::try_from(s).ok())
                {
                    if !graphs.contains_key(&s) {
                        next.insert(s);
                    }
                }
            }
        }
        // Symbols that have no graphics would be asked for again: stop at those.
        next.retain(|s| !want.contains(s));
        want = next;
    }
    fn draw(
        graphs: &BTreeMap<u64, ObjectGraph>,
        symbol: u64,
        m: &M4,
        out: &mut ImportGraphics,
        depth: usize,
    ) {
        let Some(g) = graphs.get(&symbol) else { return };
        if depth > 8 || g.objects.is_empty() {
            return;
        }
        let mut stack = vec![0usize];
        let mut seen = BTreeSet::new();
        while let Some(i) = stack.pop() {
            if !seen.insert(i) {
                continue;
            }
            let o = &g.objects[i];
            let f: &Fv = &o.fields;
            let layer = fv_id(&f["m_GInfo"]["m_categoryId"]).unwrap_or(-1);
            if let Some(children) = f["m_subNodes"].as_array() {
                for p in children {
                    if let Ok(Some(c)) = crate::native_saved_mesh::pointer(g, i, p) {
                        stack.push(c);
                    }
                }
            }
            match o.class_name.as_str() {
                "GLine" | "GArc" | "GEllipse" | "GHermiteSpline" => {
                    if let Ok(c) = crate::native_parametric_surface::Curve::read(g, i) {
                        use crate::native_parametric_surface::Curve;
                        let (range, n) = match &c {
                            Curve::Line { range, .. } => (*range, 1),
                            Curve::Arc { range, .. }
                            | Curve::Hermite { range, .. }
                            | Curve::Ellipse { range, .. } => (*range, 32),
                            Curve::Point(_) => continue,
                        };
                        let pts = (0..=n)
                            .map(|s| {
                                let t = range[0] + (range[1] - range[0]) * s as f64 / n as f64;
                                apply(m, c.eval(t))
                            })
                            .collect();
                        out.curves.push((pts, layer));
                    }
                }
                "GPolyLine" => {
                    if let Some(a) = f["m_coord"].as_array() {
                        let pts: Vec<[f64; 3]> =
                            a.iter().filter_map(fv_point).map(|p| apply(m, p)).collect();
                        if pts.len() > 1 {
                            out.curves.push((pts, layer));
                        }
                    }
                }
                "GNurbSpline" => {
                    let degree = f["m_Degree"].as_u64().unwrap_or(3) as usize;
                    let knots: Vec<f64> = f["m_Knots"]
                        .as_array()
                        .map(|a| a.iter().filter_map(|k| k.as_f64()).collect())
                        .unwrap_or_default();
                    let ctrl: Vec<([f64; 3], f64)> = f["m_ctrlPoints"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|c| {
                                    Some((
                                        fv_point(&c["m_Point"])?,
                                        c["m_w"].as_f64().unwrap_or(1.0),
                                    ))
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let pts: Vec<[f64; 3]> = nurbs_points(degree, &knots, &ctrl, 32)
                        .into_iter()
                        .map(|p| apply(m, p))
                        .collect();
                    if pts.len() > 1 {
                        out.curves.push((pts, layer));
                    }
                }
                "GText" => {
                    let value: String = f["m_textFragments"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|t| t["m_text"].as_str())
                                .collect::<Vec<_>>()
                                .join("")
                        })
                        .unwrap_or_default();
                    if let (Some(p), false) = (fv_point(&f["m_point"]), value.trim().is_empty()) {
                        let at = apply(m, p);
                        let base = fv_point(&f["m_baseVec"]).unwrap_or([1.0, 0.0, 0.0]);
                        let o0 = apply(m, [0.0; 3]);
                        let b = apply(m, base);
                        let (bx, by) = (b[0] - o0[0], b[1] - o0[1]);
                        let scale =
                            (bx * bx + by * by).sqrt() / (base[0].hypot(base[1])).max(1e-12);
                        out.texts.push(ImportText {
                            at,
                            height: f["m_height"].as_f64().unwrap_or(0.0) * scale,
                            rotation: by.atan2(bx),
                            value,
                            layer,
                        });
                    }
                }
                "GInstance" => {
                    let Ok(Some(ii)) =
                        crate::native_saved_mesh::pointer(g, i, &f["m_instanceInfo"])
                    else {
                        continue;
                    };
                    let info = &g.objects[ii].fields;
                    let Some(s) = fv_id(&info["m_symbolId"]).and_then(|s| u64::try_from(s).ok())
                    else {
                        continue;
                    };
                    let t: Vec<f64> = info["m_Trf"]["m_3x3"]
                        .as_array()
                        .map(|rows| {
                            rows.iter()
                                .flat_map(|r| r.as_array().unwrap_or(&[]).iter())
                                .filter_map(|v| v.as_f64())
                                .collect()
                        })
                        .unwrap_or_default();
                    let or = fv_point(&info["m_Trf"]["m_or"]).unwrap_or([0.0; 3]);
                    if t.len() != 9 {
                        continue;
                    }
                    let local: M4 = [
                        [t[0], t[1], t[2], or[0]],
                        [t[3], t[4], t[5], or[1]],
                        [t[6], t[7], t[8], or[2]],
                        [0., 0., 0., 1.],
                    ];
                    draw(graphs, s, &mul(m, &local), out, depth + 1);
                }
                _ => {}
            }
        }
    }
    let mut out = BTreeMap::new();
    for s in symbols {
        let mut g = ImportGraphics::default();
        draw(&graphs, *s, &IDENTITY4, &mut g, 0);
        if !g.curves.is_empty() || !g.texts.is_empty() {
            out.insert(*s as i64, g);
        }
    }
    Ok(out)
}
