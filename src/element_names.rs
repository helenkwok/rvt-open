//! The name of an element, read from its own objects.
//!
//! Revit keeps most element names in the element object (`m_name`) or in an
//! object it points to: an element type's `m_symbolInfo`, a category's,
//! material's or pattern's own object, a parameter's definition. A graphics
//! style stores no name: it is its category's, a built-in category's from
//! the catalog, a subcategory's from that category element, and, where the
//! subcategory's own name is empty, the name of the element that owns it (a
//! text, tag or dimension type's own style category).
//!
//! Measured on 1-hip-roof (Revit 2024) against its Revit API dump; other
//! releases: see `docs/full-decode.md`.

use crate::builtin_parameters;
use crate::schema_objects::{Fields, Value};

/// Paths to an element's name, tried in order: field names joined by `/`,
/// each step through a field of the object or the object it points to.
/// The first path that ends in text gives the name; an empty text is kept
/// only for `m_pCategory` (a category element's name may be empty), otherwise
/// the next path is tried, and an empty `m_name` is the name when no path
/// gives another.
pub const NAME_PATHS: &[&str] = &[
    "m_name",
    "m_viewName",
    "m_sName",
    "m_strName",
    "m_symbolInfo/m_name",
    "m_pCategory/m_name",
    "m_pMaterial/m_name",
    "m_pFillPattern/m_name",
    "m_pLinePattern/m_name",
    "m_oTilePattern/m_name",
    "m_pAppearanceAsset/m_Name",
    "m_pParamDef/m_caption",
    "m_areaElemName",
    "m_sequenceName",
    "m_text",
];

/// Classes whose elements take their owner's name (`m_ownerId`): a font
/// element is named as the text, tag or dimension type it belongs to.
pub const OWNER_NAMED: &[&str] = &["FontElem"];

/// `PROPERTY_SET_NAME`: a property set element (a material's physical or
/// thermal asset) keeps its name as this parameter of its `m_oParamSet`.
const PROPERTY_SET_NAME: i64 = -1150466;

/// The object a field holds or points to.
fn object<'a>(objects: &'a [Fields], f: &'a Fields, name: &str) -> Option<&'a Fields> {
    match f.get(name)? {
        Value::Object(o) => Some(o),
        Value::New { index, .. } => objects.get(*index),
        _ => None,
    }
}

/// The text at `path` from the element object (`objects[0]`).
fn text_at<'a>(objects: &'a [Fields], path: &str) -> Option<&'a str> {
    let mut at = objects.first()?;
    let mut steps = path.split('/').peekable();
    while let Some(step) = steps.next() {
        if steps.peek().is_none() {
            return match at.get(step)? {
                Value::Text(t) => Some(t.as_str()),
                _ => None,
            };
        }
        at = object(objects, at, step)?;
    }
    None
}

/// The name an element's own objects store, `None` where none of
/// [`NAME_PATHS`] gives one.
pub fn own_name(objects: &[Fields]) -> Option<String> {
    NAME_PATHS
        .iter()
        .find_map(|path| {
            let t = text_at(objects, path)?;
            (!t.is_empty() || *path == "m_pCategory/m_name").then(|| t.to_string())
        })
        .or_else(|| property_set_name(objects))
        .or_else(|| text_at(objects, "m_name").map(str::to_string))
}

/// The `PROPERTY_SET_NAME` text of the element's `m_oParamSet`.
fn property_set_name(objects: &[Fields]) -> Option<String> {
    let set = object(objects, objects.first()?, "m_oParamSet")?;
    let strings = object(objects, set, "m_pAStringParams")?;
    let Some(Value::List(entries)) = strings.get("m_paramSet") else {
        return None;
    };
    entries.iter().find_map(|e| match e {
        Value::Object(e) if e.get("m_paramId") == Some(&Value::Id(PROPERTY_SET_NAME)) => {
            match e.get("m_value")? {
                Value::Text(t) => Some(t.clone()),
                _ => None,
            }
        }
        _ => None,
    })
}

/// A graphics style's name (`GStyleElem`), from its `m_categoryId`: a
/// built-in category's own name from the catalog, else `element_name` of the
/// category element, else, where that is empty, `element_name` of the
/// category element's owner (`owner_of`).
pub fn graphics_style_name(
    root: &Fields,
    element_name: impl Fn(i64) -> Option<String>,
    owner_of: impl Fn(i64) -> Option<i64>,
) -> Option<String> {
    let Some(Value::Id(category)) = root.get("m_categoryId") else {
        return None;
    };
    if *category < 0 {
        return builtin_parameters::category_own_name(*category).map(str::to_string);
    }
    match element_name(*category) {
        Some(n) if !n.is_empty() => Some(n),
        _ => element_name(owner_of(*category)?),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(values: Vec<(&str, Value)>) -> Fields {
        let n = values.len();
        Fields {
            tag: 0,
            span: 0..0,
            values: values
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
            spans: vec![0..0; n],
        }
    }

    #[test]
    fn name_through_a_pointer_and_skipping_empty_texts() {
        let root = fields(vec![
            ("m_viewName", Value::Text(String::new())),
            (
                "m_symbolInfo",
                Value::New {
                    tag: 1,
                    index: 1,
                    marker: 0,
                },
            ),
        ]);
        let info = fields(vec![("m_name", Value::Text("Generic - 200mm".into()))]);
        assert_eq!(own_name(&[root, info]).as_deref(), Some("Generic - 200mm"));
    }

    #[test]
    fn graphics_style_of_a_subcategory_falls_back_to_its_owner() {
        let style = fields(vec![("m_categoryId", Value::Id(235))]);
        let name = graphics_style_name(
            &style,
            |id| {
                Some(if id == 234 {
                    "Arrowhead 1".into()
                } else {
                    String::new()
                })
            },
            |id| (id == 235).then_some(234),
        );
        assert_eq!(name.as_deref(), Some("Arrowhead 1"));
        let walls = fields(vec![("m_categoryId", Value::Id(-2000011))]);
        assert_eq!(
            graphics_style_name(&walls, |_| None, |_| None).as_deref(),
            Some("Walls")
        );
        // A built-in subcategory with no public name has none: the element
        // and owner lookups are not tried for a built-in category.
        let glass = fields(vec![("m_categoryId", Value::Id(-2000031))]);
        assert_eq!(
            graphics_style_name(&glass, |_| Some("x".into()), |_| Some(1)),
            None
        );
    }
}
