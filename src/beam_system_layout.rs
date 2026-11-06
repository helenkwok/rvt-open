//! A Structural Beam System's layout record.
//!
//! Next to the element records, a partition holds one small record per beam
//! system that states how its beams are laid out. It is found by what it
//! names, not by an ElementId of its own: the system's own type id `T`, with
//! the system's Sketch id `K` (one of the entries of the system record's
//! first list) in the slot before it. Around `T` the record reads, with `w` the
//! id width (`u64` from Revit 2024, `u32` before):
//!
//! ```text
//! 2024 and later (offsets from T)         2018 - 2023 (offsets from T)
//!  -48 f64  spacing (feet)                 -60 f64  spacing
//!  -40 f64  maximum spacing (feet)         -52 f64  maximum spacing
//!  -32 f64  0.5 (unread)                   -44 f64  0.5 (unread)
//!  -24 f64  elevation from level (feet)    -36 f64  elevation from level
//!  -16 id   Beam Type, all ones if unset   -28 u32  layout rule
//!   -8 id   Sketch K                       -24 u32  6
//!    0 id   T                              -20 u32  justification
//!   +8 i64  Tag new members in view,       -16 u32  6 or 7
//!           -1 if none                     -12 u32  1 or -1
//!  +16 u32  layout rule                     -8 id   Beam Type
//!  +20 u32  6                               -4 id   Sketch K
//!  +24 u32  justification                    0 id   T
//!  +28 u32  6 or 7                          +4 u32  Tag new members in view
//!  +32 u32  1 or -1                               +8 u8   3D
//!  +36 u8   3D
//!  +37 u32  1
//!  +41 i32  -1010106, then i32 -1, u32 n and n UTF-16 units: Comments,
//!           or i32 -1 where there are none
//! ```
//!
//! Layout rule 1 is Fixed Distance and 3 is Maximum Spacing; justification 1
//! Center, 2 End, 3 Beginning, 4 Direction Line. The spacing slot is the Fixed
//! Spacing and the maximum slot the Maximum Spacing: for a Maximum Spacing
//! system whose rule was changed from Fixed Distance the two differ.
//!
//! Where the Sketch has a Level, a counted list of three ids holds the Level, the
//! SketchPlane and the Sketch (the ids of a list run in ascending order, so none
//! has a fixed place); a Sketch on a plane that is not a Level's is in a list of
//! two, the SketchPlane and the Sketch. The Level's name is the Work Plane. A
//! plain list cannot be told from one that names a Level the file's Level read
//! missed, so with no Levels read the work plane stays unknown.
//!
//! Fail closed: every constant above must hold, the rule and justification
//! must be known, and a hit named by two systems is dropped. A system with no
//! such record gets nothing.
//!
//! Measured on the 27 public models in seven releases (2018, 2019, 2022, 2023,
//! 2024, 2026, 2027), 414 systems, against the Revit API dump

use crate::RevitFile;
use std::collections::{BTreeMap, BTreeSet};

/// Builtin parameter id of Comments.
const COMMENTS_PARAMETER: i32 = -1_010_106;

/// Longest Comments string read, in UTF-16 units.
const MAX_COMMENTS_UNITS: usize = 4096;

/// A beam system's Sketch is on this kind of work plane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkPlane {
    /// The plane of this Level.
    Level(u32),
    /// A plane that is not a Level's (Revit's `<not associated>`).
    NotAssociated,
}

/// A beam system's layout, as stated by its layout record.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    /// 1 Fixed Distance, 3 Maximum Spacing.
    pub rule: u32,
    /// 1 Center, 2 End, 3 Beginning, 4 Direction Line.
    pub justification: u32,
    /// The "3D" setting.
    pub three_d: bool,
    /// Fixed Spacing, feet.
    pub spacing_feet: f64,
    /// Maximum Spacing, feet.
    pub maximum_spacing_feet: f64,
    /// Elevation from Level, feet.
    pub elevation_feet: f64,
    /// ElementId of the Beam Type; `None` where it is unset ("Default").
    pub beam_type_id: Option<u32>,
    /// ElementId of the system's own type.
    pub type_id: u32,
    /// ElementId of the Sketch.
    pub sketch_id: u32,
    /// ElementId of the view of "Tag new members in view"; `None` if none.
    pub tag_view_id: Option<u32>,
    /// Comments, where the record has them.
    pub comments: Option<String>,
}

impl Layout {
    /// Revit's name of the layout rule.
    pub fn rule_name(&self) -> &'static str {
        match self.rule {
            1 => "Fixed Distance",
            _ => "Maximum Spacing",
        }
    }

    /// Revit's name of the justification.
    pub fn justification_name(&self) -> &'static str {
        match self.justification {
            1 => "Center",
            2 => "End",
            3 => "Beginning",
            _ => "Direction Line",
        }
    }
}

