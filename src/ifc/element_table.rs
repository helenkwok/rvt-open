//! The element table: one JSON record per exported element, as JSON Lines.
//!
//! Each record names the element (Revit ElementId, the GlobalId the IFC4
//! file gives it, IFC class, Revit category, family, type, level) and gives
//! its area, volume and length, then every property the export holds for it
//! (its own property sets and its type's), each with its set, name, value
//! and unit. Values are in SI units: lengths in metres, areas in square
//! metres, volumes in cubic metres, angles in degrees, masses in kilograms.
//!
//! Area, volume and length are Revit's own `Area`, `Volume` and `Length`
//! parameters where the file gives them, else (volume and length) the
//! measured quantities of the drawn body ([`super::quantities`]):
//! `quantity_source` says which for each, and `quantity_basis` whether the
//! body measured is the element's own shape or still its bounding box. An
//! unknown is `null`, never a guess.
//!
//! One record per line, in the order the export lists the elements, ending
//! with a newline, so data tools read it line by line.

use super::IfcModel;
use super::entities::{IfcEntity, PropertySet, PropertyValue};
use super::export_content::{FAMILY_NAME_PROPERTY, REVIT_OTHER_SET, TYPE_NAME_PROPERTY};
use super::quantities::ElementQuantities;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

const M_PER_FT: f64 = 0.3048;
const M2_PER_FT2: f64 = M_PER_FT * M_PER_FT;
const M3_PER_FT3: f64 = M2_PER_FT2 * M_PER_FT;
const KG_PER_LB: f64 = 0.453_592_37;

/// A property value as JSON, in SI units, and its unit.
pub fn json_value(value: &PropertyValue) -> (Value, Option<&'static str>) {
    match value {
        PropertyValue::Text(s) | PropertyValue::Label(s) | PropertyValue::Identifier(s) => {
            (json!(s), None)
        }
        PropertyValue::Integer(n) | PropertyValue::CountValue(n) => (json!(n), None),
        PropertyValue::Real(v) | PropertyValue::Ratio(v) => (number(*v), None),
        PropertyValue::Boolean(b) => (json!(b), None),
        PropertyValue::Logical(b) => (json!(b), None),
        PropertyValue::LengthFeet(v) | PropertyValue::PositiveLengthFeet(v) => {
            (number(v * M_PER_FT), Some("m"))
        }
        PropertyValue::AngleRadians(v) => (number(v.to_degrees()), Some("deg")),
        PropertyValue::AreaSquareFeet(v) => (number(v * M2_PER_FT2), Some("m2")),
        PropertyValue::VolumeCubicFeet(v) => (number(v * M3_PER_FT3), Some("m3")),
        PropertyValue::TimeSeconds(v) => (number(*v), Some("s")),
        PropertyValue::MassPounds(v) => (number(v * KG_PER_LB), Some("kg")),
        PropertyValue::ThermalTransmittance(v) => (number(*v), Some("W/(m2.K)")),
        PropertyValue::VolumetricFlowRate(v) => (number(*v), Some("m3/s")),
        PropertyValue::Pressure(v) => (number(*v), Some("Pa")),
        PropertyValue::Power(v) => (number(*v), Some("W")),
        PropertyValue::ElectricVoltage(v) => (number(*v), Some("V")),
        PropertyValue::ElectricCurrent(v) => (number(*v), Some("A")),
        PropertyValue::List(items) => {
            let unit = items.first().and_then(|i| json_value(i).1);
            (
                Value::Array(items.iter().map(|i| json_value(i).0).collect()),
                unit,
            )
        }
        PropertyValue::Enumerated(items) => (json!(items), None),
        PropertyValue::PhysicalComplex { quantities, .. } => {
            let mut m = Map::new();
            for q in quantities {
                m.insert(q.name.clone(), json_value(&q.value).0);
            }
            (Value::Object(m), None)
        }
    }
}

/// A finite number, else `null`.
fn number(v: f64) -> Value {
    serde_json::Number::from_f64(v).map_or(Value::Null, Value::Number)
}

/// Each element's property sets: its own, then the sets the export attaches
/// to it, by index into `model.entities`.
pub(crate) fn element_sets(model: &IfcModel) -> BTreeMap<usize, Vec<&PropertySet>> {
    let mut sets: BTreeMap<usize, Vec<&PropertySet>> = BTreeMap::new();
    for (i, e) in model.entities.iter().enumerate() {
        if let IfcEntity::BuildingElement {
            property_set: Some(set),
            ..
        } = e
        {
            sets.entry(i).or_default().push(set);
        }
    }
    for e in &model.entities {
        if let IfcEntity::ElementPropertySet { element, set } = e {
            sets.entry(*element).or_default().push(set);
        }
    }
    sets
}

