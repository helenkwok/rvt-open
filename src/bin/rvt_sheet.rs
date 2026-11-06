//! `rvt-sheet` — render a 2D plan view of a Revit file as SVG, DXF or DWG.
//!
//! The command-line front end of the plan renderer, `rvt::ifc::sheet`. Useful for:
//!
//! - embedding a plan in a markdown README or a PDF report
//! - any workflow that wants a picture without a full 3D viewer
//!
//! Usage:
//!
//! ```bash
//! rvt-sheet model.rvt                                  # writes model.svg
//! rvt-sheet model.rvt -o plan.svg --width 2000 --height 1500
//! rvt-sheet model.rvt -o plan.svg --no-labels --no-background
//! rvt-sheet --src model.rvt --dst plan.svg             # older spelling, still accepted
//! rvt-sheet model.rvt --format dxf                     # writes model.dxf (millimetres, layers by category)
//! rvt-sheet model.rvt --format dwg --dwg-version 2013  # writes model.dwg
//! ```

use clap::Parser;
use rvt::RevitFile;
use rvt::ifc::{
    Exporter, RvtDocExporter,
    sheet::{SheetOptions, render_plan_svg},
};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser, Debug)]
#[command(
    name = "rvt-sheet",
    version,
    about = "Render a 2D plan view of a Revit file as SVG, DXF or DWG",
    after_help = "Examples:\n  \
        rvt-sheet model.rvt                          writes model.svg next to the input\n  \
        rvt-sheet model.rvt -o plan.svg --width 2000 --height 1500 --no-labels\n  \
        rvt-sheet model.rvt --format dwg --dwg-version 2018   writes model.dwg"
)]
struct Cli {
    /// Revit file to render (.rvt / .rfa / .rte / .rft).
    #[arg(
        value_name = "INPUT",
        required_unless_present = "src",
        conflicts_with = "src"
    )]
    input: Option<PathBuf>,

    /// Output path. Default: the input path with the format's extension.
    #[arg(short = 'o', long = "output", alias = "dst", short_alias = 'd')]
    output: Option<PathBuf>,

    /// Older spelling of INPUT (`--src model.rvt`), kept for scripts.
    #[arg(short = 's', long = "src", hide = true)]
    src: Option<PathBuf>,

    /// Output width in pixels.
    #[arg(long, default_value_t = 1200)]
    width: u32,

    /// Output height in pixels.
    #[arg(long, default_value_t = 800)]
    height: u32,

    /// Interior margin in pixels.
    #[arg(long, default_value_t = 40.0)]
    margin: f32,

    /// Omit element name labels (useful for dense plans).
    #[arg(long)]
    no_labels: bool,

    /// Omit the white background (emit a transparent SVG).
    #[arg(long)]
    no_background: bool,

    /// Output format: svg, dxf or dwg. DXF and DWG hold the same plan outlines as the SVG, in
    /// millimetres, one layer per category.
    #[arg(long, default_value = "svg", value_parser = ["svg", "dxf", "dwg"])]
    format: String,

    /// DXF version (2018, 2013, 2010, 2007, 2004, 2000, R14, R12).
    #[arg(long, default_value = "2018")]
    dxf_version: String,

    /// DWG version (2018, 2013, 2010, 2007, 2004, 2000).
    #[arg(long, default_value = "2018")]
    dwg_version: String,

    /// Print element count + output size on success.
    #[arg(long)]
    verbose: bool,
}

fn main() -> ExitCode {
    rvt::cli::exit_quietly_on_broken_pipe();
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(1)
        }
    }
}

/// Input from INPUT or the legacy `--src`; output from `-o` or the input
/// path with `extension` swapped in. Refuses to overwrite the input.
fn resolve_paths(
    input: Option<&Path>,
    src: Option<&Path>,
    output: Option<&Path>,
    extension: &str,
) -> Result<(PathBuf, PathBuf), String> {
    let input = input.or(src).ok_or("no input file given")?.to_path_buf();
    let output = output
        .map(Path::to_path_buf)
        .unwrap_or_else(|| input.with_extension(extension));
    if output == input {
        return Err(format!(
            "refusing to overwrite the input {}; pass -o <path>",
            input.display()
        ));
    }
    Ok((input, output))
}

fn run(cli: &Cli) -> Result<(), String> {
    let (input, output) = resolve_paths(
        cli.input.as_deref(),
        cli.src.as_deref(),
        cli.output.as_deref(),
        &cli.format,
    )?;
    // Open errors from the library already name the file.
    let mut rf = RevitFile::open(&input).map_err(|e| e.to_string())?;
    let model = RvtDocExporter
        .export(&mut rf)
        .map_err(|e| format!("export {}: {e}", input.display()))?;
    // Where the last phase holds no room, the plan draws the rooms of
    // the latest phase that holds any.
    if let Some(fallback) = rf.room_phase_fallback() {
        let phase = fallback
            .name
            .unwrap_or_else(|| format!("ElementId {}", fallback.id));
        eprintln!(
            "rvt-sheet: the project's last phase holds no room, so the rooms of {phase:?}, the latest phase that holds any, are drawn; every other element is of the last phase"
        );
    }
    let options = SheetOptions {
        width_px: cli.width,
        height_px: cli.height,
        margin_px: cli.margin,
        show_labels: !cli.no_labels,
        background: if cli.no_background {
            None
        } else {
            Some("#FFFFFF".into())
        },
    };
    let bytes = match cli.format.as_str() {
        "svg" => render_plan_svg(&model, &options).into_bytes(),
        format => cad_bytes(&model, &options, format, cli)?,
    };
    rvt::cli::write_output(&output, &bytes)
        .map_err(|e| format!("write {}: {e}", output.display()))?;
    if cli.verbose {
        let drawn = model
            .entities
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    rvt::ifc::entities::IfcEntity::BuildingElement {
                        location_feet: Some(_),
                        extrusion: Some(_),
                        ..
                    }
                )
            })
            .count();
        println!(
            "rvt-sheet: wrote {} bytes ({} drawn elements) to {}",
            bytes.len(),
            drawn,
            output.display()
        );
    }
    Ok(())
}

#[cfg(feature = "dwg")]
fn cad_bytes(
    model: &rvt::ifc::IfcModel,
    options: &SheetOptions,
    format: &str,
    cli: &Cli,
) -> Result<Vec<u8>, String> {
    use rvt_dwg::write::{CadFormat, parse_version, write_drawing};
    let (format, version) = if format == "dwg" {
        (CadFormat::Dwg, &cli.dwg_version)
    } else {
        (CadFormat::Dxf, &cli.dxf_version)
    };
    let version = parse_version(version, format).map_err(|e| e.to_string())?;
    let drawing = rvt::ifc::sheet::plan_drawing(model, options);
    let (bytes, notes) = write_drawing(&drawing, format, version).map_err(|e| e.to_string())?;
    for n in notes {
        eprintln!("rvt-sheet: {n}");
    }
    Ok(bytes)
}

#[cfg(not(feature = "dwg"))]
fn cad_bytes(
    _: &rvt::ifc::IfcModel,
    _: &SheetOptions,
    format: &str,
    _: &Cli,
) -> Result<Vec<u8>, String> {
    Err(format!(
        "{format}: this build has no DWG/DXF support (feature `dwg`)"
    ))
}
