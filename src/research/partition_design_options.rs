//! Design option sets and their primary options.
//!
//! An element that belongs to a design option carries the option's
//! ElementId at `+0x2a` of its element record
//! ([`crate::partition_element_records::DESIGN_OPTION_OFFSET`]). Revit's own
//! IFC export writes the main model and each option set's primary option,
//! and leaves the other options out. Which option is primary is stored in
//! the set's name entry:
//!
//! ```text
//! option-set partition record (OST_DesignOptionSets, in the chain)
//!   +0x00  u64  the set's ElementId
//!   +0x12  i64  -2006112
//!   +0x56  u32  k
//!   +0x5a  k*8  the set's own ElementId, then its k - 1 options
//!
//! option-set name entry
//!   u32  k - 1
//!   ...  the same options, u64 each, in the same order
//!   u32  1
//!   u32  n, the name's length in UTF-16 code units
//!   n*2  the set's name, UTF-16LE
//!   u64  the ElementId of the set's primary option
//! ```
//!
//! On Autodesk's Snowdon Towers 2024 architectural sample the two sets are
//! "Bandstand Options" (four options, primary 1500724) and "Bleacher
//! Seating" (two options, primary 2059970). Each set's name entry occurs
//! once in the file, and its primary is the option whose elements Revit's
//! IFC4 export holds. Of the placed instances of recovered categories, the
//! 27 in a non-primary option (9 floors, 8 walls and 9 slab edges in
//! 2059967, one generic model in 1500727) are all missing from it, and the
//! 29 in 2059970 are all in it.
//!
//! The option records themselves (`OST_DesignOptions`) differ only in their
//! ids, and a set lists its options in ascending id order, so neither says
//! which option is primary.
//!
//! # 2018-2023
//!
//! The `u32` record releases keep the same structures with `u32`
//! ElementIds: an element's option 26 bytes before its record marker
//! ([`crate::partition_element_records_2023::DESIGN_OPTION_BEFORE_MARKER`]),
//! the set list after `0xFF` x 4 and the `ElementParents` tag, with the
//! set's category 44 bytes before it ([`option_set_list`]), the name entry
//! and the option names. Measured against the API dump on 11 models of
//! 2018, 2019, 2022 and 2023: every set, primary and option name as the
//! API's. Sets are also found by that marker on 2024 on, outside the
//! partitions' leading record chains (cjk_138949ac 2024, cjk_1706fd8d
//! 2026).
//!
//! # Fail closed
//!
//! A set whose name entry is missing, or whose entries disagree about the
//! primary, stays unresolved, and elements in its options are kept as
//! before. Only an option of a resolved set that is not its primary is
//! reported as non-primary.

use crate::le_bytes;
use crate::partition_element_records::{
    CATEGORY_OFFSET, PartitionElementRecord, partition_record_chain, supports_revit_version,
};
use crate::{Result, RevitFile};
use std::collections::{BTreeMap, BTreeSet};

/// Autodesk `BuiltInCategory.OST_DesignOptions`.
pub const OST_DESIGN_OPTIONS: i64 = -2_006_114;

/// Autodesk `BuiltInCategory.OST_DesignOptionSets`.
pub const OST_DESIGN_OPTION_SETS: i64 = -2_006_112;

/// Offset, from the start of an option-set partition record, of the `u32`
/// count that opens the set's id list.
pub const OPTION_LIST_OFFSET: usize = 0x56;

/// Most options a set is read with. A count above it is not a set list.
pub const OPTIONS_PER_SET_MAX: usize = 64;

/// Longest set name, in UTF-16 code units, read from a name entry.
pub const SET_NAME_MAX_UNITS: usize = 256;

/// A design option set whose primary option was read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesignOptionSet {
    /// The set's ElementId.
    pub set_id: u32,
    /// The set's name as stored.
    pub name: String,
    /// The set's options, in the order the set lists them.
    pub options: Vec<u32>,
    /// The primary option, one of [`Self::options`].
    pub primary: u32,
    /// Each option's name, where its record gives one ([`find_option_name`]).
    pub option_names: BTreeMap<u32, String>,
}

