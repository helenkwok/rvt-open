//! `rvt-dwg`: read DWG/DXF drawings and write their tables, SVG, DXF, DWG and report.json; score
//! one drawing against another.
//!
//! ```bash
//! rvt-dwg plan.dwg                                  # CSV, JSONL, DXF, SVG and report.json next to it
//! rvt-dwg drawings/ -o out --format csv,svg,report  # every .dwg/.dxf under drawings/
//! rvt-dwg sheet.dwg --all-layouts --format svg      # one SVG per layout
//! rvt-dwg plan.dwg --format dwg --dwg-version 2004  # re-save as DWG 2004
//! rvt-dwg compare reference.dwg candidate.dxf --tolerance 0.5 -o score
//! ```
//!
//! Each file is read in a child copy of this program, so a file that crashes the reader (an
//! allocation failure or stack overflow cannot be caught in-process) costs that file only.

use clap::{Parser, Subcommand};
use rvt_dwg::write::{CadFormat, parse_version};
use rvt_dwg::{Limits, compare, report::Report, svg, tables};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

/// How long one file may take in the worker before it is stopped.
const WORKER_TIMEOUT: Duration = Duration::from_secs(30 * 60);

#[derive(Parser, Debug)]
#[command(
    name = "rvt-dwg",
    version,
    about = "Read DWG/DXF drawings: entity, summary, layer and block tables (CSV, JSONL), SVG, DXF, DWG and report.json",
    args_conflicts_with_subcommands = true,
    subcommand_negates_reqs = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,

    /// Drawings (.dwg, .dxf) or folders of them.
    #[arg(value_name = "INPUT", required = true)]
    inputs: Vec<PathBuf>,

    /// Output folder. Default: next to each input.
    #[arg(short = 'o', long = "output")]
    output: Option<PathBuf>,

    /// Outputs, comma separated: csv, jsonl, dxf, dwg, svg, report, xlsx.
    #[arg(
        long,
        value_delimiter = ',',
        default_value = "csv,jsonl,dxf,svg,report"
    )]
    format: Vec<String>,

    /// SVG of this layout instead of model space.
    #[arg(long, conflicts_with = "all_layouts")]
    layout: Option<String>,

    /// One SVG per layout (model space and every sheet).
    #[arg(long)]
    all_layouts: bool,

    /// In the tables, also a row for every entity drawn through an insert, placed where it is
    /// drawn.
    #[arg(long)]
    explode_blocks: bool,

    /// DXF version: 2018, 2013, 2010, 2007, 2004, 2000, R14 or R12.
    #[arg(long, default_value = "2018")]
    dxf_version: String,

    /// DWG version: 2018, 2013, 2010, 2007, 2004 or 2000.
    #[arg(long, default_value = "2018")]
    dwg_version: String,

    /// Read in this process instead of a child worker.
    #[arg(long)]
    no_worker: bool,

    /// Internal: this process is the worker for one file.
    #[arg(long, hide = true)]
    worker: bool,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Score a candidate drawing against a reference: sheets, linework per layer, text, hatches,
    /// dimensions. Writes score.json and score.tsv.
    Compare {
        reference: PathBuf,
        candidate: PathBuf,
        /// Match distance in reference units.
        #[arg(long, default_value_t = 0.5)]
        tolerance: f64,
        /// Compare this layout's paper space instead of model space.
        #[arg(long)]
        layout: Option<String>,
        /// Tab-separated reference-layer, candidate-layer pairs.
        #[arg(long)]
        layer_map: Option<PathBuf>,
        /// Output folder for score.json and score.tsv. Default: print the TSV.
        #[arg(short = 'o', long = "output")]
        output: Option<PathBuf>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match &cli.command {
        Some(Cmd::Compare {
            reference,
            candidate,
            tolerance,
            layout,
            layer_map,
            output,
        }) => run_compare(
            reference,
            candidate,
            *tolerance,
            layout.clone(),
            layer_map.as_deref(),
            output.as_deref(),
        ),
        None => run(&cli),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(1)
        }
    }
}

const FORMATS: &[&str] = &["csv", "jsonl", "dxf", "dwg", "svg", "report", "xlsx"];

fn run(cli: &Cli) -> Result<(), String> {
    for f in &cli.format {
        if !FORMATS.contains(&f.as_str()) {
            return Err(format!("unknown format {f:?}: use {}", FORMATS.join(", ")));
        }
    }
    parse_version(&cli.dxf_version, CadFormat::Dxf).map_err(|e| e.0)?;
    parse_version(&cli.dwg_version, CadFormat::Dwg).map_err(|e| e.0)?;
    let mut files = Vec::new();
    for input in &cli.inputs {
        collect(input, input, &mut files)?;
    }
    if files.is_empty() {
        return Err("no .dwg or .dxf files found".into());
    }
    let mut failed = 0;
    for (root, file) in &files {
        let out_dir = out_dir(cli.output.as_deref(), root, file);
        let r = if cli.worker || cli.no_worker {
            convert(cli, file, &out_dir)
        } else {
            in_worker(cli, file, &out_dir)
        };
        if let Err(e) = r {
            eprintln!("{}: {e}", file.display());
            failed += 1;
        }
    }
    if failed > 0 {
        return Err(format!("{failed} of {} files failed", files.len()));
    }
    Ok(())
}