fn u32_at(buf: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(buf.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(buf: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(buf.get(at..at + 8)?.try_into().ok()?))
}

fn f64_at(buf: &[u8], at: usize) -> Option<f64> {
    let v = f64::from_le_bytes(buf.get(at..at + 8)?.try_into().ok()?);
    v.is_finite().then_some(v)
}

/// An id slot of width `wide` (8 or 4) at `at`: `Some(None)` for all ones,
/// `None` for anything that is not an id.
fn id_slot(buf: &[u8], at: usize, wide: bool) -> Option<Option<u32>> {
    if wide {
        let v = u64_at(buf, at)?;
        if v == u64::MAX {
            return Some(None);
        }
        u32::try_from(v).ok().map(Some)
    } else {
        let v = u32_at(buf, at)?;
        Some((v != u32::MAX).then_some(v))
    }
}

/// The layout around the type id at `t` in `buf`, or `None` unless every
/// constant of the record holds. `wide` is true from Revit 2024.
pub fn decode_layout_at(buf: &[u8], t: usize, wide: bool) -> Option<Layout> {
    let w = if wide { 8 } else { 4 };
    // The earliest slot read: the spacing, 48 (60) bytes before T.
    if t < if wide { 48 } else { 60 } {
        return None;
    }
    let sketch_id = id_slot(buf, t - w, wide)??;
    if sketch_id == 0 {
        return None;
    }
    let type_id = id_slot(buf, t, wide)??;
    let beam_type_id = id_slot(buf, t - 2 * w, wide)?;
    let (spacing, maximum, elevation) = if wide {
        (
            f64_at(buf, t - 48)?,
            f64_at(buf, t - 40)?,
            f64_at(buf, t - 24)?,
        )
    } else {
        (
            f64_at(buf, t - 60)?,
            f64_at(buf, t - 52)?,
            f64_at(buf, t - 36)?,
        )
    };
    let tag = id_slot(buf, if wide { t + 8 } else { t + 4 }, wide)?;
    let (rule, justification, three_d, tail) = if wide {
        if u32_at(buf, t + 20)? != 6 || !matches!(u32_at(buf, t + 28)?, 6 | 7) {
            return None;
        }
        if !matches!(u32_at(buf, t + 32)?, 1 | u32::MAX) || u32_at(buf, t + 37)? != 1 {
            return None;
        }
        (
            u32_at(buf, t + 16)?,
            u32_at(buf, t + 24)?,
            *buf.get(t + 36)?,
            Some(t + 41),
        )
    } else {
        if u32_at(buf, t - 24)? != 6
            || !matches!(u32_at(buf, t - 16)?, 6 | 7)
            || !matches!(u32_at(buf, t - 12)?, 1 | u32::MAX)
        {
            return None;
        }
        (
            u32_at(buf, t - 28)?,
            u32_at(buf, t - 20)?,
            *buf.get(t + 8)?,
            None,
        )
    };
    if !matches!(rule, 1 | 3) || !(1..=4).contains(&justification) || three_d > 1 {
        return None;
    }
    let comments = tail.and_then(|at| comments_at(buf, at));
    Some(Layout {
        rule,
        justification,
        three_d: three_d == 1,
        spacing_feet: spacing,
        maximum_spacing_feet: maximum,
        elevation_feet: elevation,
        beam_type_id,
        type_id,
        sketch_id,
        tag_view_id: tag,
        comments,
    })
}

/// Comments after the layout constants: `i32 -1010106, i32 -1, u32 n, n UTF-16
/// units`. `None` where the slot says there are none, or anything is off.
fn comments_at(buf: &[u8], at: usize) -> Option<String> {
    if u32_at(buf, at)? as i32 != COMMENTS_PARAMETER || u32_at(buf, at + 4)? != u32::MAX {
        return None;
    }
    text_at(buf, at + 8)
}

/// What the copies of a Sketch's list say about its work plane.
#[derive(Default)]
struct WorkPlaneEvidence {
    /// The last Level of a `[Level, Plane, Sketch]` list that the file has.
    level: Option<u32>,
    /// Some list was `[Plane, Sketch]`.
    plain: bool,
}

impl WorkPlaneEvidence {
    /// Collect the evidence in `buf`: a counted list of three ids that holds
    /// `sketch` and a Level the file has, or of two that holds `sketch` and no
    /// Level. The ids of a list are in ascending order, so where `sketch` sits
    /// in it is not fixed.
    fn collect(
        &mut self,
        buf: &[u8],
        sketch: u32,
        owner: &[u32],
        levels: &BTreeSet<u32>,
        wide: bool,
    ) {
        let w = if wide { 8 } else { 4 };
        let key: Vec<u8> = if wide {
            u64::from(sketch).to_le_bytes().to_vec()
        } else {
            sketch.to_le_bytes().to_vec()
        };
        let id_at = |p: usize| -> Option<u32> {
            if wide {
                u64_at(buf, p).and_then(|v| u32::try_from(v).ok())
            } else {
                u32_at(buf, p)
            }
        };
        for at in memchr::memmem::find_iter(buf, &key) {
            for count in [3usize, 2] {
                for index in 0..count {
                    let Some(start) = at.checked_sub(w * index) else {
                        continue;
                    };
                    if start.checked_sub(4).and_then(|p| u32_at(buf, p)) != Some(count as u32) {
                        continue;
                    }
                    let Some(ids): Option<Vec<u32>> =
                        (0..count).map(|i| id_at(start + w * i)).collect()
                    else {
                        continue;
                    };
                    // `owner` (the system and its type) is in other lists that
                    // hold the Sketch; a list of the work plane has neither.
                    if ids
                        .iter()
                        .any(|id| *id == 0 || *id == u32::MAX || owner.contains(id))
                    {
                        continue;
                    }
                    let known: Vec<u32> = ids
                        .iter()
                        .copied()
                        .filter(|id| levels.contains(id))
                        .collect();
                    match (count, &known[..]) {
                        (3, [level]) => self.level = Some(*level),
                        (2, []) => self.plain = true,
                        _ => {}
                    }
                }
            }
        }
    }

    /// A Level, else not associated where only plain lists were seen.
    fn resolve(&self) -> Option<WorkPlane> {
        match (self.level, self.plain) {
            (Some(id), _) => Some(WorkPlane::Level(id)),
            (None, true) => Some(WorkPlane::NotAssociated),
            _ => None,
        }
    }
}

/// What the system's record gives the layout reader.
#[derive(Debug)]
pub struct SystemKey {
    /// The system's ElementId.
    pub system_id: u32,
    /// The first list of the system's record: its type, its phase, its Sketch
    /// and its members among others.
    pub first_list: Vec<u64>,
}

/// Layouts of the beam systems of a file, with their work planes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SystemLayouts {
    /// Layout by system ElementId.
    pub layouts: BTreeMap<u32, Layout>,
    /// Work plane by system ElementId.
    pub work_planes: BTreeMap<u32, WorkPlane>,
}

