//! DWG and DXF for rvt-open.
//!
//! Reads a drawing into [`Drawing2d`], a plain model of layers, blocks, layouts and entities that
//! every output works from: the entity, summary, layer and block tables ([`tables`]), SVG
//! ([`svg`]), `report.json` ([`report`]), the comparison scorer ([`compare`]) and the DXF/DWG
//! writer ([`mod@write`]). The same model is what rvt-open's sheet and view export builds, so a
//! drawing read from a file and one made from a Revit model go through one path.
//!
//! The DWG/DXF engine is the `acadrust` crate (MPL-2.0), used unmodified. Nothing outside this
//! crate sees its types.
//!
//! # Damaged and hostile files
//!
//! Every read and write is guarded:
//! - the input size is checked against [`Limits::max_input_bytes`] (DWG) or
//!   [`Limits::max_dxf_input_bytes`] (DXF) before anything is decoded, and the engine's
//!   decompression is capped by [`Limits::max_decompressed_bytes`];
//! - the engine runs inside `catch_unwind`, so a panic is an error naming the file, not a crash;
//! - a read that skipped records, recovered from errors or came back empty says so in
//!   [`Read::notes`], so a partial read never passes for a complete one;
//! - insert expansion is bounded by depth and by a total count ([`Limits`]), so a block that
//!   inserts itself, or a fan-out of nested blocks, stops at the limit;
//! - an allocation failure or stack overflow ends the process and `catch_unwind` cannot stop
//!   that, so the `rvt-dwg` command runs each file in a child copy of itself (`--worker`).

#![forbid(unsafe_code)]

pub mod compare;
mod read;
pub mod report;
pub mod svg;
pub mod tables;
pub mod write;

use serde::{Deserialize, Serialize};

pub use read::{read_bytes, read_file};

/// The engine that reads and writes DWG and DXF, as it appears in `report.json`.
pub const ENGINE: &str = "acadrust 0.6.8";

/// Size and work limits for one drawing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Largest DWG input read.
    pub max_input_bytes: usize,
    /// Largest DXF input read. DXF is text and holds the same drawing in several times the bytes of
    /// DWG (a 92 MB DWG of 180,000 entities re-saves as a 750 MB DXF), so it has its own limit. A
    /// DXF file is read from disk as a stream, not loaded first.
    pub max_dxf_input_bytes: usize,
    /// Largest total of decompressed section data the engine may allocate for one read.
    pub max_decompressed_bytes: usize,
    /// Deepest chain of block inserts expanded (SVG, exploded tables).
    pub max_insert_depth: usize,
    /// Most entities produced by expanding inserts, over the whole drawing.
    pub max_expanded: usize,
    /// Largest SVG document written.
    pub max_svg_bytes: usize,
}

impl Limits {
    /// 512 MiB of DWG, 3 GiB of DXF, 2 GiB decompressed, inserts 16 deep, 4 million expanded
    /// entities, 1 GiB of SVG. Generous for real drawings; they exist for damaged ones.
    pub const DEFAULT: Limits = Limits {
        max_input_bytes: 512 << 20,
        max_dxf_input_bytes: 3 << 30,
        max_decompressed_bytes: 2 << 30,
        max_insert_depth: 16,
        max_expanded: 4_000_000,
        max_svg_bytes: 1 << 30,
    };
}

impl Default for Limits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Errors from reading or writing a drawing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

/// A drawing that was read, with what the reader noted about it.
#[derive(Debug)]
pub struct Read {
    pub drawing: Drawing2d,
    /// What was skipped, recovered or dropped, one line each. Empty for a complete read.
    pub notes: Vec<String>,
    /// The file as the engine holds it, for a lossless DWG/DXF re-save.
    pub source: Source,
}

/// The engine's document, kept for re-saving the input as DXF or DWG without going through
/// [`Drawing2d`]. Opaque on purpose.
pub struct Source(pub(crate) acadrust::CadDocument);

impl std::fmt::Debug for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Source(..)")
    }
}

/// Which file format a byte buffer is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Format {
    Dwg,
    DxfAscii,
    DxfBinary,
}

