//! Every element the file saves parameters for, read for IFCX.
//!
//! An IFC4 export holds the building elements, their types and storeys.
//! The Revit file holds much more: phases, design options, views, levels,
//! materials, project information, types no element uses, and every
//! parameter of each. [`read`] reads them all, and
//! [`super::graph::IfcxGraph::add_revit_elements`] puts them in the IFCX
//! file: an element the graph already has gets the parameters it lacks,
//! any other a node of its own under one `RevitElements` node.
//!
//! Each element keeps its own parameters (not its type's: those are on the
//! type's node), grouped and valued as Revit's property sets are
//! ([`super::revit_property_sets`]), under `rvtopen::revit::<Group>::<Name>`,
//! except that what Revit's export leaves out is written as saved (an
//! enumeration whose text is not known as its integer, a parameter of the
//! IFC group, a real of another spec, an ElementId that names nothing with a
//! name as its id). A phase also gets its name and its place in the project's
//! order (`Sequence Number`), and a design option its name and its set
//! (`Design Option Set Id`), which the file keeps outside its parameters.
//! Every element also gets the category and design option its record's
//! `ElementHeader` holds, and the fields of view, level, grid, wall,
//! revision, base point and location records that are built-in parameters
//! (`RECORD_FIELDS`, with the type a record names): revisions, base points,
//! project and site locations and the internal origin become nodes too.
//! A type no element names as its type is marked
//! [`RevitElement::unused_type`]: the IFCX file holds it only on request.
//! Its node's path is the GlobalId Revit's IFC exporter gives it (as the
//! research module `revit_global_ids` rebuilds it), so an element an IFC
//! export holds has the same id in both.

use super::entities::{Property, PropertyValue};
use super::revit_property_sets::grouped;
use crate::RevitFile;
use crate::research::native_element_parameters::ParameterLevel;
use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet};

/// One element and its own parameters.
#[derive(Debug, Clone)]
pub struct RevitElement {
    pub id: u64,
    /// The IFC GlobalId Revit's exporter gives it.
    pub global_id: String,
    /// The class of its saved record (`Level`, `MaterialElem`, ...).
    pub class: String,
    /// Its name entry, where it has one.
    pub name: Option<String>,
    /// Its parameters by group label.
    pub sets: BTreeMap<&'static str, Vec<Property>>,
    /// A type (its record's class derives from
    /// [`crate::file_types::TYPE_BASE_CLASS`]) that no element names: no
    /// element read takes a type value from it or names it in a type field
    /// of its record, and no exported element is typed by it.
    pub unused_type: bool,
}

