//! A structural framing element's Reference Level, from the id lists of its
//! partition element record.
//!
//! The record carries two counted lists of ElementIds after its box (`u64`
//! slots from Revit 2024, `u32` slots up to 2023). A beam or brace names its
//! Reference Level in them, and often the Level below it too: the highest of
//! the Levels the second list names, else of those the first names, is the
//! Reference Level. An element saved in two partitions has a record in each
//! fitted this on the 27 public models: 2,419 of 2,419 beams; reading the
//! copy with the most list entries instead misses 10 of them in 3 models).
//!
//! Fail closed: no Level named, or two Levels at the highest elevation, or a
//! named Level without an elevation, gives nothing. The current copy is
//! chosen first, and if its first list cannot be decoded the beam gets
//! nothing: falling back to an older partition's copy would resurrect a
//! stale Reference Level (bounds review Fix A).

use crate::RevitFile;
use crate::partition_element_records as per;
use std::collections::{BTreeMap, BTreeSet};

/// Value of the level-bind source field for a Level read by this module.
pub const REFERENCE_LEVEL_SOURCE: &str = "partition_element_record_reference_level";

/// Field carrying the Level the other rules gave an element before this
/// module replaced it — or the same Level again, when the recovered
/// Reference Level equals it: the containment value then stays main's, and
/// only the provenance moves to the Reference Level reader (review
/// round 3). IfcBuildingStorey containment keeps using it. Revit's
/// own export contains a beam in the building story of its Reference Level
/// (the next lower story when that Level is not a building story), or in the
/// story at its beam system's base; that is the Reference Level itself for
/// only 794 of 1,968 beams on the public models.
pub const CONTAINMENT_LEVEL_FIELD: &str = "m_containmentLevelId";

/// Field carrying how [`CONTAINMENT_LEVEL_FIELD`] was chosen.
pub const CONTAINMENT_LEVEL_SOURCE_FIELD: &str = "m_containmentLevelSource";

/// Property the emitted element carries [`CONTAINMENT_LEVEL_FIELD`] on.
pub const CONTAINMENT_LEVEL_PROPERTY: &str = "ContainmentLevelElementId";

/// Property carrying [`CONTAINMENT_LEVEL_SOURCE_FIELD`].
pub const CONTAINMENT_LEVEL_SOURCE_PROPERTY: &str = "ContainmentLevelBindSource";

/// 0.10 metres expressed in feet.
pub const TEN_CENTIMETRES_FEET: f64 = 0.10 / 0.3048;

/// Value of StoreyBindSource when a beam is placed by its Structural Beam System base.
pub const STOREY_BIND_BEAM_SYSTEM_BASE: &str = "beam_system_base";

/// Value of StoreyBindSource when a beam is placed on its Reference Level storey.
pub const STOREY_BIND_REFERENCE_LEVEL_STORY: &str = "reference_level_story";

/// Value of StoreyBindSource when a beam is placed on the next storey below its Reference Level.
pub const STOREY_BIND_REFERENCE_LEVEL_BELOW: &str = "reference_level_below_story";

/// Value of StoreyBindSource when a non-story Reference Level has no storey below it,
/// falling back to the containment Level.
pub const STOREY_BIND_CONTAINMENT_FALLBACK: &str = "containment_level_fallback";

/// Value of StoreyBindSource when no other rule placed a beam (its Reference
/// Level is below every storey, or it has none): the storey of the Level with
/// the lowest id (seen on 6 beams and 2 elements with no level). A hypothesis, not a
/// documented rule.
pub const STOREY_BIND_LOWEST_ID_STOREY_FALLBACK: &str = "lowest_id_storey_fallback";

/// Value of StoreyBindSource when nothing placed the beam and the `open`
pub const STOREY_BIND_LOWEST_ELEVATION_FALLBACK: &str = "lowest_elevation_fallback";

/// Value of StoreyBindSource when beam-system membership is unknown for
/// the file and the beam kept the containment the Level reference list gave it.
pub const STOREY_BIND_MEMBERSHIP_UNKNOWN: &str = "beam_system_membership_unknown";

/// The three-way answer to "does this beam belong to a Structural Beam
/// System?", which the IfcBeam storey rule needs kept apart: only a
/// resolved `NoSystem` may take the Reference Level rule and only a
/// resolved `Member` the beam-system rule, while `Unknown` fails closed —
/// the IfcBeam storey rule does not run and the beam keeps the containment
/// the Level reference list gave it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BeamSystemMembership {
    /// Membership unknown for the file: the membership probe has not run,
    /// or the record's membership slot could not be decoded. Fail closed.
    Unknown,
    /// Resolved: the beam belongs to no Structural Beam System.
    NoSystem,
    /// Resolved: the beam belongs to the Structural Beam System with this
    /// ElementId.
    Member(u32),
}

/// ID width in element records: `u32` for 2018–2023, `u64` for 2024 and later.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdWidth {
    U32,
    U64,
}

/// Derive the 8-byte record tag from the file's schema: the `Outline` field tag,
/// then `ff ff ff ff`, then the `ElementParents` field tag.
pub fn schema_record_tag(rf: &mut RevitFile) -> Option<[u8; 8]> {
    // The classes' own schema tags, not their positions in the class list.
    crate::partition_element_records_2023::record_marker(rf).map(|(marker, _)| marker)
}

/// Decode the outline box: six little-endian `f64` values at `tag_offset + 8` in feet
/// (min_x, min_y, min_z, max_x, max_y, max_z).
pub fn decode_outline_box(buf: &[u8], tag_offset: usize) -> Option<[f64; 6]> {
    let at = tag_offset.checked_add(8)?;
    if at.checked_add(48)? > buf.len() {
        return None;
    }
    let mut box_feet = [0.0f64; 6];
    for (i, slot) in box_feet.iter_mut().enumerate() {
        let val = f64::from_le_bytes(buf[at + i * 8..at + (i + 1) * 8].try_into().ok()?);
        if !val.is_finite() {
            return None;
        }
        *slot = val;
    }
    Some(box_feet)
}

/// Decoded Structural Beam Systems and framing beam memberships.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BeamSystems {
    /// Beam system outline boxes by ElementId (min_z is bottom elevation in feet).
    systems: BTreeMap<u32, [f64; 6]>,
    /// Structural framing beam memberships by ElementId.
    memberships: BTreeMap<u32, BeamSystemMembership>,
    /// Whether the tag / schema was successfully read.
    tag_read_ok: bool,
    /// Own type of each beam system (the one type of the category its
    /// record's first list names). Filled by `read`.
    type_names: BTreeMap<u32, (u32, String)>,
    /// Phase created of each beam system, by name.
    phases: BTreeMap<u32, String>,
    /// First list of each beam system's current record.
    first_lists: BTreeMap<u32, Vec<u64>>,
    /// Layout record and work plane of each beam system.
    layouts: crate::beam_system_layout::SystemLayouts,
    /// Every Level of the file by ElementId: name and elevation in feet.
    levels: BTreeMap<u32, (String, f64)>,
    /// Description of each beam system type by type ElementId.
    type_descriptions: BTreeMap<u32, String>,
    /// Outline boxes of the member beams of each beam system (feet).
    member_boxes: BTreeMap<u32, Vec<[f64; 6]>>,
    /// Revit release of the file.
    revit_version: u32,
    /// Structural Trusses: each Truss a framing member's current
    /// record names in its first list, with its outline box and first list.
    trusses: BTreeMap<u32, Truss>,
    /// The Truss each framing member belongs to, by member ElementId.
    truss_members: BTreeMap<u32, u32>,
}

