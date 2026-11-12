//! Type picks, type and family names, system families and type functions of record-backed elements.

use super::*;

/// Attach each element's IFC export overrides: the element's own
/// "Export to IFC As" and predefined type, else those its type sets
/// ([`TYPE_ID_FIELD`]). A type's `IfcCoveringType`
/// names `IfcCovering`.
pub(super) fn attach_ifc_export_overrides(
    rf: &mut RevitFile,
    revit_version: u32,
    mut lists: [&mut Vec<DecodedElement>; 7],
) {
    use crate::partition_ifc_export_overrides as ieo;
    // a furniture or casework element's default predefined type depends
    // on the release, so each carries it, whether or not the file holds an export
    // parameter.
    for elements in lists.iter_mut() {
        for element in elements.iter_mut() {
            if matches!(element.class.as_str(), "Furniture" | "Casework") {
                element.fields.push((
                    FURNITURE_RELEASE_FIELD.into(),
                    InstanceField::Integer {
                        value: i64::from(revit_version),
                        signed: false,
                        size: 4,
                    },
                ));
            }
        }
    }
    let declared: BTreeSet<u32> = match crate::elem_table::parse_records(rf) {
        Ok(records) => crate::elem_table::declared_ids(&records),
        Err(_) => return,
    };
    // a release with no measured element-data header (2015 to
    // 2023) is read through the file's own schema.
    let parameters = if crate::partition_names::element_data_header(revit_version).is_some() {
        ieo::scan_export_parameters(rf, revit_version, &declared)
    } else {
        ieo::scan_export_parameters_schema(rf, &declared)
    }
    .unwrap_or_default();
    if parameters.is_empty() {
        return;
    }
    let none = ieo::ExportParameters::default();
    for elements in lists {
        for element in elements.iter_mut() {
            let own = element
                .id
                .and_then(|id| parameters.get(&id))
                .unwrap_or(&none);
            let of_type = element
                .fields
                .iter()
                .find_map(|(name, value)| match value {
                    InstanceField::ElementId { id, .. } if name == TYPE_ID_FIELD => Some(*id),
                    _ => None,
                })
                .and_then(|id| parameters.get(&id));
            // An entity the exporter does not apply leaves the element on its
            // category's entity, but its predefined type still applies when it
            // is an enumerator of that entity: Revit 2027.1 writes the ceiling
            // naming `IfcSlab` as `IfcCovering` `USERDEFINED`, its type's value.
            let entity = own
                .effective_export_as(of_type)
                .filter(|entity| export_parameters_apply(&element.class, entity));
            if let Some(entity) = entity {
                element
                    .fields
                    .push((IFC_EXPORT_AS_FIELD.into(), InstanceField::String(entity)));
                element.fields.push((
                    IFC_EXPORT_RELEASE_FIELD.into(),
                    InstanceField::Integer {
                        value: i64::from(revit_version),
                        signed: false,
                        size: 4,
                    },
                ));
            }
            if let Some(predefined) = own.effective_predefined_type(of_type) {
                element.fields.push((
                    IFC_PREDEFINED_TYPE_FIELD.into(),
                    InstanceField::String(predefined),
                ));
            }
        }
    }
}

/// Each element's reference list as its newest record frame holds it, by
/// category. A Revit 2024 project rewrites an edited element into a
/// higher-numbered partition and leaves the earlier frame in place, so the
/// frame chosen for geometry can name a type the element has since left.
#[derive(Default)]
struct CurrentReferences(BTreeMap<i64, NewestFrames>);

/// Each element's newest frame's `(partition, offset)` and reference list.
type NewestFrames = BTreeMap<u32, ((u64, usize), Vec<u64>)>;

impl CurrentReferences {
    fn of(
        &mut self,
        rf: &mut RevitFile,
        revit_version: u32,
        declared: &BTreeSet<u32>,
        category: i64,
        id: u32,
    ) -> Option<&[u64]> {
        let frames = self.0.entry(category).or_insert_with(|| {
            let mut newest = NewestFrames::new();
            let records = crate::partition_element_records::scan_category_records(
                rf,
                revit_version,
                category,
                declared,
            )
            .unwrap_or_default();
            for record in records {
                let key = (
                    crate::partition_names::partition_number(&record.stream),
                    record.offset,
                );
                if newest
                    .get(&record.element_id)
                    .is_none_or(|(held, _)| key > *held)
                {
                    newest.insert(record.element_id, (key, record.references));
                }
            }
            newest
        });
        frames.get(&id).map(|(_, references)| references.as_slice())
    }
}

/// Leave out the plumbing fixtures whose type draws nothing visible
/// ([`crate::native_family_forms::types_without_visible_body`]).
/// Only this category: it is the one where the test was scored on every model
/// with a Revit export and the rule fires, and no other category has both. The
/// export diagnostics count what is left out, and a check that could not run
/// is reported there with every instance kept.
pub(super) fn without_invisible_plumbing_fixture_types(
    rf: &mut RevitFile,
    revit_version: u32,
    products: &mut Vec<DecodedElement>,
) {
    if !crate::native_document::NATIVE_RELEASES.contains(&revit_version) {
        return;
    }
    let type_of = |element: &DecodedElement| {
        element.fields.iter().find_map(|(name, value)| match value {
            InstanceField::ElementId { id, .. } if name == TYPE_ID_FIELD => Some(u64::from(*id)),
            _ => None,
        })
    };
    let is_fixture = |element: &DecodedElement| element.class == "PlumbingFixture";
    let types: BTreeSet<u64> = products
        .iter()
        .filter(|element| is_fixture(element))
        .filter_map(&type_of)
        .collect();
    if types.is_empty() {
        return;
    }
    let invisible = match crate::native_family_forms::types_without_visible_body(rf, &types) {
        Ok(invisible) => invisible,
        Err(error) => {
            rf.note_invisible_type_error(types.len(), error.to_string());
            return;
        }
    };
    if invisible.is_empty() {
        return;
    }
    let mut classes: BTreeMap<String, Vec<crate::partition_element_records::LeftOutElement>> =
        BTreeMap::new();
    products.retain(|element| {
        let left_out =
            is_fixture(element) && type_of(element).is_some_and(|id| invisible.contains(&id));
        if left_out {
            let framed =
                element.provenance.record_kind.as_deref() == Some("partition_element_record");
            classes.entry(element.class.clone()).or_default().push(
                crate::partition_element_records::LeftOutElement {
                    element_id: element.id,
                    class: element.class.clone(),
                    stream: element.provenance.source_stream.clone().filter(|_| framed),
                    offset: element.provenance.source_offset.filter(|_| framed),
                    detail: None,
                },
            );
        }
        !left_out
    });
    rf.note_invisible_type_instances(classes);
}

