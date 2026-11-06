//! Per-element IFC export-type overrides carried in the Revit 2024
//! partition parameter blocks.
//!
//! **Superseded for the export** by [`find_export_parameters`]:
//! the value is one entry `i64 BuiltInParameter · u32 n · UTF-16` of the
//! element's serialised data, keyed by [`IFC_EXPORT_ELEMENT_AS`], and a type
//! carries [`IFC_EXPORT_ELEMENT_TYPE_AS`] for its elements, each with a
//! predefined type beside it. That read works on Revit 2024 and 2025 and
//! gives the same 21 owners on `2024_Core_Interior.rvt` as the owner-offset
//! read below. Nothing calls that read now; it records the framing.
//!
//! Revit's IFC exporter maps a Revit category to an IFC entity type,
//! but a single element may carry an instance-level override — the
//! `IFC Export As` parameter — that redirects it to a different type.
//! On `2024_Core_Interior.rvt` twenty `OST_Floors` elements are
//! exported as `IfcShadingDevice` instead of `IfcSlab` while sharing
//! their `FloorType` with elements that stay `IfcSlab`: the reference
//! export carries `IFCSLABTYPE` **and** `IFCSHADINGDEVICETYPE` rows
//! with the same `Tag` (`4166`, `71848`), which is only possible if
//! the choice is made per instance.
//!
//! The override is a UTF-16LE string in the element's parameter
//! block, framed as
//!
//! ```text
//! -0x11e  u64  owning ElementId (confirmation slot)
//! -0x0dc  u64  owning ElementId
//! -0x004  u32  value length in UTF-16 code units
//! +0x000  2*n  the value, UTF-16LE ("IfcShadingDevice")
//! +0x0..  u64  parameter-definition ElementId (17368 / 17493 here)
//! ```
//!
//! Both owner slots must carry the same `ElementId` and it must be
//! declared in `Global/ElemTable`; anything else is discarded. That
//! test is what separates a real override from the parameter
//! *definition* block, which repeats the same value string with no
//! owner (one such block exists on this file) and from the second
//! string of each pair, whose owner slots hold sentinels.
//!
//! # Honesty
//!
//! - The two owner offsets are **measured**, on one file, over thirty
//!   accepted entries. They are not derived from a parsed parameter
//!   block header; the block framing itself is not decoded.
//! - The value string is returned verbatim. Nothing here decides what
//!   an override *means* — [`crate::ifc::category_map`] decides which
//!   values it is willing to act on, and an unrecognised value leaves
//!   the element on its category's default mapping.
//! - `IfcShadingDevice` was the first value proven, on floors. The scan
//!   is general because the framing is; which values the writer acts on,
//!   and where Revit's exporter ignores one (curtain walls in 2024 files),
//!   is decided in [`crate::ifc::category_map`].
//!
//! Measured on `2024_Core_Interior.rvt`: 31 accepted entries naming
//! 21 distinct ElementIds, every one of them `IfcShadingDevice`. The
//! 20 that are also standalone placed instances
//! ([`crate::partition_element_records::PartitionElementRecord::is_exported_instance`])
//! are exactly the 20 `IFCSHADINGDEVICE` `Tag` values in Revit's own
//! export — no misses, no extras. The 21st (`16925`) is a container
//! member, which the instance rule already excludes.

use crate::le_bytes;
use crate::partition_element_records::find_subslice;
use crate::{Result, RevitFile};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Releases where this framing is corpus-proven.
pub const IFC_EXPORT_OVERRIDE_SUPPORTED_REVIT_VERSIONS: &[u32] = &[2024];

/// Bytes from the value string back to the owning `ElementId`.
pub const OWNER_OFFSET_BEFORE_VALUE: usize = 220;

/// Bytes from the value string back to the confirmation copy of the
/// owning `ElementId`. Both slots must agree.
pub const OWNER_CONFIRM_OFFSET_BEFORE_VALUE: usize = 286;

/// Bytes from the value string back to its `u32` length prefix.
pub const LENGTH_PREFIX_OFFSET_BEFORE_VALUE: usize = 4;

