//! Roof edges' slopes.
//!
//! Each sketch line of a footprint roof carries, in its element data, the
//! slope Revit's roof tools set per edge. After the edge's slope angle come
//! 16 zero bytes and 32 `ff` bytes, then:
//!
//! ```text
//! f64   slope angle, radians (9:12 when the edge sets no slope)
//! 00 × 16
//! ff × 32
//! i64   a small negative value (−9, −6 on the files measured)
//! u32   0 or 1
//! u32   7
//! u8    1 when the edge defines the roof's slope
//! u8    1 when the edge defines the roof's slope
//! u8    1
//! u8    0
//! ...   on an edge that defines the slope, the slope as −rise/run at
//!       +0x0b past the flags (2024, or 0.0; on 2025 to 2027 the plane's
//!       raise)
//! ```
//!
//! On the Autodesk tutorial house (Revit 2024) three roofs have no edge
//! that defines a slope and one has exactly one, at 1:12. That shed roof,
//! rising from its defining edge across its outline at 1:12 with its type's
//! 1.25 ft thickness measured square to the slope, reproduces its record
//! box's height, 2.959193880 ft, to 1e-9 ft. No edge of Snowdon Towers'
//! roofs defines a slope. One of them, 2112673, is sloped in Revit's own
//! export all the same: a roof's slope can be set some other way, which is
//! not read, so a roof with no defining edge is not known to be flat.
//!
//! Revit 2025 writes the same block, but +0x3b is not the
//! rise/run there: it is how far the edge's plane is raised at the edge. On
//! the roofs of `R2025_Models.zip` (sheds, a gable, hips, L-shaped hips)
//! every defining edge and its angle match Revit's own IFC export.
//!
//! On the tiny reference hip and gable roofs (fixtures-tiny-1) the block is
//! the same bytes on 2024, 2025, 2026 and 2027, and +0x3b is 0.0 on every
//! defining edge, also on 2024. Their 30 degree planes then give each edge
//! the slope and the roof the top (6.2517 m) the Revit API reports.
//!
//! Revit 2018 to 2023 write the block shorter, the `u32` 0 or 1 and the 7
//! before a 16-byte `ff` run, and an `i32` in place of the `i64`:
//!
//! ```text
//! f64   slope angle, radians
//! 00 × 16
//! u32   0 or 1
//! u32   7
//! ff × 16
//! i32   a small negative value (−6, −9)
//! u8 × 4  the same flags as above
//! ```
//!
//! The bytes after the flags are the same as on 2024 to 2027 on the tiny
//! reference hip and gable roofs of every release 2018 to 2027, each edge's
//! angle and defining flag are the Revit API's, and +0x0b past the flags is
//! 0.0. What that f64 holds on a raised rising edge is not measured before
//! 2024, so such a block is read only where it is 0.0. An edge falling into
//! the roof (a negative angle, `rac_advanced_sample_project`'s butterfly roof
//! 142430, saved by every release 2018 to 2027) carries its overhang's raise
//! there, as on 2024.

use crate::ifc::element_provenance::CopyTrail;
use crate::le_bytes;
use crate::native_json::element_id_i64;
use crate::{Result, RevitFile};
use std::collections::{BTreeMap, BTreeSet};

/// Releases this layout is measured on. Revit 2026 and 2027 write the
/// block as 2025 does, byte for byte at the same offsets past the `ff` run,
/// with the plane's raise at +0x3b: on the tiny reference hip and gable
/// roofs (fixtures-tiny-1) every edge's angle and defining flag match the
/// Revit API's, and the roofs' 30 degree planes give the API's slopes and top.
/// The fascia depth and rafter cut stay read on 2025 only. Revit 2018 to 2023
/// write the shorter block (see the module), measured on the same tiny
/// reference roofs; slope arrows are not read there.
pub const ROOF_SLOPE_SUPPORTED_REVIT_VERSIONS: &[u32] =
    &[2018, 2019, 2020, 2021, 2022, 2023, 2024, 2025, 2026, 2027];

/// How far into a sketch line's data its slope block is looked for.
pub const EDGE_SLOPE_WINDOW: usize = 0x1000;

