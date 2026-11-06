//! The Entities, Summary, Layers and Blocks tables of a drawing, and their CSV and JSONL forms.
//!
//! [`sheets`] builds the four tables once; CSV ([`write_csv`]) and JSONL ([`entities_jsonl`])
//! write them, and a workbook writer takes the same [`Sheet`]s, one worksheet each, in order.
//! Columns are documented in `docs/dwg.md`.

use crate::{Affine, Drawing2d, Entity2d, Geom, Limits, Space, bounds};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap};

/// A table cell.
#[derive(Debug, Clone, PartialEq)]
pub enum Cell {
    Empty,
    Text(String),
    Number(f64),
    Integer(i64),
    Bool(bool),
}

impl Cell {
    fn text(s: impl Into<String>) -> Cell {
        let s = s.into();
        if s.is_empty() {
            Cell::Empty
        } else {
            Cell::Text(s)
        }
    }

    fn num(v: Option<f64>) -> Cell {
        match v {
            Some(v) if v.is_finite() => Cell::Number(v),
            _ => Cell::Empty,
        }
    }

    fn to_json(&self) -> Value {
        match self {
            Cell::Empty => Value::Null,
            Cell::Text(s) => Value::String(s.clone()),
            Cell::Number(v) => serde_json::Number::from_f64(*v).map_or(Value::Null, Value::Number),
            Cell::Integer(v) => Value::from(*v),
            Cell::Bool(b) => Value::Bool(*b),
        }
    }

    fn to_csv(&self) -> String {
        match self {
            Cell::Empty => String::new(),
            Cell::Text(s) => {
                // A leading = + - @ would be read as a formula by spreadsheet programs.
                let s = if s.starts_with(['=', '+', '-', '@']) && s.parse::<f64>().is_err() {
                    format!("'{s}")
                } else {
                    s.clone()
                };
                if s.contains([',', '"', '\n', '\r']) {
                    format!("\"{}\"", s.replace('"', "\"\""))
                } else {
                    s
                }
            }
            Cell::Number(v) => format!("{}", round(*v)),
            Cell::Integer(v) => v.to_string(),
            Cell::Bool(b) => b.to_string(),
        }
    }
}

/// Round to 9 decimals so outputs do not carry float noise.
fn round(v: f64) -> f64 {
    let r = (v * 1e9).round() / 1e9;
    if r == 0.0 { 0.0 } else { r }
}

/// One table: a name, its column names and its rows.
#[derive(Debug, Clone, PartialEq)]
pub struct Sheet {
    pub name: &'static str,
    pub columns: Vec<&'static str>,
    pub rows: Vec<Vec<Cell>>,
}

/// Columns of the Entities table, in order; also the JSONL keys.
pub const ENTITY_COLUMNS: &[&str] = &[
    "handle",
    "type",
    "space",
    "layout",
    "block",
    "insert_path",
    "layer",
    "colour_aci",
    "colour_rgb",
    "linetype",
    "lineweight",
    "transparency",
    "start_x",
    "start_y",
    "start_z",
    "end_x",
    "end_y",
    "end_z",
    "centre_x",
    "centre_y",
    "radius",
    "length",
    "area",
    "min_x",
    "min_y",
    "max_x",
    "max_y",
    "text",
    "insert_block",
    "insert_x",
    "insert_y",
    "scale_x",
    "scale_y",
    "rotation_deg",
    "attributes",
    "measurement",
    "hatch_pattern",
    "xdata_apps",
];

/// Options for the tables.
#[derive(Debug, Clone, Copy, Default)]
pub struct TableOptions {
    /// Add a row for every entity drawn by every insert (nested too), placed where it is drawn,
    /// with `insert_path` naming the inserts it came through.
    pub explode_blocks: bool,
    pub limits: Limits,
}

/// The four tables: Entities, Summary, Layers, Blocks.
pub fn sheets(d: &Drawing2d, options: &TableOptions) -> Vec<Sheet> {
    let ctx = Context::new(d);
    let mut rows = Vec::with_capacity(d.entities.len());
    for e in &d.entities {
        rows.push(ctx.entity_row(e, None, ""));
    }
    if options.explode_blocks {
        let mut budget = options.limits.max_expanded;
        for e in d
            .entities
            .iter()
            .filter(|e| Space::of(&e.owner) != Space::Block)
        {
            ctx.explode(
                e,
                &Affine::IDENTITY,
                String::new(),
                0,
                &options.limits,
                &mut budget,
                &mut rows,
            );
        }
    }
    vec![
        Sheet {
            name: "Entities",
            columns: ENTITY_COLUMNS.to_vec(),
            rows,
        },
        summary(d, &ctx),
        layers(d),
        blocks(d, &ctx),
    ]
}