/// The text of property `name` in the first of `sets` named `set` that has
/// it (any set when `set` is `None`).
pub(crate) fn text_in(sets: &[&PropertySet], set: Option<&str>, name: &str) -> Option<String> {
    sets.iter()
        .filter(|s| set.is_none_or(|n| s.name == n))
        .flat_map(|s| s.properties.iter())
        .find(|p| p.name == name)
        .and_then(|p| match &p.value {
            PropertyValue::Text(t) | PropertyValue::Label(t) | PropertyValue::Identifier(t)
                if !t.trim().is_empty() =>
            {
                Some(t.clone())
            }
            _ => None,
        })
}

/// Each element's Revit category (its `Other` or `Revit` set), by index
/// into `model.entities`.
pub fn categories(model: &IfcModel) -> BTreeMap<usize, String> {
    element_sets(model)
        .into_iter()
        .filter_map(|(i, sets)| category_in(&sets).map(|c| (i, c)))
        .collect()
}

/// An element's Revit category (its `Other` or `Revit` set) among its `sets`.
pub(crate) fn category_in(sets: &[&PropertySet]) -> Option<String> {
    text_in(sets, Some(REVIT_OTHER_SET), "Category")
        .or_else(|| text_in(sets, Some(super::revit_identity::SET), "Category"))
}

/// The first value of property `name` that is a length, area or volume, in
/// SI units.
fn measure_in(sets: &[&PropertySet], name: &str) -> Option<f64> {
    sets.iter()
        .flat_map(|s| s.properties.iter())
        .filter(|p| p.name == name)
        .find_map(|p| match p.value {
            PropertyValue::LengthFeet(v) | PropertyValue::PositiveLengthFeet(v) => {
                Some(v * M_PER_FT)
            }
            PropertyValue::AreaSquareFeet(v) => Some(v * M2_PER_FT2),
            PropertyValue::VolumeCubicFeet(v) => Some(v * M3_PER_FT3),
            _ => None,
        })
}

/// One parameter of an element, as the element table gives it.
#[derive(Debug, Clone, PartialEq)]
pub struct ParameterRow {
    pub set: String,
    pub name: String,
    /// In SI units ([`json_value`]).
    pub value: Value,
    pub unit: Option<&'static str>,
    /// `element` (the element's own sets) or `type` (its type's).
    pub of: &'static str,
}

impl ParameterRow {
    /// The value as text: a string as it is, a number or boolean as JSON
    /// writes it, a list or record as JSON; `None` for `null`.
    pub fn value_text(&self) -> Option<String> {
        match &self.value {
            Value::Null => None,
            Value::String(s) => Some(s.clone()),
            other => Some(other.to_string()),
        }
    }

    /// The value as a number, when it is one.
    pub fn value_number(&self) -> Option<f64> {
        self.value.as_f64()
    }
}

/// One row of the element table: one exported element. The fields are the
/// JSON Lines record's ([`elements_jsonl`]), `quantity_source` split into
/// one field per quantity.
#[derive(Debug, Clone, PartialEq)]
pub struct ElementRow {
    pub element_id: Option<u32>,
    pub global_id: Option<String>,
    pub ifc_class: String,
    pub predefined_type: Option<String>,
    pub name: String,
    pub category: Option<String>,
    pub family: Option<String>,
    pub type_name: Option<String>,
    pub type_id: Option<u32>,
    pub level: Option<String>,
    pub level_elevation_m: Option<f64>,
    pub area_m2: Option<f64>,
    pub volume_m3: Option<f64>,
    pub length_m: Option<f64>,
    /// `parameter` or `measured`, where the quantity has a value.
    pub area_source: Option<&'static str>,
    pub volume_source: Option<&'static str>,
    pub length_source: Option<&'static str>,
    /// `measured` or `estimated`, for the measured quantities.
    pub quantity_basis: Option<&'static str>,
    pub parameters: Vec<ParameterRow>,
}

/// A finite number, else `None`.
fn finite(v: f64) -> Option<f64> {
    v.is_finite().then_some(v)
}