/// One roof edge's slope.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EdgeSlope {
    /// Whether the edge defines the roof's slope.
    pub defines_slope: bool,
    /// The edge's slope angle from horizontal, radians: below zero where
    /// the roof falls from the edge, as a butterfly roof's does.
    pub angle_radians: f64,
    /// How far the edge's plane is raised at the edge, feet: the f64 at
    /// +0x3b on 2025 to 2027, 0 on 2024, where that f64 is −rise/run or 0.0.
    pub plate_offset_feet: f64,
}

/// The slope block of a roof sketch line, from its data, as Revit 2024
/// writes it. `None` when no block of this shape is found, or when an edge
/// that defines the slope does not also carry its rise over run.
pub fn edge_slope(data: &[u8]) -> Option<EdgeSlope> {
    edge_slope_in(data, 2024)
}

/// [`edge_slope`] as `revit_version` writes the block. On 2025 to 2027 the f64 at
/// +0x3b is not the rise over run but how far the edge's plane is raised
///: 0.0 on most edges; 0.17713 ft (54 mm) on `03_2Slope_Rafter2SquareCut`
/// (`R2025_Models.zip`), whose off-centre ridge it reproduces, with
/// 0.10827 ft (the edge's 33 mm offset from its wall) at +0x47; 0.98425 ft
/// on `09_LShapedHipRoof_1Edge300Offset`'s 300 mm edge.
pub fn edge_slope_in(data: &[u8], revit_version: u32) -> Option<EdgeSlope> {
    if revit_version <= 2023 {
        return edge_slope_32(data);
    }
    const RUN: [u8; 32] = [0xff; 32];
    memchr::memmem::find_iter(data, &RUN).find_map(|at| {
        let zeros = data.get(at.checked_sub(0x10)?..at)?;
        if zeros.iter().any(|&b| b != 0) {
            return None;
        }
        let angle = le_bytes::f64_at(data, at.checked_sub(0x18)?)?;
        if !(angle.is_finite() && angle != 0.0 && angle.abs() < std::f64::consts::FRAC_PI_2) {
            return None;
        }
        let small = le_bytes::i64_at(data, at + 0x20)?;
        if !(-64..=-1).contains(&small) || le_bytes::u32_at(data, at + 0x2c)? != 7 {
            return None;
        }
        let flags = data.get(at + 0x30..at + 0x34)?;
        let defines_slope = match flags {
            [0, 0, 1, 0] => false,
            [1, 1, 1, 0] => true,
            _ => return None,
        };
        // 2025 on: the plane's raise. 2024: the rise over run, or 0.0
        // where the edge is not raised (the tiny reference hip and gable).
        let plane_raise = revit_version >= 2025;
        if defines_slope {
            let rise_over_run = le_bytes::f64_at(data, at + 0x3b)?;
            // An edge sloping down into the roof (a negative angle) carries
            // its overhang's raise there on 2024 too: 0.5275 ft, 600 mm at
            // 15 degrees, on `rac_advanced_sample_project`'s roof 142430.
            let recorded = plane_raise
                || angle < 0.0
                || rise_over_run == 0.0
                || (rise_over_run + angle.tan()).abs() <= 1e-9;
            if !recorded {
                return None;
            }
        }
        let plate_offset_feet = if plane_raise {
            le_bytes::f64_at(data, at + 0x3b).filter(|v| v.is_finite())?
        } else {
            0.0
        };
        Some(EdgeSlope {
            defines_slope,
            angle_radians: angle,
            plate_offset_feet,
        })
    })
}