/// The format of `bytes` from its first bytes, or `None` for neither DWG nor DXF.
pub fn sniff(bytes: &[u8]) -> Option<Format> {
    if bytes.len() >= 6 && bytes.starts_with(b"AC10") && bytes[4..6].iter().all(u8::is_ascii_digit)
    {
        return Some(Format::Dwg);
    }
    if bytes.starts_with(b"AutoCAD Binary DXF") {
        return Some(Format::DxfBinary);
    }
    // ASCII DXF: the first two non-blank lines are a group code `0` and `SECTION` (or a `999`
    // comment first).
    let head = &bytes[..bytes.len().min(4096)];
    let text = String::from_utf8_lossy(head);
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    match (lines.next(), lines.next()) {
        (Some("0"), Some("SECTION")) | (Some("999"), Some(_)) => Some(Format::DxfAscii),
        _ => None,
    }
}

/// True for a path the DWG tools read: `.dwg` and `.dxf`, any case. The batch tool routes these
/// here.
pub fn is_cad_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("dwg") || e.eq_ignore_ascii_case("dxf"))
}

/// A 2D drawing: what a DWG/DXF holds that rvt-open reads and writes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Drawing2d {
    /// File version, e.g. `AC1032` (DWG 2018).
    pub version: String,
    /// `$INSUNITS`: 0 unitless, 1 inches, 2 feet, 4 millimetres, 5 centimetres, 6 metres.
    pub units: i16,
    /// `$DWGCODEPAGE`.
    pub code_page: String,
    pub layers: Vec<Layer2d>,
    pub blocks: Vec<Block2d>,
    pub layouts: Vec<Layout2d>,
    pub entities: Vec<Entity2d>,
}

/// A layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layer2d {
    pub name: String,
    pub colour: Colour,
    pub linetype: String,
    /// Lineweight in 1/100 mm; -1 ByLayer, -2 ByBlock, -3 default.
    pub lineweight: i16,
    pub on: bool,
    pub frozen: bool,
    pub locked: bool,
    pub plot: bool,
}

impl Layer2d {
    /// A visible, plotting layer with a colour index.
    pub fn new(name: impl Into<String>, aci: i16) -> Self {
        Self {
            name: name.into(),
            colour: Colour::aci(aci),
            linetype: "Continuous".into(),
            lineweight: -3,
            on: true,
            frozen: false,
            locked: false,
            plot: true,
        }
    }
}

/// A block definition (block record). Layout blocks (`*Model_Space`, `*Paper_Space…`) are
/// blocks too, with `layout` set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Block2d {
    pub name: String,
    pub base: [f64; 3],
    /// Path of the external reference, for an xref block.
    pub xref_path: Option<String>,
    /// The layout this block draws, for a layout block.
    pub layout: Option<String>,
    /// Tags of the attribute definitions in the block.
    pub attribute_tags: Vec<String>,
}

/// A layout: model space or a paper-space sheet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layout2d {
    pub name: String,
    /// Name of the block that holds its entities.
    pub block: String,
    pub tab_order: i16,
    pub is_model: bool,
    /// Paper limits (min x, min y, max x, max y) in paper units, when set.
    pub limits: Option<[f64; 4]>,
}

/// A colour: an index (ACI; 0 ByBlock, 256 ByLayer) and/or a true colour.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Colour {
    pub aci: Option<i16>,
    pub rgb: Option<[u8; 3]>,
}

impl Colour {
    pub const BY_LAYER: Colour = Colour {
        aci: Some(256),
        rgb: None,
    };
    pub const BY_BLOCK: Colour = Colour {
        aci: Some(0),
        rgb: None,
    };

    pub fn aci(index: i16) -> Self {
        Colour {
            aci: Some(index),
            rgb: None,
        }
    }

    pub fn is_by_layer(&self) -> bool {
        self.rgb.is_none() && matches!(self.aci, Some(256) | None)
    }

    pub fn is_by_block(&self) -> bool {
        self.rgb.is_none() && self.aci == Some(0)
    }

    /// The colour as RGB: the true colour, else the index looked up, else `None` (ByLayer,
    /// ByBlock).
    pub fn to_rgb(&self) -> Option<[u8; 3]> {
        if let Some(rgb) = self.rgb {
            return Some(rgb);
        }
        match self.aci? {
            i @ 1..=255 => acadrust::Color::Index(i as u8)
                .rgb()
                .map(|(r, g, b)| [r, g, b]),
            _ => None,
        }
    }
}

