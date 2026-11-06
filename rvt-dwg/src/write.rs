//! Writing DXF and DWG.
//!
//! [`write_drawing`] builds a new file from a [`Drawing2d`] with standard tables and entities
//! only, so it writes to every DWG version from 2000 to 2018. [`resave`] writes a file that was
//! read to another format or version through the engine's own document, which keeps everything
//! the engine read; when the engine cannot write some of it to the version asked for (custom
//! objects, named colours before 2004), it falls back to the drawing model and says what that
//! dropped.

use crate::read::panic_message;
use crate::{Drawing2d, Entity2d, Error, Geom, Read, Space};
use acadrust::entities::{
    Arc, AttributeEntity, BoundaryEdge, BoundaryPath, BoundaryPathFlags, Circle, Ellipse,
    EntityType, Hatch, HatchPattern, Insert, Line, LwPolyline, LwVertex, MText, Point,
    PolylineEdge, Text, Viewport,
};
use acadrust::objects::ObjectType;
use acadrust::tables::{BlockRecord, Layer};
use acadrust::{
    CadDocument, Color, DwgWriter, DxfVersion, DxfWriter, Handle, LineWeight, Vector2, Vector3,
};
use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};

/// Output file format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CadFormat {
    Dxf,
    DxfBinary,
    Dwg,
}

/// DWG versions the writer produces, newest first, as (name, file version).
pub const DWG_VERSIONS: &[(&str, &str)] = &[
    ("2018", "AC1032"),
    ("2013", "AC1027"),
    ("2010", "AC1024"),
    ("2007", "AC1021"),
    ("2004", "AC1018"),
    ("2000", "AC1015"),
];

/// Parse a version given as a release year (`2018`), a file version (`AC1032`) or `R12`/`R14`
/// (DXF only).
pub fn parse_version(s: &str, format: CadFormat) -> Result<DxfVersion, Error> {
    let up = s.trim().to_ascii_uppercase();
    let v = DWG_VERSIONS
        .iter()
        .find(|(year, file)| *year == up || *file == up)
        .map(|(_, file)| DxfVersion::from_version_string(file))
        .or(match up.as_str() {
            "R12" | "AC1009" => Some(DxfVersion::AC1009),
            "R14" | "AC1014" => Some(DxfVersion::AC1014),
            _ => None,
        })
        .ok_or_else(|| {
            Error(format!(
                "unknown version {s:?}: use 2018, 2013, 2010, 2007, 2004 or 2000"
            ))
        })?;
    if format == CadFormat::Dwg && v.version_code() < 1015 {
        return Err(Error(format!(
            "DWG is written in versions 2000 to 2018, not {s}"
        )));
    }
    Ok(v)
}

fn encode(doc: &CadDocument, format: CadFormat) -> Result<Vec<u8>, Error> {
    let r = catch_unwind(AssertUnwindSafe(|| match format {
        CadFormat::Dxf => DxfWriter::new(doc).write_to_vec(),
        CadFormat::DxfBinary => DxfWriter::new_binary(doc).write_to_vec(),
        CadFormat::Dwg => DwgWriter::write_to_vec(doc),
    }));
    match r {
        Ok(Ok(b)) => Ok(b),
        Ok(Err(e)) => Err(Error(e.to_string())),
        Err(p) => Err(Error(format!("the writer stopped: {}", panic_message(&p)))),
    }
}

/// Write `read`'s file as `format` in `version`. Returns the bytes and notes on anything dropped.
pub fn resave(
    read: &Read,
    format: CadFormat,
    version: DxfVersion,
) -> Result<(Vec<u8>, Vec<String>), Error> {
    let mut doc = read.source.0.clone();
    doc.version = version;
    match encode(&doc, format) {
        Ok(b) => Ok((b, Vec::new())),
        Err(e) => {
            let (b, mut notes) = write_drawing(&read.drawing, format, version)?;
            notes.insert(0, format!("written from the drawing model: {e}; objects outside it (custom objects, styles, dictionaries) were left out"));
            Ok((b, notes))
        }
    }
}

/// Write `d` as a new DXF or DWG file. Returns the bytes and notes on anything not written.
pub fn write_drawing(
    d: &Drawing2d,
    format: CadFormat,
    version: DxfVersion,
) -> Result<(Vec<u8>, Vec<String>), Error> {
    let (doc, notes) = catch_unwind(AssertUnwindSafe(|| build(d, version))).map_err(|p| {
        Error(format!(
            "building the drawing failed: {}",
            panic_message(&p)
        ))
    })??;
    Ok((encode(&doc, format)?, notes))
}

fn color(c: &crate::Colour) -> Color {
    match (c.rgb, c.aci) {
        (Some([r, g, b]), _) => Color::Rgb { r, g, b },
        (None, Some(i)) => Color::from_index(i),
        (None, None) => Color::ByLayer,
    }
}

