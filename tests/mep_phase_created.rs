//! Snowdon Towers' MEP models name their phases in a second
//! layout, so their elements used to carry no `Phase Created`.
//!
//! Snowdon Towers Sample Electrical / HVAC / Plumbing (Autodesk, Revit 2024,
//! CC BY-NC-SA) whose Revit IFC4 exports carry `Phase Created` on every element. Their phase
//! objects put the class tag 40 bytes before the ElementId (20 zero bytes,
//! `ff ff ff ff`, a 2-byte tag, 8 zero bytes between), not 38. The two phases
//! are `New Construction` and `Existing`, and rvt-open must write `Phase
//! Created` for at least 99% of the elements Revit does (counts measured against
//! the exports; `category_type_phase` checks the values).
//!
//! Runs against `RVT_REVIT2024_SAMPLES_DIR`. Skips what is absent.

use rvt::RevitFile;
use rvt::ifc::{RvtDocExporter, write_step};
use std::path::PathBuf;

fn phase_values(step: &str) -> usize {
    step.matches("IFCPROPERTYSINGLEVALUE('Phase Created'")
        .count()
}

fn snowdon_mep_elements_carry_phase_created(k: usize, n: usize) {
    let Some(dir) = std::env::var_os("RVT_REVIT2024_SAMPLES_DIR").map(PathBuf::from) else {
        eprintln!("skipping: RVT_REVIT2024_SAMPLES_DIR is not set");
        return;
    };
    // Elements with a Phase Created in Revit's export (measured).
    let parts = [("Electrical", 1576), ("HVAC", 2596), ("Plumbing", 6253)];
    for (part, theirs) in crate::shard(parts, k, n) {
        let rvt = dir.join(format!("Snowdon Towers Sample {part}.rvt"));
        if !rvt.exists() {
            eprintln!("skipping: {} is absent", rvt.display());
            continue;
        }
        let mut rf = RevitFile::open(&rvt).expect("open");
        let phases = rvt::research::partition_phases::phase_names(&mut rf).expect("phases");
        let names: Vec<&str> = phases.values().map(String::as_str).collect();
        assert!(
            names.contains(&"New Construction") && names.contains(&"Existing"),
            "{part}: {names:?}"
        );
        let result = RvtDocExporter
            .export_with_diagnostics(&mut rf)
            .expect("export");
        let ours = phase_values(&write_step(&result.model));
        eprintln!("{part}: Phase Created ours {ours} of {theirs}");
        assert!(ours * 100 >= theirs * 99, "{part}: {ours} of {theirs}");
    }
}

split!(snowdon_mep_elements_carry_phase_created, snowdon_mep_elements_carry_phase_created, s0 0, s1 1, s2 2);
