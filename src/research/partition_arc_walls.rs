//! Shared partition ArcWall iteration.
//!
//! The production walker (`iter_elements`) only scans `Global/Latest`
//! and therefore cannot see ArcWall instance records, which live in
//! `Partitions/*`. The IFC exporter previously inlined its own
//! partition scan; this module is the shared path for IFC export,
//! diagnostics, CLI counts, and ElemTable linkage.
//!
//! A record the byte pattern finds is kept only where the native record of
//! its ElementId is a wall, an `ArcWall` or `SWall`. On Revit 2023
//! `Revit_IFC5_Einhoven`, the one local file the pattern matches at all, its
//! 24 matches are the curtain wall's panels (15 `FamilyInstance`s), grid
//! lines (7 `CurtainGridLine`s) and 2 matches with no ElementId: none is a
//! wall, and taking them as walls shadowed the 15 panels the element records
//! read under the same ElementIds.

use crate::arc_wall_record::{
    ArcWallRecord, ArcWallScanStatus, ArcWallTrailer, STANDARD_RECORD_MIN_SIZE,
    STANDARD_TRAILER_DECODE_END,
};
use crate::element_record_storeys::elevation_storey_name;
use crate::ifc::Storey;
use crate::walker::{WalkerLimitHit, WalkerLimits};
use crate::{Result, RevitFile};
use std::collections::{BTreeMap, BTreeSet};

/// Stable reference to a decoded ArcWall record inside a partition
/// stream. Suitable as an ElemTable → partition index value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartitionArcWallRef {
    /// Stream name, e.g. `"Partitions/5"`.
    pub partition: String,
    /// Byte offset of the ArcWall tag inside the decompressed
    /// partition buffer.
    pub offset: usize,
}

/// One standard ArcWall recovered from a partition stream, including
/// optional trailer fields when the singleton trailer region is
/// present and validates.
#[derive(Debug, Clone, PartialEq)]
pub struct PartitionArcWall {
    pub partition: String,
    pub offset: usize,
    pub record: ArcWallRecord,
    pub trailer: Option<ArcWallTrailer>,
}

impl PartitionArcWall {
    /// ElemTable-linkable ElementId when the trailer validated.
    pub fn element_id(&self) -> Option<u32> {
        self.trailer
            .as_ref()
            .and_then(|t| t.element_id)
            .and_then(crate::ElementId::as_u32)
    }

    /// Shared type-symbol candidate from the trailer (`+0xfe`).
    pub fn type_id(&self) -> Option<u32> {
        self.trailer
            .as_ref()
            .and_then(|t| t.type_id)
            .and_then(crate::ElementId::as_u32)
    }

    /// Base elevation in feet (trailer `+0xf6`, else core start Z).
    pub fn base_elevation_feet(&self) -> Option<f64> {
        self.trailer
            .as_ref()
            .and_then(|t| t.base_elevation_feet)
            .or_else(|| {
                let z = self.record.start_point().2;
                z.is_finite().then_some(z)
            })
    }

    /// Unconnected height from the core Z delta when present.
    pub fn height_feet(&self) -> Option<f64> {
        self.record.height_feet()
    }

    /// Thickness is not present in the 2023 singleton ArcWall trailer.
    /// Always `None` until a WallType / HostObjAttr width
    /// join lands.
    pub fn thickness_feet(&self) -> Option<f64> {
        None
    }

    pub fn partition_ref(&self) -> PartitionArcWallRef {
        PartitionArcWallRef {
            partition: self.partition.clone(),
            offset: self.offset,
        }
    }
}

/// Version-scoped scan report across every `Partitions/*` stream.
#[derive(Debug, Clone, PartialEq)]
pub struct PartitionArcWallScan {
    pub status: ArcWallScanStatus,
    pub walls: Vec<PartitionArcWall>,
    /// Bytes inspected across all partition buffers (after inflate).
    pub scanned_bytes: usize,
    /// Present when a [`WalkerLimits`] cap stopped or truncated the scan.
    pub limit_hit: Option<WalkerLimitHit>,
}

/// Scan every `Partitions/*` stream for version-gated standard
/// ArcWall records. Unsupported Revit releases return an empty wall
/// list with an `UnsupportedVersion` status (no 2023 pattern applied).
pub fn scan_partition_arc_walls(
    rf: &mut RevitFile,
    revit_version: u32,
) -> Result<PartitionArcWallScan> {
    scan_partition_arc_walls_with_limits(rf, revit_version, WalkerLimits::default())
}

