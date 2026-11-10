//! Every element the file saves parameters for, read for IFCX.
//!
//! An IFC4 export holds the building elements, their types and storeys.
//! The Revit file holds much more: phases, design options, views, levels,
//! materials, project information, types no element uses, and every
//! parameter of each. [`read`] reads them all, and
//! [`super::graph::IfcxGraph::add_revit_elements`] puts them in the IFCX
//! file: an element the graph already has gets the parameters it lacks,
//! any other a node of its own under one `RevitElements` node.
//!
//! Each element keeps its own parameters (not its type's: those are on the
//! type's node), grouped and valued as Revit's property sets are
//! ([`super::revit_property_sets`]), under `rvtopen::revit::<Group>::<Name>`,
//! except that an enumeration whose text is not known keeps its integer. A
//! phase also gets its name and its place in the project's order (`Sequence
//! Number`), and a design option its name and its set (`Design Option Set
//! Id`), which the file keeps outside its parameters.
//! A type no element names as its type is marked
//! [`RevitElement::unused_type`]: the IFCX file holds it only on request.
//! Its node's path is the GlobalId Revit's IFC exporter gives it (as the
//! research module `revit_global_ids` rebuilds it), so an element an IFC
//! export holds has the same id in both.

use super::entities::{Property, PropertyValue};
use super::revit_property_sets::grouped;
use crate::RevitFile;
use crate::research::native_element_parameters::ParameterLevel;
use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet};

/// One element and its own parameters.
#[derive(Debug, Clone)]
pub struct RevitElement {
    pub id: u64,
    /// The IFC GlobalId Revit's exporter gives it.
    pub global_id: String,
    /// The class of its saved record (`Level`, `MaterialElem`, ...).
    pub class: String,
    /// Its name entry, where it has one.
    pub name: Option<String>,
    /// Its parameters by group label.
    pub sets: BTreeMap<&'static str, Vec<Property>>,
    /// A type (its record's class derives from
    /// [`crate::file_types::TYPE_BASE_CLASS`]) that no element names: no
    /// element read takes a type value from it or names it in a type field
    /// of its record, and no exported element is typed by it.
    pub unused_type: bool,
}

/// Every element with a GlobalId and at least one parameter Revit's property
/// sets would write. Empty for a file whose parameters are not read (before
/// Revit 2015) or whose ElemTable layout gives no GlobalIds. `named_types`
/// are types the caller knows an element names (the exported model's
/// element types): they are not unused.
pub fn read(file: &mut RevitFile, named_types: &BTreeSet<u64>) -> Result<Vec<RevitElement>> {
    let global_ids = crate::revit_global_ids::revit_global_ids(file)?;
    let ids: BTreeSet<u64> = global_ids.keys().map(|&id| u64::from(id)).collect();
    let saved = crate::research::native_element_parameters::read(file, &ids)?;
    let names = file.memoised_element_names();
    let type_classes: BTreeSet<String> = file
        .schema_layouts()
        .map(|layouts| {
            layouts
                .classes
                .keys()
                .filter_map(|&tag| {
                    let chain = layouts.chain(tag)?;
                    chain
                        .iter()
                        .any(|c| c.name == crate::file_types::TYPE_BASE_CLASS)
                        .then(|| chain.last().map(|c| c.name.clone()))
                        .flatten()
                })
                .collect()
        })
        .unwrap_or_default();
    // The types an element takes a type value from, or that a type field of
    // an element's record (not a type's) names: a railing's or stair's type.
    let mut used: BTreeSet<i64> = saved
        .elements
        .iter()
        .flat_map(|(&id, parameters)| {
            parameters
                .iter()
                .filter(move |p| {
                    p.level == ParameterLevel::Type && i64::try_from(id) != Ok(p.source_element_id)
                })
                .map(|p| p.source_element_id)
        })
        .collect();
    for (class, named) in saved.named_ids.values() {
        if !type_classes.contains(class) {
            used.extend(named.iter().copied());
        }
    }
    let mut out = Vec::new();
    for (&id, parameters) in &saved.elements {
        let (Ok(id32), Ok(own_id)) = (u32::try_from(id), i64::try_from(id)) else {
            continue;
        };
        let Some(global_id) = global_ids.get(&id32) else {
            continue;
        };
        let own: Vec<_> = parameters
            .iter()
            .filter(|p| p.name.is_some() && p.source_element_id == own_id)
            .collect();
        let Some(class) = own.first().map(|p| p.source_class.clone()) else {
            continue;
        };
        let (sets, _) = grouped(own.into_iter(), true);
        if sets.is_empty() {
            continue;
        }
        let unused_type =
            type_classes.contains(&class) && !used.contains(&own_id) && !named_types.contains(&id);
        out.push(RevitElement {
            id,
            global_id: global_id.clone(),
            class,
            name: names.entries.get(&id32).map(|e| e.name.clone()),
            sets,
            unused_type,
        });
    }
    let phases = file.memoised_phase_filter();
    for (at, &phase) in phases.order.iter().enumerate() {
        let number = i64::try_from(at + 1).unwrap_or(i64::MAX);
        let property = (
            PHASE_SEQUENCE_NUMBER,
            "Sequence Number",
            PropertyValue::Integer(number),
        );
        add(
            &mut out,
            &global_ids,
            phase,
            "Phase",
            phases.names.get(&phase),
            property,
        );
    }
    let options = file.memoised_design_options();
    for set in options.sets.values() {
        for &option in &set.options {
            let property = (
                OPTION_SET_ID,
                "Design Option Set Id",
                PropertyValue::Label(set.set_id.to_string()),
            );
            let name = set.option_names.get(&option);
            add(
                &mut out,
                &global_ids,
                i64::from(option),
                "DesignOption",
                name,
                property,
            );
        }
    }
    out.sort_by_key(|e| e.id);
    Ok(out)
}

/// `PHASE_SEQUENCE_NUMBER` and `OPTION_SET_ID`.
const PHASE_SEQUENCE_NUMBER: i64 = -1_012_111;
const OPTION_SET_ID: i64 = -1_133_603;

/// Give element `id` `property` (a built-in's id, name and value) in the
/// set Revit groups that built-in in, and its name where it has none; the
/// element is added where `out` lacks it.
fn add(
    out: &mut Vec<RevitElement>,
    global_ids: &BTreeMap<u32, String>,
    id: i64,
    class: &str,
    name: Option<&String>,
    (builtin, label, value): (i64, &str, PropertyValue),
) {
    let (Ok(id), Some(global_id)) = (
        u64::try_from(id),
        u32::try_from(id).ok().and_then(|i| global_ids.get(&i)),
    ) else {
        return;
    };
    let at = match out.iter().position(|e| e.id == id) {
        Some(at) => at,
        None => {
            out.push(RevitElement {
                id,
                global_id: global_id.clone(),
                class: class.to_string(),
                name: None,
                sets: BTreeMap::new(),
                unused_type: false,
            });
            out.len() - 1
        }
    };
    let e = &mut out[at];
    if e.name.is_none() {
        e.name = name.cloned();
    }
    let group = crate::builtin_parameters::parameter(builtin)
        .map(|b| b.group)
        .filter(|g| !g.is_empty())
        .unwrap_or("Other");
    let set = e.sets.entry(group).or_default();
    if !set.iter().any(|p| p.name == label) {
        set.push(Property {
            name: label.to_string(),
            value,
        });
    }
}
