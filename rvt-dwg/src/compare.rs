//! Scoring one drawing against another: a reference (a DWG exported by the authoring tool) and a
//! candidate (what rvt-open wrote for the same sheet or view).
//!
//! What is scored, all in the reference's units (the candidate is scaled by the two files'
//! `$INSUNITS` when both are set):
//! - **Sheets** (paper layouts): names matched; per matched sheet, viewport count, each viewport's
//!   centre distance and scale, title-block insert names and attribute values, sheet text.
//! - **Linework**, per layer and pooled: both drawings are sampled every `step` units with inserts
//!   and dimensions expanded; precision is the share of candidate samples within `tolerance` of a
//!   reference sample, recall the reverse, F1 their harmonic mean.
//! - **Text**: strings matched as multisets, and how many matched strings sit within
//!   `tolerance` of their reference position.
//! - **Hatches** (count and pattern names per layer) and **dimension** values (to 1e-3).

use crate::{Affine, Drawing2d, Entity2d, Geom, Limits, Space};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};

/// What to compare and how closely.
#[derive(Debug, Clone)]
pub struct Options {
    /// Distance within which a sample counts as matched, in reference units.
    pub tolerance: f64,
    /// Sample spacing along curves; defaults to half the tolerance.
    pub step: Option<f64>,
    /// Reference layer name -> candidate layer name, for layers named differently.
    pub layer_map: HashMap<String, String>,
    /// Compare this layout's paper space instead of model space.
    pub layout: Option<String>,
    pub limits: Limits,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            tolerance: 0.5,
            step: None,
            layer_map: HashMap::new(),
            layout: None,
            limits: Limits::DEFAULT,
        }
    }
}

/// Precision, recall and F1 of one sample set against another.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Match {
    pub reference: usize,
    pub candidate: usize,
    pub precision: f64,
    pub recall: f64,
    pub f1: f64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct LayerScore {
    /// Reference layer name (the candidate's, through the layer map).
    pub layer: String,
    pub linework: Match,
    pub hatches_reference: usize,
    pub hatches_candidate: usize,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ViewportScore {
    pub centre_distance: f64,
    pub scale_reference: f64,
    pub scale_candidate: f64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct SheetScore {
    pub name: String,
    pub viewports_reference: usize,
    pub viewports_candidate: usize,
    /// Viewports paired by nearest centre.
    pub viewports: Vec<ViewportScore>,
    pub title_blocks_reference: Vec<String>,
    pub title_blocks_candidate: Vec<String>,
    /// Attribute values and text on the sheet, matched as strings.
    pub text: Match,
}

/// The score table.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Score {
    pub units_scale: f64,
    pub tolerance: f64,
    pub sheets_reference: Vec<String>,
    pub sheets_candidate: Vec<String>,
    pub sheets: Vec<SheetScore>,
    pub linework: Match,
    pub layers: Vec<LayerScore>,
    pub text: Match,
    pub text_in_place: usize,
    pub hatch_patterns: Match,
    pub dimensions: Match,
}

impl Score {
    /// The score as tab-separated rows: metric, layer, reference, candidate, precision, recall, f1.
    pub fn to_tsv(&self) -> String {
        let mut out = String::from("metric\tlayer\treference\tcandidate\tprecision\trecall\tf1\n");
        let row = |out: &mut String, metric: &str, layer: &str, m: &Match| {
            out.push_str(&format!(
                "{metric}\t{layer}\t{}\t{}\t{:.4}\t{:.4}\t{:.4}\n",
                m.reference, m.candidate, m.precision, m.recall, m.f1
            ));
        };
        row(&mut out, "linework", "*", &self.linework);
        for l in &self.layers {
            row(&mut out, "linework", &l.layer, &l.linework);
        }
        row(&mut out, "text", "*", &self.text);
        row(&mut out, "hatch_patterns", "*", &self.hatch_patterns);
        row(&mut out, "dimensions", "*", &self.dimensions);
        for s in &self.sheets {
            row(&mut out, &format!("sheet_text:{}", s.name), "*", &s.text);
        }
        out
    }
}

/// Metres per `$INSUNITS` unit, for the units that matter here.
fn metres(units: i16) -> Option<f64> {
    Some(match units {
        1 => 0.0254,
        2 => 0.3048,
        4 => 0.001,
        5 => 0.01,
        6 => 1.0,
        14 => 0.1,
        _ => return None,
    })
}

/// An entity placed where it is drawn, with the layer it is drawn on.
struct Placed<'a> {
    layer: String,
    kind: &'a str,
    geom: Geom,
}