/// Read the layouts of `systems` from every partition of `rf`. The type id
/// `T` of the record is any entry of a system's first list (but a member beam,
/// in `members`): the record is accepted where every constant holds and the
/// Sketch before it is in the list of the same system. `levels` is the set of
/// Level ElementIds the Work Plane may name.
pub fn read(
    rf: &mut RevitFile,
    revit_version: u32,
    systems: &[SystemKey],
    members: &BTreeSet<u32>,
    levels: &BTreeSet<u32>,
) -> SystemLayouts {
    let wide = revit_version >= 2024;
    let mut by_type: BTreeMap<u32, Vec<&SystemKey>> = BTreeMap::new();
    for s in systems {
        for entry in &s.first_list {
            let Ok(id) = u32::try_from(*entry) else {
                continue;
            };
            if id != 0 && id != u32::MAX && !members.contains(&id) {
                by_type.entry(id).or_default().push(s);
            }
        }
    }
    // (partition number, offset) of the winning layout of each system.
    let mut best: BTreeMap<u32, ((u32, usize), Layout)> = BTreeMap::new();
    let mut streams: Vec<(u32, String)> = rf
        .partition_stream_names()
        .into_iter()
        .map(|s| (partition_number(&s), s))
        .collect();
    streams.sort();
    let mut buffers = Vec::new();
    for (number, stream) in &streams {
        let Ok(inflated) = rf.inflated_partition(stream) else {
            continue;
        };
        let buf = inflated.bytes();
        buffers.push((*number, inflated.clone()));
        for (type_id, keys) in &by_type {
            let needle: Vec<u8> = if wide {
                u64::from(*type_id).to_le_bytes().to_vec()
            } else {
                type_id.to_le_bytes().to_vec()
            };
            for t in memchr::memmem::find_iter(buf, &needle) {
                let Some(layout) = decode_layout_at(buf, t, wide) else {
                    continue;
                };
                let owners: Vec<u32> = keys
                    .iter()
                    .filter(|k| k.first_list.contains(&u64::from(layout.sketch_id)))
                    .map(|k| k.system_id)
                    .collect();
                let [owner] = owners[..] else { continue };
                let at = (*number, t);
                if best.get(&owner).is_none_or(|(old, _)| *old < at) {
                    best.insert(owner, (at, layout));
                }
            }
        }
    }
    let mut out = SystemLayouts::default();
    for (system, (_, layout)) in best {
        // Without the file's Levels a list cannot be told from one that names
        // a Level, so the work plane stays unknown.
        let mut evidence = WorkPlaneEvidence::default();
        for (_, inflated) in buffers.iter().filter(|_| !levels.is_empty()) {
            evidence.collect(
                inflated.bytes(),
                layout.sketch_id,
                &[system, layout.type_id],
                levels,
                wide,
            );
        }
        if let Some(plane) = evidence.resolve() {
            out.work_planes.insert(system, plane);
        }
        out.layouts.insert(system, layout);
    }
    out
}

