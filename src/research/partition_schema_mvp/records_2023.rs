//! The Revit 2023 record path.

use super::*;

/// Revit 2023 elements from their partition element records: each
/// placed instance of a [`crate::partition_element_records::RECOVERED_CATEGORIES`]
/// category, with its box as its body, into the bucket its category
/// feeds. Components nested in doors and windows are left out
/// ([`nested_in_openings`]). Doors and windows bind to their host wall by
/// the 2024 rule on 2023's reference lists, and every element to
/// the one Level its list names. Elements of a non-primary design
/// option are left out unless the convention keeps them, and on 2016 to 2023
/// an element not in the export phase is left out.
pub(super) fn recover_2023_records(
    rf: &mut RevitFile,
    revit_version: u32,
    out: &mut PartitionSchemaMvp,
) -> BTreeMap<u32, Vec<u64>> {
    use crate::partition_element_records as per;
    // The frames that stop are listed only for the log of decisions.
    let want_frames = rf.recording_frame_decisions();
    let (records, mut stages) =
        crate::partition_element_records_2023::scan_records_counted(rf, revit_version, want_frames);
    // Non-primary design options left out under `revit`, as on
    // 2024.
    let records = without_non_primary_options(rf, records);
    let names = type_and_family_names_2023(rf, &records);
    let system_types = system_type_names_2023(rf, revit_version, &records);
    // A curtain wall's grid lines name it, and no other wall has grid
    // lines: Einhoven's curtain wall 7187 has 8 and no mullion, so the
    // mullion rule of [`attach_curtain_walls`] alone misses it. A grid line's
    // record has no volume, so it is read before the selection drops it.
    let gridded: BTreeSet<u32> = records
        .iter()
        .filter(|record| record.builtin_category == OST_CURTAIN_GRIDS_WALL)
        .flat_map(|record| record.references.iter().copied())
        .filter_map(|id| u32::try_from(id).ok())
        .collect();
    let mut selected = select_newest_instance_records(records, &mut stages);
    // Revit's export leaves out an element that does not exist in the export
    // phase, as on 2024, and a room by its own phase.
    // Applied on the releases measured only (2015 to 2023).
    if crate::partition_element_records_2023::applies_phase_filter(revit_version) {
        let phases = rf.memoised_phase_filter();
        selected.retain(|_, record| {
            if phases.excludes(record) {
                stages.counts_mut(&record.stream).not_in_export_phase += 1;
                false
            } else {
                true
            }
        });
    }
    // Only a family instance can be nested; a host wall and the doors it
    // hosts name each other too (the class tag, at the same place on
    // 2023).
    let family_instance = rf.memoised_schema_classes().ok().and_then(|classes| {
        classes
            .classes
            .iter()
            .find(|class| class.name == "FamilyInstance")
            .map(|class| class.tag)
    });
    let nested = match family_instance {
        Some(tag) => nested_in_openings(&selected, tag),
        None => BTreeSet::new(),
    };
    // Each record names its Level in its reference list, as on 2024.
    let level_ids = level_element_ids(rf, revit_version).unwrap_or_default();
    for record in selected.values() {
        if nested.contains(&record.element_id) {
            stages.counts_mut(&record.stream).nested_in_door_or_window += 1;
            continue;
        }
        let Some((_, class)) = per::RECOVERED_CATEGORIES
            .iter()
            .find(|(category, _)| *category == record.builtin_category)
        else {
            stages.counts_mut(&record.stream).category_not_exported += 1;
            continue;
        };
        stages.counts_mut(&record.stream).handed_to_export += 1;
        stages.handed.insert(record.element_id, *class);
        let mut decoded = element_record_decoded(record, class, &level_ids);
        if let Some((type_id, type_name, family_name)) = names.get(&record.element_id) {
            decoded.fields.push((
                TYPE_ID_FIELD.into(),
                InstanceField::ElementId {
                    tag: 0,
                    id: *type_id,
                },
            ));
            decoded.fields.push((
                TYPE_NAME_FIELD.into(),
                InstanceField::String(type_name.clone()),
            ));
            decoded.fields.push((
                FAMILY_NAME_FIELD.into(),
                InstanceField::String(family_name.clone()),
            ));
        } else if let Some((type_id, type_name)) = system_types.get(&record.element_id) {
            decoded.fields.push((
                TYPE_ID_FIELD.into(),
                InstanceField::ElementId {
                    tag: 0,
                    id: *type_id,
                },
            ));
            decoded.fields.push((
                TYPE_NAME_FIELD.into(),
                InstanceField::String(type_name.clone()),
            ));
        }
        match record.builtin_category {
            per::OST_WALLS => out.walls.push(decoded),
            per::OST_DOORS => out.doors.push(decoded),
            per::OST_WINDOWS => out.windows.push(decoded),
            per::OST_FLOORS | per::OST_BUILDING_PAD => out.slabs.push(decoded),
            per::OST_ROOMS | per::OST_AREAS | per::OST_MEP_SPACES => out.rooms.push(decoded),
            per::OST_COLUMNS => out.columns.push(decoded),
            _ => out.products.push(decoded),
        }
    }
    // --- Pipes' and ducts' types, which have no name entry ---
    attach_mep_types_2023(rf, &mut out.products, &selected);
    // What became of each frame, for the diagnostics.
    rf.note_element_record_stages(stages);
    // --- Walls cut back by the walls they join, by 2024's solver: a 2023 record box is the untrimmed wall too, and its
    // reference list names the walls it joins ---
    let wall_records: Vec<per::PartitionElementRecord> = out
        .walls
        .iter()
        .filter_map(|wall| wall.id.and_then(|id| selected.get(&id)).cloned())
        .collect();
    let trims = crate::element_record_wall_joins::join_trims(&wall_records);
    for wall in out.walls.iter_mut() {
        if let Some((record, trim)) = wall
            .id
            .and_then(|id| Some((selected.get(&id)?, trims.get(&id)?)))
        {
            apply_wall_join_trim(wall, record, trim);
        }
    }
    // --- Beams stopped at the faces of the columns their records name
    // ---
    let framing: Vec<per::PartitionElementRecord> = out
        .products
        .iter()
        .filter(|element| element.class == "StructuralFraming")
        .filter_map(|element| element.id.and_then(|id| selected.get(&id)).cloned())
        .collect();
    let columns: Vec<per::PartitionElementRecord> = selected
        .values()
        .filter(|record| {
            matches!(
                record.builtin_category,
                per::OST_COLUMNS | per::OST_STRUCTURAL_COLUMNS
            ) && !nested.contains(&record.element_id)
        })
        .cloned()
        .collect();
    let beam_trims = crate::element_record_beam_cuts::beam_column_trims(&framing, &columns);
    for beam in out.products.iter_mut() {
        if let Some(trim) = beam.id.and_then(|id| beam_trims.get(&id)) {
            apply_beam_column_trim(beam, trim);
        }
    }
    // --- Rooms' numbers and names ---
    let room_ids: BTreeSet<u32> = out.rooms.iter().filter_map(|room| room.id).collect();
    attach_room_parameter_entries(rf, revit_version, &room_ids, &mut out.rooms);
    let wall_ids: BTreeSet<u32> = out.walls.iter().filter_map(|wall| wall.id).collect();
    for element in out.doors.iter_mut().chain(out.windows.iter_mut()) {
        if let Some(record) = element.id.and_then(|id| selected.get(&id)) {
            attach_host_candidates(record, &wall_ids, element);
        }
    }
    // --- Curtain walls and their panels and mullions, from the
    // records' own lists ---
    attach_curtain_walls_by(
        rf,
        revit_version,
        &mut out.walls,
        &mut out.products,
        [&mut out.doors, &mut out.windows],
        gridded,
        |_, element| Some(selected.get(&element.id?)?.references.clone()),
    );
    bind_opening_hosts(&out.walls, &mut out.doors);
    bind_opening_hosts(&out.walls, &mut out.windows);
    // Each record's reference list, as read, for the readers it names.
    selected
        .into_iter()
        .map(|(id, record)| (id, record.references))
        .collect()
}

