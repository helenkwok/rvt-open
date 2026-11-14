//! Plan views and the annotation they own, read through the file's schema.
//!
//! - A plan view is a `DBViewPlan`: its name (`m_viewName`), the Level it is generated from
//!   (`m_genElemId`), its view range (`m_cutPlaneElev`, `m_bottomClipElev`, `m_topClipElev`,
//!   feet above the internal origin), whether it is a template (`m_isTemplate`) and its plan
//!   type (`m_planViewType`).
//! - Its crop box is the `BoundedSpace` (`m_boundedSpace`) of the `Viewer` whose `m_dbViewId`
//!   names it: an origin, three axes and, per axis, the offsets of the box's two faces
//!   (`m_boundOffset`), with `m_boundActive` saying which faces crop. Its annotation crop is on when
//!   `m_bAnnotationCropActive` is set, lying outside the crop box by
//!   `m_pAnnotationCropOffsets.m_leftRightTopBottomOffsets` (sheet feet).
//! - What a view hides: the categories in its `IckyExcludedCategoriesSetPtrWrapper` draw filter
//!   (`m_categoryIds`, BuiltInCategory ids; Planting, Topography and Mass on a floor plan of
//!   rac_basic_sample_project) and the elements in its `HiddenElementsViewSettings`
//!   (`m_hiddenElements`).
//! - Levels and grids are `DatumPlane`s: the name is `m_text`; a grid's line is its plane
//!   (`m_pSurface`) from origin along `m_xVec` across the plane's extent.
//! - Annotation belongs to the view named by its `m_ownerDBViewId`: room tags (`RoomTag`, at its
//!   head, `m_oTagOrientationCell.m_headLocation`, text from the tagged room's Name and Number),
//!   text notes (`TextElement`, text from its saved rich text) and linear dimensions
//!   (`LinearDimString`: its `m_refPnts` projected onto its dimension line, `m_pDimLine`).
//! - A sheet is a view with a sheet number (`m_sheetNumber`; its name is `m_viewName`). What it
//!   shows: the `Viewport`s it owns, each naming a view (`m_dbViewId`) and where that view's
//!   internal origin lies on the sheet (`m_offset`, paper feet), and the title blocks it owns
//!   (`FamilyInstance`s holding a `FamInstTitleBlock`, by symbol `m_masterSymbolId` and origin
//!   `m_instOrigin`). Its text notes and detail lines are the annotation it owns, in paper feet.

use crate::schema_objects::{Fields, ParamValue, SchemaObject, Value, for_each_object_where};
use crate::{Result, RevitFile};
use std::collections::BTreeMap;

/// Autodesk `BuiltInParameter.ROOM_NAME` and `ROOM_NUMBER`.
const ROOM_NAME: i64 = -1_006_900;
const ROOM_NUMBER: i64 = -1_006_901;

/// `m_planViewType` of a floor plan (a reflected ceiling plan is 2), measured on
/// rac_basic_sample_project against the API's ViewType.
const FLOOR_PLAN: i64 = 1;

/// A plan view.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct PlanView {
    pub id: i64,
    pub name: String,
    pub template: bool,
    /// `m_planViewType` as stored.
    pub plan_type: Option<i64>,
    /// The Level it is generated from, and that Level's name.
    pub level_id: Option<i64>,
    pub level: Option<String>,
    /// View range, feet above the internal origin.
    pub cut_plane: Option<f64>,
    pub bottom: Option<f64>,
    pub top: Option<f64>,
    pub level_below: Option<f64>,
    pub crop: Option<CropBox>,
    /// View scale (100 for 1:100): sheet sizes times this are model sizes. Stored as its inverse
    /// (`m_scale`, 0.01 for 1:100 and 1/96 for 1/8" = 1'-0").
    pub scale: Option<f64>,
    /// Categories the view hides (its visibility settings), as BuiltInCategory ids.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hidden_categories: Vec<i64>,
    /// Elements hidden in the view.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hidden_elements: Vec<i64>,
    /// The view template that controls its visibility settings (`m_viewTemplateId`).
    pub template_id: Option<i64>,
    /// Whether it is placed on a sheet (`m_sheetViewportElemId` names a viewport).
    pub on_sheet: bool,
}

/// A view's crop box: `origin` and the view's axes (`x`, `y`, `z`, model feet), and per axis
/// the box's two faces as offsets from the origin, with which of them crop.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct CropBox {
    pub origin: [f64; 3],
    pub axes: [[f64; 3]; 3],
    pub min: [f64; 3],
    pub max: [f64; 3],
    pub active: [[bool; 2]; 3],
    pub on: bool,
    /// The annotation crop, when it is on: how far it lies outside the crop box on the left,
    /// right, top and bottom, in sheet feet.
    pub annotation: Option<[f64; 4]>,
}

/// A level or grid.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct Datum {
    pub id: i64,
    pub name: String,
    /// A grid's line, its two ends (model feet).
    pub ends: Option<[[f64; 3]; 2]>,
    /// The line's origin and unit direction: a view's extents of the grid are positions along it.
    #[serde(skip)]
    pub axis: Option<([f64; 3], [f64; 3])>,
}

/// A grid's extent in one view (`ExtentElem.m_idExtents`, a `DatumPlaneRec` per grid): its free
/// and bubble ends as positions along the grid's line, and which ends show a head.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct GridExtent {
    pub view: i64,
    pub grid: i64,
    pub free: f64,
    pub bubble: f64,
    pub free_head: bool,
    pub bubble_head: bool,
}