/// An entity: its common properties and its geometry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entity2d {
    /// Handle in hex, as the file has it. Empty for an entity rvt-open made.
    pub handle: String,
    /// DXF type name, e.g. `LINE`, `LWPOLYLINE`, `INSERT`.
    pub kind: String,
    /// The block that owns it: `*Model_Space`, a `*Paper_Space…` block, or a block definition.
    pub owner: String,
    pub layer: String,
    pub colour: Colour,
    /// Linetype name; empty for ByLayer.
    pub linetype: String,
    /// Lineweight in 1/100 mm; -1 ByLayer, -2 ByBlock, -3 default.
    pub lineweight: i16,
    /// Transparency in percent, when set on the entity.
    pub transparency: Option<f64>,
    pub geom: Geom,
    /// Names of the applications with extended data on it.
    pub xdata_apps: Vec<String>,
}

impl Entity2d {
    /// An entity on `layer` with ByLayer properties, owned by model space.
    pub fn new(kind: &str, layer: impl Into<String>, geom: Geom) -> Self {
        Self {
            handle: String::new(),
            kind: kind.into(),
            owner: MODEL_SPACE.into(),
            layer: layer.into(),
            colour: Colour::BY_LAYER,
            linetype: String::new(),
            lineweight: -1,
            transparency: None,
            geom,
            xdata_apps: Vec::new(),
        }
    }
}

/// The model-space block's name.
pub const MODEL_SPACE: &str = "*Model_Space";

/// Which space an owner block is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Space {
    Model,
    Paper,
    Block,
}

impl Space {
    pub fn of(owner: &str) -> Space {
        let lower = owner.to_ascii_lowercase();
        if lower == "*model_space" {
            Space::Model
        } else if lower.starts_with("*paper_space") {
            Space::Paper
        } else {
            Space::Block
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Space::Model => "model",
            Space::Paper => "paper",
            Space::Block => "block",
        }
    }
}

/// A point: x, y, z.
pub type P3 = [f64; 3];

/// Entity geometry, in the coordinates of the owner block (world coordinates for model space).
/// Angles are radians, counter-clockwise from +X.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum Geom {
    Line {
        a: P3,
        b: P3,
    },
    Circle {
        c: P3,
        r: f64,
    },
    Arc {
        c: P3,
        r: f64,
        start: f64,
        end: f64,
    },
    Ellipse {
        c: P3,
        /// Major axis end point, relative to the centre.
        major: P3,
        ratio: f64,
        start: f64,
        end: f64,
    },
    /// A 2D polyline: vertices as (x, y, bulge), at elevation `z`.
    Polyline {
        pts: Vec<P3>,
        closed: bool,
        z: f64,
    },
    /// Points joined by straight segments (3D polylines, splines as their fit or control points,
    /// leaders, solids and faces as outlines).
    Path {
        pts: Vec<P3>,
        closed: bool,
    },
    Point {
        p: P3,
    },
    Text {
        at: P3,
        height: f64,
        rotation: f64,
        value: String,
    },
    Insert {
        block: String,
        at: P3,
        scale: P3,
        rotation: f64,
        /// Attribute (tag, value) pairs.
        attributes: Vec<(String, String)>,
    },
    Dimension {
        /// The anonymous block that draws it.
        block: String,
        measurement: f64,
        /// The text override, when set.
        text: Option<String>,
        at: P3,
        /// The two points an aligned dimension measures between, for a dimension drawn from a
        /// model (empty when read from a file). With no block, the writer writes it as an aligned
        /// DIMENSION through these points.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        points: Vec<P3>,
    },
    Hatch {
        /// Boundary loops as (x, y, bulge).
        loops: Vec<Vec<P3>>,
        pattern: String,
        solid: bool,
    },
    Viewport {
        /// Centre on the sheet, paper units.
        center: P3,
        width: f64,
        height: f64,
        /// Model point at the viewport centre.
        view_center: [f64; 2],
        /// Model height shown; scale = height / view_height.
        view_height: f64,
        id: i16,
    },
    /// Not drawn: kept with its bounding box (min, max), when the engine gives one.
    Other {
        bbox: Option<[f64; 6]>,
    },
}