/// The block as Revit 2018 to 2023 write it (see the module): the plane is
/// read only where the f64 at +0x0b past the flags is 0.0.
fn edge_slope_32(data: &[u8]) -> Option<EdgeSlope> {
    const RUN: [u8; 20] = [
        7, 0, 0, 0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xff, 0xff,
    ];
    memchr::memmem::find_iter(data, &RUN).find_map(|at| {
        let zeros = data.get(at.checked_sub(0x14)?..at.checked_sub(4)?)?;
        if zeros.iter().any(|&b| b != 0) || le_bytes::u32_at(data, at.checked_sub(4)?)? > 1 {
            return None;
        }
        let angle = le_bytes::f64_at(data, at.checked_sub(0x1c)?)?;
        if !(angle.is_finite() && angle != 0.0 && angle.abs() < std::f64::consts::FRAC_PI_2) {
            return None;
        }
        let small = le_bytes::i32_at(data, at + 0x14)?;
        if !(-64..=-1).contains(&small) {
            return None;
        }
        let defines_slope = match data.get(at + 0x18..at + 0x1c)? {
            [0, 0, 1, 0] => false,
            [1, 1, 1, 0] => true,
            _ => return None,
        };
        // A rising edge only where nothing raises it; a falling edge's raise
        // is its overhang's, which the record box places.
        let raise = le_bytes::f64_at(data, at + 0x18 + 0x0b)?;
        if !raise.is_finite() || (angle > 0.0 && raise != 0.0) {
            return None;
        }
        Some(EdgeSlope {
            defines_slope,
            angle_radians: angle,
            plate_offset_feet: 0.0,
        })
    })
}

/// Each sketch line's slope, by ElementId, from its own data; a line whose
/// copies disagree is dropped. Empty on a release this layout is not
/// measured on.
pub fn scan_edge_slopes(
    rf: &mut RevitFile,
    revit_version: u32,
    lines: &BTreeSet<u32>,
) -> Result<BTreeMap<u32, EdgeSlope>> {
    let layout = match crate::partition_names::element_data_layout(revit_version) {
        Some(layout) if ROOF_SLOPE_SUPPORTED_REVIT_VERSIONS.contains(&revit_version) => layout,
        _ => return Ok(BTreeMap::new()),
    };
    let header = layout.header;
    let mut found: BTreeMap<u32, Option<EdgeSlope>> = BTreeMap::new();
    let recording = rf.recording_value_sources();
    let mut trail = CopyTrail::default();
    for stream in rf.partition_stream_names() {
        let Ok(inflated) = rf.inflated_partition(&stream) else {
            continue;
        };
        let buf = inflated.bytes();
        let Ok(hits) = rf.header_hits(&stream, &header) else {
            continue;
        };
        for (index, &hit) in hits.iter().enumerate() {
            let id_at = hit + header.len();
            let Some(id) = layout.id_at(buf, id_at).filter(|id| lines.contains(id)) else {
                continue;
            };
            let end = hits
                .get(index + 1)
                .copied()
                .unwrap_or(buf.len())
                .min(hit.saturating_add(EDGE_SLOPE_WINDOW))
                .min(buf.len());
            if recording {
                trail.named(
                    id,
                    crate::partition_room_parameters::partition_number(&stream),
                );
            }
            let Some(slope) = buf
                .get(id_at + 8..end)
                .and_then(|data| edge_slope_in(data, revit_version))
            else {
                continue;
            };
            if recording {
                let partition = crate::partition_room_parameters::partition_number(&stream);
                trail.gave(id, partition, hit);
            }
            match found.get_mut(&id) {
                None => {
                    found.insert(id, Some(slope));
                }
                Some(held) => {
                    if *held != Some(slope) {
                        *held = None;
                    }
                }
            }
        }
    }
    let kept: BTreeMap<u32, EdgeSlope> = found
        .into_iter()
        .filter_map(|(id, slope)| slope.map(|s| (id, s)))
        .collect();
    if recording {
        trail.note(rf, "edge_slope", kept.keys().copied());
    }
    Ok(kept)
}