/// Shortest override value the scan will accept, in UTF-16 units.
pub const MIN_VALUE_CHARS: usize = 4;

/// Longest override value the scan will accept, in UTF-16 units.
pub const MAX_VALUE_CHARS: usize = 64;

/// Prefix every IFC entity name carries; the scan anchor.
pub const VALUE_PREFIX: &str = "Ifc";

/// One accepted override entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IfcExportOverride {
    /// Stream the entry was found in, e.g. `"Partitions/46"`.
    pub stream: String,
    /// Byte offset of the value string in the inflated stream.
    pub offset: usize,
    /// ElementId the override applies to.
    pub element_id: u32,
    /// The value string, verbatim (e.g. `"IfcShadingDevice"`).
    pub value: String,
}

/// Whether this release's override framing is proven.
pub fn supports_revit_version(revit_version: u32) -> bool {
    IFC_EXPORT_OVERRIDE_SUPPORTED_REVIT_VERSIONS.contains(&revit_version)
}

/// Decode one override entry whose value string starts at `offset`.
///
/// Fail-closed at every step: a bad length, a non-alphanumeric value,
/// disagreeing owner slots, or an owner that `Global/ElemTable` does
/// not declare all reject the entry.
pub fn decode_at(
    stream: &str,
    buf: &[u8],
    offset: usize,
    declared: &BTreeSet<u32>,
) -> Option<IfcExportOverride> {
    if offset < OWNER_CONFIRM_OFFSET_BEFORE_VALUE {
        return None;
    }
    let chars = le_bytes::u32_at(buf, offset - LENGTH_PREFIX_OFFSET_BEFORE_VALUE)? as usize;
    if !(MIN_VALUE_CHARS..=MAX_VALUE_CHARS).contains(&chars) {
        return None;
    }
    let end = offset.checked_add(chars.checked_mul(2)?)?;
    if end > buf.len() {
        return None;
    }
    let mut value = String::with_capacity(chars);
    for index in 0..chars {
        let at = offset + index * 2;
        let unit = le_bytes::u16_at(buf, at)?;
        // An IFC entity name is ASCII alphanumeric; anything else here
        // means the anchor landed inside unrelated text.
        let byte = u8::try_from(unit).ok()?;
        if !byte.is_ascii_alphanumeric() {
            return None;
        }
        value.push(char::from(byte));
    }
    if !value.starts_with(VALUE_PREFIX) {
        return None;
    }
    let owner = le_bytes::u64_at(buf, offset - OWNER_OFFSET_BEFORE_VALUE)?;
    let confirm = le_bytes::u64_at(buf, offset - OWNER_CONFIRM_OFFSET_BEFORE_VALUE)?;
    if owner != confirm || owner == 0 || owner > u64::from(u32::MAX) {
        return None;
    }
    let element_id = owner as u32;
    if !declared.contains(&element_id) {
        return None;
    }
    Some(IfcExportOverride {
        stream: stream.to_string(),
        offset,
        element_id,
        value,
    })
}

/// Find every accepted override entry in one inflated stream.
pub fn find_overrides(
    stream: &str,
    buf: &[u8],
    declared: &BTreeSet<u32>,
) -> Vec<IfcExportOverride> {
    let needle: Vec<u8> = VALUE_PREFIX
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let mut out = Vec::new();
    let mut cursor = 0usize;
    while cursor + needle.len() <= buf.len() {
        let Some(found) = find_subslice(&buf[cursor..], &needle) else {
            break;
        };
        let hit = cursor + found;
        if let Some(entry) = decode_at(stream, buf, hit, declared) {
            out.push(entry);
        }
        cursor = hit + 1;
    }
    out
}

/// Collapse accepted entries to one value per ElementId.
///
/// An ElementId that names two different values is dropped: an
/// override that cannot be read unambiguously is not an override.
pub fn overrides_by_element_id(
    entries: impl IntoIterator<Item = IfcExportOverride>,
) -> BTreeMap<u32, String> {
    let mut out: BTreeMap<u32, String> = BTreeMap::new();
    let mut conflicting: BTreeSet<u32> = BTreeSet::new();
    for entry in entries {
        match out.get(&entry.element_id) {
            Some(existing) if *existing != entry.value => {
                conflicting.insert(entry.element_id);
            }
            Some(_) => {}
            None => {
                out.insert(entry.element_id, entry.value);
            }
        }
    }
    for id in conflicting {
        out.remove(&id);
    }
    out
}

