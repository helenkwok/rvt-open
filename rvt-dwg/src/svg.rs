//! SVG of a layout: model space, or a paper-space sheet with model space shown through its
//! viewports.
//!
//! Inserts and dimensions are expanded to what they draw (bounded by [`Limits`]). Every element
//! sits in a `<g class="layer" data-layer="…">` group and carries `data-handle`, so a viewer can
//! hide layers with CSS and map a click to the entity row. Geometry is written in drawing units
//! inside one Y-flipped group; strokes do not scale with zoom.
//!
//! Not drawn, and counted in [`Svg::not_drawn`]: 3D solids, meshes, regions, images, underlays,
//! tables, multileaders and other entities without plain 2D geometry in [`Geom`], hatch patterns
//! (a hatched area is tinted), linetype dashes, and text fonts (one sans-serif face).

use crate::{
    Affine, Colour, Drawing2d, Entity2d, Error, Geom, Limits, Space, bounds, polyline_segments,
};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write;

/// A rendered layout.
#[derive(Debug, Clone, PartialEq)]
pub struct Svg {
    pub svg: String,
    /// Layers that drew something, in drawing order.
    pub layers: Vec<String>,
    /// Entities not drawn, by type.
    pub not_drawn: BTreeMap<String, usize>,
    /// True when an expansion or size limit cut the drawing short.
    pub truncated: bool,
}

/// Render `layout` (a layout name; `None` for model space).
pub fn render(d: &Drawing2d, layout: Option<&str>, limits: &Limits) -> Result<Svg, Error> {
    let block = match layout {
        None => crate::MODEL_SPACE.to_string(),
        Some(name) => d
            .layouts
            .iter()
            .find(|l| l.name == name)
            .map(|l| l.block.clone())
            .ok_or_else(|| {
                Error(format!(
                    "no layout named {name:?}; the drawing has {}",
                    layout_names(d)
                ))
            })?,
    };
    let mut r = Renderer::new(d, limits);
    let paper = Space::of(&block) == Space::Paper;
    let main = r.block(&block);
    let viewports: Vec<&Entity2d> = if paper {
        d.entities
            .iter()
            .filter(|e| e.owner == block && matches!(e.geom, Geom::Viewport { id, view_height, .. } if id != 1 && view_height > 0.0))
            .collect()
    } else {
        Vec::new()
    };
    let model = if viewports.is_empty() {
        String::new()
    } else {
        r.block(crate::MODEL_SPACE)
    };

    // The view box: the layout's own geometry, and the viewports' frames on a sheet.
    let mut pts: Vec<[f64; 2]> = r.points.clone();
    for v in &viewports {
        pts.extend(v.geom.samples(f64::MAX));
    }
    if paper {
        if let Some(l) = d
            .layouts
            .iter()
            .find(|l| l.block == block)
            .and_then(|l| l.limits)
        {
            pts.extend([[l[0], l[1]], [l[2], l[3]]]);
        }
    }
    let b = bounds(pts.into_iter()).unwrap_or([0.0, 0.0, 100.0, 100.0]);
    let pad = ((b[2] - b[0]).max(b[3] - b[1]) * 0.02).max(1e-6);
    let (x0, y0, w, h) = (
        b[0] - pad,
        -(b[3] + pad),
        b[2] - b[0] + 2.0 * pad,
        b[3] - b[1] + 2.0 * pad,
    );

    let mut out = String::with_capacity(main.len() + model.len() + 1024);
    write!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{} {} {} {}\" data-layout=\"{}\" font-family=\"sans-serif\" \
         stroke-linecap=\"round\" stroke-linejoin=\"round\" fill=\"none\">",
        f(x0),
        f(y0),
        f(w),
        f(h),
        esc(layout.unwrap_or("Model"))
    )
    .ok();
    if !viewports.is_empty() {
        out.push_str("<defs><g id=\"model-space\">");
        out.push_str(&model);
        out.push_str("</g>");
        for (i, v) in viewports.iter().enumerate() {
            if let Geom::Viewport {
                center,
                width,
                height,
                ..
            } = v.geom
            {
                write!(
                    out,
                    "<clipPath id=\"vp{i}\"><rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/></clipPath>",
                    f(center[0] - width / 2.0),
                    f(center[1] - height / 2.0),
                    f(width),
                    f(height)
                )
                .ok();
            }
        }
        out.push_str("</defs>");
    }
    out.push_str("<g transform=\"scale(1 -1)\">");
    for (i, v) in viewports.iter().enumerate() {
        if let Geom::Viewport {
            center,
            height,
            view_center,
            view_height,
            ..
        } = v.geom
        {
            // ponytail: twisted and perspective views are drawn as plan views; add the twist angle if a sheet needs it.
            let k = height / view_height;
            write!(
                out,
                "<g clip-path=\"url(#vp{i})\" data-viewport=\"{}\"><use href=\"#model-space\" transform=\"matrix({} 0 0 {} {} {})\"/></g>",
                esc(&v.handle),
                f(k),
                f(k),
                f(center[0] - k * view_center[0]),
                f(center[1] - k * view_center[1])
            )
            .ok();
        }
    }
    out.push_str(&main);
    out.push_str("</g></svg>");
    if out.len() > limits.max_svg_bytes {
        return Err(Error(format!(
            "the SVG would be {} MB, over the {} MB limit",
            out.len() >> 20,
            limits.max_svg_bytes >> 20
        )));
    }
    let mut layers = r.layer_order;
    layers.dedup();
    Ok(Svg {
        svg: out,
        layers,
        not_drawn: r.not_drawn,
        truncated: r.truncated,
    })
}

