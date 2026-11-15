//! `rvt-batch` — convert every Revit file in a folder tree.
//!
//! Each input file gets its own output folder, named after the file and
//! placed at the same relative path under the output folder
//! (`in/a/b/model.rvt` gives `out/a/b/model.rvt/`), holding the outputs asked
//! for, `report.json` (`rvt::ifc::run_report`) and, written last,
//! `manifest.json` (every file with its size and SHA-256, and the decode
//! limits). Every file is written whole or not at all (written beside its
//! name, then renamed). An output folder that already exists is never written
//! into: its file is skipped, so a re-run converts only what is new; a folder
//! a crash left unfinished (it still holds `.incomplete`) is converted again.
//! One JSON status line per file goes to stdout.
//!
//! ```bash
//! rvt-batch projects/ -o converted/
//! rvt-batch projects/ -o converted/ --formats ifc,table,glb,obj --ifc-units mm
//! rvt-batch projects/ -o converted/ --formats xlsx,parquet,sqlite
//! rvt-batch inbox/ -o converted/ --watch      # keep converting new files
//! ```
//!
//! Exit status: 0 every file converted (or skipped), 1 the run could not
//! start, 2 at least one file failed, 3 at least one file was read only in
//! part (a resource limit; nothing written for it unless `--allow-partial`)
//! and none failed.

use clap::{Parser, ValueEnum};
use rvt::RevitFile;
use rvt::ifc::run_report::{
    DecodeLimits, OUTPUT_MANIFEST_FILE, OutputManifest, RunReport, RunStatus,
};
use rvt::ifc::table_formats::TableFormat;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant, SystemTime};

// The process's allocator (see `mimalloc` in Cargo.toml).
#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[derive(Parser, Debug)]
#[command(
    name = "rvt-batch",
    version,
    about = "Convert every Revit file in a folder tree, one output folder per file",
    after_help = "Examples:\n  \
        rvt-batch projects/ -o converted/\n  \
        rvt-batch projects/ -o converted/ --formats ifc,table,glb,obj --ifc-units mm\n  \
        rvt-batch inbox/ -o converted/ --watch\n\n\
        Each file's folder holds <name>.ifc, <name>.elements.jsonl (.xlsx, .parquet,\n\
        .sqlite), <name>.glb, <name>.obj + <name>.mtl (as asked), report.json and\n\
        manifest.json. An existing folder is never written into: its file is skipped.\n\n\
        Exit status: 0 all converted or skipped; 1 the run could not start; 2 at\n\
        least one file failed; 3 at least one file was read only in part and none\n\
        failed."
)]
struct Cli {
    /// A folder (searched with its subfolders for .rvt, .rfa, .rte and .rft
    /// files) or one Revit file.
    #[arg(value_name = "INPUT")]
    input: PathBuf,

    /// The output folder (created if missing).
    #[arg(short = 'o', long, value_name = "DIR")]
    output: PathBuf,

    /// What to write for each file, comma separated: `ifc`, `table` (the
    /// element table as JSON Lines), `xlsx`, `parquet`, `sqlite` (the same
    /// table as a workbook, a Parquet file, an SQLite database), `glb`,
    /// `obj`. `report.json` and `manifest.json` are always written.
    #[arg(long, value_enum, value_delimiter = ',', default_values = ["ifc", "table"])]
    formats: Vec<Output>,

    #[command(flatten)]
    ifc_output: rvt::cli::IfcOutputArgs,

    /// Raise the stream, inflate and walker-scan limits for large models
    /// (as `rvt-ifc --large-model`).
    #[arg(long)]
    large_model: bool,

    /// Write a file's outputs even when a resource limit kept part of it
    /// from being read; its status is then `partial`.
    #[arg(long)]
    allow_partial: bool,

    /// Keep running: look for new files every `--interval` seconds and
    /// convert each once its size has stopped changing. Stop with Ctrl-C.
    #[arg(long)]
    watch: bool,

