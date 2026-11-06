//! Revit's built-in parameters and categories by id: the name
//! Revit shows for a built-in parameter's negative id, its storage type,
//! spec, parameter group and enumeration texts, and each `BuiltInCategory`'s
//! enum name and the name Revit shows.
//!
//! Every entry comes from a public source or our own Revit API dump, as the
//! specs and enumeration texts from `Parameter.Definition.Name`,
//! `StorageType` and `AsValueString` dumped from 102 public sample models
//! writes each parameter in; ids the dump never saw named by the label in the
//! Revit 2027 API reference's `BuiltInParameter` page; enum names from the
//! same reference. Labels are English, as Revit 2018-2027 shows them.

mod table;

/// What is known of one built-in parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltInParameter {
    pub id: i64,
    /// The name Revit shows (`LabelUtils`), e.g. `Unconnected Height`.
    pub name: &'static str,
    /// The `BuiltInParameter` enum name, e.g. `WALL_USER_HEIGHT_PARAM`;
    /// empty where the API reference lists none.
    pub enum_name: &'static str,
    /// `String`, `Integer`, `Double` or `ElementId`; empty where unseen.
    pub storage_type: &'static str,
    /// The spec type id the display was measured as (length, area, Yes/No,
    /// ...); empty where not known or not a measure.
    pub spec_type_id: &'static str,
    /// The parameter group's label, as Revit's own IFC export names the set
    /// (`Constraints`, `Identity Data`); empty where never exported.
    pub group: &'static str,
    /// `true` where the entry comes from our API dump, `false` where only
    /// from the API reference.
    pub dumped: bool,
}

/// The built-in parameter `id`, `None` for one not known.
pub fn parameter(id: i64) -> Option<BuiltInParameter> {
    let at = table::PARAMETERS
        .binary_search_by_key(&id, |row| row.0)
        .ok()?;
    let (id, name, enum_name, storage_type, spec_type_id, group, source) = table::PARAMETERS[at];
    Some(BuiltInParameter {
        id,
        name,
        enum_name,
        storage_type,
        spec_type_id,
        group,
        dumped: source == 'D',
    })
}

/// The text Revit shows for the integer `value` of the enumeration
/// parameter `id`, `None` where not known.
pub fn enum_text(id: i64, value: i64) -> Option<&'static str> {
    table::ENUM_TEXTS
        .binary_search_by_key(&(id, value), |row| (row.0, row.1))
        .ok()
        .map(|at| table::ENUM_TEXTS[at].2)
}

/// Whether `id`'s integers are an enumeration with known texts.
pub fn is_enumeration(id: i64) -> bool {
    let at = table::ENUM_TEXTS.partition_point(|row| row.0 < id);
    table::ENUM_TEXTS.get(at).is_some_and(|row| row.0 == id)
}

/// The `BuiltInCategory` `id`'s enum name (`OST_Walls`) and the name Revit
/// shows (`Walls`; empty where the dump never saw the category).
pub fn category(id: i64) -> Option<(&'static str, &'static str)> {
    table::CATEGORIES
        .binary_search_by_key(&id, |row| row.0)
        .ok()
        .map(|at| (table::CATEGORIES[at].1, table::CATEGORIES[at].2))
}

/// The name Revit shows for category `id`: the dumped name, else the enum
/// name without `OST_`.
pub fn category_name(id: i64) -> Option<&'static str> {
    let (enum_name, name) = category(id)?;
    Some(if name.is_empty() {
        enum_name.strip_prefix("OST_").unwrap_or(enum_name)
    } else {
        name
    })
}

/// Revit's `Category.Name` for category `id` and its parent's name, where it
/// has one: the dumped `Stairs: Runs` is `Runs`, under `Stairs`.
pub fn category_name_and_parent(id: i64) -> Option<(&'static str, Option<&'static str>)> {
    let full = category_name(id)?;
    Some(match full.split_once(": ") {
        Some((parent, name)) => (name, Some(parent)),
        None => (full, None),
    })
}

/// Every known category id, for scans that look for any of them.
pub fn category_ids() -> impl Iterator<Item = i64> {
    table::CATEGORIES.iter().map(|row| row.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_are_sorted_for_lookup() {
        assert!(table::PARAMETERS.windows(2).all(|w| w[0].0 < w[1].0));
        assert!(
            table::ENUM_TEXTS
                .windows(2)
                .all(|w| (w[0].0, w[0].1) < (w[1].0, w[1].1))
        );
        assert!(table::CATEGORIES.windows(2).all(|w| w[0].0 < w[1].0));
    }

    #[test]
    fn known_parameters_and_categories() {
        let height = parameter(-1_001_105).expect("Unconnected Height");
        assert_eq!(height.name, "Unconnected Height");
        assert_eq!(height.enum_name, "WALL_USER_HEIGHT_PARAM");
        assert_eq!(height.storage_type, "Double");
        assert!(height.spec_type_id.starts_with("autodesk.spec.aec:length-"));
        assert_eq!(height.group, "Constraints");
        assert_eq!(parameter(-1_140_422).map(|p| p.name), Some("Keynote"));
        assert_eq!(parameter(1), None);
        assert_eq!(category(-2_000_011), Some(("OST_Walls", "Walls")));
        assert_eq!(category_name(-2_000_080), Some("Furniture"));
        assert_eq!(
            category_name_and_parent(-2_000_919),
            Some(("Runs", Some("Stairs")))
        );
        assert_eq!(
            category_name_and_parent(-2_000_080),
            Some(("Furniture", None))
        );
    }
}