/// Every drawing under `path`, with the folder it was found from.
fn collect(root: &Path, path: &Path, out: &mut Vec<(PathBuf, PathBuf)>) -> Result<(), String> {
    if path.is_dir() {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(path)
            .map_err(|e| format!("{}: {e}", path.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .collect();
        entries.sort();
        for p in entries {
            if p.is_dir() || rvt_dwg::is_cad_path(&p) {
                collect(root, &p, out)?;
            }
        }
    } else if path.exists() {
        out.push((root.to_path_buf(), path.to_path_buf()));
    } else {
        return Err(format!("{}: no such file", path.display()));
    }
    Ok(())
}

/// The output folder for `file`: under `-o`, keeping the file's place below the input folder.
fn out_dir(output: Option<&Path>, root: &Path, file: &Path) -> PathBuf {
    let parent = file.parent().unwrap_or(Path::new("."));
    match output {
        None => parent.to_path_buf(),
        Some(o) if root.is_dir() => o.join(parent.strip_prefix(root).unwrap_or(Path::new(""))),
        Some(o) => o.to_path_buf(),
    }
}

fn in_worker(cli: &Cli, file: &Path, out_dir: &Path) -> Result<(), String> {
    let Ok(exe) = std::env::current_exe() else {
        return convert(cli, file, out_dir);
    };
    let mut args: Vec<std::ffi::OsString> = vec![
        "--worker".into(),
        "-o".into(),
        out_dir.into(),
        "--format".into(),
        cli.format.join(",").into(),
    ];
    if let Some(l) = &cli.layout {
        args.extend(["--layout".into(), l.into()]);
    }
    if cli.all_layouts {
        args.push("--all-layouts".into());
    }
    if cli.explode_blocks {
        args.push("--explode-blocks".into());
    }
    args.extend([
        "--dxf-version".into(),
        cli.dxf_version.clone().into(),
        "--dwg-version".into(),
        cli.dwg_version.clone().into(),
    ]);
    args.push(file.into());
    let mut child = match Command::new(exe).args(args).stdin(Stdio::null()).spawn() {
        Ok(c) => c,
        Err(_) => return convert(cli, file, out_dir),
    };
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            // The worker printed its own error for a file it could read and report on.
            Ok(Some(status)) if status.code() == Some(1) => return Err("conversion failed".into()),
            Ok(Some(status)) => return Err(format!("the reader stopped on this file ({status})")),
            Ok(None) if start.elapsed() > WORKER_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "stopped after {} minutes",
                    WORKER_TIMEOUT.as_secs() / 60
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(e.to_string()),
        }
    }
}

fn write(path: &Path, bytes: &[u8], outputs: &mut Vec<String>) -> Result<(), String> {
    std::fs::write(path, bytes).map_err(|e| format!("write {}: {e}", path.display()))?;
    outputs.push(
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
    );
    Ok(())
}

