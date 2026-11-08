//! Wall lengths, tee reaches, butt joins and join trims.

use super::*;

/// Where each layer of a wall ends at its layered butt joins.
#[derive(Debug, Clone, PartialEq)]
pub struct WallLayerEnds {
    /// Plan unit vector towards the wall's exterior face.
    pub exterior: [f64; 2],
    /// The layers' widths, exterior first, feet.
    pub widths: Vec<f64>,
    /// How far past its start and its end each layer reaches along its
    /// exterior-side and its interior-side edge, exterior first, where a
    /// layered or angled join decides it. The two are equal where the end
    /// is square.
    pub reach_feet: [Option<Vec<[f64; 2]>>; 2],
}

/// The [`WALL_JOIN_LAYER_EDGE_REACH_FIELDS`] or
/// [`WALL_JOIN_LAYER_REACH_FIELDS`] a wall carries, with its layers' widths
/// and exterior side. `None` without a layered or angled join at either
/// end, or where a reach list does not match the layers.
pub fn wall_layer_ends_from_fields(fields: &[(String, InstanceField)]) -> Option<WallLayerEnds> {
    let floats = |wanted: &str| {
        fields.iter().find_map(|(name, value)| match value {
            InstanceField::Vector(items) if name == wanted => items
                .iter()
                .map(|item| match item {
                    InstanceField::Float { value, .. } => Some(*value),
                    _ => None,
                })
                .collect::<Option<Vec<f64>>>(),
            _ => None,
        })
    };
    let square = WALL_JOIN_LAYER_REACH_FIELDS.map(floats);
    let slanted = WALL_JOIN_LAYER_EDGE_REACH_FIELDS.map(floats);
    if square.iter().chain(&slanted).all(Option::is_none) {
        return None;
    }
    let widths: Vec<f64> = fields
        .iter()
        .find_map(|(name, value)| {
            (name == WALL_LAYERS_FIELD).then(|| layer_bands_from_field(value))
        })
        .flatten()?
        .iter()
        .map(|band| band.width_feet)
        .collect();
    if square
        .iter()
        .flatten()
        .any(|reach| reach.len() != widths.len())
        || slanted
            .iter()
            .flatten()
            .any(|reach| reach.len() != 2 * widths.len())
    {
        return None;
    }
    let [start, end] = [0, 1].map(|slot| match (&slanted[slot], &square[slot]) {
        (Some(edges), _) => Some(edges.chunks(2).map(|pair| [pair[0], pair[1]]).collect()),
        (None, Some(reaches)) => Some(reaches.iter().map(|reach| [*reach, *reach]).collect()),
        (None, None) => None,
    });
    let reach_feet = [start, end];
    let float = |wanted: &str| {
        fields.iter().find_map(|(name, value)| match value {
            InstanceField::Float { value, .. } if name == wanted => Some(*value),
            _ => None,
        })
    };
    let [Some(ex), Some(ey)] = WALL_EXTERIOR_FIELDS.map(float) else {
        return None;
    };
    Some(WallLayerEnds {
        exterior: [ex, ey],
        widths,
        reach_feet,
    })
}

