//! Revit's built-in parameters and categories by id: the name
//! Revit shows for a built-in parameter's negative id, its storage type,
//! spec, parameter group and enumeration texts, its name in the other 12
//! languages of Revit's help, and each `BuiltInCategory`'s enum name and
//! the name Revit shows.
//!
//! Every entry comes from a public source, and each field says which: the
//! names and enum names from the Revit 2027 API reference's
//! `BuiltInParameter` page; the storage type from the values public sample
//! files store; the spec, group and enumeration texts from Revit's own IFC
//! export of public samples; the names in the other languages from the
//! same Revit 2027 help topic in each language (machine-matched, cited per
//! entry). A field no public source gives is empty. The Revit API dump of
//! the public samples is not a source: it is the answer key
//! `tools/builtin-parameters/score.py` scores the table against.

mod table;

/// What is known of one built-in parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltInParameter {
    pub id: i64,
    /// The name Revit shows, e.g. `Unconnected Height`: the label of the
    /// Revit 2027 API reference; empty where the reference has none.
    pub name: &'static str,
    /// The `BuiltInParameter` enum name, e.g. `WALL_USER_HEIGHT_PARAM`;
    /// empty where the API reference lists none.
    pub enum_name: &'static str,
    /// `String`, `Integer`, `Double` or `ElementId`, the type of the values
    /// public files store for this id; empty where none does.
    pub storage_type: &'static str,
    /// The spec type id from the value type of the property of the same
    /// name in Revit's IFC export (length, area, Yes/No, ...); empty where
    /// it is not exported or not a measure.
    pub spec_type_id: &'static str,
    /// The parameter group's label, as Revit's own IFC export names the set
    /// (`Constraints`, `Identity Data`); empty where never exported.
    pub group: &'static str,
    /// One code per field, in the order name, enum name, storage type,
    /// spec, group, localized names: `R` the API reference, `F` stored
    /// values of public files, `E` Revit's IFC export, `H` machine-matched
    /// help terms, `-` no public source (the field is empty).
    pub sources: &'static str,
}

/// The languages of [`localized_name`], in the order of the table.
pub const LOCALIZED_LANGUAGES: [&str; 12] = [
    "CHT", "CHS", "CSY", "DEU", "ESP", "FRA", "ITA", "JPN", "KOR", "PLK", "PTB", "RUS",
];

/// The built-in parameter `id`, `None` for one not known.
pub fn parameter(id: i64) -> Option<BuiltInParameter> {
    let at = table::PARAMETERS
        .binary_search_by_key(&id, |row| row.0)
        .ok()?;
    let (id, name, enum_name, storage_type, spec_type_id, group, sources) = table::PARAMETERS[at];
    Some(BuiltInParameter {
        id,
        name,
        enum_name,
        storage_type,
        spec_type_id,
        group,
        sources,
    })
}

/// The name of built-in parameter `id` in `language` (a Revit locale code
/// from [`LOCALIZED_LANGUAGES`]) and the help topic (`<component>/<topic>`,
/// help.autodesk.com Revit 2027) the term was taken from. Only parameters
/// whose term is the same on every help topic that carries their name, in
/// all 12 languages, have one. The match is by machine: the term at the same
/// place of the same topic as the English name.
pub fn localized_name(id: i64, language: &str) -> Option<(&'static str, &'static str)> {
    let col = LOCALIZED_LANGUAGES
        .iter()
        .position(|l| l.eq_ignore_ascii_case(language))?;
    let at = table::LOCALIZED
        .binary_search_by_key(&id, |row| row.0)
        .ok()?;
    let (term, topic) = table::LOCALIZED[at].1[col];
    (!term.is_empty()).then_some((term, topic))
}

/// The built-in parameter whose `BuiltInParameter` enum name is
/// `enum_name`, `None` for one not known.
pub fn parameter_by_enum(enum_name: &str) -> Option<BuiltInParameter> {
    table::PARAMETERS
        .iter()
        .find(|row| row.2 == enum_name)
        .map(|row| row.0)
        .or_else(|| ALIASES.iter().find(|a| a.0 == enum_name).map(|a| a.1))
        .and_then(parameter)
}