/// Every design option set of a file.
#[derive(Debug, Clone, Default)]
pub struct DesignOptions {
    /// Sets whose primary option was read, by set ElementId.
    pub sets: BTreeMap<u32, DesignOptionSet>,
    /// Sets found in a partition record chain whose name entry is missing
    /// or ambiguous. Their options are neither primary nor non-primary.
    pub unresolved_sets: BTreeSet<u32>,
    /// Keep the elements of non-primary options (the `open` convention,
    pub keep_non_primary: bool,
}

impl DesignOptions {
    /// Whether `option` belongs to a resolved set and is not its primary.
    pub fn is_non_primary(&self, option: u32) -> bool {
        self.sets
            .values()
            .any(|set| set.primary != option && set.options.contains(&option))
    }

    /// Every option of a resolved set other than its primary.
    pub fn non_primary_options(&self) -> BTreeSet<u32> {
        self.sets
            .values()
            .flat_map(|set| set.options.iter().copied().filter(|o| *o != set.primary))
            .collect()
    }

    /// Whether `record` is in a non-primary design option, which Revit's
    /// export leaves out; never when [`Self::keep_non_primary`].
    pub fn excludes(&self, record: &PartitionElementRecord) -> bool {
        record
            .design_option
            .is_some_and(|option| self.excludes_option(option))
    }

    /// Whether elements of `option` are left out of the export.
    pub fn excludes_option(&self, option: u32) -> bool {
        !self.keep_non_primary && self.is_non_primary(option)
    }

    /// The resolved set `option` belongs to.
    pub fn set_of(&self, option: u32) -> Option<&DesignOptionSet> {
        self.sets.values().find(|set| set.options.contains(&option))
    }
}

/// The options an option-set partition record lists, or `None` when the
/// record in `buf[start..end]` is not one: its category must be
/// [`OST_DESIGN_OPTION_SETS`], and the list at [`OPTION_LIST_OFFSET`] must
/// start with the record's own `set_id`, stay inside the record, and name
/// at least one option distinct from the set.
pub fn option_set_members(buf: &[u8], start: usize, end: usize, set_id: u64) -> Option<Vec<u32>> {
    let category = le_bytes::u64_at(buf, start.checked_add(CATEGORY_OFFSET)?)? as i64;
    if category != OST_DESIGN_OPTION_SETS {
        return None;
    }
    let list_at = start.checked_add(OPTION_LIST_OFFSET)?;
    let count = le_bytes::u32_at(buf, list_at)? as usize;
    if !(2..=OPTIONS_PER_SET_MAX + 1).contains(&count) {
        return None;
    }
    let ids_at = list_at.checked_add(4)?;
    if ids_at.checked_add(count.checked_mul(8)?)? > end {
        return None;
    }
    if le_bytes::u64_at(buf, ids_at)? != set_id {
        return None;
    }
    let mut options = Vec::with_capacity(count - 1);
    for index in 1..count {
        let id = le_bytes::u64_at(buf, ids_at + index * 8)?;
        let id = u32::try_from(id).ok()?;
        if u64::from(id) == set_id || options.contains(&id) {
            return None;
        }
        options.push(id);
    }
    Some(options)
}

/// Bytes from a set list to its record's `BuiltInCategory`, for `u64`
/// ElementIds (2024 on: [`OPTION_LIST_OFFSET`] - [`CATEGORY_OFFSET`]) and
/// for the `u32` record releases 2018-2023.
pub const fn category_before_list(wide: bool) -> usize {
    if wide {
        OPTION_LIST_OFFSET - CATEGORY_OFFSET
    } else {
        44
    }
}

