//! The structural type of a Structural Framing instance, from its serialised
//! data in a Revit 2024 partition.
//!
//! Revit's IFC export writes a framing element as `IfcBeam`, `IfcMember`
//! (`.BRACE.`) or `IfcBuildingElementProxy` by the element's
//! `FamilyInstance.StructuralType` (Beam, Brace, NonStructural). Measured on
//! the 26 Revit 2024 samples that have a Revit IFC next to them
//! Of the 2,421 framing
//! instances Revit writes as one of those three classes, 2,338 are Beam and
//! IfcBeam, 81 are Brace and IfcMember, 2 are NonStructural and proxies.
//!
//! The data opens with `crate::partition_names::element_data_header` and the
//! element's `u64` ElementId. On a line-located framing instance the bytes
//! at data offsets `300..321` are all `0xff` and the `u32` at `+321` is the
//! structural type, `1` Beam and `2` Brace; the `u32` after it is the
//! Structural Usage (`3` Girder, `4` Joist, `6` Other, `11` and `12` truss
//! members, `0` Undefined, matching the API's raw values). Every one of the
//! 2,338 beams and 81 braces has that layout. The 2 NonStructural elements,
//! both point-located (`LocationPoint`), do not: their data has other bytes
//! there, which is the only thing this reads about them.
//!
//! 2022, 2023, 2024, 2026, 2027), scored against the API's `StructuralType`
//! on the 50 corpus models with framing (15,177 elements, all agree):
//!
//! - 2024, 2026, 2027 (`u64` ids): the layout above, at the same offsets.
//! - 2018 to 2023 (`u32` ids): the type is not at a fixed offset (the data
//!   before it varies in length), but always follows the same fields: a
//!   `double` `1.0` ends (`f0 3f`), then `ff ff ff ff`, a `u32`,
//!   `ff ff ff ff`, a `u32`, and four `i32` that are each `-12` or `-1`; the
//!   `u32` after them is the structural type (offset 257 to 263 on most
//!   elements).
//!
//! On every release a value other than `1` or `2` where the type sits, or
//! data without the layout, is a NonStructural element ([`FramingLayout::Other`]):
//! 64 of 64 NonStructural framing elements of the corpus.
//!
//! The data header is `ff ff ff ff`, the `u16` schema tag of the file's
//! `CellList` class and `01 00 00 00` ([`crate::partition_assoc_levels`]).
//!
//! # Honesty
//!
//! - Measured on 2018, 2019, 2022, 2023, 2024, 2026 and 2027 files. Other
//!   releases (2020, 2021, 2025) return nothing, and the caller keeps the
//!   category's `IfcBeam`.
//! - The layouts are measured, not parsed fields. Where the element's data
//!   is too short, nothing is read.
//! - That a NonStructural element is a proxy in Revit's IFC rests on 2
//!   elements of one family; the API agreement above covers the
//!   structural type, not the IFC class.

use crate::{Result, RevitFile};
use std::collections::{BTreeMap, BTreeSet};

/// the 2016 rst_basic sample types its 370 beams as its 2018 copy does;
pub fn supports_revit_version(revit_version: u32) -> bool {
    matches!(
        revit_version,
        2010 | 2011
            | 2012
            | 2013
            | 2014
            | 2015
            | 2016
            | 2017
            | 2018
            | 2019
            | 2020
            | 2021
            | 2022
            | 2023
            | 2024
            | 2026
            | 2027
    )
}

/// Offset in an element's data where the line-located layout's `0xff` run starts.
const RUN_START: usize = 300;
/// Offset of the `u32` structural type (the end of the run).
const TYPE_OFFSET: usize = 321;
/// Bytes of the element's data that must exist for a read.
const NEEDED: usize = TYPE_OFFSET + 8;

/// What an element's data says about its structural type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FramingLayout {
    /// Line-located, structural type `1`.
    Beam,
    /// Line-located, structural type `2`.
    Brace,
    /// Not the line-located layout (a point-located, non-structural element).
    Other,
}