/// Every element with a GlobalId and at least one parameter Revit's property
/// sets would write. Empty for a file whose parameters are not read (before
/// Revit 2015) or whose ElemTable layout gives no GlobalIds. `named_types`
/// are types the caller knows an element names (the exported model's
/// element types): they are not unused.
pub fn read(
    file: &mut RevitFile,
    named_types: &BTreeSet<u64>,
    georeference: Option<&crate::georeference::Georeference>,
) -> Result<Vec<RevitElement>> {
    let global_ids = crate::revit_global_ids::revit_global_ids(file)?;
    let ids: BTreeSet<u64> = global_ids.keys().map(|&id| u64::from(id)).collect();
    let saved =
        crate::research::native_element_parameters::read_with_records(file, &ids, kept_record)?;
    let names = file.memoised_element_names();
    let type_classes: BTreeSet<String> = file
        .schema_layouts()
        .map(|layouts| {
            layouts
                .classes
                .keys()
                .filter_map(|&tag| {
                    let chain = layouts.chain(tag)?;
                    chain
                        .iter()
                        .any(|c| c.name == crate::file_types::TYPE_BASE_CLASS)
                        .then(|| chain.last().map(|c| c.name.clone()))
                        .flatten()
                })
                .collect()
        })
        .unwrap_or_default();
    // The types an element takes a type value from, or that a type field of
    // an element's record (not a type's) names: a railing's or stair's type.
    let mut used: BTreeSet<i64> = saved
        .elements
        .iter()
        .flat_map(|(&id, parameters)| {
            parameters
                .iter()
                .filter(move |p| {
                    p.level == ParameterLevel::Type && i64::try_from(id) != Ok(p.source_element_id)
                })
                .map(|p| p.source_element_id)
        })
        .collect();
    for (class, named) in saved.named_ids.values() {
        if !type_classes.contains(class) {
            used.extend(named.iter().copied());
        }
    }
    let mut out = Vec::new();
    for (&id, parameters) in &saved.elements {
        let (Ok(id32), Ok(own_id)) = (u32::try_from(id), i64::try_from(id)) else {
            continue;
        };
        let Some(global_id) = global_ids.get(&id32) else {
            continue;
        };
        let own: Vec<_> = parameters
            .iter()
            .filter(|p| p.name.is_some() && p.source_element_id == own_id)
            .collect();
        let Some(class) = own.first().map(|p| p.source_class.clone()) else {
            continue;
        };
        let (sets, _) = grouped(own.into_iter(), true);
        if sets.is_empty() {
            continue;
        }
        let unused_type =
            type_classes.contains(&class) && !used.contains(&own_id) && !named_types.contains(&id);
        out.push(RevitElement {
            id,
            global_id: global_id.clone(),
            class,
            name: names.entries.get(&id32).map(|e| e.name.clone()),
            sets,
            unused_type,
        });
    }
    let phases = file.memoised_phase_filter();
    for (at, &phase) in phases.order.iter().enumerate() {
        let number = i64::try_from(at + 1).unwrap_or(i64::MAX);
        let property = (
            PHASE_SEQUENCE_NUMBER,
            "Sequence Number",
            PropertyValue::Integer(number),
        );
        add(
            &mut out,
            &global_ids,
            phase,
            "Phase",
            phases.names.get(&phase),
            property,
        );
    }
    let options = file.memoised_design_options();
    for set in options.sets.values() {
        for &option in &set.options {
            let property = (
                OPTION_SET_ID,
                "Design Option Set Id",
                PropertyValue::Label(set.set_id.to_string()),
            );
            let name = set.option_names.get(&option);
            add(
                &mut out,
                &global_ids,
                i64::from(option),
                "DesignOption",
                name,
                property,
            );
        }
    }
    record_fields(&mut out, &global_ids, &saved, &names);
    if let Some(georeference) = georeference {
        site_elements(
            &mut out,
            &global_ids,
            &georeference.elements,
            georeference.true_north,
        );
    }
    // Every element's category and design option, from its ElementHeader.
    for e in &mut out {
        if let Some(&category) = saved.header_categories.get(&e.id) {
            // No category (-1) as the API shows it.
            let value = (category < -1)
                .then(|| crate::builtin_parameters::category_name(category))
                .flatten()
                .map_or_else(|| category.to_string(), str::to_string);
            put(
                e,
                (ELEM_CATEGORY_PARAM, "Category", PropertyValue::Label(value)),
            );
        }
        if let Some(&option) = saved.header_design_options.get(&e.id) {
            let value = PropertyValue::Label(option.to_string());
            put(
                e,
                (
                    crate::native_metadata::DESIGN_OPTION_ID,
                    "Design Option",
                    value,
                ),
            );
        }
    }
    out.sort_by_key(|e| e.id);
    Ok(out)
}

/// `ELEM_CATEGORY_PARAM`.
const ELEM_CATEGORY_PARAM: i64 = -1_140_362;

/// The records whose fields [`record_fields`] reads: views, levels, grids,
/// their types, walls, revisions, base points, project and site locations,
/// and the internal origin.
fn kept_record(class: &str) -> bool {
    class.starts_with("DBView")
        || class.starts_with("Level")
        || class.starts_with("Grid")
        || class.ends_with("Wall")
        || matches!(
            class,
            "ProjectRevision"
                | "Viewer"
                | "BasePoint"
                | "GeoLocation"
                | "GeoSite"
                | "CoordinateSystemDisplayElem"
        )
}

