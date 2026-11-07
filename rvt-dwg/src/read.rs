//! Reading DWG and DXF through the engine, guarded, and mapping its document to [`Drawing2d`].

use crate::{
    Block2d, Colour, Drawing2d, Entity2d, Error, Format, Geom, Layer2d, Layout2d, Limits, P3, Read,
    Source, sniff,
};
use acadrust::entities::{Dimension, EntityType};
use acadrust::objects::ObjectType;
use acadrust::{
    CadDocument, Color, DwgReadOptions, DwgReader, DxfReader, Entity, LineWeight, Transparency,
};
use std::collections::HashMap;
use std::io::{Cursor, Read as _};
use std::panic::{AssertUnwindSafe, catch_unwind};

/// Most read diagnostics copied into the notes; the rest are counted.
const MAX_NOTED_DIAGNOSTICS: usize = 50;

/// Read a DWG or DXF file. See [`read_bytes`]. A DXF file is streamed from disk, so a DXF larger
/// than memory would hold twice is read without loading it first.
pub fn read_file(path: &std::path::Path, limits: Limits) -> Result<Read, Error> {
    let at = |e: std::io::Error| Error(format!("{}: {e}", path.display()));
    let len = std::fs::metadata(path).map_err(at)?.len();
    let mut head = Vec::with_capacity(4096);
    std::fs::File::open(path)
        .map_err(at)?
        .take(4096)
        .read_to_end(&mut head)
        .map_err(at)?;
    match sniff(&head) {
        Some(format @ (Format::DxfAscii | Format::DxfBinary)) => {
            check_size(len, format, limits)?;
            read_input(Input::File(path), format, len, limits)
        }
        _ => {
            check_size(len, Format::Dwg, limits)?;
            let bytes = std::fs::read(path).map_err(at)?;
            read_bytes(&bytes, limits)
        }
    }
}

/// The input limit for `format`: [`Limits::max_dxf_input_bytes`] for DXF, else
/// [`Limits::max_input_bytes`].
fn check_size(len: u64, format: Format, limits: Limits) -> Result<(), Error> {
    let max = match format {
        Format::DxfAscii | Format::DxfBinary => limits.max_dxf_input_bytes,
        Format::Dwg => limits.max_input_bytes,
    };
    if len > max as u64 {
        return Err(Error(format!(
            "the file is {:.1} MB, larger than the {:.1} MB limit",
            len as f64 / 1e6,
            max as f64 / 1e6
        )));
    }
    Ok(())
}

/// Read a DWG (R13-2018 and R11/12) or DXF (ASCII or binary) from bytes.
///
/// A strict read comes first. If it fails, the file is read again in the engine's recovery mode
/// and the strict error becomes the first note. Either way, every record the engine skipped and
/// every error it recovered from is noted, and an empty result from a file that has records is
/// noted too, so a partial read never passes for a complete one.
pub fn read_bytes(bytes: &[u8], limits: Limits) -> Result<Read, Error> {
    let format = sniff(bytes);
    check_size(bytes.len() as u64, format.unwrap_or(Format::Dwg), limits)?;
    let format = format.ok_or_else(|| Error("not a DWG or DXF file".into()))?;
    read_input(Input::Bytes(bytes), format, bytes.len() as u64, limits)
}