/// Autodesk `BuiltInCategory.OST_CurtainGridsWall`: a curtain wall's grid
/// lines.
const OST_CURTAIN_GRIDS_WALL: i64 = -2_000_321;

/// Revit 2023: give each pipe and duct without a type the one id in its
/// record's reference list that carries a pipe or duct type name
/// ([`crate::partition_names::find_mep_curve_types_2023`]). The
/// type of a duct takes its system family from the profile class that
/// marks its name: a circular sweep profile is a round duct, a rectangular
/// one a rectangular duct, as [`attach_duct_system_families`] reads it from
/// the type's profile object on later releases. A pipe's family is
/// [`system_family`]'s. An element whose list names no such type, or two,
/// gets nothing.
fn attach_mep_types_2023(
    rf: &mut RevitFile,
    products: &mut [DecodedElement],
    selected: &BTreeMap<u32, crate::partition_element_records::PartitionElementRecord>,
) {
    let is_curve = |element: &DecodedElement| matches!(element.class.as_str(), "Pipe" | "Duct");
    let has_type = |element: &DecodedElement| {
        element
            .fields
            .iter()
            .any(|(name, _)| name == TYPE_NAME_FIELD)
    };
    let mut pending: Vec<(usize, BTreeSet<u32>)> = Vec::new();
    for (index, element) in products.iter().enumerate() {
        if !is_curve(element) || has_type(element) {
            continue;
        }
        let Some((own, record)) = element.id.and_then(|id| Some((id, selected.get(&id)?))) else {
            continue;
        };
        let references: BTreeSet<u32> = record
            .references
            .iter()
            .filter_map(|&id| u32::try_from(id).ok())
            .filter(|&id| id != own)
            .collect();
        pending.push((index, references));
    }
    if pending.is_empty() {
        return;
    }
    let classes = crate::partition_names::mep_curve_type_profile_classes(rf);
    let tags: Vec<u16> = classes.iter().map(|(tag, _)| *tag).collect();
    let wanted: BTreeSet<u32> = pending
        .iter()
        .flat_map(|(_, ids)| ids.iter().copied())
        .collect();
    let mut types: BTreeMap<u32, Option<(String, u16)>> = BTreeMap::new();
    for stream in rf.partition_stream_names() {
        let Ok(inflated) = rf.inflated_partition(&stream) else {
            continue;
        };
        for (id, read) in
            crate::partition_names::find_mep_curve_types_2023(inflated.bytes(), &wanted, &tags)
        {
            match types.get_mut(&id) {
                None => {
                    types.insert(id, Some(read));
                }
                Some(held) => {
                    if held.as_ref() != Some(&read) {
                        *held = None;
                    }
                }
            }
        }
    }
    let locale = rf.basic_file_info().ok().and_then(|info| info.locale);
    for (index, references) in pending {
        let mut typed = references
            .iter()
            .filter_map(|id| Some((*id, types.get(id)?.clone()?)));
        let (Some((type_id, (type_name, tag))), None) = (typed.next(), typed.next()) else {
            continue;
        };
        let element = &mut products[index];
        element.fields.push((
            TYPE_ID_FIELD.into(),
            InstanceField::ElementId {
                tag: 0,
                id: type_id,
            },
        ));
        element
            .fields
            .push((TYPE_NAME_FIELD.into(), InstanceField::String(type_name)));
        if element.class != "Duct" {
            continue;
        }
        let shape = classes
            .iter()
            .find(|(t, _)| *t == tag)
            .and_then(|(_, class)| match class.as_str() {
                "AbsSysCircSweepProfile" => Some(crate::native_duct_shapes::DuctShape::Round),
                "AbsSysRectSweepProfile" => Some(crate::native_duct_shapes::DuctShape::Rectangular),
                _ => None,
            });
        let Some(shape) = shape else {
            continue;
        };
        element.fields.push((
            FAMILY_NAME_FIELD.into(),
            InstanceField::String(
                localized_system_family(shape.system_family(), locale.as_deref()).into(),
            ),
        ));
        element.fields.push((
            FAMILY_NAME_SOURCE_FIELD.into(),
            InstanceField::String(SYSTEM_FAMILY_SOURCE.into()),
        ));
        element.fields.push((
            DUCT_SHAPE_FIELD.into(),
            InstanceField::String(shape.ifc_shape().into()),
        ));
    }
}

