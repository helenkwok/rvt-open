//! the MEP categories Revit's IFC4 export writes as cable
//! carriers, valves, fire-suppression terminals, flow terminals and flexible
//! duct segments are in rvt-open's export, as the same entity, with exactly
//! Revit's `Tag` set on each file.
//!
//! Runs against `RVT_REVIT2024_SAMPLES_DIR` (Autodesk's Revit 2024 samples
//! with Revit's own IFC4 export beside each: Snowdon Towers Electrical,
//! Plumbing and HVAC, rme_basic and rme_advanced). Skips what is absent.

use rvt::RevitFile;
use rvt::ifc::{RvtDocExporter, write_step};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const ENTITIES: [&str; 6] = [
    "IFCCABLECARRIERSEGMENT",
    "IFCCABLECARRIERFITTING",
    "IFCVALVE",
    "IFCFIRESUPPRESSIONTERMINAL",
    "IFCFLOWTERMINAL",
    "IFCDUCTSEGMENT",
];

const FILES: [&str; 5] = [
    "Snowdon Towers Sample Electrical",
    "Snowdon Towers Sample Plumbing",
    "Snowdon Towers Sample HVAC",
    "rmebasicsampleproject",
    "rmeadvancedsampleproject",
];

/// The 8th top-level argument of a STEP argument list (an element's `Tag`).
fn tag_of(args: &str) -> Option<String> {
    let (mut quoted, mut depth, mut index) = (false, 0usize, 0usize);
    let mut current = String::new();
    for c in args.chars() {
        match c {
            '\'' => quoted = !quoted,
            '(' if !quoted => depth += 1,
            ')' if !quoted => depth = depth.saturating_sub(1),
            ',' if !quoted && depth == 0 => {
                if index == 7 {
                    break;
                }
                index += 1;
                current.clear();
                continue;
            }
            _ => {}
        }
        if index == 7 {
            current.push(c);
        }
    }
    (index == 7).then(|| current.trim().trim_matches('\'').to_string())
}

/// `entity -> Tag set` for [`ENTITIES`] in a STEP text.
fn tags(step: &str) -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for line in step.lines() {
        let Some((_, body)) = line.split_once('=') else {
            continue;
        };
        let Some((entity, args)) = body.trim_start().split_once('(') else {
            continue;
        };
        if ENTITIES.contains(&entity) {
            if let Some(tag) = tag_of(args) {
                out.entry(entity.to_string()).or_default().insert(tag);
            }
        }
    }
    out
}

fn export(path: &Path) -> String {
    let mut rf = RevitFile::open(path).expect("open");
    let result = RvtDocExporter
        .export_with_diagnostics(&mut rf)
        .expect("export");
    write_step(&result.model)
}

fn mep_category_entities_match_revits_tags(k: usize, n: usize) {
    let Some(dir) = std::env::var_os("RVT_REVIT2024_SAMPLES_DIR").map(PathBuf::from) else {
        eprintln!("skipping: RVT_REVIT2024_SAMPLES_DIR is not set");
        return;
    };
    let mut checked = 0;
    for name in crate::shard(FILES, k, n) {
        let (rvt, ifc) = (
            dir.join(format!("{name}.rvt")),
            dir.join(format!("{name}.ifc")),
        );
        let (true, Ok(reference)) = (rvt.exists(), std::fs::read_to_string(&ifc)) else {
            eprintln!("skipping {name}: absent");
            continue;
        };
        let theirs = tags(&reference);
        let ours = tags(&export(&rvt));
        for entity in ENTITIES {
            let empty = BTreeSet::new();
            let (r, o) = (
                theirs.get(entity).unwrap_or(&empty),
                ours.get(entity).unwrap_or(&empty),
            );
            assert_eq!(
                o.len(),
                r.len(),
                "{name} {entity}: {} written, Revit {}",
                o.len(),
                r.len()
            );
            assert_eq!(o, r, "{name} {entity}: Tag sets differ");
            checked += r.len();
        }
    }
    eprintln!("checked {checked} elements");
}

split!(mep_category_entities_match_revits_tags, mep_category_entities_match_revits_tags, s0 0, s1 1, s2 2, s3 3, s4 4);