/// A slope arrow's heights: the `SlopeCurveData` an arrow sketch
/// line carries, in its members' order.
///
/// ```text
/// -0x28  f64      height offset at the tail (m_startHeight), feet
/// -0x20  f64      height offset at the head (m_endHeight), feet
/// -0x18  f64 × 2  0.0, 0.0 (the tail's and head's level elevations)
/// -0x08  f64      slope angle, radians (m_angle)
///  0x00  ff × 16  the tail's and head's levels, none
/// +0x10  u32      m_useAngle: 0 set by "Height at Tail", 1 by "Slope"
/// ```
///
/// On `05_1Slope_ArrowDefined` ( `R2025_Models.zip`; "Height at Tail",
/// 22 and 3000 mm) the roof rises from the tail's height to the head's along
/// the arrow; on `06_1Slope_ArrowDefined_SetToSlope` (the same arrow set by
/// "Slope", `m_useAngle` 1) it rises at the angle from the tail's height,
/// and the head's height is unused. Each
/// reproduces its record box. `None` unless the word at +0x10 is 0 or 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArrowHeights {
    /// Height offset at the tail, feet.
    pub tail_feet: f64,
    /// Height offset at the head, feet.
    pub head_feet: f64,
    /// The arrow's slope angle, radians.
    pub angle_radians: f64,
    /// Whether the arrow is set by its angle (`m_useAngle`) rather than by
    /// its two heights.
    pub by_angle: bool,
}

impl ArrowHeights {
    /// The rise over run along the arrow, `run` its length in feet.
    pub fn rise_over_run(&self, run: f64) -> f64 {
        if self.by_angle {
            self.angle_radians.tan()
        } else {
            (self.head_feet - self.tail_feet) / run
        }
    }
}

/// The [`ArrowHeights`] in a slope arrow's data, from the first block of
/// this shape: a slope angle followed by 16 `ff`, not 32 (an edge's
/// block), then `u32 0` or `u32 1`.
pub fn arrow_heights(data: &[u8]) -> Option<ArrowHeights> {
    const RUN: [u8; 16] = [0xff; 16];
    memchr::memmem::find_iter(data, &RUN).find_map(|at| {
        let by_angle = match data.get(at + 0x10..at + 0x14)? {
            [0, 0, 0, 0] => false,
            [1, 0, 0, 0] => true,
            _ => return None,
        };
        let base = at.checked_sub(0x28)?;
        let angle = le_bytes::f64_at(data, base + 0x20)?;
        if !(angle.is_finite() && angle > 0.0 && angle < std::f64::consts::FRAC_PI_2) {
            return None;
        }
        let levels = [
            le_bytes::f64_at(data, base + 0x10)?,
            le_bytes::f64_at(data, base + 0x18)?,
        ];
        if levels != [0.0, 0.0] {
            return None;
        }
        let (tail_feet, head_feet) = (
            le_bytes::f64_at(data, base)?,
            le_bytes::f64_at(data, base + 0x08)?,
        );
        (tail_feet.is_finite() && head_feet.is_finite() && (by_angle || head_feet != tail_feet))
            .then_some(ArrowHeights {
                tail_feet,
                head_feet,
                angle_radians: angle,
                by_angle,
            })
    })
}

/// Each listed sketch line's [`ArrowHeights`], read as [`scan_edge_slopes`]
/// reads edges; a line whose copies disagree is dropped.
pub fn scan_arrow_heights(
    rf: &mut RevitFile,
    revit_version: u32,
    lines: &BTreeSet<u32>,
) -> Result<BTreeMap<u32, ArrowHeights>> {
    let header = match crate::partition_names::element_data_header(revit_version) {
        Some(header) if ROOF_SLOPE_SUPPORTED_REVIT_VERSIONS.contains(&revit_version) => header,
        _ => return Ok(BTreeMap::new()),
    };
    let mut found: BTreeMap<u32, Option<ArrowHeights>> = BTreeMap::new();
    let recording = rf.recording_value_sources();
    let mut trail = CopyTrail::default();
    for stream in rf.partition_stream_names() {
        let Ok(inflated) = rf.inflated_partition(&stream) else {
            continue;
        };
        let buf = inflated.bytes();
        let Ok(hits) = rf.header_hits(&stream, &header) else {
            continue;
        };
        for (index, &hit) in hits.iter().enumerate() {
            let id_at = hit + header.len();
            let Some(id) = le_bytes::u64_at(buf, id_at)
                .and_then(|id| u32::try_from(id).ok())
                .filter(|id| lines.contains(id))
            else {
                continue;
            };
            let end = hits
                .get(index + 1)
                .copied()
                .unwrap_or(buf.len())
                .min(hit.saturating_add(EDGE_SLOPE_WINDOW))
                .min(buf.len());
            if recording {
                trail.named(
                    id,
                    crate::partition_room_parameters::partition_number(&stream),
                );
            }
            let Some(heights) = buf.get(id_at + 8..end).and_then(arrow_heights) else {
                continue;
            };
            if recording {
                let partition = crate::partition_room_parameters::partition_number(&stream);
                trail.gave(id, partition, hit);
            }
            match found.get_mut(&id) {
                None => {
                    found.insert(id, Some(heights));
                }
                Some(held) => {
                    if *held != Some(heights) {
                        *held = None;
                    }
                }
            }
        }
    }
    let kept: BTreeMap<u32, ArrowHeights> = found
        .into_iter()
        .filter_map(|(id, heights)| heights.map(|h| (id, h)))
        .collect();
    if recording {
        trail.note(rf, "arrow_heights", kept.keys().copied());
    }
    Ok(kept)
}

