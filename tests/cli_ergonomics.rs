//! First-run ergonomics shared by the shipped CLIs: positional input with a
//! default output next to it, the older `--src` / `--dst` spelling, errors
//! that name the file, `--json` as shorthand for `--format json`, and a quiet
//! exit when stdout is closed early. Tier-1 synthetic fixtures only, except
//! the closed-pipe test, which needs output larger than a pipe buffer.

mod common;

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn fixture() -> Option<PathBuf> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("corpus")
        .join("tier1")
        .join("architectural-2024")
        .join("architectural-2024.rvt");
    if path.exists() {
        Some(path)
    } else {
        eprintln!("skipping: tier-1 fixture missing at {}", path.display());
        None
    }
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rvt-cli-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(bin: &str, args: &[&std::ffi::OsStr]) -> Output {
    Command::new(bin).args(args).output().expect("spawn CLI")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn gltf_and_sheet_write_next_to_a_positional_input() {
    let Some(source) = fixture() else { return };
    let dir = scratch("positional");
    let input = dir.join("model.rvt");
    std::fs::copy(&source, &input).unwrap();

    let out = run(env!("CARGO_BIN_EXE_rvt-gltf"), &[input.as_os_str()]);
    assert!(out.status.success(), "{}", stderr(&out));
    let glb = std::fs::read(dir.join("model.glb")).expect("model.glb written");
    assert_eq!(&glb[..4], b"glTF");
    assert_eq!(
        u32::from_le_bytes([glb[4], glb[5], glb[6], glb[7]]),
        2,
        "a glTF 2.0 binary container"
    );

    let out = run(env!("CARGO_BIN_EXE_rvt-sheet"), &[input.as_os_str()]);
    assert!(out.status.success(), "{}", stderr(&out));
    let svg = std::fs::read_to_string(dir.join("model.svg")).expect("model.svg written");
    assert!(svg.contains("<svg"), "{svg}");

    let custom = dir.join("plan.svg");
    let out = run(
        env!("CARGO_BIN_EXE_rvt-sheet"),
        &[input.as_os_str(), "-o".as_ref(), custom.as_os_str()],
    );
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(custom.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn legacy_src_dst_flags_still_work() {
    let Some(source) = fixture() else { return };
    let dir = scratch("legacy");
    let glb = dir.join("legacy.glb");
    let out = run(
        env!("CARGO_BIN_EXE_rvt-gltf"),
        &[
            "--src".as_ref(),
            source.as_os_str(),
            "--dst".as_ref(),
            glb.as_os_str(),
        ],
    );
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(glb.exists());

    let svg = dir.join("legacy.svg");
    let out = run(
        env!("CARGO_BIN_EXE_rvt-sheet"),
        &[
            "-s".as_ref(),
            source.as_os_str(),
            "-d".as_ref(),
            svg.as_os_str(),
        ],
    );
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(svg.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn refuses_to_overwrite_the_input() {
    let Some(source) = fixture() else { return };
    let dir = scratch("overwrite");
    let input = dir.join("model.rvt");
    std::fs::copy(&source, &input).unwrap();
    let before = std::fs::read(&input).unwrap();
    let out = run(
        env!("CARGO_BIN_EXE_rvt-gltf"),
        &[input.as_os_str(), "-o".as_ref(), input.as_os_str()],
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("refusing to overwrite"),
        "{}",
        stderr(&out)
    );
    assert_eq!(std::fs::read(&input).unwrap(), before);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_missing_input_is_named_with_one_error_prefix() {
    for bin in [
        env!("CARGO_BIN_EXE_rvt-gltf"),
        env!("CARGO_BIN_EXE_rvt-sheet"),
        env!("CARGO_BIN_EXE_rvt-ifc"),
        env!("CARGO_BIN_EXE_rvt-elements"),
        env!("CARGO_BIN_EXE_rvt-doc"),
    ] {
        let out = run(bin, &["no/such/model.rvt".as_ref()]);
        let err = stderr(&out);
        assert_eq!(out.status.code(), Some(1), "{bin}: {err}");
        assert!(err.starts_with("error: "), "{bin}: {err}");
        assert_eq!(
            err.matches("no/such/model.rvt").count(),
            1,
            "{bin} should name the path exactly once: {err}"
        );
        assert!(!err.contains("panicked"), "{bin}: {err}");
    }
}

#[test]
fn json_is_shorthand_for_format_json() {
    let Some(source) = fixture() else { return };
    let long = run(
        env!("CARGO_BIN_EXE_rvt-schema"),
        &[source.as_os_str(), "--format".as_ref(), "json".as_ref()],
    );
    let short = run(
        env!("CARGO_BIN_EXE_rvt-schema"),
        &[source.as_os_str(), "--json".as_ref()],
    );
    assert!(short.status.success(), "{}", stderr(&short));
    assert_eq!(long.stdout, short.stdout);
    let parsed: serde_json::Value = serde_json::from_slice(&short.stdout).expect("JSON");
    assert!(parsed["classes"].is_array());

    let caps = run(env!("CARGO_BIN_EXE_rvt-capabilities"), &["--json".as_ref()]);
    assert!(caps.status.success(), "{}", stderr(&caps));
    serde_json::from_slice::<serde_json::Value>(&caps.stdout).expect("capabilities JSON");

    let clash = run(
        env!("CARGO_BIN_EXE_rvt-schema"),
        &[
            source.as_os_str(),
            "--json".as_ref(),
            "-f".as_ref(),
            "text".as_ref(),
        ],
    );
    assert_eq!(
        clash.status.code(),
        Some(2),
        "--json with --format is a usage error"
    );
}

/// `rvt-schema family.rfa --json | head` closes stdout long before the
/// ~1 MB of schema JSON is written; the CLI must exit quietly instead of
/// panicking. Needs a real family file for output larger than a pipe
/// buffer.
#[test]
fn a_closed_stdout_pipe_is_a_quiet_exit() {
    let sample = common::sample_for_year(2024);
    if !sample.exists() {
        eprintln!("skipping: {} not present", sample.display());
        return;
    }
    let mut child = Command::new(env!("CARGO_BIN_EXE_rvt-schema"))
        .args([sample.as_os_str(), "--json".as_ref()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rvt-schema");
    let mut first = [0u8; 16];
    child
        .stdout
        .as_mut()
        .expect("stdout")
        .read_exact(&mut first)
        .expect("first bytes");
    drop(child.stdout.take());
    let out = child.wait_with_output().expect("wait");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!err.contains("panicked"), "{err}");
    assert!(!err.contains("Broken pipe"), "{err}");
    #[cfg(unix)]
    assert_eq!(out.status.code(), Some(141), "{err}");
}

/// `rvt-sheet --format dxf|dwg` writes the plan as a drawing that reads back: closed outlines in
/// millimetres on category layers, the same element count as the SVG draws.
#[cfg(feature = "dwg")]
#[test]
fn sheet_writes_the_plan_as_dxf_and_dwg() {
    let Some(source) = fixture() else { return };
    let dir = scratch("sheet-cad");
    let svg = dir.join("plan.svg");
    let out = run(
        env!("CARGO_BIN_EXE_rvt-sheet"),
        &[
            source.as_os_str(),
            "-o".as_ref(),
            svg.as_os_str(),
            "--no-labels".as_ref(),
        ],
    );
    assert!(out.status.success(), "{}", stderr(&out));
    let svg = std::fs::read_to_string(&svg).unwrap();
    let drawn = svg.matches("<rect").count() - 2 + svg.matches("<path").count();
    for (format, version) in [("dxf", "2018"), ("dwg", "2018"), ("dwg", "2000")] {
        let path = dir.join(format!("plan-{version}.{format}"));
        let out = run(
            env!("CARGO_BIN_EXE_rvt-sheet"),
            &[
                source.as_os_str(),
                "-o".as_ref(),
                path.as_os_str(),
                "--format".as_ref(),
                format.as_ref(),
                format!("--{format}-version").as_ref(),
                version.as_ref(),
                "--no-labels".as_ref(),
            ],
        );
        assert!(out.status.success(), "{format} {version}: {}", stderr(&out));
        let read = rvt_dwg::read_file(&path, rvt_dwg::Limits::DEFAULT).unwrap();
        assert!(read.notes.is_empty(), "{:?}", read.notes);
        assert_eq!(read.drawing.units, 4, "millimetres");
        let outlines: std::collections::BTreeSet<&str> = read
            .drawing
            .entities
            .iter()
            .map(|e| e.kind.as_str())
            .collect();
        assert!(
            outlines.iter().all(|k| *k == "LWPOLYLINE"),
            "{format} {version}: {outlines:?}"
        );
        // One outline per drawn element, plus one per hole.
        assert!(
            read.drawing.entities.len() >= drawn,
            "{format} {version}: {} outlines, {drawn} drawn in the SVG",
            read.drawing.entities.len()
        );
        if drawn > 0 {
            let category =
                |l: &&rvt_dwg::Layer2d| l.name.starts_with("A-") || l.name.starts_with("S-");
            assert!(
                read.drawing.layers.iter().any(|l| category(&l)),
                "{format} {version}"
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}