struct Context<'a> {
    layer_colour: HashMap<&'a str, crate::Colour>,
    layout_of: HashMap<&'a str, &'a str>,
    by_owner: HashMap<&'a str, Vec<&'a Entity2d>>,
    base: HashMap<&'a str, [f64; 3]>,
}

impl<'a> Context<'a> {
    fn new(d: &'a Drawing2d) -> Self {
        let mut by_owner: HashMap<&str, Vec<&Entity2d>> = HashMap::new();
        for e in &d.entities {
            by_owner.entry(e.owner.as_str()).or_default().push(e);
        }
        Self {
            layer_colour: d
                .layers
                .iter()
                .map(|l| (l.name.as_str(), l.colour))
                .collect(),
            layout_of: d
                .layouts
                .iter()
                .map(|l| (l.block.as_str(), l.name.as_str()))
                .collect(),
            by_owner,
            base: d.blocks.iter().map(|b| (b.name.as_str(), b.base)).collect(),
        }
    }

    fn entity_row(&self, e: &Entity2d, placed: Option<&Geom>, path: &str) -> Vec<Cell> {
        let g = placed.unwrap_or(&e.geom);
        let space = Space::of(&e.owner);
        let layout = self.layout_of.get(e.owner.as_str()).copied().unwrap_or("");
        let rgb = e.colour.to_rgb().or_else(|| {
            if e.colour.is_by_layer() {
                self.layer_colour
                    .get(e.layer.as_str())
                    .and_then(|c| c.to_rgb())
            } else {
                None
            }
        });
        let (start, end) = ends(g);
        let (centre, radius) = match g {
            Geom::Circle { c, r } | Geom::Arc { c, r, .. } => (Some(*c), Some(*r)),
            Geom::Ellipse { c, major, .. } => (Some(*c), Some(major[0].hypot(major[1]))),
            _ => (None, None),
        };
        let b = g.bbox();
        let text = match g {
            Geom::Text { value, .. } => value.clone(),
            Geom::Dimension { text: Some(t), .. } => crate::plain_text(t),
            _ => String::new(),
        };
        let (insert_block, insert_at, scale, rotation, attributes) = match g {
            Geom::Insert {
                block,
                at,
                scale,
                rotation,
                attributes,
            } => (
                block.as_str(),
                Some(*at),
                Some(*scale),
                Some(rotation.to_degrees()),
                attributes
                    .iter()
                    .map(|(t, v)| format!("{t}={v}"))
                    .collect::<Vec<_>>()
                    .join(";"),
            ),
            Geom::Text { rotation, .. } => {
                ("", None, None, Some(rotation.to_degrees()), String::new())
            }
            _ => ("", None, None, None, String::new()),
        };
        vec![
            Cell::text(e.handle.clone()),
            Cell::text(e.kind.clone()),
            Cell::text(space.as_str()),
            Cell::text(layout),
            Cell::text(if space == Space::Block {
                e.owner.as_str()
            } else {
                ""
            }),
            Cell::text(path),
            Cell::text(e.layer.clone()),
            e.colour
                .aci
                .map_or(Cell::Empty, |a| Cell::Integer(a as i64)),
            rgb.map_or(Cell::Empty, |[r, g, b]| {
                Cell::Text(format!("#{r:02X}{g:02X}{b:02X}"))
            }),
            Cell::text(if e.linetype.is_empty() {
                "ByLayer"
            } else {
                e.linetype.as_str()
            }),
            Cell::Integer(e.lineweight as i64),
            Cell::num(e.transparency),
            Cell::num(start.map(|p| p[0])),
            Cell::num(start.map(|p| p[1])),
            Cell::num(start.map(|p| p[2])),
            Cell::num(end.map(|p| p[0])),
            Cell::num(end.map(|p| p[1])),
            Cell::num(end.map(|p| p[2])),
            Cell::num(centre.map(|p| p[0])),
            Cell::num(centre.map(|p| p[1])),
            Cell::num(radius),
            Cell::num(g.length()),
            Cell::num(g.area()),
            Cell::num(b.map(|b| b[0])),
            Cell::num(b.map(|b| b[1])),
            Cell::num(b.map(|b| b[2])),
            Cell::num(b.map(|b| b[3])),
            Cell::text(text),
            Cell::text(insert_block),
            Cell::num(insert_at.map(|p| p[0])),
            Cell::num(insert_at.map(|p| p[1])),
            Cell::num(scale.map(|s| s[0])),
            Cell::num(scale.map(|s| s[1])),
            Cell::num(rotation),
            Cell::text(attributes),
            Cell::num(match g {
                Geom::Dimension { measurement, .. } => Some(*measurement),
                _ => None,
            }),
            Cell::text(match g {
                Geom::Hatch { solid: true, .. } => "SOLID".to_string(),
                Geom::Hatch { pattern, .. } => pattern.clone(),
                _ => String::new(),
            }),
            Cell::text(e.xdata_apps.join(";")),
        ]
    }