/// Every drawable entity of `block`, with inserts and dimensions expanded (bounded by `limits`).
fn flatten<'a>(d: &'a Drawing2d, block: &str, scale: f64, limits: &Limits) -> Vec<Placed<'a>> {
    let mut by_owner: HashMap<&str, Vec<&Entity2d>> = HashMap::new();
    for e in &d.entities {
        by_owner.entry(e.owner.as_str()).or_default().push(e);
    }
    let base: HashMap<&str, [f64; 3]> =
        d.blocks.iter().map(|b| (b.name.as_str(), b.base)).collect();
    let mut out = Vec::new();
    let mut budget = limits.max_expanded;
    let t = Affine {
        a: scale,
        b: 0.0,
        c: 0.0,
        d: scale,
        e: 0.0,
        f: 0.0,
    };
    #[allow(clippy::too_many_arguments)]
    fn walk<'a>(
        block: &str,
        t: &Affine,
        layer: &str,
        depth: usize,
        by_owner: &HashMap<&str, Vec<&'a Entity2d>>,
        base: &HashMap<&str, [f64; 3]>,
        limits: &Limits,
        budget: &mut usize,
        out: &mut Vec<Placed<'a>>,
    ) {
        if depth > limits.max_insert_depth {
            return;
        }
        for e in by_owner.get(block).into_iter().flatten() {
            if *budget == 0 {
                return;
            }
            *budget -= 1;
            let layer = if e.layer == "0" && depth > 0 {
                layer
            } else {
                e.layer.as_str()
            };
            match &e.geom {
                Geom::Insert {
                    block,
                    at,
                    scale,
                    rotation,
                    ..
                } => {
                    let b = base.get(block.as_str()).copied().unwrap_or([0.0; 3]);
                    walk(
                        block,
                        &t.then(&Affine::insert(b, *at, *scale, *rotation)),
                        layer,
                        depth + 1,
                        by_owner,
                        base,
                        limits,
                        budget,
                        out,
                    );
                    out.push(Placed {
                        layer: layer.into(),
                        kind: &e.kind,
                        geom: e.geom.transformed(t),
                    });
                }
                Geom::Dimension { block, .. } => {
                    walk(
                        block,
                        t,
                        layer,
                        depth + 1,
                        by_owner,
                        base,
                        limits,
                        budget,
                        out,
                    );
                    out.push(Placed {
                        layer: layer.into(),
                        kind: &e.kind,
                        geom: e.geom.transformed(t),
                    });
                }
                _ => out.push(Placed {
                    layer: layer.into(),
                    kind: &e.kind,
                    geom: e.geom.transformed(t),
                }),
            }
        }
    }
    walk(
        block,
        &t,
        "0",
        0,
        &by_owner,
        &base,
        limits,
        &mut budget,
        &mut out,
    );
    out
}

/// Points hashed on a grid of `cell`, for nearest-within-tolerance queries.
struct Grid {
    cell: f64,
    map: HashMap<(i64, i64), Vec<[f64; 2]>>,
}

impl Grid {
    fn new(points: &[[f64; 2]], cell: f64) -> Grid {
        let mut map: HashMap<(i64, i64), Vec<[f64; 2]>> = HashMap::new();
        for p in points {
            map.entry(Self::key(p, cell)).or_default().push(*p);
        }
        Grid { cell, map }
    }

    fn key(p: &[f64; 2], cell: f64) -> (i64, i64) {
        ((p[0] / cell).floor() as i64, (p[1] / cell).floor() as i64)
    }