/// How a record field is written.
#[derive(Clone, Copy)]
enum Field {
    /// Non-empty text.
    Text,
    /// An integer: an enumeration by its text where the built-in's is known.
    Int,
    /// A boolean.
    Bool,
    /// A length in feet.
    Feet,
    /// An ElementId, by the name of what it names, else by its id.
    Id,
    /// The view scale, saved as its inverse.
    Scale,
}

/// Fields of a record's root object (or, as `Class:path`, of its object of
/// that class) that are a built-in parameter's value
/// (the API's, read on the tiny reference models of Revit 2019 and 2024): the
/// root class's prefix, the field, the built-in's `BuiltInParameter` name
/// and how it is written.
const RECORD_FIELDS: &[(&str, &str, &str, Field)] = &[
    ("DBView", "m_viewName", "VIEW_NAME", Field::Text),
    (
        "DBView",
        "m_scale",
        "VIEW_SCALE_PULLDOWN_METRIC",
        Field::Scale,
    ),
    ("DBView", "m_scale", "VIEW_SCALE", Field::Scale),
    ("DBView", "m_viewTemplateId", "VIEW_TEMPLATE", Field::Id),
    (
        "DBView",
        "m_partsVisibility",
        "VIEW_PARTS_VISIBILITY",
        Field::Int,
    ),
    (
        "DBView",
        "m_modelDisplayMode",
        "VIEW_MODEL_DISPLAY_MODE",
        Field::Int,
    ),
    ("DBView", "m_joinCleanType", "VIEW_CLEAN_JOINS", Field::Int),
    (
        "DBView",
        "m_underlayOrientation",
        "VIEW_UNDERLAY_ORIENTATION",
        Field::Int,
    ),
    (
        "DBView",
        "m_idUnderlayLevel",
        "VIEW_UNDERLAY_BOTTOM_ID",
        Field::Id,
    ),
    (
        "DBView",
        "m_idUnderlayTopLevel",
        "VIEW_UNDERLAY_TOP_ID",
        Field::Id,
    ),
    (
        "DBView",
        "m_analysisDisplayStyleId",
        "VIEW_ANALYSIS_DISPLAY_STYLE",
        Field::Id,
    ),
    (
        "DBView",
        "m_fixedSketchPlaneId",
        "VIEW_FIXED_SKETCH_PLANE",
        Field::Id,
    ),
    ("DBView", "m_bUseTrueNorth", "PLAN_VIEW_NORTH", Field::Int),
    (
        "DBView",
        "m_viewDescription",
        "VIEW_DESCRIPTION",
        Field::Text,
    ),
    (
        "DBView",
        "ViewDisplayMgr:m_model.m_showHiddenLinesVal",
        "VIEW_SHOW_HIDDEN_LINES",
        Field::Int,
    ),
    (
        "DBView",
        "ColorFillSchemaSetting:m_bBackground",
        "COLOR_SCHEME_LOCATION",
        Field::Int,
    ),
    ("DBViewPlan", "m_genElemId", "PLAN_VIEW_LEVEL", Field::Id),
    (
        "SWall",
        "m_roomBounding",
        "WALL_ATTR_ROOM_BOUNDING",
        Field::Bool,
    ),
    (
        "SWall",
        "m_wallCrossSection",
        "WALL_CROSS_SECTION",
        Field::Int,
    ),
    (
        "SWall",
        "m_isStructuralSignificant",
        "WALL_STRUCTURAL_SIGNIFICANT",
        Field::Bool,
    ),
    ("SWall", "m_wallKeyRef", "WALL_KEY_REF_PARAM", Field::Int),
    (
        "ArcWall",
        "m_roomBounding",
        "WALL_ATTR_ROOM_BOUNDING",
        Field::Bool,
    ),
    (
        "ArcWall",
        "m_wallCrossSection",
        "WALL_CROSS_SECTION",
        Field::Int,
    ),
    (
        "ArcWall",
        "m_isStructuralSignificant",
        "WALL_STRUCTURAL_SIGNIFICANT",
        Field::Bool,
    ),
    ("ArcWall", "m_wallKeyRef", "WALL_KEY_REF_PARAM", Field::Int),
    ("Level", "m_text", "DATUM_TEXT", Field::Text),
    (
        "Level",
        "m_isBuildingStory",
        "LEVEL_IS_BUILDING_STORY",
        Field::Bool,
    ),
    (
        "Level",
        "m_roomComputationElevationOffset",
        "LEVEL_ROOM_COMPUTATION_HEIGHT",
        Field::Feet,
    ),
    ("Grid", "m_text", "DATUM_TEXT", Field::Text),
    (
        "ProjectRevision",
        "m_date",
        "PROJECT_REVISION_REVISION_DATE",
        Field::Text,
    ),
    (
        "ProjectRevision",
        "m_description",
        "PROJECT_REVISION_REVISION_DESCRIPTION",
        Field::Text,
    ),
    (
        "ProjectRevision",
        "m_numberStr",
        "PROJECT_REVISION_REVISION_NUM",
        Field::Text,
    ),
    (
        "ProjectRevision",
        "m_sequenceNumber",
        "PROJECT_REVISION_SEQUENCE_NUM",
        Field::Int,
    ),
    (
        "ProjectRevision",
        "m_issued",
        "PROJECT_REVISION_REVISION_ISSUED",
        Field::Bool,
    ),
    (
        "ProjectRevision",
        "m_issuedBy",
        "PROJECT_REVISION_REVISION_ISSUED_BY",
        Field::Text,
    ),
    (
        "ProjectRevision",
        "m_issuedTo",
        "PROJECT_REVISION_REVISION_ISSUED_TO",
        Field::Text,
    ),
    (
        "ProjectRevision",
        "m_enumeration",
        "PROJECT_REVISION_ENUMERATION",
        Field::Int,
    ),
];

