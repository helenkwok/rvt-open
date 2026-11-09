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
    /// Each family instance's plan symbol in model feet, by its index in `model.entities`
    /// ([`super::plan_maps::instance_plan_symbols`]): drawn in DXF and DWG output with the
    /// instance's outline, on its layer.
    #[serde(skip)]
    pub symbols: std::collections::BTreeMap<usize, Vec<super::entities::PlanPolyline>>,
}

/// What a plan view shows, in model feet: the elements whose height range meets
/// `bottom_feet..=top_feet` (the view depth up to the cut plane), inside `crop` when set, leaving
/// out the categories (by Revit name) and elements (by ElementId) the view hides.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ViewWindow {
    pub bottom_feet: f64,
    pub top_feet: f64,
    #[serde(default)]
    pub crop: Option<CropRect>,
    #[serde(default)]
    pub hidden_categories: Vec<String>,
    #[serde(default)]
    pub hidden_elements: Vec<i64>,
    /// The view's right and up directions in model plan coordinates, when they are not the
    /// model's axes (a site plan turned to true north): DXF and DWG output is drawn in the view's
    /// own axes about the internal origin, as the view shows it.
    #[serde(default)]
    pub frame: Option<[[f64; 2]; 2]>,
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
    pub fn clip(&self, a: (f64, f64), b: (f64, f64)) -> Option<((f64, f64), (f64, f64))> {
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
    /// Drawn where it is even outside the crop (a grid head beyond the model crop, inside the
    /// annotation crop).
    #[serde(default)]
    pub outside_crop: bool,
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
            symbols: Default::default(),
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
    /// Index into `model.entities` (its layer in DXF and DWG output).
    #[cfg_attr(not(feature = "dwg"), allow(dead_code))]
    index: usize,
    /// For an element wholly below the view's cut plane, the edges of its body seen from above
    /// (projection lines), drawn in DXF and DWG output in place of its outline.
    #[cfg_attr(not(feature = "dwg"), allow(dead_code))]
    edges: Vec<((f64, f64), (f64, f64))>,
    /// For a stair or railing the cut plane cuts, the edges of its body above the cut plane,
    /// drawn as overhead lines on its layer's `-MBND` layer.
    #[cfg_attr(not(feature = "dwg"), allow(dead_code))]
    overhead: Vec<((f64, f64), (f64, f64))>,
    /// The body's height range, model feet.
    heights: (f64, f64),
    /// A roof, or one of a roof's slabs (category Roofs).
    roof: bool,
    /// Wholly hidden under a roof: neither its lines nor its outline are drawn.
    #[cfg_attr(not(feature = "dwg"), allow(dead_code))]
    covered: bool,
}