/// Give each wall with a centreline whose native curve, cut back where its
/// body meets the walls joined at its ends, is as long as the curve, that
/// length ([`WALL_LENGTH_FIELD`]): the centreline's length, which Revit's
/// export writes as its `Length` whether it measures the curve or the cut
/// back curve. The reach each end's join lists give
/// ([`WALL_JOIN_REACH_FIELDS`], or the largest of its layers' where they end
/// apart) is the second reading the record's must agree with.
pub(super) fn attach_wall_lengths(
    walls: &mut [DecodedElement],
    native: &BTreeMap<u64, crate::native_walls::NativeWall>,
) {
    const SAME_LENGTH_FEET: f64 = 1e-4;
    let joined = crate::native_walls::joined_at_ends(native);
    let near = |a: [f64; 2], b: [f64; 2]| {
        (a[0] - b[0]).hypot(a[1] - b[1]) <= NATIVE_WALL_LINE_TOLERANCE_FEET
    };
    for wall in walls.iter_mut() {
        let Some(id) = wall.id.map(u64::from) else {
            continue;
        };
        if wall.class == CURTAIN_WALL_CLASS {
            if let Some(curve) = native.get(&id) {
                let length = (curve.end[0] - curve.start[0]).hypot(curve.end[1] - curve.start[1]);
                if length.is_finite() && length > 0.0 {
                    wall.fields.push((
                        WALL_LENGTH_FIELD.into(),
                        InstanceField::Float {
                            value: length,
                            size: 8,
                        },
                    ));
                }
            }
            continue;
        }
        let (Some(axis), Some(curve)) = (wall_axis_from_fields(&wall.fields), native.get(&id))
        else {
            continue;
        };
        let ends = if near(curve.start, axis.start) && near(curve.end, axis.end) {
            [0, 1]
        } else if near(curve.start, axis.end) && near(curve.end, axis.start) {
            [1, 0]
        } else {
            continue;
        };
        let reach_of = |slot: usize| {
            wall.fields.iter().find_map(|(name, value)| match value {
                InstanceField::Float { value, .. } if name == WALL_JOIN_REACH_FIELDS[slot] => {
                    Some(*value)
                }
                _ => None,
            })
        };
        let layer_reach_of = |slot: usize| {
            wall.fields.iter().find_map(|(name, value)| match value {
                InstanceField::Vector(items) if name == WALL_JOIN_LAYER_REACH_FIELDS[slot] => items
                    .iter()
                    .map(|item| match item {
                        InstanceField::Float { value, .. } => Some(*value),
                        _ => None,
                    })
                    .collect::<Option<Vec<f64>>>()?
                    .into_iter()
                    .max_by(|a, b| a.abs().total_cmp(&b.abs())),
                _ => None,
            })
        };
        let mut reaches = [None, None];
        for (slot, end) in ends.into_iter().enumerate() {
            reaches[end] = reach_of(slot).or_else(|| layer_reach_of(slot));
        }
        let curve_length = (curve.end[0] - curve.start[0]).hypot(curve.end[1] - curve.start[1]);
        if curve_length.is_finite() && curve_length > 0.0 {
            wall.fields.push((
                WALL_CURVE_LENGTH_FIELD.into(),
                InstanceField::Float {
                    value: curve_length,
                    size: 8,
                },
            ));
        }
        let Some([start, end]) = crate::native_walls::trimmed_ends(id, native, &joined, reaches)
        else {
            continue;
        };
        let cutback = end - start;
        if cutback.is_finite() && cutback > 0.0 {
            wall.fields.push((
                WALL_CUTBACK_LENGTH_FIELD.into(),
                InstanceField::Float {
                    value: cutback,
                    size: 8,
                },
            ));
        }
        let length = (curve.end[0] - curve.start[0]).hypot(curve.end[1] - curve.start[1]);
        if ((end - start) - length).abs() <= SAME_LENGTH_FEET && length.is_finite() && length > 0.0
        {
            wall.fields.push((
                WALL_LENGTH_FIELD.into(),
                InstanceField::Float {
                    value: length,
                    size: 8,
                },
            ));
        }
    }
}