/// The set an option-set list at `list_at` belongs to and its options, or
/// `None` when it is not one. The list opens right after the six bytes
/// `0xFF` x 4 and the `ElementParents` tag (an element-record marker
/// without its `Outline` tag); the record's category, [`category_before_list`]
/// bytes back, must be [`OST_DESIGN_OPTION_SETS`]; the list is a `u32`
/// count `k`, the set's own ElementId and its `k - 1` options, `u64` each
/// from 2024 and `u32` each on 2018-2023. Found this way, a set
/// need not sit in a partition's leading record chain: on cjk_138949ac
/// unresolved.
pub fn option_set_list(buf: &[u8], list_at: usize, wide: bool) -> Option<(u32, Vec<u32>)> {
    let category = le_bytes::u64_at(buf, list_at.checked_sub(category_before_list(wide))?)? as i64;
    if category != OST_DESIGN_OPTION_SETS {
        return None;
    }
    let count = le_bytes::u32_at(buf, list_at)? as usize;
    if !(2..=OPTIONS_PER_SET_MAX + 1).contains(&count) {
        return None;
    }
    let width = if wide { 8 } else { 4 };
    let id = |index: usize| -> Option<u32> {
        let at = list_at + 4 + index * width;
        if wide {
            u32::try_from(le_bytes::u64_at(buf, at)?).ok()
        } else {
            le_bytes::u32_at(buf, at)
        }
    };
    let set_id = id(0)?;
    let mut options = Vec::with_capacity(count - 1);
    for index in 1..count {
        let option = id(index)?;
        if option == set_id || options.contains(&option) {
            return None;
        }
        options.push(option);
    }
    Some((set_id, options))
}

/// Every `(name, primary)` the name entries in `buf` give for the set that
/// lists `options`. A primary that is not one of `options`, or a name with
/// a control character or an unpaired surrogate, is not an entry.
///
/// `wide`: ElementIds are `u64` (2024 on); else `u32` (2018-2023:
/// the same entry with `u32` ids).
pub fn find_set_entries(buf: &[u8], options: &[u32], wide: bool) -> Vec<(String, u32)> {
    let mut needle = Vec::with_capacity(8 + options.len() * 8);
    needle.extend_from_slice(&(options.len() as u32).to_le_bytes());
    for option in options {
        if wide {
            needle.extend_from_slice(&u64::from(*option).to_le_bytes());
        } else {
            needle.extend_from_slice(&option.to_le_bytes());
        }
    }
    needle.extend_from_slice(&1u32.to_le_bytes());
    let mut out = Vec::new();
    for hit in memchr::memmem::find_iter(buf, &needle) {
        let length_at = hit + needle.len();
        let Some(units) = le_bytes::u32_at(buf, length_at).map(|n| n as usize) else {
            continue;
        };
        if !(1..=SET_NAME_MAX_UNITS).contains(&units) {
            continue;
        }
        let name_at = length_at + 4;
        let Some(bytes) = buf.get(name_at..name_at + units * 2) else {
            continue;
        };
        let code_units: Vec<u16> = le_bytes::u16_units(bytes).collect();
        let Ok(name) = String::from_utf16(&code_units) else {
            continue;
        };
        if name.trim().is_empty() || name.chars().any(char::is_control) {
            continue;
        }
        let primary_at = name_at + units * 2;
        let primary = if wide {
            le_bytes::u64_at(buf, primary_at).and_then(|v| u32::try_from(v).ok())
        } else {
            le_bytes::u32_at(buf, primary_at)
        };
        let Some(primary) = primary else {
            continue;
        };
        if options.contains(&primary) {
            out.push((name, primary));
        }
    }
    out
}

/// The name of design option `option` of set `set_id` in `buf`.
///
/// An option's partition record holds `01 00 00 00` and its ElementId
/// (`u64` from 2024, `u32` before), and within the next
/// [`OPTION_NAME_WINDOW`] bytes its name: a `u32` length in UTF-16 code
/// units, the name, and the ElementId of its set. Measured on rstbasic
/// ("Option Set 1": "Option 1", "Option 2", "Option 3", the API's names).
/// Every name found for the option is returned.
pub fn find_option_name(buf: &[u8], option: u32, set_id: u32, wide: bool) -> Vec<String> {
    let id_bytes = |id: u32| -> Vec<u8> {
        if wide {
            u64::from(id).to_le_bytes().to_vec()
        } else {
            id.to_le_bytes().to_vec()
        }
    };
    let mut needle = 1u32.to_le_bytes().to_vec();
    needle.extend(id_bytes(option));
    let set = id_bytes(set_id);
    let mut out = Vec::new();
    for hit in memchr::memmem::find_iter(buf, &needle) {
        let from = hit + needle.len();
        let to = (from + OPTION_NAME_WINDOW).min(buf.len());
        for at in from..to {
            let Some(units) = le_bytes::u32_at(buf, at).map(|n| n as usize) else {
                break;
            };
            if !(1..=SET_NAME_MAX_UNITS).contains(&units) {
                continue;
            }
            let name_at = at + 4;
            let set_at = name_at + units * 2;
            if buf.get(set_at..set_at + set.len()) != Some(&set[..]) {
                continue;
            }
            let code_units: Vec<u16> = buf[name_at..set_at]
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect();
            if let Ok(name) = String::from_utf16(&code_units) {
                if !name.trim().is_empty() && !name.chars().any(char::is_control) {
                    out.push(name);
                    break;
                }
            }
        }
    }
    out
}

