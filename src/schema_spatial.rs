//! Rooms', areas', MEP spaces' and HVAC zones' stored facts, read through
//!
//! Rooms, areas and MEP spaces are all `RoomElem` objects. Measured on the
//! 12 Nordic LCA models against the Revit API:
//! - `m_areaSchemeId` names an `AreaMeasureElem` (an area scheme) whose
//!   `m_eAreaType` is 0 for the Gross Building scheme: 48 of 48 areas whose
//!   API "Area Type" is Gross Building Area, and none of the other 152;
//! - `m_upperLevelId`, `m_upperOffset`, `m_lowerOffset` are the API's Upper
//!   Limit (1,291 of 1,296; the 5 others unplaced), Limit Offset and Base
//!   Offset (1,296 of 1,296 each, feet);
//! - `m_volumeBoundingElems` the elements bounding the room's volume;
//! - `m_zoneElementId` the HVAC zone (`ZoneElement`) a space is in, and a
//!   zone's `m_spaceIds` its spaces; `m_bDefault` marks each model's Default
//!   zone.

use crate::schema_objects::{Fields, ParamValue, Value, for_each_object_where};
use crate::{Result, RevitFile};
use std::collections::{BTreeMap, BTreeSet};

/// Autodesk `BuiltInParameter.ZONE_NAME`.
const ZONE_NAME: i64 = -1_114_300;

/// A room's, area's or MEP space's stored facts.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SpatialElement {
    /// The area scheme, for an area.
    pub area_scheme: Option<i64>,
    /// Upper Limit (a Level), Limit Offset and Base Offset, feet.
    pub upper_level: Option<i64>,
    pub upper_offset: Option<f64>,
    pub lower_offset: Option<f64>,
    /// The elements bounding its volume.
    pub bounding: Vec<i64>,
    /// The HVAC zone it is in.
    pub zone: Option<i64>,
}

/// An HVAC zone.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Zone {
    pub id: i64,
    pub name: Option<String>,
    /// The model's Default zone, which holds every space not zoned.
    pub default: bool,
    pub spaces: Vec<i64>,
}

/// Everything [`read`] finds.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SpatialFacts {
    pub elements: BTreeMap<i64, SpatialElement>,
    /// Area schemes whose type is Gross Building (`m_eAreaType` 0).
    pub gross_schemes: BTreeSet<i64>,
    pub zones: Vec<Zone>,
}

impl SpatialFacts {
    /// True when `area` lies in a Gross Building area scheme.
    pub fn is_gross_area(&self, area: i64) -> bool {
        self.elements
            .get(&area)
            .and_then(|e| e.area_scheme)
            .is_some_and(|s| self.gross_schemes.contains(&s))
    }
}

fn id(f: &Fields, name: &str) -> Option<i64> {
    match f.get(name)? {
        Value::Id(i) if *i >= 0 => Some(*i),
        _ => None,
    }
}

fn float(f: &Fields, name: &str) -> Option<f64> {
    match f.get(name)? {
        Value::Float(v) => Some(*v),
        _ => None,
    }
}

fn ids(f: &Fields, name: &str) -> Vec<i64> {
    let Some(Value::List(items)) = f.get(name) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in items {
        match item {
            Value::Id(i) => out.push(*i),
            Value::Object(o) => out.extend(o.values.iter().filter_map(|(_, v)| match v {
                Value::Id(i) => Some(*i),
                _ => None,
            })),
            _ => {}
        }
    }
    out.retain(|i| *i >= 0);
    out
}

/// Read every `RoomElem`, `AreaMeasureElem` and `ZoneElement` of `rf`, one
/// object in memory at a time.
pub fn read(rf: &mut RevitFile) -> Result<SpatialFacts> {
    let layouts = rf.schema_layouts()?;
    let wanted: BTreeSet<u16> = layouts
        .classes
        .iter()
        .filter(|(_, c)| {
            matches!(
                c.name.as_str(),
                "RoomElem" | "AreaMeasureElem" | "ZoneElement"
            )
        })
        .map(|(t, _)| *t)
        .collect();
    let mut out = SpatialFacts::default();
    for_each_object_where(
        rf,
        |f, _| wanted.contains(&f.header.tag),
        |o| {
            let (Some(element), Some(root)) = (o.element(), o.root()) else {
                return;
            };
            let class = layouts
                .classes
                .get(&o.frame.header.tag)
                .map(|c| c.name.as_str())
                .unwrap_or("");
            match class {
                "RoomElem" => {
                    out.elements.insert(
                        element,
                        SpatialElement {
                            area_scheme: id(root, "m_areaSchemeId"),
                            upper_level: id(root, "m_upperLevelId"),
                            upper_offset: float(root, "m_upperOffset"),
                            lower_offset: float(root, "m_lowerOffset"),
                            bounding: ids(root, "m_volumeBoundingElems"),
                            zone: id(root, "m_zoneElementId"),
                        },
                    );
                }
                "AreaMeasureElem" => {
                    if matches!(root.get("m_eAreaType"), Some(Value::Int(0))) {
                        out.gross_schemes.insert(element);
                    }
                }
                "ZoneElement" => {
                    let name = o
                        .stored_parameters()
                        .into_iter()
                        .find_map(|(p, v)| match v {
                            ParamValue::String(s) if p == ZONE_NAME => Some(s),
                            _ => None,
                        });
                    out.zones.push(Zone {
                        id: element,
                        name,
                        default: matches!(root.get("m_bDefault"), Some(Value::Bool(true))),
                        spaces: ids(root, "m_spaceIds"),
                    });
                }
                _ => {}
            }
        },
    )?;
    Ok(out)
}

/// The elements of `ids` whose data object names no Level
/// (`m_assocLevelId` unset). A family instance hosted on a face has none,
/// and Revit's exporter draws no plan symbol (`FootPrint`) for it: it reads
/// the symbol's curves in the plan view of the instance's Level (Revit's IFC exporter
/// On the 12 Nordic LCA models, the 91 light fixtures and 129 plumbing
/// fixtures we drew a `FootPrint` for and Revit did not all have no Level
/// in the API.
pub fn levelless(rf: &mut RevitFile, ids: &BTreeSet<u32>) -> Result<BTreeSet<u32>> {
    let mut out = BTreeSet::new();
    let keep = |f: &crate::schema_objects::Frame, _: &[u8]| {
        u32::try_from(f.header.id).is_ok_and(|id| ids.contains(&id))
    };
    for_each_object_where(rf, keep, |o| {
        let (Some(element), Some(root)) = (o.element(), o.root()) else {
            return;
        };
        if matches!(root.get("m_assocLevelId"), Some(Value::Id(level)) if *level < 0) {
            if let Ok(id) = u32::try_from(element) {
                out.insert(id);
            }
        }
    })?;
    Ok(out)
}