/// A 2D affine transform: x' = a·x + c·y + e, y' = b·x + d·y + f.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Affine {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

impl Affine {
    pub const IDENTITY: Affine = Affine {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    /// The transform of an insert: move the block base to the origin, scale, rotate, move to the
    /// insertion point.
    pub fn insert(base: P3, at: P3, scale: P3, rotation: f64) -> Affine {
        let (s, c) = rotation.sin_cos();
        let (a, b, cc, d) = (c * scale[0], s * scale[0], -s * scale[1], c * scale[1]);
        Affine {
            a,
            b,
            c: cc,
            d,
            e: at[0] - (a * base[0] + cc * base[1]),
            f: at[1] - (b * base[0] + d * base[1]),
        }
    }

    /// `self` after `inner`: apply `inner` first.
    pub fn then(&self, inner: &Affine) -> Affine {
        Affine {
            a: self.a * inner.a + self.c * inner.b,
            b: self.b * inner.a + self.d * inner.b,
            c: self.a * inner.c + self.c * inner.d,
            d: self.b * inner.c + self.d * inner.d,
            e: self.a * inner.e + self.c * inner.f + self.e,
            f: self.b * inner.e + self.d * inner.f + self.f,
        }
    }

    pub fn apply(&self, p: P3) -> P3 {
        [
            self.a * p[0] + self.c * p[1] + self.e,
            self.b * p[0] + self.d * p[1] + self.f,
            p[2],
        ]
    }

    pub fn det(&self) -> f64 {
        self.a * self.d - self.b * self.c
    }

    /// Mean linear scale.
    pub fn scale(&self) -> f64 {
        self.det().abs().sqrt()
    }

    /// Rotation of the X axis.
    pub fn angle(&self) -> f64 {
        self.b.atan2(self.a)
    }
}

impl Geom {
    /// The geometry under `t`. Circles and arcs take the mean scale: a non-uniform insert scale
    /// keeps them circular.
    // ponytail: non-uniform scale of circles/arcs/text is approximated by the mean scale; turn them into ellipses if a drawing needs it.
    pub fn transformed(&self, t: &Affine) -> Geom {
        let mirror = t.det() < 0.0;
        let rot = t.angle();
        let angle = |a: f64| if mirror { rot - a } else { rot + a };
        let bulge = |b: f64| if mirror { -b } else { b };
        let pts = |v: &[P3], bulged: bool| -> Vec<P3> {
            v.iter()
                .map(|p| {
                    let q = t.apply(*p);
                    if bulged { [q[0], q[1], bulge(p[2])] } else { q }
                })
                .collect()
        };
        match self {
            Geom::Line { a, b } => Geom::Line {
                a: t.apply(*a),
                b: t.apply(*b),
            },
            Geom::Circle { c, r } => Geom::Circle {
                c: t.apply(*c),
                r: r * t.scale(),
            },
            Geom::Arc { c, r, start, end } => {
                let (s, e) = if mirror {
                    (angle(*end), angle(*start))
                } else {
                    (angle(*start), angle(*end))
                };
                Geom::Arc {
                    c: t.apply(*c),
                    r: r * t.scale(),
                    start: s,
                    end: e,
                }
            }
            Geom::Ellipse {
                c,
                major,
                ratio,
                start,
                end,
            } => {
                let m = [
                    t.a * major[0] + t.c * major[1],
                    t.b * major[0] + t.d * major[1],
                    major[2],
                ];
                let (s, e) = if mirror {
                    (-end, -start)
                } else {
                    (*start, *end)
                };
                Geom::Ellipse {
                    c: t.apply(*c),
                    major: m,
                    ratio: *ratio,
                    start: s,
                    end: e,
                }
            }
            Geom::Polyline { pts: p, closed, z } => Geom::Polyline {
                pts: pts(p, true),
                closed: *closed,
                z: *z,
            },
            Geom::Path { pts: p, closed } => Geom::Path {
                pts: pts(p, false),
                closed: *closed,
            },
            Geom::Point { p } => Geom::Point { p: t.apply(*p) },
            Geom::Text {
                at,
                height,
                rotation,
                value,
            } => Geom::Text {
                at: t.apply(*at),
                height: height * t.scale(),
                rotation: angle(*rotation),
                value: value.clone(),
            },
            Geom::Insert {
                block,
                at,
                scale,
                rotation,
                attributes,
            } => Geom::Insert {
                block: block.clone(),
                at: t.apply(*at),
                scale: [scale[0] * t.scale(), scale[1] * t.scale(), scale[2]],
                rotation: angle(*rotation),
                attributes: attributes.clone(),
            },
            Geom::Dimension {
                block,
                measurement,
                text,
                at,
                points,
            } => Geom::Dimension {
                block: block.clone(),
                measurement: *measurement,
                text: text.clone(),
                at: t.apply(*at),
                points: points.iter().map(|p| t.apply(*p)).collect(),
            },
            Geom::Hatch {
                loops,
                pattern,
                solid,
            } => Geom::Hatch {
                loops: loops.iter().map(|l| pts(l, true)).collect(),
                pattern: pattern.clone(),
                solid: *solid,
            },
            Geom::Viewport { .. } => self.clone(),
            Geom::Other { bbox } => Geom::Other {
                bbox: bbox.map(|b| {
                    let (p, q) = (t.apply([b[0], b[1], b[2]]), t.apply([b[3], b[4], b[5]]));
                    [
                        p[0].min(q[0]),
                        p[1].min(q[1]),
                        p[2],
                        p[0].max(q[0]),
                        p[1].max(q[1]),
                        q[2],
                    ]
                }),
            },
        }
    }