/// Leave out the family instances written as spaces through their "Export to
/// IFC As" that have no 3D body, as Revit's export leaves them out.
/// Revit writes a family instance that is not a door or a window
/// only when it has a 3D body, and an instance has none when every model
/// form of its family is a void or hidden for its parameter values
/// ([`crate::native_family_forms::instances_without_body`]). On the BIM4LCA
/// terrain office the 92 bicycle bays of "Parking Space", whose "Stripe
/// Y/N" is 0, are not in Revit's export and its 2 car bays are. The
/// export diagnostics count what is left out, and a check that could not
/// run is reported there with every instance kept.
pub(super) fn without_bodiless_family_spaces(
    rf: &mut RevitFile,
    products: &mut Vec<DecodedElement>,
) {
    let candidates: BTreeSet<u64> = products
        .iter()
        .filter(|element| {
            element.fields.iter().any(|(name, value)| {
                matches!(value, InstanceField::String(entity)
                    if name == IFC_EXPORT_AS_FIELD
                        && crate::ifc::category_map::export_value_is_space(entity, &element.class))
            })
        })
        .filter_map(|element| element.id.map(u64::from))
        .collect();
    if candidates.is_empty() {
        return;
    }
    let bodiless = match crate::native_family_forms::instances_without_body(rf, &candidates) {
        Ok(bodiless) => bodiless,
        Err(error) => {
            rf.note_bodiless_family_space_error(candidates.len(), error.to_string());
            return;
        }
    };
    if bodiless.is_empty() {
        return;
    }
    let mut classes: BTreeMap<String, Vec<crate::partition_element_records::LeftOutElement>> =
        BTreeMap::new();
    products.retain(|element| {
        let left_out = element
            .id
            .is_some_and(|id| bodiless.contains(&u64::from(id)));
        if left_out {
            let framed =
                element.provenance.record_kind.as_deref() == Some("partition_element_record");
            classes.entry(element.class.clone()).or_default().push(
                crate::partition_element_records::LeftOutElement {
                    element_id: element.id,
                    class: element.class.clone(),
                    stream: element.provenance.source_stream.clone().filter(|_| framed),
                    offset: element.provenance.source_offset.filter(|_| framed),
                    detail: None,
                },
            );
        }
        !left_out
    });
    rf.note_bodiless_family_spaces(classes);
}

/// Give elements of system families (walls, floors, roofs, ceilings,
/// railings) their type's name.
///
/// The type is the one type-definition record of the element's category
/// that its reference list names (
/// [`crate::partition_type_records::unique_type_reference`]). Its name is
/// read from its serialised data
/// ([`crate::partition_names::find_element_data_names`]). Such a type has no
/// family in the file, since Revit names the system family ("Basic Wall")
/// in its own UI language, so only [`TYPE_ID_FIELD`] and [`TYPE_NAME_FIELD`]
/// are set.
pub(super) fn attach_system_type_names(
    rf: &mut RevitFile,
    revit_version: u32,
    elements: &mut [&mut DecodedElement],
) {
    use crate::partition_type_records as ptr;
    if elements.is_empty() || !ptr::supports_revit_version(revit_version) {
        return;
    }
    let Some(header) = crate::partition_names::element_data_header(revit_version) else {
        return;
    };
    let declared: BTreeSet<u32> = match crate::elem_table::parse_records(rf) {
        Ok(records) => crate::elem_table::declared_ids(&records),
        Err(_) => return,
    };
    let mut type_ids_by_category: BTreeMap<i64, BTreeSet<u32>> = BTreeMap::new();
    let mut picks: Vec<(usize, u32)> = Vec::new();
    let mut current = CurrentReferences::default();
    for (index, element) in elements.iter().enumerate() {
        let Some((mut references, category)) = record_references(rf, element) else {
            continue;
        };
        if let Some(newest) = element
            .id
            .and_then(|id| current.of(rf, revit_version, &declared, category, id))
        {
            references = newest.to_vec();
        }
        let type_ids = type_ids_by_category.entry(category).or_insert_with(|| {
            let records =
                ptr::scan_type_records(rf, revit_version, category, &declared).unwrap_or_default();
            ptr::type_definition_ids(&records)
        });
        if let Some(type_id) = ptr::unique_type_reference(&references, type_ids) {
            picks.push((index, type_id));
        }
    }
    // A railing whose reference list names no type definition holds its
    // type in its own data object.
    let unpicked: Vec<(usize, u32)> = elements
        .iter()
        .enumerate()
        .filter(|(index, element)| {
            element.class == "Railing" && !picks.iter().any(|(picked, _)| picked == index)
        })
        .filter_map(|(index, element)| Some((index, element.id?)))
        .collect();
    let mut railing_names = BTreeMap::new();
    if revit_version >= 2024 && !unpicked.is_empty() {
        let (railing_picks, names) = railing_type_picks(rf, &unpicked);
        picks.extend(railing_picks);
        railing_names = names;
    }
    if picks.is_empty() {
        return;
    }
    let wanted: BTreeSet<u32> = picks.iter().map(|(_, id)| *id).collect();
    let mut names = type_data_names(rf, &header, &wanted);
    // A curtain wall type's data gives no name; its native record
    // does.
    let unread: BTreeSet<u64> = wanted
        .iter()
        .filter(|id| !names.contains_key(id))
        .map(|id| u64::from(*id))
        .collect();
    let curtain_names = if unread.is_empty() {
        BTreeMap::new()
    } else {
        crate::native_walls::curtain_wall_type_names(rf, &unread).unwrap_or_default()
    };
    for (id, name) in curtain_names {
        if let Ok(id) = u32::try_from(id) {
            names.insert(id, Some(name));
        }
    }
    // A railing type with no element data keeps its name in its type object
    // (measured on Revit 2024, and the RE1 railing on 2025).
    let railing_types: BTreeSet<u32> = picks
        .iter()
        .filter(|(index, id)| {
            elements[*index].class == "Railing" && !matches!(names.get(id), Some(Some(_)))
        })
        .map(|(_, id)| *id)
        .collect();
    let railing_tags = rf
        .memoised_schema_classes()
        .ok()
        .and_then(|classes| crate::partition_names::RailingTypeTags::from_classes(&classes));
    if let Some(tags) = railing_tags.filter(|_| revit_version >= 2024 && !railing_types.is_empty())
    {
        // A renamed type leaves its earlier name in the older partitions; the
        // copy in the highest-numbered partition is the current one, as for
        // `type_data_names`. Two names in one partition still cancel.
        let mut newest: BTreeMap<u32, (u64, Option<String>)> = BTreeMap::new();
        for stream in rf.partition_stream_names() {
            let Ok(inflated) = rf.inflated_partition(&stream) else {
                continue;
            };
            let number = crate::partition_names::partition_number(&stream);
            for (id, name) in crate::partition_names::find_railing_type_names(
                inflated.bytes(),
                &railing_types,
                tags,
            ) {
                match newest.get_mut(&id) {
                    Some((held, _)) if number < *held => {}
                    Some((held, slot)) if number == *held => {
                        if slot.as_deref() != Some(name.as_str()) {
                            *slot = None;
                        }
                    }
                    _ => {
                        newest.insert(id, (number, Some(name)));
                    }
                }
            }
        }
        for (id, (_, name)) in newest {
            names.insert(id, name);
        }
    }
    for (id, name) in railing_names {
        if !matches!(names.get(&id), Some(Some(_))) {
            names.insert(id, Some(name));
        }
    }
    // A type whose data frames no name: its code string leads to it.
    let unnamed: BTreeSet<u32> = picks
        .iter()
        .map(|(_, id)| *id)
        .filter(|id| !matches!(names.get(id), Some(Some(_))))
        .collect();
    if !unnamed.is_empty() {
        for (id, name) in coded_type_names(rf, &unnamed) {
            names.insert(id, Some(name));
        }
    }
    // The rest: a type whose value block names it.
    let unnamed: BTreeSet<u32> = picks
        .iter()
        .map(|(_, id)| *id)
        .filter(|id| !matches!(names.get(id), Some(Some(_))))
        .collect();
    if !unnamed.is_empty() {
        for (id, read) in value_block_reads(rf, &unnamed) {
            names.insert(id, Some(read.read.name));
        }
    }
    attach_type_picks(elements, picks, &names);
}