/// The category a truss type's name entry carries: not the Structural
/// Trusses category of the truss itself but `-2009600` (rstadvanced, the one
/// truss type of the 2 trusses, releases 2018-2024).
const TRUSS_TYPE_NAME_CATEGORY: i64 = -2_009_600;

/// A Structural Truss: what its own current record says.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Truss {
    /// Outline box in feet: min_x, min_y, min_z, max_x, max_y, max_z.
    pub outline_feet: [f64; 6],
    /// `(type id, type name)` of the truss type.
    pub type_name: Option<(u32, String)>,
    /// Family of the truss type.
    pub family_name: Option<String>,
    /// Phase created, by name.
    pub phase: Option<String>,
    /// The one Level the record's first list names: the Reference Level.
    pub reference_level: Option<u32>,
    first_list: Vec<u64>,
}

impl BeamSystems {
    /// The Structural Truss a framing member belongs to.
    pub fn truss_of(&self, member_id: u32) -> Option<u32> {
        self.truss_members.get(&member_id).copied()
    }

    /// A Structural Truss by ElementId.
    pub fn truss(&self, truss_id: u32) -> Option<&Truss> {
        self.trusses.get(&truss_id)
    }

    /// Name and elevation (feet) of a Level of the file.
    pub fn level(&self, level_id: u32) -> Option<(&str, f64)> {
        self.levels.get(&level_id).map(|(n, e)| (n.as_str(), *e))
    }

    /// Outline boxes of a Structural Beam System's member beams, in feet.
    pub fn member_boxes(&self, system_id: u32) -> &[[f64; 6]] {
        self.member_boxes.get(&system_id).map_or(&[], Vec::as_slice)
    }

    /// The Description of a Structural Beam System's own type, if it has one.
    pub fn type_description(&self, system_id: u32) -> Option<&str> {
        let type_id = self.layouts.layouts.get(&system_id)?.type_id;
        self.type_descriptions.get(&type_id).map(String::as_str)
    }

    /// Revit release of the file the systems were read from (0 if unread).
    pub fn revit_version(&self) -> u32 {
        self.revit_version
    }

    /// The layout record of a Structural Beam System.
    pub fn layout(&self, system_id: u32) -> Option<&crate::beam_system_layout::Layout> {
        self.layouts.layouts.get(&system_id)
    }

    /// The Level a Structural Beam System's work plane is, with its elevation
    /// in feet: `Some(Some((name, elevation)))` for a Level's plane,
    /// `Some(None)` for a plane that is not a Level's, `None` when unknown.
    #[allow(clippy::option_option)]
    pub fn work_plane(&self, system_id: u32) -> Option<Option<(&str, f64)>> {
        use crate::beam_system_layout::WorkPlane;
        match self.layouts.work_planes.get(&system_id)? {
            WorkPlane::NotAssociated => Some(None),
            WorkPlane::Level(id) => {
                let (name, elevation) = self.levels.get(id)?;
                Some(Some((name.as_str(), *elevation)))
            }
        }
    }

    /// Structural Beam System membership of a beam.
    pub fn membership(&self, beam_id: u32) -> BeamSystemMembership {
        if !self.tag_read_ok {
            return BeamSystemMembership::Unknown;
        }
        self.memberships
            .get(&beam_id)
            .copied()
            .unwrap_or(BeamSystemMembership::Unknown)
    }

    /// The bottom elevation (min z of bounding box) of a Structural Beam System in feet.
    pub fn bottom_elevation_feet(&self, system_id: u32) -> Option<f64> {
        self.systems.get(&system_id).map(|b| b[2])
    }

    /// `(type id, type name)` of a Structural Beam System's own type.
    pub fn type_names(&self, system_id: u32) -> Option<&(u32, String)> {
        self.type_names.get(&system_id)
    }

    /// Name of the phase a Structural Beam System was created in.
    pub fn phase_created(&self, system_id: u32) -> Option<&str> {
        self.phases.get(&system_id).map(String::as_str)
    }

    /// The outline box of a Structural Beam System in feet:
    /// min_x, min_y, min_z, max_x, max_y, max_z.
    pub fn outline_box(&self, system_id: u32) -> Option<[f64; 6]> {
        self.systems.get(&system_id).copied()
    }

    #[cfg(test)]
    pub(crate) fn from_systems_and_memberships(
        systems: BTreeMap<u32, [f64; 6]>,
        memberships: BTreeMap<u32, BeamSystemMembership>,
    ) -> Self {
        Self {
            systems,
            memberships,
            tag_read_ok: true,
            ..Self::default()
        }
    }

    /// Decode beam systems and memberships from raw partition slices.
    pub fn from_raw_partitions(
        tag: Option<[u8; 8]>,
        id_width: IdWidth,
        partitions: &[(u32, &[u8])],
    ) -> Self {
        let Some(tag) = tag else {
            return Self::default();
        };
        let mut collector = RecordCollector::new(tag, id_width);
        for &(part_num, buf) in partitions {
            collector.scan_partition(part_num, buf);
        }
        collector.finish()
    }

    /// Decode beam systems and memberships from partition slices with stream names (e.g. `"Partitions/1"`).
    pub fn from_partitions(
        tag: Option<[u8; 8]>,
        id_width: IdWidth,
        partitions: &[(&str, &[u8])],
    ) -> Self {
        let raw: Vec<(u32, &[u8])> = partitions
            .iter()
            .map(|(stream, buf)| (partition_number(stream), *buf))
            .collect();
        Self::from_raw_partitions(tag, id_width, &raw)
    }

