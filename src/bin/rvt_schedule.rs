//! `rvt-schedule` — element and room schedules of a Revit file as CSV,
//! ready for Excel, Google Sheets or LibreOffice.
//!
//! Runs the document exporter and writes one row per decoded element (or
//! per room) with the columns documented in `rvt::ifc::schedule_csv`. The
//! rows are the elements the IFC export writes, each on the storey the export
//! puts it on. Only decoded values are written; unknowns are empty cells.
//!
//! ```bash
//! rvt-schedule model.rvt                       # writes model.elements.csv
//! rvt-schedule model.rvt --schedule rooms      # writes model.rooms.csv
//! rvt-schedule model.rvt --metric --excel -o schedule.csv
//! rvt-schedule model.rvt -o -                  # CSV to stdout
//! rvt-schedule model.rvt --format jsonl        # model.elements.jsonl: every element with all its parameters
//! rvt-schedule model.rvt --format xlsx         # model.elements.xlsx (also parquet, sqlite)
//! rvt-schedule model.rvt --phase "Existing"    # the schedule of another phase
//! ```
//!
//! `--format jsonl` writes the element table (`rvt::ifc::element_table`):
//! one JSON record per element with its GlobalId, category, family, type,
//! level, area, volume, length and every decoded parameter with its unit. It
//! runs the full export (`rvt-ifc`'s), so its GlobalIds are the IFC file's.
//! `xlsx`, `parquet` and `sqlite` write the same table
//! (`rvt::ifc::table_formats`); `sqlite` needs a build with the `sqlite`
//! feature.

use clap::{Parser, ValueEnum};
use rvt::RevitFile;
use rvt::ifc::schedule_csv::{CsvOptions, LengthUnit, elements_csv, rooms_csv};
use rvt::ifc::{Exporter, RvtDocExporter};
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser, Debug)]
#[command(
    name = "rvt-schedule",
    version,
    about = "Export element and room schedules of a Revit file as CSV (Excel, Sheets, LibreOffice)",
    after_help = "Examples:\n  \
        rvt-schedule model.rvt                     writes model.elements.csv next to the input\n  \
        rvt-schedule model.rvt --schedule rooms    writes model.rooms.csv\n  \
        rvt-schedule model.rvt --metric --excel -o schedule.csv\n  \
        rvt-schedule model.rvt -o -                CSV to stdout\n  \
        rvt-schedule model.rvt --format jsonl      writes model.elements.jsonl, every element with all its parameters\n  \
        rvt-schedule model.rvt --format xlsx       writes model.elements.xlsx (also parquet, sqlite)\n  \
        rvt-schedule model.rvt --phase \"Existing\"  the schedule of that phase, not the last"
)]
struct Cli {
    /// Revit file (.rvt / .rfa / .rte / .rft).
    #[arg(value_name = "INPUT")]
    input: PathBuf,

    /// Which schedule to write.
    #[arg(long, value_enum, default_value = "elements")]
    schedule: Kind,

    /// Output path, or `-` for stdout. Default: `<input>.elements.csv` /
    /// `<input>.rooms.csv` next to the input.
    #[arg(short = 'o', long)]
    output: Option<PathBuf>,

    /// Lengths in metres instead of feet (columns end in `_m`).
    #[arg(long)]
    metric: bool,

    /// Start the file with a UTF-8 byte-order mark so Excel on Windows reads
    /// non-ASCII names correctly.
    #[arg(long)]
    excel: bool,

    /// Schedule this phase, by its name (exactly) or its ElementId, instead of
    /// the project's last, as `rvt-ifc --phase` does: an element that does
    /// not exist in it, and a room whose own phase is not it, are left out.
    /// Without it, a project whose last phase holds no room schedules the
    /// rooms of the latest phase that holds any, and says so; naming the
    /// last phase schedules none. An unknown phase is an error that lists
    /// the file's phases.
    #[arg(long, value_name = "NAME|ID")]
    phase: Option<String>,
    /// `csv` (the columns above) or the element table: `jsonl`, one JSON
    /// record per element with its GlobalId, category, family, type, level,
    /// area, volume, length and every decoded parameter with its unit (SI
    /// units; `--metric` and `--excel` do not apply), or the same table as
    /// `xlsx`, `parquet` or `sqlite`. The element table is elements only.
    #[arg(long, value_enum, default_value = "csv")]
    format: TableFormat,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum TableFormat {
    Csv,
    Jsonl,
    Xlsx,
    Parquet,
    Sqlite,
}

impl TableFormat {
    /// The element table format, for every format but `csv`.
    fn table(self) -> Option<rvt::ifc::table_formats::TableFormat> {
        use rvt::ifc::table_formats::TableFormat as T;
        match self {
            TableFormat::Csv => None,
            TableFormat::Jsonl => Some(T::Jsonl),
            TableFormat::Xlsx => Some(T::Xlsx),
            TableFormat::Parquet => Some(T::Parquet),
            TableFormat::Sqlite => Some(T::Sqlite),
        }
    }
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// Every building element the IFC export writes, on the storey it
    /// puts it on: type, level, material, placement, size, host.
    Elements,
    /// Every room: number, name, level.
    Rooms,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Elements => "elements",
            Kind::Rooms => "rooms",
        }
    }

    fn noun(self) -> &'static str {
        match self {
            Kind::Elements => "element",
            Kind::Rooms => "room",
        }
    }
}