    /// Points along the geometry, every `step` drawing units at most (arcs and bulges are
    /// sampled too), for scoring and bounds. Text and inserts give their insertion point.
    pub fn samples(&self, step: f64) -> Vec<[f64; 2]> {
        let step = step.max(1e-9);
        let mut out = Vec::new();
        let seg = |out: &mut Vec<[f64; 2]>, a: [f64; 2], b: [f64; 2]| {
            let n = ((b[0] - a[0]).hypot(b[1] - a[1]) / step)
                .ceil()
                .clamp(1.0, 100_000.0) as usize;
            for i in 0..=n {
                let k = i as f64 / n as f64;
                out.push([a[0] + (b[0] - a[0]) * k, a[1] + (b[1] - a[1]) * k]);
            }
        };
        let arc = |out: &mut Vec<[f64; 2]>, c: [f64; 2], r: f64, s: f64, sweep: f64| {
            let n = ((r * sweep.abs()) / step).ceil().clamp(1.0, 100_000.0) as usize;
            for i in 0..=n {
                let a = s + sweep * i as f64 / n as f64;
                out.push([c[0] + r * a.cos(), c[1] + r * a.sin()]);
            }
        };
        match self {
            Geom::Line { a, b } => seg(&mut out, [a[0], a[1]], [b[0], b[1]]),
            Geom::Circle { c, r } => arc(&mut out, [c[0], c[1]], *r, 0.0, std::f64::consts::TAU),
            Geom::Arc { c, r, start, end } => {
                arc(&mut out, [c[0], c[1]], *r, *start, ccw_sweep(*start, *end))
            }
            Geom::Ellipse {
                c,
                major,
                ratio,
                start,
                end,
            } => {
                let len = major[0].hypot(major[1]);
                let rot = major[1].atan2(major[0]);
                let sweep = ccw_sweep(*start, *end);
                let n = ((len * sweep.abs()) / step).ceil().clamp(1.0, 100_000.0) as usize;
                let (sr, cr) = rot.sin_cos();
                for i in 0..=n {
                    let t = start + sweep * i as f64 / n as f64;
                    let (x, y) = (len * t.cos(), len * ratio * t.sin());
                    out.push([c[0] + x * cr - y * sr, c[1] + x * sr + y * cr]);
                }
            }
            Geom::Polyline { pts, closed, .. } => {
                for (a, b) in polyline_segments(pts, *closed) {
                    if a[2].abs() < 1e-12 {
                        seg(&mut out, [a[0], a[1]], [b[0], b[1]]);
                    } else {
                        let (c, r, s, sweep) = bulge_arc(a, b);
                        arc(&mut out, c, r, s, sweep);
                    }
                }
            }
            Geom::Path { pts, closed } => {
                for (a, b) in polyline_segments(pts, *closed) {
                    seg(&mut out, [a[0], a[1]], [b[0], b[1]]);
                }
                if pts.len() == 1 {
                    out.push([pts[0][0], pts[0][1]]);
                }
            }
            Geom::Hatch { loops, .. } => {
                for l in loops {
                    out.extend(
                        Geom::Polyline {
                            pts: l.clone(),
                            closed: true,
                            z: 0.0,
                        }
                        .samples(step),
                    );
                }
            }
            Geom::Point { p } => out.push([p[0], p[1]]),
            Geom::Text { at, .. } | Geom::Insert { at, .. } | Geom::Dimension { at, .. } => {
                out.push([at[0], at[1]])
            }
            Geom::Viewport {
                center,
                width,
                height,
                ..
            } => {
                let (x0, y0, x1, y1) = (
                    center[0] - width / 2.0,
                    center[1] - height / 2.0,
                    center[0] + width / 2.0,
                    center[1] + height / 2.0,
                );
                for (a, b) in [
                    ([x0, y0], [x1, y0]),
                    ([x1, y0], [x1, y1]),
                    ([x1, y1], [x0, y1]),
                    ([x0, y1], [x0, y0]),
                ] {
                    seg(&mut out, a, b);
                }
            }
            Geom::Other { bbox } => {
                if let Some(b) = bbox {
                    out.push([b[0], b[1]]);
                    out.push([b[3], b[4]]);
                }
            }
        }
        out
    }