/// Where the engine reads from.
#[derive(Clone, Copy)]
enum Input<'a> {
    Bytes(&'a [u8]),
    File(&'a std::path::Path),
}

fn read_input(input: Input, format: Format, len: u64, limits: Limits) -> Result<Read, Error> {
    // The smallest DWG file header is 0x100 bytes.
    if format == Format::Dwg && len < 0x100 {
        return Err(Error(format!("{len} bytes is too short for a DWG file")));
    }
    let mut notes = Vec::new();
    let outcome = match engine_read(input, format, false, limits) {
        Ok(o) => o,
        Err(strict) => {
            let o = engine_read(input, format, true, limits)
                .map_err(|e| Error(format!("{strict}; recovery read: {e}")))?;
            notes.push(format!(
                "read in recovery mode: the strict read failed ({strict})"
            ));
            o
        }
    };
    let acadrust::ReadOutcome { document, stats } = outcome;
    notes.extend(stat_notes(&stats));
    let drawing = catch_unwind(AssertUnwindSafe(|| to_drawing(&document)))
        .map_err(|p| Error(format!("mapping the drawing failed: {}", panic_message(&p))))?;
    if drawing.entities.is_empty() && drawing.layers.is_empty() && drawing.blocks.is_empty() {
        notes.push("nothing was read from this file: it may be damaged".into());
    }
    Ok(Read {
        drawing,
        notes,
        source: Source(document),
    })
}

fn engine_read(
    input: Input,
    format: Format,
    failsafe: bool,
    limits: Limits,
) -> Result<acadrust::ReadOutcome, Error> {
    let file = match input {
        Input::File(path) => {
            Some(std::fs::File::open(path).map_err(|e| Error(format!("{}: {e}", path.display())))?)
        }
        Input::Bytes(_) => None,
    };
    let data = match input {
        Input::Bytes(bytes) => bytes.to_vec(),
        Input::File(_) => Vec::new(),
    };
    let result = catch_unwind(AssertUnwindSafe(move || match (format, file) {
        (Format::Dwg, _) => {
            let options = DwgReadOptions {
                failsafe,
                ..DwgReadOptions::default()
            }
            .with_max_decompressed_bytes(limits.max_decompressed_bytes);
            DwgReader::from_stream_with_options(Cursor::new(data), options).read_with_stats()
        }
        (Format::DxfAscii | Format::DxfBinary, file) => {
            let configuration = acadrust::DxfReaderConfiguration {
                failsafe,
                ..Default::default()
            };
            match file {
                // The engine buffers the file itself.
                Some(file) => DxfReader::from_reader(file)?,
                None => DxfReader::from_reader(Cursor::new(data))?,
            }
            .with_configuration(configuration)
            .read_with_stats()
        }
    }));
    match result {
        Ok(Ok(o)) => Ok(o),
        Ok(Err(e)) => Err(Error(e.to_string())),
        Err(p) => Err(Error(format!(
            "the reader stopped on this file: {}",
            panic_message(&p)
        ))),
    }
}

pub(crate) fn panic_message(p: &Box<dyn std::any::Any + Send>) -> String {
    p.downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| p.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "panic".into())
}

fn stat_notes(stats: &acadrust::ReadStats) -> Vec<String> {
    let mut notes = Vec::new();
    if stats.skipped_source_records > 0 {
        let mut by_type: std::collections::BTreeMap<&str, usize> = Default::default();
        for d in &stats.diagnostics {
            if let Some(t) = d.record_type.as_deref() {
                *by_type.entry(t).or_default() += 1;
            }
        }
        let kinds = by_type
            .iter()
            .map(|(t, n)| format!("{t} {n}"))
            .collect::<Vec<_>>()
            .join(", ");
        notes.push(format!(
            "{} of {} records could not be read and were left out{}",
            stats.skipped_source_records,
            stats.source_records,
            if kinds.is_empty() {
                String::new()
            } else {
                format!(" ({kinds})")
            }
        ));
    }
    if stats.recovered_errors > 0 {
        notes.push(format!(
            "the reader recovered from {} errors",
            stats.recovered_errors
        ));
    }
    if stats.record_stream_read && !stats.stream_completed {
        notes.push("the record stream ended early".into());
    }
    for d in stats.diagnostics.iter().take(MAX_NOTED_DIAGNOSTICS) {
        notes.push(format!("{}: {}", d.code, d.message));
    }
    if stats.diagnostics.len() > MAX_NOTED_DIAGNOSTICS {
        notes.push(format!(
            "{} more reader diagnostics",
            stats.diagnostics.len() - MAX_NOTED_DIAGNOSTICS
        ));
    }
    notes
}

fn v3(v: acadrust::Vector3) -> P3 {
    [v.x, v.y, v.z]
}