/// Type id and name, and phase created, of the elements of `category` whose
/// reference lists are `lists` (ElementId, list): the same reads as
/// [`attach_system_type_names`] and [`attach_phases_created`], for elements
/// that are not decoded instances (Structural Beam Systems).
pub(crate) fn types_and_phases_for_lists(
    rf: &mut RevitFile,
    category: i64,
    lists: &BTreeMap<u32, Vec<u64>>,
) -> (BTreeMap<u32, (u32, String)>, BTreeMap<u32, String>) {
    use crate::partition_type_records as ptr;
    let mut phase_of = BTreeMap::new();
    if let Ok(phases) = crate::partition_phases::phase_names(rf) {
        for (id, list) in lists {
            if let Some((_, name)) = crate::partition_phases::phase_created(list, &phases) {
                phase_of.insert(*id, name.to_string());
            }
        }
    }
    let Ok(version) = rf.basic_file_info().map(|info| info.version) else {
        return (BTreeMap::new(), phase_of);
    };
    let (Some(header), true) = (
        crate::partition_names::element_data_header(version),
        ptr::supports_revit_version(version),
    ) else {
        return (BTreeMap::new(), phase_of);
    };
    let Ok(records) = crate::elem_table::parse_records(rf) else {
        return (BTreeMap::new(), phase_of);
    };
    let declared = crate::elem_table::declared_ids(&records);
    let type_ids = ptr::type_definition_ids(
        &ptr::scan_type_records(rf, version, category, &declared).unwrap_or_default(),
    );
    let picks: BTreeMap<u32, u32> = lists
        .iter()
        .filter_map(|(id, list)| Some((*id, ptr::unique_type_reference(list, &type_ids)?)))
        .collect();
    let wanted: BTreeSet<u32> = picks.values().copied().collect();
    let mut names = type_data_names(rf, &header, &wanted);
    for pass in 0..2 {
        let unnamed: BTreeSet<u32> = wanted
            .iter()
            .copied()
            .filter(|id| !matches!(names.get(id), Some(Some(_))))
            .collect();
        if unnamed.is_empty() {
            break;
        }
        if pass == 0 {
            for (id, name) in coded_type_names(rf, &unnamed) {
                names.insert(id, Some(name));
            }
        } else {
            for (id, read) in value_block_reads(rf, &unnamed) {
                names.insert(id, Some(read.read.name));
            }
        }
    }
    let typed = picks
        .into_iter()
        .filter_map(|(id, ty)| Some((id, (ty, names.get(&ty)?.clone()?))))
        .collect();
    (typed, phase_of)
}

/// The type of each of `railings` (index, ElementId) that its own
/// `BaseRailing` data object names: the one ElementId in the
/// object's payload, at any offset, of a railing type's `StairsRailingAttr`
/// object. RE1 Architecture's railing 462556 holds its type 446543, Revit's
/// `IfcRailingType` Tag, at +283 of its 329-byte object. With the picks, the
/// picked types' names their own objects hold
/// ([`crate::partition_names::railing_type_name_in`]), each from the latest
/// partition that holds one, as a type's element data name is: on the
/// BIM4LCA housing every railing type has an older copy with its Finnish name
/// and a newer one with the English name Revit's export carries. Revit 2024 and
/// later.
pub(super) fn railing_type_picks(
    rf: &mut RevitFile,
    railings: &[(usize, u32)],
) -> (Vec<(usize, u32)>, BTreeMap<u32, String>) {
    let Ok(classes) = rf.memoised_schema_classes() else {
        return Default::default();
    };
    let tag_of = |name: &str| {
        classes
            .classes
            .iter()
            .find(|class| class.name == name)
            .map(|class| u32::from(class.tag))
    };
    let (Some(railing_tag), Some(type_tag)) = (tag_of("BaseRailing"), tag_of("StairsRailingAttr"))
    else {
        return Default::default();
    };
    let name_tags = crate::partition_names::RailingTypeTags::from_classes(&classes);
    let wanted: BTreeSet<u32> = railings.iter().map(|(_, id)| *id).collect();
    let mut type_ids: BTreeSet<u32> = BTreeSet::new();
    // Each type's name where its copies that read one agree; a copy whose
    // name does not read takes no part.
    let mut type_names: BTreeMap<u32, (u64, Option<String>)> = BTreeMap::new();
    let mut held: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    for stream in rf.partition_stream_names() {
        let Ok(inflated) = rf.inflated_partition(&stream) else {
            continue;
        };
        let buf = inflated.bytes();
        let number = crate::partition_names::partition_number(&stream);
        let Ok(objects) = rf.memoised_partition_data_objects(&stream) else {
            continue;
        };
        for &(p, object) in objects.iter() {
            let class = object.class & 0xffff;
            if class == type_tag {
                let name = name_tags.and_then(|tags| {
                    crate::partition_names::railing_type_name_in(&buf[p..object.end], tags)
                });
                type_ids.insert(object.element_id);
                if let Some(name) = name {
                    // The copy in the highest-numbered partition is the
                    // current name of a renamed type.
                    match type_names.get_mut(&object.element_id) {
                        Some((held, _)) if number < *held => {}
                        Some((held, slot)) if number == *held => {
                            if slot.as_deref() != Some(name.as_str()) {
                                *slot = None;
                            }
                        }
                        _ => {
                            type_names.insert(object.element_id, (number, Some(name)));
                        }
                    }
                }
            } else if class == railing_tag && wanted.contains(&object.element_id) {
                let payload =
                    &buf[p + crate::partition_room_parameters::DATA_OBJECT_HEADER..object.end];
                let values = held.entry(object.element_id).or_default();
                for word in payload.windows(4) {
                    values.insert(u32::from_le_bytes(word.try_into().expect("4 bytes")));
                }
            }
        }
    }
    let picks: Vec<(usize, u32)> = railings
        .iter()
        .filter_map(|(index, id)| {
            let mut named = held.get(id)?.intersection(&type_ids);
            match (named.next(), named.next()) {
                (Some(&type_id), None) => Some((*index, type_id)),
                _ => None,
            }
        })
        .collect();
    let names = picks
        .iter()
        .filter_map(|(_, type_id)| Some((*type_id, type_names.get(type_id)?.1.clone()?)))
        .collect();
    (picks, names)
}

/// Each wanted type's name, read from its serialised data
/// ([`crate::partition_names::find_element_data_names`]), from the latest
/// partition that holds one; `None` when two copies there
/// disagree. On the BIM4LCA office's permit file every type has an older copy
/// with its Finnish name and a newer one with its English name.
pub(super) fn type_data_names(
    rf: &mut RevitFile,
    header: &[u8; 10],
    wanted: &BTreeSet<u32>,
) -> BTreeMap<u32, Option<String>> {
    use crate::partition_room_parameters::{LatestCopy, partition_number};
    let mut names = LatestCopy::default();
    for stream in rf.partition_stream_names() {
        let Ok(inflated) = rf.inflated_partition(&stream) else {
            continue;
        };
        let partition = partition_number(&stream);
        for (id, name) in
            crate::partition_names::find_element_data_names(inflated.bytes(), header, wanted)
        {
            names.offer(id, partition, name);
        }
    }
    names.into_options()
}

