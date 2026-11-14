//! The six shipped CLIs that only `--help` was tested for, run end to
//! end on real Revit files.
//!
//! Each test runs the built binary and checks what the tool's own
//! documentation promises, against facts the files state independently of
//! the tool (the release a family corpus file's name gives, the stream list,
//! an unchanged export):
//!
//! - `rvt-elem-table --json`: the parsed records number the table's
//!   `record_count`, their ids rise strictly, and every owner is a declared id;
//! - `rvt-history --json`: the releases named rise and end with the file's own;
//! - `rvt-analyze --json`: the identity's release is the file's, and with
//!   `--redact` no Windows user folder, Autodesk OneDrive path or internal
//!   project-id folder survives, where the reports without it name some;
//! - `rvt-doc --json`: a file run from under a user folder and an Autodesk
//!   OneDrive folder prints both in its path, and with `--redact` neither
//!   survives anywhere in the output while the fields and notes are unchanged;
//! - `rvt-diff`: a file against itself has every stream identical, and two
//!   releases of the same family differ in some stream;
//! - `rvt-ifc-compare --fail-on-diff`: an export against itself exits 0 with
//!   no entity count delta, and against Revit's own export exits 2;
//!
//! Runs against `RVT_SAMPLES_DIR` (the phi-ag family corpus) and
//! `RVT_PROJECT_CORPUS_DIR`. Skips what is absent.

mod common;

use common::{ALL_YEARS, sample_for_year};
use serde_json::Value;
use std::path::PathBuf;
use std::process::{Command, Output};

fn run(bin: &str, args: &[&std::ffi::OsStr]) -> Output {
    Command::new(bin).args(args).output().expect("run binary")
}

fn ok(bin: &str, output: &Output) -> String {
    assert!(
        output.status.success(),
        "{bin} failed ({:?})\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn families() -> Vec<(u32, PathBuf)> {
    ALL_YEARS
        .iter()
        .map(|&year| (year, sample_for_year(year)))
        .filter(|(_, path)| path.exists())
        .collect()
}

fn project(name: &str) -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var_os("RVT_PROJECT_CORPUS_DIR")?).join(name);
    path.exists().then_some(path)
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rvt-open-cli-real-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Every four-digit release year (2000 to 2099) in `text`, in order.
fn years(text: &str) -> Vec<u32> {
    let bytes = text.as_bytes();
    (0..bytes.len().saturating_sub(3))
        .filter(|&i| {
            bytes[i] == b'2'
                && bytes[i + 1] == b'0'
                && bytes[i + 2..i + 4].iter().all(u8::is_ascii_digit)
                && (i == 0 || !bytes[i - 1].is_ascii_digit())
                && bytes.get(i + 4).is_none_or(|b| !b.is_ascii_digit())
        })
        .filter_map(|i| text[i..i + 4].parse().ok())
        .collect()
}

#[test]
fn elem_table_json_holds_the_tables_records() {
    let mut files: Vec<PathBuf> = families().into_iter().map(|(_, p)| p).collect();
    files.extend(project("Revit_IFC5_Einhoven.rvt"));
    files.extend(project("2024_Core_Interior.rvt"));
    if files.is_empty() {
        eprintln!("skipping: no corpus");
        return;
    }
    for file in &files {
        let out = ok(
            "rvt-elem-table",
            &run(
                env!("CARGO_BIN_EXE_rvt-elem-table"),
                &[file.as_os_str(), "--json".as_ref()],
            ),
        );
        let json: Value = serde_json::from_str(&out).expect("rvt-elem-table JSON");
        let name = file.file_name().unwrap_or_default().to_string_lossy();
        let records = json["records"].as_array().expect("records");
        assert_eq!(
            json["parsed_records"].as_u64(),
            json["record_count"].as_u64(),
            "{name}: parsed records against the table's record_count"
        );
        assert_eq!(
            records.len() as u64,
            json["parsed_records"].as_u64().unwrap_or(0),
            "{name}"
        );
        let ids: Vec<u64> = records
            .iter()
            .filter_map(|r| r["id_primary"].as_u64())
            .collect();
        assert!(
            ids.windows(2).all(|w| w[0] < w[1]),
            "{name}: record ids do not rise strictly"
        );
        let declared: std::collections::BTreeSet<u64> = ids.iter().copied().collect();
        let stray: Vec<u64> = records
            .iter()
            .filter_map(|r| r["owner_id"].as_u64())
            .filter(|owner| !declared.contains(owner))
            .take(5)
            .collect();
        assert!(stray.is_empty(), "{name}: owners not declared: {stray:?}");
    }
}