    /// Read beam systems and memberships from a Revit file.
    pub fn read(rf: &mut RevitFile) -> Self {
        let tag = match schema_record_tag(rf) {
            Some(t) => t,
            None => return Self::default(),
        };
        let revit_version = rf.basic_file_info().ok().map(|b| b.version).unwrap_or(0);
        let id_width = if revit_version >= 2024 {
            IdWidth::U64
        } else {
            IdWidth::U32
        };
        // Records come from the crate's record scanners; the tag is searched for
        // after each record's start, since its distance from the start is not
        // fixed in every file (Snowdon Towers, Golden Nugget, DACH).
        use crate::partition_element_records_2023 as r23;
        let cats = [
            per::OST_STRUCTURAL_FRAMING,
            per::OST_STRUCTURAL_FRAMING_SYSTEM,
            per::OST_STRUCTURAL_TRUSS,
        ];
        let recs = if r23::is_u32_record_release(revit_version) {
            r23::scan_records(rf, revit_version)
        } else {
            let Ok(table) = crate::elem_table::parse_records(rf) else {
                return Self::default();
            };
            let declared = crate::elem_table::declared_ids(&table);
            match per::scan_category_records_multi(rf, revit_version, &cats, &declared) {
                Ok(r) => r,
                Err(_) => return Self::default(),
            }
        };
        let mut by_stream: BTreeMap<String, Vec<per::PartitionElementRecord>> = BTreeMap::new();
        for r in recs
            .into_iter()
            .filter(|r| cats.contains(&r.builtin_category))
        {
            by_stream.entry(r.stream.clone()).or_default().push(r);
        }
        let mut collector = RecordCollector::new(tag, id_width);
        for (stream, rs) in by_stream {
            let Ok(inflated) = rf.inflated_partition(&stream) else {
                continue;
            };
            let buf = inflated.bytes();
            for r in rs {
                collector.add_scanned(partition_number(&stream), buf, &r);
            }
        }
        let mut found = collector.finish();
        let (types, phases) = crate::partition_schema_mvp::types_and_phases_for_lists(
            rf,
            per::OST_STRUCTURAL_FRAMING_SYSTEM,
            &found.first_lists,
        );
        found.type_names = types;
        found.phases = phases;
        found.revit_version = revit_version;
        found.read_layouts(rf, revit_version);
        found.read_trusses(rf);
        found
    }
}

impl BeamSystems {
    /// Type, family, phase and Reference Level of each Truss with members,
    /// from the truss record's first list as for beam systems:
    /// the type the list names (`partition_names::resolve_type`), its family,
    /// the phase, and the one Level it names. Runs after `read_layouts`,
    /// which reads the Levels.
    fn read_trusses(&mut self, rf: &mut RevitFile) {
        if self.trusses.is_empty() {
            return;
        }
        let lists: BTreeMap<u32, Vec<u64>> = self
            .trusses
            .iter()
            .map(|(id, t)| (*id, t.first_list.clone()))
            .collect();
        let (_, phases) = crate::partition_schema_mvp::types_and_phases_for_lists(
            rf,
            per::OST_STRUCTURAL_TRUSS,
            &lists,
        );
        let names = rf.memoised_element_names();
        let level_ids: BTreeSet<u32> = self.levels.keys().copied().collect();
        for (id, truss) in &mut self.trusses {
            truss.phase = phases.get(id).cloned();
            if let Some(type_id) = crate::partition_names::resolve_type(
                &names,
                &truss.first_list,
                TRUSS_TYPE_NAME_CATEGORY,
                *id,
            ) {
                if let Some(entry) = names.entries.get(&type_id) {
                    truss.type_name = Some((type_id, entry.name.clone()));
                }
                truss.family_name = crate::partition_names::resolve_family(&names, type_id)
                    .and_then(|family| names.entries.get(&family))
                    .map(|entry| entry.name.clone());
            }
            if let [level] = named(&truss.first_list, &level_ids)[..] {
                truss.reference_level = Some(level);
            }
        }
    }

    /// Fill the layout records and work planes.
    fn read_layouts(&mut self, rf: &mut RevitFile, revit_version: u32) {
        let levels = crate::partition_level_records::recover_partition_levels(rf, revit_version)
            .unwrap_or_default();
        self.levels = levels
            .into_iter()
            .map(|l| (l.element_id, (l.name, l.elevation_feet)))
            .collect();
        let keys: Vec<crate::beam_system_layout::SystemKey> = self
            .first_lists
            .iter()
            .map(|(id, list)| crate::beam_system_layout::SystemKey {
                system_id: *id,
                first_list: list.clone(),
            })
            .collect();
        let members: BTreeSet<u32> = self
            .memberships
            .iter()
            .filter(|(_, m)| matches!(m, BeamSystemMembership::Member(_)))
            .map(|(id, _)| *id)
            .collect();
        let level_ids: BTreeSet<u32> = self.levels.keys().copied().collect();
        self.layouts =
            crate::beam_system_layout::read(rf, revit_version, &keys, &members, &level_ids);
        let type_ids: BTreeSet<u32> = self.layouts.layouts.values().map(|l| l.type_id).collect();
        // A system whose type the type-record read did not name: the name its
        // own type data object holds, for the type the layout record names.
        let unnamed: BTreeSet<u32> = self
            .layouts
            .layouts
            .iter()
            .filter(|(id, _)| !self.type_names.contains_key(id))
            .map(|(_, l)| l.type_id)
            .collect();
        if !unnamed.is_empty() {
            let names = crate::partition_schema_mvp::data_type_names(rf, revit_version, &unnamed);
            for (id, layout) in &self.layouts.layouts {
                if let (false, Some(name)) =
                    (self.type_names.contains_key(id), names.get(&layout.type_id))
                {
                    self.type_names.insert(*id, (layout.type_id, name.clone()));
                }
            }
        }
        self.type_descriptions =
            crate::beam_system_layout::type_descriptions(rf, revit_version, &type_ids);
    }
}

/// Read all beam systems and framing memberships from a Revit file.
pub fn read_beam_systems(rf: &mut RevitFile) -> BeamSystems {
    BeamSystems::read(rf)
}

/// Structural Beam System membership of a beam, from decoded [`BeamSystems`].
pub fn beam_system_for_beam(beam_id: u32, systems: &BeamSystems) -> BeamSystemMembership {
    systems.membership(beam_id)
}

/// The bottom elevation (min z of bounding box) of a Structural Beam System in feet.
pub fn beam_system_bottom_elevation_feet(system_id: u32, systems: &BeamSystems) -> Option<f64> {
    systems.bottom_elevation_feet(system_id)
}

/// The outline box of a Structural Beam System in feet:
/// min_x, min_y, min_z, max_x, max_y, max_z.
pub fn beam_system_outline_box(system_id: u32, systems: &BeamSystems) -> Option<[f64; 6]> {
    systems.outline_box(system_id)
}

struct FoundSystem {
    partition: u32,
    offset: usize,
    bbox: [f64; 6],
    first: Option<Vec<u64>>,
}

struct FoundBeam {
    partition: u32,
    offset: usize,
    first: Option<Vec<u64>>,
    bbox: Option<[f64; 6]>,
}

struct RecordCollector {
    tag: [u8; 8],
    id_width: IdWidth,
    beam_systems: BTreeMap<u32, Vec<FoundSystem>>,
    beams: BTreeMap<u32, Vec<FoundBeam>>,
    trusses: BTreeMap<u32, Vec<FoundSystem>>,
}

impl RecordCollector {
    fn new(tag: [u8; 8], id_width: IdWidth) -> Self {
        Self {
            tag,
            id_width,
            beam_systems: BTreeMap::new(),
            beams: BTreeMap::new(),
            trusses: BTreeMap::new(),
        }
    }