/// Builtin parameter id of Description (`ALL_MODEL_DESCRIPTION`).
const DESCRIPTION_PARAMETER: i32 = -1_010_103;

/// How far past a type's data header its Description is looked for, at most:
/// the search also stops at the next data header.
const DESCRIPTION_WINDOW: usize = 0x4000;

/// The Description of each type in `wanted`. A type's value block (see
/// [`crate::partition_type_names`], Revit 2024) holds its string parameters
/// between the flat fields and the name, as `i64 id, u32 n, n UTF-16 units`,
/// and Description is the one with id -1010103. From Revit 2025 a type whose
/// data object opens with the measured element-data header
/// (`crate::partition_names::element_data_header`) holds them the same way,
/// as `i32 id, i32 -1, u32 n, n UTF-16 units`. The copy in the
/// highest-numbered partition wins. A type with no non-empty Description, or
/// on a release with neither, gets nothing.
pub fn type_descriptions(
    rf: &mut RevitFile,
    revit_version: u32,
    wanted: &BTreeSet<u32>,
) -> BTreeMap<u32, String> {
    let mut out = BTreeMap::new();
    if revit_version == crate::partition_type_names::MEASURED_REVIT_VERSION {
        let reads = crate::partition_type_names::read_type_names(rf, wanted)
            .map(|(reads, _)| reads)
            .unwrap_or_default();
        let id = i64::from(DESCRIPTION_PARAMETER).to_le_bytes();
        for (type_id, at) in reads {
            let Ok(inflated) = rf.inflated_partition(&at.stream) else {
                continue;
            };
            let buf = inflated.bytes();
            let Some(span) = buf.get(at.id_at + at.read.flat_end..at.id_at + at.read.name_at)
            else {
                continue;
            };
            if let Some(found) = memchr::memmem::find(span, &id) {
                if let Some(text) = text_at(span, found + 8) {
                    out.insert(type_id, text);
                }
            }
        }
        return out;
    }
    let Some(header) = crate::partition_names::element_data_header(revit_version) else {
        return out;
    };
    let mut streams: Vec<(u32, String)> = rf
        .partition_stream_names()
        .into_iter()
        .map(|s| (partition_number(&s), s))
        .collect();
    streams.sort();
    let mut needle = [0u8; 8];
    needle[..4].copy_from_slice(&DESCRIPTION_PARAMETER.to_le_bytes());
    needle[4..].copy_from_slice(&u32::MAX.to_le_bytes());
    for (_, stream) in &streams {
        let Ok(inflated) = rf.inflated_partition(stream) else {
            continue;
        };
        let buf = inflated.bytes();
        let starts: Vec<usize> = memchr::memmem::find_iter(buf, &header).collect();
        for (i, start) in starts.iter().enumerate() {
            let Some(id) = u64_at(buf, start + header.len()).and_then(|v| u32::try_from(v).ok())
            else {
                continue;
            };
            if !wanted.contains(&id) {
                continue;
            }
            let end = starts
                .get(i + 1)
                .copied()
                .unwrap_or(buf.len())
                .min(start + DESCRIPTION_WINDOW);
            let Some(span) = buf.get(*start..end) else {
                continue;
            };
            if let Some(at) = memchr::memmem::find(span, &needle) {
                if let Some(text) = text_at(span, at + 8) {
                    out.insert(id, text);
                }
            }
        }
    }
    out
}

/// A `u32 n` and `n` UTF-16 units at `at`, `n` in 1 to [`MAX_COMMENTS_UNITS`].
fn text_at(buf: &[u8], at: usize) -> Option<String> {
    let n = u32_at(buf, at)? as usize;
    if n == 0 || n > MAX_COMMENTS_UNITS {
        return None;
    }
    let units: Vec<u16> = buf
        .get(at + 4..at + 4 + 2 * n)?
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16(&units).ok()
}

/// Number of a `Partitions/N` stream (0 for any other name).
fn partition_number(stream: &str) -> u32 {
    stream
        .rsplit('/')
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}
