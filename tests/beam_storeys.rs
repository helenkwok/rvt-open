//! Real-file integration test: IfcBeam building storey containment matches Revit's reference IFC export.
//!
//! Rule:
//! 1. If the beam belongs to a Structural Beam System, use the highest building storey
//!    whose elevation minus 0.10 m is at or below the bottom (min z of the bounding box)
//!    of the beam system; if there is none, use the lowest building storey.
//! 2. Otherwise, use the beam's Reference Level if that Level is a building storey;
//!    if not, use the next building storey below it (inclusive of equal elevation).
//!    If no building storey is below it, retain the fallback containment.
//!
//! Runs against `RVT_REVIT2024_SAMPLES_DIR` and skips gracefully when absent.
//!
//! Coverage (review): every beam present in both exports is compared;
//! a beam Revit contains in a storey that our export does not bind — or does
//! not export at all — fails the test rather than being skipped. A
//! pinned per-file minimum of compared beams, measured on the corpus by the
//! gate run, is asserted.

use rvt::RevitFile;
use rvt::ifc::{RvtDocExporter, write_ifcx};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Minimum number of beams expected to be compared across the target files.
/// Measured by the gate run over `SAMPLE_STEMS`: 969 beams (14 + 328 + 0 + 14 + 249 + 364).
const MIN_BEAMS_COMPARED: usize = 969;

/// Sample stems to check in `RVT_REVIT2024_SAMPLES_DIR`.
const SAMPLE_STEMS: &[&str] = &[
    "racadvancedsampleproject",
    "rstadvancedsampleproject",
    "rstbasicsampleproject",
    "Technicalschoolcurrentm",
    "DACHsampleproject",
    // stem contains spaces; the .rvt and .ifc sit side by side in the same dir
    "Snowdon Towers Sample Structural",
];