/// The names of the types `wanted` from their own data objects, on every
/// release whose element-data header is measured ([`element_data_layout`]:
/// 2015 to 2026); the copy in the highest-numbered partition wins. For a type
/// whose id is known, where the type's partition record is not read.
///
/// [`element_data_layout`]: crate::partition_names::element_data_layout
pub(crate) fn data_type_names(
    rf: &mut RevitFile,
    revit_version: u32,
    wanted: &BTreeSet<u32>,
) -> BTreeMap<u32, String> {
    let Some(layout) = crate::partition_names::element_data_layout(revit_version) else {
        return BTreeMap::new();
    };
    let mut best: BTreeMap<u32, (u64, String)> = BTreeMap::new();
    for stream in rf.partition_stream_names() {
        let Ok(inflated) = rf.inflated_partition(&stream) else {
            continue;
        };
        let number = crate::partition_names::partition_number(&stream);
        let found = if layout.wide_id {
            crate::partition_names::find_element_data_names(
                inflated.bytes(),
                &layout.header,
                wanted,
            )
        } else {
            crate::partition_names::find_element_data_names_32(
                inflated.bytes(),
                revit_version,
                wanted,
            )
        };
        for (id, name) in found {
            match best.get(&id) {
                Some((held, _)) if number <= *held => {}
                _ => {
                    best.insert(id, (number, name));
                }
            }
        }
    }
    best.into_iter().map(|(id, (_, name))| (id, name)).collect()
}

/// BuiltInParameter whose string opens the run a type's name ends, in the
/// serialised data of a curtain wall, railing, stair or ramp type of the
/// Nordic Revit 2024 samples: the type's code, a parameter id, then its name.
const TYPE_CODE_PARAMETER: i64 = -1_140_422;

/// A `u32 n · UTF-16 × n` string at `at` of `buf`, `n` in 1 to `limit`, with
/// no control character, and the offset past it.
fn text_entry(buf: &[u8], at: usize, limit: usize) -> Option<(String, usize)> {
    let n = usize::try_from(u32::from_le_bytes(buf.get(at..at + 4)?.try_into().ok()?)).ok()?;
    if !(1..=limit).contains(&n) {
        return None;
    }
    let units: Vec<u16> = buf
        .get(at + 4..at + 4 + 2 * n)?
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    let text = String::from_utf16(&units).ok()?;
    (!text.chars().any(char::is_control)).then_some((text, at + 4 + 2 * n))
}

/// The names of the types `wanted`, from their own data objects, which carry
/// no framed name: after the code string, `TYPE_CODE_PARAMETER`'s, come a
/// parameter id (`u64`), a zero `u32` and the name (on a floor, roof or ceiling
/// type behind a framed field tag). The copy in the
/// highest-numbered partition wins. A type whose object is not verified,
/// or shows another shape, gets nothing.
fn coded_type_names(rf: &mut RevitFile, wanted: &BTreeSet<u32>) -> BTreeMap<u32, String> {
    use crate::partition_room_parameters as prp;
    let key = TYPE_CODE_PARAMETER.to_le_bytes();
    let mut newest: BTreeMap<u32, (u64, String)> = BTreeMap::new();
    for stream in rf.partition_stream_names() {
        let Ok(inflated) = rf.inflated_partition(&stream) else {
            continue;
        };
        let number = crate::partition_names::partition_number(&stream);
        let buf = inflated.bytes();
        for (at, object) in prp::data_objects(buf) {
            if !wanted.contains(&object.element_id) {
                continue;
            }
            let payload = &buf[at + prp::DATA_OBJECT_HEADER..object.end];
            let name = memchr::memmem::find(payload, &key).and_then(|hit| {
                let (_, after) = text_entry(payload, hit + key.len(), 64)?;
                let parameter = u64::from_le_bytes(payload.get(after..after + 8)?.try_into().ok()?);
                let zero = u32::from_le_bytes(payload.get(after + 8..after + 12)?.try_into().ok()?);
                if parameter == 0 || parameter > u64::from(u32::MAX) || zero != 0 {
                    return None;
                }
                // The name follows, or, on a floor, roof or ceiling type, a
                // `1` and a framed field tag (`ff ff ff ff · u16`) first.
                let framed = payload.get(after + 12..after + 20)
                    == Some(&[1, 0, 0, 0, 0xff, 0xff, 0xff, 0xff][..]);
                let at = after + 12 + if framed { 10 } else { 0 };
                text_entry(payload, at, 256).map(|(name, _)| name)
            });
            let Some(name) = name else {
                continue;
            };
            if newest
                .get(&object.element_id)
                .is_none_or(|(held, _)| number > *held)
            {
                newest.insert(object.element_id, (number, name));
            }
        }
    }
    newest
        .into_iter()
        .map(|(id, (_, name))| (id, name))
        .collect()
}

/// The types of `wanted` whose value block names them (/// [`crate::partition_type_names`]): the read, with the class the type's
/// record names in the file's schema. A type whose block does not parse has
/// no entry.
pub(super) fn value_block_reads(
    rf: &mut RevitFile,
    wanted: &BTreeSet<u32>,
) -> BTreeMap<u32, crate::partition_type_names::ReadAt> {
    // Measured on Revit 2024 only: 27 sample models, all of that release.
    if rf.basic_file_info().map(|info| info.version).ok()
        != Some(crate::partition_type_names::MEASURED_REVIT_VERSION)
    {
        return BTreeMap::new();
    }
    crate::partition_type_names::read_type_names(rf, wanted)
        .map(|(read, _)| read)
        .unwrap_or_default()
}

/// Set [`TYPE_ID_FIELD`] and [`TYPE_NAME_FIELD`] on each picked element
/// whose type has one agreed name.
pub(super) fn attach_type_picks(
    elements: &mut [&mut DecodedElement],
    picks: Vec<(usize, u32)>,
    names: &BTreeMap<u32, Option<String>>,
) {
    for (index, type_id) in picks {
        let Some(Some(name)) = names.get(&type_id) else {
            continue;
        };
        let element = &mut elements[index];
        element.fields.push((
            TYPE_ID_FIELD.into(),
            InstanceField::ElementId {
                tag: 0,
                id: type_id,
            },
        ));
        element
            .fields
            .push((TYPE_NAME_FIELD.into(), InstanceField::String(name.clone())));
    }
}