    fn near(&self, p: &[f64; 2], tol: f64) -> bool {
        let (kx, ky) = Self::key(p, self.cell);
        (-1..=1).any(|dx| {
            (-1..=1).any(|dy| {
                self.map
                    .get(&(kx + dx, ky + dy))
                    .is_some_and(|v| v.iter().any(|q| (q[0] - p[0]).hypot(q[1] - p[1]) <= tol))
            })
        })
    }
}

fn point_match(reference: &[[f64; 2]], candidate: &[[f64; 2]], tol: f64) -> Match {
    let (gr, gc) = (Grid::new(reference, tol), Grid::new(candidate, tol));
    let p_hit = candidate.iter().filter(|p| gr.near(p, tol)).count();
    let r_hit = reference.iter().filter(|p| gc.near(p, tol)).count();
    ratio(reference.len(), candidate.len(), p_hit, r_hit)
}

fn ratio(reference: usize, candidate: usize, p_hit: usize, r_hit: usize) -> Match {
    let precision = if candidate == 0 {
        0.0
    } else {
        p_hit as f64 / candidate as f64
    };
    let recall = if reference == 0 {
        0.0
    } else {
        r_hit as f64 / reference as f64
    };
    let f1 = if precision + recall == 0.0 {
        0.0
    } else {
        2.0 * precision * recall / (precision + recall)
    };
    Match {
        reference,
        candidate,
        precision,
        recall,
        f1,
    }
}

fn multiset_match<T: Ord + Clone>(reference: &[T], candidate: &[T]) -> Match {
    let mut left: BTreeMap<T, usize> = BTreeMap::new();
    for r in reference {
        *left.entry(r.clone()).or_default() += 1;
    }
    let mut hit = 0;
    for c in candidate {
        if let Some(n) = left.get_mut(c).filter(|n| **n > 0) {
            *n -= 1;
            hit += 1;
        }
    }
    ratio(reference.len(), candidate.len(), hit, hit)
}