    /// Seconds between looks with `--watch`.
    #[arg(long, default_value_t = 10, requires = "watch")]
    interval: u64,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum Output {
    Ifc,
    Table,
    Xlsx,
    Parquet,
    Sqlite,
    Glb,
    Obj,
}

impl Output {
    /// The element table format of `table`, `xlsx`, `parquet` and `sqlite`.
    fn table(self) -> Option<TableFormat> {
        match self {
            Output::Table => Some(TableFormat::Jsonl),
            Output::Xlsx => Some(TableFormat::Xlsx),
            Output::Parquet => Some(TableFormat::Parquet),
            Output::Sqlite => Some(TableFormat::Sqlite),
            Output::Ifc | Output::Glb | Output::Obj => None,
        }
    }
}

/// The marker of an output folder whose conversion has not finished; it goes
/// once `manifest.json` is written.
const INCOMPLETE: &str = ".incomplete";

fn main() -> ExitCode {
    rvt::cli::exit_quietly_on_broken_pipe();
    let cli = Cli::parse();
    match run(&cli) {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(1)
        }
    }
}

fn is_revit_file(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        matches!(
            e.to_ascii_lowercase().as_str(),
            "rvt" | "rfa" | "rte" | "rft"
        )
    })
}

/// Every Revit file under `dir`, sorted, `skip` (the output folder) left
/// out. Symbolic links to folders are not followed.
fn find_files(dir: &Path, skip: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            if path.canonicalize().ok().as_deref() != Some(skip) {
                find_files(&path, skip, out)?;
            }
        } else if is_revit_file(&path) {
            out.push(path);
        }
    }
    Ok(())
}

/// The input files and, for each, its output folder.
fn plan(cli: &Cli) -> Result<Vec<(PathBuf, PathBuf)>, String> {
    let skip = cli
        .output
        .canonicalize()
        .unwrap_or_else(|_| cli.output.clone());
    if cli.input.is_file() {
        let name = cli.input.file_name().ok_or("the input has no file name")?;
        return Ok(vec![(cli.input.clone(), cli.output.join(name))]);
    }
    let mut files = Vec::new();
    find_files(&cli.input, &skip, &mut files)
        .map_err(|e| format!("read {}: {e}", cli.input.display()))?;
    Ok(files
        .into_iter()
        .map(|f| {
            let rel = f.strip_prefix(&cli.input).unwrap_or(&f).to_path_buf();
            let folder = cli.output.join(rel);
            (f, folder)
        })
        .collect())
}

fn run(cli: &Cli) -> Result<u8, String> {
    if !cli.input.exists() {
        return Err(format!("{} does not exist", cli.input.display()));
    }
    if let Some(f) = cli
        .formats
        .iter()
        .filter_map(|o| o.table())
        .find(|f| !f.available())
    {
        return Err(format!(
            "--formats {}: this rvt-batch was built without the `sqlite` feature",
            f.name()
        ));
    }
    std::fs::create_dir_all(&cli.output)
        .map_err(|e| format!("create {}: {e}", cli.output.display()))?;
    let mut worst = 0u8;
    for (input, folder) in plan(cli)? {
        worst = worst.max(convert_and_print(cli, &input, &folder));
    }
    if !cli.watch {
        return Ok(worst);
    }
    // Files that appear later are converted once their size and modified
    // time are the same on two looks in a row.
    let mut seen: HashMap<PathBuf, (u64, Option<SystemTime>)> = HashMap::new();
    loop {
        std::thread::sleep(Duration::from_secs(cli.interval.max(1)));
        for (input, folder) in plan(cli)? {
            if folder.exists() && !folder.join(INCOMPLETE).exists() {
                continue;
            }
            let Ok(meta) = std::fs::metadata(&input) else {
                continue;
            };
            let now = (meta.len(), meta.modified().ok());
            if seen.insert(input.clone(), now) == Some(now) {
                convert_and_print(cli, &input, &folder);
                seen.remove(&input);
            }
        }
    }
}

/// The open limits `rvt-batch` reads with.
fn limits(cli: &Cli) -> rvt::reader::OpenLimits {
    rvt::cli::open_limits(None, None, cli.large_model)
}