/// At each end of a wall with a centreline where its native record says
/// another wall meets that end with its middle, the body stops half that
/// wall's thickness short ([`crate::native_walls::tee_reach`]),
/// in place of the reach its join lists give. An end whose layers each end
/// on their own line keeps them, and so does one whose other
/// wall's thickness is not known: its type's, or its record box's thin
/// extent where it runs along a model axis.
pub(super) fn attach_native_tee_reaches(
    walls: &mut [DecodedElement],
    native: &BTreeMap<u64, crate::native_walls::NativeWall>,
) {
    // A wall's type thickness, or, for a wall along a model axis, its record
    // box's thin extent, which is its thickness there.
    let along_axis = |id: u64| {
        native.get(&id).is_some_and(|curve| {
            let (dx, dy) = (curve.end[0] - curve.start[0], curve.end[1] - curve.start[1]);
            dx.abs() <= 1e-9 || dy.abs() <= 1e-9
        })
    };
    let thickness: BTreeMap<u64, f64> = walls
        .iter()
        .filter_map(|wall| {
            let id = u64::from(wall.id?);
            let float = |wanted: &str| {
                wall.fields.iter().find_map(|(name, value)| match value {
                    InstanceField::Float { value, .. } if name == wanted => Some(*value),
                    _ => None,
                })
            };
            let thickness = float(WALL_TYPE_THICKNESS_FIELD).or_else(|| {
                float(crate::element_record_wall_joins::WALL_THICKNESS_FIELD)
                    .filter(|_| along_axis(id))
            })?;
            Some((id, thickness))
        })
        .collect();
    let near = |a: [f64; 2], b: [f64; 2]| {
        (a[0] - b[0]).hypot(a[1] - b[1]) <= NATIVE_WALL_LINE_TOLERANCE_FEET
    };
    for wall in walls.iter_mut() {
        let Some(id) = wall.id.map(u64::from) else {
            continue;
        };
        let (Some(axis), Some(curve)) = (wall_axis_from_fields(&wall.fields), native.get(&id))
        else {
            continue;
        };
        let ends = if near(curve.start, axis.start) && near(curve.end, axis.end) {
            [0, 1]
        } else if near(curve.start, axis.end) && near(curve.end, axis.start) {
            [1, 0]
        } else {
            continue;
        };
        for (slot, end) in ends.into_iter().enumerate() {
            let layered = [
                WALL_JOIN_LAYER_REACH_FIELDS[slot],
                WALL_JOIN_LAYER_EDGE_REACH_FIELDS[slot],
            ];
            if wall
                .fields
                .iter()
                .any(|(name, _)| layered.contains(&name.as_str()))
            {
                continue;
            }
            if let Some(reach) = crate::native_walls::tee_reach(id, end, native, |other| {
                thickness.get(&other).copied()
            }) {
                wall.fields
                    .retain(|(name, _)| name != WALL_JOIN_REACH_FIELDS[slot]);
                wall.fields.push((
                    WALL_JOIN_REACH_FIELDS[slot].into(),
                    InstanceField::Float {
                        value: reach,
                        size: 8,
                    },
                ));
            }
        }
    }
}

/// Each wall with a centreline, with its type's layers, its exterior side and
/// its elevation range, which [`crate::element_record_wall_joins::butt_joins`]
/// and [`crate::element_record_wall_joins::corner_joins`] read.
fn wall_lines(
    walls: &[DecodedElement],
    type_layers: &BTreeMap<u32, Vec<crate::partition_compound_structure::CompoundLayer>>,
    native: &BTreeMap<u64, crate::native_walls::NativeWall>,
) -> Vec<crate::element_record_wall_joins::WallLine> {
    use crate::element_record_wall_joins as joins;
    let float = |wall: &DecodedElement, wanted: &str| {
        wall.fields.iter().find_map(|(name, value)| match value {
            InstanceField::Float { value, .. } if name == wanted => Some(*value),
            _ => None,
        })
    };
    walls
        .iter()
        .filter_map(|wall| {
            let axis = wall_axis_from_fields(&wall.fields)?;
            let base = float(wall, "m_locationZ")?;
            let type_id = wall.fields.iter().find_map(|(name, value)| match value {
                InstanceField::ElementId { id, .. } if name == TYPE_ID_FIELD => Some(*id),
                _ => None,
            })?;
            let layers: Vec<(f64, u32)> = type_layers
                .get(&type_id)?
                .iter()
                .filter(|layer| layer.width_feet > 0.0)
                .map(|layer| (layer.width_feet, layer.function))
                .collect();
            let [Some(ex), Some(ey)] = WALL_EXTERIOR_FIELDS.map(|name| float(wall, name)) else {
                return None;
            };
            Some(joins::WallLine {
                element_id: wall.id?,
                start: axis.start,
                end: axis.end,
                thickness_feet: axis.thickness_feet,
                layers,
                exterior: [ex, ey],
                base_feet: base,
                top_feet: base + float(wall, "m_bboxHeight")?,
                shells: native
                    .get(&u64::from(wall.id?))
                    .and_then(|native| native.shells),
            })
        })
        .collect()
}

