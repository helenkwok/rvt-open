//! The Level an element's association record names.
//!
//! For Revit 2024 there is one record per element in the inflated
//! `Partitions/*` streams that opens `<CellList tag u16> 01 00 00 00 · ElementId ·
//! LevelId · ff ff ff ff ..`. Its `LevelId` is the element's Level (a wall's Base
//! Constraint) where it is not `0xffffffff`. That reading needed the Level names
//! of `crate::partition_level_records::recover_partition_levels`, which are all
//! or nothing per file, and a fixed tag `0x02d3`. Both were file-specific:
//!
//! - the tag is the definition ordinal of the schema class `CellList`: `0x02d3`
//!   in 2024 files, `0x02fd` in the 2026 file measured, `0x0310` in 2027, `0x02c0` in 2023;
//! - 2024, 2026 and 2027 write ElementIds as `u64` (`id · 0 · level · 0`), 2022 and 2023 as `u32`
//!   (`id · level`); the 2022 tag is `0x028d`;
//! - the Level ids only need to be the Level records of the file
//!   (`crate::partition_level_records::scan_partition_level_ids`), not named ones.
//!
//! Fail closed: a release not measured, a schema with no or two `CellList`
//! classes, a level word outside the Level record ids, a record not followed by
//! the `ff ff ff ff` run, or two records of one element naming different Levels
//! yield no entry.

use crate::RevitFile;
use std::collections::{BTreeMap, BTreeSet};

/// Name of the schema class of the association record.
pub const ASSOC_CLASS: &str = "CellList";

/// Releases the layout was measured on.
pub fn supports_revit_version(revit_version: u32) -> bool {
    matches!(revit_version, 2022 | 2023 | 2024 | 2026 | 2027)
}

fn u32_at(buf: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(buf.get(at..at + 4)?.try_into().ok()?))
}

/// The `CellList` tag of the file's own schema, `None` unless exactly one class has that name.
fn assoc_tag(rf: &mut RevitFile) -> Option<u16> {
    let classes = rf.schema_classes().ok()?;
    let mut tags = classes
        .classes
        .iter()
        .filter(|c| c.name == ASSOC_CLASS)
        .map(|c| c.tag);
    let tag = tags.next()?;
    tags.next().is_none().then_some(tag)
}

/// Level ElementId by element, from the association records of `buf`.
fn scan(
    buf: &[u8],
    tag: u16,
    wide: bool,
    level_ids: &BTreeSet<u32>,
    out: &mut BTreeMap<u32, Option<u32>>,
) {
    let mark = [tag.to_le_bytes()[0], tag.to_le_bytes()[1], 1, 0, 0, 0];
    for at in memchr::memmem::find_iter(buf, &mark) {
        let id = at + mark.len();
        let step = if wide { 8 } else { 4 };
        let (Some(element), Some(level), Some(after)) = (
            u32_at(buf, id),
            u32_at(buf, id + step),
            u32_at(buf, id + 2 * step),
        ) else {
            continue;
        };
        if !level_ids.contains(&level) {
            continue;
        }
        // 2024 and later: the high words of both ids are zero; 2023: the level is
        // followed by the `ff ff ff ff` run.
        let shaped = if wide {
            u32_at(buf, id + 4) == Some(0) && u32_at(buf, id + 12) == Some(0)
        } else {
            after == u32::MAX
        };
        if !shaped {
            continue;
        }
        match out.get(&element) {
            Some(Some(held)) if *held != level => {
                out.insert(element, None);
            }
            Some(_) => {}
            None => {
                out.insert(element, Some(level));
            }
        }
    }
}

/// Level ElementId by ElementId, from every association record that names one of `level_ids`.
pub fn assoc_level_ids(
    rf: &mut RevitFile,
    revit_version: u32,
    level_ids: &BTreeSet<u32>,
) -> BTreeMap<u32, u32> {
    if !supports_revit_version(revit_version) || level_ids.is_empty() {
        return BTreeMap::new();
    }
    let Some(tag) = assoc_tag(rf) else {
        return BTreeMap::new();
    };
    let mut found: BTreeMap<u32, Option<u32>> = BTreeMap::new();
    for stream in rf.partition_stream_names() {
        let Ok(inflated) = rf.inflated_partition(&stream) else {
            continue;
        };
        scan(
            inflated.bytes(),
            tag,
            revit_version >= 2024,
            level_ids,
            &mut found,
        );
    }
    found
        .into_iter()
        .filter_map(|(element, level)| Some((element, level?)))
        .collect()
}