/// A text a view owns, model feet.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct ViewText {
    pub view: i64,
    pub at: [f64; 3],
    pub value: String,
    pub height: Option<f64>,
    /// Angle of the text's base direction, radians.
    pub rotation: f64,
}

/// A dimension a view owns: its reference points, model feet.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct ViewDimension {
    pub view: i64,
    pub points: Vec<[f64; 3]>,
}

/// A room tag a view owns.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct ViewRoomTag {
    pub view: i64,
    pub room: Option<i64>,
    /// Where the tag sits, when stored; else the room's point is used.
    pub at: Option<[f64; 3]>,
}

/// A room's tag text and its point.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct RoomLabel {
    pub name: Option<String>,
    pub number: Option<String>,
    pub point: Option<[f64; 2]>,
}

/// A line of the project (not of a family): a model line, a room separation line, a property
/// line, or a detail line or filled region edge a view owns (`view`). `category` is
/// `CurveElemData.m_categoryId`: a built-in category (negative) or the line style. Points in model
/// feet; an arc as chords.
///
/// A sketch's lines (`SketchMembership.m_groupId`) are kept where a plan draws its owner as its
/// boundary: a filled region, a property, a floor, a toposolid, a building pad. Lines that only
/// shape another element are left out: a family's (`m_famId` set), other sketches' (ceilings,
/// roofs, railings, stairs, walls, openings, plan regions), a sweep's path (`Path3d`) and a
/// profile's reference plane, and area boundaries (`m_areaSchemeId` set, drawn only in area plans).
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct ViewCurve {
    pub view: Option<i64>,
    pub category: Option<i64>,
    pub kind: LineKind,
    pub points: Vec<[f64; 3]>,
}

/// What a project line is, from the element that owns it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub enum LineKind {
    /// A model or detail line, or a filled region's edge.
    #[default]
    Line,
    /// A room separation line (`m_areaSeparation` with no area scheme).
    RoomSeparation,
    /// An edge of a property's sketch (`Property.m_sketchId`).
    PropertyLine,
    /// An edge of a floor's sketch, its boundary or an opening in it.
    Floor,
    /// An edge of a toposolid's sketch.
    Toposolid,
    /// An edge of a building pad's sketch.
    BuildingPad,
}

/// A section or elevation view (`DBViewSection`): its axes on the sheet, right (`m_horzDir`) and up
/// (`m_vertDir`), the direction toward its viewer (`m_viewDir`), and, as for a plan, its crop box,
/// scale and what it hides. Model feet.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct SectionView {
    pub id: i64,
    pub name: String,
    pub template: bool,
    pub right: [f64; 3],
    pub up: [f64; 3],
    pub toward_viewer: [f64; 3],
    pub crop: Option<CropBox>,
    pub scale: Option<f64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hidden_categories: Vec<i64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hidden_elements: Vec<i64>,
    pub template_id: Option<i64>,
}

/// A sheet.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct Sheet {
    pub id: i64,
    pub number: String,
    pub name: String,
}

/// A view placed on a sheet.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct SheetViewport {
    pub id: i64,
    pub sheet: i64,
    pub view: i64,
    /// Where the view's internal origin lies on the sheet, paper feet (`m_offset`).
    pub offset: [f64; 2],
    /// The view title's origin on the sheet, paper feet, and the length of its line.
    pub label_origin: [f64; 2],
    pub label_length: f64,
    pub detail_number: String,
}

/// A title block placed on a sheet.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct TitleBlock {
    pub id: i64,
    pub sheet: i64,
    pub symbol: i64,
    /// Origin on the sheet and x direction, paper feet.
    pub origin: [f64; 2],
    pub x_dir: [f64; 2],
}

/// Everything [`read`] finds.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct ViewFacts {
    pub views: Vec<PlanView>,
    pub levels: Vec<Datum>,
    pub grids: Vec<Datum>,
    pub texts: Vec<ViewText>,
    pub dimensions: Vec<ViewDimension>,
    pub room_tags: Vec<ViewRoomTag>,
    pub rooms: BTreeMap<i64, RoomLabel>,
    pub curves: Vec<ViewCurve>,
    pub grid_extents: Vec<GridExtent>,
    pub sections: Vec<SectionView>,
    pub sheets: Vec<Sheet>,
    pub viewports: Vec<SheetViewport>,
    pub title_blocks: Vec<TitleBlock>,
}

fn id(f: &Fields, name: &str) -> Option<i64> {
    match f.get(name)? {
        Value::Id(i) if *i >= 0 => Some(*i),
        _ => None,
    }
}

fn float(f: &Fields, name: &str) -> Option<f64> {
    match f.get(name)? {
        Value::Float(v) if v.is_finite() => Some(*v),
        _ => None,
    }
}

fn text(f: &Fields, name: &str) -> Option<String> {
    match f.get(name)? {
        Value::Text(s) => Some(s.clone()),
        _ => None,
    }
}

fn flag(f: &Fields, name: &str) -> bool {
    matches!(f.get(name), Some(Value::Bool(true)) | Some(Value::Int(1)))
}

fn floats(v: &Value) -> Vec<f64> {
    match v {
        Value::Float(x) => vec![*x],
        Value::List(items) => items.iter().flat_map(floats).collect(),
        Value::Object(o) => o.values.iter().flat_map(|(_, v)| floats(v)).collect(),
        _ => Vec::new(),
    }
}

fn bools(v: &Value) -> Vec<bool> {
    match v {
        Value::Bool(b) => vec![*b],
        Value::Int(i) => vec![*i != 0],
        Value::List(items) => items.iter().flat_map(bools).collect(),
        Value::Object(o) => o.values.iter().flat_map(|(_, v)| bools(v)).collect(),
        _ => Vec::new(),
    }
}