fn collect_footprints(model: &IfcModel, window: Option<&ViewWindow>) -> Vec<Footprint> {
    let mut out = Vec::new();
    let sets = window
        .map(|_| super::element_table::element_sets(model))
        .unwrap_or_default();
    // What each element hosts (doors, windows, openings), for walls cut at the cut plane.
    let mut hosted: std::collections::BTreeMap<usize, Vec<usize>> = Default::default();
    if window.is_some() {
        for (i, e) in model.entities.iter().enumerate() {
            if let IfcEntity::BuildingElement {
                host_element_index: Some(h),
                ..
            } = e
            {
                hosted.entry(*h).or_default().push(i);
            }
        }
    }
    for (index, ent) in model.entities.iter().enumerate() {
        let IfcEntity::BuildingElement {
            ifc_type,
            name,
            type_guid,
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
        let mut roof = ifc_type == "IFCROOF";
        if let Some(w) = window {
            // `type_guid` holds the element's ElementId.
            let id = type_guid.as_deref().and_then(|t| t.parse::<i64>().ok());
            if id.is_some_and(|id| w.hidden_elements.contains(&id)) {
                continue;
            }
            let category = sets
                .get(&index)
                .and_then(|s| super::element_table::category_in(s));
            roof |= category.as_deref() == Some("Roofs");
            if category.is_some_and(|c| w.hidden_categories.contains(&c)) {
                continue;
            }
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
        let cut = window.map(|w| w.top_feet);
        let pieces = match (cut, hosted.get(&index)) {
            (Some(cut), Some(h))
                if matches!(ifc_type.as_str(), "IFCWALL" | "IFCWALLSTANDARDCASE") =>
            {
                cut_at_openings(model, &outer, h, cut)
            }
            _ => vec![outer],
        };
        let whole = pieces.len() == 1;
        let heights = height_range(body, &placement);
        // Below the cut plane an element is seen, not cut: its projection lines.
        // A stair or railing the cut plane cuts: below it seen, above it overhead.
        let (edges, overhead) = match window {
            Some(w) if !is_underlay(ifc_type) => {
                let below = height_range(body, &placement).1 <= w.top_feet + 1e-3;
                let overhead = matches!(
                    ifc_type.as_str(),
                    "IFCSTAIR" | "IFCSTAIRFLIGHT" | "IFCRAILING"
                );
                if below || overhead {
                    projected_edges(body, &placement, w.top_feet)
                } else {
                    (Vec::new(), Vec::new())
                }
            }
            _ => (Vec::new(), Vec::new()),
        };
        for outer in pieces {
            out.push(Footprint {
                outer,
                holes: if whole { holes.clone() } else { Vec::new() },
                axis_aligned_rectangle,
                ifc_type: ifc_type.clone(),
                name: name.clone(),
                index,
                edges: edges.clone(),
                overhead: overhead.clone(),
                heights,
                roof,
                covered: false,
            });
        }
    }
    if window.is_some() {
        hide_under_roofs(&mut out);
    }
    // Plates and spaces underlie the plan: drawn after the walls, their
    // fills would hide every wall, column and door on their storey.
    out.sort_by_key(|fp| !is_underlay(&fp.ifc_type));
    out
}

/// Hidden lines under roofs: a roof in the view hides the projection lines of what lies under it, as a site plan shows a building's roof and not its
/// rooms. A line is hidden where it lies inside the roof's outline (and not in one of its holes)
/// and the element it belongs to is no higher than the roof's lowest point.
// ponytail: the roof's lowest point, not its surface above each line, so a gable wall reaching
// above the eaves stays drawn; test each line's height against the roof's faces if that matters.
fn hide_under_roofs(fps: &mut [Footprint]) {
    let roofs: Vec<(Ring, Vec<Ring>, f64)> = fps
        .iter()
        .filter(|f| f.roof)
        .map(|f| (f.outer.clone(), f.holes.clone(), f.heights.0))
        .collect();
    if roofs.is_empty() {
        return;
    }
    let inside = |p: (f64, f64), ring: &[(f64, f64)]| {
        let mut odd = false;
        for i in 0..ring.len() {
            let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
            if (a.1 > p.1) != (b.1 > p.1) && p.0 < a.0 + (p.1 - a.1) / (b.1 - a.1) * (b.0 - a.0) {
                odd = !odd;
            }
        }
        odd
    };
    for fp in fps.iter_mut() {
        if fp.roof || fp.edges.is_empty() {
            continue;
        }
        let over: Vec<&(Ring, Vec<Ring>, f64)> = roofs
            .iter()
            .filter(|r| fp.heights.1 <= r.2 + 1e-3)
            .collect();
        if over.is_empty() {
            continue;
        }
        let hidden = |p: (f64, f64)| {
            over.iter()
                .any(|(o, holes, _)| inside(p, o) && !holes.iter().any(|h| inside(p, h)))
        };
        let mut kept = Vec::new();
        for &(p, q) in &fp.edges {
            // Where the line crosses a roof's outline or holes, then each piece by its middle.
            let mut ts = vec![0.0, 1.0];
            for (o, holes, _) in &over {
                for ring in std::iter::once(o).chain(holes.iter()) {
                    for i in 0..ring.len() {
                        let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
                        let d = (q.0 - p.0) * (b.1 - a.1) - (q.1 - p.1) * (b.0 - a.0);
                        if d.abs() < 1e-12 {
                            continue;
                        }
                        let t = ((a.0 - p.0) * (b.1 - a.1) - (a.1 - p.1) * (b.0 - a.0)) / d;
                        let u = ((a.0 - p.0) * (q.1 - p.1) - (a.1 - p.1) * (q.0 - p.0)) / d;
                        if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) {
                            ts.push(t);
                        }
                    }
                }
            }
            ts.sort_by(f64::total_cmp);
            let at = |t: f64| (p.0 + t * (q.0 - p.0), p.1 + t * (q.1 - p.1));
            for w in ts.windows(2) {
                if w[1] - w[0] > 1e-9 && !hidden(at((w[0] + w[1]) / 2.0)) {
                    kept.push((at(w[0]), at(w[1])));
                }
            }
        }
        fp.covered = kept.is_empty();
        fp.edges = kept;
    }
}

/// The body's height range, model feet. A body with no mesh is placed by its origin's height.
fn height_range(body: Body<'_>, placement: &Placement) -> (f64, f64) {
    super::body_geometry::body_mesh(body)
        .filter(|m| !m.vertices.is_empty())
        .map(|m| {
            m.vertices
                .iter()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
                    let z = placement.apply(*v)[2];
                    (lo.min(z), hi.max(z))
                })
        })
        .unwrap_or((placement.origin[2], placement.origin[2]))
}