/// The conversion options `rvt-batch` exports with.
fn convert_options(cli: &Cli) -> rvt::convert::ConvertOptions {
    let mut opts = rvt::convert::ConvertOptions::rvt_ifc_defaults();
    if cli.large_model {
        opts.walker_limits.max_scan_bytes = rvt::cli::LARGE_MODEL_MAX_INFLATE_BYTES;
    }
    opts
}

/// Make `folder` for a new conversion, marked unfinished; `Err` says why it
/// is skipped. A folder a crash left unfinished is removed first.
fn start_folder(folder: &Path) -> Result<(), String> {
    if folder.join(INCOMPLETE).is_file() {
        std::fs::remove_dir_all(folder)
            .map_err(|e| format!("unfinished output folder could not be removed: {e}"))?;
    }
    if std::fs::create_dir_all(folder.parent().unwrap_or(Path::new("."))).is_err()
        || std::fs::create_dir(folder).is_err()
    {
        return Err(if folder.exists() {
            "output folder already exists".into()
        } else {
            "output folder could not be created".into()
        });
    }
    std::fs::write(folder.join(INCOMPLETE), b"")
        .map_err(|e| format!("output folder could not be written: {e}"))
}

/// Convert one file, print its status line, and return its exit status
/// class (0 ok or skipped, 2 failed, 3 partial).
fn convert_and_print(cli: &Cli, input: &Path, folder: &Path) -> u8 {
    let started = Instant::now();
    let mut report = RunReport::new("rvt-batch", &input.display().to_string());
    if let Err(why) = start_folder(folder) {
        report.status = RunStatus::Skipped;
        report.error = Some(why);
    } else {
        let mut files = Files::default();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            convert(cli, input, folder, &mut report, &mut files)
        }));
        match outcome {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                report.status = RunStatus::Failed;
                report.error = Some(e);
            }
            Err(_) => {
                report.status = RunStatus::Failed;
                report.error = Some("the conversion stopped on an internal error (a panic)".into());
            }
        }
        report.seconds = started.elapsed().as_secs_f64();
        let report_json = report.to_json();
        let mut manifest = OutputManifest::new(
            "rvt-batch",
            &input.display().to_string(),
            report.status,
            DecodeLimits::new(&limits(cli), &convert_options(cli).walker_limits),
        );
        manifest.outputs = files.0;
        manifest.add("report", "report.json", report_json.as_bytes());
        manifest.report = Some("report.json".into());
        let done = rvt::cli::write_output(folder.join("report.json"), &report_json)
            .and_then(|()| {
                rvt::cli::write_output(folder.join(OUTPUT_MANIFEST_FILE), manifest.to_json())
            })
            .and_then(|()| std::fs::remove_file(folder.join(INCOMPLETE)));
        if let Err(e) = done {
            eprintln!("rvt-batch: could not finish {}: {e}", folder.display());
        }
    }
    let line = serde_json::json!({
        "input": input.display().to_string(),
        "output": folder.display().to_string(),
        "status": report.status,
        "seconds": (started.elapsed().as_secs_f64() * 1000.0).round() / 1000.0,
        "elements": report.elements_by_category.values().sum::<usize>(),
        "outputs": report.outputs.iter().map(|o| o.path.clone()).collect::<Vec<_>>(),
        "error": report.error,
    });
    println!("{line}");
    match report.status {
        RunStatus::Ok | RunStatus::Skipped => 0,
        RunStatus::Failed => 2,
        RunStatus::Partial => 3,
    }
}

/// The files written so far, for the manifest.
#[derive(Default)]
struct Files(Vec<rvt::ifc::run_report::ManifestOutput>);

impl Files {
    /// Write `bytes` as `name` in `folder` (whole or not at all) and record
    /// it in the report and the manifest as `format`.
    fn put(
        &mut self,
        folder: &Path,
        name: &str,
        format: &str,
        bytes: &[u8],
        report: &mut RunReport,
    ) -> Result<(), String> {
        let path = folder.join(name);
        rvt::cli::write_output(&path, bytes)
            .map_err(|e| format!("write {}: {e}", path.display()))?;
        report.output(format, &path);
        self.0.push(rvt::ifc::run_report::ManifestOutput::of(
            format, name, bytes,
        ));
        Ok(())
    }
}

