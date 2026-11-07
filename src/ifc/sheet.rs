//! Sheet rendering — emit a 2D plan view of an `IfcModel`
//! as SVG.
//!
//! Each placed `BuildingElement` with a body is drawn as what that body
//! covers in plan ([`super::body_geometry::plan_outline`]): an extrusion's
//! profile turned by the element's rotation, holes left open, and the
//! convex hull of a swept, revolved or brep solid. An axis-aligned
//! rectangle stays a `<rect>`; every other outline is a `<path>`. Element
//! `ifc_type` drives the stroke colour (walls black, doors amber, columns
//! red, etc.).
//!
//! Output is a self-contained SVG document — no external
//! stylesheets, no JS. Drop it in a browser, embed in a report,
//! or convert to PDF via any SVG-to-PDF tool.

use super::IfcModel;
use super::body_geometry::{Body, Placement, Ring, element_body, plan_outline};
use super::entities::IfcEntity;
use std::fmt::Write;

/// Options controlling sheet SVG output.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SheetOptions {
    /// Plot width in SVG user units (pixels by default).
    pub width_px: u32,
    /// Plot height in SVG user units.
    pub height_px: u32,
    /// Margin inside the SVG viewBox, in user units.
    pub margin_px: f32,
    /// Show element name labels? (Large plans often want them off.)
    pub show_labels: bool,
    /// Background fill colour. `None` = transparent.
    pub background: Option<String>,
    /// The plan view to draw: its height band and crop. `None` draws every element of the model.
    #[serde(default)]
    pub window: Option<ViewWindow>,
    /// The view's own annotation (room tags, grid heads, text notes, dimensions), drawn in DXF and
    /// DWG output in place of element names when labels are on.
    #[serde(default)]
    pub annotation: Option<PlanAnnotation>,
}

/// What a plan view shows, in model feet: the elements whose height range meets
/// `bottom_feet..=top_feet` (the view depth up to the cut plane), inside `crop` when set.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ViewWindow {
    pub bottom_feet: f64,
    pub top_feet: f64,
    #[serde(default)]
    pub crop: Option<CropRect>,
}

/// A plan view's crop region: a rectangle `min..max` in the view's own axes, whose origin is
/// `origin` and whose x axis is `x_axis` (a unit vector in model plan coordinates; y is x turned
/// a quarter left). Model feet.
#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CropRect {
    pub origin: [f64; 2],
    pub x_axis: [f64; 2],
    pub min: [f64; 2],
    pub max: [f64; 2],
}

impl CropRect {
    /// A model point in the view's axes.
    fn local(&self, p: (f64, f64)) -> (f64, f64) {
        let (dx, dy) = (p.0 - self.origin[0], p.1 - self.origin[1]);
        let [c, s] = self.x_axis;
        (dx * c + dy * s, -dx * s + dy * c)
    }

    /// A point in the view's axes back in the model.
    fn model(&self, p: (f64, f64)) -> (f64, f64) {
        let [c, s] = self.x_axis;
        (
            self.origin[0] + p.0 * c - p.1 * s,
            self.origin[1] + p.0 * s + p.1 * c,
        )
    }

    /// Whether model point `p` lies in the crop.
    pub fn contains(&self, p: (f64, f64)) -> bool {
        let (x, y) = self.local(p);
        x >= self.min[0] && x <= self.max[0] && y >= self.min[1] && y <= self.max[1]
    }

    /// The part of segment `a`-`b` inside the crop (Liang-Barsky), in model coordinates.
    fn clip(&self, a: (f64, f64), b: (f64, f64)) -> Option<((f64, f64), (f64, f64))> {
        let (p, q) = (self.local(a), self.local(b));
        let (dx, dy) = (q.0 - p.0, q.1 - p.1);
        let (mut t0, mut t1) = (0.0f64, 1.0f64);
        for (num, den) in [
            (p.0 - self.min[0], -dx),
            (self.max[0] - p.0, dx),
            (p.1 - self.min[1], -dy),
            (self.max[1] - p.1, dy),
        ] {
            if den == 0.0 {
                if num < 0.0 {
                    return None;
                }
            } else {
                let t = num / den;
                if den < 0.0 {
                    t0 = t0.max(t);
                } else {
                    t1 = t1.min(t);
                }
            }
        }
        if t0 > t1 {
            return None;
        }
        let at = |t: f64| self.model((p.0 + dx * t, p.1 + dy * t));
        Some((at(t0), at(t1)))
    }