    /// Rows for what `e` draws through inserts (and dimension blocks), placed by `t`.
    #[allow(clippy::too_many_arguments)]
    fn explode(
        &self,
        e: &Entity2d,
        t: &Affine,
        path: String,
        depth: usize,
        limits: &Limits,
        budget: &mut usize,
        rows: &mut Vec<Vec<Cell>>,
    ) {
        let (block, inner) = match &e.geom {
            Geom::Insert {
                block,
                at,
                scale,
                rotation,
                ..
            } => {
                let base = self.base.get(block.as_str()).copied().unwrap_or([0.0; 3]);
                (block, t.then(&Affine::insert(base, *at, *scale, *rotation)))
            }
            Geom::Dimension { block, .. } => (block, *t),
            _ => return,
        };
        if depth >= limits.max_insert_depth {
            return;
        }
        let path = if path.is_empty() {
            format!("{}:{}", e.handle, block)
        } else {
            format!("{path}>{}:{}", e.handle, block)
        };
        for child in self.by_owner.get(block.as_str()).into_iter().flatten() {
            if *budget == 0 {
                return;
            }
            *budget -= 1;
            let placed = child.geom.transformed(&inner);
            rows.push(self.entity_row(child, Some(&placed), &path));
            self.explode(child, &inner, path.clone(), depth + 1, limits, budget, rows);
        }
    }
}

fn ends(g: &Geom) -> (Option<[f64; 3]>, Option<[f64; 3]>) {
    match g {
        Geom::Line { a, b } => (Some(*a), Some(*b)),
        Geom::Arc { c, r, start, end } => {
            let p = |a: f64| [c[0] + r * a.cos(), c[1] + r * a.sin(), c[2]];
            (Some(p(*start)), Some(p(*end)))
        }
        Geom::Polyline {
            pts,
            closed: false,
            z,
        } if !pts.is_empty() => {
            let (f, l) = (pts[0], pts[pts.len() - 1]);
            (Some([f[0], f[1], *z]), Some([l[0], l[1], *z]))
        }
        Geom::Path { pts, closed: false } if !pts.is_empty() => {
            (Some(pts[0]), Some(pts[pts.len() - 1]))
        }
        Geom::Point { p } => (Some(*p), None),
        Geom::Text { at, .. } | Geom::Dimension { at, .. } => (Some(*at), None),
        _ => (None, None),
    }
}

