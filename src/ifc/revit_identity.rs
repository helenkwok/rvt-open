//! Each element's Revit identity as properties: its ElementId,
//! the name Revit shows for its category (`Walls`, `Structural Framing`; a
//! subcategory's short name, `Runs`, with `ParentCategory` `Stairs`) and
//! the category's `BuiltInCategory` enum name (`OST_Walls`), in a set
//! [`SET`] on the element, and the same on its type object. IFC classes are
//! unchanged: the category is what a viewer names sheets and filters by,
//! as APS and Revit's schedules do.
//!
//! The category is the one each element's partition record carries (its
//! header's `BuiltInCategory`), read for every category of
//! [`crate::builtin_parameters`]: Revit 2018 to 2023 by
//! `partition_element_records_2023::scan_records`, later releases
//! by `partition_element_records::scan_category_records_multi`. An
//! element with no record found gets its ElementId alone.

use super::IfcModel;
use super::entities::{IfcEntity, Property, PropertySet, PropertyValue};
use crate::RevitFile;
use std::collections::BTreeMap;

/// The set's name.
pub const SET: &str = "Revit";

/// The categories of elements that only cut openings: Shaft Openings and the
/// rectangular wall, structural framing and roof opening cuts.
const OPENING_CATEGORIES: [i64; 5] = [-2_000_996, -2_000_997, -2_000_999, -2_000_995, -2_000_897];

/// What [`attach`] wrote.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Report {
    /// Elements given the set.
    pub elements: usize,
    /// Of them, those with a category.
    pub with_category: usize,
    /// Type objects' types given the set.
    pub types: usize,
}

/// Each element's `BuiltInCategory` id, by ElementId, from its partition
/// records; empty on a release whose records are not read.
pub fn element_categories(file: &mut RevitFile) -> BTreeMap<u32, i64> {
    let Ok(info) = file.basic_file_info() else {
        return BTreeMap::new();
    };
    let version = info.version;
    let categories: Vec<i64> = crate::builtin_parameters::category_ids().collect();
    let records = if crate::partition_element_records_2023::is_u32_record_release(version) {
        crate::partition_element_records_2023::scan_records(file, version)
    } else if crate::partition_element_records::supports_revit_version(version) {
        let declared = crate::elem_table::parse_records(file)
            .map(|records| crate::elem_table::declared_ids(&records))
            .unwrap_or_default();
        crate::partition_element_records::scan_category_records_multi(
            file,
            version,
            &categories,
            &declared,
        )
        .unwrap_or_default()
    } else {
        Vec::new()
    };
    let mut by_id = BTreeMap::new();
    for record in records {
        if crate::builtin_parameters::category(record.builtin_category).is_some() {
            by_id
                .entry(record.element_id)
                .or_insert(record.builtin_category);
        }
    }
    by_id
}

/// The set for `id` in `category`.
fn identity(id: u32, category: Option<i64>) -> PropertySet {
    let mut properties = vec![Property {
        name: "ElementId".into(),
        value: PropertyValue::Integer(i64::from(id)),
    }];
    if let Some(category) = category
        && let Some((name, parent)) = crate::builtin_parameters::category_name_and_parent(category)
        && let Some((enum_name, _)) = crate::builtin_parameters::category(category)
    {
        // Revit's short `Category.Name` (`Runs`), which a viewer
        // names its sheet after, and the parent category (`Stairs`).
        properties.push(Property {
            name: "Category".into(),
            value: PropertyValue::Label(name.into()),
        });
        if let Some(parent) = parent {
            properties.push(Property {
                name: "ParentCategory".into(),
                value: PropertyValue::Label(parent.into()),
            });
        }
        properties.push(Property {
            name: "BuiltInCategory".into(),
            value: PropertyValue::Label(enum_name.into()),
        });
    }
    PropertySet {
        name: SET.into(),
        properties,
    }
}

/// Give every building element of `model` whose name tag is its ElementId
/// the set [`SET`], and each of their types' type objects one.
pub fn attach(file: &mut RevitFile, model: &mut IfcModel) -> Report {
    let categories = element_categories(file);
    attach_with(&categories, model)
}

/// [`attach`] with the categories already read.
pub fn attach_with(categories: &BTreeMap<u32, i64>, model: &mut IfcModel) -> Report {
    let mut report = Report::default();
    let mut added = Vec::new();
    for (index, entity) in model.entities.iter().enumerate() {
        let IfcEntity::BuildingElement {
            type_guid: Some(tag),
            property_set,
            ..
        } = entity
        else {
            continue;
        };
        let Ok(id) = tag.parse::<u32>() else {
            continue;
        };
        // An Area's record carries no box, so the record scan above does not
        // find it; the area reader names it an Area.
        let area = property_set.as_ref().is_some_and(|set| {
            set.properties.iter().any(|p| {
                p.name == super::export_content::SPATIAL_KIND_PROPERTY
                    && matches!(&p.value, PropertyValue::Text(t) if t == "Area")
            })
        });
        let category = categories
            .get(&id)
            .copied()
            .or_else(|| area.then_some(crate::partition_area_records::OST_AREAS));
        let set = identity(id, category);
        report.elements += 1;
        report.with_category += usize::from(set.properties.len() > 1);
        added.push(IfcEntity::ElementPropertySet {
            element: index,
            set,
        });
    }
    model.entities.extend(added);
    // The elements that cut openings and are no building element themselves,
    // each listed in the element table under its category.
    for entity in &model.entities {
        let tag = match entity {
            IfcEntity::VoidOpening { tag, .. }
            | IfcEntity::WallProfileOpening { tag, .. }
            | IfcEntity::WallCutOpening { tag, .. }
            | IfcEntity::HostedCutOpening { tag, .. } => tag,
            _ => continue,
        };
        if let Ok(id) = tag.parse::<u32>()
            && let Some(category) = categories.get(&id)
            && OPENING_CATEGORIES.contains(category)
            && let Some((name, _)) = crate::builtin_parameters::category_name_and_parent(*category)
        {
            model.opening_categories.insert(id, name.to_string());
        }
    }
    let types: std::collections::BTreeSet<u32> = model.element_type_ids.values().copied().collect();
    for type_id in types {
        let sets = model.type_property_sets.entry(type_id).or_default();
        if sets.iter().any(|set| set.name == SET) {
            continue;
        }
        sets.push(identity(type_id, categories.get(&type_id).copied()));
        report.types += 1;
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_names_the_category_both_ways() {
        let set = identity(42, Some(-2_000_011));
        let text: Vec<String> = set
            .properties
            .iter()
            .map(|p| format!("{}={:?}", p.name, p.value))
            .collect();
        assert_eq!(set.name, "Revit");
        assert!(text[0].starts_with("ElementId=Integer(42)"), "{text:?}");
        assert!(text[1].contains("Walls"), "{text:?}");
        assert!(text[2].contains("OST_Walls"), "{text:?}");
        assert_eq!(identity(7, None).properties.len(), 1);
        let runs: Vec<String> = identity(9, Some(-2_000_919))
            .properties
            .iter()
            .map(|p| format!("{}={:?}", p.name, p.value))
            .collect();
        assert_eq!(runs[1], "Category=Label(\"Runs\")", "{runs:?}");
        assert_eq!(runs[2], "ParentCategory=Label(\"Stairs\")", "{runs:?}");
        assert!(runs[3].contains("OST_StairsRuns"), "{runs:?}");
    }
}