/// Where three or more walls meet end to end at a butt join, the end of each
/// wall the record's join order decides
/// ([`crate::element_record_wall_joins::corner_joins`]): how far past
/// its line's end the body reaches, every layer alike, in place of the reach a
/// join list or a T joint gave that end. It names the wall it stops at or runs
/// through only where one perpendicular wall alone decides it; at a corner
/// where several do, or where it meets a wall that continues it flush, there
/// is no one partner and none is named. Two single-layer walls that meet at a
/// mitre get the slanted end it makes, along their exterior-side and
/// interior-side edges, and no partner.
pub(super) fn attach_wall_corner_reaches(
    walls: &mut [DecodedElement],
    type_layers: &BTreeMap<u32, Vec<crate::partition_compound_structure::CompoundLayer>>,
    native: &BTreeMap<u64, crate::native_walls::NativeWall>,
) {
    use crate::element_record_wall_joins as joins;
    let lines = wall_lines(walls, type_layers, native);
    if lines.len() < 3 {
        return;
    }
    let near = |a: [f64; 2], b: [f64; 2]| {
        (a[0] - b[0]).hypot(a[1] - b[1]) <= NATIVE_WALL_LINE_TOLERANCE_FEET
    };
    // The record's joins are indexed by the end of its own curve, the
    // reaches by the end of the wall's axis: which of the two orders each
    // wall's ends are in. A wall whose curve is not its axis, either way
    // round, is left out, and so is any corner it is a member of.
    let reversed: BTreeMap<u32, bool> = lines
        .iter()
        .filter_map(|line| {
            let curve = native.get(&u64::from(line.element_id))?;
            if near(curve.start, line.start) && near(curve.end, line.end) {
                Some((line.element_id, false))
            } else if near(curve.start, line.end) && near(curve.end, line.start) {
                Some((line.element_id, true))
            } else {
                None
            }
        })
        .collect();
    let by_axis_end = |id: u32, end: u32| -> Option<u32> {
        if end > 1 {
            return Some(end);
        }
        Some(if *reversed.get(&id)? { 1 - end } else { end })
    };
    let narrow = |join: &Option<Vec<(u64, u32)>>| -> Option<Vec<(u32, u32)>> {
        join.as_ref()?
            .iter()
            .map(|&(id, end)| {
                let id = u32::try_from(id).ok()?;
                Some((id, by_axis_end(id, end)?))
            })
            .collect()
    };
    let ends: BTreeMap<u32, joins::NativeEnds> = native
        .iter()
        .filter_map(|(&id, wall)| {
            let id = u32::try_from(id).ok()?;
            let flipped = *reversed.get(&id)?;
            let (first, second) = if flipped { (1, 0) } else { (0, 1) };
            Some((
                id,
                joins::NativeEnds {
                    members: [narrow(&wall.joins[first]), narrow(&wall.joins[second])],
                    join_types: [wall.join_types[first], wall.join_types[second]],
                    other_cutters: wall
                        .partners
                        .iter()
                        .any(|other| !native.contains_key(other)),
                },
            ))
        })
        .collect();
    let found = joins::corner_joins(&lines, &ends);
    let layer_counts: BTreeMap<u32, usize> = lines
        .iter()
        .map(|line| (line.element_id, line.layers.len()))
        .collect();
    for wall in walls.iter_mut() {
        let Some((id, reaches)) = wall.id.and_then(|id| Some((id, found.get(&id)?))) else {
            continue;
        };
        for (slot, cut) in reaches.iter().enumerate() {
            let Some(cut) = cut.clone() else {
                continue;
            };
            let replaced = [
                WALL_JOIN_PARTNER_FIELDS[slot],
                WALL_JOIN_THROUGH_FIELDS[slot],
                WALL_JOIN_REACH_FIELDS[slot],
                WALL_JOIN_LAYER_REACH_FIELDS[slot],
                WALL_JOIN_LAYER_EDGE_REACH_FIELDS[slot],
                WALL_JOIN_AXIS_REACH_FIELDS[slot],
            ];
            wall.fields
                .retain(|(name, _)| !replaced.contains(&name.as_str()));
            let float = |value: f64| InstanceField::Float { value, size: 8 };
            match cut {
                joins::CornerEnd::Reach {
                    reach_feet: reach,
                    partner,
                } => {
                    if let Some((partner, through)) = partner {
                        wall.fields.push((
                            WALL_JOIN_PARTNER_FIELDS[slot].into(),
                            InstanceField::ElementId {
                                tag: 0,
                                id: partner,
                            },
                        ));
                        wall.fields.push((
                            WALL_JOIN_THROUGH_FIELDS[slot].into(),
                            InstanceField::Bool(through),
                        ));
                    }
                    wall.fields
                        .push((WALL_JOIN_REACH_FIELDS[slot].into(), float(reach)));
                    let layers = layer_counts.get(&id).copied().unwrap_or(1);
                    if layers > 1 {
                        wall.fields.push((
                            WALL_JOIN_LAYER_REACH_FIELDS[slot].into(),
                            InstanceField::Vector(vec![float(reach); layers]),
                        ));
                    }
                }
                joins::CornerEnd::Layers {
                    reach_feet,
                    partner,
                    axis_reach_feet,
                } => {
                    wall.fields.push((
                        WALL_JOIN_AXIS_REACH_FIELDS[slot].into(),
                        float(axis_reach_feet),
                    ));
                    wall.fields.push((
                        WALL_JOIN_PARTNER_FIELDS[slot].into(),
                        InstanceField::ElementId {
                            tag: 0,
                            id: partner,
                        },
                    ));
                    wall.fields.push((
                        WALL_JOIN_THROUGH_FIELDS[slot].into(),
                        InstanceField::Bool(false),
                    ));
                    wall.fields.push((
                        WALL_JOIN_LAYER_REACH_FIELDS[slot].into(),
                        InstanceField::Vector(reach_feet.into_iter().map(float).collect()),
                    ));
                }
                joins::CornerEnd::Mitre(edges) => {
                    wall.fields.push((
                        WALL_JOIN_LAYER_EDGE_REACH_FIELDS[slot].into(),
                        InstanceField::Vector(edges.into_iter().map(float).collect()),
                    ));
                }
            }
        }
    }
}