/// The fields of the records [`kept_record`] keeps, as their built-in
/// parameters ([`RECORD_FIELDS`]), and the type an element names (a view's
/// `m_dbViewTypeId`, a level's or grid's `m_attrId`, a project location's
/// `InstanceInfo.m_symbolId`) as its Type Id, Type, Family and Family and
/// Type: the type by its name (its `SymbolInfo.m_name`), the family, whose
/// name a system type does not save, by the type's ElementId as the API
/// gives it. An element is added where `out` lacks it.
fn record_fields(
    out: &mut Vec<RevitElement>,
    global_ids: &BTreeMap<u32, String>,
    saved: &crate::research::native_element_parameters::ElementParameters,
    names: &crate::partition_names::ElementNames,
) {
    use crate::graph_value::Fv;
    let id_of = |v: &Fv| crate::native_metadata::identifier(v).ok();
    let root = |id: i64| {
        u64::try_from(id)
            .ok()
            .and_then(|id| saved.records.get(&id))
            .and_then(|objects| objects.first())
    };
    let symbol_name = |id: i64| {
        u64::try_from(id)
            .ok()
            .and_then(|id| saved.records.get(&id))
            .and_then(|objects| objects.iter().find(|(class, _)| class == "SymbolInfo"))
            .and_then(|(_, fields)| fields.get("m_name"))
            .and_then(Fv::as_str)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
    };
    // The name of what an ElementId names: a level's or grid's text, a
    // view's name, a type's name, else its name entry.
    let name_of = |id: i64| -> Option<String> {
        if let Some((_, fields)) = root(id) {
            for field in ["m_text", "m_viewName", "m_name"] {
                if let Some(name) = fields
                    .get(field)
                    .and_then(Fv::as_str)
                    .filter(|n| !n.is_empty())
                {
                    return Some(name.to_string());
                }
            }
        }
        symbol_name(id).or_else(|| {
            u32::try_from(id)
                .ok()
                .and_then(|id| names.entries.get(&id))
                .map(|e| e.name.clone())
        })
    };
    // The type an element's record names: a view's `m_dbViewTypeId`, a
    // level's or grid's `m_attrId`, a project location's
    // `InstanceInfo.m_symbolId`.
    let type_of = |class: &str, fields: &Fv, objects: &[(String, Fv)]| -> Option<i64> {
        let id = if class.starts_with("DBView") && class != "DBViewType" {
            fields.get("m_dbViewTypeId").and_then(id_of)
        } else if class == "Level" || class == "Grid" {
            fields.get("m_attrId").and_then(id_of)
        } else if class == "GeoLocation" {
            objects
                .iter()
                .find(|(c, _)| c == "InstanceInfo")
                .and_then(|(_, f)| f.get("m_symbolId"))
                .and_then(id_of)
        } else {
            None
        };
        id.filter(|&t| t > 0)
    };
    let named: BTreeSet<i64> = saved
        .records
        .values()
        .filter_map(|objects| {
            let (class, fields) = objects.first()?;
            type_of(class, fields, objects)
        })
        .collect();
    for (&id, objects) in &saved.records {
        let Some((class, fields)) = objects.first() else {
            continue;
        };
        let Ok(id) = i64::try_from(id) else {
            continue;
        };
        let mut properties: Vec<(i64, &str, PropertyValue)> = Vec::new();
        for &(prefix, field, builtin, how) in RECORD_FIELDS {
            if !class.starts_with(prefix) {
                continue;
            }
            let Some(parameter) = crate::builtin_parameters::parameter_by_enum(builtin) else {
                continue;
            };
            // `Class:path` names a field of the record's object of that class.
            let v = match field.split_once(':') {
                Some((object, path)) => objects
                    .iter()
                    .find(|(class, _)| class == object)
                    .and_then(|(_, f)| path.split('.').try_fold(f, |v, key| v.get(key))),
                None => fields.get(field),
            };
            let Some(v) = v else {
                continue;
            };
            let value = match how {
                Field::Text => match v.as_str() {
                    Some(text) if !text.is_empty() => PropertyValue::Text(text.to_string()),
                    _ => continue,
                },
                Field::Int => {
                    let Some(n) = v.as_i64().or_else(|| v.as_bool().map(i64::from)) else {
                        continue;
                    };
                    match crate::builtin_parameters::enum_text(parameter.id, n) {
                        Some(text) => PropertyValue::Label(text.to_string()),
                        None => PropertyValue::Integer(n),
                    }
                }
                Field::Bool => match v.as_bool().or_else(|| v.as_i64().map(|n| n != 0)) {
                    Some(b) => PropertyValue::Boolean(b),
                    None => continue,
                },
                Field::Feet => match v.as_f64() {
                    Some(x) if x.is_finite() => PropertyValue::LengthFeet(x),
                    _ => continue,
                },
                Field::Id => match id_of(v) {
                    Some(target) => {
                        PropertyValue::Label(name_of(target).unwrap_or_else(|| target.to_string()))
                    }
                    None => continue,
                },
                Field::Scale => match v.as_f64() {
                    Some(x) if x > 0.0 && x.is_finite() => {
                        PropertyValue::Integer((1.0 / x).round() as i64)
                    }
                    _ => continue,
                },
            };
            properties.push((parameter.id, parameter.name, value));
        }
        // A view's crop settings are on its Viewer (`m_viewerId`): the
        // annotation crop flag and the scope box it saves.
        if class.starts_with("DBView") {
            let viewer = fields.get("m_viewerId").and_then(id_of).filter(|&v| v > 0);
            if let Some((_, viewer_fields)) = viewer.and_then(root).filter(|(c, _)| c == "Viewer") {
                if let Some(on) = viewer_fields
                    .get("m_bAnnotationCropActive")
                    .and_then(Fv::as_bool)
                {
                    properties.push((
                        VIEWER_ANNOTATION_CROP_ACTIVE,
                        "Annotation Crop",
                        PropertyValue::Boolean(on),
                    ));
                }
            }
            let scope_box = viewer
                .and_then(|v| u64::try_from(v).ok())
                .and_then(|v| saved.elements.get(&v))
                .and_then(|parameters| {
                    parameters
                        .iter()
                        .find(|p| p.parameter_id == VIEWER_VOLUME_OF_INTEREST_CROP)
                })
                .and_then(|p| p.value.as_i64());
            if let Some(target) = scope_box {
                properties.push((
                    VIEWER_VOLUME_OF_INTEREST_CROP,
                    "Scope Box",
                    PropertyValue::Label(name_of(target).unwrap_or_else(|| target.to_string())),
                ));
            }
        }
        let type_id = type_of(class, fields, objects);
        if let Some(type_id) = type_id {
            let type_name = symbol_name(type_id);
            let id_text = type_id.to_string();
            properties.push((
                SYMBOL_ID_PARAM,
                "Type Id",
                PropertyValue::Label(id_text.clone()),
            ));
            properties.push((
                ELEM_TYPE_PARAM,
                "Type",
                PropertyValue::Label(type_name.clone().unwrap_or_else(|| id_text.clone())),
            ));
            properties.push((
                ELEM_FAMILY_PARAM,
                "Family",
                PropertyValue::Label(id_text.clone()),
            ));
            properties.push((
                ELEM_FAMILY_AND_TYPE_PARAM,
                "Family and Type",
                PropertyValue::Label(id_text),
            ));
        }
        let is_type = class == "GeoSite" || class.ends_with("Attributes") || class == "DBViewType";
        if is_type {
            if let Some(name) = symbol_name(id) {
                properties.push((TYPE_NAME, "Type Name", PropertyValue::Text(name)));
            }
        }
        let name = fields
            .get(if class == "GeoLocation" {
                "m_name"
            } else {
                "m_viewName"
            })
            .and_then(Fv::as_str)
            .filter(|n| !n.is_empty())
            .map(str::to_string);
        if properties.is_empty()
            && !matches!(
                class.as_str(),
                "BasePoint" | "CoordinateSystemDisplayElem" | "GeoSite"
            )
        {
            continue;
        }
        let revit_class = match class.as_str() {
            "ProjectRevision" => "Revision",
            "GeoLocation" => "ProjectLocation",
            "GeoSite" => "SiteLocation",
            "CoordinateSystemDisplayElem" => "InternalOrigin",
            other => other,
        };
        let added = !out.iter().any(|e| i64::try_from(e.id) == Ok(id));
        ensure(out, global_ids, id, revit_class, name.as_ref());
        if let Some(e) = out.iter_mut().find(|e| i64::try_from(e.id) == Ok(id)) {
            // A type no record here names, read only for its name, is
            // written only with the types no element uses.
            if added && is_type {
                e.unused_type = !named.contains(&id);
            }
            for property in properties {
                put(e, property);
            }
        }
    }
}