#[test]
fn history_ends_with_the_files_own_release() {
    let files = families();
    if files.is_empty() {
        eprintln!("skipping: family corpus missing");
        return;
    }
    for (year, file) in &files {
        let out = ok(
            "rvt-history",
            &run(
                env!("CARGO_BIN_EXE_rvt-history"),
                &[file.as_os_str(), "--json".as_ref()],
            ),
        );
        let json: Value = serde_json::from_str(&out).expect("rvt-history JSON");
        let entries: Vec<String> = json["entries"]
            .as_array()
            .expect("entries")
            .iter()
            .filter_map(|e| e.as_str().map(String::from))
            .collect();
        let releases: Vec<u32> = entries
            .iter()
            .filter_map(|e| years(e).first().copied())
            .collect();
        assert!(!releases.is_empty(), "{year}: no release in {entries:?}");
        assert!(
            releases.windows(2).all(|w| w[0] <= w[1]),
            "{year}: releases out of order: {releases:?}"
        );
        assert_eq!(releases.last(), Some(year), "{year}: history {entries:?}");
    }
}

#[test]
fn analyze_identity_is_the_files_release() {
    let files = families();
    if files.is_empty() {
        eprintln!("skipping: family corpus missing");
        return;
    }
    for (year, file) in &files {
        let out = ok(
            "rvt-analyze",
            &run(
                env!("CARGO_BIN_EXE_rvt-analyze"),
                &[file.as_os_str(), "--json".as_ref()],
            ),
        );
        let json: Value = serde_json::from_str(&out).expect("rvt-analyze JSON");
        assert_eq!(
            json["identity"]["revit_version"].as_u64(),
            Some(u64::from(*year)),
            "{year}: rvt-analyze identity {}",
            json["identity"]
        );
    }
}

#[test]
fn diff_finds_a_file_identical_to_itself_and_two_releases_different() {
    let files = families();
    let (Some((_, a)), Some((_, b))) = (files.first(), files.last()) else {
        eprintln!("skipping: family corpus missing");
        return;
    };
    if a == b {
        eprintln!("skipping: one family file");
        return;
    }
    let same = ok(
        "rvt-diff",
        &run(
            env!("CARGO_BIN_EXE_rvt-diff"),
            &[a.as_os_str(), a.as_os_str()],
        ),
    );
    let byte_level = same
        .split_once("=== byte-level diff")
        .map(|(_, rest)| rest)
        .expect("byte-level section");
    let lines: Vec<&str> = byte_level
        .lines()
        .skip(1)
        .filter(|l| l.starts_with("  ") && !l.starts_with("   "))
        .collect();
    assert!(!lines.is_empty(), "no stream compared:\n{same}");
    assert!(
        lines.iter().all(|l| l.trim_end().ends_with("IDENTICAL")),
        "a file differs from itself:\n{byte_level}"
    );
    let differ = ok(
        "rvt-diff",
        &run(
            env!("CARGO_BIN_EXE_rvt-diff"),
            &[a.as_os_str(), b.as_os_str()],
        ),
    );
    let byte_level = differ
        .split_once("=== byte-level diff")
        .map(|(_, rest)| rest)
        .expect("byte-level section");
    assert!(
        byte_level.lines().any(|l| l.contains("Δbytes=")),
        "two releases show no differing stream:\n{byte_level}"
    );
}