    /// The parts of closed ring `ring` inside the crop: the ring itself when wholly inside, else
    /// the open runs of its clipped edges.
    pub fn clip_ring(&self, ring: &[(f64, f64)]) -> Vec<(Vec<(f64, f64)>, bool)> {
        if ring.iter().all(|&p| self.contains(p)) {
            return vec![(ring.to_vec(), true)];
        }
        let mut runs: Vec<Vec<(f64, f64)>> = Vec::new();
        let n = ring.len();
        for i in 0..n {
            let Some((a, b)) = self.clip(ring[i], ring[(i + 1) % n]) else {
                continue;
            };
            match runs.last_mut() {
                Some(run)
                    if run
                        .last()
                        .is_some_and(|&l| (l.0 - a.0).hypot(l.1 - a.1) < 1e-9) =>
                {
                    run.push(b)
                }
                _ => runs.push(vec![a, b]),
            }
        }
        // The last run may continue into the first.
        if runs.len() > 1 {
            let (first, last) = (runs[0][0], *runs[runs.len() - 1].last().unwrap());
            if (first.0 - last.0).hypot(first.1 - last.1) < 1e-9 {
                let mut tail = runs.pop().unwrap();
                tail.extend(runs[0].drain(1..));
                runs[0] = tail;
            }
        }
        runs.into_iter().map(|r| (r, false)).collect()
    }
}

/// A plan view's annotation, in model feet.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PlanAnnotation {
    /// Text: room tags, grid and level heads, text notes.
    pub texts: Vec<PlanText>,
    /// Lines drawn with the annotation (grid lines), on `layer`.
    pub lines: Vec<PlanLine>,
    /// Dimensions: the two points measured between.
    pub dimensions: Vec<[[f64; 2]; 2]>,
}

/// One text of a plan's annotation.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PlanText {
    pub at: [f64; 2],
    pub value: String,
    /// Text height on the model, feet.
    pub height_feet: f64,
    pub rotation: f64,
    pub layer: String,
}

/// One line of a plan's annotation.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PlanLine {
    pub a: [f64; 2],
    pub b: [f64; 2],
    pub layer: String,
}

impl Default for SheetOptions {
    fn default() -> Self {
        Self {
            width_px: 1200,
            height_px: 800,
            margin_px: 40.0,
            show_labels: true,
            background: Some("#FFFFFF".into()),
            window: None,
            annotation: None,
        }
    }
}

/// Render `model` as an SVG plan view. Returns a string
/// ready to write to a `.svg` file or embed inline.
///
/// The plan is a top-down projection: X maps to SVG x, Y maps to
/// SVG y (flipped so +Y runs up, matching drafting conventions).
/// The model bounding box is the box of every element's plan outline,
/// fit to `options.width_px × options.height_px` preserving aspect.
///
/// Elements without a body or a `location_feet` are skipped
/// (nothing to draw).
///
/// # Examples
///
/// ```
/// use rvt::RevitFile;
/// use rvt::ifc::sheet::{SheetOptions, render_plan_svg};
/// use rvt::ifc::{Exporter, RvtDocExporter};
///
/// let mut rf = RevitFile::open("corpus/tier1/architectural-2024/architectural-2024.rvt")?;
/// let svg = render_plan_svg(&RvtDocExporter.export(&mut rf)?, &SheetOptions::default());
/// assert!(svg.contains("<svg"));
/// # Ok::<(), rvt::Error>(())
/// ```
pub fn render_plan_svg(model: &IfcModel, options: &SheetOptions) -> String {
    let footprints = collect_footprints(model, options.window.as_ref());
    let (min_x, min_y, max_x, max_y) = bbox_of_footprints(&footprints);
    let world_w = (max_x - min_x).max(1e-6) as f32;
    let world_h = (max_y - min_y).max(1e-6) as f32;

    let plot_w = options.width_px as f32 - 2.0 * options.margin_px;
    let plot_h = options.height_px as f32 - 2.0 * options.margin_px;
    let scale = (plot_w / world_w).min(plot_h / world_h);
    let offset_x = options.margin_px + (plot_w - world_w * scale) * 0.5;
    let offset_y = options.margin_px + (plot_h - world_h * scale) * 0.5;

    let mut out = String::with_capacity(1024 + footprints.len() * 128);
    write!(
        &mut out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" \
         height=\"{}\" viewBox=\"0 0 {} {}\">",
        options.width_px, options.height_px, options.width_px, options.height_px
    )
    .unwrap();
    if let Some(bg) = options.background.as_deref() {
        write!(
            &mut out,
            "<rect width=\"{}\" height=\"{}\" fill=\"{}\"/>",
            options.width_px, options.height_px, bg
        )
        .unwrap();
    }
    // Border around the plot area so the sheet feels like a drawing.
    write!(
        &mut out,
        "<rect x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" \
         fill=\"none\" stroke=\"#CDCED0\" stroke-width=\"1\"/>",
        options.margin_px, options.margin_px, plot_w, plot_h,
    )
    .unwrap();

    let to_svg = |(x, y): (f64, f64)| {
        (
            offset_x + (x - min_x) as f32 * scale,
            offset_y + (max_y - y) as f32 * scale,
        )
    };
    for fp in &footprints {
        let (x0, y0, x1, y1) = ring_bounds(&fp.outer);
        let (sx, sy) = to_svg((x0, y1));
        let sw = ((x1 - x0) as f32 * scale).max(1.0);
        let sh = ((y1 - y0) as f32 * scale).max(1.0);
        let (stroke, fill) = colors_for_ifc_type(&fp.ifc_type);
        if fp.axis_aligned_rectangle {
            write!(
                &mut out,
                "<rect x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" \
                 fill=\"{}\" stroke=\"{}\" stroke-width=\"1\"/>",
                sx, sy, sw, sh, fill, stroke
            )
            .unwrap();
        } else {
            let mut d = String::new();
            for ring in std::iter::once(&fp.outer).chain(&fp.holes) {
                for (i, &p) in ring.iter().enumerate() {
                    let (px, py) = to_svg(p);
                    write!(
                        &mut d,
                        "{}{:.1} {:.1} ",
                        if i == 0 { "M" } else { "L" },
                        px,
                        py
                    )
                    .unwrap();
                }
                d.push_str("Z ");
            }
            write!(
                &mut out,
                "<path d=\"{}\" fill-rule=\"evenodd\" fill=\"{}\" stroke=\"{}\" \
                 stroke-width=\"1\"/>",
                d.trim_end(),
                fill,
                stroke
            )
            .unwrap();
        }
        if options.show_labels {
            write!(
                &mut out,
                "<text x=\"{:.1}\" y=\"{:.1}\" font-size=\"9\" \
                 fill=\"#2C2E30\" font-family=\"sans-serif\">{}</text>",
                sx + sw * 0.5,
                sy + sh * 0.5,
                xml_escape(&fp.name)
            )
            .unwrap();
        }
    }
    out.push_str("</svg>");
    out
}