/// Each exported 2023 wall's, floor's, roof's and ceiling's type and its
/// name, by ElementId.
///
/// A 2023 system-family type has no record and no name entry. Its
/// name is in its element data, read as on 2024
/// ([`crate::partition_names::find_element_data_names_2023`]). Of the ids
/// an element's record names that have such a name and neither a record
/// nor a name entry, the type is the one whose naming records are of a
/// strict subset of the categories that name each of the others: a wall
/// type is named by walls and the doors and windows they host, a floor
/// type by floors, where the document's other named elements are named by
/// records of more categories. No such one, no type.
pub(super) fn system_type_names_2023(
    rf: &mut RevitFile,
    revit_version: u32,
    records: &[crate::partition_element_records::PartitionElementRecord],
) -> BTreeMap<u32, (u32, String)> {
    let Ok(table) = crate::elem_table::parse_records(rf) else {
        return BTreeMap::new();
    };
    let declared = crate::elem_table::declared_ids(&table);
    let entries = crate::partition_names::name_entries_2023(rf, &declared);
    let recorded: BTreeSet<u32> = records.iter().map(|record| record.element_id).collect();
    let mut named_by: BTreeMap<u32, BTreeSet<i64>> = BTreeMap::new();
    for record in records {
        for &reference in &record.references {
            if let Ok(id) = u32::try_from(reference) {
                if !recorded.contains(&id) && !entries.contains_key(&id) {
                    named_by
                        .entry(id)
                        .or_default()
                        .insert(record.builtin_category);
                }
            }
        }
    }
    let wanted: BTreeSet<u32> = named_by.keys().copied().collect();
    let mut names: BTreeMap<u32, Option<String>> = BTreeMap::new();
    for stream in rf.partition_stream_names() {
        let Ok(inflated) = rf.inflated_partition(&stream) else {
            continue;
        };
        for (id, name) in crate::partition_names::find_element_data_names_32(
            inflated.bytes(),
            revit_version,
            &wanted,
        ) {
            match names.get(&id) {
                None => {
                    names.insert(id, Some(name));
                }
                Some(held) if held.as_deref() != Some(name.as_str()) => {
                    names.insert(id, None);
                }
                _ => {}
            }
        }
    }
    let mut out = BTreeMap::new();
    for record in records.iter().filter(|record| {
        record.is_exported_instance()
            && SYSTEM_TYPE_CATEGORIES_2023.contains(&record.builtin_category)
    }) {
        let candidates: BTreeSet<u32> = record
            .references
            .iter()
            .filter_map(|&reference| u32::try_from(reference).ok())
            .filter(|id| matches!(names.get(id), Some(Some(_))))
            .collect();
        let most_specific = candidates.iter().copied().filter(|id| {
            candidates.iter().all(|other| {
                other == id
                    || (named_by[id].is_subset(&named_by[other]) && named_by[id] != named_by[other])
            })
        });
        let mut picked = most_specific;
        let (Some(type_id), None) = (picked.next(), picked.next()) else {
            continue;
        };
        if let Some(Some(name)) = names.get(&type_id) {
            out.insert(record.element_id, (type_id, name.clone()));
        }
    }
    out
}