#[test]
fn ifc_compare_tells_an_export_from_itself_and_from_revits() {
    let Some(rvt) = project("RE1-Architecture.rvt") else {
        eprintln!("skipping: RE1-Architecture.rvt absent");
        return;
    };
    let revit_ifc = rvt.with_extension("ifc");
    let dir = scratch("ifc-compare");
    let ours = dir.join("ours.ifc");
    ok(
        "rvt-ifc",
        &run(
            env!("CARGO_BIN_EXE_rvt-ifc"),
            &[rvt.as_os_str(), "-o".as_ref(), ours.as_os_str()],
        ),
    );
    let report = dir.join("self.json");
    let itself = run(
        env!("CARGO_BIN_EXE_rvt-ifc-compare"),
        &[
            ours.as_os_str(),
            ours.as_os_str(),
            "--fail-on-diff".as_ref(),
            "--quiet".as_ref(),
            "--json".as_ref(),
            report.as_os_str(),
        ],
    );
    assert_eq!(itself.status.code(), Some(0), "an export against itself");
    let json: Value =
        serde_json::from_slice(&std::fs::read(&report).expect("report")).expect("report JSON");
    assert_eq!(
        json["entity_count_deltas"]
            .as_object()
            .map(|deltas| deltas.len()),
        Some(0),
        "entity count deltas against itself: {}",
        json["entity_count_deltas"]
    );
    if revit_ifc.exists() {
        let against = dir.join("against-revit.json");
        let against_revit = run(
            env!("CARGO_BIN_EXE_rvt-ifc-compare"),
            &[
                ours.as_os_str(),
                revit_ifc.as_os_str(),
                "--fail-on-diff".as_ref(),
                "--quiet".as_ref(),
                "--json".as_ref(),
                against.as_os_str(),
            ],
        );
        assert_eq!(
            against_revit.status.code(),
            Some(2),
            "rvt-open's IFC4 export against Revit's IFC2X3 export differs structurally"
        );
        // rvt-open writes metres and Revit's export of RE1 millimetres: once
        // both are read in metres, RE1's storeys are the same on both sides.
        let json: Value =
            serde_json::from_slice(&std::fs::read(&against).expect("report")).expect("report JSON");
        for side in ["storeys_only_left", "storeys_only_right"] {
            assert_eq!(
                json[side].as_array().map(Vec::len),
                Some(0),
                "{side} against Revit's export: {}",
                json[side]
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// `rvt-ifc --phase` names the phase the export shows. RE1
/// Architecture's phases are Existing (12589) and New Construction (86961),
/// and its 11 rooms are in the second, the last.
#[test]
fn ifc_phase_option_chooses_the_export_phase_of_a_real_file() {
    let Some(rvt) = project("RE1-Architecture.rvt") else {
        eprintln!("skipping: RE1-Architecture.rvt absent");
        return;
    };
    let dir = scratch("ifc-phase");
    let export = |tag: &str, phase: Option<&str>| {
        let ifc = dir.join(format!("{tag}.ifc"));
        let diagnostics = dir.join(format!("{tag}.json"));
        let mut command = Command::new(env!("CARGO_BIN_EXE_rvt-ifc"));
        command
            .env("SOURCE_DATE_EPOCH", "0")
            .arg(&rvt)
            .arg("-o")
            .arg(&ifc)
            .arg("--diagnostics")
            .arg(&diagnostics);
        if let Some(phase) = phase {
            command.arg("--phase").arg(phase);
        }
        (command.output().expect("run rvt-ifc"), ifc, diagnostics)
    };
    let read_json = |path: &PathBuf| -> Value {
        serde_json::from_slice(&std::fs::read(path).expect("diagnostics"))
            .expect("diagnostics JSON")
    };
    let spaces = |json: &Value| json["exported"]["by_ifc_type"]["IFCSPACE"].as_u64();
    let room_items = |json: &Value| -> Vec<Value> {
        json["skipped"]
            .as_array()
            .expect("skipped")
            .iter()
            .filter(|item| item["reason"] == "room_not_in_export_phase")
            .cloned()
            .collect()
    };

    // The default is the last phase: every room is written and none is counted.
    let (output, default_ifc, default_json) = export("default", None);
    ok("rvt-ifc", &output);
    let default_diagnostics = read_json(&default_json);
    assert_eq!(spaces(&default_diagnostics), Some(11), "rooms by default");
    assert!(room_items(&default_diagnostics).is_empty());
    let default_bytes = std::fs::read(&default_ifc).expect("default IFC");

    // The last phase named, by its name and by its ElementId, is the default.
    for (tag, phase) in [("by-name", "New Construction"), ("by-id", "86961")] {
        let (output, ifc, _) = export(tag, Some(phase));
        ok("rvt-ifc", &output);
        assert_eq!(
            std::fs::read(&ifc).expect("IFC"),
            default_bytes,
            "--phase {phase} is the default export"
        );
    }

    // The first phase leaves out the 11 rooms, which sit in the second, and
    // counts them by it.
    let (output, ifc, diagnostics) = export("existing", Some("Existing"));
    ok("rvt-ifc", &output);
    assert!(ifc.exists());
    let json = read_json(&diagnostics);
    assert_eq!(
        spaces(&json).unwrap_or(0),
        0,
        "rooms written for the first phase"
    );
    let items = room_items(&json);
    assert_eq!(items.len(), 1, "room items: {items:?}");
    assert_eq!(items[0]["count"], 11);
    assert_eq!(items[0]["classes"]["Room"], 11);
    assert_eq!(items[0]["phase"], "New Construction");

    // A phase the file does not list is an error that lists its phases, and
    // nothing is written.
    let (output, ifc, _) = export("unknown", Some("No such phase"));
    assert_eq!(output.status.code(), Some(1), "an unknown phase");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Existing") && stderr.contains("New Construction"),
        "the error lists the file's phases: {stderr}"
    );
    assert!(!ifc.exists(), "no IFC for an unknown phase");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Every string in a JSON document.
fn strings(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(s) => out.push(s.clone()),
        Value::Array(items) => items.iter().for_each(|v| strings(v, out)),
        Value::Object(map) => map.values().for_each(|v| strings(v, out)),
        _ => {}
    }
}

#[test]
fn analyze_redact_leaves_no_user_folder_or_autodesk_internal_path() {
    let files = families();
    if files.is_empty() {
        eprintln!("skipping: family corpus missing");
        return;
    }
    let bin = env!("CARGO_BIN_EXE_rvt-analyze");
    let mut to_scrub = 0;
    for (year, path) in &files {
        let report = |redact: bool| -> Vec<String> {
            let mut args = vec![path.as_os_str(), "--json".as_ref()];
            if redact {
                args.push("--redact".as_ref());
            }
            let json: Value = serde_json::from_str(&ok(bin, &run(bin, &args))).expect("JSON");
            let mut out = Vec::new();
            strings(&json, &mut out);
            out
        };
        to_scrub += report(false)
            .iter()
            .filter(|s| {
                s.contains("\\Users\\")
                    || s.contains("/Users/")
                    || s.contains("OneDrive - Autodesk")
            })
            .count();
        for s in report(true) {
            for folder in ["\\Users\\", "/Users/"] {
                for (i, _) in s.match_indices(folder) {
                    assert!(
                        s[i + folder.len()..].starts_with("<redacted>"),
                        "{year}: a user folder survives --redact: {s}"
                    );
                }
            }
            assert!(
                !s.contains("OneDrive - Autodesk"),
                "{year}: an Autodesk OneDrive path survives --redact: {s}"
            );
            for (i, _) in s.match_indices("Revit - ") {
                assert!(
                    !s[i + "Revit - ".len()..].starts_with(|c: char| c.is_ascii_digit()),
                    "{year}: a project-id folder survives --redact: {s}"
                );
            }
        }
    }
    // The 2024 sample's report names the user folder and the Autodesk
    // OneDrive path it was authored in (docs/demo/rvt-analyze-2024-redacted.json).
    assert!(
        to_scrub > 0,
        "no report named a user folder, so --redact was not exercised"
    );
    eprintln!("rvt-analyze --redact: {to_scrub} strings with a user or internal path scrubbed");
}

#[test]
fn doc_redact_scrubs_the_path_given_and_changes_nothing_else() {
    let files = families();
    if files.is_empty() {
        eprintln!("skipping: family corpus missing");
        return;
    }
    let bin = env!("CARGO_BIN_EXE_rvt-doc");
    // rvt-doc prints the path it was given, so each file is run from under
    // the folders `--redact` is promised to scrub.
    let dir = scratch("doc-redact");
    let home = dir.join("Users").join("alice").join("OneDrive - Autodesk");
    std::fs::create_dir_all(&home).expect("create the synthetic home folder");
    for (year, path) in &files {
        let copy = home.join(path.file_name().expect("file name"));
        std::fs::copy(path, &copy).expect("copy the sample");
        let doc = |redact: bool| -> Value {
            let mut args = vec![copy.as_os_str(), "--json".as_ref()];
            if redact {
                args.push("--redact".as_ref());
            }
            serde_json::from_str(&ok(bin, &run(bin, &args))).expect("JSON")
        };
        let (plain, redacted) = (doc(false), doc(true));
        let named = plain["path"].as_str().expect("path");
        assert!(
            named.contains("alice") && named.contains("OneDrive - Autodesk"),
            "{year}: the output does not name the folders to scrub: {named}"
        );
        let mut seen = Vec::new();
        strings(&redacted, &mut seen);
        for s in &seen {
            for leaked in ["alice", "OneDrive"] {
                assert!(
                    !s.contains(leaked),
                    "{year}: {leaked} survives --redact: {s}"
                );
            }
        }
        // Scrubbing takes nothing else out of the output.
        for key in ["version", "adocument_entry_offset", "fields", "notes"] {
            assert_eq!(redacted[key], plain[key], "{year}: --redact changed {key}");
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}