/// The body's edges as a plan sees them from above, in model feet: its mesh's boundary edges and
/// the edges where its faces meet at more than 30 degrees, projected, without the ones seen end on
/// (vertical edges) and with each projected segment once.
// ponytail: no hidden-line removal, so edges under an element's own faces are drawn too; add a
// depth test against its faces if precision needs it.
type Segments = Vec<((f64, f64), (f64, f64))>;
/// An edge by its two quantised ends, and the normals of the faces that meet at it with its ends.
type EdgeKey = ([i64; 3], [i64; 3]);
type EdgeFaces = (Vec<[f64; 3]>, [f64; 3], [f64; 3]);

fn projected_edges(body: Body<'_>, placement: &Placement, cut: f64) -> (Segments, Segments) {
    let Some(mesh) = super::body_geometry::body_mesh(body) else {
        return (Vec::new(), Vec::new());
    };
    let q = |p: [f64; 3]| p.map(|v| (v * 1e5).round() as i64);
    let normal = |t: &[u32; 3]| -> Option<[f64; 3]> {
        let [a, b, c] = t.map(|i| mesh.vertices.get(i as usize).copied());
        let (a, b, c) = (a?, b?, c?);
        let (u, v) = (
            [0, 1, 2].map(|i| b[i] - a[i]),
            [0, 1, 2].map(|i| c[i] - a[i]),
        );
        let n = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        (l > 1e-12).then(|| n.map(|x| x / l))
    };
    let mut faces: std::collections::HashMap<EdgeKey, EdgeFaces> = Default::default();
    for t in &mesh.triangles {
        let Some(n) = normal(t) else { continue };
        for k in 0..3 {
            let (a, b) = (
                mesh.vertices[t[k] as usize],
                mesh.vertices[t[(k + 1) % 3] as usize],
            );
            let (ka, kb) = (q(a), q(b));
            let key = if ka <= kb { (ka, kb) } else { (kb, ka) };
            faces
                .entry(key)
                .or_insert_with(|| (Vec::new(), a, b))
                .0
                .push(n);
        }
    }
    let mut seen = std::collections::HashSet::new();
    let (mut below, mut above) = (Vec::new(), Vec::new());
    let k2 = |p: (f64, f64)| ((p.0 * 1e4).round() as i64, (p.1 * 1e4).round() as i64);
    let mut push = |out: &mut Segments, p: [f64; 3], r: [f64; 3], tag: bool| {
        let (p, r) = ((p[0], p[1]), (r[0], r[1]));
        if (p.0 - r.0).hypot(p.1 - r.1) < 1e-4 {
            return;
        }
        let key = if k2(p) <= k2(r) {
            (k2(p), k2(r))
        } else {
            (k2(r), k2(p))
        };
        if seen.insert((key, tag)) {
            out.push((p, r));
        }
    };
    for (normals, a, b) in faces.into_values() {
        let sharp = normals.len() != 2 || {
            let (m, n) = (normals[0], normals[1]);
            m[0] * n[0] + m[1] * n[1] + m[2] * n[2] < 0.866
        };
        if !sharp {
            continue;
        }
        let (pa, pb) = (placement.apply(a), placement.apply(b));
        // Split at the cut plane.
        let (lo, hi) = if pa[2] <= pb[2] { (pa, pb) } else { (pb, pa) };
        if hi[2] <= cut {
            push(&mut below, lo, hi, false);
        } else if lo[2] >= cut {
            push(&mut above, lo, hi, true);
        } else {
            let t = (cut - lo[2]) / (hi[2] - lo[2]);
            let m = [0, 1, 2].map(|i| lo[i] + t * (hi[i] - lo[i]));
            push(&mut below, lo, m, false);
            push(&mut above, m, hi, true);
        }
    }
    (below, above)
}