fn main() -> ExitCode {
    rvt::cli::exit_quietly_on_broken_pipe();
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(1)
        }
    }
}

fn run(cli: Cli) -> Result<(), String> {
    if let Some(format) = cli.format.table() {
        return run_table(&cli, format);
    }
    // Open errors from the library already name the file.
    let mut rf = RevitFile::open(&cli.input).map_err(|e| e.to_string())?;
    if let Some(phase) = &cli.phase {
        rf.set_export_phase(phase).map_err(|e| e.to_string())?;
    }
    let model = RvtDocExporter
        .export(&mut rf)
        .map_err(|e| format!("export {}: {e}", cli.input.display()))?;
    // Where no phase is named and the last holds no room, the rooms
    // are those of the latest phase that holds any.
    if let Some(fallback) = rf.room_phase_fallback() {
        let phase = fallback
            .name
            .unwrap_or_else(|| format!("ElementId {}", fallback.id));
        eprintln!(
            "rvt-schedule: the project's last phase holds no room, so the rooms of {phase:?}, the latest phase that holds any, are scheduled; --phase NAME|ID chooses the phase exactly (naming the last writes no room)"
        );
    }
    let options = CsvOptions {
        unit: if cli.metric {
            LengthUnit::Metres
        } else {
            LengthUnit::Feet
        },
        excel_bom: cli.excel,
    };
    let csv = match cli.schedule {
        Kind::Elements => elements_csv(&model, &options),
        Kind::Rooms => rooms_csv(&model, &options),
    };
    let rows = csv.matches("\r\n").count().saturating_sub(1);

    let output = cli.output.unwrap_or_else(|| {
        cli.input
            .with_extension(format!("{}.csv", cli.schedule.as_str()))
    });
    if output.as_os_str() == "-" {
        std::io::stdout()
            .write_all(csv.as_bytes())
            .map_err(|e| format!("write stdout: {e}"))?;
        return Ok(());
    }
    if output == cli.input {
        return Err(format!(
            "refusing to overwrite the input {}; pass -o <path>",
            cli.input.display()
        ));
    }
    rvt::cli::write_output(&output, csv.as_bytes())
        .map_err(|e| format!("write {}: {e}", output.display()))?;
    eprintln!(
        "rvt-schedule: wrote {rows} {} row{} to {}",
        cli.schedule.noun(),
        if rows == 1 { "" } else { "s" },
        output.display()
    );
    if rows == 0 {
        eprintln!(
            "rvt-schedule: no {} decoded from this file — `rvt-inspect {}` explains what was recovered",
            cli.schedule.as_str(),
            cli.input.display()
        );
    }
    Ok(())
}

/// `--format jsonl|xlsx|parquet|sqlite`: the element table from the full
/// export.
fn run_table(cli: &Cli, format: rvt::ifc::table_formats::TableFormat) -> Result<(), String> {
    if cli.schedule != Kind::Elements {
        return Err(format!(
            "--format {} writes the element table; rooms are in it (ifc_class IfcSpace)",
            format.name()
        ));
    }
    if !format.available() {
        return Err(format!(
            "--format {}: this rvt-schedule was built without the `sqlite` feature",
            format.name()
        ));
    }
    let mut rf = RevitFile::open(&cli.input).map_err(|e| e.to_string())?;
    if let Some(phase) = &cli.phase {
        rf.set_export_phase(phase).map_err(|e| e.to_string())?;
    }
    let exported = rvt::convert::export(&mut rf, &rvt::convert::ConvertOptions::rvt_ifc_defaults())
        .map_err(|e| format!("export {}: {e}", cli.input.display()))?;
    drop(rf);
    let rows = rvt::ifc::element_table::element_rows(&exported.model, &exported.quantities);
    drop(exported);
    let bytes = rvt::ifc::table_formats::write_table(&rows, format)?;
    let output = cli
        .output
        .clone()
        .unwrap_or_else(|| cli.input.with_extension(format.extension()));
    if output.as_os_str() == "-" {
        return std::io::stdout()
            .write_all(&bytes)
            .map_err(|e| format!("write stdout: {e}"));
    }
    if output == cli.input {
        return Err(format!(
            "refusing to overwrite the input {}; pass -o <path>",
            cli.input.display()
        ));
    }
    rvt::cli::write_output(&output, &bytes)
        .map_err(|e| format!("write {}: {e}", output.display()))?;
    eprintln!(
        "rvt-schedule: wrote {} element record{} to {}",
        rows.len(),
        if rows.len() == 1 { "" } else { "s" },
        output.display()
    );
    Ok(())
}