struct Footprint {
    /// Plan outline in model feet, and its holes.
    outer: Ring,
    holes: Vec<Ring>,
    /// A rectangle on the model axes, drawn as a `<rect>`.
    axis_aligned_rectangle: bool,
    ifc_type: String,
    name: String,
}

fn collect_footprints(model: &IfcModel, window: Option<&ViewWindow>) -> Vec<Footprint> {
    let mut out = Vec::new();
    for ent in &model.entities {
        let IfcEntity::BuildingElement {
            ifc_type,
            name,
            location_feet,
            rotation_radians,
            ..
        } = ent
        else {
            continue;
        };
        // An opening is a void in its host, not something drawn.
        if ifc_type == "IFCOPENINGELEMENT" {
            continue;
        }
        let Some(location) = location_feet else {
            continue;
        };
        let Some(body) = element_body(ent, &model.representation_maps) else {
            continue;
        };
        let placement = Placement::new(Some(*location), *rotation_radians);
        if let Some(w) = window {
            if !meets_band(body, &placement, w) {
                continue;
            }
        }
        let Some((outer, holes)) = plan_outline(body, &placement) else {
            continue;
        };
        if let Some(crop) = window.and_then(|w| w.crop.as_ref()) {
            if !outer.iter().any(|&p| crop.contains(p))
                && !(0..outer.len())
                    .any(|i| crop.clip(outer[i], outer[(i + 1) % outer.len()]).is_some())
            {
                continue;
            }
        }
        let on_axes = placement.sin.abs() < 1e-12 || placement.cos.abs() < 1e-12;
        let axis_aligned_rectangle = on_axes
            && matches!(body, Body::Extrusion(extrusion) if extrusion.profile_override.is_none());
        out.push(Footprint {
            outer,
            holes,
            axis_aligned_rectangle,
            ifc_type: ifc_type.clone(),
            name: name.clone(),
        });
    }
    // Plates and spaces underlie the plan: drawn after the walls, their
    // fills would hide every wall, column and door on their storey.
    out.sort_by_key(|fp| !is_underlay(&fp.ifc_type));
    out
}