fn lineweight(w: i16) -> LineWeight {
    match w {
        -1 => LineWeight::ByLayer,
        -2 => LineWeight::ByBlock,
        -3 => LineWeight::Default,
        v => LineWeight::from_value(v),
    }
}

fn v3(p: [f64; 3]) -> Vector3 {
    Vector3::new(p[0], p[1], p[2])
}

/// A block name the file keeps: anonymous names (`*D12`, `*U3`) become ordinary ones (`_D12`), as
/// their entities are written plainly.
fn block_name(name: &str) -> String {
    match name.strip_prefix('*') {
        Some(rest) => format!("_{rest}"),
        None => name.to_string(),
    }
}

fn build(d: &Drawing2d, version: DxfVersion) -> Result<(CadDocument, Vec<String>), Error> {
    let mut doc = CadDocument::with_version(version);
    doc.header.insertion_units = d.units;
    let mut notes = Vec::new();

    for l in &d.layers {
        let mut layer = doc
            .layers
            .get(&l.name)
            .cloned()
            .unwrap_or_else(|| Layer::new(l.name.clone()));
        layer.color = color(&l.colour);
        if l.colour.is_by_layer() || l.colour.is_by_block() {
            layer.color = Color::Index(7);
        }
        if doc.line_types.get(&l.linetype).is_some() {
            layer.line_type = l.linetype.clone();
        }
        layer.line_weight = lineweight(l.lineweight);
        layer.flags.off = !l.on;
        layer.flags.frozen = l.frozen;
        layer.flags.locked = l.locked;
        layer.is_plottable = l.plot;
        doc.layers.add_or_replace(layer);
    }
    // Entities on layers the drawing does not list get them made.
    for e in &d.entities {
        if !e.layer.is_empty() && doc.layers.get(&e.layer).is_none() {
            doc.layers.add_or_replace(Layer::new(e.layer.clone()));
        }
    }

    // Owner block -> handle of the record it is written into.
    let mut owners: HashMap<String, Handle> = HashMap::new();
    if let Some(ms) = doc.block_records.get(crate::MODEL_SPACE) {
        owners.insert(crate::MODEL_SPACE.to_string(), ms.handle);
    }
    for layout in d.layouts.iter().filter(|l| !l.is_model) {
        let exists = doc
            .objects
            .values()
            .any(|o| matches!(o, ObjectType::Layout(x) if x.name == layout.name));
        if !exists {
            doc.add_layout(&layout.name)
                .map_err(|e| Error(format!("layout {}: {e}", layout.name)))?;
        }
        let record = doc.objects.values().find_map(|o| match o {
            ObjectType::Layout(x) if x.name == layout.name => Some(x.block_record),
            _ => None,
        });
        if let Some(h) = record {
            owners.insert(layout.block.clone(), h);
        }
    }
    for b in d
        .blocks
        .iter()
        .filter(|b| Space::of(&b.name) == Space::Block && b.xref_path.is_none())
    {
        let mut record = BlockRecord::new(block_name(&b.name));
        record.handle = doc.allocate_handle();
        record.block_entity_handle = doc.allocate_handle();
        record.block_end_handle = doc.allocate_handle();
        record.base_point = v3(b.base);
        owners.insert(b.name.clone(), record.handle);
        doc.block_records.add_or_replace(record);
    }
    let xrefs = d.blocks.iter().filter(|b| b.xref_path.is_some()).count();
    if xrefs > 0 {
        notes.push(format!(
            "{xrefs} external references are not written (their inserts are left out)"
        ));
    }

    let mut skipped: std::collections::BTreeMap<&str, usize> = Default::default();
    let mut orphans: std::collections::BTreeMap<&str, usize> = Default::default();
    let mut viewport_id = 2;
    for e in &d.entities {
        let Some(&owner) = owners.get(&e.owner) else {
            *orphans.entry(e.owner.as_str()).or_default() += 1;
            continue;
        };
        if matches!(e.geom, Geom::Viewport { id: 1, .. }) {
            // A layout's own view: every written layout gets one.
            continue;
        }
        let Some(mut entity) = entity(e, &owners, &mut viewport_id) else {
            *skipped.entry(e.kind.as_str()).or_default() += 1;
            continue;
        };
        let common = entity.common_mut();
        common.layer = if e.layer.is_empty() {
            "0".into()
        } else {
            e.layer.clone()
        };
        common.color = color(&e.colour);
        common.line_weight = lineweight(e.lineweight);
        if !e.linetype.is_empty() && doc.line_types.get(&e.linetype).is_some() {
            common.linetype = e.linetype.clone();
        }
        common.owner_handle = owner;
        doc.add_entity(entity)
            .map_err(|err| Error(format!("entity {} ({}): {err}", e.handle, e.kind)))?;
    }
    for (owner, n) in orphans {
        let owner = if owner.is_empty() {
            "no owner block"
        } else {
            owner
        };
        notes.push(format!(
            "{n} entities of {owner:?} not written: that block is not written"
        ));
    }
    for (kind, n) in skipped {
        notes.push(format!(
            "{n} {kind} not written: no plain DXF/DWG equivalent in the drawing model"
        ));
    }
    Ok((doc, notes))
}