/// `SYMBOL_ID_PARAM`, `ELEM_TYPE_PARAM`, `ELEM_FAMILY_PARAM`,
/// `ELEM_FAMILY_AND_TYPE_PARAM` and `ALL_MODEL_TYPE_NAME`.
const SYMBOL_ID_PARAM: i64 = -1_002_000;
const ELEM_TYPE_PARAM: i64 = -1_002_050;
const ELEM_FAMILY_PARAM: i64 = -1_002_051;
const ELEM_FAMILY_AND_TYPE_PARAM: i64 = -1_002_052;
const TYPE_NAME: i64 = -1_002_001;

/// The survey point's and project base point's position in shared
/// coordinates (E/W, N/S, Elev), and the project base point's Angle to True
/// North, as the georeference reads them.
fn site_elements(
    out: &mut Vec<RevitElement>,
    global_ids: &BTreeMap<u32, String>,
    site: &crate::georeference::SiteElements,
    true_north_degrees: f64,
) {
    for &(id, [east, north, elevation], kind) in &site.base_points {
        let Ok(id) = i64::try_from(id) else {
            continue;
        };
        ensure(out, global_ids, id, "BasePoint", None);
        let Some(e) = out.iter_mut().find(|e| i64::try_from(e.id) == Ok(id)) else {
            continue;
        };
        put(
            e,
            (
                BASEPOINT_EASTWEST_PARAM,
                "E/W",
                PropertyValue::LengthFeet(east),
            ),
        );
        put(
            e,
            (
                BASEPOINT_NORTHSOUTH_PARAM,
                "N/S",
                PropertyValue::LengthFeet(north),
            ),
        );
        put(
            e,
            (
                BASEPOINT_ELEVATION_PARAM,
                "Elev",
                PropertyValue::LengthFeet(elevation),
            ),
        );
        if kind == Some(0) {
            let angle = PropertyValue::AngleRadians(true_north_degrees.to_radians());
            put(e, (BASEPOINT_ANGLETON_PARAM, "Angle to True North", angle));
        }
    }
}