fn colour(c: Color) -> Colour {
    match c {
        Color::ByLayer => Colour::BY_LAYER,
        Color::ByBlock => Colour::BY_BLOCK,
        Color::None => Colour {
            aci: Some(257),
            rgb: None,
        },
        Color::Index(i) => Colour::aci(i as i16),
        Color::Rgb { r, g, b } => Colour {
            aci: None,
            rgb: Some([r, g, b]),
        },
    }
}

fn lineweight(w: LineWeight) -> i16 {
    match w {
        LineWeight::ByLayer => -1,
        LineWeight::ByBlock => -2,
        LineWeight::Default => -3,
        LineWeight::Value(v) => v,
    }
}

fn transparency(t: Transparency) -> Option<f64> {
    match t {
        Transparency::ByLayer | Transparency::ByBlock => None,
        other => Some(other.as_percent()),
    }
}

pub(crate) fn to_drawing(doc: &CadDocument) -> Drawing2d {
    let mut block_names: HashMap<acadrust::Handle, String> = HashMap::new();
    for br in doc.block_records.iter() {
        block_names.insert(br.handle, canonical_block(&br.name));
    }
    let mut layouts: Vec<Layout2d> = doc
        .objects
        .values()
        .filter_map(|o| match o {
            ObjectType::Layout(l) => {
                let block = block_names
                    .get(&l.block_record)
                    .cloned()
                    .unwrap_or_default();
                let is_model = l.name.eq_ignore_ascii_case("Model")
                    || block.eq_ignore_ascii_case(crate::MODEL_SPACE);
                let limits = [
                    l.min_limits.0,
                    l.min_limits.1,
                    l.max_limits.0,
                    l.max_limits.1,
                ];
                let has_limits = limits[2] > limits[0] && limits[3] > limits[1];
                Some(Layout2d {
                    name: l.name.clone(),
                    block,
                    tab_order: l.tab_order,
                    is_model,
                    limits: has_limits.then_some(limits),
                })
            }
            _ => None,
        })
        .collect();
    layouts.sort_by(|a, b| {
        (!a.is_model, a.tab_order, &a.name).cmp(&(!b.is_model, b.tab_order, &b.name))
    });
    let layout_of: HashMap<&str, &str> = layouts
        .iter()
        .map(|l| (l.block.as_str(), l.name.as_str()))
        .collect();

    let layers = doc
        .layers
        .iter()
        .map(|l| Layer2d {
            name: l.name.clone(),
            colour: colour(l.color),
            linetype: l.line_type.clone(),
            lineweight: lineweight(l.line_weight),
            on: !l.flags.off,
            frozen: l.flags.frozen,
            locked: l.flags.locked,
            plot: l.is_plottable,
        })
        .collect();

    let mut entities = Vec::with_capacity(doc.entity_count());
    let mut tags: HashMap<String, Vec<String>> = HashMap::new();
    for e in doc.entities() {
        if matches!(e, EntityType::Seqend(_) | EntityType::AttributeEntity(_)) {
            continue;
        }
        let common = e.common();
        let owner = block_names
            .get(&common.owner_handle)
            .cloned()
            .unwrap_or_default();
        if let EntityType::AttributeDefinition(a) = e {
            tags.entry(owner.clone()).or_default().push(a.tag.clone());
        }
        let ent = e.as_entity();
        entities.push(Entity2d {
            handle: format!("{:X}", common.handle),
            kind: kind_name(e),
            owner,
            layer: common.layer.clone(),
            colour: colour(common.color),
            linetype: if common.linetype.eq_ignore_ascii_case("ByLayer") {
                String::new()
            } else {
                common.linetype.clone()
            },
            lineweight: lineweight(common.line_weight),
            transparency: transparency(common.transparency),
            geom: geom(e, ent),
            xdata_apps: common
                .extended_data
                .records()
                .iter()
                .map(|r| r.application_name.clone())
                .collect(),
        });
    }

    let blocks = doc
        .block_records
        .iter()
        .map(|br| Block2d {
            name: canonical_block(&br.name),
            base: v3(br.base_point),
            xref_path: (br.flags.is_xref || br.flags.is_xref_overlay || !br.xref_path.is_empty())
                .then(|| br.xref_path.clone()),
            layout: layout_of
                .get(canonical_block(&br.name).as_str())
                .map(|s| s.to_string()),
            attribute_tags: tags.remove(&canonical_block(&br.name)).unwrap_or_default(),
        })
        .collect();

    Drawing2d {
        version: doc.version.as_str().to_string(),
        units: doc.header.insertion_units,
        code_page: doc.header.code_page.clone(),
        layers,
        blocks,
        layouts,
        entities,
    }
}