/// Give each wall with a centreline the wall it butt-joins at each end and
/// whether it runs through there, where its and its partner's join lists
/// decide it, and, between single-layer walls, how far past the end its
/// body reaches ([`crate::element_record_wall_joins::butt_joins`]).
pub(super) fn attach_wall_butt_joins(
    rf: &mut RevitFile,
    revit_version: u32,
    walls: &mut [DecodedElement],
    type_layers: &BTreeMap<u32, Vec<crate::partition_compound_structure::CompoundLayer>>,
    native: &BTreeMap<u64, crate::native_walls::NativeWall>,
) {
    use crate::element_record_wall_joins as joins;
    let lines = wall_lines(walls, type_layers, native);
    if lines.len() < 2 {
        return;
    }
    let every: BTreeSet<u32> = walls.iter().filter_map(|wall| wall.id).collect();
    let read: BTreeSet<u32> = lines.iter().map(|line| line.element_id).collect();
    let Ok(partners) = crate::partition_compound_structure::scan_wall_join_partners(
        rf,
        revit_version,
        &every,
        &read,
    ) else {
        return;
    };
    // The joined-wall lists decide the L joints the join lists do not.
    let joined = crate::partition_compound_structure::scan_wall_joined_entries(
        rf,
        revit_version,
        &every,
        &read,
    )
    .unwrap_or_default();
    let found = joins::butt_joins(&lines, &partners, &joined);
    for wall in walls.iter_mut() {
        let Some(ends) = wall.id.and_then(|id| found.get(&id)) else {
            continue;
        };
        for (slot, join) in ends.iter().enumerate() {
            let Some(join) = join else {
                continue;
            };
            wall.fields.push((
                WALL_JOIN_PARTNER_FIELDS[slot].into(),
                InstanceField::ElementId {
                    tag: 0,
                    id: join.partner,
                },
            ));
            wall.fields.push((
                WALL_JOIN_THROUGH_FIELDS[slot].into(),
                InstanceField::Bool(join.runs_through),
            ));
            if let Some(reach) = join.reach_feet {
                wall.fields.push((
                    WALL_JOIN_REACH_FIELDS[slot].into(),
                    InstanceField::Float {
                        value: reach,
                        size: 8,
                    },
                ));
            }
            if let Some(edges) = &join.layer_edge_reach_feet {
                wall.fields.push((
                    WALL_JOIN_LAYER_EDGE_REACH_FIELDS[slot].into(),
                    InstanceField::Vector(
                        edges
                            .iter()
                            .flatten()
                            .map(|value| InstanceField::Float {
                                value: *value,
                                size: 8,
                            })
                            .collect(),
                    ),
                ));
            }
            if let (Some(reaches), Some(partner)) = (
                &join.layer_reach_feet,
                lines.iter().find(|line| line.element_id == join.partner),
            ) {
                // The plain join: the partner's far face where this wall
                // runs through, its near face where it stops.
                let half = partner.thickness_feet * 0.5;
                let plain = if join.runs_through { half } else { -half };
                if reaches.iter().any(|reach| (reach - plain).abs() > 1e-9) {
                    wall.fields.push((
                        WALL_JOIN_AXIS_REACH_FIELDS[slot].into(),
                        InstanceField::Float {
                            value: plain,
                            size: 8,
                        },
                    ));
                }
            }
            if let Some(reaches) = &join.layer_reach_feet {
                wall.fields.push((
                    WALL_JOIN_LAYER_REACH_FIELDS[slot].into(),
                    InstanceField::Vector(
                        reaches
                            .iter()
                            .map(|value| InstanceField::Float {
                                value: *value,
                                size: 8,
                            })
                            .collect(),
                    ),
                ));
            }
        }
    }
}