/// Whether the body's height range meets the window's band.
fn meets_band(body: Body<'_>, placement: &Placement, w: &ViewWindow) -> bool {
    let (lo, hi) = height_range(body, placement);
    // A thousandth of a foot: a slab whose top is the level counts as in the band.
    lo <= w.top_feet + 1e-3 && hi >= w.bottom_feet - 1e-3
}

/// A wall as the view's cut plane cuts it: its plan outline split where a door, window or
/// opening it hosts crosses the cut plane, as a plan draws a wall broken at its openings. Each
/// hosted element's outline, measured along the wall's axis, is taken out of the wall; the
/// pieces between are returned (the whole outline when nothing it hosts is cut).
fn cut_at_openings(model: &IfcModel, outer: &Ring, hosted: &[usize], cut_feet: f64) -> Vec<Ring> {
    // The wall's axis: its outline's longest edge.
    let Some(axis) = (0..outer.len())
        .map(|i| {
            let (p, q) = (outer[i], outer[(i + 1) % outer.len()]);
            (q.0 - p.0, q.1 - p.1)
        })
        .max_by(|a, b| a.0.hypot(a.1).total_cmp(&b.0.hypot(b.1)))
        .map(|(x, y)| (x / x.hypot(y), y / x.hypot(y)))
        .filter(|a| a.0.is_finite() && a.1.is_finite())
    else {
        return vec![outer.clone()];
    };
    let origin = outer[0];
    let along = |p: (f64, f64)| (p.0 - origin.0) * axis.0 + (p.1 - origin.1) * axis.1;
    let mut gaps: Vec<(f64, f64)> = Vec::new();
    for &i in hosted {
        let ent = &model.entities[i];
        let IfcEntity::BuildingElement {
            location_feet: Some(location),
            rotation_radians,
            ..
        } = ent
        else {
            continue;
        };
        let Some(body) = element_body(ent, &model.representation_maps) else {
            continue;
        };
        let p = Placement::new(Some(*location), *rotation_radians);
        let (lo, hi) = height_range(body, &p);
        if !(lo < cut_feet && cut_feet < hi) {
            continue;
        }
        let Some((ring, _)) = plan_outline(body, &p) else {
            continue;
        };
        let (a, b) = ring
            .iter()
            .map(|&q| along(q))
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), t| {
                (a.min(t), b.max(t))
            });
        if b - a > 1e-6 {
            gaps.push((a, b));
        }
    }
    if gaps.is_empty() {
        return vec![outer.clone()];
    }
    gaps.sort_by(|x, y| x.0.total_cmp(&y.0));
    let (u0, u1) = outer
        .iter()
        .map(|&q| along(q))
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), t| {
            (a.min(t), b.max(t))
        });
    let mut pieces = Vec::new();
    let mut from = u0;
    for (a, b) in gaps.into_iter().chain(std::iter::once((u1, u1))) {
        if a > from + 1e-6 {
            let piece = clip_half(
                &clip_half(outer, origin, axis, from, 1.0),
                origin,
                axis,
                a,
                -1.0,
            );
            if piece.len() >= 3 {
                pieces.push(piece);
            }
        }
        from = from.max(b);
    }
    pieces
}