/// Layout block names in one spelling (`*Model_Space`, `*Paper_Space…`): older files write them in
/// capitals.
fn canonical_block(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    if lower == "*model_space" {
        crate::MODEL_SPACE.to_string()
    } else if let Some(rest) = lower.strip_prefix("*paper_space") {
        format!("*Paper_Space{rest}")
    } else {
        name.to_string()
    }
}

fn kind_name(e: &EntityType) -> String {
    match e {
        EntityType::Dimension(_) => "DIMENSION".into(),
        EntityType::Unknown(u) if !u.dxf_name.is_empty() => u.dxf_name.clone(),
        EntityType::Extended(x) => x.class_name().to_string(),
        EntityType::Polyline2D(_) | EntityType::Polyline(_) | EntityType::Polyline3D(_) => {
            "POLYLINE".into()
        }
        other => other.as_entity().entity_type().to_string(),
    }
}

/// The Z axis is flipped (normal (0, 0, -1), what a mirrored drawing writes): object coordinates
/// then have X mirrored.
// ponytail: only the flipped-Z case of arbitrary-axis object coordinates is handled; other normals are taken as +Z.
fn flipped(n: acadrust::Vector3) -> bool {
    n.z < -0.5
}

fn ocs(p: P3, flip: bool) -> P3 {
    if flip { [-p[0], p[1], -p[2]] } else { p }
}

