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
use rvt::ifc::{
    Exporter, RvtDocExporter,
    sheet::{
        CropRect, PlanAnnotation, PlanLine, PlanText, SheetOptions, ViewWindow, render_plan_svg,
    },
};
use rvt::plan_views::LineKind;
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

const MM_PER_FOOT: f64 = 304.8;
/// BuiltInCategory ids: Lines, Grids.
const LINES: i64 = -2_000_051;
const GRIDS: i64 = -2_000_220;

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

/// A plan view's band (view depth to cut plane), crop and annotation.
fn view_window(
    model: &rvt::ifc::IfcModel,
    facts: &rvt::plan_views::ViewFacts,
    view: &rvt::plan_views::PlanView,
) -> Result<(ViewWindow, PlanAnnotation), String> {
    let level_elevation = view.level.as_ref().and_then(|name| {
        model
            .building_storeys
            .iter()
            .find(|s| &s.name == name)
            .map(|s| s.elevation_feet)
    });
    let cut = view
        .cut_plane
        .or_else(|| level_elevation.map(|e| e + 1200.0 / MM_PER_FOOT))
        .ok_or_else(|| format!("plan view {:?}: no cut plane or level decoded", view.name))?;
    let bottom = view.bottom.or(level_elevation).unwrap_or(cut).min(cut);
    // The crop applies when the view crops (`on`) and a side face is active.
    let crop = view
        .crop
        .as_ref()
        .filter(|c| c.on && c.active[..2].iter().flatten().any(|a| *a))
        .map(|c| {
            let [x, y, _] = c.axes[0];
            let n = x.hypot(y);
            let x_axis = if n > 1e-9 { [x / n, y / n] } else { [1.0, 0.0] };
            // The view's y axis is x turned a quarter left, or the opposite (a mirrored box):
            // then its y range is negated.
            let left = [-x_axis[1], x_axis[0]];
            let flipped = c.axes[1][0] * left[0] + c.axes[1][1] * left[1] < 0.0;
            let (y0, y1) = if flipped {
                (-c.max[1], -c.min[1])
            } else {
                (c.min[1], c.max[1])
            };
            CropRect {
                origin: [c.origin[0], c.origin[1]],
                x_axis,
                min: [c.min[0], y0],
                max: [c.max[0], y1],
            }
        });
    // The view's own axes (its crop box's x axis, kept whether or not it crops, and that turned
    // a quarter left): a site plan turned to true north draws the model turned.
    let frame = view.crop.as_ref().and_then(|c| {
        let (x, y) = (c.axes[0][0], c.axes[0][1]);
        let n = x.hypot(y);
        (n > 1e-9 && ((x / n - 1.0).abs() > 1e-9 || (y / n).abs() > 1e-9))
            .then(|| [[x / n, y / n], [-y / n, x / n]])
    });
    let scale = view.scale.unwrap_or(100.0);
    // Annotation is drawn out to the view's annotation crop when that is on: the crop box grown
    // by its offsets (sheet sizes, left, right, top, bottom). With it off, text, tags and
    // dimensions are kept wherever they are. Grid lines are cut at the model crop when drawn.
    let in_crop = |p: [f64; 2]| crop.is_none_or(|c| c.contains((p[0], p[1])));
    let annotation_crop = crop.zip(view.crop.as_ref().and_then(|c| c.annotation)).map(
        |(mut c, [left, right, top, bottom])| {
            c.min[0] -= left * scale;
            c.max[0] += right * scale;
            c.max[1] += top * scale;
            c.min[1] -= bottom * scale;
            c
        },
    );
    let in_annotation_crop = |p: [f64; 2]| annotation_crop.is_none_or(|c| c.contains((p[0], p[1])));
    let hidden_element = |id: i64| view.hidden_elements.contains(&id);
    let mut a = PlanAnnotation::default();
    for t in facts
        .texts
        .iter()
        .filter(|t| t.view == view.id && in_annotation_crop([t.at[0], t.at[1]]))
    {
        a.texts.push(PlanText {
            at: [t.at[0], t.at[1]],
            value: t.value.clone(),
            // Text heights are sheet sizes.
            height_feet: t.height.unwrap_or(2.5 / MM_PER_FOOT) * scale,
            rotation: t.rotation,
            layer: "G-ANNO-TEXT".into(),
        });
    }
    for tag in facts.room_tags.iter().filter(|t| t.view == view.id) {
        let Some(room) = tag.room.and_then(|r| facts.rooms.get(&r)) else {
            continue;
        };
        let Some(at) = tag.at.map(|p| [p[0], p[1]]).or(room.point) else {
            continue;
        };
        if !in_annotation_crop(at) {
            continue;
        }
        for (i, value) in [&room.name, &room.number].into_iter().flatten().enumerate() {
            a.texts.push(PlanText {
                at: [at[0], at[1] - i as f64 * 4.0 * scale / MM_PER_FOOT],
                value: value.clone(),
                height_feet: 2.5 * scale / MM_PER_FOOT,
                rotation: 0.0,
                layer: "A-AREA-IDEN".into(),
            });
        }
    }
    let grids_hidden = facts.hidden_categories(view).contains(&GRIDS);
    // A grid head's circle, sheet radius: 1/4" in a view at an imperial scale (1/8" = 1'-0" is
    // 96), else 6.5 mm (the heads of the samples' DWG exports).
    let imperial = (scale.round() - scale).abs() < 1e-6
        && (scale.round() as i64 % 12 == 0 || (scale.round() as i64).count_ones() == 1);
    let radius = if imperial {
        0.25 / 12.0
    } else {
        6.5 / MM_PER_FOOT
    } * scale;
    for g in facts
        .grids
        .iter()
        .filter(|g| !grids_hidden && !hidden_element(g.id))
    {
        // The view's extent of the grid when it has one, else the grid's own (free end first,
        // the head at the bubble end).
        let Some((o, dir)) = g.axis else { continue };
        let at = |t: f64| [o[0] + t * dir[0], o[1] + t * dir[1]];
        let extent = facts
            .grid_extents
            .iter()
            .find(|e| e.view == view.id && e.grid == g.id);
        let (free, head, free_head, bubble_head) = match (extent, g.ends) {
            (Some(e), _) => (at(e.free), at(e.bubble), e.free_head, e.bubble_head),
            (None, Some([f, b])) => ([f[0], f[1]], [b[0], b[1]], false, true),
            (None, None) => continue,
        };
        // Grids run out to the annotation crop when it is on, else to the crop.
        let (free, head) = match annotation_crop.or(crop) {
            Some(c) => match c.clip((free[0], free[1]), (head[0], head[1])) {
                Some((p, q)) => ([p.0, p.1], [q.0, q.1]),
                None => continue,
            },
            None => (free, head),
        };
        a.lines.push(PlanLine {
            a: free,
            b: head,
            layer: "S-GRID".into(),
            outside_crop: true,
        });
        // A head: a circle just beyond its end, the grid's name in it (placed where the export's
        // text box starts, up and left of the centre).
        let mut ends = Vec::new();
        if bubble_head {
            ends.push((head, free));
        }
        if free_head {
            ends.push((free, head));
        }
        for (end, other) in ends {
            let (dx, dy) = (end[0] - other[0], end[1] - other[1]);
            let n = dx.hypot(dy);
            if n < 1e-9 {
                continue;
            }
            let centre = [end[0] + dx / n * radius, end[1] + dy / n * radius];
            let on_circle = |k: usize| {
                let t = std::f64::consts::TAU * k as f64 / 32.0;
                [centre[0] + radius * t.cos(), centre[1] + radius * t.sin()]
            };
            for k in 0..32 {
                a.lines.push(PlanLine {
                    a: on_circle(k),
                    b: on_circle(k + 1),
                    layer: "S-GRID-IDEN".into(),
                    outside_crop: true,
                });
            }
            if !g.name.is_empty() {
                a.texts.push(PlanText {
                    at: [centre[0] - 0.28 * radius, centre[1] + 0.37 * radius],
                    value: g.name.clone(),
                    height_feet: 2.5 * scale / MM_PER_FOOT,
                    rotation: 0.0,
                    layer: "S-GRID-IDEN".into(),
                });
            }
        }
    }
    for d in facts.dimensions.iter().filter(|d| d.view == view.id) {
        for w in d.points.windows(2) {
            let (p, q) = ([w[0][0], w[0][1]], [w[1][0], w[1][1]]);
            if in_annotation_crop([(p[0] + q[0]) / 2.0, (p[1] + q[1]) / 2.0]) {
                a.dimensions.push([p, q]);
            }
        }
    }
    // The project's own lines: the view's detail lines, and model and separation lines in its
    // range, unless their category is hidden.
    let hidden_ids = facts.hidden_categories(view);
    for c in &facts.curves {
        let (lo, hi) = c
            .points
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| {
                (a.min(p[2]), b.max(p[2]))
            });
        // Property lines show in every plan, whatever their height.
        let shown = match c.view {
            Some(v) => v == view.id,
            None if c.kind == LineKind::PropertyLine => true,
            None => lo <= cut + 1e-3 && hi >= bottom - 1e-3,
        };
        // Most are stored with no category (-1); a hidden Lines category hides them all.
        let hidden = hidden_ids.contains(&LINES)
            || c.category
                .is_some_and(|k| k < -1 && hidden_ids.contains(&k));
        if !shown || hidden {
            continue;
        }
        // The export's layers: `Lines` on A-DETL, `<Room Separation>` on A-AREA, `Site: Property
        // Lines` on C-PROP-LINE, a floor's, toposolid's or pad's boundary on its category's.
        let layer = match c.kind {
            LineKind::Line => "A-DETL",
            LineKind::RoomSeparation => "A-AREA",
            LineKind::PropertyLine => "C-PROP-LINE",
            LineKind::Floor => "A-FLOR",
            LineKind::Toposolid => "C-TOPO",
            LineKind::BuildingPad => "C-BLDG-PADS",
        };
        for w in c.points.windows(2) {
            let (p, q) = ([w[0][0], w[0][1]], [w[1][0], w[1][1]]);
            if crop.is_some() && !(in_crop(p) || in_crop(q)) {
                continue;
            }
            a.lines.push(PlanLine {
                a: p,
                b: q,
                layer: layer.into(),
                outside_crop: false,
            });
        }
    }
    // Hidden categories (the view template's when one controls the view), by the names elements
    // carry.
    let hidden_categories: Vec<String> = hidden_ids
        .iter()
        .filter_map(|id| rvt::builtin_parameters::category_name(*id))
        .map(String::from)
        .collect();
    Ok((
        ViewWindow {
            bottom_feet: bottom,
            top_feet: cut,
            crop,
            hidden_categories,
            hidden_elements: view.hidden_elements.clone(),
            frame,
        },
        a,
    ))
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
    use rvt_dwg::write::{CadFormat, parse_version, write_drawing};
    if format == "pdf" {
        return rvt_dwg::pdf::render(drawing, layout, scale, &rvt_dwg::Limits::DEFAULT)
            .map(|p| p.bytes)
            .map_err(|e| e.to_string());
    }
    let (format, version) = if format == "dwg" {
        (CadFormat::Dwg, &cli.dwg_version)
    } else {
        (CadFormat::Dxf, &cli.dxf_version)
    };
    let version = parse_version(version, format).map_err(|e| e.to_string())?;
    let (bytes, notes) = write_drawing(drawing, format, version).map_err(|e| e.to_string())?;
    for n in notes {
        eprintln!("rvt-sheet: {n}");
    }
    Ok(bytes)
}