fn layout_names(d: &Drawing2d) -> String {
    d.layouts
        .iter()
        .map(|l| format!("{:?}", l.name))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Number formatting: short, finite.
fn f(v: f64) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    let s = format!("{:.4}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

struct Inherit<'a> {
    layer: &'a str,
    colour: Colour,
}

struct Renderer<'a> {
    limits: &'a Limits,
    by_owner: HashMap<&'a str, Vec<&'a Entity2d>>,
    base: HashMap<&'a str, [f64; 3]>,
    layer: HashMap<&'a str, &'a crate::Layer2d>,
    budget: usize,
    points: Vec<[f64; 2]>,
    layer_order: Vec<String>,
    not_drawn: BTreeMap<String, usize>,
    truncated: bool,
}

impl<'a> Renderer<'a> {
    fn new(d: &'a Drawing2d, limits: &'a Limits) -> Self {
        let mut by_owner: HashMap<&str, Vec<&Entity2d>> = HashMap::new();
        for e in &d.entities {
            by_owner.entry(e.owner.as_str()).or_default().push(e);
        }
        Self {
            limits,
            by_owner,
            base: d.blocks.iter().map(|b| (b.name.as_str(), b.base)).collect(),
            layer: d.layers.iter().map(|l| (l.name.as_str(), l)).collect(),
            budget: limits.max_expanded,
            points: Vec::new(),
            layer_order: Vec::new(),
            not_drawn: BTreeMap::new(),
            truncated: false,
        }
    }

    /// The entities of `block`, grouped by layer in order of first use.
    fn block(&mut self, block: &str) -> String {
        let mut groups: Vec<(String, String)> = Vec::new();
        let root = Inherit {
            layer: "0",
            colour: Colour::aci(7),
        };
        let entities = self.by_owner.get(block).cloned().unwrap_or_default();
        for e in entities {
            self.entity(e, &Affine::IDENTITY, &root, 0, &mut groups);
        }
        let mut out = String::new();
        for (layer, body) in groups {
            write!(
                out,
                "<g class=\"layer\" data-layer=\"{}\">{body}</g>",
                esc(&layer)
            )
            .ok();
            self.layer_order.push(layer);
        }
        out
    }

    fn visible(&self, layer: &str) -> bool {
        self.layer.get(layer).is_none_or(|l| l.on && !l.frozen)
    }

