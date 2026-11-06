//! The GlobalId namespace is a user option: the GlobalIds the
//! writer generates are derived under `--guid-namespace` (default
//! `rvt-open ifc GlobalId v1`), and Revit's own GlobalIds never change.
//! Runs `rvt-ifc` on the tier-1 synthetic project.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

const OTHER: &str = "another ifc GlobalId v1";

fn export(args: &[&str], env: &[(&str, &str)]) -> (bool, BTreeSet<String>) {
    let input = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("corpus/tier1/architectural-2024/architectural-2024.rvt");
    let dir = std::env::temp_dir().join(format!(
        "guid-ns-{}-{}",
        std::process::id(),
        args.join("_").len()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("out.ifc");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rvt-ifc"));
    cmd.arg(&input)
        .arg("-o")
        .arg(&out)
        .args(args)
        .env("SOURCE_DATE_EPOCH", "1700000000")
        .env_remove("RVT_OPEN_GUID_NAMESPACE");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let status = cmd.output().expect("spawn rvt-ifc").status;
    let ids = std::fs::read_to_string(&out)
        .map(|s| global_ids(&s))
        .unwrap_or_default();
    let _ = std::fs::remove_dir_all(&dir);
    (status.success(), ids)
}

/// The first attribute of every `IFC...('<22 characters>'` line.
fn global_ids(step: &str) -> BTreeSet<String> {
    step.lines()
        .filter_map(|l| {
            let (_, rest) = l.split_once("=IFC")?;
            let (_, args) = rest.split_once("('")?;
            let id = args.get(..22)?;
            (args.as_bytes().get(22) == Some(&b'\'')).then(|| id.to_string())
        })
        .collect()
}

#[test]
fn namespace_changes_only_generated_ids() {
    let (ok, default) = export(&[], &[]);
    assert!(ok && !default.is_empty());
    let (_, custom) = export(&["--guid-namespace", "my-company ids"], &[]);
    let (_, custom_again) = export(&["--guid-namespace", "my-company ids"], &[]);
    let (_, other) = export(&["--guid-namespace", OTHER], &[]);
    // The default is not the other namespace, a custom one is neither, and a
    // namespace gives the same ids on every run.
    assert_ne!(default, other, "default GlobalId namespace is rvt-open's");
    assert_ne!(default, custom);
    assert_ne!(other, custom);
    assert_eq!(custom, custom_again);
    // What a namespace does not touch (Revit's own GlobalIds) is the same set
    // under every namespace, and it is not everything.
    let kept: BTreeSet<_> = default.intersection(&custom).cloned().collect();
    let kept_other: BTreeSet<_> = default.intersection(&other).cloned().collect();
    assert_eq!(kept, kept_other);
    assert!(kept.len() < default.len(), "some ids are generated");
}

#[test]
fn a_namespace_reproduces_fixed_output() {
    let golden = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/guid-namespace/other-ids.txt"),
    )
    .unwrap();
    let expected: BTreeSet<String> = golden.lines().map(str::to_string).collect();
    let (ok, other) = export(&["--guid-namespace", OTHER], &[]);
    assert!(ok);
    assert_eq!(other, expected);
}

#[test]
fn env_var_sets_the_namespace_and_bad_values_are_refused() {
    let (_, flag) = export(&["--guid-namespace", "from env"], &[]);
    let (ok, env) = export(&[], &[("RVT_OPEN_GUID_NAMESPACE", "from env")]);
    assert!(ok);
    assert_eq!(flag, env);
    assert!(!export(&["--guid-namespace", ""], &[]).0, "empty refused");
    let long = "x".repeat(257);
    assert!(
        !export(&["--guid-namespace", &long], &[]).0,
        "over 256 bytes refused"
    );
}