fn summary(d: &Drawing2d, ctx: &Context) -> Sheet {
    let mut rows = Vec::new();
    let extents = bounds(
        d.entities
            .iter()
            .filter(|e| Space::of(&e.owner) == Space::Model)
            .filter_map(|e| e.geom.bbox())
            .flat_map(|b| [[b[0], b[1]], [b[2], b[3]]]),
    );
    let meta = |item: &str, value: Cell| {
        vec![
            Cell::text(item),
            Cell::Empty,
            Cell::Empty,
            Cell::Empty,
            Cell::Empty,
            value,
        ]
    };
    rows.push(meta("version", Cell::text(d.version.clone())));
    rows.push(meta("units", Cell::Integer(d.units as i64)));
    rows.push(meta("code_page", Cell::text(d.code_page.clone())));
    rows.push(meta("entities", Cell::Integer(d.entities.len() as i64)));
    rows.push(meta("layers", Cell::Integer(d.layers.len() as i64)));
    rows.push(meta(
        "blocks",
        Cell::Integer(d.blocks.iter().filter(|b| b.layout.is_none()).count() as i64),
    ));
    rows.push(meta("layouts", Cell::Integer(d.layouts.len() as i64)));
    for (i, k) in ["model_min_x", "model_min_y", "model_max_x", "model_max_y"]
        .iter()
        .enumerate()
    {
        rows.push(meta(k, Cell::num(extents.map(|b| b[i]))));
    }
    let mut counts: BTreeMap<(&str, &str, &str), i64> = BTreeMap::new();
    for e in &d.entities {
        let space = match Space::of(&e.owner) {
            Space::Block => "block",
            Space::Model => "model",
            Space::Paper => ctx
                .layout_of
                .get(e.owner.as_str())
                .copied()
                .unwrap_or("paper"),
        };
        *counts
            .entry((e.kind.as_str(), e.layer.as_str(), space))
            .or_default() += 1;
    }
    for ((kind, layer, space), n) in counts {
        rows.push(vec![
            Cell::text("count"),
            Cell::text(kind),
            Cell::text(layer),
            Cell::text(space),
            Cell::Integer(n),
            Cell::Empty,
        ]);
    }
    Sheet {
        name: "Summary",
        columns: vec!["item", "type", "layer", "space", "count", "value"],
        rows,
    }
}

fn layers(d: &Drawing2d) -> Sheet {
    let mut used: HashMap<&str, i64> = HashMap::new();
    for e in &d.entities {
        *used.entry(e.layer.as_str()).or_default() += 1;
    }
    let rows = d
        .layers
        .iter()
        .map(|l| {
            vec![
                Cell::text(l.name.clone()),
                Cell::Bool(l.on),
                Cell::Bool(l.frozen),
                Cell::Bool(l.locked),
                Cell::Bool(l.plot),
                l.colour
                    .aci
                    .map_or(Cell::Empty, |a| Cell::Integer(a as i64)),
                l.colour.to_rgb().map_or(Cell::Empty, |[r, g, b]| {
                    Cell::Text(format!("#{r:02X}{g:02X}{b:02X}"))
                }),
                Cell::text(l.linetype.clone()),
                Cell::Integer(l.lineweight as i64),
                Cell::Integer(used.get(l.name.as_str()).copied().unwrap_or(0)),
            ]
        })
        .collect();
    Sheet {
        name: "Layers",
        columns: vec![
            "name",
            "on",
            "frozen",
            "locked",
            "plot",
            "colour_aci",
            "colour_rgb",
            "linetype",
            "lineweight",
            "entity_count",
        ],
        rows,
    }
}

fn blocks(d: &Drawing2d, ctx: &Context) -> Sheet {
    let mut inserts: HashMap<&str, i64> = HashMap::new();
    for e in &d.entities {
        if let Geom::Insert { block, .. } = &e.geom {
            *inserts.entry(block.as_str()).or_default() += 1;
        }
    }
    let rows = d
        .blocks
        .iter()
        .map(|b| {
            vec![
                Cell::text(b.name.clone()),
                Cell::Number(round(b.base[0])),
                Cell::Number(round(b.base[1])),
                Cell::Number(round(b.base[2])),
                Cell::Integer(
                    ctx.by_owner
                        .get(b.name.as_str())
                        .map_or(0, |v| v.len() as i64),
                ),
                Cell::Integer(inserts.get(b.name.as_str()).copied().unwrap_or(0)),
                Cell::Bool(b.xref_path.is_some()),
                Cell::text(b.xref_path.clone().unwrap_or_default()),
                Cell::text(b.layout.clone().unwrap_or_default()),
                Cell::text(b.attribute_tags.join(";")),
            ]
        })
        .collect();
    Sheet {
        name: "Blocks",
        columns: vec![
            "name",
            "base_x",
            "base_y",
            "base_z",
            "entity_count",
            "insert_count",
            "is_xref",
            "xref_path",
            "layout",
            "attribute_tags",
        ],
        rows,
    }
}