    fn first_list(&self, buf: &[u8], tag_offset: usize) -> Option<Vec<u64>> {
        match self.id_width {
            IdWidth::U32 => u32_list(buf, tag_offset + 56).map(|(l, _)| l),
            IdWidth::U64 => u64_list(buf, tag_offset + 56).map(|(l, _)| l),
        }
    }

    /// One record the crate's scanner found: the tag is the first one within
    /// 4 KiB after the record's start.
    fn add_scanned(&mut self, partition: u32, buf: &[u8], r: &per::PartitionElementRecord) {
        if r.element_id == 0 || r.placement_kind == per::PLACEMENT_KIND_SYMBOL {
            return;
        }
        let start = r.offset.min(buf.len());
        let end = (start + 4096).min(buf.len());
        let Some(k) = memchr::memmem::find(&buf[start..end], &self.tag) else {
            return;
        };
        let tag_offset = start + k;
        if r.builtin_category == per::OST_STRUCTURAL_TRUSS {
            if let Some(bbox) = decode_outline_box(buf, tag_offset) {
                let first = self.first_list(buf, tag_offset);
                self.trusses
                    .entry(r.element_id)
                    .or_default()
                    .push(FoundSystem {
                        partition,
                        offset: tag_offset,
                        bbox,
                        first,
                    });
            }
        } else if r.builtin_category == per::OST_STRUCTURAL_FRAMING_SYSTEM {
            if let Some(bbox) = decode_outline_box(buf, tag_offset) {
                let first = self.first_list(buf, tag_offset);
                self.beam_systems
                    .entry(r.element_id)
                    .or_default()
                    .push(FoundSystem {
                        partition,
                        offset: tag_offset,
                        bbox,
                        first,
                    });
            }
        } else {
            let first = match self.id_width {
                IdWidth::U32 => u32_list(buf, tag_offset + 56).map(|(l, _)| l),
                IdWidth::U64 => u64_list(buf, tag_offset + 56).map(|(l, _)| l),
            };
            self.beams.entry(r.element_id).or_default().push(FoundBeam {
                partition,
                offset: tag_offset,
                first,
                bbox: decode_outline_box(buf, tag_offset),
            });
        }
    }

    fn scan_partition(&mut self, partition: u32, buf: &[u8]) {
        for tag_offset in memchr::memmem::find_iter(buf, &self.tag) {
            let record_start = match self.id_width {
                IdWidth::U32 => match tag_offset.checked_sub(52) {
                    Some(s) => s,
                    None => continue,
                },
                IdWidth::U64 => match tag_offset.checked_sub(80) {
                    Some(s) => s,
                    None => continue,
                },
            };

            // Records with placement kind 0xFFFF8000 are family type/symbol envelopes, not beams: skip them.
            // Placement kind sits 14 bytes before the tag marker on both u32 and u64.
            if let Some(pk_at) = tag_offset.checked_sub(14) {
                if let Some(pk_bytes) = buf.get(pk_at..pk_at + 4) {
                    let pk = u32::from_le_bytes(pk_bytes.try_into().unwrap());
                    if pk == per::PLACEMENT_KIND_SYMBOL {
                        continue;
                    }
                }
            }

            // BuiltInCategory: i32 at record_start + 0x12 (and on u32 2018-2023 also tag_offset - 38).
            let cat_12 = buf
                .get(record_start + 0x12..record_start + 0x12 + 4)
                .map(|b| i32::from_le_bytes(b.try_into().unwrap()));
            let cat_38 = if self.id_width == IdWidth::U32 {
                tag_offset
                    .checked_sub(38)
                    .and_then(|at| buf.get(at..at + 4))
                    .map(|b| i32::from_le_bytes(b.try_into().unwrap()))
            } else {
                None
            };

            let category = if cat_12 == Some(per::OST_STRUCTURAL_FRAMING as i32)
                || cat_12 == Some(per::OST_STRUCTURAL_FRAMING_SYSTEM as i32)
            {
                cat_12
            } else if cat_38 == Some(per::OST_STRUCTURAL_FRAMING as i32)
                || cat_38 == Some(per::OST_STRUCTURAL_FRAMING_SYSTEM as i32)
            {
                cat_38
            } else {
                None
            };

            let Some(category) = category else {
                continue;
            };

            let element_id = match self.id_width {
                IdWidth::U32 => {
                    let Some(id_bytes) = buf.get(record_start..record_start + 4) else {
                        continue;
                    };
                    u32::from_le_bytes(id_bytes.try_into().unwrap())
                }
                IdWidth::U64 => {
                    let Some(id_bytes) = buf.get(record_start..record_start + 8) else {
                        continue;
                    };
                    let raw = u64::from_le_bytes(id_bytes.try_into().unwrap());
                    match u32::try_from(raw) {
                        Ok(id) => id,
                        Err(_) => continue,
                    }
                }
            };

            if element_id == 0 {
                continue;
            }

            if category == per::OST_STRUCTURAL_FRAMING_SYSTEM as i32 {
                if let Some(bbox) = decode_outline_box(buf, tag_offset) {
                    let first = self.first_list(buf, tag_offset);
                    self.beam_systems
                        .entry(element_id)
                        .or_default()
                        .push(FoundSystem {
                            partition,
                            offset: tag_offset,
                            bbox,
                            first,
                        });
                }
            } else if category == per::OST_STRUCTURAL_FRAMING as i32 {
                let first = match self.id_width {
                    IdWidth::U32 => u32_list(buf, tag_offset + 56).map(|(l, _)| l),
                    IdWidth::U64 => u64_list(buf, tag_offset + 56).map(|(l, _)| l),
                };
                self.beams.entry(element_id).or_default().push(FoundBeam {
                    partition,
                    offset: tag_offset,
                    first,
                    bbox: decode_outline_box(buf, tag_offset),
                });
            }
        }
    }

