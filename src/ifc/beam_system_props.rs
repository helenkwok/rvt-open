//! The parameters Revit's IFC export writes on a Structural Beam System
//! assembly, under its property-set and property names:
//! `Pattern`, `Constraints`, `Identity Data` and the type's `Other(Type)` and
//! `Identity Data(Type)`. Each value comes from the system's layout record
//! ([`crate::beam_system_layout`]) or its member beams, and each is written
//! only from a value the file states: a missing one leaves the property out.

use super::entities::{Property, PropertySet, PropertyValue};
use crate::beam_reference_levels::BeamSystems;
use std::collections::BTreeMap;

/// First release whose API has the Work Plane, Reference Level Elevation and
/// Elevation from Level parameters on a beam system (the API dump of 2018 and
/// 2019 has none of them).
const WORK_PLANE_FIRST_RELEASE: u32 = 2022;

/// First release whose API has a Family Name parameter on a beam system type
/// (the API dump of 2018 and 2019 has none).
const FAMILY_NAME_FIRST_RELEASE: u32 = 2022;

/// First release whose API has no "3D" parameter on a beam system (the API
/// dump of 2026 and 2027 has none).
const THREE_D_LAST_RELEASE: u32 = 2025;

/// Revit's family name of the beam system category's one family.
const FAMILY_NAME: &str = "Structural Beam System";

/// Name of a beam type by ElementId, as `(family, type)`.
pub(crate) type BeamTypeNames = BTreeMap<u32, (String, String)>;

/// A point on one member beam's axis and the direction of the axis in plan.
pub(crate) type MemberAxis = ([f64; 3], [f64; 2]);

fn label(name: &str, value: impl Into<String>) -> Property {
    Property {
        name: name.into(),
        value: PropertyValue::Label(value.into()),
    }
}

fn text(name: &str, value: impl Into<String>) -> Property {
    Property {
        name: name.into(),
        value: PropertyValue::Text(value.into()),
    }
}

fn identifier(name: &str, value: impl Into<String>) -> Property {
    Property {
        name: name.into(),
        value: PropertyValue::Identifier(value.into()),
    }
}

fn length(name: &str, feet: f64) -> Property {
    Property {
        name: name.into(),
        value: PropertyValue::LengthFeet(feet),
    }
}

fn set(name: &str, properties: Vec<Property>) -> Option<PropertySet> {
    (!properties.is_empty()).then(|| PropertySet {
        name: name.into(),
        properties,
    })
}

/// Distance between neighbouring member beams, in feet: the axes are parallel
/// and evenly spaced, so it is the span of their positions across the axis over
/// the gaps. `None` unless there are two members, their axes are parallel in
/// plan and every gap is equal.
pub(crate) fn centerline_spacing_feet(members: &[MemberAxis]) -> Option<f64> {
    let [(_, first), ..] = members else {
        return None;
    };
    let length = first[0].hypot(first[1]);
    if !length.is_finite() || length <= 1e-9 {
        return None;
    }
    let across = [-first[1] / length, first[0] / length];
    let parallel = members.iter().all(|(_, d)| {
        let l = d[0].hypot(d[1]);
        l > 1e-9 && (d[0] * across[0] + d[1] * across[1]).abs() / l <= 1e-9
    });
    if !parallel {
        return None;
    }
    let mut offsets: Vec<f64> = members
        .iter()
        .map(|(at, _)| at[0] * across[0] + at[1] * across[1])
        .collect();
    offsets.sort_by(f64::total_cmp);
    let gaps = offsets.len().checked_sub(1).filter(|n| *n >= 1)?;
    let spacing = (offsets[gaps] - offsets[0]) / gaps as f64;
    offsets
        .windows(2)
        .all(|w| ((w[1] - w[0]) - spacing).abs() <= 1e-6)
        .then_some(spacing)
}

/// Distance between neighbouring member beams, in feet, from their outline
/// boxes: where every beam runs along the x axis (or every one along y) the
/// centre of its box across the beam is where its axis is, whatever the
/// section and cut ends. `None` unless every box is that long and thin, there
/// are two or more, and the gaps are equal.
pub(crate) fn centerline_spacing_from_boxes(boxes: &[[f64; 6]]) -> Option<f64> {
    if boxes.len() < 2 {
        return None;
    }
    let extent = |b: &[f64; 6]| [b[3] - b[0], b[4] - b[1]];
    let along_x = boxes.iter().all(|b| {
        let [dx, dy] = extent(b);
        dx >= 3.0 * dy
    });
    let along_y = boxes.iter().all(|b| {
        let [dx, dy] = extent(b);
        dy >= 3.0 * dx
    });
    let across = match (along_x, along_y) {
        (true, false) => 1,
        (false, true) => 0,
        _ => return None,
    };
    let mut offsets: Vec<f64> = boxes
        .iter()
        .map(|b| (b[across] + b[across + 3]) / 2.0)
        .collect();
    offsets.sort_by(f64::total_cmp);
    let gaps = offsets.len() - 1;
    let spacing = (offsets[gaps] - offsets[0]) / gaps as f64;
    offsets
        .windows(2)
        .all(|w| ((w[1] - w[0]) - spacing).abs() <= 1e-6)
        .then_some(spacing)
}