/// Bytes after an option's id that its name is searched in.
pub const OPTION_NAME_WINDOW: usize = 256;

/// The design option sets of a whole file and their primary options.
/// Empty where the release's record shape is not proven.
/// `RevitFile::design_options` memoises it.
pub fn compute_design_options(rf: &mut RevitFile) -> Result<DesignOptions> {
    let mut out = DesignOptions::default();
    let version = rf.layout_release()?;
    // 2018-2023 write `u32` ElementIds; 2024 on `u64`.
    let wide = !crate::partition_element_records_2023::is_u32_record_release(version);
    let chain_marker = if wide {
        crate::partition_element_records::file_bbox_marker(rf, version)
    } else {
        None
    };
    let full_marker = match chain_marker {
        Some(marker) => Some(marker),
        None if !wide => {
            crate::partition_element_records_2023::record_marker(rf).map(|(marker, _)| marker)
        }
        None => None,
    };
    let Some(full_marker) = full_marker else {
        return Ok(out);
    };
    // A set list follows `0xFF` x 4 and the `ElementParents` tag.
    let list_marker: [u8; 6] = full_marker[2..].try_into().expect("6 bytes");
    let declared = crate::elem_table::parse_records(rf)
        .map(|records| crate::elem_table::declared_ids(&records))
        .unwrap_or_default();
    let streams = rf.partition_stream_names();
    let mut members: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    let mut add = |out: &mut DesignOptions, set_id: u32, options: Vec<u32>| {
        // A set framed twice with different lists is unresolved.
        match members.get(&set_id) {
            Some(held) if *held != options => {
                out.unresolved_sets.insert(set_id);
            }
            _ => {
                members.insert(set_id, options);
            }
        }
    };
    for stream in &streams {
        let Ok(inflated) = rf.inflated_partition(stream) else {
            continue;
        };
        let buf = inflated.bytes();
        if let Some(marker) = chain_marker {
            for span in partition_record_chain(buf, &marker) {
                let Ok(set_id) = u32::try_from(span.element_id) else {
                    continue;
                };
                if let Some(options) =
                    option_set_members(buf, span.start, span.end, span.element_id)
                {
                    add(&mut out, set_id, options);
                }
            }
        }
        for hit in memchr::memmem::find_iter(buf, &list_marker) {
            if let Some((set_id, options)) = option_set_list(buf, hit + list_marker.len(), wide) {
                if declared.contains(&set_id) {
                    add(&mut out, set_id, options);
                }
            }
        }
    }
    for (set_id, options) in members {
        if out.unresolved_sets.contains(&set_id) {
            continue;
        }
        let mut entries: BTreeSet<(String, u32)> = BTreeSet::new();
        for stream in &streams {
            let Ok(inflated) = rf.inflated_partition(stream) else {
                continue;
            };
            entries.extend(find_set_entries(inflated.bytes(), &options, wide));
        }
        let primaries: BTreeSet<u32> = entries.iter().map(|(_, primary)| *primary).collect();
        match (primaries.len(), entries.into_iter().next()) {
            (1, Some((name, primary))) => {
                // An option whose copies give different names gets none.
                let mut option_names = BTreeMap::new();
                for &option in &options {
                    let mut found: BTreeSet<String> = BTreeSet::new();
                    for stream in &streams {
                        let Ok(inflated) = rf.inflated_partition(stream) else {
                            continue;
                        };
                        found.extend(find_option_name(inflated.bytes(), option, set_id, wide));
                    }
                    if let (Some(name), 1) = (found.iter().next(), found.len()) {
                        option_names.insert(option, name.clone());
                    }
                }
                out.sets.insert(
                    set_id,
                    DesignOptionSet {
                        set_id,
                        name,
                        options,
                        primary,
                        option_names,
                    },
                );
            }
            _ => {
                out.unresolved_sets.insert(set_id);
            }
        }
    }
    Ok(out)
}