fn point(f: &Fields, name: &str) -> Option<[f64; 3]> {
    let v = floats(f.get(name)?);
    (v.len() == 3 && v.iter().all(|x| x.is_finite())).then(|| [v[0], v[1], v[2]])
}

/// The fields of the object a `New` pointer in `f` introduced.
fn pointee<'a>(o: &'a SchemaObject, f: &Fields, name: &str) -> Option<&'a Fields> {
    match f.get(name)? {
        Value::New { index, .. } => o.objects.get(*index),
        _ => None,
    }
}

fn crop_box(f: &Fields) -> Option<CropBox> {
    let origin = point(f, "m_orig")?;
    let basis = floats(f.get("m_basis")?);
    let offsets = floats(f.get("m_boundOffset")?);
    if basis.len() != 9 || offsets.len() != 6 {
        return None;
    }
    let active = bools(f.get("m_boundActive")?);
    let axis = |i: usize| [basis[3 * i], basis[3 * i + 1], basis[3 * i + 2]];
    let on = |i: usize| active.get(i).copied().unwrap_or(false);
    // Each axis holds its two faces' offsets, the far face first (measured on
    // rac_basic_sample_project against the API's CropBox Min and Max).
    let lo = |i: usize| offsets[2 * i].min(offsets[2 * i + 1]);
    let hi = |i: usize| offsets[2 * i].max(offsets[2 * i + 1]);
    Some(CropBox {
        origin,
        axes: [axis(0), axis(1), axis(2)],
        min: [lo(0), lo(1), lo(2)],
        max: [hi(0), hi(1), hi(2)],
        active: [[on(0), on(1)], [on(2), on(3)], [on(4), on(5)]],
        on: flag(f, "m_isOn"),
        annotation: None,
    })
}