/// Give wall sweeps their type and family, or Revit's name for one without a
/// type.
///
/// A wall sweep's type is a system-family type with no type-definition
/// record of the sweep category, so the join finds none. Its record
/// names it among other things: its profile (a family type with a name
/// entry), its material, the walls it runs along. The type is the
/// one id the record names that has a type object
/// ([`crate::partition_names::find_type_object_ids`]), no name entry, and
/// an element-data name; the sweep is then named `Wall Sweep:<type>:<id>`.
/// A sweep whose record names no such object at all is a sweep of a wall
/// type's own structure, which Revit names by its ElementId alone.
///
/// Measured on Snowdon Towers only (Revit 2024): 249 sweeps take a type and
/// 9 are named by ElementId, all as Revit's export names them.
pub(super) fn attach_wall_sweep_types(
    rf: &mut RevitFile,
    revit_version: u32,
    products: &mut [DecodedElement],
) {
    if revit_version != 2024 {
        return;
    }
    let Some(header) = crate::partition_names::element_data_header(revit_version) else {
        return;
    };
    let has = |element: &DecodedElement, field: &str| {
        element.fields.iter().any(|(name, _)| name == field)
    };
    let mut lists: Vec<(usize, BTreeSet<u32>)> = Vec::new();
    for (index, element) in products.iter().enumerate() {
        if element.class != "WallSweep"
            || has(element, TYPE_NAME_FIELD)
            || has(element, FAMILY_NAME_FIELD)
            || has(element, ELEMENT_NAME_FIELD)
        {
            continue;
        }
        let Some((references, _)) = record_references(rf, element) else {
            continue;
        };
        let named: BTreeSet<u32> = references
            .iter()
            .filter_map(|&slot| u32::try_from(slot).ok())
            .filter(|id| Some(*id) != element.id)
            .collect();
        lists.push((index, named));
    }
    if lists.is_empty() {
        return;
    }
    let entries = rf.memoised_element_names();
    let wanted: BTreeSet<u32> = lists
        .iter()
        .flat_map(|(_, ids)| ids.iter().copied())
        .filter(|id| !entries.entries.contains_key(id))
        .collect();
    let mut objects: BTreeSet<u32> = BTreeSet::new();
    for stream in rf.partition_stream_names() {
        let Ok(inflated) = rf.inflated_partition(&stream) else {
            continue;
        };
        objects.extend(crate::partition_names::find_type_object_ids(
            inflated.bytes(),
            &wanted,
        ));
    }
    let mut names = type_data_names(rf, &header, &objects);
    // A sweep type whose data frames no name keeps it in its value block.
    let unnamed: BTreeSet<u32> = objects
        .iter()
        .copied()
        .filter(|id| !matches!(names.get(id), Some(Some(_))))
        .collect();
    if !unnamed.is_empty() {
        for (id, read) in value_block_reads(rf, &unnamed) {
            names.insert(id, Some(read.read.name));
        }
    }
    for (index, ids) in lists {
        let types: Vec<u32> = ids
            .iter()
            .copied()
            .filter(|id| objects.contains(id))
            .collect();
        let element = &mut products[index];
        match types.as_slice() {
            [] => {
                if let Some(id) = element.id {
                    element.fields.push((
                        ELEMENT_NAME_FIELD.into(),
                        InstanceField::String(id.to_string()),
                    ));
                }
            }
            [type_id] => {
                let Some(Some(name)) = names.get(type_id) else {
                    continue;
                };
                element.fields.push((
                    TYPE_ID_FIELD.into(),
                    InstanceField::ElementId {
                        tag: 0,
                        id: *type_id,
                    },
                ));
                element
                    .fields
                    .push((TYPE_NAME_FIELD.into(), InstanceField::String(name.clone())));
                element.fields.push((
                    FAMILY_NAME_FIELD.into(),
                    InstanceField::String("Wall Sweep".into()),
                ));
                element.fields.push((
                    FAMILY_NAME_SOURCE_FIELD.into(),
                    InstanceField::String(SYSTEM_FAMILY_SOURCE.into()),
                ));
            }
            _ => {}
        }
    }
}

/// The field of a type's `HostObjAttr` that is set exactly when the type has
/// compound layers.
const COMPOUND_STRUCTURE_POINTER: &str = "HostObjAttr.m_pCompoundStructure";

/// The schema class of a curtain roof type's record: Revit's Sloped Glazing.
const SLOPED_GLAZING_TYPE_CLASS: &str = "CurtainRoofAttributes";

/// The schema class of a wall foundation type's record.
const WALL_FOUNDATION_TYPE_CLASS: &str = "ContFootingType";

/// The schema class of a legacy stair type's record.
pub(super) const LEGACY_STAIR_TYPE_CLASS: &str = "StairsAttributes";

/// The schema class of a model text type's record.
const MODEL_TEXT_TYPE_CLASS: &str = "Text3dAttrSymbol";
/// Give each pipe, wall sweep and ramp still without a type, or a name of
/// its own, the type its native record names
/// ([`crate::native_element_types::system_type_names`]). Its family
/// is then its class's system family ([`system_family`]), and a sweep's is
/// Wall Sweep.
pub(super) fn attach_native_type_names(rf: &mut RevitFile, products: &mut [DecodedElement]) {
    let has = |element: &DecodedElement, field: &str| {
        element.fields.iter().any(|(name, _)| name == field)
    };
    let wanted: BTreeMap<u64, String> = products
        .iter()
        .filter(|element| {
            crate::native_element_types::reads_class(&element.class)
                && !has(element, TYPE_NAME_FIELD)
                && !has(element, ELEMENT_NAME_FIELD)
        })
        .filter_map(|element| Some((u64::from(element.id?), element.class.clone())))
        .collect();
    if wanted.is_empty() {
        return;
    }
    let Ok(found) = crate::native_element_types::system_type_names(rf, &wanted) else {
        return;
    };
    for element in products.iter_mut() {
        let Some((type_id, name)) = element.id.and_then(|id| found.get(&u64::from(id))) else {
            continue;
        };
        let Ok(type_id) = u32::try_from(*type_id) else {
            continue;
        };
        element.fields.push((
            TYPE_ID_FIELD.into(),
            InstanceField::ElementId {
                tag: 0,
                id: type_id,
            },
        ));
        element
            .fields
            .push((TYPE_NAME_FIELD.into(), InstanceField::String(name.clone())));
        if element.class == "WallSweep" {
            element.fields.push((
                FAMILY_NAME_FIELD.into(),
                InstanceField::String("Wall Sweep".into()),
            ));
            element.fields.push((
                FAMILY_NAME_SOURCE_FIELD.into(),
                InstanceField::String(SYSTEM_FAMILY_SOURCE.into()),
            ));
        }
    }
}

