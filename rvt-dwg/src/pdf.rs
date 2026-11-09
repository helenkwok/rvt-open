//! Vector PDF of a layout: model space at a scale, or a paper-space sheet at full size with model
//! space shown through its viewports (clipped to each viewport's frame).
//!
//! One page (ISO 32000-1), the drawing as stroked paths and text in a PDF standard font (sans serif),
//! colours from the entity or its layer (index 7 drawn black). Inserts and dimensions are expanded
//! (bounded by [`Limits`]); curves are written as short straight segments; hatches are filled
//! (solid) or tinted (patterned). Not written: linetype dashes, lineweights (one hairline width),
//! other fonts, images and the entities the SVG does not draw either.

use crate::compare::flatten_coloured;
use crate::{Drawing2d, Error, Geom, Limits, Space, bounds};
use std::fmt::Write;

/// Points per millimetre.
const PT_PER_MM: f64 = 72.0 / 25.4;

/// A rendered page.
#[derive(Debug, Clone, PartialEq)]
pub struct Pdf {
    pub bytes: Vec<u8>,
    /// Page width and height, points.
    pub page: [f64; 2],
    /// Paths and texts written.
    pub paths: usize,
    pub texts: usize,
    /// The drawing point at the page's bottom-left corner, and page points per drawing unit.
    pub origin: [f64; 2],
    pub points_per_unit: f64,
}

/// Millimetres per drawing unit (`$INSUNITS`); unitless drawings are taken as millimetres.
fn mm_per_unit(units: i16) -> f64 {
    match units {
        1 => 25.4,
        2 => 304.8,
        5 => 10.0,
        6 => 1000.0,
        14 => 100.0,
        _ => 1.0,
    }
}

