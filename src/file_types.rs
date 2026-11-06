//! Every element type the file holds, placed or not.
//!
//! Revit users expect every family and type of a project to survive an
//! export, whether or not an instance places it, so it can be reused.
//! The instance walk only reaches the types its instances name; this reads
//! the types themselves: every partition element record
//! (`crate::partition_element_records`, or the `u32` frame of
//! `crate::partition_element_records_2023`) whose class, in the file's
//! own schema, derives from [`TYPE_BASE_CLASS`] (`FamilySymbol`,
//! `BasicWallType`, `FloorAttributes`, ...). The name and category come from
//! the record's name entry (`crate::partition_names`), the family from
//! `crate::partition_names::resolve_family`.
//!
//! Measured against the public Revit API dump of the 101 corpus models
//! release. A type the scan does not reach, or a release whose record shape
//! is not proven, gives nothing (fail closed).

use crate::RevitFile;
use crate::partition_element_records::{
    BBOX_MARKER_OFFSET, CLASS_TAG_OFFSET, decode_at_with_marker,
};
use std::collections::{BTreeMap, BTreeSet};

/// The schema class every element type derives from.
pub const TYPE_BASE_CLASS: &str = "Symbol";

/// The classes a family type derives from: `FamilySymbol` and the curtain
/// mullion and panel symbols (`partition_schema_mvp`'s list).
pub const FAMILY_SYMBOL_CLASSES: [&str; 3] = ["FamilySymbol", "SysMullionFamSym", "SysPanelFamSym"];

/// One element type of the file.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FileType {
    pub id: u32,
    /// The record's class in the file's own schema (`FamilySymbol`).
    pub class_name: String,
    /// A loadable family's type: its class derives from one of
    /// [`FAMILY_SYMBOL_CLASSES`], so its instances are family instances.
    pub family_symbol: bool,
    /// The record's `BuiltInCategory`.
    pub builtin_category: i64,
    /// The type's name, where its name entry is read.
    pub name: Option<String>,
    /// The type's family (loadable families), where it resolves.
    pub family_id: Option<u32>,
    pub family_name: Option<String>,
    /// The type's text parameters by the name Revit shows
    /// (`crate::partition_type_parameters::type_text_parameters`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub parameters: BTreeMap<String, String>,
}

/// Every element record of the file, by ElementId: its class tag and
/// category. Records of one id in several partition copies keep the first
/// class read; an id read with two classes is dropped.
pub fn element_record_classes(rf: &mut RevitFile) -> BTreeMap<u32, (u16, i64)> {
    let Ok(version) = rf.basic_file_info().map(|b| b.version) else {
        return BTreeMap::new();
    };
    let mut seen: BTreeMap<u32, Option<(u16, i64)>> = BTreeMap::new();
    let mut add = |id: u32, tag: u16, category: i64| {
        let entry = seen.entry(id).or_insert(Some((tag, category)));
        if entry.is_some_and(|(t, _)| t != tag) {
            *entry = None;
        }
    };
    if crate::partition_element_records_2023::is_u32_record_release(version) {
        for r in crate::partition_element_records_2023::scan_records(rf, version) {
            add(r.element_id, r.class_tag, r.builtin_category);
        }
    } else {
        let Some(marker) = crate::partition_element_records::file_bbox_marker(rf, version) else {
            return BTreeMap::new();
        };
        let Ok(declared) = crate::elem_table::declared_element_ids(rf) else {
            return BTreeMap::new();
        };
        let declared: BTreeSet<u32> = declared.into_iter().collect();
        for stream in rf.partition_stream_names() {
            let Ok(inflated) = rf.inflated_partition(&stream) else {
                continue;
            };
            let buf = inflated.bytes();
            for at in memchr::memmem::find_iter(buf, &marker) {
                let Some(start) = at.checked_sub(BBOX_MARKER_OFFSET) else {
                    continue;
                };
                if let Some(r) = decode_at_with_marker(&stream, buf, start, &declared, &marker) {
                    add(r.element_id, r.class_tag, r.builtin_category);
                }
            }
            // Types without a box (a wall type): the same prologue,
            // the class tag at the same `+0x4a`, and the box marker's last
            // six bytes at `+0x50`.
            let short = crate::partition_type_records::record_marker(&marker);
            for at in memchr::memmem::find_iter(buf, &short) {
                let Some(start) = at.checked_sub(BBOX_MARKER_OFFSET) else {
                    continue;
                };
                let Some(r) = crate::partition_type_records::decode_at_with_marker(
                    &stream, buf, start, &declared, &marker,
                ) else {
                    continue;
                };
                let Some(tag) = buf
                    .get(start + CLASS_TAG_OFFSET..start + CLASS_TAG_OFFSET + 2)
                    .map(|b| u16::from_le_bytes([b[0], b[1]]))
                else {
                    continue;
                };
                add(r.element_id, tag, r.builtin_category);
            }
        }
    }
    seen.into_iter()
        .filter_map(|(id, v)| v.map(|v| (id, v)))
        .collect()
}