pub(super) fn type_and_family_names_2023(
    rf: &mut RevitFile,
    records: &[crate::partition_element_records::PartitionElementRecord],
) -> BTreeMap<u32, (u32, String, String)> {
    use crate::partition_element_records::PLACEMENT_KIND_SYMBOL;
    let Ok(table) = crate::elem_table::parse_records(rf) else {
        return BTreeMap::new();
    };
    let names =
        crate::partition_names::name_entries_2023(rf, &crate::elem_table::declared_ids(&table));
    let recorded: BTreeSet<u32> = records.iter().map(|record| record.element_id).collect();
    let mut symbols: BTreeMap<i64, BTreeSet<u32>> = BTreeMap::new();
    let mut symbol_records: BTreeMap<
        u32,
        &crate::partition_element_records::PartitionElementRecord,
    > = BTreeMap::new();
    for record in records
        .iter()
        .filter(|record| record.placement_kind == PLACEMENT_KIND_SYMBOL)
    {
        symbols
            .entry(record.builtin_category)
            .or_default()
            .insert(record.element_id);
        symbol_records.insert(record.element_id, record);
    }
    let family_of = |type_id: u32| {
        let record = symbol_records.get(&type_id)?;
        let mut families = record.references.iter().filter_map(|&reference| {
            let id = u32::try_from(reference).ok()?;
            (id != type_id && !recorded.contains(&id))
                .then(|| names.get(&id))
                .flatten()
        });
        let family = families.next()?;
        families.next().is_none().then(|| family.clone())
    };
    let mut out = BTreeMap::new();
    for record in records
        .iter()
        .filter(|record| record.is_exported_instance())
    {
        // The type is anywhere in the list: a type made after the instance
        // has the larger id, and another type can sit between them.
        let Some(set) = symbols.get(&record.builtin_category) else {
            continue;
        };
        let mut types = record.references.iter().filter_map(|&reference| {
            let id = u32::try_from(reference).ok()?;
            (set.contains(&id) && names.contains_key(&id)).then_some(id)
        });
        let (Some(type_id), None) = (types.next(), types.next()) else {
            continue;
        };
        let (Some(type_name), Some(family_name)) = (names.get(&type_id), family_of(type_id)) else {
            continue;
        };
        out.insert(record.element_id, (type_id, type_name.clone(), family_name));
    }
    // a curtain mullion's or panel's type has no partition record
    // of its own (a system type), so `symbols` misses it. It is the one named
    // id in the instance's list whose element data names a named family.
    let curtain = |record: &crate::partition_element_records::PartitionElementRecord| {
        matches!(
            record.builtin_category,
            crate::partition_element_records::OST_CURTAIN_WALL_MULLIONS
                | crate::partition_element_records::OST_CURTAIN_WALL_PANELS
        )
    };
    let pending: Vec<(u32, BTreeSet<u32>)> = records
        .iter()
        .filter(|record| record.is_exported_instance() && curtain(record))
        .filter(|record| !out.contains_key(&record.element_id))
        .map(|record| {
            let named = record
                .references
                .iter()
                .filter_map(|&reference| u32::try_from(reference).ok())
                .filter(|id| *id != record.element_id && !recorded.contains(id))
                .filter(|id| names.contains_key(id))
                .collect();
            (record.element_id, named)
        })
        .collect();
    let wanted: BTreeSet<u32> = pending
        .iter()
        .flat_map(|(_, ids)| ids.iter().copied())
        .collect();
    if wanted.is_empty() {
        return out;
    }
    let mut families: BTreeMap<u32, Option<u32>> = BTreeMap::new();
    for stream in rf.partition_stream_names() {
        let Ok(inflated) = rf.inflated_partition(&stream) else {
            continue;
        };
        for (id, family) in
            crate::partition_names::find_curtain_type_families_2023(inflated.bytes(), &wanted)
        {
            match families.get(&id) {
                None => {
                    families.insert(id, family);
                }
                Some(held) if *held != family => {
                    families.insert(id, None);
                }
                _ => {}
            }
        }
    }
    for (element, ids) in pending {
        let mut typed = ids.iter().filter_map(|id| {
            let family = families.get(id).copied().flatten()?;
            Some((*id, names.get(id)?, names.get(&family)?))
        });
        let (Some((type_id, type_name, family_name)), None) = (typed.next(), typed.next()) else {
            continue;
        };
        out.insert(element, (type_id, type_name.clone(), family_name.clone()));
    }
    out
}