/// `BuiltInParameter` members that name the same id as the member the
/// catalog lists: older names the Revit 2026 API renamed (`OMNICLASS_*` to
/// `CLASSIFICATION_*`, `UNIFORMAT_*` to `ASSEMBLY_*`, the analytical heat
/// transfer coefficient to thermal transmittance), and synonyms the API
/// keeps beside them (`SYMBOL_NAME_PARAM` beside `ALL_MODEL_TYPE_NAME`).
/// Each was checked on the tiny model 1-hip-roof (Revit 2024; `DOOR_HEIGHT`
/// and `GENERIC_WIDTH`, the names the Revit 2027 API gives two of the same
/// ids, on its Revit 2027 copy): the Revit API reports the parameter under
/// this name with the catalog id's name and storage type, and where the
/// file stores a value under that id (36 values), it is the API's.
///
/// Every id is as the public Revit API reference gives it, page
/// `BuiltInParameter Enumeration` (help.autodesk.com, `cloudhelp/<year>/ENU/
/// Revit-API-MainReference/files/html/fb011c91-be7e-f737-28c7-3f1e1917a0e0.htm`),
/// which lists each member with its value: the 2027 page for the names it
/// still lists, the 2025 page for the names the 2026 API renamed. The two
/// misspelled `BENDING_DETAIL_*` names are in the Revit 2024 reference (same
/// page id, no values) with the labels `Representation` and `View for 3D
/// Shape`, which the 2025 page gives `BENDING_DETAIL_SEGMENT_REPRESENTATION`
/// (-1,155,310) and `BENDING_DETAIL_REPRESENTATION_FOR_3D_BARS` (-1,155,308).
pub const ALIASES: &[(&str, i64)] = &[
    ("SYMBOL_NAME_PARAM", -1002001), // 2027 page: -1,002,001 Type Name
    ("SYMBOL_FAMILY_NAME_PARAM", -1002002), // 2027 page: -1,002,002 Family Name
    ("UNIFORMAT_CODE", -1002500),    // 2025 page: -1,002,500
    ("UNIFORMAT_DESCRIPTION", -1002501), // 2025 page: -1,002,501
    ("OMNICLASS_CODE", -1002502),    // 2025 page: -1,002,502
    ("OMNICLASS_DESCRIPTION", -1002503), // 2025 page: -1,002,503
    ("ANALYTICAL_HEAT_TRANSFER_COEFFICIENT", -1005430), // 2025 and 2026 pages: -1,005,430
    ("WINDOW_TYPE_ID", -1001405),    // 2027 page: -1,001,405 Type Mark
    ("GENERIC_HEIGHT", -1001300),    // 2027 page: -1,001,300 Height
    ("DOOR_WIDTH", -1001301),        // 2027 page: -1,001,301 Width
    ("DOOR_HEIGHT", -1001300),       // 2027 page: -1,001,300 Height
    ("GENERIC_WIDTH", -1001301),     // 2027 page: -1,001,301 Width
    ("WINDOW_THICKNESS", -1001302),  // 2027 page: -1,001,302 Thickness
    ("BENDING_DETAIL_SEGMENET_REPESENTATION", -1155310), // 2025 page: BENDING_DETAIL_SEGMENT_REPRESENTATION
    ("BENDING_DETAIL_REPESENTATION_FOR_3D_BARS", -1155308), // 2025 page: BENDING_DETAIL_REPRESENTATION_FOR_3D_BARS
];

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
/// shows (`Walls`: the `Category` property of Revit's IFC export of public
/// samples, else the term of Revit's public help, see
/// [`crate::program_names`]; empty where neither gives one).
pub fn category(id: i64) -> Option<(&'static str, &'static str)> {
    table::CATEGORIES
        .binary_search_by_key(&id, |row| row.0)
        .ok()
        .map(|at| {
            let (_, enum_name, name) = table::CATEGORIES[at];
            (
                enum_name,
                if name.is_empty() {
                    crate::program_names::category_name(id, "ENU").unwrap_or("")
                } else {
                    name
                },
            )
        })
}

/// The name Revit shows for category `id`: the help's name, else the enum
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
/// has one: `Stairs: Runs` is `Runs`, under `Stairs`.
pub fn category_name_and_parent(id: i64) -> Option<(&'static str, Option<&'static str>)> {
    let full = category_name(id)?;
    Some(match full.split_once(": ") {
        Some((parent, name)) => (name, Some(parent)),
        None => (full, None),
    })
}

/// The name Revit shows for category `id` without its parent: a
/// subcategory's own name (`Glass` for the windows' glass), as Revit names
/// the category's graphics styles; `None` for a category whose display name
/// the catalog does not hold. Revit does not store a built-in category's
/// name in the file, so a subcategory the catalog's public sources never
/// showed has no name here (the enum name, `OST_DoorsGlassProjection`, is
/// not the name Revit shows, so it is not offered as one).
pub fn category_own_name(id: i64) -> Option<&'static str> {
    let (_, full) = category(id)?;
    (!full.is_empty()).then(|| full.rsplit_once(": ").map_or(full, |(_, name)| name))
}

/// Every known built-in parameter id, in ascending order.
pub fn parameter_ids() -> impl Iterator<Item = i64> {
    table::PARAMETERS.iter().map(|row| row.0)
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
        for (name, id) in ALIASES {
            assert_eq!(parameter_by_enum(name).map(|p| p.id), Some(*id), "{name}");
        }
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
        assert_eq!(height.sources, "RRFEE-");
        assert_eq!(parameter(1), None);
        // No public source names this id: kept, with an empty name.
        assert_eq!(parameter(-1_005_051).map(|p| p.name), Some(""));
        assert_eq!(
            parameter_by_enum("WALL_USER_HEIGHT_PARAM").map(|p| p.id),
            Some(-1_001_105)
        );
        assert_eq!(category(-2_000_011), Some(("OST_Walls", "Walls")));
        assert_eq!(category_own_name(-2_000_011), Some("Walls"));
        assert_eq!(category_own_name(-2_000_919), Some("Runs"));
        // No public name: not the enum name, and not a name from a model's API dump.
        assert_eq!(category_own_name(-2_000_031), None);
        assert_eq!(category_name(-2_000_080), Some("Furniture"));
        assert_eq!(
            category_name_and_parent(-2_000_919).map(|c| c.1),
            Some(None)
        );
        assert_eq!(
            category_name_and_parent(-2_000_080),
            Some(("Furniture", None))
        );
    }
}