fn geom(e: &EntityType, ent: &dyn Entity) -> Geom {
    match e {
        EntityType::Line(l) => Geom::Line {
            a: v3(l.start),
            b: v3(l.end),
        },
        EntityType::Circle(c) => Geom::Circle {
            c: ocs(v3(c.center), flipped(c.normal)),
            r: c.radius,
        },
        EntityType::Arc(a) => {
            let flip = flipped(a.normal);
            let (start, end) = if flip {
                (
                    std::f64::consts::PI - a.end_angle,
                    std::f64::consts::PI - a.start_angle,
                )
            } else {
                (a.start_angle, a.end_angle)
            };
            Geom::Arc {
                c: ocs(v3(a.center), flip),
                r: a.radius,
                start,
                end,
            }
        }
        EntityType::Ellipse(el) => Geom::Ellipse {
            c: v3(el.center),
            major: v3(el.major_axis),
            ratio: el.minor_axis_ratio,
            start: el.start_parameter,
            end: el.end_parameter,
        },
        EntityType::LwPolyline(p) => {
            let flip = flipped(p.normal);
            Geom::Polyline {
                pts: p
                    .vertices
                    .iter()
                    .map(|v| {
                        let q = ocs([v.location.x, v.location.y, 0.0], flip);
                        [q[0], q[1], if flip { -v.bulge } else { v.bulge }]
                    })
                    .collect(),
                closed: p.is_closed,
                z: p.elevation,
            }
        }
        EntityType::Polyline2D(p) => {
            let flip = flipped(p.normal);
            Geom::Polyline {
                pts: p
                    .vertices
                    .iter()
                    .map(|v| {
                        let q = ocs(v3(v.location), flip);
                        [q[0], q[1], if flip { -v.bulge } else { v.bulge }]
                    })
                    .collect(),
                closed: p.flags.is_closed(),
                z: p.elevation,
            }
        }
        EntityType::Polyline3D(p) => Geom::Path {
            pts: p.vertices.iter().map(|v| v3(v.position)).collect(),
            closed: p.flags.closed,
        },
        EntityType::Polyline(p) => Geom::Path {
            pts: p.vertices.iter().map(|v| v3(v.location)).collect(),
            closed: p.flags.is_closed(),
        },
        EntityType::Spline(s) => {
            let pts = if s.fit_points.len() >= 2 {
                &s.fit_points
            } else {
                &s.control_points
            };
            Geom::Path {
                pts: pts.iter().map(|p| v3(*p)).collect(),
                closed: s.flags.closed,
            }
        }
        EntityType::Leader(l) => Geom::Path {
            pts: l.vertices.iter().map(|p| v3(*p)).collect(),
            closed: false,
        },
        EntityType::Solid(s) => {
            // A SOLID's corners run 1, 2, 4, 3 round its outline.
            let pts = [
                s.first_corner,
                s.second_corner,
                s.fourth_corner,
                s.third_corner,
            ]
            .map(v3);
            let flip = flipped(s.normal);
            Geom::Path {
                pts: pts.iter().map(|p| ocs(*p, flip)).collect(),
                closed: true,
            }
        }
        EntityType::Face3D(f) => Geom::Path {
            pts: vec![
                v3(f.first_corner),
                v3(f.second_corner),
                v3(f.third_corner),
                v3(f.fourth_corner),
            ],
            closed: true,
        },
        EntityType::Point(p) => Geom::Point { p: v3(p.location) },
        EntityType::Text(t) => Geom::Text {
            at: v3(t
                .alignment_point
                .filter(|_| {
                    t.horizontal_alignment != acadrust::entities::TextHorizontalAlignment::Left
                        || t.vertical_alignment
                            != acadrust::entities::TextVerticalAlignment::Baseline
                })
                .unwrap_or(t.insertion_point)),
            height: t.height,
            rotation: t.rotation,
            value: crate::plain_text(&t.value),
        },
        EntityType::MText(t) => Geom::Text {
            at: v3(t.insertion_point),
            height: t.height,
            rotation: t.rotation,
            value: crate::plain_text(&t.value),
        },
        EntityType::AttributeDefinition(a) => Geom::Text {
            at: v3(a.insertion_point),
            height: a.height,
            rotation: a.rotation,
            value: a.tag.clone(),
        },
        EntityType::Insert(i) => Geom::Insert {
            block: i.block_name.clone(),
            at: v3(i.insert_point),
            scale: [i.x_scale(), i.y_scale(), i.z_scale()],
            rotation: i.rotation,
            attributes: i
                .attributes
                .iter()
                .map(|a| (a.tag.clone(), a.value.clone()))
                .collect(),
        },
        EntityType::Dimension(d) => {
            let b = d.base();
            Geom::Dimension {
                block: b.block_name.clone(),
                measurement: measurement(d),
                text: b.user_text.clone().filter(|t| !t.is_empty() && t != "<>"),
                at: v3(b.text_middle_point),
                points: Vec::new(),
            }
        }
        EntityType::Hatch(h) => Geom::Hatch {
            loops: h
                .paths
                .iter()
                .map(hatch_loop)
                .filter(|l| l.len() >= 2)
                .collect(),
            pattern: h.pattern.name.clone(),
            solid: h.is_solid,
        },
        EntityType::Viewport(v) => Geom::Viewport {
            center: v3(v.center),
            width: v.width,
            height: v.height,
            view_center: [v.view_center.x, v.view_center.y],
            view_height: v.view_height,
            id: v.id,
        },
        _ => {
            let b = catch_unwind(AssertUnwindSafe(|| ent.bounding_box())).ok();
            Geom::Other {
                bbox: b
                    .map(|b| [b.min.x, b.min.y, b.min.z, b.max.x, b.max.y, b.max.z])
                    .filter(|b| b.iter().all(|v| v.is_finite()) && b[3] >= b[0] && b[4] >= b[1]),
            }
        }
    }
}