/// Whether the body's height range meets the window's band. A body with no mesh is placed by its
/// origin's height.
fn meets_band(body: Body<'_>, placement: &Placement, w: &ViewWindow) -> bool {
    let (lo, hi) = super::body_geometry::body_mesh(body)
        .filter(|m| !m.vertices.is_empty())
        .map(|m| {
            m.vertices
                .iter()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
                    let z = placement.apply(*v)[2];
                    (lo.min(z), hi.max(z))
                })
        })
        .unwrap_or((placement.origin[2], placement.origin[2]));
    // A thousandth of a foot: a slab whose top is the level counts as in the band.
    lo <= w.top_feet + 1e-3 && hi >= w.bottom_feet - 1e-3
}

/// Types drawn beneath the rest of the plan.
fn is_underlay(ifc_type: &str) -> bool {
    matches!(
        ifc_type,
        "IFCSLAB" | "IFCROOF" | "IFCCOVERING" | "IFCSPACE" | "IFCPLATE"
    )
}

fn ring_bounds(ring: &[(f64, f64)]) -> (f64, f64, f64, f64) {
    ring.iter().fold(
        (
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ),
        |(x0, y0, x1, y1), &(x, y)| (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
    )
}

fn bbox_of_footprints(fps: &[Footprint]) -> (f64, f64, f64, f64) {
    if fps.is_empty() {
        return (0.0, 0.0, 100.0, 100.0);
    }
    fps.iter().map(|fp| ring_bounds(&fp.outer)).fold(
        (
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ),
        |(a, b, c, d), (x0, y0, x1, y1)| (a.min(x0), b.min(y0), c.max(x1), d.max(y1)),
    )
}

/// Per-category colour mapping, from the viewer design system's
/// ramps: walls black, windows blue, doors amber, columns and beams red,
/// slabs gray, stairs dark amber and furniture green. Each fill is the
/// light tint of its stroke's ramp.
fn colors_for_ifc_type(ifc_type: &str) -> (&'static str, &'static str) {
    match ifc_type {
        "IFCWALL" | "IFCWALLSTANDARDCASE" => ("#000000", "#E4E5E7"),
        "IFCDOOR" => ("#D97706", "#FFFBEB"),
        "IFCWINDOW" => ("#1881FF", "#E6F1FF"),
        "IFCCOLUMN" => ("#DC2626", "#FEE2E2"),
        "IFCBEAM" | "IFCMEMBER" => ("#F87171", "#FEF2F2"),
        "IFCSLAB" | "IFCROOF" | "IFCCOVERING" => ("#909398", "#F5F5F7"),
        "IFCSTAIR" | "IFCRAILING" => ("#92400E", "#FEF3C7"),
        "IFCFURNITURE" | "IFCFURNISHINGELEMENT" => ("#16A34A", "#F0FDF4"),
        _ => ("#4C4E52", "#FAFAFA"),
    }
}

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

/// The plan as a 2D drawing for DXF and DWG export (`rvt-sheet --format dxf|dwg`): each element's
/// plan outline (and its holes) as a closed polyline on a layer named for its category in the
/// style of the AIA layer standard (`A-WALL`, `A-DOOR`, `S-COLS`, …), in millimetres
/// (`$INSUNITS` 4) at the model's own origin, with the element name as text when
/// `options.show_labels` is set.
///
/// With `options.window`, only the elements in the view's height band are drawn, and outlines
/// crossing its crop are cut at it (open polylines). With `options.annotation`, labels are the
/// view's own annotation (text notes, room tags, grid lines and heads, dimensions) in place of
/// element names. Outlines are still the elements' plan outlines, not Revit's cut and projection
/// lines, and line styles are not drawn.
#[cfg(feature = "dwg")]
pub fn plan_drawing(model: &IfcModel, options: &SheetOptions) -> rvt_dwg::Drawing2d {
    use rvt_dwg::{Drawing2d, Entity2d, Geom, Layer2d, Layout2d, MODEL_SPACE};
    const MM_PER_FOOT: f64 = 304.8;
    let mm = |r: &Ring| {
        r.iter()
            .map(|&(x, y)| [x * MM_PER_FOOT, y * MM_PER_FOOT, 0.0])
            .collect::<Vec<_>>()
    };
    let mut d = Drawing2d {
        version: "AC1032".into(),
        units: 4,
        code_page: "ANSI_1252".into(),
        layers: vec![Layer2d::new("0", 7)],
        layouts: vec![Layout2d {
            name: "Model".into(),
            block: MODEL_SPACE.into(),
            tab_order: 0,
            is_model: true,
            limits: None,
        }],
        ..Default::default()
    };
    let crop = options.window.as_ref().and_then(|w| w.crop.as_ref());
    let ensure = |d: &mut Drawing2d, layer: &str, aci: i16| {
        if !d.layers.iter().any(|l| l.name == layer) {
            d.layers.push(Layer2d::new(layer, aci));
        }
    };
    for fp in collect_footprints(model, options.window.as_ref()) {
        let (layer, aci) = layer_for_ifc_type(&fp.ifc_type);
        ensure(&mut d, layer, aci);
        for ring in std::iter::once(&fp.outer).chain(&fp.holes) {
            let parts = match crop {
                Some(c) => c.clip_ring(ring),
                None => vec![(ring.clone(), true)],
            };
            for (pts, closed) in parts {
                d.entities.push(Entity2d::new(
                    "LWPOLYLINE",
                    layer,
                    Geom::Polyline {
                        pts: mm(&pts),
                        closed,
                        z: 0.0,
                    },
                ));
            }
        }
        if options.show_labels && options.annotation.is_none() && !fp.name.is_empty() {
            let (x0, y0, x1, y1) = ring_bounds(&fp.outer);
            let at = [
                (x0 + x1) / 2.0 * MM_PER_FOOT,
                (y0 + y1) / 2.0 * MM_PER_FOOT,
                0.0,
            ];
            if !d.layers.iter().any(|l| l.name == "A-ANNO-IDEN") {
                d.layers.push(Layer2d::new("A-ANNO-IDEN", 7));
            }
            d.entities.push(Entity2d::new(
                "TEXT",
                "A-ANNO-IDEN",
                Geom::Text {
                    at,
                    height: 150.0,
                    rotation: 0.0,
                    value: fp.name.clone(),
                },
            ));
        }
    }
    if let Some(a) = options.annotation.as_ref().filter(|_| options.show_labels) {
        let at = |p: [f64; 2]| [p[0] * MM_PER_FOOT, p[1] * MM_PER_FOOT, 0.0];
        for l in &a.lines {
            let segment = match crop {
                Some(c) => c.clip((l.a[0], l.a[1]), (l.b[0], l.b[1])),
                None => Some(((l.a[0], l.a[1]), (l.b[0], l.b[1]))),
            };
            if let Some((p, q)) = segment {
                ensure(&mut d, &l.layer, 8);
                d.entities.push(Entity2d::new(
                    "LINE",
                    &l.layer,
                    Geom::Line {
                        a: at([p.0, p.1]),
                        b: at([q.0, q.1]),
                    },
                ));
            }
        }
        for t in &a.texts {
            ensure(&mut d, &t.layer, 7);
            d.entities.push(Entity2d::new(
                "TEXT",
                &t.layer,
                Geom::Text {
                    at: at(t.at),
                    height: t.height_feet * MM_PER_FOOT,
                    rotation: t.rotation,
                    value: t.value.clone(),
                },
            ));
        }
        for [p, q] in &a.dimensions {
            let measurement = (q[0] - p[0]).hypot(q[1] - p[1]) * MM_PER_FOOT;
            if measurement < 1e-6 {
                continue;
            }
            ensure(&mut d, "A-ANNO-DIMS", 7);
            d.entities.push(Entity2d::new(
                "DIMENSION",
                "A-ANNO-DIMS",
                Geom::Dimension {
                    block: String::new(),
                    measurement,
                    text: None,
                    at: at([(p[0] + q[0]) / 2.0, (p[1] + q[1]) / 2.0]),
                    points: vec![at(*p), at(*q)],
                },
            ));
        }
    }
    d
}

/// Layer (in the style of the AIA standard) and colour index for an IFC type.
#[cfg(feature = "dwg")]
fn layer_for_ifc_type(ifc_type: &str) -> (&'static str, i16) {
    match ifc_type {
        "IFCWALL" | "IFCWALLSTANDARDCASE" | "IFCCURTAINWALL" => ("A-WALL", 7),
        "IFCDOOR" => ("A-DOOR", 30),
        "IFCWINDOW" => ("A-GLAZ", 5),
        "IFCCOLUMN" => ("S-COLS", 1),
        "IFCBEAM" | "IFCMEMBER" => ("S-BEAM", 10),
        "IFCSLAB" => ("A-FLOR", 8),
        "IFCROOF" => ("A-ROOF", 8),
        "IFCCOVERING" => ("A-CLNG", 8),
        "IFCSTAIR" | "IFCSTAIRFLIGHT" => ("A-FLOR-STRS", 32),
        "IFCRAILING" => ("A-FLOR-HRAL", 32),
        "IFCFURNITURE" | "IFCFURNISHINGELEMENT" => ("I-FURN", 3),
        "IFCSPACE" => ("A-AREA", 4),
        "IFCPLATE" => ("A-GLAZ-CWMG", 5),
        _ => ("A-GENM", 8),
    }
}