    fn entity(
        &mut self,
        e: &Entity2d,
        t: &Affine,
        inherit: &Inherit,
        depth: usize,
        groups: &mut Vec<(String, String)>,
    ) {
        // Layer 0 and ByBlock inside a block take the insert's.
        let layer = if e.layer == "0" && depth > 0 {
            inherit.layer
        } else {
            e.layer.as_str()
        };
        if !self.visible(layer) {
            return;
        }
        let colour = if e.colour.is_by_block() {
            inherit.colour
        } else if e.colour.is_by_layer() {
            self.layer
                .get(layer)
                .map(|l| l.colour)
                .filter(|c| c.to_rgb().is_some())
                .unwrap_or(Colour::aci(7))
        } else {
            e.colour
        };
        match &e.geom {
            Geom::Insert {
                block,
                at,
                scale,
                rotation,
                ..
            } => {
                let base = self.base.get(block.as_str()).copied().unwrap_or([0.0; 3]);
                self.expand(
                    block,
                    &t.then(&Affine::insert(base, *at, *scale, *rotation)),
                    layer,
                    colour,
                    depth,
                    groups,
                );
                return;
            }
            Geom::Dimension { block, .. } => {
                self.expand(block, t, layer, colour, depth, groups);
                return;
            }
            Geom::Viewport { .. } => return,
            Geom::Other { .. } => {
                *self.not_drawn.entry(e.kind.clone()).or_default() += 1;
                return;
            }
            _ => {}
        }
        if self.budget == 0 {
            self.truncated = true;
            return;
        }
        self.budget -= 1;
        let g = if *t == Affine::IDENTITY {
            e.geom.clone()
        } else {
            e.geom.transformed(t)
        };
        let stroke = rgb(colour);
        let mut el = String::new();
        let handle = esc(&e.handle);
        match &g {
            Geom::Line { a, b } => {
                write!(el, "<line data-handle=\"{handle}\" x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{stroke}\"/>", f(a[0]), f(a[1]), f(b[0]), f(b[1])).ok();
            }
            Geom::Circle { c, r } => {
                write!(el, "<circle data-handle=\"{handle}\" cx=\"{}\" cy=\"{}\" r=\"{}\" stroke=\"{stroke}\"/>", f(c[0]), f(c[1]), f(*r)).ok();
            }
            Geom::Point { p } => {
                write!(el, "<circle data-handle=\"{handle}\" cx=\"{}\" cy=\"{}\" r=\"0\" stroke=\"{stroke}\" stroke-width=\"3\"/>", f(p[0]), f(p[1])).ok();
            }
            Geom::Arc { .. } | Geom::Ellipse { .. } | Geom::Polyline { .. } | Geom::Path { .. } => {
                write!(
                    el,
                    "<path data-handle=\"{handle}\" d=\"{}\" stroke=\"{stroke}\"/>",
                    path_d(&g)
                )
                .ok();
            }
            Geom::Hatch { loops, solid, .. } => {
                let d: String = loops
                    .iter()
                    .map(|l| {
                        path_d(&Geom::Polyline {
                            pts: l.clone(),
                            closed: true,
                            z: 0.0,
                        })
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                let opacity = if *solid { "1" } else { "0.25" };
                write!(el, "<path data-handle=\"{handle}\" d=\"{d}\" fill=\"{stroke}\" fill-opacity=\"{opacity}\" fill-rule=\"evenodd\" stroke=\"none\"/>").ok();
            }
            Geom::Text {
                at,
                height,
                rotation,
                value,
            } => {
                let lines: Vec<&str> = value.lines().collect();
                write!(
                    el,
                    "<text data-handle=\"{handle}\" transform=\"translate({} {}) rotate({}) scale(1 -1)\" font-size=\"{}\" fill=\"{stroke}\" stroke=\"none\">",
                    f(at[0]),
                    f(at[1]),
                    f(rotation.to_degrees()),
                    f(height.max(1e-6) * 1.4)
                )
                .ok();
                for (i, line) in lines.iter().enumerate() {
                    if i == 0 {
                        el.push_str(&esc(line));
                    } else {
                        write!(el, "<tspan x=\"0\" dy=\"1.25em\">{}</tspan>", esc(line)).ok();
                    }
                }
                el.push_str("</text>");
            }
            _ => return,
        }
        let mut samples = g.samples(f64::MAX);
        if let Geom::Text {
            at, height, value, ..
        } = &g
        {
            // Text reaches roughly its height times its length to the right.
            samples.push([
                at[0] + height * 0.6 * value.chars().count() as f64,
                at[1] + height,
            ]);
        }
        self.points.extend(samples);
        match groups.iter_mut().find(|(l, _)| l == layer) {
            Some((_, body)) => body.push_str(&el),
            None => groups.push((layer.to_string(), el)),
        }
    }

    fn expand(
        &mut self,
        block: &str,
        t: &Affine,
        layer: &str,
        colour: Colour,
        depth: usize,
        groups: &mut Vec<(String, String)>,
    ) {
        if depth >= self.limits.max_insert_depth {
            self.truncated = true;
            return;
        }
        let entities = self.by_owner.get(block).cloned().unwrap_or_default();
        let layer = layer.to_string();
        let inherit = Inherit {
            layer: &layer,
            colour,
        };
        for child in entities {
            self.entity(child, t, &inherit, depth + 1, groups);
        }
    }
}

fn rgb(c: Colour) -> String {
    match c.to_rgb() {
        // Index 7 is white on a dark screen and black on paper: drawn black here.
        Some([255, 255, 255]) | None => "#000000".into(),
        Some([r, g, b]) => format!("#{r:02X}{g:02X}{b:02X}"),
    }
}

/// SVG path data for a curve, in drawing coordinates (counter-clockwise is the positive sweep).
fn path_d(g: &Geom) -> String {
    let mut d = String::new();
    let arc_to = |d: &mut String, r: f64, sweep: f64, end: [f64; 2]| {
        write!(
            d,
            " A{} {} 0 {} {} {} {}",
            f(r),
            f(r),
            u8::from(sweep.abs() > std::f64::consts::PI),
            u8::from(sweep > 0.0),
            f(end[0]),
            f(end[1])
        )
        .ok();
    };
    match g {
        Geom::Arc { c, r, start, end } => {
            let sweep = crate::ccw_sweep(*start, *end);
            let p = |a: f64| [c[0] + r * a.cos(), c[1] + r * a.sin()];
            let (p0, p1) = (p(*start), p(*end));
            write!(d, "M{} {}", f(p0[0]), f(p0[1])).ok();
            if sweep >= std::f64::consts::TAU - 1e-9 {
                let mid = p(start + std::f64::consts::PI);
                arc_to(&mut d, *r, std::f64::consts::PI, mid);
                arc_to(&mut d, *r, std::f64::consts::PI, p0);
            } else {
                arc_to(&mut d, *r, sweep, p1);
            }
        }
        Geom::Ellipse { .. } => {
            let pts = g.samples(
                g.bbox()
                    .map_or(1.0, |b| ((b[2] - b[0]).max(b[3] - b[1]) / 96.0).max(1e-9)),
            );
            for (i, p) in pts.iter().enumerate() {
                write!(
                    d,
                    "{}{} {}",
                    if i == 0 { "M" } else { " L" },
                    f(p[0]),
                    f(p[1])
                )
                .ok();
            }
        }
        Geom::Polyline { pts, closed, .. } => {
            if let Some(p0) = pts.first() {
                write!(d, "M{} {}", f(p0[0]), f(p0[1])).ok();
            }
            for (a, b) in polyline_segments(pts, *closed) {
                if a[2].abs() < 1e-12 {
                    write!(d, " L{} {}", f(b[0]), f(b[1])).ok();
                } else {
                    let (_, r, _, sweep) = crate::bulge_arc(a, b);
                    arc_to(&mut d, r, sweep, [b[0], b[1]]);
                }
            }
            if *closed {
                d.push_str(" Z");
            }
        }
        Geom::Path { pts, closed } => {
            for (i, p) in pts.iter().enumerate() {
                write!(
                    d,
                    "{}{} {}",
                    if i == 0 { "M" } else { " L" },
                    f(p[0]),
                    f(p[1])
                )
                .ok();
            }
            if *closed {
                d.push_str(" Z");
            }
        }
        _ => {}
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Block2d, Layer2d, Layout2d};

    #[test]
    fn sheet_shows_model_space_through_its_viewport_and_groups_layers() {
        let mut vp = Entity2d::new(
            "VIEWPORT",
            "0",
            Geom::Viewport {
                center: [100.0, 100.0, 0.0],
                width: 50.0,
                height: 50.0,
                view_center: [5.0, 5.0],
                view_height: 10.0,
                id: 2,
            },
        );
        vp.owner = "*Paper_Space".into();
        vp.handle = "40".into();
        let mut wall = Entity2d::new(
            "LINE",
            "A-WALL",
            Geom::Line {
                a: [0.0, 0.0, 0.0],
                b: [10.0, 0.0, 0.0],
            },
        );
        wall.handle = "41".into();
        let mut frozen = Entity2d::new(
            "LINE",
            "OFF",
            Geom::Line {
                a: [0.0, 0.0, 0.0],
                b: [1.0, 1.0, 0.0],
            },
        );
        frozen.handle = "42".into();
        let mut off = Layer2d::new("OFF", 1);
        off.on = false;
        let d = Drawing2d {
            version: "AC1032".into(),
            layers: vec![Layer2d::new("0", 7), Layer2d::new("A-WALL", 1), off],
            blocks: vec![Block2d {
                name: "*Paper_Space".into(),
                base: [0.0; 3],
                xref_path: None,
                layout: Some("A101".into()),
                attribute_tags: vec![],
            }],
            layouts: vec![
                Layout2d {
                    name: "Model".into(),
                    block: crate::MODEL_SPACE.into(),
                    tab_order: 0,
                    is_model: true,
                    limits: None,
                },
                Layout2d {
                    name: "A101".into(),
                    block: "*Paper_Space".into(),
                    tab_order: 1,
                    is_model: false,
                    limits: Some([0.0, 0.0, 420.0, 297.0]),
                },
            ],
            entities: vec![vp, wall, frozen],
            ..Default::default()
        };
        let s = render(&d, Some("A101"), &Limits::DEFAULT).unwrap();
        assert!(
            s.svg
                .contains("<g id=\"model-space\"><g class=\"layer\" data-layer=\"A-WALL\">"),
            "{}",
            s.svg
        );
        assert!(
            s.svg.contains("matrix(5 0 0 5 75 75)"),
            "scale 50/10, model (5,5) at (100,100): {}",
            s.svg
        );
        assert!(s.svg.contains("stroke=\"#FF0000\""));
        assert!(
            !s.svg.contains("data-handle=\"42\""),
            "a layer that is off is not drawn"
        );
        assert!(
            s.svg.starts_with("<svg") && s.svg.contains("viewBox=\"-8.4 -305.4 436.8 313.8\""),
            "{}",
            &s.svg[..200]
        );
        assert!(
            render(&d, Some("nope"), &Limits::DEFAULT)
                .unwrap_err()
                .0
                .contains("\"A101\"")
        );
    }

    #[test]
    fn self_insert_stops_at_the_depth_limit() {
        let mut ins = Entity2d::new(
            "INSERT",
            "0",
            Geom::Insert {
                block: "LOOP".into(),
                at: [1.0, 0.0, 0.0],
                scale: [1.0; 3],
                rotation: 0.0,
                attributes: vec![],
            },
        );
        let mut inner = ins.clone();
        inner.owner = "LOOP".into();
        let mut line = Entity2d::new(
            "LINE",
            "0",
            Geom::Line {
                a: [0.0; 3],
                b: [1.0, 0.0, 0.0],
            },
        );
        line.owner = "LOOP".into();
        ins.handle = "1".into();
        let d = Drawing2d {
            blocks: vec![Block2d {
                name: "LOOP".into(),
                base: [0.0; 3],
                xref_path: None,
                layout: None,
                attribute_tags: vec![],
            }],
            entities: vec![ins, inner, line],
            ..Default::default()
        };
        let s = render(&d, None, &Limits::DEFAULT).unwrap();
        assert!(s.truncated);
        assert_eq!(
            s.svg.matches("<line").count(),
            Limits::DEFAULT.max_insert_depth
        );
    }
}
