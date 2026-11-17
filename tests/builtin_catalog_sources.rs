//! The built-in parameter catalog is filled from public sources only, and each field says which.
//!
//! A field is filled exactly where its source code (`R` the Revit 2027 API reference, `F` the values
//! public files store, `E` Revit's IFC export of public samples, `H` machine-matched help terms,
//! `-` none) says a source gave it. The count of fields each source fills is pinned, so a field
//! filled from anywhere else (the Revit API dump of the public samples is the answer key only:
//! `tools/builtin-parameters/score.py`) changes a count and fails here.

use rvt::builtin_parameters::{
    LOCALIZED_LANGUAGES, enum_text, localized_name, parameter, parameter_ids,
};
use std::collections::BTreeMap;
use std::path::Path;

/// Entries, then fields filled per source: name, enum name (API reference), storage type
/// (files), spec and group (IFC export), localized names (help).
const ENTRIES: usize = 3812;
const FILLED: [usize; 6] = [3805, 3805, 486, 303, 495, LOCALIZED];
const LOCALIZED: usize = 1455;

#[test]
fn every_field_is_from_its_public_source_or_empty() {
    let mut filled = [0usize; 6];
    let mut entries = 0;
    for id in parameter_ids() {
        entries += 1;
        let p = parameter(id).expect("listed id");
        let codes: Vec<char> = p.sources.chars().collect();
        assert_eq!(codes.len(), 6, "{id}: {}", p.sources);
        let has_help = LOCALIZED_LANGUAGES
            .iter()
            .all(|l| localized_name(id, l).is_some());
        let values = [
            !p.name.is_empty(),
            !p.enum_name.is_empty(),
            !p.storage_type.is_empty(),
            !p.spec_type_id.is_empty(),
            !p.group.is_empty(),
            has_help,
        ];
        for (k, (&on, allowed)) in values
            .iter()
            .zip(['R', 'R', 'F', 'E', 'E', 'H'])
            .enumerate()
        {
            let want = if on { allowed } else { '-' };
            assert_eq!(codes[k], want, "{id} field {k}: {}", p.sources);
            filled[k] += usize::from(on);
        }
    }
    assert_eq!(entries, ENTRIES);
    assert_eq!(filled, FILLED);
}

#[test]
fn ids_no_public_source_names_are_kept_with_no_name() {
    for id in [
        -1_154_658, -1_154_657, -1_140_313, -1_017_024, -1_017_023, -1_006_800, -1_005_051,
    ] {
        let p = parameter(id).expect("kept id");
        assert_eq!((p.name, p.enum_name), ("", ""), "{id}");
        assert_eq!(&p.sources[..2], "--", "{id}");
    }
}

/// The rows of a tab-separated public input of `tools/builtin-parameters/inputs/`, without its comments.
fn input(name: &str) -> Vec<Vec<String>> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tools/builtin-parameters/inputs")
        .join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| l.split('\t').map(str::to_string).collect())
        .collect()
}

/// The spec type id of an IFC value type of Revit's export.
fn spec_of(ifc_type: &str) -> &'static str {
    match ifc_type {
        "IFCLENGTHMEASURE" => "autodesk.spec.aec:length-2.0.0",
        "IFCAREAMEASURE" => "autodesk.spec.aec:area-2.0.0",
        "IFCVOLUMEMEASURE" => "autodesk.spec.aec:volume-2.0.0",
        "IFCPLANEANGLEMEASURE" => "autodesk.spec.aec:angle-2.0.0",
        "IFCBOOLEAN" => "autodesk.spec:spec.bool-1.0.0",
        "IFCINTEGER" => "autodesk.spec:spec.int64-2.0.0",
        "IFCREAL" => "autodesk.spec.aec:number-2.0.0",
        "IFCELECTRICCURRENTMEASURE" => "autodesk.spec.aec.electrical:current-2.0.0",
        "IFCELECTRICVOLTAGEMEASURE" => "autodesk.spec.aec.electrical:potential-2.0.0",
        "IFCPOWERMEASURE" => "autodesk.spec.aec.electrical:power-2.0.0",
        "IFCVOLUMETRICFLOWRATEMEASURE" => "autodesk.spec.aec.hvac:airFlow-2.0.0",
        "IFCPRESSUREMEASURE" => "autodesk.spec.aec.hvac:pressure-2.0.0",
        _ => "",
    }
}

#[test]
fn every_value_is_a_row_of_a_public_input() {
    let mut reference: BTreeMap<i64, (String, String)> = BTreeMap::new();
    for r in input("bip_enum.tsv") {
        let id: i64 = r[1].parse().expect("id");
        reference
            .entry(id)
            .or_insert((r[0].clone(), r.get(2).cloned().unwrap_or_default()));
    }
    let storage: BTreeMap<i64, String> = input("storage.tsv")
        .into_iter()
        .map(|r| (r[0].parse().expect("id"), r[1].clone()))
        .collect();
    let group_spec: BTreeMap<i64, (String, String)> = input("group-spec.tsv")
        .into_iter()
        .map(|r| (r[0].parse().expect("id"), (r[1].clone(), r[2].clone())))
        .collect();
    for id in parameter_ids() {
        let p = parameter(id).expect("listed id");
        let (enum_name, label) = reference.get(&id).cloned().unwrap_or_default();
        assert_eq!(
            (p.name, p.enum_name),
            (label.as_str(), enum_name.as_str()),
            "{id}"
        );
        assert_eq!(
            p.storage_type,
            storage.get(&id).map_or("", String::as_str),
            "{id}"
        );
        let (group, ifc_type) = group_spec.get(&id).cloned().unwrap_or_default();
        assert_eq!(
            (p.group, p.spec_type_id),
            (group.as_str(), spec_of(&ifc_type)),
            "{id}"
        );
    }
    for r in input("enum-texts.tsv") {
        let (id, value): (i64, i64) = (r[0].parse().expect("id"), r[1].parse().expect("value"));
        assert_eq!(enum_text(id, value), Some(r[2].as_str()), "{id} {value}");
    }
    let mut terms = 0;
    for r in input("help-terms.tsv") {
        let id: i64 = r[0].parse().expect("id");
        assert_eq!(
            localized_name(id, &r[1]),
            Some((r[2].as_str(), r[3].as_str())),
            "{id} {}",
            r[1]
        );
        terms += 1;
    }
    assert_eq!(terms, 12 * LOCALIZED);
}