/// The rows of the element table of `model`, in the order the export lists
/// the elements. `quantities` are the measured quantities
/// [`crate::convert::export`] returns (empty leaves area, volume and length
/// to the file's own parameters).
pub fn element_rows(model: &IfcModel, quantities: &[ElementQuantities]) -> Vec<ElementRow> {
    let global_ids = super::step_writer::building_element_global_ids(model);
    let sets = element_sets(model);
    let measured: BTreeMap<usize, &ElementQuantities> =
        quantities.iter().map(|q| (q.index, q)).collect();
    let mut rows = Vec::new();
    // A roof's slabs are listed as the roof.
    let roof_slabs = super::roof_slab_wholes(model);
    for (i, entity) in model.entities.iter().enumerate() {
        if roof_slabs.contains_key(&i) {
            continue;
        }
        let IfcEntity::BuildingElement {
            ifc_type,
            name,
            type_guid,
            predefined_type,
            storey_index,
            ..
        } = entity
        else {
            continue;
        };
        let own: Vec<&PropertySet> = sets.get(&i).cloned().unwrap_or_default();
        let element_id: Option<u32> = type_guid.as_deref().and_then(|t| t.parse().ok());
        let type_id = element_id.and_then(|id| model.element_type_ids.get(&id).copied());
        let type_sets: Vec<&PropertySet> = type_id
            .and_then(|t| model.type_property_sets.get(&t))
            .map(|v| v.iter().collect())
            .unwrap_or_default();
        let storey = storey_index.and_then(|s| model.building_storeys.get(s));
        let category = text_in(&own, Some(REVIT_OTHER_SET), "Category")
            .or_else(|| text_in(&own, Some(super::revit_identity::SET), "Category"));
        let family = text_in(&own, Some(REVIT_OTHER_SET), "Family")
            .or_else(|| text_in(&own, None, FAMILY_NAME_PROPERTY))
            .or_else(|| text_in(&type_sets, None, "Family Name"));
        let type_name = text_in(&own, Some(REVIT_OTHER_SET), "Type")
            .or_else(|| text_in(&own, None, TYPE_NAME_PROPERTY))
            .or_else(|| text_in(&type_sets, None, "Type Name"));

        // Revit's own parameters first, then the measured body.
        let q = measured.get(&i).copied();
        let pick = |name: &str, fallback: Option<f64>| -> (Option<f64>, Option<&'static str>) {
            let (v, from) = match measure_in(&own, name) {
                Some(v) => (Some(v), "parameter"),
                None => (fallback, "measured"),
            };
            let v = v.and_then(finite);
            (v, v.map(|_| from))
        };
        let (area_m2, area_source) = pick("Area", None);
        let (volume_m3, volume_source) = pick(
            "Volume",
            q.and_then(|q| q.net_volume_cuft).map(|v| v * M3_PER_FT3),
        );
        let (length_m, length_source) = pick(
            "Length",
            q.and_then(|q| q.length_feet).map(|v| v * M_PER_FT),
        );

        let mut parameters = Vec::new();
        for (of, list) in [("element", &own), ("type", &type_sets)] {
            for set in list.iter() {
                for p in &set.properties {
                    let (value, unit) = json_value(&p.value);
                    parameters.push(ParameterRow {
                        set: set.name.clone(),
                        name: p.name.clone(),
                        value,
                        unit,
                        of,
                    });
                }
            }
        }

        rows.push(ElementRow {
            element_id,
            global_id: global_ids.get(i).cloned().flatten(),
            ifc_class: super::ifcx_writer::ifc_class_name(ifc_type),
            predefined_type: predefined_type.clone(),
            name: name.clone(),
            category,
            family,
            type_name,
            type_id,
            level: storey.map(|s| s.name.clone()),
            level_elevation_m: storey.and_then(|s| finite(s.elevation_feet * M_PER_FT)),
            area_m2,
            volume_m3,
            length_m,
            area_source,
            volume_source,
            length_source,
            quantity_basis: q.map(|q| match q.basis {
                super::quantities::Basis::Measured => "measured",
                super::quantities::Basis::Estimated => "estimated",
            }),
            parameters,
        });
    }
    // Each element that only cuts openings (a shaft opening, a rectangular
    // wall opening), once, as its first opening: it has no body of its own.
    let mut listed: std::collections::BTreeSet<u32> =
        rows.iter().filter_map(|row| row.element_id).collect();
    for entity in &model.entities {
        let (tag, name) = match entity {
            IfcEntity::VoidOpening { tag, name, .. }
            | IfcEntity::WallProfileOpening { tag, name, .. }
            | IfcEntity::WallCutOpening { tag, name, .. }
            | IfcEntity::HostedCutOpening { tag, name, .. } => (tag, name),
            _ => continue,
        };
        let Some((id, category)) = tag
            .parse::<u32>()
            .ok()
            .and_then(|id| Some((id, model.opening_categories.get(&id)?)))
        else {
            continue;
        };
        let Some(global_id) = super::step_writer::opening_global_id(model, &global_ids, entity)
        else {
            continue;
        };
        if !listed.insert(id) {
            continue;
        }
        rows.push(ElementRow {
            element_id: Some(id),
            global_id: Some(global_id),
            ifc_class: "IfcOpeningElement".into(),
            predefined_type: Some("OPENING".into()),
            name: name.clone(),
            category: Some(category.clone()),
            family: None,
            type_name: None,
            type_id: None,
            level: None,
            level_elevation_m: None,
            area_m2: None,
            volume_m3: None,
            length_m: None,
            area_source: None,
            volume_source: None,
            length_source: None,
            quantity_basis: None,
            parameters: Vec::new(),
        });
    }
    // Each grid line, as a row of the `IfcGrid` that holds it.
    if super::grids::axes(model).is_some() && !model.building_storeys.is_empty() {
        let global_id = super::step_writer::grid_global_id(model);
        let storey = model.building_storeys.first();
        let category = crate::builtin_parameters::category_name_and_parent(super::grids::OST_GRIDS)
            .map(|(name, _)| name.to_string());
        for line in &model.grids {
            let length = (line.end[0] - line.start[0]).hypot(line.end[1] - line.start[1]);
            rows.push(ElementRow {
                element_id: Some(line.id),
                global_id: Some(global_id.clone()),
                ifc_class: "IfcGrid".into(),
                predefined_type: None,
                name: line.name.clone(),
                category: category.clone(),
                family: None,
                type_name: None,
                type_id: None,
                level: storey.map(|s| s.name.clone()),
                level_elevation_m: storey.and_then(|s| finite(s.elevation_feet * M_PER_FT)),
                area_m2: None,
                volume_m3: None,
                length_m: finite(length * M_PER_FT),
                area_source: None,
                volume_source: None,
                length_source: Some("measured"),
                quantity_basis: None,
                parameters: Vec::new(),
            });
        }
    }
    rows
}