/// `VIEWER_ANNOTATION_CROP_ACTIVE` and `VIEWER_VOLUME_OF_INTEREST_CROP`.
const VIEWER_ANNOTATION_CROP_ACTIVE: i64 = -1_005_094;
const VIEWER_VOLUME_OF_INTEREST_CROP: i64 = -1_012_202;

/// `BASEPOINT_*_PARAM`.
const BASEPOINT_NORTHSOUTH_PARAM: i64 = -1_150_191;
const BASEPOINT_EASTWEST_PARAM: i64 = -1_150_192;
const BASEPOINT_ELEVATION_PARAM: i64 = -1_150_193;
const BASEPOINT_ANGLETON_PARAM: i64 = -1_150_194;

/// Add element `id` to `out` with `class` and `name`, where it has a
/// GlobalId and `out` lacks it.
fn ensure(
    out: &mut Vec<RevitElement>,
    global_ids: &BTreeMap<u32, String>,
    id: i64,
    class: &str,
    name: Option<&String>,
) {
    let (Ok(id), Some(global_id)) = (
        u64::try_from(id),
        u32::try_from(id).ok().and_then(|i| global_ids.get(&i)),
    ) else {
        return;
    };
    if let Some(e) = out.iter_mut().find(|e| e.id == id) {
        if e.name.is_none() {
            e.name = name.cloned();
        }
        return;
    }
    out.push(RevitElement {
        id,
        global_id: global_id.clone(),
        class: class.to_string(),
        name: name.cloned(),
        sets: BTreeMap::new(),
        unused_type: false,
    });
}