/// Scan every `Partitions/*` stream for IFC export-type overrides.
///
/// Returns an empty map for unsupported releases (fail closed).
pub fn scan_ifc_export_overrides(
    rf: &mut RevitFile,
    revit_version: u32,
    declared: &BTreeSet<u32>,
) -> Result<BTreeMap<u32, String>> {
    if !supports_revit_version(revit_version) || declared.is_empty() {
        return Ok(BTreeMap::new());
    }
    let streams = rf.partition_stream_names();
    let mut entries = Vec::new();
    for stream in streams {
        let Ok(inflated) = rf.inflated_partition(&stream) else {
            continue;
        };
        entries.extend(find_overrides(&stream, inflated.bytes(), declared));
    }
    Ok(overrides_by_element_id(entries))
}

/// Autodesk `BuiltInParameter.IFC_EXPORT_ELEMENT_AS`, an instance's
/// "Export to IFC As".
pub const IFC_EXPORT_ELEMENT_AS: i64 = -1_019_014;
/// Autodesk `BuiltInParameter.IFC_EXPORT_ELEMENT_TYPE_AS`, a type's
/// "Export Type to IFC As".
pub const IFC_EXPORT_ELEMENT_TYPE_AS: i64 = -1_019_015;
/// Autodesk `BuiltInParameter.IFC_EXPORT_PREDEFINEDTYPE`, an instance's
/// IFC predefined type.
pub const IFC_EXPORT_PREDEFINEDTYPE: i64 = -1_019_016;
/// Autodesk `BuiltInParameter.IFC_EXPORT_PREDEFINEDTYPE_TYPE`, a type's
/// IFC predefined type.
pub const IFC_EXPORT_PREDEFINEDTYPE_TYPE: i64 = -1_019_017;

/// Furthest an export parameter entry has to sit past the element-data
/// header of the element it belongs to. The furthest measured is `0x1ac`,
/// on `2024_Core_Interior.rvt`.
pub const EXPORT_PARAMETER_WINDOW: usize = 0x800;

/// The IFC export parameters one element's serialised data sets.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportParameters {
    /// [`IFC_EXPORT_ELEMENT_AS`], verbatim (e.g. `"ifcSlab"`).
    pub export_as: Option<String>,
    /// [`IFC_EXPORT_PREDEFINEDTYPE`], verbatim (e.g. `"ROOF"`).
    pub predefined_type: Option<String>,
    /// [`IFC_EXPORT_ELEMENT_TYPE_AS`], verbatim (e.g. `"IfcCoveringType"`).
    pub type_export_as: Option<String>,
    /// [`IFC_EXPORT_PREDEFINEDTYPE_TYPE`], verbatim (e.g. `"CLADDING"`).
    pub type_predefined_type: Option<String>,
}

impl ExportParameters {
    fn slot(&mut self, key: i64) -> Option<&mut Option<String>> {
        match key {
            IFC_EXPORT_ELEMENT_AS => Some(&mut self.export_as),
            IFC_EXPORT_PREDEFINEDTYPE => Some(&mut self.predefined_type),
            IFC_EXPORT_ELEMENT_TYPE_AS => Some(&mut self.type_export_as),
            IFC_EXPORT_PREDEFINEDTYPE_TYPE => Some(&mut self.type_predefined_type),
            _ => None,
        }
    }

    /// The entity an element of this type or instance exports as: the
    /// instance's own value, else its type's with the trailing `Type` of
    /// an IFC type entity dropped (`IfcCoveringType` is `IfcCovering`).
    pub fn effective_export_as(
        &self,
        type_parameters: Option<&ExportParameters>,
    ) -> Option<String> {
        let value = match &self.export_as {
            Some(value) => value.as_str(),
            None => type_parameters?.type_export_as.as_deref()?,
        };
        // an instance's own value can name the type entity too
        // (168 Nordic family instances say `IfcSpaceType`, 4 rooms likewise).
        let entity = match value.len().checked_sub(4) {
            Some(cut) if value[cut..].eq_ignore_ascii_case("type") => &value[..cut],
            _ => value,
        };
        Some(entity.to_string())
    }

