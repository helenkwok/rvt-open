//! tools/ci/measure-reference-models.sh refuses a scorer name it cannot run.
//! A misspelt name in MEASURE_SCORERS or PROBE_SCORER ran nothing and the
//! run ended with status 0; it now stops before any export with status 2 and
//! names the bad value. "none" and "-" still name no step, and a named scorer
//! that runs and fails still writes its report and ends the run with status 1.
//! The script runs on Linux runners only (/proc/meminfo, GNU stat and date).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rvt-measure-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("models")).unwrap();
    fs::create_dir_all(dir.join("bin/examples")).unwrap();
    fs::create_dir_all(dir.join("path")).unwrap();
    // The script calls `python`; a runner may only have `python3`.
    script(&dir.join("path/python"), "#!/bin/sh\nexec python3 \"$@\"\n");
    // An existing definitions file where the script reads it
    // (RUNNER_TEMP), so the run never fetches it; its hash check fails,
    // which the script does not stop on.
    fs::write(
        dir.join("ifc4-pset-definitions.csv"),
        "not the definitions\n",
    )
    .unwrap();
    dir
}

fn script(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

fn run(dir: &Path, env: &[(&str, &str)]) -> Output {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let path = format!(
        "{}:{}",
        dir.join("path").display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut cmd = Command::new("bash");
    cmd.arg(root.join("tools/ci/measure-reference-models.sh"))
        .arg(dir.join("bin"))
        .arg(dir.join("models"))
        .arg(dir.join("out"))
        .env("PATH", path)
        .env("RUNNER_TEMP", dir)
        .env("MEASURE_JOBS", "1")
        .env_remove("MEASURE_SCORERS")
        .env_remove("PROBE")
        .env_remove("PROBE_SCORER")
        .env_remove("PROBE_SCORER_DIR")
        .env_remove("MEASURE_MODELS");
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("bash runs")
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn unknown_scorer_names_stop_the_run_with_status_2() {
    if !cfg!(target_os = "linux") {
        eprintln!("skipped: the script runs on Linux runners");
        return;
    }
    let dir = scratch("unknown");
    // A model and an exporter that records each call: a refused run exports nothing.
    fs::write(dir.join("models/2024_Core_Interior.rvt"), b"").unwrap();
    script(
        &dir.join("bin/rvt-ifc"),
        &format!(
            "#!/bin/sh\necho invoked >> '{}'\n",
            dir.join("invoked").display()
        ),
    );
    for (key, value, bad) in [
        (
            "MEASURE_SCORERS",
            "item_mesh_censu_vs_ifc",
            "item_mesh_censu_vs_ifc",
        ),
        ("MEASURE_SCORERS", "schema not_a_scorer_x", "not_a_scorer_x"),
    ] {
        let o = run(&dir, &[(key, value)]);
        assert_eq!(o.status.code(), Some(2), "{key}={value}: {}", stderr(&o));
        assert!(
            stderr(&o).contains(&format!("MEASURE_SCORERS names {bad}")),
            "{}",
            stderr(&o)
        );
        assert!(!dir.join("out/core-interior").exists());
        assert!(!dir.join("invoked").exists(), "{key}={value} exported");
    }
    let o = run(
        &dir,
        &[
            ("PROBE", "some_probe"),
            ("PROBE_SCORER", "missing_probe_scorer_x"),
        ],
    );
    assert_eq!(o.status.code(), Some(2), "{}", stderr(&o));
    assert!(
        stderr(&o).contains("PROBE_SCORER names missing_probe_scorer_x"),
        "{}",
        stderr(&o)
    );
    assert!(!dir.join("invoked").exists(), "PROBE_SCORER exported");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn none_dash_and_real_scorers_pass_the_check() {
    if !cfg!(target_os = "linux") {
        eprintln!("skipped: the script runs on Linux runners");
        return;
    }
    let dir = scratch("known");
    for value in [
        "none",
        "-",
        "item_mesh_census_vs_ifc",
        "schema info mesh_bodies_vs_ifc",
    ] {
        let o = run(&dir, &[("MEASURE_SCORERS", value)]);
        assert_eq!(
            o.status.code(),
            Some(0),
            "MEASURE_SCORERS={value}: {}",
            stderr(&o)
        );
        assert!(!stderr(&o).contains("::error::"), "{}", stderr(&o));
    }
    for value in ["-", "none", "item_mesh_census_vs_ifc"] {
        let o = run(&dir, &[("PROBE", "some_probe"), ("PROBE_SCORER", value)]);
        assert_eq!(
            o.status.code(),
            Some(0),
            "PROBE_SCORER={value}: {}",
            stderr(&o)
        );
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_named_scorer_that_fails_writes_its_report_and_ends_with_status_1() {
    if !cfg!(target_os = "linux") {
        eprintln!("skipped: the script runs on Linux runners");
        return;
    }
    let dir = scratch("fails");
    fs::write(dir.join("models/RE1-Architecture.rvt"), b"").unwrap();
    fs::write(dir.join("models/RE1-Architecture.ifc"), b"").unwrap();
    // The same recording exporter as the refusal test: an accepted run calls it.
    script(
        &dir.join("bin/rvt-ifc"),
        &format!(
            "#!/bin/sh\necho invoked >> '{}'\n",
            dir.join("invoked").display()
        ),
    );
    script(
        &dir.join("bin/examples/fake_probe"),
        "#!/bin/sh\necho probe output\n",
    );
    fs::create_dir_all(dir.join("scorers")).unwrap();
    fs::write(
        dir.join("scorers/fails.py"),
        "print('the report')\nraise SystemExit(1)\n",
    )
    .unwrap();
    fs::write(dir.join("scorers/passes.py"), "print('the report')\n").unwrap();
    let scorers = dir.join("scorers");
    for (scorer, code) in [("fails", 1), ("passes", 0)] {
        let o = run(
            &dir,
            &[
                ("MEASURE_SCORERS", "none"),
                ("MEASURE_MODELS", "re1-architecture"),
                ("PROBE", "fake_probe"),
                ("PROBE_SCORER", scorer),
                ("PROBE_SCORER_DIR", scorers.to_str().unwrap()),
            ],
        );
        assert_eq!(o.status.code(), Some(code), "{scorer}: {}", stderr(&o));
        let report = fs::read_to_string(dir.join("out/re1-architecture/probe_score.txt")).unwrap();
        assert!(
            report.contains("the report") && report.contains(&format!("exit {code}")),
            "{report}"
        );
        let failures = fs::read_to_string(dir.join("out/scorer-failures.txt")).unwrap_or_default();
        assert_eq!(failures.contains("probe_score"), code == 1, "{failures}");
        assert!(dir.join("invoked").exists(), "{scorer}: no export ran");
    }
    let _ = fs::remove_dir_all(&dir);
}