    /// Length of the curve (line, arc, circle, ellipse, polyline, path); `None` for the rest.
    pub fn length(&self) -> Option<f64> {
        Some(match self {
            Geom::Line { a, b } => (b[0] - a[0]).hypot(b[1] - a[1]).hypot(b[2] - a[2]),
            Geom::Circle { r, .. } => std::f64::consts::TAU * r,
            Geom::Arc { r, start, end, .. } => r * ccw_sweep(*start, *end),
            Geom::Polyline { pts, closed, .. } => polyline_segments(pts, *closed)
                .map(|(a, b)| {
                    if a[2].abs() < 1e-12 {
                        (b[0] - a[0]).hypot(b[1] - a[1])
                    } else {
                        let (_, r, _, sweep) = bulge_arc(a, b);
                        r * sweep.abs()
                    }
                })
                .sum(),
            Geom::Path { pts, closed } => polyline_segments(pts, *closed)
                .map(|(a, b)| (b[0] - a[0]).hypot(b[1] - a[1]).hypot(b[2] - a[2]))
                .sum(),
            Geom::Ellipse { .. } => {
                let s = self.samples(self.chord_step() / 4.0);
                s.windows(2)
                    .map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]))
                    .sum()
            }
            _ => return None,
        })
    }

    /// Enclosed area of a circle, a closed polyline or a hatch (outer loops minus inner); `None`
    /// for the rest.
    pub fn area(&self) -> Option<f64> {
        match self {
            Geom::Circle { r, .. } => Some(std::f64::consts::PI * r * r),
            Geom::Polyline {
                pts, closed: true, ..
            } => Some(bulged_area(pts).abs()),
            Geom::Hatch { loops, .. } => {
                // Signed sum: loops wound against the first one are holes.
                let mut areas = loops.iter().map(|l| bulged_area(l));
                let first = areas.next()?;
                Some(
                    areas
                        .fold(first.abs(), |acc, a| {
                            if a.signum() == first.signum() {
                                acc + a.abs()
                            } else {
                                acc - a.abs()
                            }
                        })
                        .abs(),
                )
            }
            _ => None,
        }
    }

    /// Bounding box (min x, min y, max x, max y), from [`Geom::samples`].
    pub fn bbox(&self) -> Option<[f64; 4]> {
        bounds(self.samples(f64::MAX).into_iter().chain(self.fine_extra()))
    }

    // Arcs sampled at an infinite step give only their end points: add the quadrant points so
    // the box holds the whole curve.
    fn fine_extra(&self) -> Vec<[f64; 2]> {
        match self {
            Geom::Circle { c, r } => vec![[c[0] - r, c[1] - r], [c[0] + r, c[1] + r]],
            Geom::Arc { c, r, start, end } => {
                let sweep = ccw_sweep(*start, *end);
                (0..4)
                    .map(|q| q as f64 * std::f64::consts::FRAC_PI_2)
                    .filter(|a| ccw_sweep(*start, *a) <= sweep)
                    .map(|a| [c[0] + r * a.cos(), c[1] + r * a.sin()])
                    .collect()
            }
            Geom::Ellipse { .. } => self.samples(self.chord_step()),
            Geom::Polyline { pts, .. } if pts.iter().any(|p| p[2].abs() > 1e-12) => {
                self.samples(self.chord_step())
            }
            Geom::Hatch { .. } => self.samples(self.chord_step()),
            _ => Vec::new(),
        }
    }

    fn chord_step(&self) -> f64 {
        let s = self.samples(f64::MAX);
        bounds(s.into_iter())
            .map(|b| ((b[2] - b[0]).max(b[3] - b[1]) / 64.0).max(1e-6))
            .unwrap_or(1.0)
    }
}