/// A file-name-safe form of a layout name.
fn safe(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || "-_ ".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn convert(cli: &Cli, file: &Path, out_dir: &Path) -> Result<(), String> {
    let limits = Limits::DEFAULT;
    let t0 = Instant::now();
    let bytes = std::fs::read(file).map_err(|e| e.to_string())?;
    let read = rvt_dwg::read_bytes(&bytes, limits).map_err(|e| e.0)?;
    let mut timings = vec![("read", t0.elapsed())];
    std::fs::create_dir_all(out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))?;
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = file
        .file_stem()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "drawing".into());
    let mut report = Report::new(&name, &bytes, &read.drawing, read.notes.clone());
    let mut outputs = Vec::new();
    let want = |f: &str| cli.format.iter().any(|x| x == f);
    let path = |suffix: &str| out_dir.join(format!("{stem}{suffix}"));

    if want("csv") || want("jsonl") || want("xlsx") {
        let t = Instant::now();
        let sheets = tables::sheets(
            &read.drawing,
            &tables::TableOptions {
                explode_blocks: cli.explode_blocks,
                limits,
            },
        );
        if want("csv") {
            for s in &sheets {
                write(
                    &path(&format!(".{}.csv", s.name.to_ascii_lowercase())),
                    tables::write_csv(s).as_bytes(),
                    &mut outputs,
                )?;
            }
        }
        if want("jsonl") {
            write(
                &path(".entities.jsonl"),
                tables::entities_jsonl(&sheets).as_bytes(),
                &mut outputs,
            )?;
        }
        if want("xlsx") {
            // The workbook writer takes the same four sheets; it is not in this build yet.
            report
                .skipped
                .entry("xlsx".into())
                .or_default()
                .push("the workbook writer is not in this build: use csv".into());
            eprintln!("{name}: xlsx is not in this build; the same tables are written as csv");
        }
        timings.push(("tables", t.elapsed()));
    }
    if want("svg") {
        let t = Instant::now();
        let layouts: Vec<Option<String>> = if cli.all_layouts {
            read.drawing
                .layouts
                .iter()
                .map(|l| (!l.is_model).then(|| l.name.clone()))
                .collect()
        } else {
            vec![cli.layout.clone()]
        };
        for layout in layouts {
            let s = svg::render(&read.drawing, layout.as_deref(), &limits).map_err(|e| e.0)?;
            let suffix = match (&layout, cli.all_layouts) {
                (Some(l), _) => format!(".{}.svg", safe(l)),
                (None, true) => ".Model.svg".into(),
                (None, false) => ".svg".into(),
            };
            write(&path(&suffix), s.svg.as_bytes(), &mut outputs)?;
            for (k, n) in s.not_drawn {
                *report.not_drawn.entry(k).or_default() += n;
            }
            if s.truncated {
                report
                    .skipped
                    .entry("svg".into())
                    .or_default()
                    .push(format!(
                        "{suffix}: an insert depth or count limit cut the drawing short"
                    ));
            }
        }
        timings.push(("svg", t.elapsed()));
    }
    for (format, flag, version, ext) in [
        (CadFormat::Dxf, "dxf", &cli.dxf_version, "dxf".to_string()),
        (
            CadFormat::Dwg,
            "dwg",
            &cli.dwg_version,
            format!("{}.dwg", cli.dwg_version),
        ),
    ] {
        if !want(flag) {
            continue;
        }
        let t = Instant::now();
        let v = parse_version(version, format).map_err(|e| e.0)?;
        let (b, notes) =
            rvt_dwg::write::resave(&read, format, v).map_err(|e| format!("{flag}: {e}"))?;
        write(&path(&format!(".{ext}")), &b, &mut outputs)?;
        if !notes.is_empty() {
            report.skipped.entry(flag.into()).or_default().extend(notes);
        }
        timings.push((flag, t.elapsed()));
    }
    report.outputs = outputs.clone();
    report.timings_ms = timings
        .into_iter()
        .map(|(k, d)| (k.to_string(), d.as_millis() as u64))
        .collect();
    if want("report") {
        write(
            &path(".report.json"),
            report.to_json().as_bytes(),
            &mut outputs,
        )?;
    }
    println!(
        "{name}: {} {} entities, {} layers, {} layouts; {} notes -> {}",
        read.drawing.version,
        read.drawing.entities.len(),
        read.drawing.layers.len(),
        read.drawing.layouts.len(),
        read.notes.len(),
        outputs.join(" ")
    );
    Ok(())
}

fn run_compare(
    reference: &Path,
    candidate: &Path,
    tolerance: f64,
    layout: Option<String>,
    layer_map: Option<&Path>,
    output: Option<&Path>,
) -> Result<(), String> {
    let read = |p: &Path| {
        rvt_dwg::read_file(p, Limits::DEFAULT).map_err(|e| format!("{}: {e}", p.display()))
    };
    let (r, c) = (read(reference)?, read(candidate)?);
    let mut map = HashMap::new();
    if let Some(p) = layer_map {
        let text = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
        for line in text
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        {
            let (a, b) = line.split_once('\t').ok_or_else(|| {
                format!(
                    "{}: expected reference<TAB>candidate: {line:?}",
                    p.display()
                )
            })?;
            map.insert(a.trim().to_string(), b.trim().to_string());
        }
    }
    let options = compare::Options {
        tolerance,
        layout,
        layer_map: map,
        ..Default::default()
    };
    let score = compare::compare(&r.drawing, &c.drawing, &options);
    match output {
        Some(dir) => {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            let json = serde_json::to_string_pretty(&score).map_err(|e| e.to_string())?;
            std::fs::write(dir.join("score.json"), json + "\n").map_err(|e| e.to_string())?;
            std::fs::write(dir.join("score.tsv"), score.to_tsv()).map_err(|e| e.to_string())?;
            println!(
                "linework f1 {:.4}, text f1 {:.4} -> {}",
                score.linework.f1,
                score.text.f1,
                dir.display()
            );
        }
        None => print!("{}", score.to_tsv()),
    }
    Ok(())
}