/// A hatch boundary path as (x, y, bulge) vertices: polyline edges directly, line edges as
/// points, arc edges as bulges, ellipse and spline edges as their end points.
/// What a dimension measures, in drawing units. A linear, aligned or radial dimension is measured
/// from its definition points: the stored measurement can carry the dimension style's linear
/// scale factor (DIMLFAC), which Revit's DWG export applies to it (914400 stored for a 3000 mm
/// distance, under a style whose factor is 304.8), so it is used only when the points give none.
fn measurement(d: &Dimension) -> f64 {
    let stored = d.base().actual_measurement;
    let measured = match d {
        Dimension::Angular2Ln(_) | Dimension::Angular3Pt(_) => return stored.to_degrees(),
        Dimension::Linear(l) => l.measurement(),
        Dimension::Aligned(a) => a.measurement(),
        Dimension::Radius(r) => r.measurement(),
        _ => return stored,
    };
    if measured.is_finite() && measured > 0.0 {
        measured
    } else {
        stored
    }
}

fn hatch_loop(path: &acadrust::entities::BoundaryPath) -> Vec<P3> {
    use acadrust::entities::BoundaryEdge;
    let mut pts: Vec<P3> = Vec::new();
    for edge in &path.edges {
        match edge {
            BoundaryEdge::Polyline(p) => pts.extend(p.vertices.iter().map(|v| [v.x, v.y, v.z])),
            BoundaryEdge::Line(l) => {
                pts.push([l.start.x, l.start.y, 0.0]);
                pts.push([l.end.x, l.end.y, 0.0]);
            }
            BoundaryEdge::CircularArc(a) => {
                let sweep = if a.counter_clockwise {
                    crate::ccw_sweep(a.start_angle, a.end_angle)
                } else {
                    -crate::ccw_sweep(-a.end_angle, -a.start_angle)
                };
                let (s, e) = if a.counter_clockwise {
                    (a.start_angle, a.end_angle)
                } else {
                    (-a.start_angle, -a.end_angle)
                };
                let p0 = [
                    a.center.x + a.radius * s.cos(),
                    a.center.y + a.radius * s.sin(),
                ];
                let p1 = [
                    a.center.x + a.radius * e.cos(),
                    a.center.y + a.radius * e.sin(),
                ];
                pts.push([p0[0], p0[1], (sweep / 4.0).tan()]);
                pts.push([p1[0], p1[1], 0.0]);
            }
            BoundaryEdge::EllipticArc(el) => {
                let len = el.major_axis_endpoint.x.hypot(el.major_axis_endpoint.y);
                let rot = el.major_axis_endpoint.y.atan2(el.major_axis_endpoint.x);
                let sweep = crate::ccw_sweep(el.start_angle, el.end_angle);
                let n = 16;
                for i in 0..=n {
                    let t = el.start_angle + sweep * i as f64 / n as f64;
                    let (x, y) = (len * t.cos(), len * el.minor_axis_ratio * t.sin());
                    let (sr, cr) = rot.sin_cos();
                    pts.push([
                        el.center.x + x * cr - y * sr,
                        el.center.y + x * sr + y * cr,
                        0.0,
                    ]);
                }
            }
            BoundaryEdge::Spline(s) => {
                if s.fit_points.len() >= 2 {
                    pts.extend(s.fit_points.iter().map(|p| [p.x, p.y, 0.0]));
                } else {
                    pts.extend(s.control_points.iter().map(|p| [p.x, p.y, 0.0]));
                }
            }
        }
    }
    // Consecutive edges share end points: drop repeats, keeping the first one's bulge.
    pts.dedup_by(|later, earlier| {
        let same = (later[0] - earlier[0]).abs() < 1e-9 && (later[1] - earlier[1]).abs() < 1e-9;
        if same && later[2] != 0.0 {
            earlier[2] = later[2];
        }
        same
    });
    if pts.len() > 2 {
        let (f, l) = (pts[0], pts[pts.len() - 1]);
        if (f[0] - l[0]).abs() < 1e-9 && (f[1] - l[1]).abs() < 1e-9 {
            pts.pop();
        }
    }
    pts
}
