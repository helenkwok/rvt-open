//! Names Revit keeps in its program, not in the file: the names of built-in
//! categories and subcategories (what a graphics style of a built-in
//! subcategory shows) and of system families (`Basic Wall`, `Text`), in the
//! 13 languages Revit's help offers.
//!
//! Every name is a term from Revit 2027's public help
//! (`help.autodesk.com/cloudhelp/2027/<LANG>/...`), the same page in each
//! language; each entry cites the page it came from. Names only, no help
//! text. A built-in category is tied to its help term by its
//! `BuiltInCategory` enum name (`OST_StairsRiserLines` -> `Riser Lines`); a
//! system family to the file class of its types.

mod table;

/// The language codes of Revit's help, in the order of each entry's names.
pub const LANGUAGES: [&str; 13] = [
    "ENU", "CHT", "CHS", "CSY", "DEU", "ESP", "FRA", "ITA", "JPN", "KOR", "PLK", "PTB", "RUS",
];

/// The index of `language` (a Revit locale code such as `ENU` or `DEU`, as
/// `BasicFileInfo` stores "Locale when saved"); English for any other.
fn index(language: &str) -> usize {
    LANGUAGES
        .iter()
        .position(|l| l.eq_ignore_ascii_case(language))
        .unwrap_or(0)
}

fn pick(names: &[&'static str; 13], language: &str) -> Option<&'static str> {
    let n = names[index(language)];
    let n = if n.is_empty() { names[0] } else { n };
    (!n.is_empty()).then_some(n)
}

/// The name Revit shows for the built-in category or subcategory `id` in
/// `language`, English where the help gives none in that language.
pub fn category_name(id: i64, language: &str) -> Option<&'static str> {
    let at = table::CATEGORIES.binary_search_by_key(&id, |r| r.0).ok()?;
    pick(&table::CATEGORIES[at].1, language)
}

/// The system family name of an element type of file class `class` in
/// `language`, `None` for a class whose types belong to a loadable family or
/// to more than one system family.
pub fn system_family_name(class: &str, language: &str) -> Option<&'static str> {
    let at = table::SYSTEM_FAMILIES
        .binary_search_by_key(&class, |r| r.0)
        .ok()?;
    pick(&table::SYSTEM_FAMILIES[at].1, language)
}

/// The help page (`<component>/<topic>`) an entry's names come from.
pub fn category_source(id: i64) -> Option<&'static str> {
    let at = table::CATEGORIES.binary_search_by_key(&id, |r| r.0).ok()?;
    Some(table::CATEGORIES[at].2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_are_sorted_for_lookup() {
        assert!(table::CATEGORIES.windows(2).all(|w| w[0].0 < w[1].0));
        assert!(table::SYSTEM_FAMILIES.windows(2).all(|w| w[0].0 < w[1].0));
        assert!(
            table::CATEGORIES
                .iter()
                .all(|r| !r.1[0].is_empty() && !r.2.is_empty())
        );
    }

    #[test]
    fn names_by_language() {
        assert_eq!(category_name(-2_000_936, "ENU"), Some("Riser Lines"));
        assert_eq!(category_name(-2_000_936, "XYZ"), Some("Riser Lines"));
        assert_ne!(category_name(-2_000_936, "DEU"), Some("Riser Lines"));
        assert_eq!(
            system_family_name("TextNoteAttributes", "ENU"),
            Some("Text")
        );
        assert_eq!(system_family_name("DBViewType", "ENU"), None);
    }
}
