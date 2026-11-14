//! `rvt-sheet` — render a 2D plan view or a sheet of a Revit file as SVG, DXF, DWG or PDF.
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
//! rvt-sheet model.rvt --list-views                     # the plan views, as JSON
//! rvt-sheet model.rvt --view "Level 1" --format dxf    # only what that plan view shows
//! rvt-sheet model.rvt --level "Level 2" --cut-height 1200
//! rvt-sheet model.rvt --view "Level 1" --format pdf    # vector PDF at the view's scale
//! rvt-sheet model.rvt --list-sheets                    # the sheets and the views on them, as JSON
//! rvt-sheet model.rvt --sheet A102 --format dxf        # a sheet: its plan views in viewports, title block, notes
//! ```
//!
//! `--sheet` draws a sheet as the authoring tool's DWG export lays one out: each plan view on it
//! drawn in model space (side by side, millimetres) and shown through a viewport placed where the
//! view sits on the sheet, at the view's scale; the title block's lines, the sheet's own text
//! notes and detail lines, and each view's title in paper space (layout `Layout1`, millimetres).
//!
//! `--view` draws what a plan view shows: the elements between its view depth and its cut plane,
//! clipped to its crop region when the crop is on, with its room tags, grid heads, text notes and
//! dimensions as labels in DXF and DWG. `--level` draws the band from a level (by name or
//! elevation in millimetres) up to `--cut-height` above it, with no crop.

