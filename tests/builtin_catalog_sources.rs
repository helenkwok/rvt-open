//! The built-in parameter catalog is filled from public sources only, and each field says which.
//!
//! A field is filled exactly where its source code (`R` the Revit 2027 API reference, `F` the values
//! public files store, `E` Revit's IFC export of public samples, `H` machine-matched help terms,
//! `-` none) says a source gave it. The count of fields each source fills is pinned, so a field
//! filled from anywhere else (the Revit API dump of the public samples is the answer key only:
//! `tools/builtin-parameters/score.py`) changes a count and fails here.

use rvt::builtin_parameters::{LOCALIZED_LANGUAGES, localized_name, parameter, parameter_ids};

/// Entries, then fields filled per source: name, enum name (API reference), storage type
/// (files), spec and group (IFC export), localized names (help).
const ENTRIES: usize = 3812;
const FILLED: [usize; 6] = [3805, 3805, 486, 303, 495, LOCALIZED];
const LOCALIZED: usize = 1476;

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