/// The parameter id the roof's fascia depth precedes in its element data.
pub const FASCIA_DEPTH_NEXT_ID: i64 = -1_001_711;

/// A footprint roof's fascia depth, feet: in the roof's own
/// element data, the f64 just before the first `i64` [`FASCIA_DEPTH_NEXT_ID`].
/// On the roofs `R2025_Models.zip` it is 0.0 on every roof but
/// `07_1Slope_ArrowDefined_2Cut50mmFascia`, where it is 0.164042 ft
/// (50 mm); a two-cut rafter's plumb face at the eave is that tall.
pub fn fascia_depth(data: &[u8]) -> Option<f64> {
    let at = memchr::memmem::find(data, &FASCIA_DEPTH_NEXT_ID.to_le_bytes())?;
    le_bytes::f64_at(data, at.checked_sub(8)?).filter(|v| v.is_finite() && (0.0..100.0).contains(v))
}

/// Each roof's [`fascia_depth`], by ElementId, from its element data; a
/// roof whose copies disagree is dropped. Empty on a release this layout is
/// not measured on (2025 only).
pub fn scan_fascia_depths(
    rf: &mut RevitFile,
    revit_version: u32,
    roofs: &BTreeSet<u32>,
) -> Result<BTreeMap<u32, f64>> {
    let header = match crate::partition_names::element_data_header(revit_version) {
        Some(header) if revit_version == 2025 => header,
        _ => return Ok(BTreeMap::new()),
    };
    let mut found: BTreeMap<u32, Option<f64>> = BTreeMap::new();
    let recording = rf.recording_value_sources();
    let mut trail = CopyTrail::default();
    for stream in rf.partition_stream_names() {
        let Ok(inflated) = rf.inflated_partition(&stream) else {
            continue;
        };
        let buf = inflated.bytes();
        let Ok(hits) = rf.header_hits(&stream, &header) else {
            continue;
        };
        for (index, &hit) in hits.iter().enumerate() {
            let id_at = hit + header.len();
            let Some(id) = le_bytes::u64_at(buf, id_at)
                .and_then(|id| u32::try_from(id).ok())
                .filter(|id| roofs.contains(id))
            else {
                continue;
            };
            let end = hits
                .get(index + 1)
                .copied()
                .unwrap_or(buf.len())
                .min(hit.saturating_add(EDGE_SLOPE_WINDOW))
                .min(buf.len());
            if recording {
                trail.named(
                    id,
                    crate::partition_room_parameters::partition_number(&stream),
                );
            }
            let Some(depth) = buf.get(id_at + 8..end).and_then(fascia_depth) else {
                continue;
            };
            if recording {
                let partition = crate::partition_room_parameters::partition_number(&stream);
                trail.gave(id, partition, hit);
            }
            match found.get_mut(&id) {
                None => {
                    found.insert(id, Some(depth));
                }
                Some(held) => {
                    if *held != Some(depth) {
                        *held = None;
                    }
                }
            }
        }
    }
    let kept: BTreeMap<u32, f64> = found
        .into_iter()
        .filter_map(|(id, depth)| depth.map(|d| (id, d)))
        .collect();
    if recording {
        trail.note(rf, "roof_fascia_depth", kept.keys().copied());
    }
    Ok(kept)
}