/// Give model text its type and family.
///
/// A model text element is in the Generic Models category, but it is not a
/// family instance: its reference list names a text type, which has no name
/// entry and no type-definition record of the category. The text
/// type is the one id the list names whose type object carries a font
/// ([`crate::partition_names::find_text_type_names`]), and Revit names the
/// element `Model Text:<type>:<ElementId>`. Measured on Snowdon Towers only
/// (Revit 2024), where it names all 7 model texts as Revit's export does.
pub(super) fn attach_model_text_types(
    rf: &mut RevitFile,
    revit_version: u32,
    products: &mut [DecodedElement],
) {
    if revit_version != 2024 {
        return;
    }
    let has = |element: &DecodedElement, field: &str| {
        element.fields.iter().any(|(name, _)| name == field)
    };
    let mut lists: Vec<(usize, BTreeSet<u32>)> = Vec::new();
    for (index, element) in products.iter().enumerate() {
        if element.class != "GenericModel"
            || has(element, TYPE_NAME_FIELD)
            || has(element, FAMILY_NAME_FIELD)
        {
            continue;
        }
        let Some((references, _)) = record_references(rf, element) else {
            continue;
        };
        let named: BTreeSet<u32> = references
            .iter()
            .filter_map(|&slot| u32::try_from(slot).ok())
            .filter(|id| Some(*id) != element.id)
            .collect();
        lists.push((index, named));
    }
    if lists.is_empty() {
        return;
    }
    let wanted: BTreeSet<u32> = lists
        .iter()
        .flat_map(|(_, ids)| ids.iter().copied())
        .collect();
    let mut names: BTreeMap<u32, Option<String>> = BTreeMap::new();
    for stream in rf.partition_stream_names() {
        let Ok(inflated) = rf.inflated_partition(&stream) else {
            continue;
        };
        for (id, name) in crate::partition_names::find_text_type_names(inflated.bytes(), &wanted) {
            match names.get(&id) {
                Some(Some(held)) if *held != name => {
                    names.insert(id, None);
                }
                Some(None) => {}
                _ => {
                    names.insert(id, Some(name));
                }
            }
        }
    }
    // A text type that does not frame its name (DACH's "INNOCAD"): its value
    // block names it. Only a model text type's class counts.
    let unframed: BTreeSet<u32> = wanted
        .iter()
        .copied()
        .filter(|id| !matches!(names.get(id), Some(Some(_))))
        .collect();
    if !unframed.is_empty() {
        for (id, read) in value_block_reads(rf, &unframed) {
            if read.read.class_name != MODEL_TEXT_TYPE_CLASS {
                continue;
            }
            match names.get(&id) {
                Some(Some(held)) if *held != read.read.name => {
                    names.insert(id, None);
                }
                Some(None) => {}
                _ => {
                    names.insert(id, Some(read.read.name));
                }
            }
        }
    }
    for (index, ids) in lists {
        let text_types: Vec<(u32, &String)> = ids
            .iter()
            .filter_map(|id| match names.get(id) {
                Some(Some(name)) => Some((*id, name)),
                _ => None,
            })
            .collect();
        let [(type_id, name)] = text_types.as_slice() else {
            continue;
        };
        let element = &mut products[index];
        element.fields.push((
            TYPE_ID_FIELD.into(),
            InstanceField::ElementId {
                tag: 0,
                id: *type_id,
            },
        ));
        element.fields.push((
            TYPE_NAME_FIELD.into(),
            InstanceField::String((*name).clone()),
        ));
        element.fields.push((
            FAMILY_NAME_FIELD.into(),
            InstanceField::String("Model Text".into()),
        ));
        element.fields.push((
            FAMILY_NAME_SOURCE_FIELD.into(),
            InstanceField::String(TYPE_KIND_FAMILY_SOURCE.into()),
        ));
    }
}

/// Give a curtain panel that is a wall its wall type.
///
/// Revit lets a curtain grid cell hold a basic wall instead of a panel. The
/// element keeps the panel category, but its reference list names a wall
/// type rather than a panel type, so [`attach_system_type_names`] finds no
/// type for it. Here the type is the one wall-type record the list names,
/// taken only when that type has compound layers
/// ([`crate::partition_compound_structure::scan_type_layers`]): a panel's
/// list also names its curtain wall, whose type is a wall type with none.
/// [`attach_system_family_names`] then names its family "Basic Wall".
pub(super) fn attach_panel_wall_types(
    rf: &mut RevitFile,
    revit_version: u32,
    panels: &mut [&mut DecodedElement],
) {
    use crate::partition_type_records as ptr;
    if panels.is_empty()
        || !ptr::supports_revit_version(revit_version)
        || !crate::partition_compound_structure::COMPOUND_STRUCTURE_SUPPORTED_REVIT_VERSIONS
            .contains(&revit_version)
    {
        return;
    }
    let Some(header) = crate::partition_names::element_data_header(revit_version) else {
        return;
    };
    let declared: BTreeSet<u32> = match crate::elem_table::parse_records(rf) {
        Ok(records) => crate::elem_table::declared_ids(&records),
        Err(_) => return,
    };
    let wall_types = ptr::type_definition_ids(
        &ptr::scan_type_records(
            rf,
            revit_version,
            crate::partition_element_records::OST_WALLS,
            &declared,
        )
        .unwrap_or_default(),
    );
    let mut picks: Vec<(usize, u32)> = Vec::new();
    for (index, panel) in panels.iter().enumerate() {
        let Some((references, _)) = record_references(rf, panel) else {
            continue;
        };
        if let Some(type_id) = ptr::unique_type_reference(&references, &wall_types) {
            picks.push((index, type_id));
        }
    }
    if picks.is_empty() {
        return;
    }
    let candidates: BTreeSet<u32> = picks.iter().map(|(_, id)| *id).collect();
    let materials: BTreeSet<u32> =
        ptr::scan_type_records(rf, revit_version, ptr::OST_MATERIALS, &declared)
            .unwrap_or_default()
            .iter()
            .map(|record| record.element_id)
            .collect();
    let layered: BTreeSet<u32> = crate::partition_compound_structure::scan_type_layers(
        rf,
        revit_version,
        &candidates,
        &materials,
        &declared,
    )
    .unwrap_or_default()
    .into_iter()
    .filter(|(_, layers)| !layers.is_empty())
    .map(|(id, _)| id)
    .collect();
    picks.retain(|(_, id)| layered.contains(id));
    let names = type_data_names(rf, &header, &layered);
    attach_type_picks(panels, picks, &names);
    // Revit exports such a panel as a curtain wall of its own.
    for panel in panels.iter_mut() {
        let wall_type = panel.fields.iter().any(|(name, value)| {
            matches!(value, InstanceField::ElementId { id, .. } if name == TYPE_ID_FIELD && layered.contains(id))
        });
        if wall_type {
            panel
                .fields
                .push((CURTAIN_PANEL_WALL_FIELD.into(), InstanceField::Bool(true)));
        }
    }
}

/// Name each record-backed element's class: the tag its record
/// holds at `+0x4a` is the definition ordinal of a class in the file's own
/// schema, `SWall`, `ArcWall`, `Floor`, `FamilyInstance` and so on. An
/// element whose tag resolves to no class gets no name.
pub(super) fn attach_revit_classes(
    classes: &crate::formats::SchemaClasses,
    elements: &mut [DecodedElement],
) {
    for element in elements.iter_mut() {
        let tag = element.fields.iter().find_map(|(name, value)| match value {
            InstanceField::Integer { value, .. } if name == CLASS_TAG_FIELD => {
                u16::try_from(*value).ok()
            }
            _ => None,
        });
        if let Some(class) = tag.and_then(|tag| classes.by_tag(tag)) {
            element.fields.push((
                REVIT_CLASS_FIELD.into(),
                InstanceField::String(class.name.clone()),
            ));
        }
    }
}

/// Give family instances their type and family from the name entries.
///
/// Fail-closed: an element whose reference list also names another
/// type-definition record of its category gets nothing. Name entries exist
/// for family types only, so a wall that names its system type (no entry)
/// and a family type (DACH: 2 walls, "Golden Nugget_Wandabschluss") would
/// otherwise take the family type as its own. `type_definitions` caches the
/// type-definition ids per category across calls.
pub(super) fn attach_family_and_type_names(
    rf: &mut RevitFile,
    revit_version: u32,
    elements: &mut [DecodedElement],
    type_definitions: &mut BTreeMap<i64, BTreeSet<u32>>,
) {
    use crate::partition_type_records as ptr;
    let names = rf.memoised_element_names();
    if names.entries.is_empty() {
        return;
    }
    let declared: BTreeSet<u32> = match crate::elem_table::parse_records(rf) {
        Ok(records) => crate::elem_table::declared_ids(&records),
        Err(_) => BTreeSet::new(),
    };
    let mut current = CurrentReferences::default();
    for element in elements.iter_mut() {
        let Some(own) = element.id else {
            continue;
        };
        let Some((mut references, category)) = record_references(rf, element) else {
            continue;
        };
        if let Some(newest) = current.of(rf, revit_version, &declared, category, own) {
            references = newest.to_vec();
        }
        let Some(type_id) =
            crate::partition_names::resolve_type(&names, &references, category, own)
        else {
            continue;
        };
        let definitions = type_definitions.entry(category).or_insert_with(|| {
            ptr::scan_type_records(rf, revit_version, category, &declared)
                .map(|records| ptr::type_definition_ids(&records))
                .unwrap_or_default()
        });
        if references.iter().any(|&id| {
            u32::try_from(id)
                .is_ok_and(|id| id != own && id != type_id && definitions.contains(&id))
        }) {
            continue;
        }
        let Some(family_id) = crate::partition_names::resolve_family(&names, type_id) else {
            continue;
        };
        let (Some(type_entry), Some(family_entry)) =
            (names.entries.get(&type_id), names.entries.get(&family_id))
        else {
            continue;
        };
        element.fields.push((
            TYPE_ID_FIELD.into(),
            InstanceField::ElementId {
                tag: 0,
                id: type_id,
            },
        ));
        element.fields.push((
            TYPE_NAME_FIELD.into(),
            InstanceField::String(type_entry.name.clone()),
        ));
        element.fields.push((
            FAMILY_NAME_FIELD.into(),
            InstanceField::String(family_entry.name.clone()),
        ));
    }
}