/// Bounding box of points.
pub fn bounds(points: impl Iterator<Item = [f64; 2]>) -> Option<[f64; 4]> {
    let mut b: Option<[f64; 4]> = None;
    for p in points {
        if !(p[0].is_finite() && p[1].is_finite()) {
            continue;
        }
        b = Some(match b {
            None => [p[0], p[1], p[0], p[1]],
            Some(b) => [
                b[0].min(p[0]),
                b[1].min(p[1]),
                b[2].max(p[0]),
                b[3].max(p[1]),
            ],
        });
    }
    b
}

/// Counter-clockwise sweep from `start` to `end`, in (0, 2π].
pub fn ccw_sweep(start: f64, end: f64) -> f64 {
    let s = (end - start).rem_euclid(std::f64::consts::TAU);
    if s < 1e-12 { std::f64::consts::TAU } else { s }
}

/// Consecutive vertex pairs of a polyline, with the closing pair when closed.
pub fn polyline_segments(pts: &[P3], closed: bool) -> impl Iterator<Item = (P3, P3)> + '_ {
    let n = pts.len();
    let count = if closed && n > 1 {
        n
    } else {
        n.saturating_sub(1)
    };
    (0..count).map(move |i| (pts[i], pts[(i + 1) % n]))
}

/// The arc of a bulged segment from `a` to `b` (bulge in `a[2]`): centre, radius, start angle and
/// signed sweep.
pub fn bulge_arc(a: P3, b: P3) -> ([f64; 2], f64, f64, f64) {
    let bulge = a[2];
    let sweep = 4.0 * bulge.atan();
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let chord = dx.hypot(dy).max(1e-300);
    let r = chord / (2.0 * (sweep / 2.0).sin().abs());
    // The centre is off the chord midpoint along its left normal, by h·(1 - b²)/(2b).
    let d = chord / 2.0 * (1.0 - bulge * bulge) / (2.0 * bulge);
    let c = [
        (a[0] + b[0]) / 2.0 - dy / chord * d,
        (a[1] + b[1]) / 2.0 + dx / chord * d,
    ];
    let start = (a[1] - c[1]).atan2(a[0] - c[0]);
    (c, r, start, sweep)
}

/// Signed area of a polyline with bulges (shoelace plus each bulge's circular segment).
pub fn bulged_area(pts: &[P3]) -> f64 {
    let mut area = 0.0;
    for (a, b) in polyline_segments(pts, true) {
        area += (a[0] * b[1] - b[0] * a[1]) / 2.0;
        if a[2].abs() > 1e-12 {
            let (_, r, _, sweep) = bulge_arc(a, b);
            area += r * r * (sweep - sweep.sin()) / 2.0;
        }
    }
    area
}