/// A footprint roof's "Rafter Cut".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RafterCut {
    /// "Plumb Cut", the default.
    Plumb,
    /// "Two Cut - Plumb".
    TwoCutPlumb,
    /// "Two Cut - Square".
    TwoCutSquare,
}

impl RafterCut {
    /// The name [`RAFTER_CUT_FIELD`](crate::partition_schema_mvp::ROOF_RAFTER_CUT_FIELD)
    /// records.
    pub fn name(self) -> &'static str {
        match self {
            Self::Plumb => "plumb",
            Self::TwoCutPlumb => "two_cut_plumb",
            Self::TwoCutSquare => "two_cut_square",
        }
    }

    /// The cut a [`RafterCut::name`] names.
    pub fn from_name(name: &str) -> Option<Self> {
        [Self::Plumb, Self::TwoCutPlumb, Self::TwoCutSquare]
            .into_iter()
            .find(|cut| cut.name() == name)
    }
}

/// The parameter in a `ProfileRoof`'s `m_paramSet` that holds its rafter
/// cut.
pub const RAFTER_CUT_PARAMETER: i64 = -1_001_710;

/// The rafter cut a [`RAFTER_CUT_PARAMETER`] value names. On the 13
/// footprint roofs `R2025_Models.zip`: 33615 on the eight with the
/// default plumb cut, 33619 on the two two-cut plumb, 33618 on the three
/// two-cut square. `02_1Slope`, `02_1Slope_Rafter2PlumbCut` and
/// `02_1Slope_Rafter2SquareCut` are one roof with each cut.
pub fn rafter_cut_from_value(value: i64) -> Option<RafterCut> {
    match value {
        33615 => Some(RafterCut::Plumb),
        33619 => Some(RafterCut::TwoCutPlumb),
        33618 => Some(RafterCut::TwoCutSquare),
        _ => None,
    }
}

/// Each of `roofs` that is a `ProfileRoof` (a footprint roof) with a known
/// [`RAFTER_CUT_PARAMETER`], read from its native record's object graph,
/// which needs no parameter definitions
/// ([`crate::native_document::extract_graphs`]). Empty where the native
/// record path does not read the file, or on a release this value is not
/// measured on (2025 only).
pub fn scan_rafter_cuts(
    rf: &mut RevitFile,
    revit_version: u32,
    roofs: &BTreeSet<u32>,
) -> BTreeMap<u32, RafterCut> {
    let mut found = BTreeMap::new();
    if revit_version != 2025 || roofs.is_empty() {
        return found;
    }
    let options = crate::native_document::Options {
        selected_ids: roofs.iter().map(|&id| u64::from(id)).collect(),
        value_source: Some("roof_rafter_cut"),
        ..crate::native_document::Options::default()
    };
    let _ = crate::native_document::extract_graphs(rf, &options, |record| {
        if record.class_name.as_deref() != Some("ProfileRoof") {
            return Ok(());
        }
        let Ok(id) = u32::try_from(record.identity.element_id) else {
            return Ok(());
        };
        let cut = record
            .graph
            .iter()
            .flat_map(|graph| graph.objects.iter())
            .filter_map(|object| object.fields.get("m_paramSet")?.as_array())
            .flatten()
            .find(|pair| {
                pair.get("m_paramId").and_then(element_id_i64) == Some(RAFTER_CUT_PARAMETER)
            })
            .and_then(|pair| pair.get("m_value")?.as_i64())
            .and_then(rafter_cut_from_value);
        if let Some(cut) = cut {
            found.insert(id, cut);
        }
        Ok(())
    });
    // A selected record that gave no cut is not a source of one.
    if rf.recording_value_sources() {
        for roof in roofs.iter().filter(|roof| !found.contains_key(*roof)) {
            rf.forget_value_source(*roof, "roof_rafter_cut");
        }
    }
    found
}