/// The conversion of one file into `folder` (which exists and is empty).
fn convert(
    cli: &Cli,
    input: &Path,
    folder: &Path,
    report: &mut RunReport,
    files: &mut Files,
) -> Result<(), String> {
    let stem = input
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "model".into());
    let wants = |o: Output| cli.formats.contains(&o);
    let mut rf = RevitFile::open_with_limits(input, limits(cli)).map_err(|e| e.to_string())?;
    cli.ifc_output
        .set_phase(&mut rf)
        .map_err(|e| e.to_string())?;
    let ifc_options = cli.ifc_output.options();
    let opts = convert_options(cli);
    let exported = rvt::convert::export(&mut rf, &opts).map_err(|e| format!("export: {e}"))?;
    report.notes.clone_from(&exported.notes);
    let mut model = exported.model;
    let mut diagnostics = exported.diagnostics;
    if wants(Output::Ifc)
        && let Some(why) = rvt::ifc::ifc_options::origin_unavailable(&model, ifc_options.origin)
    {
        return Err(why);
    }
    rvt::ifc::record_unread_streams(&mut diagnostics, rf.unread_streams());
    report.elements_by_category = rvt::ifc::element_table::count_by_category(&model);
    let unread = rf.unread_streams().len();
    if unread > 0 {
        report.status = RunStatus::Partial;
        report
            .notes
            .extend(rvt::cli::unread_stream_report(rf.unread_streams()));
        if !cli.allow_partial {
            report.diagnostics = Some(diagnostics);
            report.error = Some(format!(
                "{unread} stream(s) not read whole (a resource limit); nothing written. Re-run with --large-model, or --allow-partial"
            ));
            return Ok(());
        }
    }
    let tables: Vec<TableFormat> = cli.formats.iter().filter_map(|o| o.table()).collect();
    if !tables.is_empty() {
        let rows = rvt::ifc::element_table::element_rows(&model, &exported.quantities);
        for format in TableFormat::ALL.into_iter().filter(|f| tables.contains(f)) {
            let bytes = rvt::ifc::table_formats::write_table(&rows, format)?;
            files.put(
                folder,
                &format!("{stem}.{}", format.extension()),
                &format!("elements-{}", format.name()),
                &bytes,
                report,
            )?;
        }
    }
    report
        .notes
        .extend(rvt::convert::share_and_map(&mut rf, &mut model, &opts));
    drop(rf);
    if wants(Output::Glb) {
        // As `rvt-gltf` writes by default: instanced, feature edges, rooms
        // in the second scene.
        let options = rvt::ifc::gltf::GltfOptions::new(true, false, false).with_display(
            rvt::ifc::gltf::DisplayOptions {
                rooms: false,
                edges: true,
            },
        );
        let glb = rvt::ifc::gltf::model_to_glb_with(&model, &options);
        files.put(folder, &format!("{stem}.glb"), "glb", &glb, report)?;
    }
    if wants(Output::Obj) {
        let (obj, mtl) = rvt::ifc::obj::model_to_obj(&model, &format!("{stem}.mtl"));
        files.put(
            folder,
            &format!("{stem}.obj"),
            "obj",
            obj.as_bytes(),
            report,
        )?;
        files.put(
            folder,
            &format!("{stem}.mtl"),
            "mtl",
            mtl.as_bytes(),
            report,
        )?;
    }
    if wants(Output::Ifc) {
        let georeference = model.georeference.clone();
        let graph = rvt::convert::graph(&model, &opts, false);
        drop(model);
        let timestamp = std::env::var("SOURCE_DATE_EPOCH")
            .ok()
            .and_then(|s| s.trim().parse::<i64>().ok());
        let mut step = rvt::convert::ifc4_step(&graph, timestamp);
        drop(graph);
        if !ifc_options.is_default() {
            let (edited, options_report) =
                rvt::ifc::ifc_options::apply(&step, &ifc_options, georeference.as_ref())?;
            step = edited;
            report.ifc_options = Some(options_report);
        }
        files.put(
            folder,
            &format!("{stem}.ifc"),
            "ifc",
            step.as_bytes(),
            report,
        )?;
    }
    report.diagnostics = Some(diagnostics);
    Ok(())
}
