//! Embedded family documents: the family's own solid geometry.
//!
//! A project file stores each loaded family as a content document: a keyed
//! section of the partition stream that starts at a 220-byte header holding
//! the document's GUID (header bytes 178..194, T on rac_basic 2024: 163 of 163
//! headers equal a host `FamilyDocument.m_contentDocGUID`). The host `Family`
//! element names its document by that GUID, a `FamilySymbol` names its family
//! (`m_familyId`) and a `FamilyInstance` names its symbol (`m_masterSymbolId`)
//! and carries its placement (`m_instOrigin`, `m_RefDir`, `m_zAxis`).
//!
//! The document's solid forms are ordinary elements. An `ExtrusionElem`
//! points at a `VarSketch` (`m_sketchId`), which lists its `CurveElem`s and a
//! `SketchPlane`; the depth is the `-1001800`/`-1001801` double parameters.
//! Curves are stored in the document's own 3D coordinates (`GLine`, `GArc`,
//! `GEllipse`); the sketch plane's `Trf` third column is the extrusion
//! direction, negated when `m_flipZ` is set. Decoding checks itself: the
//! extrusion's own `GElement` bounding box must equal the decoded solid's
//! (397 of 437 on rac_basic 2024 within 0.3 mm before arcs were bounded
//! exactly).
//!
//! Only forms that decode and match their stored bounding box are returned.
//! A family with any other solid form (sweep, revolve, blend, void, nested
//! family) is incomplete: [`DocSolids::complete`] is false and callers fall
//! back to the saved graphics.

use crate::RevitFile;
use crate::schema_objects::{Fields, SchemaObject, Value, for_each_object_where};
use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet};

/// A content document's GUID.
pub type Guid = [u8; 16];

/// Parameter ids of an extrusion's start and end offsets along the sketch
/// plane's normal, in feet.
const EXTRUSION_START: i64 = -1_001_800;
const EXTRUSION_END: i64 = -1_001_801;
/// Joined curve ends closer than this (feet) are one point.
const JOIN_TOLERANCE: f64 = 1e-5;
/// A decoded solid's bounding box must equal the stored one within this.
pub const BBOX_TOLERANCE: f64 = 1e-3;
/// A solved sweep or revolution is tessellated (arcs in steps of pi/16), so
/// its box may fall short of the stored one by the chord error: this share
/// of the form's size.
pub const SOLVED_BBOX_FRACTION: f64 = 0.01;

/// One decoded solid form, in the family document's coordinates (feet).
#[derive(Debug, Clone, Default)]
pub struct FamilySolid {
    pub vertices: Vec<[f64; 3]>,
    pub triangles: Vec<[u32; 3]>,
    /// The host material element the form names, `None` for "by category".
    pub material_id: Option<i64>,
    /// The form element's id in its document.
    pub form_id: i64,
    /// For an extrusion or a blend, the direction it runs along: its
    /// surface is ruled between profiles in planes normal to this, so
    /// scaling it along this axis is the same form over another length.
    /// `None` for sweeps and revolutions.
    pub ruled_axis: Option<[f64; 3]>,
}

/// What one content document yielded.
#[derive(Debug, Default, Clone)]
pub struct DocSolids {
    pub solids: Vec<FamilySolid>,
    /// Solid forms that were not decoded, by reason.
    pub undecoded: BTreeMap<&'static str, usize>,
    /// Why its sweeps, revolutions and blends did not solve, by
    /// `kind:reason` (the document's share of [`DocStats::solve_failures`]).
    pub solve_failures: BTreeMap<String, usize>,
}

impl DocSolids {
    /// Every solid form of the document decoded and checked.
    pub fn complete(&self) -> bool {
        !self.solids.is_empty() && self.undecoded.is_empty()
    }
}

/// A host family instance's placement.
#[derive(Debug, Clone)]
pub struct InstancePose {
    /// `m_masterSymbolId` of the instance.
    pub symbol: i64,
    /// `m_symbolId` of its `InstanceInfo`.
    pub info_symbol: Option<i64>,
    /// `InstanceInfo.m_Trf`: the 3x3 and the origin (model feet).
    pub matrix: [[f64; 3]; 3],
    pub origin: [f64; 3],
    pub flipped_x: bool,
    pub flipped_y: bool,
}

/// Counters for reports.
#[derive(Debug, Default, Clone)]
pub struct DocStats {
    /// Keyed sections (content documents) found.
    pub sections: usize,
    /// Extrusions seen / decoded / matching their stored bounding box.
    pub extrusions: usize,
    pub extrusions_decoded: usize,
    pub extrusions_bbox_match: usize,
    /// Other solid-form elements seen, by class.
    pub other_forms: BTreeMap<String, usize>,
    /// Sweeps, revolutions and blends solved from their sketches and
    /// matching their stored bounding box, by kind.
    pub solved: BTreeMap<String, usize>,
    /// Why the others were not, by `kind:reason`.
    pub solve_failures: BTreeMap<String, usize>,
    /// 3D paths (`Path3d`, `PathFromFace`) read, and profile family symbols
    /// whose document curves were found.
    pub paths_3d: usize,
    pub profile_families: usize,
    /// Void forms whose box meets none of their document's solids, so they
    /// cut nothing of the family's own form.
    pub voids_outside_solids: usize,
}

#[derive(Debug, Default)]
pub struct FamilyDocuments {
    /// Decoded solids by content-document GUID.
    pub docs: BTreeMap<Guid, DocSolids>,
    /// Host family element id -> content-document GUID.
    pub family_guid: BTreeMap<i64, Guid>,
    /// Host family symbol id -> host family id.
    pub symbol_family: BTreeMap<i64, i64>,
    /// Host family instance id -> placement.
    pub instances: BTreeMap<i64, InstancePose>,
    pub stats: DocStats,
}

/// Why an instance's family document could not give a body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// The family document decoded completely.
    FamilyDocument,
    /// Not an instance of a loaded family (system family, or not a
    /// `FamilyInstance` object).
    NoFamilyDocument,
    /// The family's document holds a form this reader does not decode.
    Incomplete(Vec<&'static str>),
}

impl FamilyDocuments {
    /// The decoded document of host family instance `id`.
    pub fn solids_for(&self, id: i64) -> Option<(&InstancePose, &DocSolids)> {
        let pose = self.instances.get(&id)?;
        let family = self.symbol_family.get(&pose.symbol)?;
        let guid = self.family_guid.get(family)?;
        Some((pose, self.docs.get(guid)?))
    }

    pub fn route(&self, id: i64) -> Route {
        match self.solids_for(id) {
            None => Route::NoFamilyDocument,
            Some((_, doc)) if doc.complete() => Route::FamilyDocument,
            Some((_, doc)) => {
                let mut why: Vec<&'static str> = doc.undecoded.keys().copied().collect();
                if doc.solids.is_empty() && why.is_empty() {
                    why.push("no_solid_form");
                }
                Route::Incomplete(why)
            }
        }
    }
}

// ---- reading --------------------------------------------------------------

fn id_of(f: &Fields, name: &str) -> Option<i64> {
    match f.get(name)? {
        Value::Id(i) => Some(*i),
        _ => None,
    }
}

fn float_of(v: &Value) -> Option<f64> {
    match v {
        Value::Float(f) => Some(*f),
        Value::Int(i) => Some(*i as f64),
        _ => None,
    }
}

fn vec3_of(v: &Value) -> Option<[f64; 3]> {
    match v {
        Value::List(l) if l.len() == 3 => {
            Some([float_of(&l[0])?, float_of(&l[1])?, float_of(&l[2])?])
        }
        _ => None,
    }
}

fn bool_of(f: &Fields, name: &str) -> bool {
    matches!(f.get(name), Some(Value::Bool(true)))
}

/// The object a pointer or by-value field names.
fn target<'a>(x: &'a SchemaObject, v: &'a Value) -> Option<&'a Fields> {
    match v {
        Value::New { index, .. } => x.objects.get(*index),
        Value::Object(f) => Some(f),
        _ => None,
    }
}

#[derive(Debug, Clone)]
enum Curve {
    Line {
        origin: [f64; 3],
        dir: [f64; 3],
        t: [f64; 2],
    },
    Arc {
        center: [f64; 3],
        x: [f64; 3],
        y: [f64; 3],
        radius: f64,
        t: [f64; 2],
    },
    Ellipse {
        center: [f64; 3],
        x: [f64; 3],
        y: [f64; 3],
        a: f64,
        b: f64,
        t: [f64; 2],
    },
}

struct RawExtrusion {
    guid: Guid,
    id: i64,
    material: i64,
    cutting: bool,
    sketch: i64,
    start: f64,
    end: f64,
}

/// A sweep, revolution, blend or swept blend.
struct RawForm {
    guid: Guid,
    id: i64,
    kind: &'static str,
    material: i64,
    cutting: bool,
    /// Every `...SketchId` field of the form and its profile data.
    sketch_ids: BTreeMap<String, i64>,
    /// Its double parameters by id, and its profile offsets by field name.
    doubles: BTreeMap<String, f64>,
    /// The path is picked 3D edges, not a sketch.
    path3d: bool,
}

#[derive(Clone)]
struct RawSketch {
    plane: i64,
    curves: Vec<i64>,
}

#[derive(Clone)]
struct RawPlane {
    m: [[f64; 3]; 3],
    flip_z: bool,
}

fn curve_of(x: &SchemaObject, names: &BTreeMap<u16, String>) -> Option<Curve> {
    let root = x.objects.first()?;
    let driver = target(x, root.get("m_pCurveDriver")?)?;
    let crv = target(x, driver.get("m_pCrv")?)?;
    curve_from_fields(crv, names)
}