/// Give each wall what its native record says about the ends of its location
/// curve ([`crate::native_walls::WallEnds`]): the curve itself
/// ([`WALL_LOCATION_LINE_FIELD`]) and its length
/// ([`WALL_CURVE_LENGTH_FIELD`], where none is given yet), and at each end
/// how it is joined ([`WALL_JOIN_TYPE_FIELDS`]) and the wall it is joined to
/// ([`WALL_JOIN_PARTNER_FIELDS`]).
///
/// The record decides the partner: a partner another reading named that the
/// record does not list at that end is dropped, with whether it runs
/// through, and where the record lists one other wall alone there and none is
/// named, that wall is. At an end where joins are disallowed, nothing is
/// joined: its partner and the reaches a join gave it are dropped, so the
/// body runs to the end of its line, as Revit's does.
///
/// The fields' ends are the wall's centreline's where it has one
/// ([`wall_axis_from_fields`]) and the curve runs the other way, and the
/// curve's otherwise.
pub fn attach_wall_end_records(
    walls: &mut [DecodedElement],
    ends: &BTreeMap<u64, crate::native_walls::WallEnds>,
) {
    let near = |a: [f64; 2], b: [f64; 2]| {
        (a[0] - b[0]).hypot(a[1] - b[1]) <= NATIVE_WALL_LINE_TOLERANCE_FEET
    };
    for wall in walls.iter_mut() {
        let Some(read) = wall.id.and_then(|id| ends.get(&u64::from(id))) else {
            continue;
        };
        let [start, end] = read.curve;
        let reversed = wall_axis_from_fields(&wall.fields)
            .is_some_and(|axis| near(start, axis.end) && near(end, axis.start));
        let float = |value: f64| InstanceField::Float { value, size: 8 };
        wall.fields.push((
            WALL_LOCATION_LINE_FIELD.into(),
            InstanceField::Vector(vec![
                float(start[0]),
                float(start[1]),
                float(end[0]),
                float(end[1]),
            ]),
        ));
        let length = (end[0] - start[0]).hypot(end[1] - start[1]);
        if length.is_finite()
            && length > 0.0
            && !wall
                .fields
                .iter()
                .any(|(name, _)| name == WALL_CURVE_LENGTH_FIELD)
        {
            wall.fields
                .push((WALL_CURVE_LENGTH_FIELD.into(), float(length)));
        }
        for curve_end in 0..2 {
            let slot = if reversed { 1 - curve_end } else { curve_end };
            let partner_of = |fields: &[(String, InstanceField)]| {
                fields.iter().find_map(|(name, value)| match value {
                    InstanceField::ElementId { id, .. }
                        if name == WALL_JOIN_PARTNER_FIELDS[slot] =>
                    {
                        Some(*id)
                    }
                    _ => None,
                })
            };
            let joined = &read.joined[curve_end];
            let named = partner_of(&wall.fields);
            let kind = if read.disallowed[curve_end] {
                let dropped = [
                    WALL_JOIN_PARTNER_FIELDS[slot],
                    WALL_JOIN_THROUGH_FIELDS[slot],
                    WALL_JOIN_REACH_FIELDS[slot],
                    WALL_JOIN_LAYER_REACH_FIELDS[slot],
                    WALL_JOIN_LAYER_EDGE_REACH_FIELDS[slot],
                    WALL_JOIN_AXIS_REACH_FIELDS[slot],
                ];
                wall.fields
                    .retain(|(name, _)| !dropped.contains(&name.as_str()));
                undo_record_box_trim(
                    &mut wall.fields,
                    read.curve[curve_end],
                    read.curve[1 - curve_end],
                );
                Some(WALL_JOIN_DISALLOWED)
            } else {
                if named.is_some_and(|id| !joined.contains(&u64::from(id))) {
                    let dropped = [
                        WALL_JOIN_PARTNER_FIELDS[slot],
                        WALL_JOIN_THROUGH_FIELDS[slot],
                    ];
                    wall.fields
                        .retain(|(name, _)| !dropped.contains(&name.as_str()));
                }
                if joined.len() >= 2 {
                    wall.fields.push((
                        WALL_JOIN_WALLS_FIELDS[slot].into(),
                        InstanceField::Vector(
                            joined
                                .iter()
                                .filter_map(|other| u32::try_from(*other).ok())
                                .map(|id| InstanceField::ElementId { tag: 0, id })
                                .collect(),
                        ),
                    ));
                }
                if partner_of(&wall.fields).is_none() {
                    if let [other] = joined.as_slice() {
                        if let Ok(id) = u32::try_from(*other) {
                            wall.fields.push((
                                WALL_JOIN_PARTNER_FIELDS[slot].into(),
                                InstanceField::ElementId { tag: 0, id },
                            ));
                        }
                    }
                }
                (!joined.is_empty())
                    .then(|| {
                        read.join_types[curve_end]
                            .and_then(|kind| usize::try_from(kind).ok())
                            .and_then(|kind| WALL_JOIN_TYPE_NAMES.get(kind).copied())
                    })
                    .flatten()
            };
            if let Some(kind) = kind {
                wall.fields.push((
                    WALL_JOIN_TYPE_FIELDS[slot].into(),
                    InstanceField::String(kind.into()),
                ));
            }
        }
    }
}