/// A sheet as CSV (comma separated, header row, CRLF line ends as RFC 4180).
pub fn write_csv(sheet: &Sheet) -> String {
    let mut out = String::new();
    out.push_str(&sheet.columns.join(","));
    out.push_str("\r\n");
    for row in &sheet.rows {
        out.push_str(&row.iter().map(Cell::to_csv).collect::<Vec<_>>().join(","));
        out.push_str("\r\n");
    }
    out
}

/// A sheet as JSON Lines: one object per row, keyed by the column names. Empty cells are left
/// out.
pub fn write_jsonl(sheet: &Sheet) -> String {
    let mut out = String::new();
    for row in &sheet.rows {
        let mut m = Map::new();
        for (k, c) in sheet.columns.iter().zip(row) {
            if *c != Cell::Empty {
                m.insert((*k).to_string(), c.to_json());
            }
        }
        out.push_str(&Value::Object(m).to_string());
        out.push('\n');
    }
    out
}

/// The Entities table as JSON Lines.
pub fn entities_jsonl(sheets: &[Sheet]) -> String {
    sheets
        .iter()
        .find(|s| s.name == "Entities")
        .map(write_jsonl)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Block2d, Layer2d, Layout2d};

    fn drawing() -> Drawing2d {
        let mut door = Entity2d::new(
            "LINE",
            "A-DOOR",
            Geom::Line {
                a: [0.0, 0.0, 0.0],
                b: [1.0, 0.0, 0.0],
            },
        );
        door.owner = "DOOR".into();
        let mut ins = Entity2d::new(
            "INSERT",
            "A-DOOR",
            Geom::Insert {
                block: "DOOR".into(),
                at: [10.0, 5.0, 0.0],
                scale: [2.0, 2.0, 1.0],
                rotation: 0.0,
                attributes: vec![("MARK".into(), "D1".into())],
            },
        );
        ins.handle = "2A".into();
        Drawing2d {
            version: "AC1032".into(),
            units: 4,
            code_page: "ANSI_1252".into(),
            layers: vec![Layer2d::new("A-DOOR", 2)],
            blocks: vec![Block2d {
                name: "DOOR".into(),
                base: [0.0; 3],
                xref_path: None,
                layout: None,
                attribute_tags: vec!["MARK".into()],
            }],
            layouts: vec![Layout2d {
                name: "Model".into(),
                block: crate::MODEL_SPACE.into(),
                tab_order: 0,
                is_model: true,
                limits: None,
            }],
            entities: vec![door, ins],
        }
    }

    #[test]
    fn four_sheets_and_exploded_rows_are_placed() {
        let s = sheets(
            &drawing(),
            &TableOptions {
                explode_blocks: true,
                limits: Limits::DEFAULT,
            },
        );
        assert_eq!(
            s.iter().map(|s| s.name).collect::<Vec<_>>(),
            ["Entities", "Summary", "Layers", "Blocks"]
        );
        let ents = &s[0];
        assert_eq!(ents.rows.len(), 3, "two entities and one exploded line");
        let col = |name| ENTITY_COLUMNS.iter().position(|c| *c == name).unwrap();
        let exploded = &ents.rows[2];
        assert_eq!(exploded[col("insert_path")], Cell::Text("2A:DOOR".into()));
        assert_eq!(exploded[col("end_x")], Cell::Number(12.0));
        assert_eq!(
            exploded[col("colour_rgb")],
            Cell::Text("#FFFF00".into()),
            "ByLayer resolves to the layer's yellow"
        );
        assert_eq!(
            ents.rows[1][col("attributes")],
            Cell::Text("MARK=D1".into())
        );
        let blocks = &s[3];
        assert_eq!(blocks.rows[0][5], Cell::Integer(1), "one insert of DOOR");
    }

    #[test]
    fn csv_quotes_and_defuses_formulas() {
        let sheet = Sheet {
            name: "T",
            columns: vec!["a", "b"],
            rows: vec![vec![Cell::text("=SUM(A1)"), Cell::text("x,\"y\"")]],
        };
        assert_eq!(write_csv(&sheet), "a,b\r\n'=SUM(A1),\"x,\"\"y\"\"\"\r\n");
        let j = write_jsonl(&sheet);
        assert_eq!(j, "{\"a\":\"=SUM(A1)\",\"b\":\"x,\\\"y\\\"\"}\n");
    }
}