/// Split a STEP argument list on top-level commas.
fn split_args(args: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let (mut quoted, mut depth) = (false, 0usize);
    for c in args.chars() {
        match c {
            '\'' => {
                quoted = !quoted;
                current.push(c);
            }
            '(' if !quoted => {
                depth += 1;
                current.push(c);
            }
            ')' if !quoted => {
                depth = depth.saturating_sub(1);
                current.push(c);
            }
            ',' if !quoted && depth == 0 => out.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    out.push(current);
    out
}

/// A STEP string's decoded text: doubled quotes `''`, `\X\hh`
/// (ISO 8859-1) and `\X2\hhhh…\X0\` (UTF-16) are decoded, so a name from
/// Revit's STEP export compares equal to the same name in our IFCX JSON
/// (already decoded by serde_json). Reuses the STEP decoding of
/// tests/level_parameters.rs (its `step_text`, via
/// `super::category_type_phase::decode`).
fn unquote(raw: &str) -> String {
    let raw = raw.trim();
    match (raw.find('\''), raw.rfind('\'')) {
        (Some(a), Some(b)) if b > a => format!(
            "{}{}{}",
            &raw[..a],
            super::category_type_phase::decode(&raw[a + 1..b]),
            &raw[b + 1..]
        ),
        _ => raw.to_string(),
    }
}

fn references(field: &str) -> Vec<u64> {
    split_args(field.trim().trim_start_matches('(').trim_end_matches(')'))
        .iter()
        .filter_map(|f| f.trim().strip_prefix('#')?.parse().ok())
        .collect()
}

/// Parse `#id = ENTITY(args);` lines from a STEP export.
fn parse_step_entities(step: &str) -> BTreeMap<u64, (String, String)> {
    let mut out = BTreeMap::new();
    for line in step.lines() {
        let Some(rest) = line.strip_prefix('#') else {
            continue;
        };
        let Some((id, body)) = rest.split_once('=') else {
            continue;
        };
        let Some((entity, args)) = body.split_once('(') else {
            continue;
        };
        let args = args.trim_end().trim_end_matches(';');
        let args = args.strip_suffix(')').unwrap_or(args);
        if let Ok(id) = id.trim().parse() {
            out.insert(id, (entity.trim().to_string(), args.to_string()));
        }
    }
    out
}

/// Map each beam's `Tag` (ElementId) to the `Name` of the `IfcBuildingStorey` containing it in Revit's export.
///
/// A beam is contained either directly in an `IfcRelContainedInSpatialStructure`
/// whose relating structure is a building storey, or through an `IfcElementAssembly`
/// (via `IfcRelAggregates`) that is contained in a building storey. A member beam
/// must not be directly contained in any spatial structure.
fn revit_beam_storeys(step: &str) -> BTreeMap<String, String> {
    let ents = parse_step_entities(step);
    let mut storey_names: BTreeMap<u64, String> = BTreeMap::new();
    for (&id, (entity, args)) in &ents {
        if entity == "IFCBUILDINGSTOREY" {
            let f = split_args(args);
            if let Some(name) = f.get(2).map(|n| unquote(n)) {
                storey_names.insert(id, name);
            }
        }
    }

    let mut directly_contained: std::collections::BTreeSet<u64> = std::collections::BTreeSet::new();
    let mut direct_beams: BTreeMap<String, String> = BTreeMap::new();
    let mut assembly_storeys: BTreeMap<u64, String> = BTreeMap::new();

    for (entity, args) in ents.values() {
        if entity != "IFCRELCONTAINEDINSPATIALSTRUCTURE" {
            continue;
        }
        let f = split_args(args);
        let related_elements = references(f.get(4).map(String::as_str).unwrap_or(""));
        for &elem_id in &related_elements {
            directly_contained.insert(elem_id);
        }

        let Some(structure_id) = f.get(5).and_then(|s| references(s).first().copied()) else {
            continue;
        };
        let Some(storey_name) = storey_names.get(&structure_id) else {
            continue;
        };

        for elem_id in related_elements {
            let Some((elem_entity, elem_args)) = ents.get(&elem_id) else {
                continue;
            };
            if elem_entity == "IFCBEAM" {
                let tag = split_args(elem_args)
                    .get(7)
                    .map(|t| unquote(t))
                    .unwrap_or_default();
                if !tag.is_empty() && tag.chars().all(|c| c.is_ascii_digit()) {
                    direct_beams.insert(tag, storey_name.clone());
                }
            } else if elem_entity == "IFCELEMENTASSEMBLY" {
                assembly_storeys.insert(elem_id, storey_name.clone());
            }
        }
    }

    let mut out = direct_beams;

    for (entity, args) in ents.values() {
        if entity != "IFCRELAGGREGATES" {
            continue;
        }
        let f = split_args(args);
        let Some(whole_id) = f.get(4).and_then(|s| references(s).first().copied()) else {
            continue;
        };
        let Some((whole_entity, _)) = ents.get(&whole_id) else {
            continue;
        };
        if whole_entity != "IFCELEMENTASSEMBLY" {
            continue;
        }
        let assembly_storey = assembly_storeys.get(&whole_id);

        for part_id in references(f.get(5).map(String::as_str).unwrap_or("")) {
            let Some((part_entity, part_args)) = ents.get(&part_id) else {
                continue;
            };
            if part_entity != "IFCBEAM" {
                continue;
            }
            assert!(
                !directly_contained.contains(&part_id),
                "member beam #{part_id} of assembly #{whole_id} is ALSO directly contained in IfcRelContainedInSpatialStructure"
            );
            let tag = split_args(part_args)
                .get(7)
                .map(|t| unquote(t))
                .unwrap_or_default();
            if !tag.is_empty() && tag.chars().all(|c| c.is_ascii_digit()) {
                if let Some(storey_name) = assembly_storey {
                    out.insert(tag, storey_name.clone());
                }
            }
        }
    }

    out
}

/// Map each IfcBeam node's Tag (ElementId) to its parent IfcBuildingStorey's Name in our IFCX output.
/// Every tagged beam is returned: `None` when it has no storey parent, so a
/// beam Revit binds but we leave unbound is detected, not silently skipped.
///
/// A member beam's storey is resolved through its IfcElementAssembly parent node.
/// A member beam must not be directly contained under any spatial structure node.
fn our_ifcx_beam_storeys(ifcx_json: &Value) -> BTreeMap<String, Option<String>> {
    let Some(data) = ifcx_json["data"].as_array() else {
        return BTreeMap::new();
    };

    let mut child_to_storey: BTreeMap<String, String> = BTreeMap::new();
    let mut directly_contained: std::collections::BTreeSet<String> =
        std::collections::BTreeSet::new();

    for node in data {
        let is_spatial = node["attributes"]["bsi::ifc::class"]["code"]
            .as_str()
            .is_some_and(|c| {
                c == "IfcBuildingStorey"
                    || c == "IFCBUILDINGSTOREY"
                    || c == "IfcBuilding"
                    || c == "IFCBUILDING"
                    || c == "IfcSpace"
                    || c == "IFCSPACE"
            });
        let is_storey = node["attributes"]["bsi::ifc::class"]["code"]
            .as_str()
            .is_some_and(|c| c == "IfcBuildingStorey" || c == "IFCBUILDINGSTOREY");
        let storey_name = if is_storey {
            node["attributes"]["bsi::ifc::prop::Name"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        } else {
            String::new()
        };

        if let Some(children) = node["children"].as_object() {
            for child_path in children.values().filter_map(|v| v.as_str()) {
                if is_spatial {
                    directly_contained.insert(child_path.to_string());
                }
                if is_storey {
                    child_to_storey.insert(child_path.to_string(), storey_name.clone());
                }
            }
        }
    }

    let mut member_to_storey: BTreeMap<String, Option<String>> = BTreeMap::new();
    for node in data {
        let is_assembly = node["attributes"]["bsi::ifc::class"]["code"]
            .as_str()
            .is_some_and(|c| c == "IfcElementAssembly" || c == "IFCELEMENTASSEMBLY");
        if !is_assembly {
            continue;
        }
        let assembly_path = node["path"].as_str().unwrap_or_default();
        let assembly_storey = child_to_storey.get(assembly_path).cloned();

        if let Some(children) = node["children"].as_object() {
            for member_path in children.values().filter_map(|v| v.as_str()) {
                assert!(
                    !directly_contained.contains(member_path),
                    "member beam {member_path} of assembly {assembly_path} is ALSO directly contained in a spatial structure node"
                );
                member_to_storey.insert(member_path.to_string(), assembly_storey.clone());
            }
        }
    }

    let mut out = BTreeMap::new();
    for node in data {
        let is_beam = node["attributes"]["bsi::ifc::class"]["code"]
            .as_str()
            .is_some_and(|c| c == "IfcBeam" || c == "IFCBEAM");
        if !is_beam {
            continue;
        }
        let tag = node["attributes"]["ifc-src::attr::Tag"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if tag.is_empty() {
            continue;
        }
        let Some(path) = node["path"].as_str() else {
            continue;
        };
        let storey = match member_to_storey.get(path) {
            Some(s) => s.clone(),
            None => child_to_storey.get(path).cloned(),
        };
        out.insert(tag, storey);
    }
    out
}

/// Minimum number of beams present in both exports for one file.
/// Measured: 14 is the smallest of the five samples whose Revit
/// export contains storeys (14, 328, 14, 249, 364); rstbasicsampleproject is
/// exempt because Revit's export of it has no `IfcBuildingStorey` at all, so
/// no beam there can be compared.
const MIN_BEAMS_PER_FILE: usize = 14;

fn check_model(rvt_path: &Path, ifc_path: &Path, failures: &mut Vec<String>) -> usize {
    let step_content = std::fs::read_to_string(ifc_path).expect("read reference IFC");
    let revit_beams = revit_beam_storeys(&step_content);

    let mut rf = RevitFile::open(rvt_path).expect("open rvt");
    let result = RvtDocExporter
        .export_with_diagnostics(&mut rf)
        .expect("export rvt");
    let ifcx_str = write_ifcx(&result.model).expect("write ifcx");
    let ifcx_val: Value = serde_json::from_str(&ifcx_str).expect("parse ifcx json");
    let our_beams = our_ifcx_beam_storeys(&ifcx_val);

    let our_step_content = rvt::ifc::write_step(&result.model);
    let our_step_beams = revit_beam_storeys(&our_step_content);

    // Verify consistency between our IFCX and our STEP outputs
    for (tag, ifcx_storey) in &our_beams {
        if let Some(ifcx_s) = ifcx_storey {
            if our_step_beams.get(tag) != Some(ifcx_s) {
                failures.push(format!(
                    "{}: beam {tag} storey mismatch between our IFCX ({ifcx_s:?}) and our STEP ({:?})",
                    rvt_path.file_stem().unwrap_or_default().to_string_lossy(),
                    our_step_beams.get(tag)
                ));
            }
        }
    }

    let stem = rvt_path.file_stem().unwrap_or_default().to_string_lossy();
    let mut compared = 0;
    let mut mismatches = Vec::new();

    for (tag, revit_storey) in &revit_beams {
        // Beams present in both exports are compared; a beam Revit puts in a
        // storey that ours does not bind — or does not export at all — is a
        // failure, not a skip (review: one unchanged match must not
        // mask the affected beams).
        let Some(our_storey) = our_beams.get(tag) else {
            mismatches.push(format!(
                "Beam {tag}: Revit contains it in storey {revit_storey:?}; our IFCX has no such beam"
            ));
            continue;
        };
        compared += 1;
        match our_storey {
            Some(our) if our != revit_storey => {
                mismatches.push(format!(
                    "Beam {tag}: Revit export storey={revit_storey:?}, our IFCX storey={our:?}"
                ));
            }
            None => {
                mismatches.push(format!(
                    "Beam {tag}: Revit contains it in storey {revit_storey:?}; our IFCX leaves it without a storey parent"
                ));
            }
            _ => {}
        }
    }

    eprintln!(
        "{stem}: {compared} beams compared (of {revit_total} Revit-storey'd; {our_bound} of our {our_total} tagged beams are storey-bound), {} mismatches",
        mismatches.len(),
        revit_total = revit_beams.len(),
        our_total = our_beams.len(),
        our_bound = our_beams.values().filter(|s| s.is_some()).count(),
    );

    if !mismatches.is_empty() {
        failures.push(format!(
            "{stem}: {} of {compared} compared beams had storey mismatches; first 5: {:?}",
            mismatches.len(),
            mismatches.iter().take(5).collect::<Vec<_>>()
        ));
    }

    // A sample whose Revit export contains no storey-bound beams has nothing
    // to compare, so it cannot be held to the per-file minimum.
    if !revit_beams.is_empty() && compared < MIN_BEAMS_PER_FILE {
        failures.push(format!(
            "{stem}: {compared} beam(s) common to both exports, below the pinned minimum {MIN_BEAMS_PER_FILE}"
        ));
    }

    compared
}

fn ifc_beam_storeys_match_revit_export(k: usize, n: usize) {
    let Some(dir) = std::env::var_os("RVT_REVIT2024_SAMPLES_DIR").map(PathBuf::from) else {
        eprintln!("skipping: RVT_REVIT2024_SAMPLES_DIR is not set");
        return;
    };

    let mut total_compared = 0;
    let mut failures = Vec::new();

    for stem in crate::shard(SAMPLE_STEMS, k, n) {
        let rvt_path = dir.join(format!("{stem}.rvt"));
        if !rvt_path.exists() {
            eprintln!("skipping {stem}: .rvt absent");
            continue;
        }

        let ref_ifc = [format!("{stem}.ifc"), format!("{stem}_Revit.ifc")]
            .into_iter()
            .map(|name| dir.join(name))
            .find(|p| p.exists());

        let Some(ifc_path) = ref_ifc else {
            eprintln!("skipping {stem}: reference .ifc absent");
            continue;
        };

        total_compared += check_model(&rvt_path, &ifc_path, &mut failures);
    }

    assert!(
        failures.is_empty(),
        "Storey containment mismatches:\n{}",
        failures.join("\n")
    );

    // The pin is on the total over all shards, checked once all have run.
    let all = crate::all_shards(
        "beam_storeys-ifc_beam_storeys",
        k,
        n,
        &total_compared.to_string(),
    );
    if let Some(all) = all.filter(|_| dir.exists()) {
        let total_compared: usize = all.iter().map(|v| v.parse::<usize>().unwrap()).sum();
        assert!(
            total_compared >= MIN_BEAMS_COMPARED,
            "Only {total_compared} beams compared in RVT_REVIT2024_SAMPLES_DIR, below the pinned {MIN_BEAMS_COMPARED}"
        );
    }
}

split!(ifc_beam_storeys_match_revit_export, ifc_beam_storeys_match_revit_export, s0 0, s1 1, s2 2, s3 3, s4 4, s5 5);

#[test]
fn test_revit_beam_storeys_synthetic_assembly() {
    let step = r#"
#1 = IFCBUILDINGSTOREY('gid1', #0, 'Level 1', $, $, #10, $, $, .ELEMENT., 0.0);
#2 = IFCBUILDINGSTOREY('gid2', #0, 'Level 2', $, $, #20, $, $, .ELEMENT., 10.0);
#100 = IFCELEMENTASSEMBLY('gid100', #0, 'Structural Beam System:Beam System:100', $, 'Structural Beam System:Beam System', #101, $, '100', .NOTDEFINED., .BEAM_GRID.);
#1001 = IFCBEAM('gid1001', #0, 'Beam 1', $, $, #102, $, '1001', .BEAM.);
#1002 = IFCBEAM('gid1002', #0, 'Beam 2', $, $, #103, $, '1002', .BEAM.);
#1003 = IFCBEAM('gid1003', #0, 'Beam 3', $, $, #104, $, '1003', .BEAM.);
#200 = IFCRELAGGREGATES('gid200', #0, $, $, #100, (#1001, #1002));
#300 = IFCRELCONTAINEDINSPATIALSTRUCTURE('gid300', #0, $, $, (#100), #2);
#301 = IFCRELCONTAINEDINSPATIALSTRUCTURE('gid301', #0, $, $, (#1003), #1);
"#;
    let storeys = revit_beam_storeys(step);
    assert_eq!(storeys.get("1001"), Some(&"Level 2".to_string()));
    assert_eq!(storeys.get("1002"), Some(&"Level 2".to_string()));
    assert_eq!(storeys.get("1003"), Some(&"Level 1".to_string()));
}

#[test]
#[should_panic(expected = "is ALSO directly contained in IfcRelContainedInSpatialStructure")]
fn test_revit_beam_storeys_fails_if_member_directly_contained() {
    let step = r#"
#1 = IFCBUILDINGSTOREY('gid1', #0, 'Level 1', $, $, #10, $, $, .ELEMENT., 0.0);
#2 = IFCBUILDINGSTOREY('gid2', #0, 'Level 2', $, $, #20, $, $, .ELEMENT., 10.0);
#100 = IFCELEMENTASSEMBLY('gid100', #0, 'Structural Beam System:Beam System:100', $, 'Structural Beam System:Beam System', #101, $, '100', .NOTDEFINED., .BEAM_GRID.);
#1001 = IFCBEAM('gid1001', #0, 'Beam 1', $, $, #102, $, '1001', .BEAM.);
#200 = IFCRELAGGREGATES('gid200', #0, $, $, #100, (#1001));
#300 = IFCRELCONTAINEDINSPATIALSTRUCTURE('gid300', #0, $, $, (#100), #2);
#301 = IFCRELCONTAINEDINSPATIALSTRUCTURE('gid301', #0, $, $, (#1001), #1);
"#;
    revit_beam_storeys(step);
}

#[test]
fn test_our_ifcx_beam_storeys_synthetic_assembly() {
    let ifcx_json = serde_json::json!({
        "data": [
            {
                "path": "p/storey1",
                "attributes": {
                    "bsi::ifc::class": { "code": "IfcBuildingStorey" },
                    "bsi::ifc::prop::Name": "Level 1"
                },
                "children": {
                    "c1": "p/beam1003"
                }
            },
            {
                "path": "p/storey2",
                "attributes": {
                    "bsi::ifc::class": { "code": "IfcBuildingStorey" },
                    "bsi::ifc::prop::Name": "Level 2"
                },
                "children": {
                    "c1": "p/assembly100"
                }
            },
            {
                "path": "p/assembly100",
                "attributes": {
                    "bsi::ifc::class": { "code": "IfcElementAssembly" }
                },
                "children": {
                    "c1": "p/beam1001",
                    "c2": "p/beam1002"
                }
            },
            {
                "path": "p/beam1001",
                "attributes": {
                    "bsi::ifc::class": { "code": "IfcBeam" },
                    "ifc-src::attr::Tag": "1001"
                }
            },
            {
                "path": "p/beam1002",
                "attributes": {
                    "bsi::ifc::class": { "code": "IfcBeam" },
                    "ifc-src::attr::Tag": "1002"
                }
            },
            {
                "path": "p/beam1003",
                "attributes": {
                    "bsi::ifc::class": { "code": "IfcBeam" },
                    "ifc-src::attr::Tag": "1003"
                }
            }
        ]
    });
    let storeys = our_ifcx_beam_storeys(&ifcx_json);
    assert_eq!(storeys.get("1001"), Some(&Some("Level 2".to_string())));
    assert_eq!(storeys.get("1002"), Some(&Some("Level 2".to_string())));
    assert_eq!(storeys.get("1003"), Some(&Some("Level 1".to_string())));
}

#[test]
#[should_panic(expected = "is ALSO directly contained in a spatial structure node")]
fn test_our_ifcx_beam_storeys_fails_if_member_directly_contained() {
    let ifcx_json = serde_json::json!({
        "data": [
            {
                "path": "p/storey1",
                "attributes": {
                    "bsi::ifc::class": { "code": "IfcBuildingStorey" },
                    "bsi::ifc::prop::Name": "Level 1"
                },
                "children": {
                    "c1": "p/beam1001"
                }
            },
            {
                "path": "p/storey2",
                "attributes": {
                    "bsi::ifc::class": { "code": "IfcBuildingStorey" },
                    "bsi::ifc::prop::Name": "Level 2"
                },
                "children": {
                    "c1": "p/assembly100"
                }
            },
            {
                "path": "p/assembly100",
                "attributes": {
                    "bsi::ifc::class": { "code": "IfcElementAssembly" }
                },
                "children": {
                    "c1": "p/beam1001"
                }
            },
            {
                "path": "p/beam1001",
                "attributes": {
                    "bsi::ifc::class": { "code": "IfcBeam" },
                    "ifc-src::attr::Tag": "1001"
                }
            }
        ]
    });
    our_ifcx_beam_storeys(&ifcx_json);
}