/// Every ElementId in field `field` of the objects of class `class` that `o` introduced (a
/// view's hidden categories and elements live in such objects).
fn ids_in(
    o: &SchemaObject,
    layouts: &crate::formats::SchemaLayouts,
    class: &str,
    field: &str,
) -> Vec<i64> {
    fn ids(v: &Value, out: &mut Vec<i64>) {
        match v {
            Value::Id(i) => out.push(*i),
            Value::List(items) => items.iter().for_each(|i| ids(i, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for f in &o.objects {
        if layouts.classes.get(&f.tag).is_some_and(|l| l.name == class) {
            if let Some(v) = f.get(field) {
                ids(v, &mut out);
            }
        }
    }
    out
}

/// The tag of every class that is, or derives from, one of `names`, with that name.
fn tags_of(
    layouts: &crate::formats::SchemaLayouts,
    names: &[&'static str],
) -> BTreeMap<u16, &'static str> {
    let mut out = BTreeMap::new();
    for tag in layouts.classes.keys() {
        let Some(chain) = layouts.chain(*tag) else {
            continue;
        };
        // Nearest class first, so a RoomTag is a RoomTag before it is anything wider.
        if let Some(name) = chain
            .iter()
            .rev()
            .find_map(|c| names.iter().find(|n| **n == c.name))
        {
            out.insert(*tag, *name);
        }
    }
    out
}

/// Read every plan view, level, grid and the annotation views own, one object at a time.
pub fn read(rf: &mut RevitFile) -> Result<ViewFacts> {
    let layouts = rf.schema_layouts()?;
    let wanted = tags_of(
        &layouts,
        &[
            "DBViewPlan",
            "Viewer",
            "Level",
            "Grid",
            "RoomTag",
            "RoomElem",
            "TextElement",
            "LinearDimString",
            "CurveElem",
            "ContourLabelingElem",
            "FilledRegion",
            "Property",
            "Floor",
            "Ceiling",
            "Toposolid",
            "BuildingPad",
            "Path3d",
            "ProfileRefPlane",
            "ExtentElem",
            "DBView",
            "DBViewSection",
            "Viewport",
            "FamilyInstance",
        ],
    );
    // Partition sections are the loaded families' own documents: a curve after a section start
    // is a family's, not the project's.
    let mut sections: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for name in rf.partition_stream_names() {
        if let Ok(inflated) = rf.inflated_partition(&name) {
            let starts: Vec<usize> = inflated.sections().iter().map(|(at, _)| *at).collect();
            sections.insert(name, starts);
        }
    }
    let in_family = |o: &SchemaObject| {
        sections
            .get(&o.stream)
            .and_then(|s| s.first())
            .is_some_and(|first| *first <= o.frame.span.start)
    };
    let mut out = ViewFacts::default();
    let mut crops: BTreeMap<i64, CropBox> = BTreeMap::new();
    // Sketches whose lines are drawn (a filled region's, a property's), lines that only shape
    // something else (a sweep path's, a profile plane's), and each candidate line's id and sketch.
    let mut drawn_sketches: BTreeMap<i64, LineKind> = BTreeMap::new();
    let mut shaping: std::collections::BTreeSet<i64> = Default::default();
    let mut curve_owners: Vec<(i64, Option<i64>)> = Vec::new();
    let sketch_of = |o: &SchemaObject| {
        o.objects.iter().find_map(|f| {
            (layouts.classes.get(&f.tag)?.name == "SketchMembership")
                .then(|| id(f, "m_groupId"))
                .flatten()
        })
    };
    for_each_object_where(
        rf,
        |f, _| wanted.contains_key(&f.header.tag),
        |o| {
            let (Some(element), Some(root)) = (o.element(), o.root()) else {
                return;
            };
            let owner = id(root, "m_ownerDBViewId");
            match wanted.get(&o.frame.header.tag).copied() {
                // A loaded family's own plan views (its `Ref. Level` plans) are not the project's.
                Some("DBViewPlan") if !in_family(o) => out.views.push(PlanView {
                    id: element,
                    name: text(root, "m_viewName").unwrap_or_default(),
                    template: flag(root, "m_isTemplate"),
                    plan_type: match root.get("m_planViewType") {
                        Some(Value::Int(i)) => Some(*i),
                        _ => None,
                    },
                    level_id: id(root, "m_genElemId"),
                    level: None,
                    cut_plane: float(root, "m_cutPlaneElev"),
                    bottom: float(root, "m_bottomClipElev"),
                    top: float(root, "m_topClipElev"),
                    level_below: float(root, "m_levelBelowElev"),
                    crop: None,
                    scale: float(root, "m_scale").filter(|s| *s > 0.0).map(|s| 1.0 / s),
                    hidden_categories: ids_in(
                        o,
                        &layouts,
                        "IckyExcludedCategoriesSetPtrWrapper",
                        "m_categoryIds",
                    ),
                    hidden_elements: ids_in(
                        o,
                        &layouts,
                        "HiddenElementsViewSettings",
                        "m_hiddenElements",
                    ),
                    template_id: id(root, "m_viewTemplateId"),
                    on_sheet: id(root, "m_sheetViewportElemId").is_some(),
                }),
                Some("DBViewSection") if !in_family(o) => {
                    let dir = |name: &str| point(root, name).unwrap_or_default();
                    out.sections.push(SectionView {
                        id: element,
                        name: text(root, "m_viewName").unwrap_or_default(),
                        template: flag(root, "m_isTemplate"),
                        right: dir("m_horzDir"),
                        up: dir("m_vertDir"),
                        toward_viewer: dir("m_viewDir"),
                        crop: None,
                        scale: float(root, "m_scale").filter(|s| *s > 0.0).map(|s| 1.0 / s),
                        hidden_categories: ids_in(
                            o,
                            &layouts,
                            "IckyExcludedCategoriesSetPtrWrapper",
                            "m_categoryIds",
                        ),
                        hidden_elements: ids_in(
                            o,
                            &layouts,
                            "HiddenElementsViewSettings",
                            "m_hiddenElements",
                        ),
                        template_id: id(root, "m_viewTemplateId"),
                    });
                }
                Some("DBView") => {
                    if let Some(number) = text(root, "m_sheetNumber").filter(|n| !n.is_empty()) {
                        out.sheets.push(Sheet {
                            id: element,
                            number,
                            name: text(root, "m_viewName").unwrap_or_default(),
                        });
                    }
                }
                Some("Viewport") => {
                    let xy = |name: &str| point(root, name).map(|p| [p[0], p[1]]);
                    if let (Some(sheet), Some(view), Some(offset)) =
                        (owner, id(root, "m_dbViewId"), xy("m_offset"))
                    {
                        out.viewports.push(SheetViewport {
                            id: element,
                            sheet,
                            view,
                            offset,
                            label_origin: xy("m_labelOrigin").unwrap_or_default(),
                            label_length: float(root, "m_labelLength").unwrap_or(0.0),
                            detail_number: text(root, "m_detailNumber").unwrap_or_default(),
                        });
                    }
                }
                Some("FamilyInstance") if !in_family(o) => {
                    let title_block = o.objects.iter().skip(1).any(|f| {
                        layouts
                            .classes
                            .get(&f.tag)
                            .is_some_and(|c| c.name == "FamInstTitleBlock")
                    });
                    if let (true, Some(sheet), Some(symbol)) =
                        (title_block, owner, id(root, "m_masterSymbolId"))
                    {
                        let origin = point(root, "m_instOrigin").unwrap_or_default();
                        let x = point(root, "m_RefDir").unwrap_or([1.0, 0.0, 0.0]);
                        out.title_blocks.push(TitleBlock {
                            id: element,
                            sheet,
                            symbol,
                            origin: [origin[0], origin[1]],
                            x_dir: [x[0], x[1]],
                        });
                    }
                }
                Some("Viewer") => {
                    if let (Some(view), Some(b)) =
                        (id(root, "m_dbViewId"), root.get("m_boundedSpace"))
                    {
                        let fields = match b {
                            Value::Object(f) => Some(f),
                            _ => None,
                        };
                        if let Some(mut c) = fields.and_then(crop_box) {
                            // The annotation crop: offsets from the crop box (left, right, top,
                            // bottom), used when `m_bAnnotationCropActive`.
                            if flag(root, "m_bAnnotationCropActive") {
                                let v = pointee(o, root, "m_pAnnotationCropOffsets")
                                    .and_then(|a| a.get("m_leftRightTopBottomOffsets"))
                                    .map(floats)
                                    .unwrap_or_default();
                                c.annotation = Some(if v.len() == 4 {
                                    [v[0], v[1], v[2], v[3]]
                                } else {
                                    [0.0; 4]
                                });
                            }
                            crops.insert(view, c);
                        }
                    }
                }
                Some("ExtentElem") => {
                    let (Some(view), Some(Value::List(items))) =
                        (id(root, "m_dbViewId"), root.get("m_idExtents"))
                    else {
                        return;
                    };
                    for item in items {
                        let Value::Object(pair) = item else { continue };
                        let rec = match pair.get("second") {
                            Some(Value::Object(f)) => pointee(o, f, "m_oExtentData"),
                            _ => None,
                        };
                        let (Some(grid), Some(rec)) = (id(pair, "first"), rec) else {
                            continue;
                        };
                        if layouts.classes.get(&rec.tag).map(|c| c.name.as_str())
                            != Some("DatumPlaneRec")
                        {
                            continue;
                        }
                        // The view's own (2D) extents when set, else the grid's (3D) ones.
                        let ends = match (float(rec, "m_freeEnd"), float(rec, "m_bubbleEnd")) {
                            (Some(f), Some(b)) if f != 0.0 || b != 0.0 => Some((f, b)),
                            _ => float(rec, "m_freeEnd3d").zip(float(rec, "m_bubbleEnd3d")),
                        };
                        if let Some((free, bubble)) = ends.filter(|(f, b)| (f - b).abs() > 1e-9) {
                            // `m_endBubbleOverrides`: bit 1 shows a head at the free end, bit 8
                            // hides the one at the bubble end (measured against the DWG exports
                            // of rac_basic_sample_project's Level 2 and the Snowdon Towers
                            // sample's L1).
                            let flags = match rec.get("m_endBubbleOverrides") {
                                Some(Value::Int(i)) => *i,
                                _ => 0,
                            };
                            out.grid_extents.push(GridExtent {
                                view,
                                grid,
                                free,
                                bubble,
                                free_head: flags & 1 != 0,
                                bubble_head: flags & 8 == 0,
                            });
                        }
                    }
                }
                Some(kind @ ("Level" | "Grid")) => {
                    let axis = pointee(o, root, "m_pSurface").and_then(plane_axis);
                    let d = Datum {
                        id: element,
                        name: text(root, "m_text").unwrap_or_default(),
                        ends: axis.map(|(origin, dir, lo, hi)| {
                            [0, 1].map(|k| {
                                let t = if k == 0 { lo } else { hi };
                                [0, 1, 2].map(|i| origin[i] + t * dir[i])
                            })
                        }),
                        axis: axis.map(|(origin, dir, _, _)| (origin, dir)),
                    };
                    if kind == "Level" {
                        out.levels.push(d);
                    } else {
                        out.grids.push(d);
                    }
                }
                Some("RoomTag") => {
                    if let Some(view) = owner {
                        out.room_tags.push(ViewRoomTag {
                            view,
                            room: match root.get("m_taggedRoomElemId") {
                                Some(Value::Id(i)) if *i >= 0 => Some(*i),
                                Some(Value::Object(f)) => {
                                    f.values.iter().find_map(|(_, v)| match v {
                                        Value::Id(i) if *i >= 0 => Some(*i),
                                        _ => None,
                                    })
                                }
                                _ => None,
                            },
                            at: pointee(o, root, "m_oTagOrientationCell")
                                .and_then(|c| point(c, "m_headLocation")),
                        });
                    }
                }
                Some("RoomElem") => {
                    let mut label = RoomLabel {
                        point: {
                            let v = root.get("m_point").map(floats).unwrap_or_default();
                            (v.len() == 2).then(|| [v[0], v[1]])
                        },
                        ..RoomLabel::default()
                    };
                    for (p, v) in o.stored_parameters() {
                        match (p, v) {
                            (ROOM_NAME, ParamValue::String(s)) => label.name = Some(s),
                            (ROOM_NUMBER, ParamValue::String(s)) => label.number = Some(s),
                            _ => {}
                        }
                    }
                    out.rooms.insert(element, label);
                }
                Some("TextElement") => {
                    if let (Some(view), Some(at)) = (owner, point(root, "m_orig")) {
                        let value = rtf_text(o, root);
                        if !value.trim().is_empty() {
                            let base = point(root, "m_baseVec").unwrap_or([1.0, 0.0, 0.0]);
                            out.texts.push(ViewText {
                                view,
                                at,
                                value,
                                height: float(root, "m_height"),
                                rotation: base[1].atan2(base[0]),
                            });
                        }
                    }
                }
                Some("LinearDimString") => {
                    if let Some(view) = owner {
                        let v = root.get("m_refPnts").map(floats).unwrap_or_default();
                        let mut points: Vec<[f64; 3]> =
                            v.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
                        // A linear dimension measures along its dimension line: the reference
                        // points projected onto it.
                        if let Some(line) = pointee(o, root, "m_pDimLine") {
                            if let (Some(at), Some(d)) =
                                (point(line, "m_origin"), point(line, "m_dirVec"))
                            {
                                let n = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                                if n > 1e-12 {
                                    let d = [d[0] / n, d[1] / n, d[2] / n];
                                    for p in &mut points {
                                        let t = (p[0] - at[0]) * d[0]
                                            + (p[1] - at[1]) * d[1]
                                            + (p[2] - at[2]) * d[2];
                                        *p = [at[0] + t * d[0], at[1] + t * d[1], at[2] + t * d[2]];
                                    }
                                }
                            }
                        }
                        if points.len() >= 2 {
                            out.dimensions.push(ViewDimension { view, points });
                        }
                    }
                }
                Some(
                    kind @ ("FilledRegion" | "Property" | "Floor" | "Toposolid" | "BuildingPad"),
                ) if !in_family(o) => {
                    let kind = match kind {
                        "Property" => LineKind::PropertyLine,
                        "Floor" => LineKind::Floor,
                        "Toposolid" => LineKind::Toposolid,
                        "BuildingPad" => LineKind::BuildingPad,
                        _ => LineKind::Line,
                    };
                    // Its boundary sketch and those of the openings cut in it.
                    for sketch in o.objects.iter().filter_map(|f| id(f, "m_sketchId")) {
                        drawn_sketches.insert(sketch, kind);
                    }
                }
                Some("Path3d" | "ProfileRefPlane") if !in_family(o) => {
                    fn ids(v: &Value, out: &mut std::collections::BTreeSet<i64>) {
                        match v {
                            Value::Id(i) if *i >= 0 => {
                                out.insert(*i);
                            }
                            Value::List(items) => items.iter().for_each(|i| ids(i, out)),
                            Value::Object(f) => f.values.iter().for_each(|(_, v)| ids(v, out)),
                            _ => {}
                        }
                    }
                    for f in o.objects.iter().skip(1) {
                        f.values.iter().for_each(|(_, v)| ids(v, &mut shaping));
                    }
                    if let Some(c) = id(root, "m_curveElemId") {
                        shaping.insert(c);
                    }
                }
                // A family's own line (in-place, or kept with a loaded family) and an area
                // boundary (area plans only) are not the plan's.
                Some("CurveElem")
                    if !in_family(o)
                        && id(root, "m_famId").is_none()
                        && id(root, "m_areaSchemeId").is_none() =>
                {
                    // `CurveElemData`'s fields are read inline with the element's.
                    let category = match root.get("m_categoryId") {
                        Some(Value::Id(i)) => Some(*i),
                        _ => None,
                    };
                    let kind = if flag(root, "m_areaSeparation") {
                        LineKind::RoomSeparation
                    } else {
                        LineKind::Line
                    };
                    let points = pointee(o, root, "m_pCurveDriver")
                        .and_then(|d| pointee(o, d, "m_pCrv"))
                        .map(curve_points)
                        .unwrap_or_default();
                    if points.len() >= 2 {
                        curve_owners.push((element, sketch_of(o)));
                        out.curves.push(ViewCurve {
                            view: owner,
                            category,
                            kind,
                            points,
                        });
                    }
                }
                _ => {}
            }
        },
    )?;
    // A sketch's lines are drawn only where its owner is drawn as its boundary, as its kind.
    let mut owners = curve_owners.into_iter();
    out.curves.retain_mut(|c| {
        let Some((element, sketch)) = owners.next() else {
            return false;
        };
        if shaping.contains(&element) {
            return false;
        }
        match sketch {
            None => true,
            Some(s) => drawn_sketches.get(&s).is_some_and(|k| {
                c.kind = *k;
                true
            }),
        }
    });
    let level_names: BTreeMap<i64, String> =
        out.levels.iter().map(|l| (l.id, l.name.clone())).collect();
    for v in &mut out.views {
        v.level = v.level_id.and_then(|l| level_names.get(&l).cloned());
        v.crop = crops.remove(&v.id);
    }
    for v in &mut out.sections {
        v.crop = crops.remove(&v.id);
    }
    Ok(out)
}

/// A `GLine` or `GArc` as points: its two ends, or chords at most 1/32 of a turn.
fn curve_points(c: &Fields) -> Vec<[f64; 3]> {
    let t = c.get("m_endParams").map(floats).unwrap_or_default();
    if t.len() != 2 {
        return Vec::new();
    }
    if let (Some(origin), Some(dir)) = (point(c, "m_origin"), point(c, "m_dirVec")) {
        return t
            .iter()
            .map(|t| [0, 1, 2].map(|i| origin[i] + t * dir[i]))
            .collect();
    }
    let (Some(center), Some(x), Some(y), Some(r)) = (
        point(c, "m_center"),
        point(c, "m_xVec"),
        point(c, "m_yVec"),
        float(c, "m_radius"),
    ) else {
        return Vec::new();
    };
    let n = ((t[1] - t[0]).abs() / (std::f64::consts::TAU / 32.0))
        .ceil()
        .max(1.0) as usize;
    (0..=n)
        .map(|k| {
            let a = t[0] + (t[1] - t[0]) * k as f64 / n as f64;
            let (s, cs) = a.sin_cos();
            [0, 1, 2].map(|i| center[i] + r * (cs * x[i] + s * y[i]))
        })
        .collect()
}

/// A datum's line: its plane's origin, its direction (the plane's normal, x cross y, crossed with
/// up), and the range of the plane's extent (`m_Envelope.m_corners`, in the plane's x and y) along
/// it, free end first: the view extents of grids (`DatumPlaneRec.m_freeEnd3d`, `m_bubbleEnd3d`)
/// are positions along that direction, on rac_basic_sample_project (x horizontal) and the
/// Snowdon Towers sample (x up) alike.
fn plane_axis(plane: &Fields) -> Option<([f64; 3], [f64; 3], f64, f64)> {
    let origin = point(plane, "m_origin")?;
    let x = point(plane, "m_xVec")?;
    let y = point(plane, "m_yVec")?;
    let c = floats(plane.get("m_Envelope")?);
    if c.len() != 4 {
        return None;
    }
    let n = [
        x[1] * y[2] - x[2] * y[1],
        x[2] * y[0] - x[0] * y[2],
        x[0] * y[1] - x[1] * y[0],
    ];
    // n x (0, 0, 1).
    let (dx, dy) = (n[1], -n[0]);
    let len = dx.hypot(dy);
    if len < 1e-9 {
        return None;
    }
    let dir = [dx / len, dy / len, 0.0];
    let along = |u: f64, v: f64| (0..3).map(|i| (u * x[i] + v * y[i]) * dir[i]).sum::<f64>();
    let (a, b) = (along(c[0], c[1]), along(c[2], c[3]));
    let (lo, hi) = (a.min(b), a.max(b));
    (hi - lo > 1e-9).then_some((origin, dir, lo, hi))
}

/// The plain text of a text element, from its saved rich text (`m_oSavedRtfData`), else its
/// fragments.
fn rtf_text(o: &SchemaObject, root: &Fields) -> String {
    let bytes: Vec<u8> = pointee(o, root, "m_oSavedRtfData")
        .and_then(|r| match r.get("m_arrRtfStream")? {
            Value::List(items) => Some(
                items
                    .iter()
                    .filter_map(|v| match v {
                        Value::Int(i) => u8::try_from(*i & 0xff).ok(),
                        _ => None,
                    })
                    .collect(),
            ),
            _ => None,
        })
        .unwrap_or_default();
    if bytes.is_empty() {
        return text_fragments(o, root);
    }
    rtf_plain(&String::from_utf8_lossy(&bytes))
}

/// Rich text as plain text: control words dropped (`\par` a line break, `\'hh` a Windows-1252
/// byte, `\uN` a character), destination groups (`{\*...}`, font and colour tables) skipped.
pub(crate) fn rtf_plain(rtf: &str) -> String {
    let chars: Vec<char> = rtf.chars().collect();
    let mut out = String::new();
    // Per group depth: whether its text is skipped.
    let mut skip: Vec<bool> = vec![false];
    let mut i = 0;
    let mut uc_skip = 0usize;
    while i < chars.len() {
        let c = chars[i];
        let skipping = *skip.last().unwrap_or(&false);
        match c {
            '{' => {
                skip.push(skipping);
                i += 1;
            }
            '}' => {
                if skip.len() > 1 {
                    skip.pop();
                }
                i += 1;
            }
            '\\' => {
                i += 1;
                let Some(&n) = chars.get(i) else { break };
                if n == '*' {
                    if let Some(s) = skip.last_mut() {
                        *s = true;
                    }
                    i += 1;
                } else if n == '\'' {
                    let hex: String = chars.iter().skip(i + 1).take(2).collect();
                    if let (false, Ok(b)) = (skipping, u8::from_str_radix(&hex, 16)) {
                        if uc_skip > 0 {
                            uc_skip -= 1;
                        } else {
                            out.push(cp1252(b));
                        }
                    }
                    i += 3;
                } else if n.is_ascii_alphabetic() {
                    let start = i;
                    while chars.get(i).is_some_and(|c| c.is_ascii_alphabetic()) {
                        i += 1;
                    }
                    let word: String = chars[start..i].iter().collect();
                    let num_start = i;
                    if chars.get(i) == Some(&'-') {
                        i += 1;
                    }
                    while chars.get(i).is_some_and(|c| c.is_ascii_digit()) {
                        i += 1;
                    }
                    let num: Option<i64> =
                        chars[num_start..i].iter().collect::<String>().parse().ok();
                    if chars.get(i) == Some(&' ') {
                        i += 1;
                    }
                    match word.as_str() {
                        "fonttbl" | "colortbl" | "stylesheet" | "info" | "pict" => {
                            if let Some(s) = skip.last_mut() {
                                *s = true;
                            }
                        }
                        "par" | "line" if !skipping => out.push('\n'),
                        "tab" if !skipping => out.push('\t'),
                        "u" if !skipping => {
                            if let Some(ch) = num
                                .map(|n| if n < 0 { n + 65536 } else { n })
                                .and_then(|n| u32::try_from(n).ok())
                                .and_then(char::from_u32)
                            {
                                out.push(ch);
                            }
                            uc_skip = 1;
                        }
                        _ => {}
                    }
                } else {
                    // An escaped symbol: \\, \{, \}, \~ (no-break space).
                    if !skipping {
                        out.push(if n == '~' { ' ' } else { n });
                    }
                    i += 1;
                }
            }
            '\r' | '\n' => i += 1,
            _ => {
                if !skipping {
                    if uc_skip > 0 {
                        uc_skip -= 1;
                    } else {
                        out.push(c);
                    }
                }
                i += 1;
            }
        }
    }
    out.trim_end_matches(['\n', ' ']).to_string()
}

/// A Windows-1252 byte as a character (the bytes 0x80 to 0x9f differ from Latin-1).
fn cp1252(b: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž',
        '\u{8f}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}',
        'ž', 'Ÿ',
    ];
    match b {
        0x80..=0x9f => HIGH[usize::from(b - 0x80)],
        _ => char::from(b),
    }
}

/// The text of a text element: every string its fragments hold, in order.
fn text_fragments(o: &SchemaObject, root: &Fields) -> String {
    fn strings(o: &SchemaObject, v: &Value, out: &mut String, depth: usize) {
        if depth > 8 {
            return;
        }
        match v {
            Value::Text(s) => out.push_str(s),
            Value::List(items) => items.iter().for_each(|i| strings(o, i, out, depth + 1)),
            Value::Object(f) => f
                .values
                .iter()
                .for_each(|(_, v)| strings(o, v, out, depth + 1)),
            Value::New { index, .. } => {
                if let Some(f) = o.objects.get(*index) {
                    f.values
                        .iter()
                        .for_each(|(_, v)| strings(o, v, out, depth + 1));
                }
            }
            _ => {}
        }
    }
    let mut out = String::new();
    if let Some(v) = root.get("m_arrTextFragments") {
        strings(o, v, &mut out, 0);
    }
    out
}

impl ViewFacts {
    /// The categories `view` hides: its template's when a template controls it, else its own.
    pub fn hidden_categories<'a>(&'a self, view: &'a PlanView) -> &'a [i64] {
        view.template_id
            .and_then(|t| self.views.iter().find(|v| v.id == t && v.template))
            .map_or(&view.hidden_categories, |t| &t.hidden_categories)
    }

    /// The categories section or elevation `view` hides: its template's when one controls it.
    pub fn section_hidden_categories<'a>(&'a self, view: &'a SectionView) -> &'a [i64] {
        let t = view.template_id;
        self.views
            .iter()
            .find(|v| Some(v.id) == t && v.template)
            .map(|v| v.hidden_categories.as_slice())
            .or_else(|| {
                self.sections
                    .iter()
                    .find(|v| Some(v.id) == t && v.template)
                    .map(|v| v.hidden_categories.as_slice())
            })
            .unwrap_or(&view.hidden_categories)
    }

    /// The sheet numbered `key`, else the one named `key` (ignoring case).
    pub fn find_sheet(&self, key: &str) -> std::result::Result<&Sheet, String> {
        self.sheets
            .iter()
            .find(|s| s.number == key)
            .or_else(|| self.sheets.iter().find(|s| s.name == key))
            .or_else(|| {
                self.sheets.iter().find(|s| {
                    s.number.eq_ignore_ascii_case(key)
                        || s.name.eq_ignore_ascii_case(key)
                        || format!("{} - {}", s.number, s.name).eq_ignore_ascii_case(key)
                })
            })
            .ok_or_else(|| {
                let names: Vec<String> = self
                    .sheets
                    .iter()
                    .map(|s| format!("{} - {}", s.number, s.name))
                    .collect();
                format!("no sheet {key:?}; the sheets are: {}", names.join(", "))
            })
    }

    /// The plan view called `name`, not a template: an exact match first, then one ignoring case,
    /// then one where `_` stands for any character a file name cannot hold (`/ \ : * ? " < > |`)
    /// and accents are ignored.
    /// Among several, a floor plan is preferred, then one placed on a sheet, then one with a crop
    /// box, then the lowest id. A name no view has is read as a view's ElementId.
    pub fn find(&self, name: &str) -> std::result::Result<&PlanView, String> {
        let candidates: Vec<&PlanView> = self.views.iter().filter(|v| !v.template).collect();
        // Combining marks dropped, so a decomposed name (a file name written on macOS) meets its
        // composed form.
        let fold = |s: &str| -> Vec<char> {
            s.chars()
                .filter(|c| !('\u{300}'..='\u{36f}').contains(c))
                .collect()
        };
        // ponytail: an accented letter matches any letter, not only its base; a decomposition
        // table would be exact.
        let loose = |a: &str, b: &str| {
            let (a, b) = (fold(a), fold(b));
            a.len() == b.len()
                && a.iter().zip(&b).all(|(&x, &y)| {
                    x.to_lowercase().eq(y.to_lowercase())
                        || (x == '_' && "/\\:*?\"<>|_".contains(y))
                        || (x.is_alphabetic()
                            && y.is_alphabetic()
                            && (!x.is_ascii() || !y.is_ascii()))
                })
        };
        let tests: [&dyn Fn(&str) -> bool; 3] = [
            &|n| n == name,
            &|n| n.to_lowercase() == name.to_lowercase(),
            &|n| loose(name, n),
        ];
        for test in tests {
            let mut hits: Vec<&PlanView> = candidates
                .iter()
                .copied()
                .filter(|v| test(&v.name))
                .collect();
            if hits.is_empty() {
                continue;
            }
            hits.sort_by_key(|v| {
                (
                    v.plan_type != Some(FLOOR_PLAN),
                    !v.on_sheet,
                    v.crop.is_none(),
                    v.id,
                )
            });
            return Ok(hits[0]);
        }
        // A view's ElementId names it where its name does not, as two plans of one level (a
        // floor plan and a ceiling plan) can share a name.
        if let Some(view) = name
            .trim()
            .parse::<i64>()
            .ok()
            .and_then(|id| candidates.iter().find(|v| v.id == id))
        {
            return Ok(view);
        }
        let mut names: Vec<&str> = candidates.iter().map(|v| v.name.as_str()).collect();
        names.sort();
        names.dedup();
        Err(format!(
            "no plan view called {name:?}; the plan views are: {}",
            names.join(", ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::rtf_plain;

    #[test]
    fn rich_text_reads_as_plain_text() {
        let rtf = r"{\rtf1\ansi{\fonttbl{\f0 Arial;}}{\*\generator x;}\f0\fs20 FIRE EXIT\par Caf\'e9 \u8211? 2\par}";
        assert_eq!(rtf_plain(rtf), "FIRE EXIT\nCaf\u{e9} \u{2013} 2");
        assert_eq!(rtf_plain(r"{\rtf1 a\{b\}\\c}"), "a{b}\\c");
    }

    #[test]
    fn a_view_is_found_by_its_file_name() {
        let view = |id, name: &str| super::PlanView {
            id,
            name: name.into(),
            plan_type: Some(super::FLOOR_PLAN),
            ..Default::default()
        };
        let facts = super::ViewFacts {
            views: vec![
                view(1, "Caf\u{e9} Kitchen Plan"),
                view(2, "Live/Work Core A"),
            ],
            ..Default::default()
        };
        assert_eq!(facts.find("Cafe\u{301} Kitchen Plan").unwrap().id, 1);
        assert_eq!(facts.find("live_work core a").unwrap().id, 2);
        assert!(
            facts
                .find("Level 9")
                .unwrap_err()
                .contains("Live/Work Core A")
        );
    }
}