    fn finish(self) -> BeamSystems {
        let mut systems = BTreeMap::new();
        let mut first_lists = BTreeMap::new();
        let mut beam_system_ids = BTreeSet::new();

        for (id, copies) in self.beam_systems {
            if let Some(winner) = copies.iter().max_by_key(|c| (c.partition, c.offset)) {
                systems.insert(id, winner.bbox);
                if let Some(l) = &winner.first {
                    first_lists.insert(id, l.clone());
                }
                beam_system_ids.insert(id);
            }
        }

        // the current copy of each Structural Trusses record. Its
        // first list names the truss's type, phase and Reference Level; a
        // member's first list names the truss (rstadvanced, 26 of 26 members
        // of the 2 trusses, every release 2018-2024).
        let mut all_trusses: BTreeMap<u32, Truss> = BTreeMap::new();
        for (id, copies) in self.trusses {
            if let Some(winner) = copies.iter().max_by_key(|c| (c.partition, c.offset)) {
                all_trusses.insert(
                    id,
                    Truss {
                        outline_feet: winner.bbox,
                        first_list: winner.first.clone().unwrap_or_default(),
                        ..Truss::default()
                    },
                );
            }
        }
        let mut truss_members = BTreeMap::new();

        let mut memberships = BTreeMap::new();
        let mut member_boxes: BTreeMap<u32, Vec<[f64; 6]>> = BTreeMap::new();
        for (id, copies) in self.beams {
            let Some(winner) = copies.iter().max_by_key(|c| (c.partition, c.offset)) else {
                continue;
            };
            // A member names exactly one truss; two is not decoded.
            if let Some(list) = &winner.first {
                let mut named_trusses = list
                    .iter()
                    .filter_map(|&slot| u32::try_from(slot).ok())
                    .filter(|slot| *slot != id && all_trusses.contains_key(slot));
                if let (Some(truss), None) = (named_trusses.next(), named_trusses.next()) {
                    truss_members.insert(id, truss);
                }
            }
            let membership = match winner.first {
                None => BeamSystemMembership::Unknown,
                Some(ref list) => {
                    let mut found_system = None;
                    for &slot in list {
                        if let Ok(slot_id) = u32::try_from(slot) {
                            if beam_system_ids.contains(&slot_id) {
                                found_system = Some(slot_id);
                                break;
                            }
                        }
                    }
                    match found_system {
                        Some(sys_id) => {
                            if let Some(bbox) = winner.bbox {
                                member_boxes.entry(sys_id).or_default().push(bbox);
                            }
                            BeamSystemMembership::Member(sys_id)
                        }
                        None => BeamSystemMembership::NoSystem,
                    }
                }
            };
            memberships.insert(id, membership);
        }

        BeamSystems {
            systems,
            memberships,
            tag_read_ok: true,
            type_names: BTreeMap::new(),
            phases: BTreeMap::new(),
            first_lists,
            layouts: Default::default(),
            levels: BTreeMap::new(),
            type_descriptions: BTreeMap::new(),
            member_boxes,
            revit_version: 0,
            trusses: {
                let held: BTreeSet<u32> = truss_members.values().copied().collect();
                all_trusses.retain(|id, _| held.contains(id));
                all_trusses
            },
            truss_members,
        }
    }
}

/// Find the building storey index for a beam belonging to a Structural Beam System.
///
/// Rule: highest building storey whose elevation minus 0.10 m is at or below the bottom
/// (min z of the bounding box) of the beam system; if there is none, use the lowest building storey.
pub fn storey_index_for_beam_system(
    system_bottom_feet: f64,
    storeys: &[crate::ifc::Storey],
) -> Option<usize> {
    if storeys.is_empty() {
        return None;
    }
    storeys
        .iter()
        .enumerate()
        .rev()
        .find(|(_, s)| s.elevation_feet - TEN_CENTIMETRES_FEET <= system_bottom_feet + 1e-6)
        .map(|(idx, _)| idx)
        .or(Some(0))
}

/// Find the building storey index for a beam from its Reference Level.
///
/// Rule: use the beam's Reference Level if that Level is a building storey;
/// if not, use the next building storey below it (inclusive of equal elevation).
/// If there is no building storey below it, returns `None` so the caller retains
/// the current containment fallback.
pub fn storey_index_for_reference_level(
    ref_level_id: u32,
    storeys: &[crate::ifc::Storey],
    level_storey_bind: &crate::level_bind::LevelStoreyBind,
) -> Option<usize> {
    // 1. If Reference Level is a building storey, use it.
    if let Some(idx) = level_storey_bind.storey_index_for_level_id(ref_level_id) {
        return Some(idx);
    }
    // 2. Otherwise, use the next building storey below it (<=).
    let elevation = level_storey_bind.elevation_for_level_id(ref_level_id)?;
    storeys
        .iter()
        .enumerate()
        .rev()
        .find(|(_, s)| s.elevation_feet <= elevation + 1e-6)
        .map(|(idx, _)| idx)
}

/// The Levels two lists name, in list order, distinct.
fn named(list: &[u64], level_ids: &BTreeSet<u32>) -> Vec<u32> {
    crate::element_record_level_refs::named_levels(list, level_ids)
}

/// The highest of `levels` by elevation: the only one, or the one strictly
/// above all others. `None` when an elevation is missing or two tie on top.
fn highest(levels: &[u32], elevations: &BTreeMap<u32, f64>) -> Option<u32> {
    if let [only] = levels {
        return Some(*only);
    }
    let mut by_height: Vec<(f64, u32)> = levels
        .iter()
        .map(|id| Some((*elevations.get(id)?, *id)))
        .collect::<Option<_>>()?;
    by_height.sort_by(|a, b| b.0.total_cmp(&a.0));
    match by_height[..] {
        [(top, id), (next, _), ..] if top - next > 1e-6 => Some(id),
        _ => None,
    }
}

/// The Reference Level a record's two lists name.
fn level_of_lists(
    first: &[u64],
    second: &[u64],
    level_ids: &BTreeSet<u32>,
    elevations: &BTreeMap<u32, f64>,
) -> Option<u32> {
    let mut levels = named(second, level_ids);
    if levels.is_empty() {
        levels = named(first, level_ids);
    }
    highest(&levels, elevations)
}