/// `PHASE_SEQUENCE_NUMBER` and `OPTION_SET_ID`.
const PHASE_SEQUENCE_NUMBER: i64 = -1_012_111;
const OPTION_SET_ID: i64 = -1_133_603;

/// Give element `id` `property` (a built-in's id, name and value) in the
/// set Revit groups that built-in in, and its name where it has none; the
/// element is added where `out` lacks it.
fn add(
    out: &mut Vec<RevitElement>,
    global_ids: &BTreeMap<u32, String>,
    id: i64,
    class: &str,
    name: Option<&String>,
    property: (i64, &str, PropertyValue),
) {
    ensure(out, global_ids, id, class, name);
    if let Some(e) = out.iter_mut().find(|e| i64::try_from(e.id) == Ok(id)) {
        put(e, property);
    }
}

/// Give `e` `property` (a built-in's id, name and value) in the set Revit
/// groups that built-in in, unless a set of it holds one of that name.
fn put(e: &mut RevitElement, (builtin, label, value): (i64, &str, PropertyValue)) {
    if e.sets.values().flatten().any(|p| p.name == label) {
        return;
    }
    let group = crate::builtin_parameters::parameter(builtin)
        .map(|b| b.group)
        .filter(|g| !g.is_empty())
        .unwrap_or("Other");
    e.sets.entry(group).or_default().push(Property {
        name: label.to_string(),
        value,
    });
}
