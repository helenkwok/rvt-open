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
//! Files saved before Revit 2020 have no keyed sections. A standalone family
//! of those releases keeps its nested families' documents among its own
//! objects, and an element's `m_famId` names the family whose document holds
//! it: those objects are read as that family's document (keyed by
//! `owned_document`) rather than as the file's own forms.
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
/// Families of Revit 2009 and 2011 store a form's bounding box from its
/// faceted display, which cuts each arc short by up to this share of its
/// radius (1 - cos(pi/14), arcs in 14 facets a turn: every extrusion of the
/// families measured whose box missed by more than [`BBOX_TOLERANCE`] and
/// lay inside the decoded one missed by 0.0250 to 0.0251 of its largest
/// radius, or less where that radius is feet long).
pub const ARC_FACET_SAG: f64 = 0.0251;
/// A revolution's stored box may lie inside its solved one by this share of
/// its radius, at most half the solved box's largest side (1 - cos(pi/6): a
/// turn shown in six facets).
pub const REVOLUTION_FACET_SAG: f64 = 0.134;
/// Last release whose stored boxes are read as faceted (2012 and later were
/// not measured, and keep the exact match).
const LAST_FACETED_BOX_RELEASE: u32 = 2011;
/// A solved sweep or revolution is tessellated (arcs in steps of pi/16), so
/// its box may fall short of the stored one by the chord error: this share
/// of the form's size.
pub const SOLVED_BBOX_FRACTION: f64 = 0.01;

/// One decoded solid form, in the family document's coordinates (feet).
#[derive(Debug, Clone, Default)]
pub struct FamilySolid {
    /// Of a revolution, its axis's direction.
    pub revolution_axis: Option<[f64; 3]>,
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
    /// The stored bounding boxes of the solid forms that were not decoded
    /// (a caller may draw them as boxes, as it draws its own).
    pub unsolved_boxes: Vec<[[f64; 3]; 2]>,
    /// Why each solid form that was not decoded was not, by form id
    /// (`geometry`, `bbox_mismatch`, or the solver's reason).
    pub form_failures: BTreeMap<i64, &'static str>,
    /// Of a family file's own document, its template's sample host wall (the
    /// wall a door or window family is shown in), with the openings the
    /// family cuts in it: Revit shows it in the family, not where the family
    /// is placed.
    pub sample_walls: Vec<FamilySolid>,
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
    /// Solids a void form was cut from.
    pub void_cuts: usize,
    /// Forms and instances left out because they belong to a group no
    /// instance places (`m_unplacedOwnerId` set): Revit draws none of them.
    pub unplaced: usize,
    /// Openings of a sample host wall that could not be cut (the wall is
    /// drawn with the others).
    pub wall_openings_uncut: usize,
    /// Solids a void meets whose cut gave up (drawn uncut).
    pub void_cuts_failed: usize,
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
    /// Of a family file without keyed sections, the solid forms and family
    /// instances its nested families' documents hold (not the file's own).
    pub nested_objects: BTreeSet<i64>,
    /// Of such a file, the sample hosts of hosted families' templates (the
    /// wall of a door, the block of a face-based family), by document: the
    /// elements each `Family` names in `m_refs`.
    pub host_samples: BTreeSet<(Guid, i64)>,
    /// Of such a file, the family instances each nested family's document
    /// places, by document and instance id (in that document's coordinates).
    pub doc_instances: BTreeMap<Guid, BTreeMap<i64, InstancePose>>,
    /// Of a file with keyed sections, each document's own family symbols and
    /// families (element ids are the document's own, so they are keyed by
    /// document): symbol -> family, and family -> its content document.
    pub doc_symbol_family: BTreeMap<(Guid, i64), i64>,
    pub doc_family_guid: BTreeMap<(Guid, i64), Guid>,
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

    /// The decoded document of the family that symbol `symbol` of document
    /// `doc` belongs to: the document's own symbols in a file with keyed
    /// sections, the file's (one id space) otherwise.
    pub fn document_of_symbol_in(&self, doc: Guid, symbol: i64) -> Option<(Guid, &DocSolids)> {
        if let Some(family) = self.doc_symbol_family.get(&(doc, symbol)) {
            let guid = self.doc_family_guid.get(&(doc, *family))?;
            return Some((*guid, self.docs.get(guid)?));
        }
        self.document_of_symbol(symbol)
    }