    /// The predefined type: the instance's own, else its type's.
    pub fn effective_predefined_type(
        &self,
        type_parameters: Option<&ExportParameters>,
    ) -> Option<String> {
        self.predefined_type
            .clone()
            .or_else(|| type_parameters?.type_predefined_type.clone())
    }
}

/// One export parameter value: an IFC entity name for the two "export as"
/// keys, an enumerator-shaped identifier for the two predefined types.
fn export_parameter_value(buf: &[u8], at: usize, key: i64) -> Option<String> {
    let units = le_bytes::u32_at(buf, at)? as usize;
    if !(1..=MAX_VALUE_CHARS).contains(&units) {
        return None;
    }
    let bytes = buf.get(at + 4..at + 4 + units * 2)?;
    let mut value = String::with_capacity(units);
    for unit in le_bytes::u16_units(bytes) {
        let byte = u8::try_from(unit).ok()?;
        if !(byte.is_ascii_alphanumeric() || byte == b'_') {
            return None;
        }
        value.push(char::from(byte));
    }
    let entity = matches!(key, IFC_EXPORT_ELEMENT_AS | IFC_EXPORT_ELEMENT_TYPE_AS);
    (!entity || is_entity_name(&value)).then_some(value)
}

/// `value` when it is shaped as `key` allows: an IFC entity name (`Ifc...`,
/// no `_`) for the two "export as" keys, an identifier for the predefined
/// types.
fn valid_export_value(key: i64, value: String) -> Option<String> {
    let units = value.chars().count();
    if !(1..=MAX_VALUE_CHARS).contains(&units)
        || !value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return None;
    }
    let entity = matches!(key, IFC_EXPORT_ELEMENT_AS | IFC_EXPORT_ELEMENT_TYPE_AS);
    (!entity || is_entity_name(&value)).then_some(value)
}