/// One row as its JSON Lines record.
pub fn row_json(row: &ElementRow) -> Value {
    let mut quantity_source = Map::new();
    for (key, source) in [
        ("area", row.area_source),
        ("volume", row.volume_source),
        ("length", row.length_source),
    ] {
        if let Some(s) = source {
            quantity_source.insert(key.into(), json!(s));
        }
    }
    let parameters = row
        .parameters
        .iter()
        .map(|p| {
            let mut m = Map::new();
            m.insert("set".into(), json!(p.set));
            m.insert("name".into(), json!(p.name));
            m.insert("value".into(), p.value.clone());
            m.insert("unit".into(), json!(p.unit));
            m.insert("of".into(), json!(p.of));
            Value::Object(m)
        })
        .collect();
    let num = |v: Option<f64>| v.map_or(Value::Null, number);
    let mut r = Map::new();
    r.insert("element_id".into(), json!(row.element_id));
    r.insert("global_id".into(), json!(row.global_id));
    r.insert("ifc_class".into(), json!(row.ifc_class));
    r.insert("predefined_type".into(), json!(row.predefined_type));
    r.insert("name".into(), json!(row.name));
    r.insert("category".into(), json!(row.category));
    r.insert("family".into(), json!(row.family));
    r.insert("type".into(), json!(row.type_name));
    r.insert("type_id".into(), json!(row.type_id));
    r.insert("level".into(), json!(row.level));
    r.insert("level_elevation_m".into(), num(row.level_elevation_m));
    r.insert("area_m2".into(), num(row.area_m2));
    r.insert("volume_m3".into(), num(row.volume_m3));
    r.insert("length_m".into(), num(row.length_m));
    r.insert("quantity_source".into(), Value::Object(quantity_source));
    r.insert("quantity_basis".into(), json!(row.quantity_basis));
    r.insert("parameters".into(), Value::Array(parameters));
    Value::Object(r)
}

/// The element table of `model` as JSON Lines. `quantities` are the
/// measured quantities [`crate::convert::export`] returns (empty leaves
/// area, volume and length to the file's own parameters).
pub fn elements_jsonl(model: &IfcModel, quantities: &[ElementQuantities]) -> String {
    rows_jsonl(&element_rows(model, quantities))
}

/// `rows` as JSON Lines: one record per line, each ending with a newline.
pub fn rows_jsonl(rows: &[ElementRow]) -> String {
    let mut out = String::new();
    for row in rows {
        out.push_str(&row_json(row).to_string());
        out.push('\n');
    }
    out
}

/// How many elements of each Revit category (its IFC class where it has
/// none) `model` holds.
pub fn count_by_category(model: &IfcModel) -> BTreeMap<String, usize> {
    let categories = categories(model);
    let mut out = BTreeMap::new();
    for (i, e) in model.entities.iter().enumerate() {
        if let IfcEntity::BuildingElement { ifc_type, .. } = e {
            let key = categories
                .get(&i)
                .cloned()
                .unwrap_or_else(|| super::ifcx_writer::ifc_class_name(ifc_type));
            *out.entry(key).or_insert(0) += 1;
        }
    }
    out
}