/// Give back the trim the record-box solver ([`apply_wall_join_trim`]) took
/// from the end of a wall's body at `at`, the location curve's end where joins
/// are disallowed (`other` is its other end), so the body runs to the end of
/// its line, as Revit's does: on 2024's Core Interior a wall whose end allows
/// no join was 0.25 ft short of Revit's body there. A body with no trim is
/// left as it is.
fn undo_record_box_trim(fields: &mut [(String, InstanceField)], at: [f64; 2], other: [f64; 2]) {
    use crate::element_record_wall_joins as joins;
    let axis = usize::from((other[1] - at[1]).abs() > (other[0] - at[0]).abs());
    let at_low = at[axis] < other[axis];
    let trim_field = if at_low {
        joins::WALL_TRIM_START_FIELD
    } else {
        joins::WALL_TRIM_END_FIELD
    };
    let Some(trim) = fields.iter().find_map(|(name, value)| match value {
        InstanceField::Float { value, .. } if name == trim_field => Some(*value),
        _ => None,
    }) else {
        return;
    };
    if !(trim.is_finite() && trim > 0.0) {
        return;
    }
    let (centre_field, extent_field) = if axis == 0 {
        ("m_locationX", "m_bboxWidth")
    } else {
        ("m_locationY", "m_bboxDepth")
    };
    for (name, value) in fields.iter_mut() {
        let InstanceField::Float { value: v, .. } = value else {
            continue;
        };
        if name == trim_field {
            *v = 0.0;
        } else if name == extent_field {
            *v += trim;
        } else if name == centre_field {
            *v += if at_low { -trim / 2.0 } else { trim / 2.0 };
        }
    }
}