/// Score `candidate` against `reference`.
pub fn compare(reference: &Drawing2d, candidate: &Drawing2d, o: &Options) -> Score {
    let scale = match (metres(reference.units), metres(candidate.units)) {
        (Some(r), Some(c)) => c / r,
        _ => 1.0,
    };
    let tol = o.tolerance.max(1e-9);
    let step = o.step.unwrap_or(tol / 2.0).max(1e-9);
    let block = |d: &Drawing2d| match &o.layout {
        None => crate::MODEL_SPACE.to_string(),
        Some(name) => d
            .layouts
            .iter()
            .find(|l| &l.name == name)
            .map(|l| l.block.clone())
            .unwrap_or_default(),
    };
    let r_ents = flatten(reference, &block(reference), 1.0, &o.limits);
    let c_ents = flatten(candidate, &block(candidate), scale, &o.limits);
    let map_layer = |l: &str| o.layer_map.get(l).cloned().unwrap_or_else(|| l.to_string());

    // Linework by layer (the reference's layer names, mapped).
    let mut r_pts: BTreeMap<String, Vec<[f64; 2]>> = BTreeMap::new();
    let mut c_pts: BTreeMap<String, Vec<[f64; 2]>> = BTreeMap::new();
    let mut hatch_r: BTreeMap<String, usize> = BTreeMap::new();
    let mut hatch_c: BTreeMap<String, usize> = BTreeMap::new();
    let line = |g: &Geom| {
        matches!(
            g,
            Geom::Line { .. }
                | Geom::Circle { .. }
                | Geom::Arc { .. }
                | Geom::Ellipse { .. }
                | Geom::Polyline { .. }
                | Geom::Path { .. }
        )
    };
    for p in &r_ents {
        let layer = map_layer(&p.layer);
        if line(&p.geom) {
            r_pts
                .entry(layer.clone())
                .or_default()
                .extend(p.geom.samples(step));
        }
        if matches!(p.geom, Geom::Hatch { .. }) {
            *hatch_r.entry(layer).or_default() += 1;
        }
    }
    for p in &c_ents {
        if line(&p.geom) {
            c_pts
                .entry(p.layer.clone())
                .or_default()
                .extend(p.geom.samples(step));
        }
        if matches!(p.geom, Geom::Hatch { .. }) {
            *hatch_c.entry(p.layer.clone()).or_default() += 1;
        }
    }
    let mut names: Vec<String> = r_pts
        .keys()
        .chain(c_pts.keys())
        .chain(hatch_r.keys())
        .chain(hatch_c.keys())
        .cloned()
        .collect();
    names.sort();
    names.dedup();
    let empty = Vec::new();
    let layers = names
        .iter()
        .map(|l| LayerScore {
            layer: l.clone(),
            linework: point_match(
                r_pts.get(l).unwrap_or(&empty),
                c_pts.get(l).unwrap_or(&empty),
                tol,
            ),
            hatches_reference: hatch_r.get(l).copied().unwrap_or(0),
            hatches_candidate: hatch_c.get(l).copied().unwrap_or(0),
        })
        .collect();
    let all_r: Vec<[f64; 2]> = r_pts.values().flatten().copied().collect();
    let all_c: Vec<[f64; 2]> = c_pts.values().flatten().copied().collect();

    // Text, hatch patterns, dimensions.
    let texts = |ents: &[Placed]| -> Vec<(String, [f64; 2])> {
        ents.iter()
            .filter_map(|p| match &p.geom {
                Geom::Text { value, at, .. } if !value.trim().is_empty() && p.kind != "ATTDEF" => {
                    Some((value.trim().to_string(), [at[0], at[1]]))
                }
                _ => None,
            })
            .collect()
    };
    let (rt, ct) = (texts(&r_ents), texts(&c_ents));
    let strings = |v: &[(String, [f64; 2])]| v.iter().map(|(s, _)| s.clone()).collect::<Vec<_>>();
    let text = multiset_match(&strings(&rt), &strings(&ct));
    let text_in_place = ct
        .iter()
        .filter(|(s, at)| {
            rt.iter()
                .any(|(rs, rat)| rs == s && (rat[0] - at[0]).hypot(rat[1] - at[1]) <= tol)
        })
        .count();
    let patterns = |ents: &[Placed]| -> Vec<String> {
        ents.iter()
            .filter_map(|p| match &p.geom {
                Geom::Hatch { solid: true, .. } => Some("SOLID".to_string()),
                Geom::Hatch { pattern, .. } => Some(pattern.to_ascii_uppercase()),
                _ => None,
            })
            .collect()
    };
    let dims = |ents: &[Placed], k: f64| -> Vec<i64> {
        ents.iter()
            .filter_map(|p| match &p.geom {
                Geom::Dimension { measurement, .. } => {
                    Some((measurement * k * 1000.0).round() as i64)
                }
                _ => None,
            })
            .collect()
    };

    let paper = |d: &Drawing2d| {
        d.layouts
            .iter()
            .filter(|l| !l.is_model)
            .map(|l| l.name.clone())
            .collect::<Vec<_>>()
    };
    let (sr, sc) = (paper(reference), paper(candidate));
    let sheets = sr
        .iter()
        .filter(|n| sc.contains(n))
        .map(|n| sheet_score(reference, candidate, n, &o.limits))
        .collect();

    Score {
        units_scale: scale,
        tolerance: tol,
        sheets_reference: sr,
        sheets_candidate: sc,
        sheets,
        linework: point_match(&all_r, &all_c, tol),
        layers,
        text,
        text_in_place,
        hatch_patterns: multiset_match(&patterns(&r_ents), &patterns(&c_ents)),
        dimensions: multiset_match(&dims(&r_ents, 1.0), &dims(&c_ents, scale)),
    }
}