/// Same as [`scan_partition_arc_walls`], applying explicit
/// [`WalkerLimits`] to every partition buffer scan so crafted large
/// streams cannot force unbounded candidate materialisation.
///
/// The file keeps its latest scan (`RevitFile::memoised_arc_wall_scan`): a
/// repeat call with the same release and limits returns a copy of it, and a
/// call with other limits scans again.
pub fn scan_partition_arc_walls_with_limits(
    rf: &mut RevitFile,
    revit_version: u32,
    limits: WalkerLimits,
) -> Result<PartitionArcWallScan> {
    Ok(PartitionArcWallScan::clone(
        &rf.memoised_arc_wall_scan(revit_version, limits),
    ))
}

/// The scan [`scan_partition_arc_walls_with_limits`] describes, run in
/// full.
pub(crate) fn compute_partition_arc_walls(
    rf: &mut RevitFile,
    revit_version: u32,
    limits: WalkerLimits,
) -> PartitionArcWallScan {
    let status = ArcWallRecord::standard_decoder_status(revit_version);
    if !status.is_supported() {
        return PartitionArcWallScan {
            status,
            walls: Vec::new(),
            scanned_bytes: 0,
            limit_hit: None,
        };
    }

    let partition_streams = rf.partition_stream_names();

    let mut walls = Vec::new();
    let mut scanned_bytes = 0usize;
    let mut limit_hit = None;
    for partition in partition_streams {
        if walls.len() >= limits.max_candidates {
            limit_hit.get_or_insert(WalkerLimitHit::MaxCandidates);
            break;
        }
        let remaining = limits.max_candidates.saturating_sub(walls.len());
        let partition_limits = WalkerLimits {
            max_candidates: remaining,
            ..limits
        };
        // A stream an earlier reader already inflated is scanned in place;
        // any other is read member by member, never held whole.
        let scan = match rf.cached_partition(&partition) {
            Some(inflated) => scan_buffer(inflated.bytes(), partition_limits),
            None => {
                let Ok(raw) = rf.read_stream(&partition) else {
                    continue;
                };
                let (scan, inflate_hit, skipped) = scan_members(
                    &partition,
                    &raw,
                    rf.limits().inflate_limits,
                    partition_limits,
                );
                if let Some((code, message)) = inflate_hit {
                    rf.note_limit_event(format!("{code}: {message}"));
                }
                if let Some(event) = skipped {
                    rf.note_skip_event(event);
                }
                scan
            }
        };
        scanned_bytes = scanned_bytes.saturating_add(scan.scanned_bytes);
        if let Some(hit) = scan.limit_hit {
            limit_hit.get_or_insert(hit);
            rf.note_limit_event(format!("{}: {partition}", hit.code()));
        }
        for (offset, record, trailer) in scan.walls {
            walls.push(PartitionArcWall {
                partition: partition.clone(),
                offset,
                record,
                trailer,
            });
        }
        // A scan cap truncates this partition only: the next partition
        // is scanned from its start (a cap here used to stop
        // every later partition too).
        if limit_hit == Some(WalkerLimitHit::MaxCandidates) {
            break;
        }
    }

    if walls.len() >= limits.max_candidates {
        rf.note_limit_event(format!(
            "{}: partition ArcWall scan",
            WalkerLimitHit::MaxCandidates.code()
        ));
    }
    PartitionArcWallScan {
        status,
        walls: native_walls_only(rf, walls),
        scanned_bytes,
        limit_hit,
    }
}

/// The standard ArcWalls of one partition stream: offset, record, trailer.
struct StreamScan {
    walls: Vec<(usize, ArcWallRecord, Option<ArcWallTrailer>)>,
    scanned_bytes: usize,
    limit_hit: Option<WalkerLimitHit>,
}

/// Scan a whole inflated partition stream.
fn scan_buffer(concat: &[u8], limits: WalkerLimits) -> StreamScan {
    let mut out = StreamScan {
        walls: Vec::new(),
        scanned_bytes: 0,
        limit_hit: None,
    };
    if concat.len() < STANDARD_RECORD_MIN_SIZE {
        return out;
    }
    let found = ArcWallRecord::find_all_with_limits(concat, limits);
    out.scanned_bytes = found.scanned_bytes;
    out.limit_hit = found.limit_hit;
    for off in found.offsets {
        if let Ok(record) = ArcWallRecord::decode_standard(concat, off) {
            out.walls
                .push((off, record, ArcWallRecord::decode_trailer(concat, off)));
        }
    }
    out
}