/// [`select_instance_records`] for Revit 2023: of an element's
/// frames, the newest, in the highest-numbered partition and then the
/// latest in it. A 2023 project's frames can disagree about their height,
/// where a Level moved after an earlier save: on modelo_bim four columns
/// run from -13.12 ft in `Partitions/1` and from -5.58 ft in
/// `Partitions/2`, and Revit's export draws them from -1.7 m (-5.58 ft),
/// on the Level the newer frame names. The greatest extent would keep the
/// stale one.
///
/// Counts what became of each frame in `stages`: a record that is not a
/// placed instance, a placed instance with or without volume, a frame another
/// replaces, a nested component and the frame kept.
pub(super) fn select_newest_instance_records(
    records: Vec<crate::partition_element_records::PartitionElementRecord>,
    stages: &mut crate::partition_element_records_2023::RecordStages,
) -> BTreeMap<u32, crate::partition_element_records::PartitionElementRecord> {
    use crate::partition_element_records::PartitionElementRecord;
    let partition = |record: &PartitionElementRecord| {
        record
            .stream
            .rsplit('/')
            .next()
            .and_then(|n| n.parse::<u32>().ok())
            .unwrap_or(0)
    };
    let mut by_id: BTreeMap<u32, PartitionElementRecord> = BTreeMap::new();
    for record in records {
        let counts = stages.counts_mut(&record.stream);
        if !record.is_exported_instance() {
            counts.not_placed_instances += 1;
            continue;
        }
        counts.placed_instances += 1;
        if !record.has_volume() {
            counts.without_volume += 1;
            stages.note_volumeless(&record);
            continue;
        }
        counts.with_volume += 1;
        let newer = by_id.get(&record.element_id).is_none_or(|held| {
            (partition(&record), record.offset) > (partition(held), held.offset)
        });
        if newer {
            if let Some(replaced) = by_id.insert(record.element_id, record) {
                stages.counts_mut(&replaced.stream).superseded += 1;
            }
        } else {
            stages.counts_mut(&record.stream).superseded += 1;
        }
    }
    for id in nested_component_ids(&by_id) {
        if let Some(record) = by_id.remove(&id) {
            stages.counts_mut(&record.stream).nested_components += 1;
        }
    }
    for record in by_id.values() {
        stages.counts_mut(&record.stream).selected += 1;
    }
    by_id
}