fn entity(
    e: &Entity2d,
    owners: &HashMap<String, Handle>,
    viewport_id: &mut i16,
) -> Option<EntityType> {
    Some(match &e.geom {
        Geom::Line { a, b } => EntityType::Line(Line::from_points(v3(*a), v3(*b))),
        Geom::Circle { c, r } => EntityType::Circle(Circle::from_center_radius(v3(*c), *r)),
        Geom::Arc { c, r, start, end } => {
            let mut arc = Arc::new();
            arc.center = v3(*c);
            arc.radius = *r;
            arc.start_angle = *start;
            arc.end_angle = *end;
            EntityType::Arc(arc)
        }
        Geom::Ellipse {
            c,
            major,
            ratio,
            start,
            end,
        } => {
            let mut el = Ellipse::from_center_axes(v3(*c), v3(*major), *ratio);
            el.start_parameter = *start;
            el.end_parameter = *end;
            EntityType::Ellipse(el)
        }
        Geom::Polyline { pts, closed, z } => EntityType::LwPolyline(lwpolyline(pts, *closed, *z)),
        // ponytail: 3D paths are written flat at their first point's height; write POLYLINE (3D) if a drawing needs varying Z.
        Geom::Path { pts, closed } => {
            let flat: Vec<[f64; 3]> = pts.iter().map(|p| [p[0], p[1], 0.0]).collect();
            EntityType::LwPolyline(lwpolyline(
                &flat,
                *closed,
                pts.first().map_or(0.0, |p| p[2]),
            ))
        }
        Geom::Point { p } => EntityType::Point(Point::from_coords(p[0], p[1], p[2])),
        Geom::Text {
            at,
            height,
            rotation,
            value,
        } => {
            if value.contains('\n') {
                let mut t = MText::new();
                t.value = value.replace('\n', "\\P");
                t.insertion_point = v3(*at);
                t.height = *height;
                t.rotation = *rotation;
                EntityType::MText(Box::new(t))
            } else {
                let mut t = Text::with_value(value.clone(), v3(*at));
                t.height = *height;
                t.rotation = *rotation;
                EntityType::Text(t)
            }
        }
        Geom::Insert {
            block,
            at,
            scale,
            rotation,
            attributes,
        } => {
            owners.get(block)?;
            let mut i = Insert::new(block_name(block), v3(*at)).with_scale(
                nonzero(scale[0]),
                nonzero(scale[1]),
                nonzero(scale[2]),
            );
            i.rotation = *rotation;
            i.attributes = attributes
                .iter()
                .map(|(t, v)| AttributeEntity::new(t.clone(), v.clone()))
                .collect();
            EntityType::Insert(i)
        }
        // A dimension is written as what it draws: an insert of its block.
        Geom::Dimension { block, .. } => {
            owners.get(block)?;
            EntityType::Insert(Insert::new(block_name(block), Vector3::new(0.0, 0.0, 0.0)))
        }
        Geom::Hatch {
            loops,
            pattern,
            solid,
        } => {
            let mut h = if *solid {
                Hatch::solid()
            } else {
                Hatch::with_pattern(HatchPattern::new(pattern.clone()))
            };
            for l in loops {
                let mut path = BoundaryPath::new();
                path.flags = BoundaryPathFlags::POLYLINE;
                path.edges.push(BoundaryEdge::Polyline(PolylineEdge {
                    vertices: l.iter().map(|p| Vector3::new(p[0], p[1], p[2])).collect(),
                    is_closed: true,
                }));
                h.add_path(path);
            }
            EntityType::Hatch(Box::new(h))
        }
        Geom::Viewport {
            center,
            width,
            height,
            view_center,
            view_height,
            id,
        } => {
            if *id == 1 {
                return None;
            }
            let mut v = Viewport::new();
            v.center = v3(*center);
            v.width = *width;
            v.height = *height;
            v.view_center = Vector3::new(view_center[0], view_center[1], 0.0);
            v.view_height = *view_height;
            v.id = *viewport_id;
            *viewport_id += 1;
            EntityType::Viewport(Box::new(v))
        }
        Geom::Other { .. } => return None,
    })
}

fn nonzero(s: f64) -> f64 {
    if s.abs() < 1e-12 { 1.0 } else { s }
}

fn lwpolyline(pts: &[[f64; 3]], closed: bool, z: f64) -> LwPolyline {
    let mut p = LwPolyline::new();
    for v in pts {
        let mut vertex = LwVertex::new(Vector2::new(v[0], v[1]));
        vertex.bulge = v[2];
        p.add_vertex(vertex);
    }
    p.is_closed = closed;
    p.elevation = z;
    p
}