/// Number of a `Partitions/N` stream (0 for any other name).
fn partition_number(stream: &str) -> u32 {
    stream
        .rsplit('/')
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

/// One record's lists, with where it was found. `first` is `None` when the
/// copy's first list could not be decoded; the copy is kept anyway so the
/// current-copy selection can fail closed for the beam instead of letting
/// an older partition's copy win (bounds review Fix A).
struct Found {
    partition: u32,
    offset: usize,
    first: Option<Vec<u64>>,
    second: Vec<u64>,
}

/// A `u32`-counted list of `u32` ElementIds at `at`, widened; with the byte
/// length it occupies.
fn u32_list(buf: &[u8], at: usize) -> Option<(Vec<u64>, usize)> {
    let count = u32::from_le_bytes(buf.get(at..at + 4)?.try_into().ok()?) as usize;
    if count > per::REFERENCE_LIST_MAX_ENTRIES {
        return None;
    }
    let body = buf.get(at + 4..at + 4 + count * 4)?;
    let list = body
        .chunks_exact(4)
        .map(|c| u64::from(u32::from_le_bytes([c[0], c[1], c[2], c[3]])))
        .collect();
    Some((list, 4 + count * 4))
}

/// A `u32`-counted list of `u64` ElementIds at `at`, with the byte length it
/// occupies. Returns `None` if the count word is missing, exceeds
/// [`per::REFERENCE_LIST_MAX_ENTRIES`], or extends past the buffer end.
fn u64_list(buf: &[u8], at: usize) -> Option<(Vec<u64>, usize)> {
    let count = u32::from_le_bytes(buf.get(at..at + 4)?.try_into().ok()?) as usize;
    if count > per::REFERENCE_LIST_MAX_ENTRIES {
        return None;
    }
    let body = buf.get(at + 4..at + 4 + count * 8)?;
    let list = body
        .chunks_exact(8)
        .map(|c| u64::from_le_bytes(c.try_into().expect("8 bytes")))
        .collect();
    Some((list, 4 + count * 8))
}

/// Records of `OST_StructuralFraming` by ElementId, `u64` releases.
fn records_u64(rf: &mut RevitFile, revit_version: u32) -> BTreeMap<u32, Vec<Found>> {
    let mut out: BTreeMap<u32, Vec<Found>> = BTreeMap::new();
    let Ok(table) = crate::elem_table::parse_records(rf) else {
        return out;
    };
    let declared = crate::elem_table::declared_ids(&table);
    let Ok(records) = per::scan_category_records_multi(
        rf,
        revit_version,
        &[per::OST_STRUCTURAL_FRAMING],
        &declared,
    ) else {
        return out;
    };
    for record in records {
        let Ok(inflated) = rf.inflated_partition(&record.stream) else {
            continue;
        };
        let buf = inflated.bytes();
        let at = record.offset + per::REFERENCE_LIST_OFFSET;
        // Keep the copy even when the first list is unreadable: the current
        // copy must win the selection and then fail closed, rather than an
        // older partition's copy winning by default (bounds review Fix A).
        let (first, second) = match u64_list(buf, at) {
            Some((first, used)) => (
                Some(first),
                u64_list(buf, at + used).map(|(l, _)| l).unwrap_or_default(),
            ),
            None => (None, Vec::new()),
        };
        out.entry(record.element_id).or_default().push(Found {
            partition: partition_number(&record.stream),
            offset: record.offset,
            first,
            second,
        });
    }
    out
}

/// Records of `OST_StructuralFraming` by ElementId, `u32` releases (the lists
/// start 56 bytes after the marker).
fn records_u32(rf: &mut RevitFile) -> BTreeMap<u32, Vec<Found>> {
    use crate::partition_element_records_2023 as r23;
    let mut out: BTreeMap<u32, Vec<Found>> = BTreeMap::new();
    let Ok(table) = crate::elem_table::parse_records(rf) else {
        return out;
    };
    let declared = crate::elem_table::declared_ids(&table);
    let Some((marker, header_tag)) = r23::record_marker(rf) else {
        return out;
    };
    for stream in rf.partition_stream_names() {
        let Ok(inflated) = rf.inflated_partition(&stream) else {
            continue;
        };
        let buf = inflated.bytes();
        for at in memchr::memmem::find_iter(buf, &marker) {
            let Some(category) = at
                .checked_sub(38)
                .and_then(|p| buf.get(p..p + 8))
                .map(|b| i64::from_le_bytes(b.try_into().expect("8 bytes")))
            else {
                continue;
            };
            if category != per::OST_STRUCTURAL_FRAMING {
                continue;
            }
            let Some((_, element_id)) = r23::find_header(buf, at, &declared, header_tag) else {
                continue;
            };
            let Some((first, used)) = u32_list(buf, at + 56) else {
                // Keep the copy so the current-copy selection fails closed
                // for the beam instead of an older copy winning (Fix A).
                out.entry(element_id).or_default().push(Found {
                    partition: partition_number(&stream),
                    offset: at,
                    first: None,
                    second: Vec::new(),
                });
                continue;
            };
            let second = u32_list(buf, at + 56 + used)
                .map(|(l, _)| l)
                .unwrap_or_default();
            out.entry(element_id).or_default().push(Found {
                partition: partition_number(&stream),
                offset: at,
                first: Some(first),
                second,
            });
        }
    }
    out
}

/// Reference Level ElementId by ElementId, for every structural framing
/// record whose lists name one. Empty on a release whose records are not
/// read, or a file without recovered Levels.
///
/// Fail closed on the current copy (bounds review Fix A): the highest
/// partition's copy is selected first, and when its first list is unreadable
/// (`Found::first == None`) the beam gets nothing — an older partition's
/// copy never wins.
pub fn reference_levels(
    rf: &mut RevitFile,
    revit_version: u32,
    level_ids: &BTreeSet<u32>,
    elevations: &BTreeMap<u32, f64>,
) -> BTreeMap<u32, u32> {
    if level_ids.is_empty() {
        return BTreeMap::new();
    }
    let records = if crate::partition_element_records_2023::is_u32_record_release(revit_version) {
        records_u32(rf)
    } else if per::supports_revit_version(revit_version) {
        records_u64(rf, revit_version)
    } else {
        return BTreeMap::new();
    };
    records
        .into_iter()
        .filter_map(|(id, copies)| {
            // this on the 27 public models). Select it first; an unreadable
            // first list then fails closed for the beam — falling back to an
            // older partition's copy would resurrect a stale Reference Level
            // (bounds review Fix A).
            let current = copies
                .iter()
                .max_by_key(|found| (found.partition, found.offset))?;
            let first = current.first.as_ref()?;
            let level = level_of_lists(first, &current.second, level_ids, elevations)?;
            Some((id, level))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Measured tags from the spec (for tests only)
    const TAG_2023: [u8; 8] = [0x3c, 0x01, 0xff, 0xff, 0xff, 0xff, 0x82, 0x05];
    const TAG_2024: [u8; 8] = [0x46, 0x01, 0xff, 0xff, 0xff, 0xff, 0xab, 0x05];

    #[derive(Debug, Clone, Copy)]
    struct SyntheticRecord {
        id_width: IdWidth,
        element_id: u32,
        category: i32,
        tag: [u8; 8],
        placement_kind: u32,
        bbox: [f64; 6],
        first_list: Option<&'static [u64]>,
        override_count: Option<u32>,
    }

    fn build_synthetic_record(rec: &SyntheticRecord) -> Vec<u8> {
        let SyntheticRecord {
            id_width,
            element_id,
            category,
            tag,
            placement_kind,
            bbox,
            first_list,
            override_count,
        } = *rec;
        let tag_offset = match id_width {
            IdWidth::U32 => 52,
            IdWidth::U64 => 80,
        };
        let mut buf = vec![0u8; tag_offset + 56];

        match id_width {
            IdWidth::U32 => {
                buf[0..4].copy_from_slice(&element_id.to_le_bytes());
            }
            IdWidth::U64 => {
                buf[0..8].copy_from_slice(&(element_id as u64).to_le_bytes());
            }
        }

        buf[0x12..0x12 + 4].copy_from_slice(&category.to_le_bytes());
        if id_width == IdWidth::U64 {
            buf[0x16..0x16 + 4].copy_from_slice(&[0xff, 0xff, 0xff, 0xff]);
        } else {
            let at_38 = tag_offset - 38;
            buf[at_38..at_38 + 4].copy_from_slice(&category.to_le_bytes());
        }

        let pk_at = tag_offset - 14;
        buf[pk_at..pk_at + 4].copy_from_slice(&placement_kind.to_le_bytes());

        buf[tag_offset..tag_offset + 8].copy_from_slice(&tag);

        for (i, val) in bbox.iter().enumerate() {
            let at = tag_offset + 8 + i * 8;
            buf[at..at + 8].copy_from_slice(&val.to_le_bytes());
        }

        if let Some(list) = first_list {
            let count = override_count.unwrap_or(list.len() as u32);
            buf.extend_from_slice(&count.to_le_bytes());
            match id_width {
                IdWidth::U32 => {
                    for &id in list {
                        buf.extend_from_slice(&(id as u32).to_le_bytes());
                    }
                }
                IdWidth::U64 => {
                    for &id in list {
                        buf.extend_from_slice(&id.to_le_bytes());
                    }
                }
            }
        }

        buf
    }

    #[test]
    fn test_u32_and_u64_id_widths() {
        // u32 id width (Revit 2018–2023)
        let sys_u32 = build_synthetic_record(&SyntheticRecord {
            id_width: IdWidth::U32,
            element_id: 200,
            category: -2001327,
            tag: TAG_2023,
            placement_kind: per::PLACEMENT_KIND_INSTANCE,
            bbox: [0.0, 0.0, 10.0, 1.0, 1.0, 12.0],
            first_list: Some(&[]),
            override_count: None,
        });
        let beam_u32 = build_synthetic_record(&SyntheticRecord {
            id_width: IdWidth::U32,
            element_id: 100,
            category: -2001320,
            tag: TAG_2023,
            placement_kind: per::PLACEMENT_KIND_INSTANCE,
            bbox: [0.0, 0.0, 10.0, 1.0, 1.0, 12.0],
            first_list: Some(&[200, 100]),
            override_count: None,
        });
        let mut part_u32 = sys_u32;
        part_u32.extend_from_slice(&beam_u32);

        let systems_u32 =
            BeamSystems::from_raw_partitions(Some(TAG_2023), IdWidth::U32, &[(1, &part_u32)]);
        assert_eq!(
            beam_system_for_beam(100, &systems_u32),
            BeamSystemMembership::Member(200)
        );

        // u64 id width (Revit 2024+)
        let sys_u64 = build_synthetic_record(&SyntheticRecord {
            id_width: IdWidth::U64,
            element_id: 400,
            category: -2001327,
            tag: TAG_2024,
            placement_kind: per::PLACEMENT_KIND_INSTANCE,
            bbox: [0.0, 0.0, 15.0, 2.0, 2.0, 18.0],
            first_list: Some(&[]),
            override_count: None,
        });
        let beam_u64 = build_synthetic_record(&SyntheticRecord {
            id_width: IdWidth::U64,
            element_id: 300,
            category: -2001320,
            tag: TAG_2024,
            placement_kind: per::PLACEMENT_KIND_INSTANCE,
            bbox: [0.0, 0.0, 15.0, 2.0, 2.0, 18.0],
            first_list: Some(&[400, 300]),
            override_count: None,
        });
        let mut part_u64 = sys_u64;
        part_u64.extend_from_slice(&beam_u64);

        let systems_u64 =
            BeamSystems::from_raw_partitions(Some(TAG_2024), IdWidth::U64, &[(1, &part_u64)]);
        assert_eq!(
            beam_system_for_beam(300, &systems_u64),
            BeamSystemMembership::Member(400)
        );
    }

    #[test]
    fn test_member_system_not_slot_before_own_id() {
        // First list: [200 (system), 500 (other id), 100 (own id)]
        // The slot before own id is 500, not 200. Scanning the whole list must find 200.
        let sys = build_synthetic_record(&SyntheticRecord {
            id_width: IdWidth::U64,
            element_id: 200,
            category: -2001327,
            tag: TAG_2024,
            placement_kind: per::PLACEMENT_KIND_INSTANCE,
            bbox: [0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            first_list: Some(&[]),
            override_count: None,
        });
        let beam = build_synthetic_record(&SyntheticRecord {
            id_width: IdWidth::U64,
            element_id: 100,
            category: -2001320,
            tag: TAG_2024,
            placement_kind: per::PLACEMENT_KIND_INSTANCE,
            bbox: [0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            first_list: Some(&[200, 500, 100]),
            override_count: None,
        });
        let mut part = sys;
        part.extend_from_slice(&beam);

        let systems = BeamSystems::from_raw_partitions(Some(TAG_2024), IdWidth::U64, &[(1, &part)]);
        assert_eq!(
            beam_system_for_beam(100, &systems),
            BeamSystemMembership::Member(200)
        );
    }

    #[test]
    fn test_non_member() {
        // Beam first list has no beam system id
        let sys = build_synthetic_record(&SyntheticRecord {
            id_width: IdWidth::U64,
            element_id: 200,
            category: -2001327,
            tag: TAG_2024,
            placement_kind: per::PLACEMENT_KIND_INSTANCE,
            bbox: [0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            first_list: Some(&[]),
            override_count: None,
        });
        let beam = build_synthetic_record(&SyntheticRecord {
            id_width: IdWidth::U64,
            element_id: 100,
            category: -2001320,
            tag: TAG_2024,
            placement_kind: per::PLACEMENT_KIND_INSTANCE,
            bbox: [0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            first_list: Some(&[500, 100]),
            override_count: None,
        });
        let mut part = sys;
        part.extend_from_slice(&beam);

        let systems = BeamSystems::from_raw_partitions(Some(TAG_2024), IdWidth::U64, &[(1, &part)]);
        assert_eq!(
            beam_system_for_beam(100, &systems),
            BeamSystemMembership::NoSystem
        );
    }

    #[test]
    fn test_beam_first_list_names_id_that_is_not_beam_system() {
        // Element 999 is category -2000011 (Walls), not a beam system (-2001327)
        let wall = build_synthetic_record(&SyntheticRecord {
            id_width: IdWidth::U64,
            element_id: 999,
            category: -2000011,
            tag: TAG_2024,
            placement_kind: per::PLACEMENT_KIND_INSTANCE,
            bbox: [0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            first_list: Some(&[]),
            override_count: None,
        });
        let beam = build_synthetic_record(&SyntheticRecord {
            id_width: IdWidth::U64,
            element_id: 100,
            category: -2001320,
            tag: TAG_2024,
            placement_kind: per::PLACEMENT_KIND_INSTANCE,
            bbox: [0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            first_list: Some(&[999, 100]),
            override_count: None,
        });
        let mut part = wall;
        part.extend_from_slice(&beam);

        let systems = BeamSystems::from_raw_partitions(Some(TAG_2024), IdWidth::U64, &[(1, &part)]);
        assert_eq!(
            beam_system_for_beam(100, &systems),
            BeamSystemMembership::NoSystem
        );
    }

    #[test]
    fn test_two_partitions_highest_one_winning() {
        let sys200 = build_synthetic_record(&SyntheticRecord {
            id_width: IdWidth::U64,
            element_id: 200,
            category: -2001327,
            tag: TAG_2024,
            placement_kind: per::PLACEMENT_KIND_INSTANCE,
            bbox: [0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            first_list: Some(&[]),
            override_count: None,
        });
        let sys300 = build_synthetic_record(&SyntheticRecord {
            id_width: IdWidth::U64,
            element_id: 300,
            category: -2001327,
            tag: TAG_2024,
            placement_kind: per::PLACEMENT_KIND_INSTANCE,
            bbox: [0.0, 0.0, 5.0, 1.0, 1.0, 6.0],
            first_list: Some(&[]),
            override_count: None,
        });

        // In partition 1: beam 100 belongs to sys 200
        let beam_p1 = build_synthetic_record(&SyntheticRecord {
            id_width: IdWidth::U64,
            element_id: 100,
            category: -2001320,
            tag: TAG_2024,
            placement_kind: per::PLACEMENT_KIND_INSTANCE,
            bbox: [0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            first_list: Some(&[200, 100]),
            override_count: None,
        });
        let mut part1 = sys200;
        part1.extend_from_slice(&beam_p1);

        // In partition 2: beam 100 belongs to sys 300
        let beam_p2 = build_synthetic_record(&SyntheticRecord {
            id_width: IdWidth::U64,
            element_id: 100,
            category: -2001320,
            tag: TAG_2024,
            placement_kind: per::PLACEMENT_KIND_INSTANCE,
            bbox: [0.0, 0.0, 5.0, 1.0, 1.0, 6.0],
            first_list: Some(&[300, 100]),
            override_count: None,
        });
        let mut part2 = sys300;
        part2.extend_from_slice(&beam_p2);

        let systems = BeamSystems::from_raw_partitions(
            Some(TAG_2024),
            IdWidth::U64,
            &[(1, &part1), (2, &part2)],
        );
        // Partition 2 is higher numbered, so it wins
        assert_eq!(
            beam_system_for_beam(100, &systems),
            BeamSystemMembership::Member(300)
        );

        // Fix A / Fail closed: if the highest partition copy is unreadable, it fails closed to Unknown
        // and does NOT fall back to partition 1.
        let beam_p2_corrupt = build_synthetic_record(&SyntheticRecord {
            id_width: IdWidth::U64,
            element_id: 101,
            category: -2001320,
            tag: TAG_2024,
            placement_kind: per::PLACEMENT_KIND_INSTANCE,
            bbox: [0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            first_list: Some(&[300, 101]),
            override_count: Some(2000), // > 1024 cap: unreadable
        });
        let beam_p1_valid = build_synthetic_record(&SyntheticRecord {
            id_width: IdWidth::U64,
            element_id: 101,
            category: -2001320,
            tag: TAG_2024,
            placement_kind: per::PLACEMENT_KIND_INSTANCE,
            bbox: [0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            first_list: Some(&[200, 101]),
            override_count: None,
        });
        let mut p1 = part1;
        p1.extend_from_slice(&beam_p1_valid);
        let mut p2 = part2;
        p2.extend_from_slice(&beam_p2_corrupt);

        let systems_corrupt =
            BeamSystems::from_raw_partitions(Some(TAG_2024), IdWidth::U64, &[(1, &p1), (2, &p2)]);
        assert_eq!(
            beam_system_for_beam(101, &systems_corrupt),
            BeamSystemMembership::Unknown
        );
    }

    #[test]
    fn test_unreadable_tag_gives_unknown() {
        let sys = build_synthetic_record(&SyntheticRecord {
            id_width: IdWidth::U64,
            element_id: 200,
            category: -2001327,
            tag: TAG_2024,
            placement_kind: per::PLACEMENT_KIND_INSTANCE,
            bbox: [0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            first_list: Some(&[]),
            override_count: None,
        });
        let beam = build_synthetic_record(&SyntheticRecord {
            id_width: IdWidth::U64,
            element_id: 100,
            category: -2001320,
            tag: TAG_2024,
            placement_kind: per::PLACEMENT_KIND_INSTANCE,
            bbox: [0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            first_list: Some(&[200, 100]),
            override_count: None,
        });
        let mut part = sys;
        part.extend_from_slice(&beam);

        // tag is None: unreadable tag
        let systems = BeamSystems::from_raw_partitions(None, IdWidth::U64, &[(1, &part)]);
        assert_eq!(
            beam_system_for_beam(100, &systems),
            BeamSystemMembership::Unknown
        );
        assert_eq!(beam_system_bottom_elevation_feet(200, &systems), None);
    }

    #[test]
    fn test_list_count_over_1024_cap_rejected() {
        let sys = build_synthetic_record(&SyntheticRecord {
            id_width: IdWidth::U64,
            element_id: 200,
            category: -2001327,
            tag: TAG_2024,
            placement_kind: per::PLACEMENT_KIND_INSTANCE,
            bbox: [0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            first_list: Some(&[]),
            override_count: None,
        });
        // List count is 1025 (> 1024 cap)
        let beam = build_synthetic_record(&SyntheticRecord {
            id_width: IdWidth::U64,
            element_id: 100,
            category: -2001320,
            tag: TAG_2024,
            placement_kind: per::PLACEMENT_KIND_INSTANCE,
            bbox: [0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            first_list: Some(&[200, 100]),
            override_count: Some(1025),
        });
        let mut part = sys;
        part.extend_from_slice(&beam);

        let systems = BeamSystems::from_raw_partitions(Some(TAG_2024), IdWidth::U64, &[(1, &part)]);
        assert_eq!(
            beam_system_for_beam(100, &systems),
            BeamSystemMembership::Unknown
        );
    }

    #[test]
    fn test_outline_box_decoded_to_six_f64() {
        let expected_box = [1.25, -2.5, -15.75, 100.0, 200.5, 30.0];
        let sys = build_synthetic_record(&SyntheticRecord {
            id_width: IdWidth::U64,
            element_id: 200,
            category: -2001327,
            tag: TAG_2024,
            placement_kind: per::PLACEMENT_KIND_INSTANCE,
            bbox: expected_box,
            first_list: Some(&[]),
            override_count: None,
        });

        // Test decode_outline_box directly at tag offset 80
        let decoded = decode_outline_box(&sys, 80).expect("decode_outline_box");
        assert_eq!(decoded, expected_box);

        // Test via BeamSystems outline_box and bottom_elevation_feet
        let systems = BeamSystems::from_raw_partitions(Some(TAG_2024), IdWidth::U64, &[(1, &sys)]);
        assert_eq!(beam_system_outline_box(200, &systems), Some(expected_box));
        assert_eq!(
            beam_system_bottom_elevation_feet(200, &systems),
            Some(-15.75)
        );
    }

    #[test]
    fn test_symbol_placement_kind_skipped() {
        // Records with placement kind 0xFFFF8000 are family type/symbol envelopes, not beams: skip them.
        let beam_symbol = build_synthetic_record(&SyntheticRecord {
            id_width: IdWidth::U64,
            element_id: 100,
            category: -2001320,
            tag: TAG_2024,
            placement_kind: per::PLACEMENT_KIND_SYMBOL, // 0xffff8000
            bbox: [0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            first_list: Some(&[200, 100]),
            override_count: None,
        });
        let systems =
            BeamSystems::from_raw_partitions(Some(TAG_2024), IdWidth::U64, &[(1, &beam_symbol)]);
        // Skipped, so membership is Unknown (no instance record)
        assert_eq!(
            beam_system_for_beam(100, &systems),
            BeamSystemMembership::Unknown
        );
    }
}