/// MTEXT with its formatting codes removed: `\P` becomes a new line, `{`, `}` and `\f…;`-style
/// codes go, `%%c`/`%%d`/`%%p` become ⌀ ° ±.
pub fn plain_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' | '}' => {}
            '\\' => match chars.next() {
                Some('P') | Some('X') => out.push('\n'),
                Some('~') => out.push(' '),
                Some(c @ ('\\' | '{' | '}')) => out.push(c),
                Some('U') if chars.peek() == Some(&'+') => {
                    chars.next();
                    let hex: String = chars.by_ref().take(4).collect();
                    if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                        out.push(ch);
                    }
                }
                // Codes with an argument up to `;`.
                Some('f' | 'F' | 'H' | 'W' | 'Q' | 'T' | 'A' | 'C' | 'c' | 'p' | 'S') => {
                    for c in chars.by_ref() {
                        if c == ';' {
                            break;
                        }
                    }
                }
                // Switch codes without an argument (underline, overline, strike).
                Some(_) | None => {}
            },
            '%' if chars.peek() == Some(&'%') => {
                chars.next();
                match chars.next() {
                    Some('c' | 'C') => out.push('⌀'),
                    Some('d' | 'D') => out.push('°'),
                    Some('p' | 'P') => out.push('±'),
                    Some('%') => out.push('%'),
                    _ => {}
                }
            }
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniff_tells_dwg_dxf_and_neither() {
        assert_eq!(sniff(b"AC1032\0\0\0"), Some(Format::Dwg));
        assert_eq!(
            sniff(b"  0\r\nSECTION\r\n  2\r\nHEADER"),
            Some(Format::DxfAscii)
        );
        assert_eq!(
            sniff(b"AutoCAD Binary DXF\r\n\x1a\0"),
            Some(Format::DxfBinary)
        );
        assert_eq!(sniff(b"%PDF-1.7"), None);
        assert_eq!(sniff(b"AC10"), None);
    }

    #[test]
    fn bulge_of_one_is_a_half_circle() {
        // From (0,0) to (2,0) with bulge 1: counter-clockwise half circle below the chord.
        let (c, r, s, sweep) = bulge_arc([0.0, 0.0, 1.0], [2.0, 0.0, 0.0]);
        assert!((c[0] - 1.0).abs() < 1e-12 && c[1].abs() < 1e-12, "{c:?}");
        assert!((r - 1.0).abs() < 1e-12);
        assert!((sweep - std::f64::consts::PI).abs() < 1e-12);
        let end = [c[0] + r * (s + sweep).cos(), c[1] + r * (s + sweep).sin()];
        assert!(
            (end[0] - 2.0).abs() < 1e-9 && end[1].abs() < 1e-9,
            "{end:?}"
        );
        // The mid point is below the chord: counter-clockwise from (0,0) round to (2,0).
        let mid = [
            c[0] + r * (s + sweep / 2.0).cos(),
            c[1] + r * (s + sweep / 2.0).sin(),
        ];
        assert!(mid[1] < -0.99, "{mid:?}");
    }

    #[test]
    fn bulged_area_of_a_full_circle_from_two_half_circles() {
        let area = bulged_area(&[[0.0, 0.0, 1.0], [2.0, 0.0, 1.0]]);
        assert!((area.abs() - std::f64::consts::PI).abs() < 1e-9, "{area}");
        let square = bulged_area(&[
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ]);
        assert!((square - 1.0).abs() < 1e-12);
    }

    #[test]
    fn insert_transform_moves_scales_rotates() {
        let t = Affine::insert(
            [1.0, 0.0, 0.0],
            [10.0, 10.0, 0.0],
            [2.0, 2.0, 1.0],
            std::f64::consts::FRAC_PI_2,
        );
        let p = t.apply([2.0, 0.0, 0.0]);
        assert!(
            (p[0] - 10.0).abs() < 1e-12 && (p[1] - 12.0).abs() < 1e-12,
            "{p:?}"
        );
        let outer = Affine::insert([0.0; 3], [5.0, 0.0, 0.0], [1.0, 1.0, 1.0], 0.0);
        let q = outer.then(&t).apply([2.0, 0.0, 0.0]);
        assert!(
            (q[0] - 15.0).abs() < 1e-12 && (q[1] - 12.0).abs() < 1e-12,
            "{q:?}"
        );
    }

    #[test]
    fn arc_bbox_holds_its_quadrant_points() {
        let g = Geom::Arc {
            c: [0.0; 3],
            r: 1.0,
            start: 0.0,
            end: std::f64::consts::PI,
        };
        let b = g.bbox().unwrap();
        assert!(
            (b[3] - 1.0).abs() < 1e-12 && (b[0] + 1.0).abs() < 1e-12 && b[1].abs() < 1e-12,
            "{b:?}"
        );
    }

    #[test]
    fn plain_text_drops_mtext_codes() {
        assert_eq!(
            plain_text(r"{\fArial|b1;Level\P}2 %%c50 \U+00B2"),
            "Level\n2 ⌀50 ²"
        );
    }
}