/// The layout of each wanted framing element, from the newest partition
/// copy of its data. Empty for a release with no measured header.
pub fn scan_framing_layouts(
    rf: &mut RevitFile,
    revit_version: u32,
    wanted: &BTreeSet<u32>,
) -> Result<BTreeMap<u32, FramingLayout>> {
    if !supports_revit_version(revit_version) || wanted.is_empty() {
        return Ok(BTreeMap::new());
    }
    let Some(header) = data_header(rf) else {
        return Ok(BTreeMap::new());
    };
    let wide = revit_version >= 2024;
    let mut newest: BTreeMap<u32, (u64, Option<FramingLayout>)> = BTreeMap::new();
    for stream in rf.partition_stream_names() {
        let Ok(inflated) = rf.inflated_partition(&stream) else {
            continue;
        };
        let number = crate::partition_names::partition_number(&stream);
        let buf = inflated.bytes();
        let hits: Vec<usize> = memchr::memmem::find_iter(buf, &header).collect();
        for (n, &at) in hits.iter().enumerate() {
            let id_at = at + header.len();
            let id = if wide {
                buf.get(id_at..id_at + 8)
                    .map(|b| u64::from_le_bytes(b.try_into().expect("8 bytes")))
                    .and_then(|id| u32::try_from(id).ok())
            } else {
                buf.get(id_at..id_at + 4)
                    .map(|b| u32::from_le_bytes(b.try_into().expect("4 bytes")))
            };
            let Some(id) = id else {
                continue;
            };
            if !wanted.contains(&id) {
                continue;
            }
            let end = hits.get(n + 1).copied().unwrap_or(buf.len());
            let data = &buf[at..end];
            let layout = if wide {
                layout_of(data)
            } else {
                layout_of_u32(data)
            };
            if newest.get(&id).is_none_or(|(held, _)| number >= *held) {
                newest.insert(id, (number, layout));
            }
        }
    }
    Ok(newest
        .into_iter()
        .filter_map(|(id, (_, layout))| Some((id, layout?)))
        .collect())
}

/// The element-data header of the file: `ff ff ff ff`, its `CellList`
/// schema tag, `01 00 00 00`. `None` unless exactly one class has that name.
fn data_header(rf: &mut RevitFile) -> Option<[u8; 10]> {
    let classes = rf.schema_classes().ok()?;
    let mut tags = classes
        .classes
        .iter()
        .filter(|c| c.name == crate::partition_assoc_levels::ASSOC_CLASS)
        .map(|c| c.tag);
    let tag = tags.next()?;
    if tags.next().is_some() {
        return None;
    }
    let [lo, hi] = tag.to_le_bytes();
    Some([0xff, 0xff, 0xff, 0xff, lo, hi, 1, 0, 0, 0])
}

/// The structural type `u32` at `at`: `1` Beam, `2` Brace, anything else
/// NonStructural.
fn by_value(data: &[u8], at: usize) -> FramingLayout {
    match u32::from_le_bytes(data[at..at + 4].try_into().expect("4 bytes")) {
        1 => FramingLayout::Beam,
        2 => FramingLayout::Brace,
        _ => FramingLayout::Other,
    }
}

/// [`FramingLayout`] of one element's data (header first) on 2024 and
/// later, `None` when the data is too short to say.
fn layout_of(data: &[u8]) -> Option<FramingLayout> {
    if data.len() < NEEDED {
        return None;
    }
    if data[RUN_START..TYPE_OFFSET].iter().all(|&b| b == 0xff) {
        Some(by_value(data, TYPE_OFFSET))
    } else {
        Some(FramingLayout::Other)
    }
}

/// Bytes from the end of the `double` `1.0` to the structural type on 2018
/// to 2023: `ff ff ff ff`, `u32`, `ff ff ff ff`, `u32`, four `i32`.
const U32_ANCHOR_SPAN: usize = 32;

/// [`FramingLayout`] of one element's data (header first) on 2018 to 2023:
/// the type after the first run of the anchored fields (module doc), else
/// NonStructural. `None` when the data is too short to hold the fields.
fn layout_of_u32(data: &[u8]) -> Option<FramingLayout> {
    const ONE: [u8; 2] = [0xf0, 0x3f];
    const NONE: [u8; 4] = [0xff; 4];
    let i32_at = |at: usize| i32::from_le_bytes(data[at..at + 4].try_into().expect("4 bytes"));
    if data.len() < U32_ANCHOR_SPAN + 2 + 8 {
        return None;
    }
    let found = (U32_ANCHOR_SPAN + 2..data.len() - 8).find(|&t| {
        data[t - 34..t - 32] == ONE
            && data[t - 32..t - 28] == NONE
            && data[t - 24..t - 20] == NONE
            && [16, 12, 8, 4]
                .iter()
                .all(|&k| matches!(i32_at(t - k), -12 | -1))
    });
    Some(match found {
        Some(t) => by_value(data, t),
        None => FramingLayout::Other,
    })
}