/// A sheet: each plan view on it drawn in model space, side by side, and shown through a
/// viewport where the view sits on the sheet (its internal origin at the viewport's `offset`, at
/// the view's scale); the title blocks' lines, the sheet's text notes and detail lines and each
/// view's title in paper space (`Layout1`). Millimetres.
#[cfg(feature = "dwg")]
fn sheet_drawing(
    rf: &mut RevitFile,
    model: &rvt::ifc::IfcModel,
    facts: &rvt::plan_views::ViewFacts,
    sheet: &rvt::plan_views::Sheet,
    symbols: &std::collections::BTreeMap<usize, Vec<rvt::ifc::entities::PlanPolyline>>,
) -> Result<rvt_dwg::Drawing2d, String> {
    use rvt_dwg::{Affine, Entity2d, Geom, Layer2d, Layout2d};
    const PAPER: &str = "*Paper_Space";
    let mm = |p: [f64; 2]| [p[0] * MM_PER_FOOT, p[1] * MM_PER_FOOT, 0.0];
    let mut out = rvt_dwg::Drawing2d {
        version: "AC1032".into(),
        units: 4,
        code_page: "ANSI_1252".into(),
        layers: vec![Layer2d::new("0", 7)],
        layouts: vec![
            Layout2d {
                name: "Model".into(),
                block: rvt_dwg::MODEL_SPACE.into(),
                tab_order: 0,
                is_model: true,
                limits: None,
            },
            Layout2d {
                name: "Layout1".into(),
                block: PAPER.into(),
                tab_order: 1,
                is_model: false,
                limits: None,
            },
        ],
        ..Default::default()
    };
    let ensure = |d: &mut rvt_dwg::Drawing2d, layer: &str, aci: i16| {
        if !d.layers.iter().any(|l| l.name == layer) {
            d.layers.push(Layer2d::new(layer, aci));
        }
    };
    let paper = |kind: &str, layer: &str, geom: Geom| {
        let mut e = Entity2d::new(kind, layer, geom);
        e.owner = PAPER.into();
        e
    };
    let mut next_x = 0.0;
    let mut viewport_id = 2;
    for vp in facts.viewports.iter().filter(|v| v.sheet == sheet.id) {
        let plan = facts.views.iter().find(|v| v.id == vp.view && !v.template);
        let section = facts
            .sections
            .iter()
            .find(|v| v.id == vp.view && !v.template);
        let (d, scale, name) = match (plan, section) {
            (Some(view), _) => {
                let (window, annotation) = view_window(model, facts, view)?;
                let options = SheetOptions {
                    window: Some(window),
                    annotation: Some(annotation),
                    symbols: symbols.clone(),
                    ..SheetOptions::default()
                };
                (
                    rvt::ifc::sheet::plan_drawing(model, &options),
                    view.scale.unwrap_or(100.0),
                    view.name.clone(),
                )
            }
            (None, Some(view)) => {
                let Some(window) = section_window(facts, view) else {
                    eprintln!(
                        "rvt-sheet: sheet {}: view {:?} has no crop box; not drawn",
                        sheet.number, view.name
                    );
                    continue;
                };
                (
                    rvt::ifc::sheet::projected_drawing(model, &window),
                    view.scale.unwrap_or(100.0),
                    view.name.clone(),
                )
            }
            _ => {
                eprintln!(
                    "rvt-sheet: sheet {}: view {} is not a plan, section or elevation view; not drawn yet",
                    sheet.number, vp.view
                );
                continue;
            }
        };
        let Some(b) = rvt_dwg::bounds(d.entities.iter().flat_map(|e| e.geom.samples(f64::MAX)))
        else {
            continue;
        };
        // Model space holds the views side by side, each moved along x clear of the last.
        let shift = next_x - b[0];
        next_x += b[2] - b[0] + (20.0 * scale).max(10_000.0);
        let t = Affine {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: shift,
            f: 0.0,
        };
        for l in d.layers {
            if !out.layers.iter().any(|x| x.name == l.name) {
                out.layers.push(l);
            }
        }
        for mut e in d.entities {
            e.geom = e.geom.transformed(&t);
            out.entities.push(e);
        }
        // Paper = offset + view coordinates / scale.
        let c = [(b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0];
        let o = mm(vp.offset);
        // The frame holds what the view draws with 2 mm to spare, so nothing lies on its edge.
        let (w, h) = ((b[2] - b[0]) / scale + 4.0, (b[3] - b[1]) / scale + 4.0);
        ensure(&mut out, "G-ANNO-NPLT", 7);
        out.entities.push(paper(
            "VIEWPORT",
            "G-ANNO-NPLT",
            Geom::Viewport {
                center: [o[0] + c[0] / scale, o[1] + c[1] / scale, 0.0],
                width: w,
                height: h,
                view_center: [c[0] + shift, c[1]],
                view_height: h * scale,
                id: viewport_id,
            },
        ));
        viewport_id += 1;
        // The view's title: its name at the title's origin, over a line of the title's length.
        if vp.label_length > 0.0 {
            ensure(&mut out, "G-ANNO-TTLB", 7);
            let at = mm(vp.label_origin);
            out.entities.push(paper(
                "LINE",
                "G-ANNO-TTLB",
                Geom::Line {
                    a: at,
                    b: [at[0] + vp.label_length * MM_PER_FOOT, at[1], 0.0],
                },
            ));
            out.entities.push(paper(
                "TEXT",
                "G-ANNO-TTLB",
                Geom::Text {
                    at: [at[0], at[1] + 2.0, 0.0],
                    height: 3.5,
                    rotation: 0.0,
                    value: name,
                },
            ));
        }
    }
    // Title blocks: their symbol's lines, placed by the instance.
    let blocks: Vec<&rvt::plan_views::TitleBlock> = facts
        .title_blocks
        .iter()
        .filter(|t| t.sheet == sheet.id)
        .collect();
    let wanted: std::collections::BTreeSet<u64> = blocks.iter().map(|t| t.symbol as u64).collect();
    let graphics = if wanted.is_empty() {
        Default::default()
    } else {
        rvt::ifc::plan_maps::symbol_sheet_graphics(rf, &wanted).unwrap_or_else(|e| {
            eprintln!("rvt-sheet: title block lines not read: {e}");
            Default::default()
        })
    };
    for t in blocks {
        let Some((cs, texts)) = u32::try_from(t.symbol).ok().and_then(|s| graphics.get(&s)) else {
            eprintln!(
                "rvt-sheet: sheet {}: title block {} drew nothing",
                sheet.number, t.id
            );
            continue;
        };
        ensure(&mut out, "G-ANNO-TTLB", 7);
        let [c, s] = t.x_dir;
        let place = |p: (f64, f64)| {
            mm([
                t.origin[0] + p.0 * c - p.1 * s,
                t.origin[1] + p.0 * s + p.1 * c,
            ])
        };
        for curve in cs {
            for w in rvt::ifc::sheet::symbol_points(curve).windows(2) {
                out.entities.push(paper(
                    "LINE",
                    "G-ANNO-TTLB",
                    Geom::Line {
                        a: place(w[0]),
                        b: place(w[1]),
                    },
                ));
            }
        }
        for text in texts {
            out.entities.push(paper(
                "TEXT",
                "G-ANNO-TTLB",
                Geom::Text {
                    at: place((text.at[0], text.at[1])),
                    height: text.height_feet * MM_PER_FOOT,
                    rotation: text.rotation + s.atan2(c),
                    value: text.value.clone(),
                },
            ));
        }
    }
    // The sheet's own text notes and detail lines, sheet sizes.
    for t in facts.texts.iter().filter(|t| t.view == sheet.id) {
        ensure(&mut out, "G-ANNO-TEXT", 7);
        out.entities.push(paper(
            "TEXT",
            "G-ANNO-TEXT",
            Geom::Text {
                at: mm([t.at[0], t.at[1]]),
                height: t.height.unwrap_or(2.5 / MM_PER_FOOT) * MM_PER_FOOT,
                rotation: t.rotation,
                value: t.value.clone(),
            },
        ));
    }
    for c in facts.curves.iter().filter(|c| c.view == Some(sheet.id)) {
        ensure(&mut out, "A-DETL", 7);
        for w in c.points.windows(2) {
            out.entities.push(paper(
                "LINE",
                "A-DETL",
                Geom::Line {
                    a: mm([w[0][0], w[0][1]]),
                    b: mm([w[1][0], w[1][1]]),
                },
            ));
        }
    }
    let limits = rvt_dwg::bounds(
        out.entities
            .iter()
            .filter(|e| e.owner == PAPER)
            .flat_map(|e| e.geom.samples(f64::MAX)),
    );
    out.layouts[1].limits = limits;
    Ok(out)
}

/// A section or elevation's axes, crop box and hidden categories and elements.
#[cfg(feature = "dwg")]
fn section_window(
    facts: &rvt::plan_views::ViewFacts,
    view: &rvt::plan_views::SectionView,
) -> Option<rvt::ifc::sheet::ProjectedWindow> {
    let c = view.crop.as_ref()?;
    Some(rvt::ifc::sheet::ProjectedWindow {
        right: view.right,
        up: view.up,
        toward_viewer: view.toward_viewer,
        crop_origin: c.origin,
        crop_axes: c.axes,
        crop_min: c.min,
        crop_max: c.max,
        hidden_categories: facts
            .section_hidden_categories(view)
            .iter()
            .filter_map(|id| rvt::builtin_parameters::category_name(*id))
            .map(String::from)
            .collect(),
        hidden_elements: view.hidden_elements.clone(),
    })
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