/// Centerline Spacing of a system of one member beam, in feet: the beam sits
/// midway between the two edges of the Sketch, so the spacing is half the
/// Sketch's extent across the beam. Only where the beam runs along the x axis
/// or the y axis, and then the system's outline box is the Sketch's extent
/// (`outline`, `[min_x, min_y, min_z, max_x, max_y, max_z]`).
pub(crate) fn centerline_spacing_single_member(
    outline: Option<[f64; 6]>,
    boxes: &[[f64; 6]],
) -> Option<f64> {
    let ([b], Some(o)) = (boxes, outline) else {
        return None;
    };
    let (dx, dy) = (b[3] - b[0], b[4] - b[1]);
    if dx >= 3.0 * dy {
        Some((o[4] - o[1]) / 2.0)
    } else if dy >= 3.0 * dx {
        Some((o[3] - o[0]) / 2.0)
    } else {
        None
    }
}

/// The property sets of the beam system `system_id`.
pub(crate) fn property_sets(
    system_id: u32,
    beam_systems: &BeamSystems,
    beam_types: &BeamTypeNames,
    members: &[MemberAxis],
) -> Vec<PropertySet> {
    let revit_version = beam_systems.revit_version();
    let mut out = Vec::new();
    // The type's own parameters, which Revit writes on the instance under a
    // `(Type)` name: its category, its family name (a parameter of the type
    // from the 2022 API on) and its type name.
    let mut type_set = vec![label("Category", "Structural Beam Systems")];
    if revit_version >= FAMILY_NAME_FIRST_RELEASE {
        type_set.push(text("Family Name", FAMILY_NAME));
    }
    out.extend(set("Other(Type)", type_set));
    let mut type_properties = Vec::new();
    if let Some((_, name)) = beam_systems.type_names(system_id) {
        type_properties.push(text("Type Name", name.clone()));
    }
    if let Some(description) = beam_systems.type_description(system_id) {
        type_properties.push(text("Description", description));
    }
    out.extend(set("Identity Data(Type)", type_properties));
    let Some(layout) = beam_systems.layout(system_id) else {
        return out;
    };
    let mut pattern = vec![
        identifier("Layout Rule", layout.rule_name()),
        identifier("Justification", layout.justification_name()),
    ];
    match layout.beam_type_id {
        None => {
            pattern.push(label("Beam Type", "Default"));
            pattern.push(label("Beam Type (No Family Name)", "Default"));
        }
        Some(id) => {
            if let Some((family, name)) = beam_types.get(&id) {
                pattern.push(label("Beam Type", format!("{family} : {name}")));
                pattern.push(label("Beam Type (No Family Name)", name.clone()));
            }
        }
    }
    let centerline = if layout.rule == 1 {
        Some(layout.spacing_feet)
    } else {
        centerline_spacing_feet(members)
            .or_else(|| centerline_spacing_from_boxes(beam_systems.member_boxes(system_id)))
            .or_else(|| {
                centerline_spacing_single_member(
                    beam_systems.outline_box(system_id),
                    beam_systems.member_boxes(system_id),
                )
            })
    };
    if let Some(feet) = centerline {
        pattern.push(length("Centerline Spacing", feet));
    }
    if layout.rule == 1 {
        pattern.push(length("Fixed Spacing", layout.spacing_feet));
    } else {
        pattern.push(length("Maximum Spacing", layout.maximum_spacing_feet));
    }
    out.extend(set("Pattern", pattern));

    let mut constraints = Vec::new();
    if revit_version <= THREE_D_LAST_RELEASE {
        constraints.push(Property {
            name: "3D".into(),
            value: PropertyValue::Boolean(layout.three_d),
        });
    }
    if revit_version >= WORK_PLANE_FIRST_RELEASE {
        // Elevation from Level is there on a Level's plane, and on another
        // plane only when it is not zero.
        match beam_systems.work_plane(system_id) {
            Some(Some((name, elevation))) => {
                constraints.push(length("Elevation from Level", layout.elevation_feet));
                constraints.push(length("Reference Level Elevation", elevation));
                constraints.push(text("Work Plane", format!("Level : {name}")));
            }
            Some(None) => {
                if layout.elevation_feet != 0.0 {
                    constraints.push(length("Elevation from Level", layout.elevation_feet));
                }
                constraints.push(text("Work Plane", "<not associated>"));
            }
            None => {}
        }
    }
    out.extend(set("Constraints", constraints));

    let mut identity = Vec::new();
    if let Some(comments) = &layout.comments {
        identity.push(text("Comments", comments.clone()));
    }
    out.extend(set("Identity Data", identity));
    out
}