fn num(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

/// A PDF string literal in WinAnsi (characters outside it become `?`).
fn pdf_string(s: &str) -> String {
    let mut out = String::from("(");
    for c in s.chars() {
        match c {
            '(' | ')' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            ' '..='~' => out.push(c),
            '\u{a0}'..='\u{ff}' => {
                write!(out, "\\{:03o}", c as u32).ok();
            }
            _ => out.push('?'),
        }
    }
    out.push(')');
    out
}

/// Render `layout` (`None` for model space). Model space is drawn at 1:`scale` (drawing
/// millimetres to paper millimetres); a sheet is drawn at full size and `scale` is ignored.
pub fn render(
    d: &Drawing2d,
    layout: Option<&str>,
    scale: f64,
    limits: &Limits,
) -> Result<Pdf, Error> {
    let block = match layout {
        None => crate::MODEL_SPACE.to_string(),
        Some(name) => d
            .layouts
            .iter()
            .find(|l| l.name == name)
            .map(|l| l.block.clone())
            .ok_or_else(|| Error(format!("no layout named {name:?}")))?,
    };
    let unit = mm_per_unit(d.units);
    let paper = Space::of(&block) == Space::Paper;
    // Drawing units to page points.
    let k = if paper {
        unit * PT_PER_MM
    } else {
        unit * PT_PER_MM / scale.max(1e-9)
    };
    let own = flatten_coloured(d, &block, limits);
    // Each viewport: its frame (min x, min y, max x, max y), scale, centre and view centre.
    type Frame = ([f64; 4], f64, [f64; 2], [f64; 2]);
    let viewports: Vec<Frame> = if paper {
        d.entities
            .iter()
            .filter(|e| e.owner == block)
            .filter_map(|e| match e.geom {
                Geom::Viewport {
                    center,
                    width,
                    height,
                    view_center,
                    view_height,
                    id,
                } if id != 1 && view_height > 0.0 => Some((
                    [
                        center[0] - width / 2.0,
                        center[1] - height / 2.0,
                        center[0] + width / 2.0,
                        center[1] + height / 2.0,
                    ],
                    height / view_height,
                    [center[0], center[1]],
                    view_center,
                )),
                _ => None,
            })
            .collect()
    } else {
        Vec::new()
    };
    let model = if viewports.is_empty() {
        Vec::new()
    } else {
        flatten_coloured(d, crate::MODEL_SPACE, limits)
    };

    // The page: the layout's geometry, the viewports' frames and the paper limits.
    let mut pts: Vec<[f64; 2]> = own.iter().flat_map(|p| p.geom.samples(f64::MAX)).collect();
    for (r, ..) in &viewports {
        pts.extend([[r[0], r[1]], [r[2], r[3]]]);
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
    let margin = if paper { 0.0 } else { 10.0 / k * PT_PER_MM };
    let (x0, y0) = (b[0] - margin, b[1] - margin);
    let page = [
        ((b[2] - b[0] + 2.0 * margin) * k).max(1.0),
        ((b[3] - b[1] + 2.0 * margin) * k).max(1.0),
    ];

    let mut content = String::from("0.25 w 1 J 1 j\n");
    let (mut paths, mut texts) = (0, 0);
    // Draw `items`, each point mapped by `map` (drawing units to page points).
    let mut draw = |content: &mut String,
                    items: &[crate::compare::Coloured],
                    map: &dyn Fn([f64; 2]) -> [f64; 2],
                    size: f64| {
        for p in items {
            let [r, g, bl] = match p.rgb {
                [255, 255, 255] => [0, 0, 0],
                c => c,
            };
            let rgb = format!(
                "{} {} {}",
                num(r as f64 / 255.0),
                num(g as f64 / 255.0),
                num(bl as f64 / 255.0)
            );
            match &p.geom {
                Geom::Text {
                    at,
                    height,
                    rotation,
                    value,
                } => {
                    let at = map([at[0], at[1]]);
                    let h = (height * size / 0.72).max(0.1);
                    let (s, c) = rotation.sin_cos();
                    for (i, line) in value.lines().enumerate() {
                        let dy = -(i as f64) * h * 1.2;
                        writeln!(
                            content,
                            "BT {rgb} rg /F1 {} Tf {} {} {} {} {} {} Tm {} Tj ET",
                            num(h),
                            num(c),
                            num(s),
                            num(-s),
                            num(c),
                            num(at[0] - s * dy),
                            num(at[1] + c * dy),
                            pdf_string(line)
                        )
                        .ok();
                    }
                    texts += 1;
                }
                Geom::Hatch { loops, solid, .. } => {
                    for l in loops {
                        let ring = Geom::Polyline {
                            pts: l.clone(),
                            closed: true,
                            z: 0.0,
                        };
                        path(content, &curve_points(&ring, size), map, true);
                    }
                    let tint = if *solid { "" } else { "0.75 0.75 0.75 rg " };
                    writeln!(content, "{rgb} rg {tint}f*").ok();
                    paths += 1;
                }
                Geom::Line { a, b } => {
                    path(content, &[[a[0], a[1]], [b[0], b[1]]], map, false);
                    writeln!(content, "{rgb} RG S").ok();
                    paths += 1;
                }
                Geom::Circle { .. }
                | Geom::Arc { .. }
                | Geom::Ellipse { .. }
                | Geom::Polyline { .. }
                | Geom::Path { .. } => {
                    let closed = matches!(
                        p.geom,
                        Geom::Circle { .. }
                            | Geom::Polyline { closed: true, .. }
                            | Geom::Path { closed: true, .. }
                    );
                    let s = curve_points(&p.geom, size);
                    path(content, &s, map, closed);
                    writeln!(content, "{rgb} RG S").ok();
                    paths += 1;
                }
                _ => {}
            }
        }
    };
    for (r, vk, centre, view_centre) in &viewports {
        let (a, bb) = (
            [(r[0] - x0) * k, (r[1] - y0) * k],
            [(r[2] - x0) * k, (r[3] - y0) * k],
        );
        writeln!(
            content,
            "q {} {} {} {} re W n",
            num(a[0]),
            num(a[1]),
            num(bb[0] - a[0]),
            num(bb[1] - a[1])
        )
        .ok();
        let (vk, centre, view_centre) = (*vk, *centre, *view_centre);
        let map = move |q: [f64; 2]| {
            [
                (centre[0] + (q[0] - view_centre[0]) * vk - x0) * k,
                (centre[1] + (q[1] - view_centre[1]) * vk - y0) * k,
            ]
        };
        draw(&mut content, &model, &map, vk * k);
        content.push_str("Q\n");
    }
    let map = |q: [f64; 2]| [(q[0] - x0) * k, (q[1] - y0) * k];
    draw(&mut content, &own, &map, k);

    let mut out: Vec<u8> = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets = Vec::new();
    let mut obj = |out: &mut Vec<u8>, body: &[u8]| {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", offsets.len()).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };
    obj(&mut out, b"<< /Type /Catalog /Pages 2 0 R >>");
    obj(&mut out, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    obj(
        &mut out,
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>",
            num(page[0]),
            num(page[1])
        )
        .as_bytes(),
    );
    let mut stream = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
    stream.extend_from_slice(content.as_bytes());
    stream.extend_from_slice(b"\nendstream");
    obj(&mut out, &stream);
    obj(
        &mut out,
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
    );
    let xref = out.len();
    let mut tail = format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1);
    for o in &offsets {
        writeln!(tail, "{o:010} 00000 n ").ok();
    }
    writeln!(
        tail,
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF",
        offsets.len() + 1
    )
    .ok();
    out.extend_from_slice(tail.as_bytes());
    Ok(Pdf {
        bytes: out,
        page,
        paths,
        texts,
        origin: [x0, y0],
        points_per_unit: k,
    })
}

/// The page width of a PDF this module wrote, points.
fn media_width(pdf: &[u8]) -> Option<f64> {
    let text = String::from_utf8_lossy(pdf);
    let at = text.find("/MediaBox [")? + "/MediaBox [".len();
    let nums: Vec<f64> = text[at..]
        .split(']')
        .next()?
        .split_ascii_whitespace()
        .filter_map(|v| v.parse().ok())
        .collect();
    (nums.len() == 4).then(|| nums[2] - nums[0])
}

/// The points of a curve, straight segments as their ends and arcs as chords that stay within
/// 0.02 mm of the arc on the page (`size`: page points per drawing unit).
fn curve_points(g: &Geom, size: f64) -> Vec<[f64; 2]> {
    let e = 0.02 * PT_PER_MM / size.max(1e-12);
    let arc = |out: &mut Vec<[f64; 2]>, c: [f64; 2], r: f64, start: f64, sweep: f64| {
        // The largest angle whose chord stays within `e` of the arc.
        let step = if r > e {
            2.0 * (1.0 - e / r).acos()
        } else {
            std::f64::consts::FRAC_PI_2
        };
        let n = (sweep.abs() / step.max(1e-6)).ceil().clamp(1.0, 100_000.0) as usize;
        for i in 0..=n {
            let a = start + sweep * i as f64 / n as f64;
            out.push([c[0] + r * a.cos(), c[1] + r * a.sin()]);
        }
    };
    let mut out = Vec::new();
    match g {
        Geom::Line { a, b } => out.extend([[a[0], a[1]], [b[0], b[1]]]),
        Geom::Circle { c, r } => arc(&mut out, [c[0], c[1]], *r, 0.0, std::f64::consts::TAU),
        Geom::Arc { c, r, start, end } => arc(
            &mut out,
            [c[0], c[1]],
            *r,
            *start,
            crate::ccw_sweep(*start, *end),
        ),
        Geom::Ellipse { major, .. } => {
            // Sampled as finely as a circle of its major radius.
            let r = major[0].hypot(major[1]);
            let step = if r > e { (8.0 * r * e).sqrt() } else { r };
            out = g.samples(step.max(1e-9));
        }
        Geom::Polyline { pts, closed, .. } => {
            for (a, b) in crate::polyline_segments(pts, *closed) {
                if a[2].abs() < 1e-12 {
                    if out.is_empty() {
                        out.push([a[0], a[1]]);
                    }
                    out.push([b[0], b[1]]);
                } else {
                    let (c, r, s0, sweep) = crate::bulge_arc(a, b);
                    arc(&mut out, c, r, s0, sweep);
                }
            }
        }
        Geom::Path { pts, closed } => {
            out.extend(pts.iter().map(|p| [p[0], p[1]]));
            if *closed {
                if let Some(f) = pts.first() {
                    out.push([f[0], f[1]]);
                }
            }
        }
        _ => {}
    }
    out
}

fn path(content: &mut String, pts: &[[f64; 2]], map: &dyn Fn([f64; 2]) -> [f64; 2], close: bool) {
    let mut last: Option<[f64; 2]> = None;
    for (i, p) in pts.iter().enumerate() {
        let q = map(*p);
        if last.is_some_and(|l| (l[0] - q[0]).abs() < 1e-4 && (l[1] - q[1]).abs() < 1e-4) {
            continue;
        }
        let op = if i == 0 { "m" } else { "l" };
        write!(content, "{} {} {op} ", num(q[0]), num(q[1])).ok();
        last = Some(q);
    }
    if close {
        content.push_str("h ");
    }
}

/// A segment of a page, in page points, with the clip rectangle it is drawn in (min x, min y,
/// max x, max y), if any.
pub type Segment = ([[f64; 2]; 2], Option<[f64; 4]>);

/// The straight segments of a page this module wrote: every `m`/`l`/`h` run of its content
/// stream, with the `re W n` clip in force. For checking a PDF against the drawing it came from.
pub fn segments(pdf: &[u8]) -> Vec<Segment> {
    let text = String::from_utf8_lossy(pdf);
    let Some(start) = text.find("stream\n") else {
        return Vec::new();
    };
    let body = &text[start + 7..text.rfind("endstream").unwrap_or(text.len())];
    let mut out = Vec::new();
    let (mut stack, mut cur, mut first): (Vec<f64>, Option<[f64; 2]>, Option<[f64; 2]>) =
        (Vec::new(), None, None);
    let (mut clip, mut rect): (Option<[f64; 4]>, Option<[f64; 4]>) = (None, None);
    let mut in_text = false;
    for tok in body.split_ascii_whitespace() {
        let last2 = || [stack[stack.len() - 2], stack[stack.len() - 1]];
        match tok {
            "BT" => in_text = true,
            "ET" => in_text = false,
            "re" if stack.len() >= 4 => {
                let n = stack.len();
                let (x, y, w, h) = (stack[n - 4], stack[n - 3], stack[n - 2], stack[n - 1]);
                rect = Some([x.min(x + w), y.min(y + h), x.max(x + w), y.max(y + h)]);
            }
            "W" => clip = rect,
            "Q" => clip = None,
            "m" if !in_text && stack.len() >= 2 => {
                let p = last2();
                cur = Some(p);
                first = Some(p);
            }
            "l" if !in_text && stack.len() >= 2 => {
                let p = last2();
                if let Some(c) = cur {
                    out.push(([c, p], clip));
                }
                cur = Some(p);
            }
            "h" => {
                if let (Some(c), Some(f)) = (cur, first) {
                    out.push(([c, f], clip));
                }
            }
            _ => {}
        }
        match tok.parse::<f64>() {
            Ok(v) => stack.push(v),
            Err(_) => stack.clear(),
        }
    }
    out
}

/// How well a PDF's linework matches the drawing it was written from: the PDF's segments,
/// mapped back to drawing units through the page transform of `render(d, layout, scale)`,
/// sampled every half `tolerance` (inside their clip) and matched against the drawing's own
/// linework within `tolerance` drawing units (precision: PDF on the drawing; recall: the drawing
/// covered by the PDF). On a sheet, model space counts as its viewports show it.
pub fn check(
    pdf: &[u8],
    d: &Drawing2d,
    layout: Option<&str>,
    scale: Option<f64>,
    tolerance: f64,
    limits: &Limits,
) -> Result<PdfCheck, Error> {
    // Model space drawn at an unknown scale: the scale that gives the PDF's page width (the
    // drawing's width at the scale plus the two margins).
    let scale = match (scale, layout) {
        (Some(s), _) => s,
        (None, Some(_)) => 1.0,
        (None, None) => {
            let at_one = render(d, None, 1.0, limits)?;
            let width = media_width(pdf).ok_or_else(|| Error("no /MediaBox in the PDF".into()))?;
            let margins = 20.0 * PT_PER_MM;
            let drawn = at_one.page[0] - margins;
            if width - margins <= 0.0 || drawn <= 0.0 {
                return Err(Error("the page is narrower than its margins".into()));
            }
            drawn / (width - margins)
        }
    };
    let page = render(d, layout, scale, limits)?;
    let (k, o) = (page.points_per_unit, page.origin);
    let step = tolerance / 2.0;
    let mut got = Vec::new();
    for ([a, b], clip) in segments(pdf) {
        let n = (((b[0] - a[0]).hypot(b[1] - a[1]) / k) / step)
            .ceil()
            .clamp(1.0, 100_000.0) as usize;
        for i in 0..=n {
            let t = i as f64 / n as f64;
            let q = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
            if clip.is_some_and(|c| {
                q[0] < c[0] - 1e-3 || q[0] > c[2] + 1e-3 || q[1] < c[1] - 1e-3 || q[1] > c[3] + 1e-3
            }) {
                continue;
            }
            got.push([q[0] / k + o[0], q[1] / k + o[1]]);
        }
    }
    let want = crate::compare::linework_samples(d, layout, step, limits);
    Ok(PdfCheck {
        matched: crate::compare::point_match(&want, &got, tolerance),
        misses: crate::compare::unmatched(&want, &got, tolerance, 5),
    })
}

/// What [`check`] found: the match, and up to five PDF points (drawing units) that lie on none
/// of the drawing's linework.
#[derive(Debug, Clone)]
pub struct PdfCheck {
    pub matched: crate::compare::Match,
    pub misses: Vec<[f64; 2]>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Entity2d, Layer2d, Layout2d};

    #[test]
    fn a_square_and_a_text_become_a_page_at_scale_and_read_back() {
        let line = |a: [f64; 2], b: [f64; 2]| {
            Entity2d::new(
                "LINE",
                "A-WALL",
                Geom::Line {
                    a: [a[0], a[1], 0.0],
                    b: [b[0], b[1], 0.0],
                },
            )
        };
        let d = Drawing2d {
            units: 4,
            layers: vec![Layer2d::new("A-WALL", 1)],
            entities: vec![
                line([0.0, 0.0], [10_000.0, 0.0]),
                line([10_000.0, 0.0], [10_000.0, 10_000.0]),
                Entity2d::new(
                    "TEXT",
                    "A-WALL",
                    Geom::Text {
                        at: [100.0, 100.0, 0.0],
                        height: 250.0,
                        rotation: 0.0,
                        value: "Room (1)".into(),
                    },
                ),
            ],
            ..Default::default()
        };
        let p = render(&d, None, 100.0, &Limits::DEFAULT).unwrap();
        assert!(p.bytes.starts_with(b"%PDF-1.4"));
        assert!(String::from_utf8_lossy(&p.bytes).contains("(Room \\(1\\)) Tj"));
        // 10 m at 1:100 is 100 mm on paper, plus 10 mm margins: 120 mm.
        let mm = |pt: f64| pt / PT_PER_MM;
        assert!((mm(p.page[0]) - 120.0).abs() < 1e-6, "{:?}", p.page);
        let s = segments(&p.bytes);
        assert_eq!(s.len(), 2);
        assert!(
            (mm(s[0].0[1][0] - s[0].0[0][0]) - 100.0).abs() < 1e-2,
            "{s:?}"
        );
        assert_eq!((p.paths, p.texts), (2, 1));
        let m = check(&p.bytes, &d, None, None, 5.0, &Limits::DEFAULT)
            .unwrap()
            .matched;
        assert!(m.precision > 0.999 && m.recall > 0.999, "{m:?}");
    }

    #[test]
    fn a_sheet_draws_model_space_through_its_viewport_at_the_viewport_scale() {
        let mut d = Drawing2d {
            units: 4,
            ..Default::default()
        };
        d.layouts.push(Layout2d {
            name: "A101".into(),
            block: "*Paper_Space".into(),
            tab_order: 1,
            is_model: false,
            limits: Some([0.0, 0.0, 841.0, 594.0]),
        });
        d.entities.push(Entity2d::new(
            "LINE",
            "0",
            Geom::Line {
                a: [0.0, 0.0, 0.0],
                b: [5000.0, 0.0, 0.0],
            },
        ));
        let mut vp = Entity2d::new(
            "VIEWPORT",
            "0",
            Geom::Viewport {
                center: [400.0, 300.0, 0.0],
                width: 200.0,
                height: 100.0,
                view_center: [2500.0, 0.0],
                view_height: 10_000.0,
                id: 2,
            },
        );
        vp.owner = "*Paper_Space".into();
        d.entities.push(vp);
        let p = render(&d, Some("A101"), 1.0, &Limits::DEFAULT).unwrap();
        assert!((p.page[0] / PT_PER_MM - 841.0).abs() < 1e-6);
        let s = segments(&p.bytes);
        // The 5 m line at 1:100 is 50 mm long, centred on the viewport, inside its clip.
        assert_eq!(s.len(), 1);
        let x: Vec<f64> = s[0].0.iter().map(|q| q[0] / PT_PER_MM).collect();
        assert!(
            (x[0] - 375.0).abs() < 1e-2 && (x[1] - 425.0).abs() < 1e-2,
            "{x:?}"
        );
        assert!(s[0].1.is_some());
        let m = check(&p.bytes, &d, Some("A101"), None, 0.1, &Limits::DEFAULT)
            .unwrap()
            .matched;
        assert!(m.precision > 0.999 && m.recall > 0.999, "{m:?}");
    }
}