use clap::Parser;
use rvt::RevitFile;
#[cfg(feature = "dwg")]
use rvt::ifc::sheet_views::section_window;
use rvt::ifc::sheet_views::{MM_PER_FOOT, view_window};
use rvt::ifc::{
    Exporter, RvtDocExporter,
    sheet::{SheetOptions, ViewWindow, render_plan_svg},
};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser, Debug)]
#[command(
    name = "rvt-sheet",
    version,
    about = "Render a 2D plan view or a sheet of a Revit file as SVG, DXF, DWG or PDF",
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

    /// Output format: svg, dxf, dwg or pdf. DXF and DWG hold the same plan outlines as the SVG, in
    /// millimetres, one layer per category; PDF draws the DXF's drawing as vectors, a view at its
    /// scale and a sheet at full size.
    #[arg(long, default_value = "svg", value_parser = ["svg", "dxf", "dwg", "pdf"])]
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

    /// Draw only what this view shows: a plan view (its name; see `--list-views`), or a section
    /// or elevation by its name (DXF, DWG or PDF: its elements' edges in its crop box).
    #[arg(long, conflicts_with = "level")]
    view: Option<String>,

    /// Draw this sheet (its number or name; see `--list-sheets`) as DXF, DWG or PDF.
    #[arg(long, conflicts_with_all = ["view", "level"])]
    sheet: Option<String>,

    /// Print the sheets (number, name, the views placed on them) as JSON and exit.
    #[arg(long)]
    list_sheets: bool,

    /// Draw only the band from this level (its name, or its elevation in millimetres) to
    /// `--cut-height` above it.
    #[arg(long)]
    level: Option<String>,

    /// Cut height above `--level`, millimetres (a plan's usual cut plane).
    #[arg(long, default_value_t = 1200.0, requires = "level")]
    cut_height: f64,

    /// Print the plan views (name, level, view range, crop, in feet) as JSON and exit.
    #[arg(long)]
    list_views: bool,
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
    if rvt::cli::same_file(&output, &input) {
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
    if cli.list_views {
        let facts = rvt::plan_views::read(&mut rf).map_err(|e| e.to_string())?;
        println!(
            "{}",
            serde_json::to_string_pretty(&facts.views).map_err(|e| e.to_string())?
        );
        return Ok(());
    }
    if cli.list_sheets {
        let facts = rvt::plan_views::read(&mut rf).map_err(|e| e.to_string())?;
        let sheets: Vec<serde_json::Value> = facts
            .sheets
            .iter()
            .map(|s| {
                let views: Vec<serde_json::Value> = facts
                    .viewports
                    .iter()
                    .filter(|v| v.sheet == s.id)
                    .map(|v| {
                        let plan = facts.views.iter().find(|p| p.id == v.view);
                        let section = facts.sections.iter().find(|p| p.id == v.view);
                        serde_json::json!({
                            "view": v.view,
                            "name": plan.map(|p| p.name.clone()).or_else(|| section.map(|p| p.name.clone())),
                            "plan": plan.is_some(),
                            "section": section.is_some(),
                            "offset": v.offset,
                        })
                    })
                    .collect();
                serde_json::json!({"id": s.id, "number": s.number, "name": s.name, "views": views})
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&sheets).map_err(|e| e.to_string())?
        );
        return Ok(());
    }
    if cli.sheet.is_some() && cli.format == "svg" {
        return Err("--sheet writes dxf, dwg or pdf: pass --format".into());
    }
    let facts = if cli.view.is_some() || cli.sheet.is_some() {
        Some(rvt::plan_views::read(&mut rf).map_err(|e| e.to_string())?)
    } else {
        None
    };
    let model = RvtDocExporter
        .export(&mut rf)
        .map_err(|e| format!("export {}: {e}", input.display()))?;
    // A plan view draws each family instance's plan symbol (door swings, window lines).
    let symbols = if (cli.view.is_some() || cli.sheet.is_some()) && cli.format != "svg" {
        rvt::ifc::plan_maps::instance_plan_symbols(&mut rf, &model).unwrap_or_else(|e| {
            eprintln!("rvt-sheet: plan symbols not read: {e}");
            Default::default()
        })
    } else {
        Default::default()
    };
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
    if let (Some(key), Some(facts)) = (&cli.sheet, &facts) {
        let sheet = facts.find_sheet(key)?;
        let drawing = sheet_drawing(&mut rf, &model, facts, sheet, &symbols)?;
        let bytes = drawing_bytes(&drawing, Some("Layout1"), 1.0, &cli.format, cli)?;
        rvt::cli::write_output(&output, &bytes)
            .map_err(|e| format!("write {}: {e}", output.display()))?;
        return Ok(());
    }
    // A section or elevation (a name no plan view has): its projection, DXF, DWG or PDF.
    if let (Some(name), Some(facts)) = (&cli.view, &facts) {
        if facts.find(name).is_err() {
            if let Some(view) = facts
                .sections
                .iter()
                .find(|v| !v.template && v.name.eq_ignore_ascii_case(name))
            {
                if cli.format == "svg" {
                    return Err(format!(
                        "{name:?} is a section or elevation: pass --format dxf, dwg or pdf"
                    ));
                }
                let window = section_window(facts, view)
                    .ok_or_else(|| format!("{name:?} has no crop box decoded"))?;
                let drawing = projected_drawing_of(&model, &window)?;
                let bytes = drawing_bytes(
                    &drawing,
                    None,
                    view.scale.unwrap_or(100.0),
                    &cli.format,
                    cli,
                )?;
                rvt::cli::write_output(&output, &bytes)
                    .map_err(|e| format!("write {}: {e}", output.display()))?;
                return Ok(());
            }
        }
    }
    let (window, annotation) = match (&cli.view, &facts, &cli.level) {
        (Some(name), Some(facts), _) => {
            let view = facts.find(name)?;
            let (window, annotation) = view_window(&model, facts, view)?;
            (Some(window), Some(annotation))
        }
        (_, _, Some(level)) => (Some(level_window(&model, level, cli.cut_height)?), None),
        _ => (None, None),
    };
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
        window,
        annotation,
        symbols,
    };
    // A view's PDF is drawn at the view's scale; a level band or the whole model at 1:100.
    let pdf_scale = cli
        .view
        .as_ref()
        .zip(facts.as_ref())
        .and_then(|(name, f)| f.find(name).ok()?.scale)
        .unwrap_or(100.0);
    let bytes = match cli.format.as_str() {
        "svg" => render_plan_svg(&model, &options).into_bytes(),
        format => drawing_bytes(
            &plan_drawing_of(&model, &options)?,
            None,
            pdf_scale,
            format,
            cli,
        )?,
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

/// The band of `--level`: the level found by name (exact, then ignoring case) or by elevation in
/// millimetres (the nearest within 1 mm), up to `cut_mm` above it.
fn level_window(
    model: &rvt::ifc::IfcModel,
    level: &str,
    cut_mm: f64,
) -> Result<ViewWindow, String> {
    let storeys = &model.building_storeys;
    let found = storeys
        .iter()
        .find(|s| s.name == level)
        .or_else(|| storeys.iter().find(|s| s.name.eq_ignore_ascii_case(level)))
        .or_else(|| {
            let mm: f64 = level.trim().parse().ok()?;
            storeys
                .iter()
                .filter(|s| (s.elevation_feet * MM_PER_FOOT - mm).abs() <= 1.0)
                .min_by(|a, b| {
                    (a.elevation_feet * MM_PER_FOOT - mm)
                        .abs()
                        .total_cmp(&(b.elevation_feet * MM_PER_FOOT - mm).abs())
                })
        })
        .ok_or_else(|| {
            let names: Vec<String> = storeys
                .iter()
                .map(|s| format!("{} ({:.0} mm)", s.name, s.elevation_feet * MM_PER_FOOT))
                .collect();
            format!("no level {level:?}; the levels are: {}", names.join(", "))
        })?;
    Ok(ViewWindow {
        bottom_feet: found.elevation_feet,
        top_feet: found.elevation_feet + cut_mm / MM_PER_FOOT,
        ..ViewWindow::default()
    })
}

#[cfg(feature = "dwg")]
fn plan_drawing_of(
    model: &rvt::ifc::IfcModel,
    options: &SheetOptions,
) -> Result<rvt_dwg::Drawing2d, String> {
    Ok(rvt::ifc::sheet::plan_drawing(model, options))
}

#[cfg(feature = "dwg")]
fn projected_drawing_of(
    model: &rvt::ifc::IfcModel,
    window: &rvt::ifc::sheet::ProjectedWindow,
) -> Result<rvt_dwg::Drawing2d, String> {
    Ok(rvt::ifc::sheet::projected_drawing(model, window))
}

/// A drawing as DXF, DWG or PDF (`layout` drawn by the PDF: a sheet; `None` for model space at
/// 1:`scale`).
#[cfg(feature = "dwg")]
fn drawing_bytes(
    drawing: &rvt_dwg::Drawing2d,
    layout: Option<&str>,
    scale: f64,
    format: &str,
    cli: &Cli,
) -> Result<Vec<u8>, String> {
    let version = if format == "dwg" {
        &cli.dwg_version
    } else {
        &cli.dxf_version
    };
    let (bytes, notes) =
        rvt::ifc::sheet_views::drawing_bytes(drawing, layout, scale, format, version)?;
    for n in notes {
        eprintln!("rvt-sheet: {n}");
    }
    Ok(bytes)
}

/// A sheet (`rvt::ifc::sheet_views::sheet_drawing`), saying on stderr what it did not draw.
#[cfg(feature = "dwg")]
fn sheet_drawing(
    rf: &mut RevitFile,
    model: &rvt::ifc::IfcModel,
    facts: &rvt::plan_views::ViewFacts,
    sheet: &rvt::plan_views::Sheet,
    symbols: &std::collections::BTreeMap<usize, Vec<rvt::ifc::entities::PlanPolyline>>,
) -> Result<rvt_dwg::Drawing2d, String> {
    let mut notes = Vec::new();
    let drawing =
        rvt::ifc::sheet_views::sheet_drawing(rf, model, facts, sheet, symbols, &mut notes);
    for n in notes {
        eprintln!("rvt-sheet: {n}");
    }
    drawing
}

#[cfg(not(feature = "dwg"))]
fn section_window(
    _: &rvt::plan_views::ViewFacts,
    _: &rvt::plan_views::SectionView,
) -> Option<rvt::ifc::sheet::ProjectedWindow> {
    None
}

#[cfg(not(feature = "dwg"))]
fn projected_drawing_of(
    _: &rvt::ifc::IfcModel,
    _: &rvt::ifc::sheet::ProjectedWindow,
) -> Result<(), String> {
    Err("this build has no DWG/DXF support (feature `dwg`)".into())
}

#[cfg(not(feature = "dwg"))]
fn plan_drawing_of(_: &rvt::ifc::IfcModel, _: &SheetOptions) -> Result<(), String> {
    Err("this build has no DWG/DXF support (feature `dwg`)".into())
}

#[cfg(not(feature = "dwg"))]
fn drawing_bytes(
    _: &(),
    _: Option<&str>,
    _: f64,
    format: &str,
    _: &Cli,
) -> Result<Vec<u8>, String> {
    Err(format!(
        "{format}: this build has no DWG/DXF support (feature `dwg`)"
    ))
}

#[cfg(not(feature = "dwg"))]
fn sheet_drawing(
    _: &mut RevitFile,
    _: &rvt::ifc::IfcModel,
    _: &rvt::plan_views::ViewFacts,
    _: &rvt::plan_views::Sheet,
    _: &std::collections::BTreeMap<usize, Vec<rvt::ifc::entities::PlanPolyline>>,
) -> Result<(), String> {
    Err("--sheet: this build has no DWG/DXF support (feature `dwg`)".into())
}