/// The class chain names of `tag`, base first.
fn chain_names(layouts: &crate::formats::SchemaLayouts, tag: u16) -> Vec<String> {
    layouts
        .chain(tag)
        .map(|c| c.into_iter().map(|l| l.name.clone()).collect())
        .unwrap_or_default()
}

/// Every element type of the file (see the module docs), by ElementId.
pub fn file_types(rf: &mut RevitFile) -> Vec<FileType> {
    let Ok(layouts) = rf.schema_layouts() else {
        return Vec::new();
    };
    let records = element_record_classes(rf);
    let names = rf.memoised_element_names();
    let mut parameters = rf
        .basic_file_info()
        .map(|b| crate::partition_type_parameters::type_text_parameters(rf, b.version))
        .unwrap_or_default();
    // Revit 2016 to 2023 name entries carry no category.
    let names_u32 = match rf.basic_file_info().map(|b| b.version) {
        Ok(v) if crate::partition_element_records_2023::is_u32_record_release(v) => {
            let ids: BTreeSet<u32> = records.keys().copied().collect();
            crate::partition_names::name_entries_2023(rf, &ids)
        }
        _ => BTreeMap::new(),
    };
    // A system type (a wall or floor type) has no name entry: its name is in
    // its parameter value block.
    let unnamed: BTreeSet<u32> = records
        .iter()
        .filter(|(id, (tag, _))| {
            !names.entries.contains_key(id)
                && !names_u32.contains_key(id)
                && chain_names(&layouts, *tag)
                    .iter()
                    .any(|c| c == TYPE_BASE_CLASS)
        })
        .map(|(id, _)| *id)
        .collect();
    let block_names: BTreeMap<u32, String> =
        crate::partition_type_names::read_type_names(rf, &unnamed)
            .map(|(read, _)| read.into_iter().map(|(id, r)| (id, r.read.name)).collect())
            .unwrap_or_default();
    let mut out = Vec::new();
    for (id, (tag, category)) in records {
        let chain = chain_names(&layouts, tag);
        if !chain.iter().any(|c| c == TYPE_BASE_CLASS) {
            continue;
        }
        // Not types the API lists (101 corpus models): CAD import
        // symbols, and family symbol records without a name entry (nested
        // families' own symbols, one per curtain mullion or panel instance:
        // 37,000 such records, 0 of them API types, against 18,516 named
        // ones that are).
        if chain.iter().any(|c| c.ends_with("ImportSymbol")) {
            continue;
        }
        let family_symbol = chain
            .iter()
            .any(|c| FAMILY_SYMBOL_CLASSES.contains(&c.as_str()));
        if family_symbol && !names.entries.contains_key(&id) && !names_u32.contains_key(&id) {
            continue;
        }
        let family_id = crate::partition_names::resolve_family(&names, id);
        out.push(FileType {
            id,
            class_name: chain.last().cloned().unwrap_or_default(),
            family_symbol,
            builtin_category: category,
            name: names
                .entries
                .get(&id)
                .map(|e| e.name.clone())
                .or_else(|| names_u32.get(&id).cloned())
                .or_else(|| block_names.get(&id).cloned()),
            family_id,
            family_name: family_id
                .and_then(|f| names.entries.get(&f))
                .map(|e| e.name.clone()),
            parameters: parameters
                .remove(&id)
                .unwrap_or_default()
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        });
    }
    out
}

/// category and chain (base first).
pub fn record_chains(rf: &mut RevitFile) -> Vec<(u32, i64, Vec<String>)> {
    let Ok(layouts) = rf.schema_layouts() else {
        return Vec::new();
    };
    element_record_classes(rf)
        .into_iter()
        .map(|(id, (tag, category))| (id, category, chain_names(&layouts, tag)))
        .collect()
}