/// Rewrite a wall's plan centre and plan extents to the trimmed body.
pub(super) fn apply_wall_join_trim(
    decoded: &mut DecodedElement,
    record: &crate::partition_element_records::PartitionElementRecord,
    trim: &crate::element_record_wall_joins::WallJoinTrim,
) {
    use crate::element_record_wall_joins as joins;

    let low = record.bbox_feet[trim.axis] + trim.start_feet;
    let high = record.bbox_feet[trim.axis + 3] - trim.end_feet;
    let (centre_field, extent_field) = if trim.axis == 0 {
        ("m_locationX", "m_bboxWidth")
    } else {
        ("m_locationY", "m_bboxDepth")
    };
    for (name, value) in decoded.fields.iter_mut() {
        if name == centre_field {
            *value = InstanceField::Float {
                value: (low + high) * 0.5,
                size: 8,
            };
        } else if name == extent_field {
            *value = InstanceField::Float {
                value: high - low,
                size: 8,
            };
        }
    }
    decoded.fields.push((
        joins::WALL_BODY_SOURCE_FIELD.into(),
        InstanceField::String(joins::WALL_BODY_JOIN_TRIMMED.into()),
    ));
    decoded.fields.push((
        joins::WALL_THICKNESS_FIELD.into(),
        InstanceField::Float {
            value: trim.thickness_feet,
            size: 8,
        },
    ));
    decoded.fields.push((
        joins::WALL_TRIM_START_FIELD.into(),
        InstanceField::Float {
            value: trim.start_feet,
            size: 8,
        },
    ));
    decoded.fields.push((
        joins::WALL_TRIM_END_FIELD.into(),
        InstanceField::Float {
            value: trim.end_feet,
            size: 8,
        },
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element_record_wall_joins as joins;

    fn float(fields: &[(String, InstanceField)], wanted: &str) -> Option<f64> {
        fields.iter().find_map(|(name, value)| match value {
            InstanceField::Float { value, .. } if name == wanted => Some(*value),
            _ => None,
        })
    }

    /// A record-box wall along X from x = 0 to 10 ft, trimmed 0.25 ft at its
    /// high end by the solver, whose record disallows joins at that end and
    /// joins three other walls at its start.
    fn wall() -> (DecodedElement, BTreeMap<u64, crate::native_walls::WallEnds>) {
        let f = |value: f64| InstanceField::Float { value, size: 8 };
        let wall = DecodedElement {
            id: Some(7),
            class: "Wall".into(),
            fields: vec![
                ("m_locationX".into(), f(4.875)),
                ("m_bboxWidth".into(), f(9.75)),
                (joins::WALL_TRIM_START_FIELD.into(), f(0.0)),
                (joins::WALL_TRIM_END_FIELD.into(), f(0.25)),
            ],
            byte_range: 0..0,
            provenance: Default::default(),
        };
        let ends = crate::native_walls::WallEnds {
            curve: [[0.0, 0.0], [10.0, 0.0]],
            joined: [vec![8, 9], vec![10]],
            join_types: [Some(0), Some(0)],
            disallowed: [false, true],
        };
        (wall, BTreeMap::from([(7, ends)]))
    }

    #[test]
    fn a_disallowed_end_gets_back_its_trim() {
        let (mut wall, ends) = wall();
        attach_wall_end_records(std::slice::from_mut(&mut wall), &ends);
        assert_eq!(float(&wall.fields, "m_bboxWidth"), Some(10.0));
        assert_eq!(float(&wall.fields, "m_locationX"), Some(5.0));
        assert_eq!(float(&wall.fields, joins::WALL_TRIM_END_FIELD), Some(0.0));
    }

    #[test]
    fn a_join_of_three_walls_lists_both_others() {
        let (mut wall, ends) = wall();
        attach_wall_end_records(std::slice::from_mut(&mut wall), &ends);
        let listed = wall.fields.iter().find_map(|(name, value)| match value {
            InstanceField::Vector(items) if name == WALL_JOIN_WALLS_FIELDS[0] => {
                Some(items.clone())
            }
            _ => None,
        });
        let ids: Vec<u32> = listed
            .into_iter()
            .flatten()
            .filter_map(|item| match item {
                InstanceField::ElementId { id, .. } => Some(id),
                _ => None,
            })
            .collect();
        assert_eq!(ids, vec![8, 9]);
        assert!(
            !wall
                .fields
                .iter()
                .any(|(name, _)| name == WALL_JOIN_WALLS_FIELDS[1]
                    || name == WALL_JOIN_PARTNER_FIELDS[0])
        );
    }
}