fn sheet_score(
    reference: &Drawing2d,
    candidate: &Drawing2d,
    name: &str,
    limits: &Limits,
) -> SheetScore {
    let block = |d: &Drawing2d| {
        d.layouts
            .iter()
            .find(|l| l.name == name)
            .map(|l| l.block.clone())
            .unwrap_or_default()
    };
    let (rb, cb) = (block(reference), block(candidate));
    // Paper units are the same on both sides (millimetres on a sheet): no units scale here.
    let viewports = |d: &Drawing2d, b: &str| -> Vec<([f64; 2], f64)> {
        d.entities
            .iter()
            .filter(|e| e.owner == b && Space::of(b) == Space::Paper)
            .filter_map(|e| match e.geom {
                Geom::Viewport {
                    center,
                    height,
                    view_height,
                    id,
                    ..
                } if id != 1 && view_height > 0.0 => {
                    Some(([center[0], center[1]], height / view_height))
                }
                _ => None,
            })
            .collect()
    };
    let (rv, cv) = (viewports(reference, &rb), viewports(candidate, &cb));
    let pairs = rv
        .iter()
        .filter_map(|(rc, rs)| {
            cv.iter()
                .map(|(cc, cs)| ((cc[0] - rc[0]).hypot(cc[1] - rc[1]), *cs))
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(dist, cs)| ViewportScore {
                    centre_distance: dist,
                    scale_reference: *rs,
                    scale_candidate: cs,
                })
        })
        .collect();
    let inserts = |d: &Drawing2d, b: &str| -> Vec<String> {
        let mut v: Vec<String> = d
            .entities
            .iter()
            .filter(|e| e.owner == b)
            .filter_map(|e| match &e.geom {
                Geom::Insert { block, .. } => Some(block.clone()),
                _ => None,
            })
            .collect();
        v.sort();
        v
    };
    let strings = |d: &Drawing2d, b: &str| -> Vec<String> {
        let mut v: Vec<String> = flatten(d, b, 1.0, limits)
            .into_iter()
            .filter_map(|p| match p.geom {
                Geom::Text { value, .. } if !value.trim().is_empty() => {
                    Some(value.trim().to_string())
                }
                Geom::Insert { attributes, .. } => Some(
                    attributes
                        .into_iter()
                        .map(|(_, v)| v)
                        .filter(|v| !v.is_empty())
                        .collect::<Vec<_>>()
                        .join("\u{1f}"),
                )
                .filter(|s| !s.is_empty()),
                _ => None,
            })
            .flat_map(|s| s.split('\u{1f}').map(str::to_string).collect::<Vec<_>>())
            .collect();
        v.sort();
        v
    };
    SheetScore {
        name: name.to_string(),
        viewports_reference: rv.len(),
        viewports_candidate: cv.len(),
        viewports: pairs,
        title_blocks_reference: inserts(reference, &rb),
        title_blocks_candidate: inserts(candidate, &cb),
        text: multiset_match(&strings(reference, &rb), &strings(candidate, &cb)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Layer2d;

    fn line(layer: &str, a: [f64; 2], b: [f64; 2]) -> Entity2d {
        Entity2d::new(
            "LINE",
            layer,
            Geom::Line {
                a: [a[0], a[1], 0.0],
                b: [b[0], b[1], 0.0],
            },
        )
    }

    #[test]
    fn identical_drawings_score_one_and_a_missing_wall_lowers_recall() {
        let reference = Drawing2d {
            units: 4,
            layers: vec![Layer2d::new("A-WALL", 7)],
            entities: vec![
                line("A-WALL", [0.0, 0.0], [1000.0, 0.0]),
                line("A-WALL", [0.0, 0.0], [0.0, 1000.0]),
            ],
            ..Default::default()
        };
        let same = compare(&reference, &reference, &Options::default());
        assert_eq!((same.linework.precision, same.linework.recall), (1.0, 1.0));

        // The candidate in feet holds only the first wall: precision stays 1, recall about half.
        let candidate = Drawing2d {
            units: 2,
            entities: vec![line("A-WALL", [0.0, 0.0], [1000.0 / 304.8, 0.0])],
            ..Default::default()
        };
        let s = compare(&reference, &candidate, &Options::default());
        assert!((s.units_scale - 304.8).abs() < 1e-9);
        assert!(s.linework.precision > 0.999, "{:?}", s.linework);
        assert!((s.linework.recall - 0.5).abs() < 0.01, "{:?}", s.linework);
        assert!(s.to_tsv().starts_with("metric\tlayer\treference"));
    }
}
