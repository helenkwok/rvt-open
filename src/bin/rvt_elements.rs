//! `rvt-elements` — dump production decoded elements / class counts.
//!
//! Mirrors the Python `RevitFile.decoded_elements()` /
//! `RevitFile.element_counts()` surfaces so CLI and Python share one
//! JSON contract (`docs/schemas/decoded-elements.schema.json`,
//! `docs/schemas/element-counts.schema.json`).

use clap::Parser;
use rvt::RevitFile;
use rvt::walker::WalkerLimits;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "rvt-elements",
    version,
    about = "Dump production decoded elements or class counts as JSON",
    after_help = "Examples:\n  \
        rvt-elements model.rvt --counts\n  \
        rvt-elements model.rvt --min-confidence 0.55 > elements.json"
)]
struct Cli {
    /// Path to a `.rvt` / `.rfa` / `.rte` / `.rft` file.
    file: PathBuf,

    /// Emit `{total, by_class}` instead of the full element list.
    #[arg(long)]
    counts: bool,

    /// Omit the `typed` projection (the typed view of the decoded classes) on each element.
    #[arg(long)]
    no_typed: bool,

    /// Hide elements whose provenance confidence is below this floor.
    /// Default keeps everything; pass `0.55` to match the
    /// viewer/export low-confidence filter.
    #[arg(long, default_value_t = 0.0)]
    min_confidence: f32,

    /// Print scan progress to stderr (`progress: <stage> <done>/<total>`).
    /// Useful on large project files where the walker runs for a while.
    #[arg(long)]
    progress: bool,
}

fn main() -> std::process::ExitCode {
    rvt::cli::exit_quietly_on_broken_pipe();
    match run(Cli::parse()) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::ExitCode::from(1)
        }
    }
}

fn run(cli: Cli) -> anyhow::Result<()> {
    let mut rf = RevitFile::open(&cli.file)?;
    let control = if cli.progress {
        rvt::control::WalkerControl::new().with_progress(|event| {
            let total = event.total.map(|t| format!("/{t}")).unwrap_or_default();
            eprintln!("progress: {} {}{total}", event.stage.as_str(), event.done);
        })
    } else {
        rvt::control::WalkerControl::new()
    };
    // Shared with rvt-wasm's `elements_json`.
    let json = rvt::convert::elements_json(
        &mut rf,
        cli.min_confidence,
        !cli.no_typed,
        cli.counts,
        WalkerLimits::default(),
        &control,
    )?;
    println!("{json}");
    Ok(())
}