/// A `GLine`, `GArc` or `GEllipse` object as a curve.
fn curve_from_fields(crv: &Fields, names: &BTreeMap<u16, String>) -> Option<Curve> {
    let t = |f: &Fields| -> Option<[f64; 2]> {
        match f.get("m_endParams")? {
            Value::List(l) if l.len() == 2 => Some([float_of(&l[0])?, float_of(&l[1])?]),
            _ => None,
        }
    };
    let v = |f: &Fields, n: &str| vec3_of(f.get(n)?);
    let fl = |f: &Fields, n: &str| float_of(f.get(n)?);
    match names.get(&crv.tag)?.as_str() {
        "GLine" => Some(Curve::Line {
            origin: v(crv, "m_origin")?,
            dir: v(crv, "m_dirVec")?,
            t: t(crv)?,
        }),
        "GArc" => Some(Curve::Arc {
            center: v(crv, "m_center")?,
            x: v(crv, "m_xVec")?,
            y: v(crv, "m_yVec")?,
            radius: fl(crv, "m_radius")?,
            t: t(crv)?,
        }),
        "GEllipse" => Some(Curve::Ellipse {
            center: v(crv, "m_center")?,
            x: v(crv, "m_xVec")?,
            y: v(crv, "m_yVec")?,
            a: fl(crv, "m_xLen")?,
            b: fl(crv, "m_yLen")?,
            t: t(crv)?,
        }),
        _ => None,
    }
}

fn guid_of_family_document(x: &SchemaObject, names: &BTreeMap<u16, String>) -> Option<Guid> {
    let root = x.objects.first()?;
    let doc = target(x, root.get("m_oFamDoc")?)?;
    if names.get(&doc.tag).map(String::as_str) != Some("FamilyDocument") {
        return None;
    }
    let Value::Object(g) = doc.get("m_contentDocGUID")? else {
        return None;
    };
    match g.get("m_guid")? {
        Value::Guid(b) => Some(*b),
        _ => None,
    }
}

/// Read the host's family, symbol and instance objects and every embedded
/// document's solid forms.
/// The key [`FamilyDocuments::docs`] holds a family file's own document
/// under: a standalone `.rfa` is itself a family document, so its
/// host objects are read the way a project's embedded documents are.
pub const HOST_DOCUMENT: Guid = [0; 16];

pub fn read(rf: &mut RevitFile) -> Result<FamilyDocuments> {
    let host_is_family = rf.is_family_document();
    read_with_host(rf, host_is_family)
}