/// The Revit system family of a record-backed element's type.
/// Revit derives the name rather than storing it, from the type's
/// kind, and the category gives it where only one system family fits:
/// - a curtain wall is a Curtain Wall, and a railing a Railing;
/// - a floor is a Floor, and a building pad a Pad;
/// - a slab edge is a Slab Edge, and a ramp a Ramp;
/// - a wall, ceiling or roof whose type has compound layers (`has_layers`)
///   is a Basic Wall, a Compound Ceiling or a Basic Roof, since a Curtain
///   or Stacked Wall, a Basic Ceiling and Sloped Glazing have none;
/// - a curtain panel whose type is a wall type with layers is a Basic Wall;
///   - a pipe is a Pipe Types, Revit's one pipe system family.
///
/// Measured against the `Family:Type:ElementId` names Revit's own IFC
/// export gives the same elements, on every record-backed wall, floor,
/// ceiling, roof and building pad of Snowdon Towers, Core Interior and RE1
/// Architecture: 1,444 walls, 283 floors, 74 ceilings, 20 roofs and 1 pad.
pub fn system_family(class: &str, has_layers: bool) -> Option<&'static str> {
    match (class, has_layers) {
        (CURTAIN_WALL_CLASS, _) => Some("Curtain Wall"),
        ("Railing", _) => Some("Railing"),
        ("Floor", _) => Some("Floor"),
        ("BuildingPad", _) => Some("Pad"),
        ("SlabEdge", _) => Some("Slab Edge"),
        ("Ramp", _) => Some("Ramp"),
        ("Pipe", _) => Some("Pipe Types"),
        ("Wall" | "CurtainWallPanel", true) => Some("Basic Wall"),
        ("Ceiling", true) => Some("Compound Ceiling"),
        ("Roof", true) => Some("Basic Roof"),
        ("StructuralFoundation", true) => Some("Foundation Slab"),
        _ => None,
    }
}