/// Bytes past a record's start that decoding it reads: the record and its
/// trailer ([`STANDARD_TRAILER_DECODE_END`]).
const LOOKAHEAD: usize = STANDARD_TRAILER_DECODE_END;

/// [`scan_buffer`] over a stored partition stream read member by member
/// ([`crate::compression::for_each_member`]): the window held is
/// one gzip member plus the [`LOOKAHEAD`] bytes before it, not the stream,
/// and the walls found and their offsets are the ones [`scan_buffer`] finds
/// in the whole inflated stream. Returns the inflate limit that stopped the
/// read, if one did, and the `skipped-member` diagnostic.
fn scan_members(
    name: &str,
    stored: &[u8],
    inflate: crate::compression::InflateLimits,
    limits: WalkerLimits,
) -> (StreamScan, Option<(&'static str, String)>, Option<String>) {
    let mut out = StreamScan {
        walls: Vec::new(),
        scanned_bytes: 0,
        limit_hit: None,
    };
    // Positions a record may start at: [0, scan_len - MIN], where
    // scan_len = min(stream length, max_scan_bytes), as in find_all.
    let cap = limits.max_scan_bytes;
    let mut window: Vec<u8> = Vec::new();
    let mut window_start = 0usize; // stream offset of window[0]
    let mut done = false;
    // Decide every start position below `decide_to` (a stream offset).
    let decide =
        |window: &mut Vec<u8>, window_start: &mut usize, decide_to: usize, out: &mut StreamScan| {
            if decide_to <= *window_start {
                return;
            }
            let local_to = decide_to - *window_start;
            let slice_end = (local_to - 1 + STANDARD_RECORD_MIN_SIZE).min(window.len());
            let found = ArcWallRecord::find_all_with_limits(
                &window[..slice_end],
                WalkerLimits {
                    max_scan_bytes: usize::MAX,
                    max_candidates: limits.max_candidates.saturating_sub(out.walls.len()),
                    ..limits
                },
            );
            for off in found.offsets.into_iter().filter(|&o| o < local_to) {
                if let Ok(record) = ArcWallRecord::decode_standard(window, off) {
                    out.walls.push((
                        *window_start + off,
                        record,
                        ArcWallRecord::decode_trailer(window, off),
                    ));
                }
            }
            if found.limit_hit == Some(WalkerLimitHit::MaxCandidates) {
                out.limit_hit.get_or_insert(WalkerLimitHit::MaxCandidates);
            }
            window.drain(..local_to);
            *window_start = decide_to;
        };
    let (index, inflate_hit, skipped) =
        crate::compression::for_each_member(name, stored, inflate, |_, bytes| {
            window.extend_from_slice(bytes);
            let window_end = window_start + window.len();
            // Every position below this has its record and trailer in the window.
            let ready = window_end.saturating_sub(LOOKAHEAD - 1);
            let last = cap.saturating_sub(STANDARD_RECORD_MIN_SIZE - 1);
            decide(&mut window, &mut window_start, ready.min(last), &mut out);
            if ready >= last || out.limit_hit.is_some() {
                done = true;
            }
            !done
        });
    let total: usize = index.iter().map(|m| m.inflated_len).sum();
    let scan_len = total.min(cap);
    if !done && out.limit_hit.is_none() && scan_len >= STANDARD_RECORD_MIN_SIZE {
        decide(
            &mut window,
            &mut window_start,
            scan_len - STANDARD_RECORD_MIN_SIZE + 1,
            &mut out,
        );
    }
    // With `done` the rest of the stream was not inflated, so its length is
    // unknown; the cap was reached either way.
    if done && out.limit_hit.is_none() {
        out.limit_hit = Some(WalkerLimitHit::MaxScanBytes);
        out.scanned_bytes = cap;
    } else {
        out.scanned_bytes = scan_len;
        if total > cap {
            out.limit_hit.get_or_insert(WalkerLimitHit::MaxScanBytes);
        }
    }
    (out, inflate_hit, skipped)
}

/// The ArcWalls of the scan that are walls: those whose trailer type is not
/// a curtain panel's family symbol.
///
/// A loadable curtain panel family in a curtain wall keeps an ArcWall
/// record of its own as well as its `OST_CurtainWallPanels` element record.
/// Its trailer type is the panel's `FamilySymbol` (a wall's type is a
/// `WallType`, which has no element record on 2023), and its
/// trailer ElementId is not the panel's: on Einhoven all 24 ArcWalls are
/// the 24 "Family1" panels of curtain wall 7187, typed 8570, 7 of them
/// naming a curtain grid line's id and 2 none. Exported as walls they
/// doubled the panels as 0.82 ft boxes and, by taking the panels' ids
/// first, kept 15 panel records out of the export. The panels come from
/// their element records.
pub fn element_arc_walls(
    rf: &mut RevitFile,
    revit_version: u32,
    limits: WalkerLimits,
) -> Vec<PartitionArcWall> {
    use crate::partition_element_records::OST_CURTAIN_WALL_PANELS;
    let scan = rf.memoised_arc_wall_scan(revit_version, limits);
    if scan.walls.iter().all(|wall| wall.type_id().is_none()) {
        return scan.walls.clone();
    }
    let panel_types: std::collections::BTreeSet<u32> =
        crate::partition_element_records_2023::scan_records(rf, revit_version)
            .iter()
            .filter(|record| {
                record.is_type_symbol() && record.builtin_category == OST_CURTAIN_WALL_PANELS
            })
            .map(|record| record.element_id)
            .collect();
    scan.walls
        .iter()
        .filter(|wall| wall.type_id().is_none_or(|id| !panel_types.contains(&id)))
        .cloned()
        .collect()
}

/// The classes of a wall's native record.
const NATIVE_WALL_CLASSES: [&str; 2] = ["ArcWall", "SWall"];

/// The matches of `walls` whose ElementId's native record is a wall
/// ([`NATIVE_WALL_CLASSES`]). A match with no ElementId, or whose
/// record is of another class or not read, is left out, and so is every match
/// where the records cannot be read: the byte pattern alone is not evidence of
/// a wall. Only the records' classes are read, not their graphs.
fn native_walls_only(rf: &mut RevitFile, walls: Vec<PartitionArcWall>) -> Vec<PartitionArcWall> {
    use crate::native_document::{Options, extract_graphs};
    let selected_ids: BTreeSet<u64> = walls
        .iter()
        .filter_map(PartitionArcWall::element_id)
        .map(u64::from)
        .collect();
    if selected_ids.is_empty() {
        return Vec::new();
    }
    let options = Options {
        selected_ids,
        graph_classes: Some(BTreeSet::new()),
        ..Options::default()
    };
    let mut wall_ids = BTreeSet::new();
    let read = extract_graphs(rf, &options, |record| {
        if record
            .class_name
            .as_deref()
            .is_some_and(|class| NATIVE_WALL_CLASSES.contains(&class))
        {
            wall_ids.insert(record.identity.element_id);
        }
        Ok(())
    });
    if read.is_err() {
        return Vec::new();
    }
    walls
        .into_iter()
        .filter(|wall| {
            wall.element_id()
                .is_some_and(|id| wall_ids.contains(&u64::from(id)))
        })
        .collect()
}

/// Convenience wrapper: read BasicFileInfo version, then scan.
pub fn iter_partition_arc_walls(rf: &mut RevitFile) -> Result<PartitionArcWallScan> {
    let version = rf.layout_release()?;
    scan_partition_arc_walls(rf, version)
}

/// Build ElementId → partition-record index from decoded ArcWalls.
///
/// Only walls whose trailer ElementId validated are indexed. Duplicate
/// ElementIds keep the first occurrence (stable scan order).
pub fn element_id_partition_index(
    walls: &[PartitionArcWall],
) -> BTreeMap<u32, PartitionArcWallRef> {
    let mut map = BTreeMap::new();
    for wall in walls {
        if let Some(id) = wall.element_id() {
            map.entry(id).or_insert_with(|| wall.partition_ref());
        }
    }
    map
}

/// Result of deriving storeys from ArcWall elevations, optionally
/// labelled with partition Level-like display names.
#[derive(Debug, Clone, PartialEq)]
pub struct ArcWallStoreyRecovery {
    pub storeys: Vec<Storey>,
    /// Storeys whose name came from a partition Level-like string.
    pub named_from_partition: usize,
    /// Storeys that still use the `Elevation … ft` fallback label.
    pub elevation_fallback: usize,
    /// Count of building-storey name candidates considered.
    pub level_name_candidates: usize,
}

/// Derive IFC building storeys from distinct ArcWall base elevations.
///
/// When `level_names` contains confident building-storey labels
/// (see [`crate::partition_name_candidates::is_building_storey_name`]),
/// names are applied by ordinal (`Level 1` → lowest elevation, `Roof`
/// → highest) or by exact count zip. Unmatched storeys keep an
/// elevation fallback label — never invent a `"Level 1"` placeholder
/// for the whole model.
pub fn storeys_from_arc_wall_base_elevations(walls: &[PartitionArcWall]) -> Vec<Storey> {
    recover_storeys_from_arc_walls(walls, &[]).storeys
}

/// Like [`storeys_from_arc_wall_base_elevations`], but applies partition
/// Level-like display names when the match is confident enough.
pub fn recover_storeys_from_arc_walls(
    walls: &[PartitionArcWall],
    level_names: &[String],
) -> ArcWallStoreyRecovery {
    let mut elevations: Vec<f64> = walls
        .iter()
        .filter_map(PartitionArcWall::base_elevation_feet)
        .filter(|z| z.is_finite())
        .collect();
    elevations.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    elevations.dedup_by(|a, b| (*a - *b).abs() < 1e-6);

    let filtered: Vec<String> = level_names
        .iter()
        .filter(|n| crate::partition_name_candidates::is_building_storey_name(n))
        .cloned()
        .collect();

    let mut storeys: Vec<Storey> = elevations
        .iter()
        .map(|&elevation_feet| Storey {
            name: elevation_storey_name(elevation_feet),
            elevation_feet,
        })
        .collect();

    if storeys.is_empty() {
        return ArcWallStoreyRecovery {
            storeys,
            named_from_partition: 0,
            elevation_fallback: 0,
            level_name_candidates: filtered.len(),
        };
    }

    if filtered.len() == storeys.len() {
        let ordered = order_building_storey_names(&filtered);
        for (storey, name) in storeys.iter_mut().zip(ordered) {
            storey.name = name;
        }
    } else {
        apply_pattern_storey_names(&mut storeys, &filtered);
    }

    let named_from_partition = storeys
        .iter()
        .filter(|s| !s.name.starts_with("Elevation "))
        .count();
    let elevation_fallback = storeys.len() - named_from_partition;
    ArcWallStoreyRecovery {
        storeys,
        named_from_partition,
        elevation_fallback,
        level_name_candidates: filtered.len(),
    }
}

fn parse_level_number(name: &str) -> Option<u32> {
    let lower = name.trim().to_ascii_lowercase();
    let rest = lower.strip_prefix("level ")?;
    let digits: String = rest.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() || digits.len() != rest.chars().filter(|c| !c.is_whitespace()).count() {
        return None;
    }
    digits.parse().ok()
}

/// Sort recovered Level display names into building order —
/// basement, ground, numbered levels, mezzanine, roof — so a name
/// list can be zipped against an ascending elevation list when (and
/// only when) the two are the same length.
pub fn order_building_storey_names(names: &[String]) -> Vec<String> {
    let mut indexed: Vec<(i32, String)> = names
        .iter()
        .map(|name| {
            let lower = name.to_ascii_lowercase();
            let rank = if lower == "basement" {
                -2
            } else if lower == "ground floor" || lower == "groundfloor" {
                -1
            } else if lower == "roof" {
                10_000
            } else if let Some(n) = parse_level_number(name) {
                n as i32
            } else if lower == "first floor" {
                1
            } else if lower == "second floor" {
                2
            } else if lower == "third floor" {
                3
            } else if lower == "mezzanine" {
                50
            } else if lower == "podium" {
                0
            } else {
                100
            };
            (rank, name.clone())
        })
        .collect();
    indexed.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    indexed.into_iter().map(|(_, name)| name).collect()
}

fn apply_pattern_storey_names(storeys: &mut [Storey], names: &[String]) {
    for name in names {
        let lower = name.to_ascii_lowercase();
        if lower == "roof" {
            if let Some(last) = storeys.last_mut() {
                if last.name.starts_with("Elevation ") {
                    last.name = name.clone();
                }
            }
            continue;
        }
        if lower == "ground floor"
            || lower == "groundfloor"
            || lower == "basement"
            || lower == "podium"
        {
            if let Some(first) = storeys.first_mut() {
                if first.name.starts_with("Elevation ") {
                    first.name = name.clone();
                }
            }
            continue;
        }
        if let Some(n) = parse_level_number(name) {
            let idx = (n as usize).saturating_sub(1);
            if let Some(storey) = storeys.get_mut(idx) {
                if storey.name.starts_with("Elevation ") {
                    storey.name = name.clone();
                }
            }
        }
    }
}

/// Match a wall's base elevation to a storey index (nearest within
/// 1e-3 ft). Returns `None` when no storeys match.
pub fn storey_index_for_elevation(storeys: &[Storey], elevation_feet: f64) -> Option<usize> {
    storeys
        .iter()
        .enumerate()
        .find(|(_, storey)| (storey.elevation_feet - elevation_feet).abs() < 1e-3)
        .map(|(idx, _)| idx)
}