/// [`read`], reading the host document's own forms under [`HOST_DOCUMENT`]
/// when `host_is_family` (a standalone family file).
pub fn read_with_host(rf: &mut RevitFile, host_is_family: bool) -> Result<FamilyDocuments> {
    let layouts = rf.schema_layouts()?;
    let names: BTreeMap<u16, String> = layouts
        .classes
        .iter()
        .map(|(t, c)| (*t, c.name.clone()))
        .collect();
    const WANT: [&str; 15] = [
        "Path3d",
        "PathFromFace",
        "ProfileRefPlane",
        "ExtrusionElem",
        "SweepElem",
        "RevolutionElem",
        "BlendElem",
        "SweptBlendElem",
        "VarSketch",
        "SketchPlane",
        "CurveElem",
        "GElement",
        "FamilyInstance",
        "FamilySymbol",
        "Family",
    ];
    let want: BTreeSet<u16> = names
        .iter()
        .filter(|(_, n)| WANT.contains(&n.as_str()))
        .map(|(t, _)| *t)
        .collect();

    // Section starts (inflated offsets) and GUIDs of every partition stream.
    let mut sections: BTreeMap<String, Vec<(usize, Guid)>> = BTreeMap::new();
    let mut section_count = 0;
    for name in rf.partition_stream_names() {
        if let Ok(inflated) = rf.inflated_partition(&name) {
            section_count += inflated.sections().len();
            sections.insert(name, inflated.sections().to_vec());
        }
    }

    let mut out = FamilyDocuments::default();
    out.stats.sections = section_count;
    let mut extrusions: Vec<RawExtrusion> = Vec::new();
    let mut forms: Vec<RawForm> = Vec::new();
    // curves per document, 3D path curves and profile-plane
    // attachment points, by element id.
    let mut doc_curves: BTreeMap<Guid, Vec<Curve>> = BTreeMap::new();
    let mut paths3d: BTreeMap<i64, Vec<Curve>> = BTreeMap::new();
    let mut profile_planes: BTreeMap<i64, [f64; 3]> = BTreeMap::new();
    let mut sketches: BTreeMap<i64, RawSketch> = BTreeMap::new();
    let mut planes: BTreeMap<i64, RawPlane> = BTreeMap::new();
    let mut curves: BTreeMap<i64, Curve> = BTreeMap::new();
    let mut boxes: BTreeMap<i64, [[f64; 3]; 2]> = BTreeMap::new();
    // Undecoded forms per document: (kind -> count).
    let mut other: BTreeMap<Guid, BTreeMap<&'static str, usize>> = BTreeMap::new();

    for_each_object_where(
        rf,
        |frame, _| want.contains(&frame.header.tag),
        |x| {
            let Some(root) = x.objects.first() else {
                return;
            };
            let Some(class) = names.get(&x.frame.header.tag) else {
                return;
            };
            let start = x.frame.span.start;
            // The document the object lies in: `None` is the host (before
            // the stream's first header).
            let mut guid = sections.get(&x.stream).and_then(|s| {
                let n = s.partition_point(|(at, _)| *at <= start);
                n.checked_sub(1).map(|i| s[i].1)
            });
            // A family file's own forms belong to its own document; its
            // `Family`, `FamilySymbol` and `FamilyInstance` objects stay the
            // host's (nested families and their instances).
            if guid.is_none()
                && host_is_family
                && !matches!(class.as_str(), "Family" | "FamilySymbol" | "FamilyInstance")
            {
                guid = Some(HOST_DOCUMENT);
            }
            let id = x.frame.header.id as i64;
            match (class.as_str(), guid) {
                ("Family", None) => {
                    if let Some(g) = guid_of_family_document(x, &names) {
                        out.family_guid.insert(id, g);
                    }
                }
                ("FamilySymbol", None) => {
                    if let Some(f) = id_of(root, "m_familyId") {
                        out.symbol_family.insert(id, f);
                    }
                }
                ("FamilyInstance", None) => {
                    let info = x
                        .objects
                        .iter()
                        .find(|o| names.get(&o.tag).map(String::as_str) == Some("InstanceInfo"));
                    let trf = info.and_then(|i| i.get("m_Trf")).and_then(|t| target(x, t));
                    let matrix = trf.and_then(|t| match t.get("m_3x3")? {
                        Value::List(rows) if rows.len() == 3 => {
                            Some([vec3_of(&rows[0])?, vec3_of(&rows[1])?, vec3_of(&rows[2])?])
                        }
                        _ => None,
                    });
                    let origin = trf.and_then(|t| vec3_of(t.get("m_or")?));
                    if let (Some(symbol), Some(matrix), Some(origin)) =
                        (id_of(root, "m_masterSymbolId"), matrix, origin)
                    {
                        out.instances.insert(
                            id,
                            InstancePose {
                                symbol,
                                info_symbol: info.and_then(|i| id_of(i, "m_symbolId")),
                                matrix,
                                origin,
                                flipped_x: bool_of(root, "m_flippedX"),
                                flipped_y: bool_of(root, "m_flippedY"),
                            },
                        );
                    }
                }
                ("FamilyInstance", Some(g)) => {
                    *other
                        .entry(g)
                        .or_default()
                        .entry("nested_family")
                        .or_default() += 1;
                }
                (_, None) => {}
                ("GElement", Some(_)) => {
                    if let Some(Value::List(b)) = root.get("m_bBox") {
                        if let [lo, hi] = b.as_slice() {
                            if let (Some(lo), Some(hi)) = (vec3_of(lo), vec3_of(hi)) {
                                boxes.insert(id, [lo, hi]);
                            }
                        }
                    }
                }
                ("ExtrusionElem", Some(g)) => {
                    out.stats.extrusions += 1;
                    let helper = x.objects.iter().find(|o| {
                        names.get(&o.tag).map(String::as_str)
                            == Some("ExtrusionElemExtrusionHelper")
                    });
                    let params = x.objects.iter().find(|o| {
                        names.get(&o.tag).map(String::as_str) == Some("ParamValueSetDouble")
                    });
                    let (mut start_p, mut end_p) = (None, None);
                    if let Some(Value::List(l)) = params.and_then(|p| p.get("m_paramSet")) {
                        for e in l {
                            if let Value::Object(pv) = e {
                                let value = pv.get("m_value").and_then(float_of);
                                match id_of(pv, "m_paramId") {
                                    Some(EXTRUSION_START) => start_p = value,
                                    Some(EXTRUSION_END) => end_p = value,
                                    _ => {}
                                }
                            }
                        }
                    }
                    match (helper.and_then(|h| id_of(h, "m_sketchId")), start_p, end_p) {
                        (Some(sketch), Some(s), Some(e)) => extrusions.push(RawExtrusion {
                            guid: g,
                            id,
                            material: id_of(root, "m_materialId").unwrap_or(-1),
                            cutting: bool_of(root, "m_cutting"),
                            sketch,
                            start: s,
                            end: e,
                        }),
                        _ => {
                            *other
                                .entry(g)
                                .or_default()
                                .entry("extrusion_fields")
                                .or_default() += 1
                        }
                    }
                }
                ("SweepElem" | "RevolutionElem" | "BlendElem" | "SweptBlendElem", Some(g)) => {
                    *out.stats.other_forms.entry(class.clone()).or_default() += 1;
                    let kind = match class.as_str() {
                        "SweepElem" => "sweep",
                        "RevolutionElem" => "revolution",
                        "BlendElem" => "blend",
                        _ => "swept_blend",
                    };
                    // the form's sketches and parameters, solved
                    // below; it stays undecoded when that fails.
                    let mut sketch_ids = BTreeMap::new();
                    let mut doubles = BTreeMap::new();
                    for o in &x.objects {
                        for (n, v) in &o.values {
                            if let (true, Value::Id(i)) =
                                (n.to_ascii_lowercase().ends_with("sketchid"), v)
                            {
                                sketch_ids.entry(n.clone()).or_insert(*i);
                            }
                            if let ("m_sketchIds", Value::List(l)) = (n.as_str(), v) {
                                for (k, e) in l.iter().enumerate() {
                                    if let Value::Id(i) = e {
                                        sketch_ids.insert(format!("m_sketchIds{k}"), *i);
                                    }
                                }
                            }
                            if let ("m_path3dId", Value::Id(i)) = (n.as_str(), v) {
                                if *i > 0 {
                                    sketch_ids.insert("m_path3dId".into(), *i);
                                }
                            }
                            if n == "m_profileData" {
                                let first = match v {
                                    Value::List(l) => l.first(),
                                    other => Some(other),
                                };
                                if let Some(Value::Object(pd)) = first {
                                    for (pn, pv) in &pd.values {
                                        match pv {
                                            Value::Id(i) if *i > 0 => {
                                                sketch_ids.entry(pn.clone()).or_insert(*i);
                                            }
                                            Value::Float(f) => {
                                                doubles.insert(pn.clone(), *f);
                                            }
                                            Value::Bool(true) => {
                                                doubles.insert(pn.clone(), 1.0);
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                            }
                        }
                        if names.get(&o.tag).map(String::as_str) == Some("ParamValueSetDouble") {
                            if let Some(Value::List(l)) = o.get("m_paramSet") {
                                for e in l {
                                    if let Value::Object(pv) = e {
                                        if let (Some(pid), Some(val)) = (
                                            id_of(pv, "m_paramId"),
                                            pv.get("m_value").and_then(float_of),
                                        ) {
                                            doubles.insert(pid.to_string(), val);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    let path3d = id_of(root, "m_path3dId").is_some_and(|i| i > 0);
                    forms.push(RawForm {
                        guid: g,
                        id,
                        kind,
                        material: id_of(root, "m_materialId").unwrap_or(-1),
                        cutting: bool_of(root, "m_cutting"),
                        sketch_ids,
                        doubles,
                        path3d,
                    });
                }
                ("VarSketch", Some(_)) => {
                    let Some(plane) = id_of(root, "m_sketchPlaneId") else {
                        return;
                    };
                    let mut ids = Vec::new();
                    if let Some(Value::List(l)) = root.get("m_curveObjIdxMap") {
                        for e in l {
                            if let Value::Object(p) = e {
                                if let Some(c) = id_of(p, "first") {
                                    ids.push(c);
                                }
                            }
                        }
                    }
                    sketches.insert(id, RawSketch { plane, curves: ids });
                }
                ("SketchPlane", Some(_)) => {
                    let Some(trf) = root.get("m_oTrf").and_then(|t| target(x, t)) else {
                        return;
                    };
                    let Some(Value::List(rows)) = trf.get("m_3x3") else {
                        return;
                    };
                    let mut m = [[0.0; 3]; 3];
                    if rows.len() != 3 {
                        return;
                    }
                    for (r, row) in rows.iter().enumerate() {
                        let Some(v) = vec3_of(row) else {
                            return;
                        };
                        m[r] = v;
                    }
                    planes.insert(
                        id,
                        RawPlane {
                            m,
                            flip_z: bool_of(root, "m_flipZ"),
                        },
                    );
                }
                ("CurveElem", Some(g)) => {
                    if let Some(c) = curve_of(x, &names) {
                        doc_curves.entry(g).or_default().push(c.clone());
                        curves.insert(id, c);
                    }
                }
                ("Path3d" | "PathFromFace", Some(_)) => {
                    // The path's own curves, inline.
                    let list: Vec<Curve> = x
                        .objects
                        .iter()
                        .filter_map(|o| curve_from_fields(o, &names))
                        .collect();
                    if !list.is_empty() {
                        paths3d.insert(id, list);
                    }
                }
                ("ProfileRefPlane", Some(_)) => {
                    if let Some(p) = root.get("m_attachmentPoint").and_then(vec3_of) {
                        profile_planes.insert(id, p);
                    }
                }
                _ => {}
            }
        },
    )?;

    for (g, kinds) in other {
        out.docs.entry(g).or_default().undecoded.extend(kinds);
    }
    // Every document that has any form is listed, even when empty, so a
    // caller can tell "no solid form" from "not a document".
    for (_, g) in sections.values().flatten() {
        out.docs.entry(*g).or_default();
    }
    if host_is_family {
        out.docs.entry(HOST_DOCUMENT).or_default();
    }
    // Void forms, with their box when they solve: a void cuts the
    // document's own solids only where its box meets theirs.
    let mut voids: Vec<(Guid, Option<Bounds>)> = Vec::new();
    for e in &extrusions {
        let doc = out.docs.entry(e.guid).or_default();
        if e.cutting {
            voids.push((
                e.guid,
                extrusion_solid(e, &sketches, &planes, &curves).map(|(_, b)| b),
            ));
            continue;
        }
        let solid = extrusion_solid(e, &sketches, &planes, &curves);
        let Some((solid, bbox)) = solid else {
            *doc.undecoded.entry("extrusion_geometry").or_default() += 1;
            continue;
        };
        out.stats.extrusions_decoded += 1;
        let matches = boxes.get(&e.id).is_none_or(|stored| {
            (0..2).all(|i| (0..3).all(|k| (stored[i][k] - bbox[i][k]).abs() <= BBOX_TOLERANCE))
        });
        if !matches {
            *doc.undecoded.entry("extrusion_bbox_mismatch").or_default() += 1;
            continue;
        }
        out.stats.extrusions_bbox_match += 1;
        doc.solids.push(FamilySolid {
            form_id: e.id,
            ..solid
        });
    }
    let mut profile_curves = BTreeMap::new();
    for f in &forms {
        if let Some(sym) = f.sketch_ids.get("m_profileFamSymId") {
            let doc = out
                .symbol_family
                .get(sym)
                .and_then(|fam| out.family_guid.get(fam))
                .and_then(|g| doc_curves.get(g));
            if let Some(c) = doc {
                profile_curves.insert(*sym, c.clone());
            }
        }
    }
    out.stats.paths_3d = paths3d.len();
    out.stats.profile_families = profile_curves.len();
    let ctx = SolveCtx {
        sketches,
        planes,
        curves,
        paths3d,
        profile_planes,
        profile_curves,
    };
    for f in &forms {
        let doc = out.docs.entry(f.guid).or_default();
        if f.cutting {
            voids.push((f.guid, solve_form(f, &ctx).ok().map(|(_, b)| b)));
            continue;
        }
        match solve_form(f, &ctx) {
            Err(why) => {
                *doc.undecoded.entry(f.kind).or_default() += 1;
                let reason = format!("{}:{why}", f.kind);
                *doc.solve_failures.entry(reason.clone()).or_default() += 1;
                *out.stats.solve_failures.entry(reason).or_default() += 1;
            }
            Ok((solid, bbox)) => {
                let matches = boxes.get(&f.id).is_none_or(|stored| {
                    let size = (0..3)
                        .map(|k| (stored[1][k] - stored[0][k]).abs())
                        .fold(0.0, f64::max);
                    let tol = BBOX_TOLERANCE.max(SOLVED_BBOX_FRACTION * size);
                    (0..2).all(|i| (0..3).all(|k| (stored[i][k] - bbox[i][k]).abs() <= tol))
                });
                if !matches {
                    *doc.undecoded.entry(f.kind).or_default() += 1;
                    let reason = format!("{}:bbox_mismatch", f.kind);
                    *doc.solve_failures.entry(reason.clone()).or_default() += 1;
                    *out.stats.solve_failures.entry(reason).or_default() += 1;
                    continue;
                }
                *out.stats.solved.entry(f.kind.to_string()).or_default() += 1;
                doc.solids.push(FamilySolid {
                    form_id: f.id,
                    material_id: (f.material > 0).then_some(f.material),
                    ..solid
                });
            }
        }
    }
    // A void whose box meets none of its document's solids cuts nothing of
    // the family's own form (a void that cuts the host it is placed in, such
    // as an opening, lies outside them): the document stays complete. A void
    // that does not solve, or that meets a solid, leaves it incomplete.
    for (g, void_box) in voids {
        let doc = out.docs.entry(g).or_default();
        let cuts = void_box.is_none_or(|v| {
            doc.solids.iter().any(|s| {
                bounds(s.vertices.iter().copied())
                    .is_some_and(|b| (0..3).all(|k| v[0][k] < b[1][k] && b[0][k] < v[1][k]))
            })
        });
        if cuts {
            *doc.undecoded.entry("void").or_default() += 1;
        } else {
            out.stats.voids_outside_solids += 1;
        }
    }
    Ok(out)
}

// ---- geometry -------------------------------------------------------------

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn norm(a: [f64; 3]) -> [f64; 3] {
    let l = dot(a, a).sqrt();
    if l < 1e-12 {
        a
    } else {
        [a[0] / l, a[1] / l, a[2] / l]
    }
}
fn dist(a: [f64; 3], b: [f64; 3]) -> f64 {
    let d = sub(a, b);
    dot(d, d).sqrt()
}

/// A curve's points from its start to its end, and whether it closes on
/// itself, with the exact extreme points along each axis for bounding boxes.
fn polyline(c: &Curve) -> (Vec<[f64; 3]>, bool, Vec<[f64; 3]>) {
    use std::f64::consts::TAU;
    match c {
        Curve::Line { origin, dir, t } => {
            let p = |s: f64| std::array::from_fn(|k| origin[k] + dir[k] * s);
            let pts = vec![p(t[0]), p(t[1])];
            (pts.clone(), false, pts)
        }
        Curve::Arc {
            center,
            x,
            y,
            radius,
            t,
        } => {
            let ell = Curve::Ellipse {
                center: *center,
                x: *x,
                y: *y,
                a: *radius,
                b: *radius,
                t: *t,
            };
            polyline(&ell)
        }
        Curve::Ellipse {
            center,
            x,
            y,
            a,
            b,
            t,
        } => {
            // Equal end parameters are the whole curve.
            let (t0, mut t1) = (t[0], t[1]);
            if (t1 - t0).abs() < 1e-12 {
                t1 = t0 + TAU;
            }
            let at = |s: f64| -> [f64; 3] {
                std::array::from_fn(|k| center[k] + a * s.cos() * x[k] + b * s.sin() * y[k])
            };
            let sweep = t1 - t0;
            let closed = (sweep.abs() - TAU).abs() < 1e-9;
            let n = ((sweep.abs() / (std::f64::consts::PI / 16.0)).ceil() as usize).max(4);
            let mut pts: Vec<[f64; 3]> = (0..=n)
                .map(|i| at(t0 + sweep * i as f64 / n as f64))
                .collect();
            if closed {
                pts.pop();
            }
            // Per axis, the parameter where the curve is extreme.
            let (lo, hi) = (t0.min(t1), t0.max(t1));
            let mut ext = vec![at(t0), at(t1)];
            for k in 0..3 {
                let theta = (b * y[k]).atan2(a * x[k]);
                for cand in [theta, theta + std::f64::consts::PI] {
                    let mut s = cand;
                    while s < lo {
                        s += TAU;
                    }
                    while s - TAU >= lo {
                        s -= TAU;
                    }
                    if s <= hi {
                        ext.push(at(s));
                    }
                }
            }
            (pts, closed, ext)
        }
    }
}

type Bounds = [[f64; 3]; 2];

fn bounds(points: impl Iterator<Item = [f64; 3]>) -> Option<Bounds> {
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    let mut any = false;
    for p in points {
        any = true;
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    any.then_some([lo, hi])
}

/// Join curve pieces into closed loops (each without its repeated first
/// point). `None` when a chain does not close.
fn loops(pieces: &[(Vec<[f64; 3]>, bool)]) -> Option<Vec<Vec<[f64; 3]>>> {
    let mut used = vec![false; pieces.len()];
    let mut out = Vec::new();
    for first in 0..pieces.len() {
        if used[first] {
            continue;
        }
        used[first] = true;
        let mut ring = pieces[first].0.clone();
        if pieces[first].1 {
            out.push(ring);
            continue;
        }
        let start = ring[0];
        loop {
            let end = *ring.last()?;
            if ring.len() > 2 && dist(end, start) <= JOIN_TOLERANCE {
                ring.pop();
                break;
            }
            let next = (0..pieces.len()).find(|&j| {
                !used[j]
                    && !pieces[j].1
                    && (dist(pieces[j].0[0], end) <= JOIN_TOLERANCE
                        || dist(*pieces[j].0.last().unwrap(), end) <= JOIN_TOLERANCE)
            })?;
            used[next] = true;
            let p = &pieces[next].0;
            if dist(p[0], end) <= JOIN_TOLERANCE {
                ring.extend(p.iter().skip(1).copied());
            } else {
                ring.extend(p.iter().rev().skip(1).copied());
            }
        }
        if ring.len() >= 3 {
            out.push(ring);
        }
    }
    Some(out)
}

fn area2(p: &[[f64; 2]]) -> f64 {
    let n = p.len();
    (0..n)
        .map(|i| p[i][0] * p[(i + 1) % n][1] - p[(i + 1) % n][0] * p[i][1])
        .sum::<f64>()
        / 2.0
}

fn inside2(poly: &[[f64; 2]], q: [f64; 2]) -> bool {
    let mut c = false;
    let n = poly.len();
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        if (a[1] > q[1]) != (b[1] > q[1])
            && q[0] < (b[0] - a[0]) * (q[1] - a[1]) / (b[1] - a[1]) + a[0]
        {
            c = !c;
        }
    }
    c
}

/// Extrude closed loops along `n` from offset `s0` to `s1`.
fn prism(rings: &[Vec<[f64; 3]>], n: [f64; 3], s0: f64, s1: f64) -> Option<FamilySolid> {
    let (lo, hi) = (s0.min(s1), s0.max(s1));
    if hi - lo < 1e-9 {
        return None;
    }
    // Plane frame for the 2D work.
    let seed = if n[2].abs() < 0.9 {
        [0.0, 0.0, 1.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let u = norm(cross(seed, n));
    let v = cross(n, u);
    let o = rings.first()?.first().copied()?;
    let flat: Vec<Vec<[f64; 2]>> = rings
        .iter()
        .map(|r| {
            r.iter()
                .map(|p| {
                    let d = sub(*p, o);
                    [dot(d, u), dot(d, v)]
                })
                .collect()
        })
        .collect();
    // Outer loops are not inside another; a loop inside one is its hole.
    let areas: Vec<f64> = flat.iter().map(|r| area2(r)).collect();
    let mut parent: Vec<Option<usize>> = vec![None; flat.len()];
    for i in 0..flat.len() {
        let mut best: Option<usize> = None;
        for j in 0..flat.len() {
            if i != j
                && areas[j].abs() > areas[i].abs()
                && inside2(&flat[j], flat[i][0])
                && best.is_none_or(|b| areas[j].abs() < areas[b].abs())
            {
                best = Some(j);
            }
        }
        parent[i] = best;
    }
    let depth = |mut i: usize| {
        let mut d = 0;
        while let Some(p) = parent[i] {
            d += 1;
            i = p;
        }
        d
    };
    let mut solid = FamilySolid::default();
    let place = |p: [f64; 3], s: f64| -> [f64; 3] { std::array::from_fn(|k| p[k] + n[k] * s) };
    for i in 0..flat.len() {
        if depth(i) % 2 == 1 {
            continue;
        }
        // This outer loop and its direct holes, oriented outer CCW, hole CW.
        let mut members = vec![i];
        members.extend((0..flat.len()).filter(|&j| parent[j] == Some(i) && depth(j) % 2 == 1));
        let mut pts2: Vec<[f64; 2]> = Vec::new();
        let mut pts3: Vec<[f64; 3]> = Vec::new();
        let mut holes: Vec<u32> = Vec::new();
        let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
        for (k, &m) in members.iter().enumerate() {
            let mut idx: Vec<usize> = (0..flat[m].len()).collect();
            let want_ccw = k == 0;
            if (areas[m] > 0.0) != want_ccw {
                idx.reverse();
            }
            if k > 0 {
                holes.push(pts2.len() as u32);
            }
            let start = pts2.len();
            for &j in &idx {
                pts2.push(flat[m][j]);
                pts3.push(rings[m][j]);
            }
            ranges.push(start..pts2.len());
        }
        let mut ts = Vec::<u32>::new();
        earcut::Earcut::new().earcut(pts2.iter().copied(), &holes, &mut ts);
        if ts.is_empty() || !ts.len().is_multiple_of(3) {
            return None;
        }
        let base = solid.vertices.len() as u32;
        let count = pts3.len() as u32;
        for p in &pts3 {
            solid.vertices.push(place(*p, lo));
        }
        for p in &pts3 {
            solid.vertices.push(place(*p, hi));
        }
        for t in ts.chunks_exact(3) {
            let (a, b, c) = (
                pts2[t[0] as usize],
                pts2[t[1] as usize],
                pts2[t[2] as usize],
            );
            let cr = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
            if cr.abs() < 1e-14 {
                continue;
            }
            // Top cap faces +n (CCW in (u, v) with v = n x u), bottom -n.
            let (t0, t1, t2) = if cr > 0.0 {
                (t[0], t[1], t[2])
            } else {
                (t[0], t[2], t[1])
            };
            solid
                .triangles
                .push([base + count + t0, base + count + t1, base + count + t2]);
            solid.triangles.push([base + t0, base + t2, base + t1]);
        }
        for r in &ranges {
            let len = r.len();
            for j in 0..len {
                let a = (r.start + j) as u32;
                let b = (r.start + (j + 1) % len) as u32;
                solid.triangles.push([base + a, base + b, base + count + b]);
                solid
                    .triangles
                    .push([base + a, base + count + b, base + count + a]);
            }
        }
    }
    (!solid.triangles.is_empty()).then_some(solid)
}

fn extrusion_solid(
    e: &RawExtrusion,
    sketches: &BTreeMap<i64, RawSketch>,
    planes: &BTreeMap<i64, RawPlane>,
    curves: &BTreeMap<i64, Curve>,
) -> Option<(FamilySolid, Bounds)> {
    let sketch = sketches.get(&e.sketch)?;
    let plane = planes.get(&sketch.plane)?;
    let sign = if plane.flip_z { -1.0 } else { 1.0 };
    let n = [
        sign * plane.m[0][2],
        sign * plane.m[1][2],
        sign * plane.m[2][2],
    ];
    let mut pieces = Vec::new();
    let mut extremes = Vec::new();
    for id in &sketch.curves {
        let (pts, closed, ext) = polyline(curves.get(id)?);
        extremes.extend(ext);
        pieces.push((pts, closed));
    }
    if pieces.is_empty() {
        return None;
    }
    let rings = loops(&pieces)?;
    let mut solid = prism(&rings, n, e.start, e.end)?;
    solid.material_id = (e.material > 0).then_some(e.material);
    solid.ruled_axis = Some(n);
    let ends = [e.start, e.end];
    let bbox = bounds(extremes.iter().flat_map(|p| {
        ends.iter()
            .map(move |s| -> [f64; 3] { std::array::from_fn(|k| p[k] + n[k] * s) })
    }))?;
    Some((solid, bbox))
}

// ---- sweeps, revolutions and blends ---------------------------
//
// These forms keep no B-rep in the file (their `GenSweepGStep`
// holds only face and edge history), so they are rebuilt from their
// sketches, whose curves lie in the family's 3D coordinates like an
// extrusion's, and checked against the form's stored bounding box.

/// `REVOLUTION_START_ANGLE` / `REVOLUTION_END_ANGLE` (radians).
const REVOLUTION_START: &str = "-1001802";
const REVOLUTION_END: &str = "-1001803";

/// A curve's points and whether it closes on itself.
type Piece = (Vec<[f64; 3]>, bool);
/// Start and end cap triangles of a loft.
type Caps<'a> = (&'a [[u32; 3]], &'a [[u32; 3]]);

/// A sketch's curves as polylines, with their exact axis extremes.
fn sketch_pieces(
    sketch: &RawSketch,
    curves: &BTreeMap<i64, Curve>,
) -> Option<(Vec<Piece>, Vec<[f64; 3]>)> {
    let mut pieces = Vec::new();
    let mut extremes = Vec::new();
    for id in &sketch.curves {
        let (pts, closed, ext) = polyline(curves.get(id)?);
        extremes.extend(ext);
        pieces.push((pts, closed));
    }
    (!pieces.is_empty()).then_some((pieces, extremes))
}

fn plane_normal(sketch: &RawSketch, planes: &BTreeMap<i64, RawPlane>) -> Option<[f64; 3]> {
    let plane = planes.get(&sketch.plane)?;
    let sign = if plane.flip_z { -1.0 } else { 1.0 };
    Some([
        sign * plane.m[0][2],
        sign * plane.m[1][2],
        sign * plane.m[2][2],
    ])
}

/// Join pieces into one open chain (or a closed ring), in order.
fn chain(pieces: &[(Vec<[f64; 3]>, bool)]) -> Option<(Vec<[f64; 3]>, bool)> {
    if let [(p, true)] = pieces {
        return Some((p.clone(), true));
    }
    if let Some(rings) = loops(pieces) {
        if let [ring] = rings.as_slice() {
            return Some((ring.clone(), true));
        }
    }
    // Open: start at an end no other piece touches.
    let ends: Vec<[f64; 3]> = pieces
        .iter()
        .flat_map(|(p, _)| [p[0], *p.last().unwrap()])
        .collect();
    let lonely = |q: [f64; 3]| {
        ends.iter()
            .filter(|e| dist(**e, q) <= JOIN_TOLERANCE)
            .count()
            == 1
    };
    let first = (0..pieces.len())
        .find(|&i| lonely(pieces[i].0[0]) || lonely(*pieces[i].0.last().unwrap()))?;
    let mut used = vec![false; pieces.len()];
    used[first] = true;
    let mut path = pieces[first].0.clone();
    if !lonely(path[0]) {
        path.reverse();
    }
    loop {
        let end = *path.last()?;
        let Some(next) = (0..pieces.len()).find(|&j| {
            !used[j]
                && (dist(pieces[j].0[0], end) <= JOIN_TOLERANCE
                    || dist(*pieces[j].0.last().unwrap(), end) <= JOIN_TOLERANCE)
        }) else {
            break;
        };
        used[next] = true;
        let p = &pieces[next].0;
        if dist(p[0], end) <= JOIN_TOLERANCE {
            path.extend(p.iter().skip(1).copied());
        } else {
            path.extend(p.iter().rev().skip(1).copied());
        }
    }
    used.iter().all(|u| *u).then_some((path, false))
}

/// Cap triangles over closed rings lying in a plane of normal `n`: indices
/// into the rings' points taken in order, wound counter-clockwise about `n`.
fn cap_triangles(rings: &[Vec<[f64; 3]>], n: [f64; 3]) -> Option<Vec<[u32; 3]>> {
    let seed = if n[2].abs() < 0.9 {
        [0.0, 0.0, 1.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let u = norm(cross(seed, n));
    let v = cross(n, u);
    let o = rings.first()?.first().copied()?;
    let flat: Vec<Vec<[f64; 2]>> = rings
        .iter()
        .map(|r| {
            r.iter()
                .map(|p| {
                    let d = sub(*p, o);
                    [dot(d, u), dot(d, v)]
                })
                .collect()
        })
        .collect();
    let offsets: Vec<u32> = rings
        .iter()
        .scan(0u32, |acc, r| {
            let at = *acc;
            *acc += r.len() as u32;
            Some(at)
        })
        .collect();
    let areas: Vec<f64> = flat.iter().map(|r| area2(r)).collect();
    let mut parent: Vec<Option<usize>> = vec![None; flat.len()];
    for i in 0..flat.len() {
        for j in 0..flat.len() {
            if i != j
                && areas[j].abs() > areas[i].abs()
                && inside2(&flat[j], flat[i][0])
                && parent[i].is_none_or(|b| areas[j].abs() < areas[b].abs())
            {
                parent[i] = Some(j);
            }
        }
    }
    let depth = |mut i: usize| {
        let mut d = 0;
        while let Some(p) = parent[i] {
            d += 1;
            i = p;
        }
        d
    };
    let mut out = Vec::new();
    for i in 0..flat.len() {
        if depth(i) % 2 == 1 {
            continue;
        }
        let mut members = vec![i];
        members.extend((0..flat.len()).filter(|&j| parent[j] == Some(i) && depth(j) % 2 == 1));
        let mut pts2: Vec<[f64; 2]> = Vec::new();
        let mut index: Vec<u32> = Vec::new();
        let mut holes: Vec<u32> = Vec::new();
        for (k, &m) in members.iter().enumerate() {
            if k > 0 {
                holes.push(pts2.len() as u32);
            }
            for (j, p) in flat[m].iter().enumerate() {
                pts2.push(*p);
                index.push(offsets[m] + j as u32);
            }
        }
        let mut ts = Vec::<u32>::new();
        earcut::Earcut::new().earcut(pts2.iter().copied(), &holes, &mut ts);
        if !ts.len().is_multiple_of(3) {
            return None;
        }
        for t in ts.chunks_exact(3) {
            let (a, b, c) = (
                pts2[t[0] as usize],
                pts2[t[1] as usize],
                pts2[t[2] as usize],
            );
            let cr = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
            if cr.abs() < 1e-14 {
                continue;
            }
            let (i0, i1, i2) = (
                index[t[0] as usize],
                index[t[1] as usize],
                index[t[2] as usize],
            );
            out.push(if cr > 0.0 { [i0, i1, i2] } else { [i0, i2, i1] });
        }
    }
    (!out.is_empty()).then_some(out)
}

/// Loft between ring sets that correspond point for point: `sections[i]` is
/// the rings at station i. `caps` closes the first and last station with the
/// first station's cap triangulation (`None`: the loft closes on itself).
fn loft(
    sections: &[Vec<Vec<[f64; 3]>>],
    closed: bool,
    caps: Option<Caps<'_>>,
) -> Option<FamilySolid> {
    let first = sections.first()?;
    let per: usize = first.iter().map(Vec::len).sum();
    if sections
        .iter()
        .any(|s| s.iter().map(Vec::len).sum::<usize>() != per)
    {
        return None;
    }
    let mut solid = FamilySolid::default();
    for s in sections {
        for r in s {
            solid.vertices.extend(r.iter().copied());
        }
    }
    let stations = sections.len();
    let bands = if closed { stations } else { stations - 1 };
    for i in 0..bands {
        let (a, b) = (i, (i + 1) % stations);
        let mut off = 0u32;
        for r in first {
            let len = r.len() as u32;
            for j in 0..len {
                let k = (j + 1) % len;
                let p = |st: usize, q: u32| (st * per) as u32 + off + q;
                solid.triangles.push([p(a, j), p(a, k), p(b, k)]);
                solid.triangles.push([p(a, j), p(b, k), p(b, j)]);
            }
            off += len;
        }
    }
    if let Some((start, end)) = caps {
        let last = ((stations - 1) * per) as u32;
        for t in start {
            solid.triangles.push([t[0], t[2], t[1]]);
        }
        for t in end {
            solid
                .triangles
                .push([last + t[0], last + t[1], last + t[2]]);
        }
    }
    orient_consistently(&mut solid.triangles);
    // Wind outward: a negative signed volume means every face is inside out.
    let volume: f64 = solid
        .triangles
        .iter()
        .map(|t| {
            let [a, b, c] = t.map(|i| solid.vertices[i as usize]);
            dot(a, cross(b, c))
        })
        .sum();
    if volume < 0.0 {
        for t in &mut solid.triangles {
            t.swap(1, 2);
        }
    }
    (!solid.triangles.is_empty()).then_some(solid)
}

/// A blend between two closed rings of different vertex counts, each closed
/// by its cap (`cb`, `ct`, indexed into its own ring): the side is
/// triangulated by walking both rings at once in arc-length order, so every
/// vertex of each ring is kept and no new one is made. The rings run the
/// same way round and start at corresponding points.
fn blend_unequal(
    bot: &[[f64; 3]],
    top: &[[f64; 3]],
    cb: &[[u32; 3]],
    ct: &[[u32; 3]],
) -> Option<FamilySolid> {
    let (n, m) = (bot.len(), top.len());
    if n < 3 || m < 3 {
        return None;
    }
    // Each vertex's share of its ring's perimeter, from the start (index
    // `len` is 1.0, the start again).
    let at = |r: &[[f64; 3]]| -> Option<Vec<f64>> {
        let mut acc = vec![0.0];
        for (k, p) in r.iter().enumerate() {
            let d = dist(*p, r[(k + 1) % r.len()]);
            acc.push(acc[k] + d);
        }
        let total = *acc.last()?;
        (total > 1e-12).then(|| acc.iter().map(|a| a / total).collect())
    };
    let (tb, tt) = (at(bot)?, at(top)?);
    let mut solid = FamilySolid::default();
    solid.vertices.extend_from_slice(bot);
    solid.vertices.extend_from_slice(top);
    let (b, t) = (|i: usize| (i % n) as u32, |j: usize| (n + j % m) as u32);
    let (mut i, mut j) = (0, 0);
    while i < n || j < m {
        let step_bottom = j == m || (i < n && tb[i + 1] <= tt[j + 1]);
        if step_bottom {
            solid.triangles.push([b(i), b(i + 1), t(j)]);
            i += 1;
        } else {
            solid.triangles.push([b(i), t(j + 1), t(j)]);
            j += 1;
        }
    }
    for c in cb {
        solid.triangles.push([c[0], c[2], c[1]]);
    }
    for c in ct {
        solid.triangles.push(c.map(|v| v + n as u32));
    }
    orient_consistently(&mut solid.triangles);
    let volume: f64 = solid
        .triangles
        .iter()
        .map(|t| {
            let [a, b, c] = t.map(|i| solid.vertices[i as usize]);
            dot(a, cross(b, c))
        })
        .sum();
    if volume < 0.0 {
        for t in &mut solid.triangles {
            t.swap(1, 2);
        }
    }
    Some(solid)
}

/// Flip triangles so that every edge two triangles share is used once each
/// way (a breadth-first walk from each unvisited triangle).
fn orient_consistently(triangles: &mut [[u32; 3]]) {
    let mut by_edge: BTreeMap<(u32, u32), Vec<usize>> = BTreeMap::new();
    for (i, t) in triangles.iter().enumerate() {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            by_edge.entry((a.min(b), a.max(b))).or_default().push(i);
        }
    }
    let has = |t: &[u32; 3], a: u32, b: u32| (0..3).any(|k| t[k] == a && t[(k + 1) % 3] == b);
    let mut seen = vec![false; triangles.len()];
    for start in 0..triangles.len() {
        if seen[start] {
            continue;
        }
        seen[start] = true;
        let mut queue = std::collections::VecDeque::from([start]);
        while let Some(i) = queue.pop_front() {
            let t = triangles[i];
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                for &j in &by_edge[&(a.min(b), a.max(b))] {
                    if seen[j] {
                        continue;
                    }
                    seen[j] = true;
                    // A consistent neighbour runs the shared edge b -> a.
                    if has(&triangles[j], a, b) {
                        triangles[j].swap(1, 2);
                    }
                    queue.push_back(j);
                }
            }
        }
    }
}

fn rotate_about(p: [f64; 3], o: [f64; 3], axis: [f64; 3], angle: f64) -> [f64; 3] {
    let (s, c) = angle.sin_cos();
    let v = sub(p, o);
    let k = axis;
    let kv = cross(k, v);
    let kd = dot(k, v);
    std::array::from_fn(|i| o[i] + v[i] * c + kv[i] * s + k[i] * kd * (1.0 - c))
}

/// What the solvers read besides the form.
#[derive(Default)]
struct SolveCtx {
    sketches: BTreeMap<i64, RawSketch>,
    planes: BTreeMap<i64, RawPlane>,
    curves: BTreeMap<i64, Curve>,
    /// `Path3d` / `PathFromFace` curves by element id.
    paths3d: BTreeMap<i64, Vec<Curve>>,
    /// `ProfileRefPlane.m_attachmentPoint` by element id.
    profile_planes: BTreeMap<i64, [f64; 3]>,
    /// A profile family symbol's document curves, by host symbol id.
    profile_curves: BTreeMap<i64, Vec<Curve>>,
}

/// A profile family's loops (drawn in its document's XY plane) placed on the
/// plane across the path's tangent `t` at `q`: the profile's X along the
/// horizontal `Z x t` (world X when the path is vertical), its Y along
/// `t x X`, offset by `m_xOffset`/`m_yOffset`, turned by `m_angle`, X
/// negated when `m_flippedX`. Checked by the stored box like every solve.
fn place_profile(
    curves: &[Curve],
    q: [f64; 3],
    t: [f64; 3],
    doubles: &BTreeMap<String, f64>,
) -> Option<Vec<Vec<[f64; 3]>>> {
    let pieces: Vec<Piece> = curves
        .iter()
        .map(|c| {
            let (p, closed, _) = polyline(c);
            (p, closed)
        })
        .collect();
    let rings = loops(&pieces)?;
    if rings.is_empty() {
        return None;
    }
    let z = [0.0, 0.0, 1.0];
    let x = if cross(z, t).iter().map(|v| v * v).sum::<f64>() < 1e-12 {
        [1.0, 0.0, 0.0]
    } else {
        norm(cross(z, t))
    };
    let y = cross(t, x);
    let get = |k: &str| doubles.get(k).copied().unwrap_or(0.0);
    let (dx, dy, a) = (get("m_xOffset"), get("m_yOffset"), get("m_angle"));
    let flip = if get("m_flippedX") != 0.0 { -1.0 } else { 1.0 };
    let (sa, ca) = a.sin_cos();
    Some(
        rings
            .iter()
            .map(|r| {
                r.iter()
                    .map(|p| {
                        let (px, py) = (flip * p[0] + dx, p[1] + dy);
                        let (u, v) = (px * ca - py * sa, px * sa + py * ca);
                        std::array::from_fn(|k| q[k] + x[k] * u + y[k] * v)
                    })
                    .collect()
            })
            .collect(),
    )
}

fn solve_form(f: &RawForm, ctx: &SolveCtx) -> Result<(FamilySolid, Bounds), &'static str> {
    let (sketches, planes, curves) = (&ctx.sketches, &ctx.planes, &ctx.curves);
    let sketch = |name: &str| -> Result<&RawSketch, &'static str> {
        let id = f.sketch_ids.get(name).ok_or("sketch_field")?;
        sketches.get(id).ok_or("sketch")
    };
    match f.kind {
        "revolution" => {
            let sk = sketch("m_sketchId")?;
            let (pieces, extremes) = sketch_pieces(sk, curves).ok_or("curves")?;
            // The axis is the sketch's one line that is in no closed loop.
            let mut axis = None;
            let mut profile = Vec::new();
            for (i, (pts, closed)) in pieces.iter().enumerate() {
                let others: Vec<_> = pieces
                    .iter()
                    .enumerate()
                    .filter(|(j, _)| *j != i)
                    .map(|(_, p)| p.clone())
                    .collect();
                let is_line = !closed && pts.len() == 2;
                if is_line && axis.is_none() && loops(&others).is_some() && !others.is_empty() {
                    axis = Some((pts[0], norm(sub(pts[1], pts[0]))));
                } else {
                    profile.push((pts.clone(), *closed));
                }
            }
            let (o, d) = axis.ok_or("axis")?;
            let rings = loops(&profile).ok_or("profile_loops")?;
            if rings.is_empty() {
                return Err("profile_loops");
            }
            let a0 = f.doubles.get(REVOLUTION_START).copied().unwrap_or(0.0);
            let a1 = f
                .doubles
                .get(REVOLUTION_END)
                .copied()
                .unwrap_or(std::f64::consts::TAU);
            let sweep = a1 - a0;
            if sweep.abs() < 1e-9 {
                return Err("angle");
            }
            let full = (sweep.abs() - std::f64::consts::TAU).abs() < 1e-6;
            let n = ((sweep.abs() / (std::f64::consts::PI / 16.0)).ceil() as usize).max(4);
            let stations: Vec<Vec<Vec<[f64; 3]>>> = (0..if full { n } else { n + 1 })
                .map(|j| {
                    let a = a0 + sweep * j as f64 / n as f64;
                    rings
                        .iter()
                        .map(|r| r.iter().map(|p| rotate_about(*p, o, d, a)).collect())
                        .collect()
                })
                .collect();
            let solid = if full {
                loft(&stations, true, None)
            } else {
                let pn = plane_normal(sk, planes).ok_or("plane")?;
                let cap0 = cap_triangles(&stations[0], pn).ok_or("cap")?;
                loft(&stations, false, Some((&cap0, &cap0)))
            }
            .ok_or("loft")?;
            // Box from the profile's exact extremes swept finely.
            let fine = 720usize;
            let bbox = bounds(
                (0..=fine)
                    .flat_map(|j| {
                        let a = a0 + sweep * j as f64 / fine as f64;
                        extremes.iter().map(move |p| rotate_about(*p, o, d, a))
                    })
                    .chain(solid.vertices.iter().copied()),
            )
            .ok_or("bbox")?;
            Ok((solid, bbox))
        }
        "sweep" => {
            let pathp: Vec<Piece> = if let Some(id) = f.sketch_ids.get("m_path3dId") {
                ctx.paths3d
                    .get(id)
                    .ok_or("path_3d")?
                    .iter()
                    .map(|c| {
                        let (p, closed, _) = polyline(c);
                        (p, closed)
                    })
                    .collect()
            } else if f.path3d {
                return Err("path_3d");
            } else {
                sketch_pieces(sketch("m_pathSketchId")?, curves)
                    .ok_or("path_curves")?
                    .0
            };
            let (path, closed) = chain(&pathp).ok_or("path_chain")?;
            let (rings, pn) = match sketch("m_profileSketchId") {
                Ok(prof) => {
                    if ["m_xOffset", "m_yOffset", "m_angle"]
                        .iter()
                        .any(|k| f.doubles.get(*k).is_some_and(|v| v.abs() > 1e-9))
                    {
                        return Err("profile_offset");
                    }
                    let (ppieces, _) = sketch_pieces(prof, curves).ok_or("profile_curves")?;
                    let rings = loops(&ppieces).ok_or("profile_loops")?;
                    (rings, plane_normal(prof, planes).ok_or("plane")?)
                }
                Err(_) => {
                    let sym = f
                        .sketch_ids
                        .get("m_profileFamSymId")
                        .ok_or("profile_family")?;
                    let pcurves = ctx.profile_curves.get(sym).ok_or("profile_family_curves")?;
                    // The profile plane: its attachment point, else the
                    // path's first segment's middle.
                    let q = f
                        .sketch_ids
                        .get("m_profileRefPlaneId")
                        .and_then(|id| ctx.profile_planes.get(id))
                        .copied()
                        .unwrap_or_else(|| {
                            std::array::from_fn(|k| (path[0][k] + path[1][k]) / 2.0)
                        });
                    // The tangent of the path segment nearest q.
                    let n = if closed { path.len() } else { path.len() - 1 };
                    let near = (0..n)
                        .min_by(|&a, &b| {
                            let d = |i: usize| {
                                let (p0, p1) = (path[i], path[(i + 1) % path.len()]);
                                let e = sub(p1, p0);
                                let t = (dot(sub(q, p0), e) / dot(e, e).max(1e-18)).clamp(0.0, 1.0);
                                dist(q, std::array::from_fn(|k| p0[k] + e[k] * t))
                            };
                            d(a).total_cmp(&d(b))
                        })
                        .ok_or("path")?;
                    let t = norm(sub(path[(near + 1) % path.len()], path[near]));
                    let rings = place_profile(pcurves, q, t, &f.doubles).ok_or("profile_loops")?;
                    (rings, t)
                }
            };
            if rings.is_empty() {
                return Err("profile_loops");
            }
            let segs = if closed { path.len() } else { path.len() - 1 };
            if segs == 0 {
                return Err("path");
            }
            let tan = |i: usize| norm(sub(path[(i + 1) % path.len()], path[i % path.len()]));
            // The segment the profile's plane crosses.
            let o = rings[0][0];
            let side = |p: [f64; 3]| dot(sub(p, o), pn);
            let tol = 1e-6;
            let s = (0..segs)
                .find(|&i| {
                    let (a, b) = (side(path[i]), side(path[(i + 1) % path.len()]));
                    (a <= tol && b >= -tol) || (a >= -tol && b <= tol)
                })
                .ok_or("profile_plane")?;
            // Push the rings along direction `d` onto the plane through `at`
            // with normal `m`.
            let push = |rs: &[Vec<[f64; 3]>], d: [f64; 3], at: [f64; 3], m: [f64; 3]| {
                let dm = dot(d, m);
                if dm.abs() < 1e-9 {
                    return None;
                }
                Some(
                    rs.iter()
                        .map(|r| {
                            r.iter()
                                .map(|p| {
                                    let t = dot(sub(at, *p), m) / dm;
                                    std::array::from_fn(|k| p[k] + d[k] * t)
                                })
                                .collect::<Vec<[f64; 3]>>()
                        })
                        .collect::<Vec<_>>(),
                )
            };
            let miter = |v: usize| -> [f64; 3] {
                let (tin, tout) = (tan((v + path.len() - 1) % path.len()), tan(v % path.len()));
                norm([tin[0] + tout[0], tin[1] + tout[1], tin[2] + tout[2]])
            };
            // Forward from segment s to the path's end, backward to its start.
            let mut forward = Vec::new();
            let mut cur = rings.clone();
            let mut v = s + 1;
            loop {
                let d = tan(v - 1);
                let at_end = !closed && v == path.len() - 1;
                let m = if at_end { d } else { miter(v % path.len()) };
                cur = push(&cur, d, path[v % path.len()], m).ok_or("miter")?;
                forward.push(cur.clone());
                if at_end || (closed && v % path.len() == s) {
                    break;
                }
                v += 1;
                if v > segs + s + 1 {
                    break;
                }
            }
            let mut backward = Vec::new();
            if !closed {
                let mut cur = rings.clone();
                let mut v = s;
                loop {
                    let d = tan(v);
                    let m = if v == 0 { d } else { miter(v) };
                    cur = push(&cur, d, path[v], m).ok_or("miter")?;
                    backward.push(cur.clone());
                    if v == 0 {
                        break;
                    }
                    v -= 1;
                }
            }
            backward.reverse();
            let stations: Vec<Vec<Vec<[f64; 3]>>> = if closed {
                // Stations at every vertex once, starting after segment s.
                forward.into_iter().take(path.len()).collect()
            } else {
                backward.into_iter().chain(forward).collect()
            };
            if stations.len() < 2 {
                return Err("stations");
            }
            let solid = if closed {
                loft(&stations, true, None)
            } else {
                let t0 = tan(0);
                let t1 = tan(segs - 1);
                let c0 = cap_triangles(&stations[0], t0).ok_or("cap")?;
                let c1 = cap_triangles(stations.last().unwrap(), t1).ok_or("cap")?;
                loft(&stations, false, Some((&c0, &c1)))
            }
            .ok_or("loft")?;
            let bbox = bounds(solid.vertices.iter().copied()).ok_or("bbox")?;
            Ok((solid, bbox))
        }
        "blend" => {
            let mut ids: Vec<(&String, &i64)> = f
                .sketch_ids
                .iter()
                .filter(|(n, _)| !n.contains("path") && !n.contains("Path"))
                .collect();
            ids.sort_by_key(|(n, _)| {
                let l = n.to_ascii_lowercase();
                if l.contains("bottom") || l.contains("base") || l.contains("start") {
                    0
                } else if l.contains("top") || l.contains("end") {
                    2
                } else {
                    1
                }
            });
            let listed = (
                f.sketch_ids.get("m_sketchIds0"),
                f.sketch_ids.get("m_sketchIds1"),
            );
            let (b, t) = match (listed, ids.as_slice()) {
                ((Some(b), Some(t)), _) => (b, t),
                (_, [(_, b), (_, t)]) => (*b, *t),
                _ => return Err("sketch_fields"),
            };
            let bs = sketches.get(b).ok_or("sketch")?;
            let ts = sketches.get(t).ok_or("sketch")?;
            let n = plane_normal(bs, planes).ok_or("plane")?;
            let (bp, bext) = sketch_pieces(bs, curves).ok_or("curves")?;
            let (tp, text) = sketch_pieces(ts, curves).ok_or("curves")?;
            let br = loops(&bp).ok_or("loops")?;
            let tr = loops(&tp).ok_or("loops")?;
            // Start/end offsets: the two double parameters, in id order.
            let mut params: Vec<(i64, f64)> = f
                .doubles
                .iter()
                .filter_map(|(k, v)| k.parse::<i64>().ok().map(|k| (k, *v)))
                .collect();
            params.sort_by_key(|p| std::cmp::Reverse(p.0));
            let (s0, s1) = match params.as_slice() {
                [a, b, ..] => (a.1, b.1),
                [a] => (0.0, a.1),
                [] => return Err("offsets"),
            };
            if br.len() != 1 || tr.len() != 1 {
                return Err("vertex_count");
            }
            let shift = |r: &[[f64; 3]], s: f64| -> Vec<[f64; 3]> {
                r.iter()
                    .map(|p| std::array::from_fn(|k| p[k] + n[k] * s))
                    .collect()
            };
            let bot = shift(&br[0], s0);
            let mut top = shift(&tr[0], s1);
            // Same winding about n, and the start that pairs points closest.
            let wind = |r: &[[f64; 3]]| {
                let c = r
                    .iter()
                    .fold([0.0; 3], |a, p| [a[0] + p[0], a[1] + p[1], a[2] + p[2]]);
                let mut w = 0.0;
                for i in 0..r.len() {
                    w += dot(cross(sub(r[i], c), sub(r[(i + 1) % r.len()], c)), n);
                }
                w
            };
            if (wind(&bot) > 0.0) != (wind(&top) > 0.0) {
                top.reverse();
            }
            let len = bot.len();
            // (Only rings of one vertex count pair point for point.)
            let best = (0..if top.len() == len { len } else { 0 })
                .min_by(|&a, &b| {
                    let cost = |k: usize| -> f64 {
                        (0..len)
                            .map(|i| {
                                let q = top[(i + k) % len];
                                let p = bot[i];
                                let d = sub(q, p);
                                let along = dot(d, n);
                                dot(d, d) - along * along
                            })
                            .sum()
                    };
                    cost(a).total_cmp(&cost(b))
                })
                .unwrap_or(0);
            let cb = cap_triangles(std::slice::from_ref(&bot), n).ok_or("cap")?;
            let mut solid = if top.len() == len {
                top.rotate_left(best);
                let ct = cap_triangles(std::slice::from_ref(&top), n).ok_or("cap")?;
                let stations = vec![vec![bot], vec![top]];
                loft(&stations, false, Some((&cb, &ct))).ok_or("loft")?
            } else {
                // Profiles of different vertex counts (a rectangle to a
                // circle): start the top at its point nearest the bottom's
                // first, in plan, and join the two by arc length.
                let near = (0..top.len())
                    .min_by(|&a, &b| {
                        let off = |k: usize| {
                            let d = sub(top[k], bot[0]);
                            let along = dot(d, n);
                            dot(d, d) - along * along
                        };
                        off(a).total_cmp(&off(b))
                    })
                    .ok_or("loops")?;
                top.rotate_left(near);
                let ct = cap_triangles(std::slice::from_ref(&top), n).ok_or("cap")?;
                blend_unequal(&bot, &top, &cb, &ct).ok_or("loft")?
            };
            solid.ruled_axis = Some(n);
            let bbox = bounds(
                bext.iter()
                    .map(|p| std::array::from_fn(|k| p[k] + n[k] * s0))
                    .chain(
                        text.iter()
                            .map(|p| std::array::from_fn(|k| p[k] + n[k] * s1)),
                    ),
            )
            .ok_or("bbox")?;
            Ok((solid, bbox))
        }
        _ => Err("not_solved"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(z: f64) -> Vec<(Vec<[f64; 3]>, bool)> {
        let c = [[0.0, 0.0, z], [2.0, 0.0, z], [2.0, 1.0, z], [0.0, 1.0, z]];
        (0..4)
            .map(|i| (vec![c[i], c[(i + 1) % 4]], false))
            .collect()
    }

    fn volume(s: &FamilySolid) -> f64 {
        s.triangles
            .iter()
            .map(|t| {
                let (a, b, c) = (
                    s.vertices[t[0] as usize],
                    s.vertices[t[1] as usize],
                    s.vertices[t[2] as usize],
                );
                dot(a, cross(b, c)) / 6.0
            })
            .sum()
    }

    #[test]
    fn a_square_extrudes_to_a_closed_solid_of_the_right_volume() {
        let rings = loops(&square(0.0)).unwrap();
        assert_eq!(rings.len(), 1);
        let s = prism(&rings, [0.0, 0.0, 1.0], 0.0, 3.0).unwrap();
        assert!((volume(&s) - 6.0).abs() < 1e-9, "{}", volume(&s));
        // Reversed start/end is the same solid.
        let r = prism(&rings, [0.0, 0.0, 1.0], 3.0, 0.0).unwrap();
        assert!((volume(&r) - 6.0).abs() < 1e-9);
    }

    #[test]
    fn a_hole_is_subtracted() {
        let mut rings = loops(&square(0.0)).unwrap();
        let hole: Vec<[f64; 3]> = vec![
            [0.5, 0.25, 0.0],
            [1.0, 0.25, 0.0],
            [1.0, 0.75, 0.0],
            [0.5, 0.75, 0.0],
        ];
        rings.push(hole);
        let s = prism(&rings, [0.0, 0.0, 1.0], 0.0, 2.0).unwrap();
        assert!(
            (volume(&s) - 2.0 * (2.0 - 0.25)).abs() < 1e-9,
            "{}",
            volume(&s)
        );
    }

    #[test]
    fn a_vertical_plane_extrudes_along_its_normal() {
        // A square in the x-z plane extruded along +y.
        let c = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
        ];
        let pieces: Vec<_> = (0..4)
            .map(|i| (vec![c[i], c[(i + 1) % 4]], false))
            .collect();
        let rings = loops(&pieces).unwrap();
        let s = prism(&rings, [0.0, 1.0, 0.0], 1.0, 2.0).unwrap();
        let b = bounds(s.vertices.iter().copied()).unwrap();
        assert_eq!(b, [[0.0, 1.0, 0.0], [1.0, 2.0, 1.0]]);
        assert!((volume(&s).abs() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_full_circle_closes_and_its_extremes_are_exact() {
        let arc = Curve::Arc {
            center: [1.0, 2.0, 0.0],
            x: [1.0, 0.0, 0.0],
            y: [0.0, 1.0, 0.0],
            radius: 0.5,
            t: [0.0, 0.0],
        };
        let (pts, closed, ext) = polyline(&arc);
        assert!(closed);
        let b = bounds(ext.into_iter()).unwrap();
        assert!((b[0][0] - 0.5).abs() < 1e-12 && (b[1][0] - 1.5).abs() < 1e-12);
        assert!((b[0][1] - 1.5).abs() < 1e-12 && (b[1][1] - 2.5).abs() < 1e-12);
        let rings = loops(&[(pts, closed)]).unwrap();
        let s = prism(&rings, [0.0, 0.0, 1.0], 0.0, 1.0).unwrap();
        // 32-gon inscribed in the circle.
        assert!(volume(&s) > 0.77 && volume(&s) < std::f64::consts::PI * 0.25 + 1e-9);
    }

    #[test]
    fn an_open_chain_has_no_loop() {
        let mut p = square(0.0);
        p.pop();
        assert!(loops(&p).is_none());
    }

    #[test]
    fn pieces_given_in_either_direction_join() {
        let mut p = square(0.0);
        p[1].0.reverse();
        let rings = loops(&p).unwrap();
        assert_eq!(rings.len(), 1);
        assert_eq!(rings[0].len(), 4);
    }
}

#[cfg(test)]
mod probe_forms {
    use super::*;

    /// `RVT_PROBE_FILE=<rvt> cargo test --lib probe_forms -- --ignored --nocapture`
    /// Dumps the GenSweepGStep and ExtrusionGStep objects of the first two
    /// solid forms of each kind (where a form keeps its solid's B-rep).
    #[test]
    #[ignore]
    fn probe_forms() {
        let Ok(path) = std::env::var("RVT_PROBE_FILE") else {
            return;
        };
        let mut rf = RevitFile::open(&path).unwrap();
        let layouts = rf.schema_layouts().unwrap();
        let names: BTreeMap<u16, String> = layouts
            .classes
            .iter()
            .map(|(t, c)| (*t, c.name.clone()))
            .collect();
        let want = ["SweepElem", "RevolutionElem", "BlendElem", "ExtrusionElem"];
        let tags: BTreeSet<u16> = names
            .iter()
            .filter(|(_, n)| want.contains(&n.as_str()))
            .map(|(t, _)| *t)
            .collect();
        let mut shown: BTreeMap<String, usize> = BTreeMap::new();
        for_each_object_where(
            &mut rf,
            |frame, _| tags.contains(&frame.header.tag),
            |x| {
                let Some(class) = names.get(&x.frame.header.tag) else {
                    return;
                };
                let n = shown.entry(class.clone()).or_default();
                if *n >= 1 {
                    return;
                }
                *n += 1;
                let cls: Vec<&str> = x
                    .objects
                    .iter()
                    .map(|o| names.get(&o.tag).map(String::as_str).unwrap_or("?"))
                    .collect();
                println!("== {class} id {} objects {cls:?}", x.frame.header.id);
                for (oi, o) in x.objects.iter().enumerate() {
                    let c = cls[oi];
                    if !(c.ends_with("GStep") || c == "VtxConnections" || oi == 0) {
                        continue;
                    }
                    for (k, v) in o.values.iter() {
                        let s = format!("{v:?}");
                        if oi == 0 && !k.contains("Sketch") && !k.contains("path") {
                            continue;
                        }
                        println!("  [{oi}:{c}] {k} = {}", &s[..s.len().min(400)]);
                    }
                }
            },
        )
        .unwrap();
    }

    fn ctx_of(
        sketches: BTreeMap<i64, RawSketch>,
        planes: BTreeMap<i64, RawPlane>,
        curves: BTreeMap<i64, Curve>,
    ) -> SolveCtx {
        SolveCtx {
            sketches,
            planes,
            curves,
            ..Default::default()
        }
    }

    fn solid_volume(s: &FamilySolid) -> f64 {
        s.triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| s.vertices[i as usize]);
                dot(a, cross(b, c)) / 6.0
            })
            .sum()
    }

    fn line(a: [f64; 3], b: [f64; 3]) -> Curve {
        Curve::Line {
            origin: a,
            dir: sub(b, a),
            t: [0.0, 1.0],
        }
    }

    /// Sketch `sid` on plane `pid` (normal `n`) holding `lines`; curve ids
    /// start at `first`.
    #[allow(clippy::too_many_arguments)]
    fn sketch_of(
        sid: i64,
        pid: i64,
        n: [f64; 3],
        lines: &[([f64; 3], [f64; 3])],
        first: i64,
        sketches: &mut BTreeMap<i64, RawSketch>,
        planes: &mut BTreeMap<i64, RawPlane>,
        curves: &mut BTreeMap<i64, Curve>,
    ) {
        let ids: Vec<i64> = (0..lines.len() as i64).map(|i| first + i).collect();
        for (id, (a, b)) in ids.iter().zip(lines) {
            curves.insert(*id, line(*a, *b));
        }
        sketches.insert(
            sid,
            RawSketch {
                plane: pid,
                curves: ids,
            },
        );
        planes.insert(
            pid,
            RawPlane {
                m: [[0.0, 0.0, n[0]], [0.0, 0.0, n[1]], [0.0, 0.0, n[2]]],
                flip_z: false,
            },
        );
    }

    fn ring(points: &[[f64; 3]]) -> Vec<([f64; 3], [f64; 3])> {
        (0..points.len())
            .map(|i| (points[i], points[(i + 1) % points.len()]))
            .collect()
    }

    fn form(kind: &'static str, ids: &[(&str, i64)], doubles: &[(&str, f64)]) -> RawForm {
        RawForm {
            guid: [0; 16],
            id: 1,
            kind,
            material: -1,
            cutting: false,
            sketch_ids: ids.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
            doubles: doubles.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
            path3d: false,
        }
    }

    #[test]
    fn a_rectangle_revolves_into_a_closed_ring_of_the_right_volume() {
        let (mut sk, mut pl, mut cv) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
        let mut lines = ring(&[
            [1.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [2.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
        ]);
        lines.push(([0.0, 0.0, 0.0], [0.0, 0.0, 1.0])); // the axis
        sketch_of(
            10,
            20,
            [0.0, 1.0, 0.0],
            &lines,
            100,
            &mut sk,
            &mut pl,
            &mut cv,
        );
        let f = form("revolution", &[("m_sketchId", 10)], &[]);
        let (solid, bbox) = solve_form(&f, &ctx_of(sk.clone(), pl.clone(), cv.clone())).unwrap();
        let v = solid_volume(&solid);
        let exact = 3.0 * std::f64::consts::PI;
        assert!((v - exact).abs() < 0.01 * exact, "volume {v}");
        assert!((bbox[1][0] - 2.0).abs() < 1e-4 && (bbox[0][1] + 2.0).abs() < 1e-4);
        // A half turn is capped and has half the volume.
        let half = form(
            "revolution",
            &[("m_sketchId", 10)],
            &[(REVOLUTION_END, std::f64::consts::PI)],
        );
        let (solid, _) = solve_form(&half, &ctx_of(sk, pl, cv)).unwrap();
        assert!(
            (solid_volume(&solid) - exact / 2.0).abs() < 0.01 * exact,
            "{}",
            solid_volume(&solid)
        );
    }

    #[test]
    fn a_square_sweeps_along_a_bent_path_with_mitred_corners() {
        let (mut sk, mut pl, mut cv) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
        // Profile: a unit square in the plane x = 1, centred on the path.
        let profile = ring(&[
            [1.0, -0.5, -0.5],
            [1.0, 0.5, -0.5],
            [1.0, 0.5, 0.5],
            [1.0, -0.5, 0.5],
        ]);
        sketch_of(
            10,
            20,
            [1.0, 0.0, 0.0],
            &profile,
            100,
            &mut sk,
            &mut pl,
            &mut cv,
        );
        let path = [
            ([0.0, 0.0, 0.0], [2.0, 0.0, 0.0]),
            ([2.0, 0.0, 0.0], [2.0, 2.0, 0.0]),
        ];
        sketch_of(
            11,
            21,
            [0.0, 0.0, 1.0],
            &path,
            200,
            &mut sk,
            &mut pl,
            &mut cv,
        );
        let f = form(
            "sweep",
            &[("m_profileSketchId", 10), ("m_pathSketchId", 11)],
            &[("m_xOffset", 0.0)],
        );
        let (solid, bbox) = solve_form(&f, &ctx_of(sk.clone(), pl.clone(), cv.clone())).unwrap();
        assert!(
            (solid_volume(&solid) - 4.0).abs() < 1e-9,
            "volume {}",
            solid_volume(&solid)
        );
        assert!((bbox[1][0] - 2.5).abs() < 1e-9 && (bbox[0][0]).abs() < 1e-9);
        assert!((bbox[1][1] - 2.0).abs() < 1e-9);
    }

    #[test]
    fn a_blend_lofts_a_square_into_a_smaller_one() {
        let (mut sk, mut pl, mut cv) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
        let bottom = ring(&[
            [0.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [2.0, 2.0, 0.0],
            [0.0, 2.0, 0.0],
        ]);
        let top = ring(&[
            [0.5, 0.5, 0.0],
            [1.5, 0.5, 0.0],
            [1.5, 1.5, 0.0],
            [0.5, 1.5, 0.0],
        ]);
        sketch_of(
            10,
            20,
            [0.0, 0.0, 1.0],
            &bottom,
            100,
            &mut sk,
            &mut pl,
            &mut cv,
        );
        sketch_of(
            11,
            20,
            [0.0, 0.0, 1.0],
            &top,
            200,
            &mut sk,
            &mut pl,
            &mut cv,
        );
        let f = form(
            "blend",
            &[("m_bottomSketchId", 10), ("m_topSketchId", 11)],
            &[("-1001804", 0.0), ("-1001805", 3.0)],
        );
        let (solid, bbox) = solve_form(&f, &ctx_of(sk.clone(), pl.clone(), cv.clone())).unwrap();
        // Frustum of a square pyramid: h/3 (A1 + A2 + sqrt(A1 A2)) = 1 * (4 + 1 + 2).
        assert!(
            (solid_volume(&solid) - 7.0).abs() < 1e-9,
            "volume {}",
            solid_volume(&solid)
        );
        assert!((bbox[1][2] - 3.0).abs() < 1e-9);
    }
}