/// The name Revit's export gives system family `family` in a file saved
/// in `locale` (`BasicFileInfo`'s "Locale when saved", e.g. `ENU`, `PTB`).
///
/// A system family's name is not in the file: Revit supplies it in the
/// language the file is saved in. Four files saved in `PTB` (two Revit
/// 2023, two Revit 2025) name their walls, floors and roofs this way
/// in Revit's own export, 28 walls, 3 floors and 1 roof, and no other
/// system family occurs in them. Every other family, and every
/// other locale, keeps the `ENU` name [`system_family`] gives, which is
/// what the `ENU` files measure.
pub fn localized_system_family(family: &'static str, locale: Option<&str>) -> &'static str {
    match (locale, family) {
        (Some("PTB"), "Basic Wall") => "Parede básica",
        (Some("PTB"), "Floor") => "Piso",
        (Some("PTB"), "Basic Roof") => "Telhado básico",
        _ => family,
    }
}

/// Give each system-family element with a type name its system
/// family's name ([`system_family`]), so it is named `Family:Type:ElementId`
/// as Revit names it. Whether a wall's, ceiling's or roof's type has
/// compound layers is the type's: an element of the type drawn in layers
/// shows it, and otherwise the type's own data is read
/// ([`crate::partition_compound_structure::scan_type_layers`]).
pub(super) fn attach_system_family_names(
    rf: &mut RevitFile,
    revit_version: u32,
    elements: &mut [&mut Vec<DecodedElement>],
) {
    use crate::partition_compound_structure as pcs;
    let type_of = |element: &DecodedElement| {
        element.fields.iter().find_map(|(name, value)| match value {
            InstanceField::ElementId { id, .. } if name == TYPE_ID_FIELD => Some(*id),
            _ => None,
        })
    };
    let shows_layers = |element: &DecodedElement| {
        element.fields.iter().any(|(name, _)| {
            name == WALL_LAYERS_FIELD
                || name == SLAB_LAYERS_FIELD
                || name == ROOF_TYPE_THICKNESS_FIELD
        })
    };
    let mut layered: BTreeSet<u32> = elements
        .iter()
        .flat_map(|list| list.iter())
        .filter(|element| shows_layers(element))
        .filter_map(type_of)
        .collect();
    let mut disagree: BTreeSet<u32> = BTreeSet::new();
    let pending: BTreeSet<u32> = elements
        .iter()
        .flat_map(|list| list.iter())
        .filter(|element| {
            matches!(
                element.class.as_str(),
                "Wall" | "Ceiling" | "Roof" | "CurtainWallPanel" | "StructuralFoundation"
            )
        })
        .filter_map(type_of)
        .filter(|id| !layered.contains(id))
        .collect();
    if !pending.is_empty() && pcs::layer_layout(revit_version).is_some() {
        if let Ok(records) = crate::elem_table::parse_records(rf) {
            let declared = crate::elem_table::declared_ids(&records);
            // Revit 2015 to 2023 have no type records, and their materials
            // no element records either: a layer's material is only held to
            // be declared.
            let materials: BTreeSet<u32> =
                if crate::partition_element_records_2023::is_32bit_release(revit_version) {
                    declared.clone()
                } else {
                    crate::partition_type_records::scan_type_records(
                        rf,
                        revit_version,
                        crate::partition_type_records::OST_MATERIALS,
                        &declared,
                    )
                    .unwrap_or_default()
                    .iter()
                    .map(|record| record.element_id)
                    .collect()
                };
            for (id, agree) in
                pcs::scan_layered_types(rf, revit_version, &pending, &materials, &declared)
            {
                layered.insert(id);
                if !agree {
                    disagree.insert(id);
                }
            }
        }
    }
    // the type's value block says whether the type has compound
    // layers (its `HostObjAttr` compound structure is set) and its record
    // names the class, both the file's own. A type whose copies disagreed
    // about its layers, or whose layers were not read, takes the block's
    // word; one whose block does not parse keeps what was read.
    let mut class_family: BTreeMap<u32, &'static str> = BTreeMap::new();
    let check: BTreeSet<u32> = elements
        .iter()
        .flat_map(|list| list.iter())
        .filter(|element| {
            let has = |field: &str| element.fields.iter().any(|(name, _)| name == field);
            has(TYPE_NAME_FIELD)
                && !has(FAMILY_NAME_FIELD)
                && matches!(
                    element.class.as_str(),
                    "Wall" | "Ceiling" | "Roof" | "CurtainWallPanel" | "StructuralFoundation"
                )
        })
        .filter_map(type_of)
        .filter(|id| !layered.contains(id) || disagree.contains(id))
        .collect();
    if !check.is_empty() {
        for (id, read) in value_block_reads(rf, &check) {
            let layers = read
                .read
                .pointers
                .iter()
                .find(|(name, _)| name == COMPOUND_STRUCTURE_POINTER)
                .map(|(_, set)| *set);
            if layers == Some(true) {
                layered.insert(id);
                disagree.remove(&id);
            }
            match read.read.class_name.as_str() {
                SLOPED_GLAZING_TYPE_CLASS => class_family.insert(id, "Sloped Glazing"),
                WALL_FOUNDATION_TYPE_CLASS => class_family.insert(id, "Wall Foundation"),
                _ => None,
            };
        }
    }
    let locale = rf.basic_file_info().ok().and_then(|info| info.locale);
    for element in elements.iter_mut().flat_map(|list| list.iter_mut()) {
        let has = |field: &str| element.fields.iter().any(|(name, _)| name == field);
        if !has(TYPE_NAME_FIELD) || has(FAMILY_NAME_FIELD) {
            continue;
        }
        let has_layers = type_of(element).is_some_and(|id| layered.contains(&id));
        let by_class = type_of(element)
            .and_then(|id| class_family.get(&id).copied())
            .filter(|family| match *family {
                "Sloped Glazing" => element.class == "Roof",
                _ => element.class == "StructuralFoundation",
            });
        let family = by_class.or_else(|| system_family(&element.class, has_layers));
        let Some(family) = family else {
            continue;
        };
        element.fields.push((
            FAMILY_NAME_FIELD.into(),
            InstanceField::String(localized_system_family(family, locale.as_deref()).into()),
        ));
        element.fields.push((
            FAMILY_NAME_SOURCE_FIELD.into(),
            InstanceField::String(SYSTEM_FAMILY_SOURCE.into()),
        ));
        let rule = if element.class == "StructuralFoundation" && family == "Foundation Slab" {
            Some(RULE_FOUNDATION_SLAB)
        } else if type_of(element).is_some_and(|id| disagree.contains(&id)) {
            Some(RULE_COPIES_DISAGREE)
        } else {
            None
        };
        if let Some(rule) = rule {
            element.fields.push((
                FAMILY_NAME_RULE_FIELD.into(),
                InstanceField::String(rule.into()),
            ));
        }
    }
}

/// Field carrying the value of the project or shared parameter `Pset_<Door|
/// Slab>Common.IsExternal` an element or its type sets, 0 or 1:
/// Revit's export writes it as `IsExternal` before it looks at the type's
/// Function.
pub const PSET_IS_EXTERNAL_FIELD: &str = "m_pset_is_external";

/// Give each element of `groups` whose type is known its type's Function,
/// the integer entry `FUNCTION_PARAM` in the type's own data object.
/// A type whose entries disagree, or that has none, gives nothing.
/// Where the file defines a parameter `Pset_DoorCommon.IsExternal` or
/// `Pset_SlabCommon.IsExternal`, the element's own value of it, else its
/// type's, goes in [`PSET_IS_EXTERNAL_FIELD`].
pub(super) fn attach_type_functions(
    rf: &mut RevitFile,
    revit_version: u32,
    groups: [&mut Vec<DecodedElement>; 6],
) {
    use crate::partition_room_parameters as prp;
    let type_of = |element: &DecodedElement| {
        element.fields.iter().find_map(|(name, value)| match value {
            InstanceField::ElementId { id, .. } if name == TYPE_ID_FIELD => Some(*id),
            _ => None,
        })
    };
    let types: BTreeSet<u32> = groups
        .iter()
        .flat_map(|elements| elements.iter().filter_map(type_of))
        .collect();
    let functions = prp::scan_integer_parameter(rf, revit_version, &types, prp::FUNCTION_PARAMETER);
    let definitions = prp::find_parameter_definitions(rf, revit_version);
    let is_external = |class: &str| {
        let name = match class {
            "Door" => "Pset_DoorCommon.IsExternal",
            "Floor" | "BuildingPad" => "Pset_SlabCommon.IsExternal",
            _ => return None,
        };
        definitions.get(name).copied()
    };
    // Each class's parameter, read for its elements and their types at once.
    let mut values: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    for definition in groups
        .iter()
        .flat_map(|elements| elements.iter())
        .filter_map(|element| is_external(&element.class))
        .collect::<BTreeSet<u32>>()
    {
        let owners: BTreeSet<u32> = groups
            .iter()
            .flat_map(|elements| elements.iter())
            .filter(|element| is_external(&element.class) == Some(definition))
            .flat_map(|element| element.id.into_iter().chain(type_of(element)))
            .collect();
        for (owner, value) in
            prp::scan_integer_parameter(rf, revit_version, &owners, i64::from(definition))
        {
            values.insert((definition, owner), value);
        }
    }
    for element in groups.into_iter().flat_map(|elements| elements.iter_mut()) {
        if let Some(&function) = type_of(element).and_then(|id| functions.get(&id)) {
            element.fields.push((
                TYPE_FUNCTION_FIELD.into(),
                InstanceField::Integer {
                    value: i64::from(function),
                    signed: false,
                    size: 4,
                },
            ));
        }
        let Some(definition) = is_external(&element.class) else {
            continue;
        };
        let value = element
            .id
            .into_iter()
            .chain(type_of(element))
            .find_map(|owner| values.get(&(definition, owner)))
            .copied();
        if let Some(value) = value {
            element.fields.push((
                PSET_IS_EXTERNAL_FIELD.into(),
                InstanceField::Integer {
                    value: i64::from(value),
                    signed: false,
                    size: 4,
                },
            ));
        }
    }
}

/// Whether `element` has a [`TYPE_FUNCTION_FIELD`].
pub(super) fn has_type_function(element: &DecodedElement) -> bool {
    element
        .fields
        .iter()
        .any(|(name, _)| name == TYPE_FUNCTION_FIELD)
}

/// Give each of `walls` with no [`TYPE_FUNCTION_FIELD`] its type's Function
/// from the type's native record ([`crate::native_wall_types`]).
/// Nothing on a release not measured, or where the native records are not
/// read. `reads` holds what earlier reads of the walls' records found, and
/// takes what this one does.
pub(super) fn attach_wall_type_functions<'a>(
    rf: &mut RevitFile,
    revit_version: u32,
    walls: impl IntoIterator<Item = &'a mut DecodedElement>,
    reads: &mut crate::native_walls::WallReads,
) {
    if !WALL_TYPE_FUNCTION_REVIT_VERSIONS.contains(&revit_version) {
        return;
    }
    let mut walls: Vec<&mut DecodedElement> = walls
        .into_iter()
        .filter(|element| !has_type_function(element))
        .collect();
    let ids: BTreeSet<u64> = walls
        .iter()
        .filter_map(|element| element.id.map(u64::from))
        .collect();
    if ids.is_empty() {
        return;
    }
    let Ok(functions) = reads.type_functions(rf, &ids) else {
        return;
    };
    for element in walls.iter_mut() {
        if let Some(&function) = element.id.and_then(|id| functions.get(&u64::from(id))) {
            element.fields.push((
                TYPE_FUNCTION_FIELD.into(),
                InstanceField::Integer {
                    value: function,
                    signed: true,
                    size: 8,
                },
            ));
        }
    }
}