/// An IFC predefined type as a parameter holds it: ASCII alphanumeric and `_`.
fn is_enumerator(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// An IFC entity name as an export-as value holds it: `Ifc` and more, ASCII
/// alphanumeric.
fn is_entity_name(value: &str) -> bool {
    let prefixed = value
        .get(..VALUE_PREFIX.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(VALUE_PREFIX));
    prefixed && value.len() > VALUE_PREFIX.len() && value.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// Every IFC export parameter in one inflated partition, by the ElementId
/// whose serialised data holds it.
///
/// An entry is `i64 BuiltInParameter · u32 n · n UTF-16 units`, one of
/// [`IFC_EXPORT_ELEMENT_AS`], [`IFC_EXPORT_ELEMENT_TYPE_AS`],
/// [`IFC_EXPORT_PREDEFINEDTYPE`] and [`IFC_EXPORT_PREDEFINEDTYPE_TYPE`]. It
/// belongs to the element whose data header (`header`, see
/// [`crate::partition_names::element_data_header`]) is the last one before
/// it, no more than [`EXPORT_PARAMETER_WINDOW`] back, and that element must
/// be declared in `Global/ElemTable`. A key an element sets to two
/// different values is dropped. Revit 2024 and later read the owner from
/// the data object instead ([`find_export_parameters_in_objects`]).
pub fn find_export_parameters(
    buf: &[u8],
    header: &[u8; 10],
    declared: &BTreeSet<u32>,
) -> BTreeMap<u32, ExportParameters> {
    collect_export_parameters(buf, declared, |hit| {
        let from = hit.saturating_sub(EXPORT_PARAMETER_WINDOW);
        let start = memchr::memmem::rfind(&buf[from..hit], header)?;
        le_bytes::u64_at(buf, from + start + header.len()).and_then(|id| u32::try_from(id).ok())
    })
}

/// [`find_export_parameters`] on Revit 2024 and later: an entry
/// belongs to the data object whose payload holds it, whose
/// Adler-32 verifies, and that object's ElementId must be declared. An entry
/// in no data object is not read. `objects` is
/// [`crate::partition_room_parameters::data_objects`] of `buf`.
///
/// On the BIM4LCA office's building permit file the header rule gave
/// `IfcSpace` entries to doors (37), curtain walls (13), sketch lines (11)
/// and plumbing fixtures (10). Five of those fixtures are drains (four
/// floor traps and a roof drain), and their own `IfcSpace` hid their
/// types' `IfcWasteTerminalType`, while Revit exports all 78 of the file's
/// drains as `IfcWasteTerminal`.
pub fn find_export_parameters_in_objects(
    buf: &[u8],
    objects: &[(usize, crate::partition_room_parameters::DataObject)],
    declared: &BTreeSet<u32>,
) -> BTreeMap<u32, ExportParameters> {
    use crate::partition_room_parameters::DATA_OBJECT_HEADER;
    collect_export_parameters(buf, declared, |hit| {
        let index = objects.partition_point(|(p, _)| *p <= hit).checked_sub(1)?;
        let (p, object) = objects[index];
        (hit >= p + DATA_OBJECT_HEADER && hit + 8 <= object.end).then_some(object.element_id)
    })
}

/// Every export parameter entry in `buf` whose owner `owner_of` finds and
/// `declared` holds, by owner. A key an owner sets to two different values
/// is dropped.
fn collect_export_parameters(
    buf: &[u8],
    declared: &BTreeSet<u32>,
    owner_of: impl Fn(usize) -> Option<u32>,
) -> BTreeMap<u32, ExportParameters> {
    let mut found: BTreeMap<(u32, i64), Option<String>> = BTreeMap::new();
    for key in [
        IFC_EXPORT_ELEMENT_AS,
        IFC_EXPORT_ELEMENT_TYPE_AS,
        IFC_EXPORT_PREDEFINEDTYPE,
        IFC_EXPORT_PREDEFINEDTYPE_TYPE,
    ] {
        for hit in memchr::memmem::find_iter(buf, &key.to_le_bytes()) {
            let Some(value) = export_parameter_value(buf, hit + 8, key) else {
                continue;
            };
            let Some(owner) = owner_of(hit).filter(|owner| declared.contains(owner)) else {
                continue;
            };
            match found.get_mut(&(owner, key)) {
                None => {
                    found.insert((owner, key), Some(value));
                }
                Some(held) => {
                    if held.as_deref() != Some(value.as_str()) {
                        *held = None;
                    }
                }
            }
        }
    }
    let mut out: BTreeMap<u32, ExportParameters> = BTreeMap::new();
    for ((owner, key), value) in found {
        let Some(value) = value else {
            continue;
        };
        if let Some(slot) = out.entry(owner).or_default().slot(key) {
            *slot = Some(value);
        }
    }
    out
}

/// Every element's IFC export parameters, read as the element's stored
/// parameters through the file's own schema ([`crate::schema_objects`],
/// element, so no owner is guessed. On the 12 Nordic LCA models
/// against the Revit API: "Export to IFC As" 10,452 of 10,500 equal,
/// predefined type 10,202 of 10,250, the type keys 418 of 486 and 416 of 484,
/// none different; the nearest-header owner gave 9,278, 9,078, 367 and
/// 364, missed every room's and area's value (792 and 116) and put
/// `IfcSpace` on 87 elements that do not set it (mullions, plumbing
/// fixtures, sketch lines, openings).
///
/// An element read twice keeps the copy of the highest-numbered partition,
/// the newest: a Revit 2024 project rewrites an edited element into a later
/// partition and leaves the earlier copy in place. Where the file's schema
/// cannot be read, the header search ([`find_export_parameters`], 2024
/// to 2026) is used instead.
pub fn scan_export_parameters(
    rf: &mut RevitFile,
    revit_version: u32,
    declared: &BTreeSet<u32>,
) -> Result<BTreeMap<u32, ExportParameters>> {
    const KEYS: [i64; 4] = [
        IFC_EXPORT_ELEMENT_AS,
        IFC_EXPORT_PREDEFINEDTYPE,
        IFC_EXPORT_ELEMENT_TYPE_AS,
        IFC_EXPORT_PREDEFINEDTYPE_TYPE,
    ];
    let mut newest: BTreeMap<(u32, i64), (u64, String)> = BTreeMap::new();
    // Only objects holding one of the keys' bytes are read.
    let width = crate::schema_objects::IdWidth::of_release(revit_version);
    let patterns: Vec<Vec<u8>> = KEYS
        .iter()
        .map(|k| match width {
            crate::schema_objects::IdWidth::Wide => k.to_le_bytes().to_vec(),
            crate::schema_objects::IdWidth::Narrow => (*k as i32).to_le_bytes().to_vec(),
        })
        .collect();
    let keep = |frame: &crate::schema_objects::Frame, buf: &[u8]| {
        let payload = &buf[frame.payload.clone()];
        patterns
            .iter()
            .any(|p| memchr::memmem::find(payload, p).is_some())
    };
    let read = crate::schema_objects::for_each_object_where(rf, keep, |object| {
        let Some(owner) = object.element().and_then(|id| u32::try_from(id).ok()) else {
            return;
        };
        if !declared.contains(&owner) {
            return;
        }
        let number = crate::partition_names::partition_number(&object.stream);
        for (key, value) in object.stored_parameters() {
            let crate::schema_objects::ParamValue::String(value) = value else {
                continue;
            };
            if !KEYS.contains(&key) {
                continue;
            }
            let Some(value) = valid_export_value(key, value) else {
                continue;
            };
            if newest
                .get(&(owner, key))
                .is_none_or(|(held, _)| number > *held)
            {
                newest.insert((owner, key), (number, value));
            }
        }
    });
    if read.is_err() {
        return scan_export_parameters_by_header(rf, revit_version, declared);
    }
    let mut merged: BTreeMap<u32, ExportParameters> = BTreeMap::new();
    for ((owner, key), (_, value)) in newest {
        if let Some(slot) = merged.entry(owner).or_default().slot(key) {
            *slot = Some(value);
        }
    }
    Ok(merged)
}

/// The element-data read: [`find_export_parameters`] over every `Partitions/*`
/// stream, newest partition first. Empty for a release with no measured
/// element-data header.
fn scan_export_parameters_by_header(
    rf: &mut RevitFile,
    revit_version: u32,
    declared: &BTreeSet<u32>,
) -> Result<BTreeMap<u32, ExportParameters>> {
    let Some(header) = crate::partition_names::element_data_header(revit_version) else {
        return Ok(BTreeMap::new());
    };
    let mut newest: BTreeMap<(u32, i64), (u64, String)> = BTreeMap::new();
    for stream in rf.partition_stream_names() {
        let Ok(inflated) = rf.inflated_partition(&stream) else {
            continue;
        };
        let number = crate::partition_names::partition_number(&stream);
        let found = if revit_version >= 2024 {
            let Ok(objects) = rf.memoised_partition_data_objects(&stream) else {
                continue;
            };
            find_export_parameters_in_objects(inflated.bytes(), &objects, declared)
        } else {
            find_export_parameters(inflated.bytes(), &header, declared)
        };
        for (owner, parameters) in found {
            for (key, value) in [
                (IFC_EXPORT_ELEMENT_AS, parameters.export_as),
                (IFC_EXPORT_PREDEFINEDTYPE, parameters.predefined_type),
                (IFC_EXPORT_ELEMENT_TYPE_AS, parameters.type_export_as),
                (
                    IFC_EXPORT_PREDEFINEDTYPE_TYPE,
                    parameters.type_predefined_type,
                ),
            ] {
                let Some(value) = value else {
                    continue;
                };
                if newest
                    .get(&(owner, key))
                    .is_none_or(|(held, _)| number > *held)
                {
                    newest.insert((owner, key), (number, value));
                }
            }
        }
    }
    let mut merged: BTreeMap<u32, ExportParameters> = BTreeMap::new();
    for ((owner, key), (_, value)) in newest {
        if let Some(slot) = merged.entry(owner).or_default().slot(key) {
            *slot = Some(value);
        }
    }
    Ok(merged)
}

/// Names of the shared parameters Revit's IFC exporter reads as "Export to
/// IFC As" and the predefined type before the built-in parameters existed:
/// the IFC shared-parameter file's `IfcExportAs` and
/// `IfcExportType`. Revit 2023 projects carry both these and the built-ins.
pub const SHARED_EXPORT_AS: &str = "IfcExportAs";
/// See [`SHARED_EXPORT_AS`].
pub const SHARED_EXPORT_TYPE: &str = "IfcExportType";

fn strip_type_suffix(value: &str) -> &str {
    match value.len().checked_sub(4) {
        Some(cut) if cut > VALUE_PREFIX.len() && value[cut..].eq_ignore_ascii_case("type") => {
            &value[..cut]
        }
        _ => value,
    }
}

/// The IFC export parameters of every element, read through the file's own
/// schema ([`crate::schema_objects`]) rather than a measured byte
/// layout. Used for the releases with no measured element-data
/// header, Revit 2015 to 2023.
///
/// A value is a stored string parameter of the element (its
/// `m_pParamValueSet*` entries), or, on a family type, one of its
/// `FamilyParams` entries: one of the four built-ins of
/// [`find_export_parameters`], or a project/shared parameter whose parameter
/// element's data holds the text [`SHARED_EXPORT_AS`] or
/// [`SHARED_EXPORT_TYPE`]. Revit 2018 to 2022 families carry only the shared
/// parameters, as type parameters (`FamilyParams`). The holder is not classified as a type or an
/// instance: a value is set as both the instance's and the type's slot, since
/// [`ExportParameters::effective_export_as`] reads only an element's own
/// instance slot and its type's type slot. An entity name ending in `Type`
/// (`IfcValveType`, as the shared parameter is often filled) names the entity
/// without it, and `IfcValveType.NOTDEFINED` also names the predefined type
/// (below an explicit one). A built-in wins over a shared parameter of the same meaning.
pub fn scan_export_parameters_schema(
    rf: &mut RevitFile,
    declared: &BTreeSet<u32>,
) -> Result<BTreeMap<u32, ExportParameters>> {
    use crate::schema_objects::{ParamValue, Value};
    // Every element's IFC-ish string parameters, and which ids name the
    // shared parameters, in one pass that holds one object at a time.
    let mut strings: Vec<(u32, i64, String)> = Vec::new();
    let mut shared: BTreeMap<i64, i64> = BTreeMap::new();
    // A family instance's type: `InstanceInfo.m_symbolId`, else the root's
    // `m_masterSymbolId`.
    let mut symbol_of: BTreeMap<u32, u32> = BTreeMap::new();
    crate::schema_objects::for_each_object(rf, |object| {
        let Some(id) = object.element() else { return };
        // A parameter element names itself somewhere in its data object.
        for fields in &object.objects {
            for (_, value) in &fields.values {
                let Value::Text(text) = value else { continue };
                if text.eq_ignore_ascii_case(SHARED_EXPORT_AS) {
                    shared.insert(id, IFC_EXPORT_ELEMENT_AS);
                } else if text.eq_ignore_ascii_case(SHARED_EXPORT_TYPE) {
                    shared.insert(id, IFC_EXPORT_PREDEFINEDTYPE);
                }
            }
        }
        let Ok(owner) = u32::try_from(id) else { return };
        if !declared.contains(&owner) {
            return;
        }
        let symbol = ["m_symbolId", "m_masterSymbolId"].iter().find_map(|name| {
            object
                .objects
                .iter()
                .find_map(|fields| match fields.get(name) {
                    Some(Value::Id(symbol)) if *symbol > 0 => u32::try_from(*symbol).ok(),
                    _ => None,
                })
        });
        if let Some(symbol) = symbol {
            symbol_of.insert(owner, symbol);
        }
        for (param, value) in object.stored_parameters() {
            if let ParamValue::String(text) = value {
                if param > 0
                    || matches!(
                        param,
                        IFC_EXPORT_ELEMENT_AS
                            | IFC_EXPORT_ELEMENT_TYPE_AS
                            | IFC_EXPORT_PREDEFINEDTYPE
                            | IFC_EXPORT_PREDEFINEDTYPE_TYPE
                    )
                {
                    strings.push((owner, param, text));
                }
            }
        }
        // A family type's own parameters (a loaded family's type parameters,
        // shared ones among them) are `FamilyParams` entries: `m_str` holds a
        // text value and `m_paramId` the project's parameter element.
        for fields in &object.objects {
            let Some(Value::List(entries)) = fields.get("m_params") else {
                continue;
            };
            for entry in entries {
                let Value::Object(entry) = entry else {
                    continue;
                };
                let (Some(Value::Text(text)), Some(Value::Id(param))) =
                    (entry.get("m_str"), entry.get("m_paramId"))
                else {
                    continue;
                };
                if *param > 0 && !text.is_empty() {
                    strings.push((owner, *param, text.clone()));
                }
            }
        }
    })?;
    // (owner, meaning) -> (rank, value); `None` once two of one rank disagree.
    let mut found: BTreeMap<(u32, i64), (u8, Option<String>)> = BTreeMap::new();
    for (owner, param, text) in strings {
        // Rank: a built-in beats a shared parameter, which beats a predefined
        // type written after the entity (`IfcValveType.NOTDEFINED`).
        let (meaning, rank) = match param {
            IFC_EXPORT_ELEMENT_AS | IFC_EXPORT_ELEMENT_TYPE_AS => (IFC_EXPORT_ELEMENT_AS, 2u8),
            IFC_EXPORT_PREDEFINEDTYPE | IFC_EXPORT_PREDEFINEDTYPE_TYPE => {
                (IFC_EXPORT_PREDEFINEDTYPE, 2)
            }
            _ => match shared.get(&param) {
                Some(meaning) => (*meaning, 1),
                None => continue,
            },
        };
        let text = text.trim();
        let mut values = Vec::new();
        if meaning == IFC_EXPORT_ELEMENT_AS {
            let (entity, predefined) = match text.split_once('.') {
                Some((entity, predefined)) => (entity, Some(predefined)),
                None => (text, None),
            };
            if !is_entity_name(entity) {
                continue;
            }
            values.push((meaning, rank, strip_type_suffix(entity).to_string()));
            if let Some(predefined) = predefined.filter(|p| is_enumerator(p)) {
                values.push((IFC_EXPORT_PREDEFINEDTYPE, 0, predefined.to_string()));
            }
        } else if is_enumerator(text) {
            values.push((meaning, rank, text.to_string()));
        }
        for (meaning, rank, value) in values {
            match found.get_mut(&(owner, meaning)) {
                None => {
                    found.insert((owner, meaning), (rank, Some(value)));
                }
                Some((held_rank, held)) => {
                    if rank > *held_rank {
                        *held_rank = rank;
                        *held = Some(value);
                    } else if rank == *held_rank && held.as_deref() != Some(value.as_str()) {
                        *held = None;
                    }
                }
            }
        }
    }
    let mut out: BTreeMap<u32, ExportParameters> = BTreeMap::new();
    for ((owner, meaning), (_, value)) in found {
        let Some(value) = value else { continue };
        let entry = out.entry(owner).or_default();
        if meaning == IFC_EXPORT_ELEMENT_AS {
            entry.export_as = Some(value.clone());
            entry.type_export_as = Some(value);
        } else {
            entry.predefined_type = Some(value.clone());
            entry.type_predefined_type = Some(value);
        }
    }
    // An instance takes its type's values where it sets none itself, so the
    // value reaches instances the export cannot link to their type (recovered
    // MEP instances on 2018 to 2023 carry no type field there).
    for (instance, symbol) in symbol_of {
        let Some(of_type) = out.get(&symbol).cloned() else {
            continue;
        };
        let entry = out.entry(instance).or_default();
        if entry.export_as.is_none() {
            entry.export_as = of_type.type_export_as;
        }
        if entry.predefined_type.is_none() {
            entry.predefined_type = of_type.type_predefined_type;
        }
    }
    Ok(out)
}