/// The part of `ring` where `sign * (along(p) - at) >= 0`, `along` measured on `axis` from
/// `origin` (one Sutherland-Hodgman pass; exact for a convex outline).
fn clip_half(ring: &Ring, origin: (f64, f64), axis: (f64, f64), at: f64, sign: f64) -> Ring {
    let side = |p: (f64, f64)| sign * ((p.0 - origin.0) * axis.0 + (p.1 - origin.1) * axis.1 - at);
    let mut out = Vec::new();
    for i in 0..ring.len() {
        let (p, q) = (ring[i], ring[(i + 1) % ring.len()]);
        let (sp, sq) = (side(p), side(q));
        if sp >= 0.0 {
            out.push(p);
        }
        if (sp >= 0.0) != (sq >= 0.0) {
            let t = sp / (sp - sq);
            out.push((p.0 + t * (q.0 - p.0), p.1 + t * (q.1 - p.1)));
        }
    }
    out
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
        "IFCFURNITURE" | "IFCFURNISHINGELEMENT" | "IFCSYSTEMFURNITUREELEMENT" => {
            ("#16A34A", "#F0FDF4")
        }
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
/// element names. Elements wholly below the cut plane are drawn as their projection lines,
/// stairs and railings above it as overhead lines (`-MBND`), `options.symbols` (family plan
/// symbols) with their instances, and a view with turned axes (`ViewWindow::frame`) in its own
/// axes. Line styles are not drawn.
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
    let sets = super::element_table::element_sets(model);
    let mut symbol_drawn = std::collections::BTreeSet::new();
    for fp in collect_footprints(model, options.window.as_ref()) {
        let own = sets.get(&fp.index).map(Vec::as_slice).unwrap_or_default();
        let (layer, aci) = layer_for(own, &fp.ifc_type);
        ensure(&mut d, layer, aci);
        if let Some(curves) = options.symbols.get(&fp.index) {
            if symbol_drawn.insert(fp.index) {
                for c in curves {
                    let pts = symbol_points(c);
                    for w in pts.windows(2) {
                        let segment = match crop {
                            Some(cr) => cr.clip(w[0], w[1]),
                            None => Some((w[0], w[1])),
                        };
                        if let Some((p, q)) = segment {
                            d.entities.push(Entity2d::new(
                                "LINE",
                                layer,
                                Geom::Line {
                                    a: [p.0 * MM_PER_FOOT, p.1 * MM_PER_FOOT, 0.0],
                                    b: [q.0 * MM_PER_FOOT, q.1 * MM_PER_FOOT, 0.0],
                                },
                            ));
                        }
                    }
                }
            }
        }
        for &(p, q) in &fp.edges {
            let segment = match crop {
                Some(c) => c.clip(p, q),
                None => Some((p, q)),
            };
            if let Some((p, q)) = segment {
                d.entities.push(Entity2d::new(
                    "LINE",
                    layer,
                    Geom::Line {
                        a: [p.0 * MM_PER_FOOT, p.1 * MM_PER_FOOT, 0.0],
                        b: [q.0 * MM_PER_FOOT, q.1 * MM_PER_FOOT, 0.0],
                    },
                ));
            }
        }
        if !fp.overhead.is_empty() {
            let overhead = format!("{layer}-MBND");
            ensure(&mut d, &overhead, aci);
            for &(p, q) in &fp.overhead {
                let segment = match crop {
                    Some(c) => c.clip(p, q),
                    None => Some((p, q)),
                };
                if let Some((p, q)) = segment {
                    d.entities.push(Entity2d::new(
                        "LINE",
                        &overhead,
                        Geom::Line {
                            a: [p.0 * MM_PER_FOOT, p.1 * MM_PER_FOOT, 0.0],
                            b: [q.0 * MM_PER_FOOT, q.1 * MM_PER_FOOT, 0.0],
                        },
                    ));
                }
            }
        }
        let rings = if fp.edges.is_empty() && fp.overhead.is_empty() && !fp.covered {
            std::iter::once(&fp.outer)
                .chain(&fp.holes)
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        for ring in rings {
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
            let segment = match crop.filter(|_| !l.outside_crop) {
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
    if let Some([h, v]) = options.window.as_ref().and_then(|w| w.frame) {
        let t = rvt_dwg::Affine {
            a: h[0],
            b: v[0],
            c: h[1],
            d: v[1],
            e: 0.0,
            f: 0.0,
        };
        for e in &mut d.entities {
            e.geom = e.geom.transformed(&t);
        }
    }
    d
}

/// A plan symbol curve's points in plan, an arc (start, a point on it, end) as 32 chords.
#[cfg(feature = "dwg")]
fn symbol_points(c: &super::entities::PlanPolyline) -> Vec<(f64, f64)> {
    let p: Vec<(f64, f64)> = c.points_feet.iter().map(|p| (p[0], p[1])).collect();
    if !c.arc || p.len() != 3 {
        return p;
    }
    let (a, m, b) = (p[0], p[1], p[2]);
    // The circle through the three points.
    let d = 2.0 * (a.0 * (m.1 - b.1) + m.0 * (b.1 - a.1) + b.0 * (a.1 - m.1));
    if d.abs() < 1e-12 {
        return vec![a, b];
    }
    let sq = |q: (f64, f64)| q.0 * q.0 + q.1 * q.1;
    let cx = (sq(a) * (m.1 - b.1) + sq(m) * (b.1 - a.1) + sq(b) * (a.1 - m.1)) / d;
    let cy = (sq(a) * (b.0 - m.0) + sq(m) * (a.0 - b.0) + sq(b) * (m.0 - a.0)) / d;
    let r = (a.0 - cx).hypot(a.1 - cy);
    let angle = |q: (f64, f64)| (q.1 - cy).atan2(q.0 - cx);
    let tau = std::f64::consts::TAU;
    let (t0, tm, t1) = (angle(a), angle(m), angle(b));
    // Sweep from a to b through m.
    let ccw = |from: f64, to: f64| (to - from).rem_euclid(tau);
    let sweep = if ccw(t0, tm) <= ccw(t0, t1) {
        ccw(t0, t1)
    } else {
        ccw(t0, t1) - tau
    };
    (0..=32)
        .map(|i| {
            let t = t0 + sweep * f64::from(i) / 32.0;
            (cx + r * t.cos(), cy + r * t.sin())
        })
        .collect()
}

/// The layer Revit's DWG export draws an element on, by its Revit category, with its default
/// (AIA) layer names: read from Revit's own exports of the public samples, where every sample
/// gives a category the same layer. Walls go by their Function: an interior wall (not
/// `IsExternal`) on `I-WALL`. An element with no category known here falls back to its IFC type.
#[cfg(feature = "dwg")]
fn layer_for(sets: &[&super::entities::PropertySet], ifc_type: &str) -> (&'static str, i16) {
    let fallback = layer_for_ifc_type(ifc_type);
    let Some(category) = super::element_table::category_in(sets) else {
        return fallback;
    };
    if category == "Walls" {
        let external = sets
            .iter()
            .flat_map(|s| s.properties.iter())
            .find(|p| p.name == "IsExternal")
            .and_then(|p| match p.value {
                super::entities::PropertyValue::Boolean(b) => Some(b),
                _ => None,
            });
        return if external == Some(false) {
            ("I-WALL", fallback.1)
        } else {
            ("A-WALL", fallback.1)
        };
    }
    CATEGORY_LAYERS
        .iter()
        .find(|(c, _)| *c == category)
        .map_or(fallback, |(_, l)| (l, fallback.1))
}

/// Revit category to the layer its DWG export uses (projection and cut agree for each).
#[cfg(feature = "dwg")]
const CATEGORY_LAYERS: &[(&str, &str)] = &[
    ("Air Terminals", "M-HVAC-CDFF"),
    ("Areas", "A-AREA"),
    ("Cable Tray Fittings", "E-CABL-TRAY"),
    ("Cable Trays", "E-CABL-TRAY"),
    ("Casework", "Q-CASE"),
    ("Ceilings", "A-CLNG"),
    ("Columns", "A-COLS"),
    ("Communication Devices", "E-COMM"),
    ("Conduit Fittings", "E-POWR-CNDT"),
    ("Conduits", "E-POWR-CNDT"),
    ("Curtain Panels", "A-GLAZ-CURT"),
    ("Curtain Systems", "A-GLAZ-CURT"),
    ("Curtain Wall Mullions", "A-GLAZ-CWMG"),
    ("Data Devices", "E-DATA"),
    ("Doors", "A-DOOR"),
    ("Duct Accessories", "M-HVAC-DUCT"),
    ("Duct Fittings", "M-HVAC-DUCT"),
    ("Ducts", "M-HVAC-DUCT"),
    ("Electrical Equipment", "E-ELEC-EQPM"),
    ("Electrical Fixtures", "E-ELEC-FIXT"),
    ("Entourage", "A-DETL-GENF"),
    ("Fascias", "A-ROOF-OTLN"),
    ("Fire Alarm Devices", "E-FIRE"),
    ("Flex Ducts", "M-HVAC-DUCT"),
    ("Flex Pipes", "P-PIPE"),
    ("Floors", "A-FLOR"),
    ("Food Service Equipment", "A-FDSQ"),
    ("Furniture", "I-FURN"),
    ("Furniture Systems", "I-FURN-PNLS"),
    ("Generic Models", "A-GENM"),
    ("Gutters", "A-ROOF-OTLN"),
    ("Handrails", "A-FLOR-HRAL"),
    ("Hardscape", "L-HRDS"),
    ("Lighting Devices", "E-LITE"),
    ("Lighting Fixtures", "E-LITE-EQPM"),
    ("Mass", "A-MASS"),
    ("Mechanical Equipment", "M-EQPM"),
    ("Medical Equipment", "A-MEDQ"),
    ("Nurse Call Devices", "E-NURS"),
    ("Parking", "C-PRKG"),
    ("Pipe Accessories", "P-PIPE"),
    ("Pipe Fittings", "P-PIPE"),
    ("Pipes", "P-PIPE"),
    ("Planting", "L-PLNT"),
    ("Plumbing Equipment", "P-EQPM"),
    ("Plumbing Fixtures", "P-SANR-FIXT"),
    ("Railings", "A-FLOR-HRAL"),
    ("Ramps", "A-FLOR-LEVL"),
    ("Roof Soffits", "A-ROOF-OTLN"),
    ("Roofs", "A-ROOF"),
    // Rooms as the export's room polylines.
    ("Rooms", "A-AREA-BNDY"),
    ("Security Devices", "E-SERT"),
    ("Signage", "A-SIGN"),
    ("Site", "L-SITE"),
    ("Slab Edges", "A-FLOR-OTLN"),
    ("Spaces", "M-AREA"),
    ("Specialty Equipment", "Q-SPCQ"),
    ("Sprinklers", "F-SPRN"),
    ("Stairs", "S-STRS"),
    ("Structural Beam Systems", "S-BEAM"),
    ("Structural Columns", "S-COLS"),
    ("Structural Connections", "S-FSTN"),
    ("Structural Foundations", "S-FNDN"),
    ("Structural Framing", "S-BEAM"),
    ("Structural Rebar", "S-RBAR"),
    ("Structural Trusses", "S-TRUS"),
    ("Supports", "A-FLOR-HRAL"),
    ("Telephone Devices", "E-COMM"),
    ("Top Rails", "A-FLOR-HRAL"),
    ("Topography", "C-TOPO"),
    ("Toposolid", "C-TOPO"),
    ("Vertical Circulation", "A-VRTC"),
    ("Wall Sweeps", "A-WALL"),
    ("Windows", "A-GLAZ"),
];

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
        "IFCSTAIR" | "IFCSTAIRFLIGHT" => ("S-STRS", 32),
        "IFCRAILING" => ("A-FLOR-HRAL", 32),
        "IFCFURNITURE" | "IFCFURNISHINGELEMENT" => ("I-FURN", 3),
        "IFCSPACE" => ("A-AREA-BNDY", 4),
        "IFCPLATE" => ("A-GLAZ-CURT", 5),
        _ => ("A-GENM", 8),
    }
}
