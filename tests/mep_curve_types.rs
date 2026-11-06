//! the family and type of Revit 2024 ducts and pipes.
//!
//! A pipe or duct type has no name entry, so Revit's `Other.Family`,
//! `Other.Type` and `Other.Type Id` for a duct or pipe come from the type
//! object its reference list names: the ElementId twice and the name after
//! them. For each `<name>.rvt` with Revit's `<name>.ifc` beside it, every
//! element Revit gives the category Ducts or Pipes must get the same three
//! values from rvt-open, none may differ, and rvt-open must write them for at
//! least 99% of those elements.
//!
//! Runs against `RVT_REVIT2024_SAMPLES_DIR` (Autodesk's Revit 2024 sample
//! projects and Snowdon Towers, with Revit 2024 IFC4 exports). Skips what is
//! absent; the mechanical and plumbing models are the ones it checks.

use crate::category_type_phase::checked_values;
use rvt::RevitFile;
use rvt::ifc::{RvtDocExporter, write_step};
use std::path::PathBuf;

const VALUES: [(&str, &str); 3] = [("Other", "Family"), ("Other", "Type"), ("Other", "Type Id")];

fn duct_and_pipe_family_and_type_match_revits_export(k: usize, n: usize) {
    let Some(dir) = std::env::var_os("RVT_REVIT2024_SAMPLES_DIR").map(PathBuf::from) else {
        eprintln!("skipping: RVT_REVIT2024_SAMPLES_DIR is not set");
        return;
    };
    let category = ("Other".to_string(), "Category".to_string());
    let mut checked = 0;
    let mut all: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("samples dir")
        .map(|entry| entry.expect("entry").path())
        .filter(|rvt| {
            rvt.extension().is_some_and(|e| e == "rvt") && rvt.with_extension("ifc").exists()
        })
        .collect();
    all.sort();
    for rvt in crate::shard(all, k, n) {
        let reference = rvt.with_extension("ifc");
        let theirs = checked_values(&std::fs::read_to_string(&reference).expect("reference"));
        let curves: Vec<_> = theirs
            .iter()
            .filter(|(_, v)| {
                matches!(
                    v.get(&category).map(String::as_str),
                    Some("Ducts" | "Pipes")
                )
            })
            .collect();
        if curves.is_empty() {
            continue;
        }
        let mut rf = RevitFile::open(&rvt).expect("open");
        let result = RvtDocExporter
            .export_with_diagnostics(&mut rf)
            .expect("export");
        let ours = checked_values(&write_step(&result.model));
        let (mut right, mut wrong) = (0, Vec::new());
        for (tag, want) in &curves {
            let got = ours.get(*tag);
            let mut all = true;
            for (set, name) in VALUES {
                let key = (set.to_string(), name.to_string());
                match (want.get(&key), got.and_then(|g| g.get(&key))) {
                    (Some(w), Some(g)) if w == g => {}
                    (Some(w), Some(g)) => {
                        wrong.push((tag.to_string(), name, w.clone(), g.clone()));
                        all = false;
                    }
                    _ => all = false,
                }
            }
            right += usize::from(all);
        }
        eprintln!(
            "{}: {right} of {} ducts and pipes right",
            rvt.display(),
            curves.len()
        );
        assert!(
            wrong.is_empty(),
            "{}: {:?}",
            rvt.display(),
            &wrong[..wrong.len().min(5)]
        );
        assert!(
            right * 100 >= curves.len() * 99,
            "{}: {right} of {}",
            rvt.display(),
            curves.len()
        );
        checked += 1;
    }
    if checked == 0 {
        eprintln!("skipping: no duct or pipe in {}", dir.display());
    }
}

split!(duct_and_pipe_family_and_type_match_revits_export, duct_and_pipe_family_and_type_match_revits_export, s0 0, s1 1, s2 2, s3 3, s4 4, s5 5, s6 6, s7 7, s8 8, s9 9, s10 10, s11 11, s12 12, s13 13, s14 14);