/// Placed instances of the
/// [`crate::partition_element_records::RECOVERED_CATEGORIES`] that are in a
/// non-primary design option, one per ElementId, in ElementId order, each
/// with its class and the frame it was read from. The export leaves them out,
/// as Revit's does; the diagnostics count them by class so they can say so.
pub fn scan_non_primary_option_instances(
    rf: &mut RevitFile,
    revit_version: u32,
) -> Result<Vec<crate::partition_element_records::LeftOutElement>> {
    use crate::partition_element_records::{
        LeftOutElement, RECOVERED_CATEGORIES, recovered_class, scan_category_records_multi,
    };
    let mut left_out = Vec::new();
    let u32_records = crate::partition_element_records_2023::is_u32_record_release(revit_version);
    if !supports_revit_version(revit_version) && !u32_records {
        return Ok(left_out);
    }
    let options = rf.memoised_design_options();
    if options.sets.is_empty() {
        return Ok(left_out);
    }
    let declared: BTreeSet<u32> = match crate::elem_table::parse_records(rf) {
        Ok(records) => crate::elem_table::declared_ids(&records),
        Err(_) => return Ok(left_out),
    };
    let categories: Vec<i64> = RECOVERED_CATEGORIES.iter().map(|(c, _)| *c).collect();
    let records = if u32_records {
        crate::partition_element_records_2023::scan_records(rf, revit_version)
            .into_iter()
            .filter(|record| categories.contains(&record.builtin_category))
            .collect()
    } else {
        scan_category_records_multi(rf, revit_version, &categories, &declared)?
    };
    let mut excluded: BTreeMap<u32, &PartitionElementRecord> = BTreeMap::new();
    for record in records
        .iter()
        .filter(|r| r.is_exported_instance() && options.excludes(r))
    {
        excluded.insert(record.element_id, record);
    }
    for record in excluded.into_values() {
        if let Some(class) = recovered_class(record.builtin_category) {
            left_out.push(LeftOutElement::from_record(record, class));
        }
    }
    Ok(left_out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// rst_basic_sample_project 2018-2023 ("Option Set 1", primary 875635).
    #[test]
    fn reads_u32_set_list_and_name_entry() {
        let mut buf = vec![0u8; 44];
        buf[..8].copy_from_slice(&OST_DESIGN_OPTION_SETS.to_le_bytes());
        let list_at = buf.len();
        for v in [4u32, 875_634, 875_635, 875_638, 875_641] {
            buf.extend_from_slice(&v.to_le_bytes());
        }
        assert_eq!(
            option_set_list(&buf, list_at, false),
            Some((875_634, vec![875_635, 875_638, 875_641]))
        );
        assert_eq!(option_set_list(&buf, list_at, true), None);
        let mut entry = 3u32.to_le_bytes().to_vec();
        for v in [875_635u32, 875_638, 875_641, 1] {
            entry.extend_from_slice(&v.to_le_bytes());
        }
        let name: Vec<u16> = "Option Set 1".encode_utf16().collect();
        entry.extend_from_slice(&(name.len() as u32).to_le_bytes());
        entry.extend(name.iter().flat_map(|u| u.to_le_bytes()));
        entry.extend_from_slice(&875_635u32.to_le_bytes());
        assert_eq!(
            find_set_entries(&entry, &[875_635, 875_638, 875_641], false),
            vec![("Option Set 1".to_string(), 875_635)]
        );
    }
}