    /// The decoded document of the family that symbol `symbol` belongs to.
    pub fn document_of_symbol(&self, symbol: i64) -> Option<(Guid, &DocSolids)> {
        let family = self.symbol_family.get(&symbol)?;
        let guid = self.family_guid.get(family)?;
        Some((*guid, self.docs.get(guid)?))
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
    /// Its class tag and the number of the partition it was read from.
    tag: u16,
    part: u32,
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
    tag: u16,
    part: u32,
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

/// [`read`], kept on `rf`: the site content, family bodies and symbol mesh
/// passes of an export each read every partition's family objects again.
pub(crate) fn read_memoised(rf: &mut RevitFile) -> Result<std::sync::Arc<FamilyDocuments>> {
    if let Some(docs) = rf.cached_family_documents() {
        return Ok(docs);
    }
    let docs = std::sync::Arc::new(read(rf)?);
    rf.cache_family_documents(std::sync::Arc::clone(&docs));
    Ok(docs)
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
    const WANT: [&str; 18] = [
        "Opening",
        "SWall",
        "FreeFormElement",
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
    let faceted_boxes = rf
        .basic_file_info()
        .is_ok_and(|b| b.version <= LAST_FACETED_BOX_RELEASE);
    let mut extrusions: Vec<RawExtrusion> = Vec::new();
    let mut forms: Vec<RawForm> = Vec::new();
    // curves per document, 3D path curves and profile-plane
    // attachment points, by element id.
    let mut doc_curves: BTreeMap<Guid, BTreeMap<i64, Curve>> = BTreeMap::new();
    let mut paths3d: BTreeMap<i64, Vec<Curve>> = BTreeMap::new();
    let mut profile_planes: BTreeMap<i64, [f64; 3]> = BTreeMap::new();
    let mut sketches: BTreeMap<i64, RawSketch> = BTreeMap::new();
    let mut planes: BTreeMap<i64, RawPlane> = BTreeMap::new();
    let mut curves: BTreeMap<i64, Curve> = BTreeMap::new();
    let mut boxes: BTreeMap<i64, [[f64; 3]; 2]> = BTreeMap::new();
    // Undecoded forms per document: (kind -> count).
    // (document, kind, class tag, id, partition), kept for the newest copy.
    let mut other: Vec<(Guid, &'static str, u16, i64, u32)> = Vec::new();
    // The partition each object's newest copy was read from, by document,
    // class and id: a file saved several times keeps older copies of some
    // records in earlier partitions (one Revit 2011 family measured holds 27
    // of its 51 extrusions twice), and the newest is the current one.
    let mut newest: BTreeMap<(Option<Guid>, u16, i64), u32> = BTreeMap::new();
    // A family file without keyed sections (every release before 2020)
    // keeps its nested families' documents among its own objects; each
    // element's `m_famId` names the family whose document holds it.
    // Of such a file, each `Family` element's owner (`m_famId`).
    let mut family_owner: BTreeMap<i64, i64> = BTreeMap::new();
    // Each form's geometry combinations (Revit's joins and cuts), by document
    // and id: a void cuts the solids it shares a combination with.
    let mut combinations: BTreeMap<(Guid, i64), Vec<i64>> = BTreeMap::new();
    // Walls, and the openings (host wall, sketch) each document cuts.
    let mut walls: BTreeSet<(Guid, i64)> = BTreeSet::new();
    // Keyed by document and opening: a newer copy replaces an older one.
    let mut openings: BTreeMap<(Guid, i64), Option<(i64, i64)>> = BTreeMap::new();
    let host_family = if host_is_family && section_count == 0 {
        host_family_id(rf, &names)?
    } else {
        None
    };

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
            let owner = host_family.and_then(|host| {
                id_of(root, "m_famId")
                    .filter(|f| *f > 0 && *f != host)
                    .map(owned_document)
            });
            let bookkeeping =
                matches!(class.as_str(), "Family" | "FamilySymbol" | "FamilyInstance");
            if let Some(g) = owner {
                if class == "FamilyInstance" || FORM_CLASS_NAMES.contains(&class.as_str()) {
                    out.nested_objects.insert(x.frame.header.id as i64);
                }
                guid = Some(g);
            } else if guid.is_none() && host_is_family && !bookkeeping {
                guid = Some(HOST_DOCUMENT);
            }
            let id = x.frame.header.id as i64;
            let part = crate::partition_room_parameters::partition_number(&x.stream);
            match newest.get(&(guid, x.frame.header.tag, id)) {
                Some(p) if *p > part => return,
                _ => {
                    newest.insert((guid, x.frame.header.tag, id), part);
                }
            }
            // A member of a group no instance places (a group type's own
            // copy, `m_unplacedOwnerId` naming it): Revit draws none (one door
            // family measured keeps 32 handle extrusions in seven such groups,
            // one of them 7 ft from the door on the floor).
            if (class == "FamilyInstance" || FORM_CLASS_NAMES.contains(&class.as_str()))
                && id_of(root, "m_unplacedOwnerId").is_some_and(|o| o > 0)
            {
                out.stats.unplaced += 1;
                // An older, placed copy read first leaves no pose behind.
                if class == "FamilyInstance" {
                    match guid {
                        None => {
                            out.instances.remove(&id);
                        }
                        Some(g) => {
                            if let Some(m) = out.doc_instances.get_mut(&g) {
                                m.remove(&id);
                            }
                        }
                    }
                }
                return;
            }
            // A hosted family's template keeps a sample host (a wall, or a
            // block for a face-based family) that its `Family` names in
            // `m_refs`: it is not part of the family's form.
            if let (Some(_), "Family") = (host_family, class.as_str()) {
                family_owner.insert(id, id_of(root, "m_famId").unwrap_or(-1));
            }
            if let (Some(host), "Family") = (host_family, class.as_str()) {
                let doc = if id == host {
                    HOST_DOCUMENT
                } else {
                    owned_document(id)
                };
                for o in &x.objects {
                    if names.get(&o.tag).map(String::as_str) == Some("FamilyElemRef") {
                        if let Some(e) = id_of(o, "m_elemId") {
                            out.host_samples.insert((doc, e));
                        }
                    }
                }
            }
            if FORM_CLASS_NAMES.contains(&class.as_str()) {
                if let Some(g) = guid {
                    let groups: Vec<i64> = x
                        .objects
                        .iter()
                        .filter(|o| {
                            names.get(&o.tag).map(String::as_str)
                                == Some("FamilyGeomCombinationMembership")
                        })
                        .filter_map(|o| id_of(o, "m_groupId"))
                        .collect();
                    combinations.insert((g, id), groups);
                }
            }
            match (class.as_str(), guid) {
                ("Family", None) if host_family.is_some() => {
                    out.family_guid.insert(id, owned_document(id));
                }
                ("Family", None) => {
                    if let Some(g) = guid_of_family_document(x, &names) {
                        out.family_guid.insert(id, g);
                    } else if host_is_family {
                        // A keyed family file's own family (it names no
                        // content document): its template's sample hosts.
                        for o in &x.objects {
                            if names.get(&o.tag).map(String::as_str) == Some("FamilyElemRef") {
                                if let Some(e) = id_of(o, "m_elemId") {
                                    out.host_samples.insert((HOST_DOCUMENT, e));
                                }
                            }
                        }
                    }
                }
                ("FamilySymbol", None) => {
                    if let Some(f) = id_of(root, "m_familyId") {
                        out.symbol_family.insert(id, f);
                    }
                }
                ("FamilyInstance", None) => {
                    if let Some(pose) = pose_of(x, &names) {
                        out.instances.insert(id, pose);
                    }
                }
                ("FamilyInstance", Some(g)) => {
                    other.push((g, "nested_family", x.frame.header.tag, id, part));
                    // A nested family placing its own nested families: kept
                    // to place them in turn.
                    if let Some(pose) = pose_of(x, &names) {
                        out.doc_instances.entry(g).or_default().insert(id, pose);
                    }
                }
                ("Family", Some(_)) if host_family.is_some() => {
                    out.family_guid.insert(id, owned_document(id));
                }
                ("FamilySymbol", Some(_)) if host_family.is_some() => {
                    if let Some(f) = id_of(root, "m_familyId") {
                        out.symbol_family.insert(id, f);
                    }
                }
                // A keyed-section document's own nested families.
                ("Family", Some(g)) => {
                    if let Some(doc) = guid_of_family_document(x, &names) {
                        out.doc_family_guid.insert((g, id), doc);
                    }
                }
                ("FamilySymbol", Some(g)) => {
                    if let Some(f) = id_of(root, "m_familyId") {
                        out.doc_symbol_family.insert((g, id), f);
                    }
                }
                // A sectionless family's hidden forms are not drawn, and a
                // free form is not decoded.
                (c, Some(_))
                    if host_family.is_some()
                        && FORM_CLASS_NAMES.contains(&c)
                        && is_hidden(x, &names) => {}
                ("FreeFormElement", Some(g)) if host_family.is_some() => {
                    other.push((g, "free_form", x.frame.header.tag, id, part));
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
                            tag: x.frame.header.tag,
                            part,
                            material: id_of(root, "m_materialId").unwrap_or(-1),
                            cutting: bool_of(root, "m_cutting"),
                            sketch,
                            start: s,
                            end: e,
                        }),
                        _ => other.push((g, "extrusion_fields", x.frame.header.tag, id, part)),
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
                        tag: x.frame.header.tag,
                        part,
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
                        doc_curves.entry(g).or_default().insert(id, c.clone());
                        curves.insert(id, c);
                    } else {
                        // A newer copy that does not decode leaves no older
                        // copy's curve behind.
                        curves.remove(&id);
                        if let Some(m) = doc_curves.get_mut(&g) {
                            m.remove(&id);
                        }
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
                ("SWall", Some(g)) => {
                    walls.insert((g, id));
                }
                ("Opening", Some(g)) => {
                    let refs = id_of(root, "m_hostId").zip(id_of(root, "m_sketchId"));
                    openings.insert((g, id), refs);
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

    // Of a form read from several partitions, the newest copy; and no
    // template's sample host.
    extrusions.retain(|e| {
        newest.get(&(Some(e.guid), e.tag, e.id)) == Some(&e.part)
            && !out.host_samples.contains(&(e.guid, e.id))
    });
    forms.retain(|f| {
        newest.get(&(Some(f.guid), f.tag, f.id)) == Some(&f.part)
            && !out.host_samples.contains(&(f.guid, f.id))
    });

    for (g, kind, tag, id, part) in other {
        if newest.get(&(Some(g), tag, id)) == Some(&part) && !out.host_samples.contains(&(g, id)) {
            if kind == "extrusion_fields" {
                out.docs
                    .entry(g)
                    .or_default()
                    .form_failures
                    .insert(id, "fields");
            }
            *out.docs
                .entry(g)
                .or_default()
                .undecoded
                .entry(kind)
                .or_default() += 1;
        }
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
    let mut voids: Vec<(Guid, i64, Option<FamilySolid>)> = Vec::new();
    for e in &extrusions {
        let doc = out.docs.entry(e.guid).or_default();
        if e.cutting {
            voids.push((
                e.guid,
                e.id,
                extrusion_solid(e, &sketches, &planes, &curves).map(|(s, _)| s),
            ));
            continue;
        }
        let solid = extrusion_solid(e, &sketches, &planes, &curves);
        let Some((solid, bbox)) = solid else {
            *doc.undecoded.entry("extrusion_geometry").or_default() += 1;
            doc.form_failures.insert(e.id, "geometry");
            doc.unsolved_boxes.extend(boxes.get(&e.id));
            continue;
        };
        out.stats.extrusions_decoded += 1;
        // Before 2012 the stored box may lie inside the decoded one by the
        // arcs' facet sag, never outside it.
        let sag = if faceted_boxes {
            ARC_FACET_SAG * max_arc_radius(e, &sketches, &curves)
        } else {
            0.0
        };
        let matches = boxes
            .get(&e.id)
            .is_none_or(|stored| box_matches(stored, &bbox, sag));
        if !matches {
            *doc.undecoded.entry("extrusion_bbox_mismatch").or_default() += 1;
            doc.form_failures.insert(e.id, "bbox_mismatch");
            doc.unsolved_boxes.extend(boxes.get(&e.id));
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
                profile_curves.insert(*sym, c.values().cloned().collect());
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
            voids.push((f.guid, f.id, solve_form(f, &ctx).ok().map(|(s, _)| s)));
            continue;
        }
        match solve_form(f, &ctx) {
            Err(why) => {
                *doc.undecoded.entry(f.kind).or_default() += 1;
                doc.form_failures.insert(f.id, why);
                doc.unsolved_boxes.extend(boxes.get(&f.id));
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
                        // A revolution's stored box is that of its display,
                        // which turns a small one in as few as six facets: it
                        // lies inside by up to that sag of the radius (four
                        // full revolutions of a Revit 2012 family measured:
                        // stored boxes 0.0711 by 0.0616 ft, a hexagon across
                        // its corners and across its flats).
                        || solid
                            .revolution_axis
                            .is_some_and(|a| revolution_box_matches(stored, &bbox, a))
                        // A faceted stored box lies inside by the facet sag
                        // of the form's arcs, the revolution's own among them
                        // (its radius is at most the form's size).
                        || (faceted_boxes
                            && box_matches(
                                stored,
                                &bbox,
                                ARC_FACET_SAG * size.max(form_arc_radius(f, &ctx)),
                            ))
                });
                if !matches {
                    *doc.undecoded.entry(f.kind).or_default() += 1;
                    doc.form_failures.insert(f.id, "bbox_mismatch");
                    doc.unsolved_boxes.extend(boxes.get(&f.id));
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
    // A void cuts the solids it shares a geometry combination with (Revit's
    // cut relation: on four families measured, each void and the solids it
    // cuts are the members of one combination); a void in no combination,
    // the solids whose box its box meets. A void that meets none cuts nothing
    // of the family's own form (a void that cuts the host it is placed in,
    // such as an opening, lies outside them). A void that does not solve, or
    // whose cut fails, leaves the solids it meets uncut ("void"); one that
    // cut them is "void_cut", so the document still reads as not complete
    // and a project draws it from Revit's saved graphics where it can.
    let overlaps = |a: &Bounds, b: &Bounds| (0..3).all(|k| a[0][k] < b[1][k] && b[0][k] < a[1][k]);
    for (g, void_id, void) in voids {
        let void_groups = combinations.get(&(g, void_id)).cloned().unwrap_or_default();
        let void_box = void
            .as_ref()
            .and_then(|v| bounds(v.vertices.iter().copied()));
        let doc = out.docs.entry(g).or_default();
        let targets: Vec<usize> = (0..doc.solids.len())
            .filter(|i| {
                let s = &doc.solids[*i];
                let solid_box = bounds(s.vertices.iter().copied());
                let joined = combinations
                    .get(&(g, s.form_id))
                    .is_some_and(|gs| gs.iter().any(|x| void_groups.contains(x)));
                let meets = match (&void_box, &solid_box) {
                    (Some(v), Some(b)) => overlaps(v, b),
                    _ => true,
                };
                // A solid an earlier void removed whole is not cut again.
                !s.triangles.is_empty() && meets && (joined || void_groups.is_empty())
            })
            .collect();
        if targets.is_empty() {
            out.stats.voids_outside_solids += 1;
            continue;
        }
        let Some(void) = void else {
            *doc.undecoded.entry("void").or_default() += 1;
            continue;
        };
        let mut all_cut = true;
        for i in targets {
            let s = &mut doc.solids[i];
            match crate::mesh_csg::difference(
                (&s.vertices, &s.triangles),
                (&void.vertices, &void.triangles),
            ) {
                Some((v, t)) => {
                    // Counted only where material went (a cutter that meets
                    // the solid's box may miss the solid).
                    let before = mesh_volume(&s.vertices, &s.triangles);
                    let after = mesh_volume(&v, &t);
                    if after < before - 1e-9 * before.abs().max(1.0) {
                        out.stats.void_cuts += 1;
                    }
                    s.vertices = v;
                    s.triangles = t;
                }
                None => {
                    all_cut = false;
                    out.stats.void_cuts_failed += 1;
                }
            }
        }
        *doc.undecoded
            .entry(if all_cut { "void_cut" } else { "void" })
            .or_default() += 1;
    }
    // The family's own sample host wall: its stored box (a template keeps it
    // straight along an axis), less the openings the family cuts in it, each
    // its sketch's loops pushed through the wall.
    for (g, wall) in &walls {
        if *g != HOST_DOCUMENT || !out.host_samples.contains(&(*g, *wall)) {
            continue;
        }
        let Some(b) = boxes.get(wall) else { continue };
        let ring = vec![
            [b[0][0], b[0][1], 0.0],
            [b[1][0], b[0][1], 0.0],
            [b[1][0], b[1][1], 0.0],
            [b[0][0], b[1][1], 0.0],
        ];
        let Some(mut solid) = prism(&[ring], [0.0, 0.0, 1.0], b[0][2], b[1][2]) else {
            continue;
        };
        let reach = dist(b[0], b[1]) + 1.0;
        let cut_by: Vec<i64> = openings
            .iter()
            .filter_map(|((og, _), refs)| match refs {
                Some((h, k)) if og == g && h == wall => Some(*k),
                _ => None,
            })
            .collect();
        for sketch in &cut_by {
            let cutter = ctx.sketches.get(sketch).and_then(|sk| {
                let n = plane_normal(sk, &ctx.planes)?;
                let (pieces, _) = sketch_pieces(sk, &ctx.curves)?;
                prism(&loops(&pieces)?, n, -reach, reach)
            });
            match cutter.and_then(|c| {
                crate::mesh_csg::difference(
                    (&solid.vertices, &solid.triangles),
                    (&c.vertices, &c.triangles),
                )
            }) {
                Some((v, t)) => {
                    solid.vertices = v;
                    solid.triangles = t;
                }
                None => out.stats.wall_openings_uncut += 1,
            }
        }
        solid.form_id = *wall;
        out.docs.entry(*g).or_default().sample_walls.push(solid);
    }
    if let Some(host) = host_family {
        adopt_orphan_document(&mut out, &family_owner, host);
    }
    Ok(out)
}

/// A loaded family whose own element holds no document, while the file
/// keeps exactly one other document with forms that no family names: in one
/// Revit 2015 family measured, the loaded family's element (held by the
/// host) and its document's own root element (`m_famId` -1, no document
/// GUID) are two elements, and nothing in the records links them. Only the
/// one unambiguous pairing is made.
fn adopt_orphan_document(out: &mut FamilyDocuments, family_owner: &BTreeMap<i64, i64>, host: i64) {
    let has_forms = |g: &Guid| {
        out.docs
            .get(g)
            .is_some_and(|d| !d.solids.is_empty() || !d.unsolved_boxes.is_empty())
    };
    let used: BTreeSet<i64> = out
        .instances
        .values()
        .filter_map(|p| out.symbol_family.get(&p.symbol).copied())
        .collect();
    let named: BTreeSet<i64> = out.symbol_family.values().copied().collect();
    let without: Vec<i64> = used
        .iter()
        .copied()
        .filter(|f| out.family_guid.get(f).is_none_or(|g| !has_forms(g)))
        .collect();
    let orphans: Vec<i64> = family_owner
        .iter()
        .filter(|(f, o)| **o < 0 && **f != host && !named.contains(f))
        .map(|(f, _)| *f)
        .filter(|f| has_forms(&owned_document(*f)))
        .collect();
    if let ([family], [root]) = (without.as_slice(), orphans.as_slice()) {
        out.family_guid.insert(*family, owned_document(*root));
    }
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
    let alone = sketch.curves.len() == 1;
    for id in &sketch.curves {
        let (pts, closed, ext) = polyline(&whole_turn(curves.get(id)?, alone));
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

/// Whether a form's `Visible` parameter is off.
fn is_hidden(x: &SchemaObject, names: &BTreeMap<u16, String>) -> bool {
    x.objects
        .iter()
        .filter(|o| names.get(&o.tag).map(String::as_str) == Some("ParamValueSetInt"))
        .filter_map(|o| match o.get("m_paramSet") {
            Some(Value::List(l)) => Some(l),
            _ => None,
        })
        .flatten()
        .any(|e| {
            matches!(e, Value::Object(pv)
                if id_of(pv, "m_paramId") == Some(VISIBLE_PARAMETER)
                    && matches!(pv.get("m_value"), Some(Value::Int(0))))
        })
}

/// `IS_VISIBLE_PARAM`.
const VISIBLE_PARAMETER: i64 = -1_006_205;

/// A family instance's placement, from its `InstanceInfo`.
fn pose_of(x: &SchemaObject, names: &BTreeMap<u16, String>) -> Option<InstancePose> {
    let root = x.objects.first()?;
    let info = x
        .objects
        .iter()
        .find(|o| names.get(&o.tag).map(String::as_str) == Some("InstanceInfo"));
    let trf = info
        .and_then(|i| i.get("m_Trf"))
        .and_then(|t| target(x, t))?;
    let matrix = match trf.get("m_3x3")? {
        Value::List(rows) if rows.len() == 3 => {
            [vec3_of(&rows[0])?, vec3_of(&rows[1])?, vec3_of(&rows[2])?]
        }
        _ => return None,
    };
    Some(InstancePose {
        symbol: id_of(root, "m_masterSymbolId")?,
        info_symbol: info.and_then(|i| id_of(i, "m_symbolId")),
        matrix,
        origin: vec3_of(trf.get("m_or")?)?,
        flipped_x: bool_of(root, "m_flippedX"),
        flipped_y: bool_of(root, "m_flippedY"),
    })
}

/// The solid-form classes.
const FORM_CLASS_NAMES: [&str; 6] = [
    "ExtrusionElem",
    "SweepElem",
    "RevolutionElem",
    "BlendElem",
    "SweptBlendElem",
    "FreeFormElement",
];

/// The document key of the nested family `family` in a file without keyed
/// sections: its id, marked so that it is no content-document GUID.
fn owned_document(family: i64) -> Guid {
    let mut g = [0xff; 16];
    g[8..].copy_from_slice(&family.to_le_bytes());
    g
}

/// The family file's own `Family` element: of the families no other
/// family's document holds (`m_famId` -1) and no family symbol names (a
/// loaded family's own element can say -1 too, but its symbols name it), the
/// one that holds the most sketch, curve and form elements. A file can keep
/// several such roots (one Revit 2015 family measured has four; the file's
/// own holds 4,035 elements, the others 112 to 656).
fn host_family_id(rf: &mut RevitFile, names: &BTreeMap<u16, String>) -> Result<Option<i64>> {
    const COUNTED: [&str; 10] = [
        "Family",
        "FamilySymbol",
        "SketchPlane",
        "VarSketch",
        "CurveElem",
        "ExtrusionElem",
        "SweepElem",
        "RevolutionElem",
        "BlendElem",
        "SweptBlendElem",
    ];
    let tags: BTreeMap<u16, &str> = names
        .iter()
        .filter_map(|(t, n)| COUNTED.contains(&n.as_str()).then_some((*t, n.as_str())))
        .collect();
    let mut roots: BTreeSet<i64> = BTreeSet::new();
    let mut named: BTreeSet<i64> = BTreeSet::new();
    let mut held: BTreeMap<i64, usize> = BTreeMap::new();
    for_each_object_where(
        rf,
        |frame, _| tags.contains_key(&frame.header.tag),
        |x| {
            let Some(root) = x.objects.first() else {
                return;
            };
            let owner = id_of(root, "m_famId");
            match tags.get(&x.frame.header.tag) {
                Some(&"Family") if owner.is_none_or(|f| f < 0) => {
                    roots.insert(x.frame.header.id as i64);
                }
                Some(&"FamilySymbol") => {
                    if let Some(f) = id_of(root, "m_familyId") {
                        named.insert(f);
                    }
                }
                _ => {}
            }
            if let Some(f) = owner.filter(|f| *f > 0) {
                *held.entry(f).or_default() += 1;
            }
        },
    )?;
    Ok(roots
        .iter()
        .filter(|r| !named.contains(r))
        .max_by_key(|r| (held.get(r).copied().unwrap_or(0), std::cmp::Reverse(**r)))
        .copied())
}

/// Whether a decoded box is the stored one: equal within [`BBOX_TOLERANCE`],
/// or larger on any side by at most `sag` more (a faceted stored box).
fn box_matches(stored: &Bounds, decoded: &Bounds, sag: f64) -> bool {
    let ok = |gap: f64| (-BBOX_TOLERANCE..=BBOX_TOLERANCE + sag).contains(&gap);
    (0..3).all(|k| ok(stored[0][k] - decoded[0][k]) && ok(decoded[1][k] - stored[1][k]))
}

/// The volume a closed, outward-wound mesh encloses.
fn mesh_volume(vertices: &[[f64; 3]], triangles: &[[u32; 3]]) -> f64 {
    triangles
        .iter()
        .map(|t| {
            let [a, b, c] = t.map(|i| vertices[i as usize]);
            dot(a, cross(b, c))
        })
        .sum::<f64>()
        / 6.0
}

/// Whether a revolution's stored box is its six-facet display's: on each
/// axis it may lie inside the solved box by [`REVOLUTION_FACET_SAG`] of the
/// solved half-extent there, scaled by how far that axis is from the
/// revolution's (none along the axis itself, where facets change nothing).
fn revolution_box_matches(stored: &Bounds, solved: &Bounds, axis: [f64; 3]) -> bool {
    (0..3).all(|k| {
        let radial = (1.0 - axis[k] * axis[k]).max(0.0).sqrt();
        let sag = REVOLUTION_FACET_SAG * (solved[1][k] - solved[0][k]) / 2.0 * radial;
        let ok = |gap: f64| (-BBOX_TOLERANCE..=BBOX_TOLERANCE + sag).contains(&gap);
        ok(stored[0][k] - solved[0][k]) && ok(solved[1][k] - stored[1][k])
    })
}

/// The largest radius of the arcs and ellipses of a form's sketches.
fn form_arc_radius(f: &RawForm, ctx: &SolveCtx) -> f64 {
    f.sketch_ids
        .values()
        .filter_map(|id| ctx.sketches.get(id))
        .flat_map(|s| &s.curves)
        .filter_map(|c| match ctx.curves.get(c)? {
            Curve::Arc { radius, .. } => Some(*radius),
            Curve::Ellipse { a, b, .. } => Some(a.max(*b)),
            Curve::Line { .. } => None,
        })
        .fold(0.0, f64::max)
}

/// The largest radius of an extrusion's arcs and ellipses (0 without).
fn max_arc_radius(
    e: &RawExtrusion,
    sketches: &BTreeMap<i64, RawSketch>,
    curves: &BTreeMap<i64, Curve>,
) -> f64 {
    sketches
        .get(&e.sketch)
        .into_iter()
        .flat_map(|s| &s.curves)
        .filter_map(|c| match curves.get(c)? {
            Curve::Arc { radius, .. } => Some(*radius),
            Curve::Ellipse { a, b, .. } => Some(a.max(*b)),
            Curve::Line { .. } => None,
        })
        .fold(0.0, f64::max)
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

/// A sketch's only curve as the whole turn when it falls short of one by at
/// most a thousandth: Revit 2011 stores a sketched circle as one arc from
/// pi/2000 to 2pi - pi/2000 (one door family measured: 18 extrusions whose
/// only curve is such an arc). A curve with others in its sketch is kept as
/// it is, as they may close it.
fn whole_turn(c: &Curve, alone: bool) -> Curve {
    use std::f64::consts::TAU;
    let near = |t: &[f64; 2]| {
        let sweep = t[1] - t[0];
        sweep.abs() > 1e-12 && (TAU - sweep.abs()).abs() <= TAU / 1000.0
    };
    let mut c = c.clone();
    if let (true, Curve::Arc { t, .. } | Curve::Ellipse { t, .. }) = (alone, &mut c) {
        if near(t) {
            t[1] = t[0] + TAU.copysign(t[1] - t[0]);
        }
    }
    c
}

/// A sketch's curves as polylines, with their exact axis extremes.
fn sketch_pieces(
    sketch: &RawSketch,
    curves: &BTreeMap<i64, Curve>,
) -> Option<(Vec<Piece>, Vec<[f64; 3]>)> {
    let mut pieces = Vec::new();
    let mut extremes = Vec::new();
    let alone = sketch.curves.len() == 1;
    for id in &sketch.curves {
        let (pts, closed, ext) = polyline(&whole_turn(curves.get(id)?, alone));
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
            Ok((
                FamilySolid {
                    revolution_axis: Some(d),
                    ..solid
                },
                bbox,
            ))
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

    /// Element ids are each keyed document's own: a symbol id that names one
    /// family in the file's own document names another inside a nested one.
    #[test]
    fn a_nested_documents_symbols_are_its_own() {
        let (a, b, c) = ([1u8; 16], [2u8; 16], [3u8; 16]);
        let mut d = FamilyDocuments::default();
        for g in [a, b, c] {
            d.docs.insert(g, DocSolids::default());
        }
        d.symbol_family.insert(7, 70);
        d.family_guid.insert(70, a);
        d.doc_symbol_family.insert((b, 7), 71);
        d.doc_family_guid.insert((b, 71), c);
        assert_eq!(d.document_of_symbol_in(b, 7).map(|(g, _)| g), Some(c));
        assert_eq!(d.document_of_symbol_in(a, 7).map(|(g, _)| g), Some(a));
    }

    #[test]
    fn pieces_given_in_either_direction_join() {
        let mut p = square(0.0);
        p[1].0.reverse();
        let rings = loops(&p).unwrap();
        assert_eq!(rings.len(), 1);
        assert_eq!(rings[0].len(), 4);
    }

    /// A sketched circle saved as an arc a thousandth of a turn short closes
    /// into one ring and extrudes; a half circle does not close.
    #[test]
    fn an_arc_just_short_of_a_turn_is_a_closed_circle() {
        use std::f64::consts::{PI, TAU};
        let arc = |t: [f64; 2]| Curve::Arc {
            center: [0.0; 3],
            x: [1.0, 0.0, 0.0],
            y: [0.0, 1.0, 0.0],
            radius: 1.0,
            t,
        };
        let short = arc([PI / 2000.0, TAU - PI / 2000.0]);
        assert!(!polyline(&whole_turn(&short, false)).1);
        let (pts, closed, _) = polyline(&whole_turn(&short, true));
        assert!(closed);
        let rings = loops(&[(pts, closed)]).unwrap();
        let s = prism(&rings, [0.0, 0.0, 1.0], 0.0, 1.0).unwrap();
        assert!((volume(&s) - PI).abs() < 0.03, "{}", volume(&s));
        assert!(!polyline(&whole_turn(&arc([0.0, PI]), true)).1);
    }

    /// A faceted stored box may lie inside the decoded one by the sag, never
    /// outside it.
    #[test]
    fn a_faceted_box_matches_within_the_sag_only_inward() {
        let decoded = [[0.0, 0.0, 0.0], [1.0, 1.0, 1.0]];
        let inward = [[0.002, 0.0, 0.0], [1.0, 0.998, 1.0]];
        assert!(!box_matches(&inward, &decoded, 0.0));
        assert!(box_matches(&inward, &decoded, ARC_FACET_SAG * 0.1));
        let outward = [[-0.002, 0.0, 0.0], [1.0, 1.0, 1.0]];
        assert!(!box_matches(&outward, &decoded, 1.0));
        assert!(box_matches(&decoded, &decoded, 0.0));
    }

    #[test]
    fn a_nested_family_key_is_not_a_content_document_guid() {
        assert_ne!(owned_document(1801), HOST_DOCUMENT);
        assert_ne!(owned_document(1801), owned_document(2120));
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
            tag: 0,
            part: 0,
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
