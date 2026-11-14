//! Tessellation of bounded saved planar and cylindrical BRep faces. No application or vendor runtime.
use crate::graph_value::Fv;
use crate::native_parameters::ObjectGraph;
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FaceMesh {
    pub face_index: usize,
    pub face_tag: i64,
    pub render_style_id: i64,
    pub vertices: Vec<[f64; 3]>,
    pub triangles: Vec<[u32; 3]>,
    pub normal: [f64; 3],
    #[serde(default)]
    pub normals: Vec<[f64; 3]>,
    #[serde(default)]
    pub analytic_surface: Option<AnalyticCylinder>,
    #[serde(default)]
    pub trim_uv: Vec<[f64; 2]>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyticCylinder {
    pub center: [f64; 3],
    pub radius: f64,
    pub x_vec: [f64; 3],
    pub y_vec: [f64; 3],
    pub z_vec: [f64; 3],
    pub orient_flag: bool,
    pub u_range: [f64; 2],
    pub v_range: [f64; 2],
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CylinderTessellationProfile {
    pub chord_error: f64,
    pub max_v_edge: f64,
}
impl Default for CylinderTessellationProfile {
    fn default() -> Self {
        Self {
            chord_error: 0.001,
            max_v_edge: 1.0,
        }
    }
}
struct PointerIndex<'a> {
    graph: &'a ObjectGraph,
    lookup: &'a crate::native_parameters::PointerLookup,
}
impl<'a> PointerIndex<'a> {
    fn new(graph: &'a ObjectGraph) -> Self {
        Self {
            graph,
            lookup: graph.lookup(),
        }
    }
    fn resolve(&self, source: usize, v: &Fv) -> Result<Option<usize>> {
        let g = self.graph;
        let t = v["pointer_token"]
            .as_u64()
            .context("missing pointer token")?;
        if t == 0 {
            return Ok(None);
        }
        let off = v["offset"].as_u64().context("pointer offset")? as usize;
        let edges = self
            .lookup
            .edges
            .get(&(source, off))
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if edges.len() == 1 {
            let e = &g.edges[edges[0]];
            let target = g
                .objects
                .get(e.target_object_index)
                .context("pointer target out of bounds")?;
            ensure!(
                e.pointer_token as u64 == t && target.class_tag == e.target_class_tag,
                "pointer edge metadata mismatch"
            );
            if let Some(tag) = v["class_tag"].as_u64() {
                ensure!(
                    tag == target.class_tag as u64,
                    "pointer target class mismatch"
                )
            }
            return Ok(Some(e.target_object_index));
        }
        ensure!(edges.is_empty(), "ambiguous pointer edge");
        ensure!(t != u32::MAX as u64, "unresolved anonymous pointer");
        let objs = self.lookup.tokens.get(&t).map(Vec::as_slice).unwrap_or(&[]);
        ensure!(objs.len() == 1, "unresolved or ambiguous token {t}");
        Ok(Some(objs[0]))
    }
}
pub fn pointer(g: &ObjectGraph, source: usize, v: &Fv) -> Result<Option<usize>> {
    PointerIndex::new(g).resolve(source, v)
}
fn point<const N: usize>(v: &Fv) -> Result<[f64; N]> {
    let a = v.as_array().context("point array")?;
    ensure!(a.len() == N, "point dimension");
    let mut p = [0.; N];
    for (i, x) in a.iter().enumerate() {
        p[i] = x.as_f64().context("point number")?;
        ensure!(p[i].is_finite(), "nonfinite point")
    }
    Ok(p)
}
fn close(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).hypot(a[1] - b[1]) < 1e-7
}
fn cross(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}
fn area(r: &[[f64; 2]]) -> f64 {
    (0..r.len())
        .map(|i| r[i][0] * r[(i + 1) % r.len()][1] - r[(i + 1) % r.len()][0] * r[i][1])
        .sum::<f64>()
        / 2.
}
fn retain_boundary_points(
    points: &[[f64; 2]],
    boundary_edges: &BTreeSet<(u32, u32)>,
    triangles: &mut Vec<[u32; 3]>,
) -> Result<()> {
    // Which points a triangle uses, kept as splits add them, instead of a
    // search of every triangle for every point.
    let mut used = vec![false; points.len()];
    for &vertex in triangles.iter().flatten() {
        if let Some(slot) = used.get_mut(vertex as usize) {
            *slot = true;
        }
    }
    for pi in 0..points.len() {
        if used[pi] {
            continue;
        }
        let p = points[pi];
        let mut split = None;
        for (ti, t) in triangles.iter().enumerate() {
            for edge in 0..3 {
                let a = t[edge] as usize;
                let b = t[(edge + 1) % 3] as usize;
                let c = t[(edge + 2) % 3];
                let edge_key = (
                    t[edge].min(t[(edge + 1) % 3]),
                    t[edge].max(t[(edge + 1) % 3]),
                );
                let boundary_line = boundary_edges.contains(&edge_key)
                    || boundary_edges.iter().any(|(u, v)| {
                        cross(points[*u as usize], points[*v as usize], points[a]).abs() <= 1e-8
                            && cross(points[*u as usize], points[*v as usize], points[b]).abs()
                                <= 1e-8
                            && cross(points[*u as usize], points[*v as usize], p).abs() <= 1e-8
                    });
                if !boundary_line {
                    continue;
                }
                let ab = [points[b][0] - points[a][0], points[b][1] - points[a][1]];
                let ap = [p[0] - points[a][0], p[1] - points[a][1]];
                let length2 = ab[0] * ab[0] + ab[1] * ab[1];
                let dot = ap[0] * ab[0] + ap[1] * ab[1];
                if length2 > 1e-24
                    && cross(points[a], points[b], p).abs() <= 1e-8
                    && dot > 1e-12
                    && dot < length2 - 1e-12
                {
                    split = Some((
                        ti,
                        [t[edge], pi as u32, c],
                        [pi as u32, t[(edge + 1) % 3], c],
                    ));
                    break;
                }
            }
            if split.is_some() {
                break;
            }
        }
        let Some((ti, first, second)) = split else {
            bail!("boundary sample was discarded by triangulation")
        };
        triangles[ti] = first;
        triangles.push(second);
        used[pi] = true;
    }
    Ok(())
}

// Refine a particular shared edge in its curved surface's parameter domain.
// The planar side is reconstructed from the same evaluated 3D positions, not
// from interpolation along the old polygon's chords.
fn refined_plane_edge(
    g: &ObjectGraph,
    index: &PointerIndex<'_>,
    face: usize,
    edge_index: usize,
    plane_side: usize,
    original: Vec<[f64; 2]>,
) -> Result<Vec<[f64; 2]>> {
    let edge = &g.objects[edge_index];
    let faces = edge.fields["m_pFace"]
        .as_array()
        .context("edge face pair")?;
    ensure!(faces.len() == 2, "edge face pair length");
    let other_side = 1 - plane_side;
    let Some(other) = index.resolve(edge_index, &faces[other_side])? else {
        return Ok(original);
    };
    let other_surface = index
        .resolve(other, &g.objects[other].fields["m_pSurf"])?
        .context("shared edge surface missing")?;
    let kind = g.objects[other_surface].class_name.as_str();
    if !matches!(kind, "CylSurf" | "SurfRev" | "RuledSurf") {
        return Ok(original);
    }
    type SurfaceEval = Box<dyn Fn([f64; 2]) -> Result<[f64; 3]>>;
    let (u_values, v_values, evaluate): (Vec<f64>, Vec<f64>, SurfaceEval) = if kind == "CylSurf" {
        let profile = CylinderTessellationProfile::default();
        let mesh = cylinder_face(g, other, profile)?;
        let cylinder = mesh
            .analytic_surface
            .context("cylinder analytic metadata missing")?;
        let min: [f64; 2] = std::array::from_fn(|k| {
            mesh.trim_uv
                .iter()
                .map(|p| p[k])
                .fold(f64::INFINITY, f64::min)
        });
        let max: [f64; 2] = std::array::from_fn(|k| {
            mesh.trim_uv
                .iter()
                .map(|p| p[k])
                .fold(f64::NEG_INFINITY, f64::max)
        });
        let step = (4.
            * (profile.chord_error / (2. * cylinder.radius))
                .min(1.)
                .sqrt()
                .asin())
        .min(std::f64::consts::FRAC_PI_2);
        let us = tessellation_axis(min[0], max[0], step, mesh.trim_uv.iter().map(|p| p[0]))?;
        let vs = tessellation_axis(
            min[1],
            max[1],
            profile.max_v_edge,
            mesh.trim_uv.iter().map(|p| p[1]),
        )?;
        let eval = Box::new(move |uv: [f64; 2]| {
            let (sn, cs) = uv[0].sin_cos();
            Ok(std::array::from_fn(|k| {
                cylinder.center[k]
                    + cylinder.radius * (cs * cylinder.x_vec[k] + sn * cylinder.y_vec[k])
                    + uv[1] * cylinder.z_vec[k]
            }))
        });
        (us, vs, eval)
    } else {
        let grid = crate::native_parametric_mesh::face_grid(g, other)?;
        (
            grid.u,
            grid.v,
            Box::new(move |uv: [f64; 2]| grid.surface.evaluate(uv[0], uv[1])),
        )
    };
    let ends = edge.fields["m_firstAndLastEdgePnts"]
        .as_array()
        .context("edge endpoints")?;
    ensure!(ends.len() == 2, "edge endpoint count");
    let mut samples = vec![point::<2>(&ends[0]["uv"][other_side])?];
    for sample in edge.fields["m_interiorEdgePnts"]
        .as_array()
        .context("edge interior samples")?
    {
        samples.push(point::<2>(&sample["uv"][other_side])?);
    }
    samples.push(point::<2>(&ends[1]["uv"][other_side])?);
    let start = samples[0];
    let end = *samples.last().unwrap();
    let axis = if (start[1] - end[1]).abs() < 1e-7 && (start[0] - end[0]).abs() > 1e-7 {
        0
    } else if (start[0] - end[0]).abs() < 1e-7 && (start[1] - end[1]).abs() > 1e-7 {
        1
    } else {
        bail!("shared curved edge is not an isoparametric interval")
    };
    ensure!(
        samples
            .iter()
            .all(|uv| (uv[1 - axis] - start[1 - axis]).abs() < 1e-7),
        "shared curved edge changes its fixed parameter"
    );
    let plane_index = index
        .resolve(face, &g.objects[face].fields["m_pSurf"])?
        .context("plane surface missing")?;
    let plane = &g.objects[plane_index];
    ensure!(
        plane.class_name == "Plane",
        "edge refinement requires a plane"
    );
    let origin = point::<3>(&plane.fields["m_origin"])?;
    let x = point::<3>(&plane.fields["m_xVec"])?;
    let y = point::<3>(&plane.fields["m_yVec"])?;
    let dot = |a: [f64; 3], b: [f64; 3]| a.iter().zip(b).map(|(a, b)| a * b).sum::<f64>();
    let xx = dot(x, x);
    let xy = dot(x, y);
    let yy = dot(y, y);
    let det = xx * yy - xy * xy;
    ensure!(
        det.is_finite() && det > 1e-24,
        "singular shared plane basis"
    );
    let project = |uv: [f64; 2]| -> Result<[f64; 2]> {
        let q = evaluate(uv)?;
        let d = std::array::from_fn(|k| q[k] - origin[k]);
        let dx = dot(d, x);
        let dy = dot(d, y);
        let p = [(dx * yy - dy * xy) / det, (dy * xx - dx * xy) / det];
        let residual = (0..3)
            .map(|k| (origin[k] + p[0] * x[k] + p[1] * y[k] - q[k]).powi(2))
            .sum::<f64>();
        ensure!(
            residual <= 1e-14,
            "shared curved edge leaves adjacent plane"
        );
        Ok(p)
    };
    ensure!(
        samples.len() == original.len(),
        "shared edge sample count mismatch"
    );
    for (uv, expected) in samples.iter().zip(&original) {
        ensure!(
            close(project(*uv)?, *expected),
            "shared edge surface samples disagree"
        );
    }
    let knots = if axis == 0 { u_values } else { v_values };
    let lo = start[axis].min(end[axis]);
    let hi = start[axis].max(end[axis]);
    let mut values = knots
        .into_iter()
        .filter(|v| *v > lo + 1e-12 && *v < hi - 1e-12)
        .collect::<Vec<_>>();
    values.insert(0, lo);
    values.push(hi);
    if start[axis] > end[axis] {
        values.reverse();
    }
    values
        .into_iter()
        .map(|t| {
            let mut uv = start;
            uv[axis] = t;
            project(uv)
        })
        .collect()
}
fn inside(p: [f64; 2], r: &[[f64; 2]]) -> bool {
    let mut hit = false;
    for i in 0..r.len() {
        let a = r[i];
        let b = r[(i + 1) % r.len()];
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
        {
            hit = !hit
        }
    }
    hit
}
/// Every trim loop of face `fi` in UV, outer boundary and holes.
pub(crate) fn face_loops_uv(g: &ObjectGraph, fi: usize) -> Result<Vec<Vec<[f64; 2]>>> {
    let index = PointerIndex::new(g);
    let f = g.objects.get(fi).context("face index")?;
    let mut li = index
        .resolve(fi, &f.fields["m_pFirstLoop"])?
        .context("face loop")?;
    let mut loops = Vec::new();
    let mut seen = BTreeSet::new();
    loop {
        ensure!(seen.insert(li), "loop-list cycle");
        loops.push(ring(g, &index, fi, li)?);
        match index.resolve(li, &g.objects[li].fields["m_nextLoop"])? {
            Some(next) => li = next,
            None => break,
        }
    }
    Ok(loops)
}
fn ring(
    g: &ObjectGraph,
    index: &PointerIndex<'_>,
    face: usize,
    li: usize,
) -> Result<Vec<[f64; 2]>> {
    ring_impl(g, index, face, li, false)
}
fn ring_impl(
    g: &ObjectGraph,
    index: &PointerIndex<'_>,
    face: usize,
    li: usize,
    refine_plane: bool,
) -> Result<Vec<[f64; 2]>> {
    let l = &g.objects[li];
    ensure!(
        ["EdgeLoop", "EdgeLoopWithChainEnvelopes"].contains(&l.class_name.as_str()),
        "unsupported loop {}",
        l.class_name
    );
    ensure!(l.fields["m_open"] == false, "open face loop");
    ensure!(
        index.resolve(li, &l.fields["m_pFace"])? == Some(face),
        "loop owner mismatch"
    );
    let mut cur = index
        .resolve(li, &l.fields["m_next"])?
        .context("empty loop")?;
    let mut seen = BTreeSet::new();
    let mut segments = Vec::new();
    while cur != li {
        ensure!(seen.insert(cur), "edge chain cycle before loop sentinel");
        let e = &g.objects[cur];
        ensure!(e.class_name == "Edge", "nonedge loop member");
        let faces = e.fields["m_pFace"].as_array().context("edge face pair")?;
        let sides: Vec<_> = faces
            .iter()
            .enumerate()
            .filter_map(|(i, p)| match index.resolve(cur, p) {
                Ok(Some(f)) if f == face => Some(Ok(i)),
                Err(e) => Some(Err(e)),
                _ => None,
            })
            .collect::<Result<Vec<_>>>()?;
        ensure!(sides.len() == 1, "ambiguous edge face side");
        let side = sides[0];
        let endpoints = e.fields["m_firstAndLastEdgePnts"]
            .as_array()
            .context("edge endpoints")?;
        ensure!(endpoints.len() == 2, "edge endpoint count");
        let mut pts = vec![point::<2>(&endpoints[0]["uv"][side])?];
        for p in e.fields["m_interiorEdgePnts"]
            .as_array()
            .context("edge interior points")?
        {
            pts.push(point::<2>(&p["uv"][side])?)
        }
        pts.push(point::<2>(&endpoints[1]["uv"][side])?);
        if refine_plane {
            // where the curved neighbour's parameters do not give
            // the edge (a slanted or curved trim there), keep Revit's own
            // samples of the edge, which the neighbour's loop also uses.
            pts = refined_plane_edge(g, index, face, cur, side, pts.clone()).unwrap_or(pts);
        }
        segments.push(pts);
        cur = index
            .resolve(cur, &e.fields["m_next"][side])?
            .context("null next edge")?;
    }
    // Saved circular loops can be represented by two connected arc edges
    // (typically two semicircles). A third edge is not a topology invariant;
    // closure, nonzero area, and connected ordering below remain mandatory.
    ensure!(segments.len() >= 2, "too few boundary edges");
    ensure!(
        segments.iter().all(|s: &Vec<_>| !s.is_empty()),
        "empty boundary edge"
    );
    let mut output = None;
    for reverse in [false, true] {
        let mut out = segments[0].clone();
        if reverse {
            out.reverse()
        };
        let mut valid = true;
        for s in &segments[1..] {
            let last = *out.last().unwrap();
            if close(last, s[0]) {
                out.extend_from_slice(&s[1..])
            } else if close(last, *s.last().unwrap()) {
                out.extend(s.iter().rev().skip(1).copied())
            } else {
                valid = false;
                break;
            }
        }
        if valid && close(out[0], *out.last().unwrap()) {
            out.pop();
            output = Some(out);
            break;
        }
    }
    let mut out = output.context("disconnected UV boundary")?;
    out.dedup_by(|a, b| close(*a, *b));
    ensure!(
        out.len() >= 3 && area(&out).abs() > 1e-12,
        "degenerate boundary"
    );
    Ok(out)
}

fn tessellation_axis(
    min: f64,
    max: f64,
    step: f64,
    knots: impl Iterator<Item = f64>,
) -> Result<Vec<f64>> {
    ensure!(
        min.is_finite() && max.is_finite() && step.is_finite() && step > 0. && max > min,
        "invalid tessellation axis"
    );
    let count = ((max - min) / step).ceil();
    ensure!(
        count.is_finite() && (1. ..=2_000_000.).contains(&count),
        "tessellation axis too large"
    );
    let n = count as usize;
    let mut values = (0..=n)
        .map(|i| min + (max - min) * i as f64 / n as f64)
        .collect::<Vec<_>>();
    values.extend(knots.filter(|v| v.is_finite() && *v > min && *v < max));
    values.sort_by(f64::total_cmp);
    values.dedup_by(|a, b| (*a - *b).abs() <= 1e-12);
    ensure!(values.len() >= 2, "degenerate tessellation axis");
    Ok(values)
}
/// Returns no mesh for a face with no trimming boundary (an auxiliary surface).
pub fn face(g: &ObjectGraph, fi: usize) -> Result<Option<FaceMesh>> {
    face_indexed(g, &PointerIndex::new(g), fi)
}
fn face_indexed(g: &ObjectGraph, index: &PointerIndex<'_>, fi: usize) -> Result<Option<FaceMesh>> {
    let f = g.objects.get(fi).context("face index")?;
    ensure!(f.class_name == "Face", "not Face");
    let Some(mut li) = index.resolve(fi, &f.fields["m_pFirstLoop"])? else {
        return Ok(None);
    };
    // a face split into regions (Revit's Split Face) keeps its
    // own trimming loops round the whole face; its regions are further Faces
    // inside it. The face is drawn whole from its own loops.
    f.fields["m_faceRegions"]
        .as_array()
        .context("face regions")?;
    let pi = index
        .resolve(fi, &f.fields["m_pSurf"])?
        .context("face surface")?;
    let p = &g.objects[pi];
    if p.class_name == "CylSurf" {
        return Ok(Some(cylinder_face(
            g,
            fi,
            CylinderTessellationProfile::default(),
        )?));
    }
    if p.class_name == "ConeSurf" {
        return Ok(Some(cone_face(
            g,
            fi,
            CylinderTessellationProfile::default(),
        )?));
    }
    if matches!(p.class_name.as_str(), "SurfRev" | "RuledSurf") {
        return Ok(Some(crate::native_parametric_mesh::face(g, fi)?));
    }
    ensure!(
        p.class_name == "Plane",
        "unsupported surface {}",
        p.class_name
    );
    let origin = point::<3>(&p.fields["m_origin"])?;
    let x = point::<3>(&p.fields["m_xVec"])?;
    let y = point::<3>(&p.fields["m_yVec"])?;
    let orient = p.fields["m_orientFlag"]
        .as_bool()
        .context("plane orientation")?
        ^ (f.fields["m_faceFlags_v9"].as_u64().context("face flags")? & 2 != 0);
    let mut normal = [
        x[1] * y[2] - x[2] * y[1],
        x[2] * y[0] - x[0] * y[2],
        x[0] * y[1] - x[1] * y[0],
    ];
    let norm = normal.iter().map(|v| v * v).sum::<f64>().sqrt();
    ensure!(norm > 1e-12, "singular plane");
    for v in &mut normal {
        *v /= if orient { norm } else { -norm }
    }
    let mut loops = Vec::new();
    let mut seen = BTreeSet::new();
    loop {
        ensure!(seen.insert(li), "loop-list cycle");
        loops.push(ring_impl(g, index, fi, li, true)?);
        match index.resolve(li, &g.objects[li].fields["m_nextLoop"])? {
            Some(next) => li = next,
            None => break,
        }
    }
    // A face can contain multiple disconnected outer rings, with nested holes.
    let depths: Vec<_> = loops
        .iter()
        .enumerate()
        .map(|(i, r)| {
            loops
                .iter()
                .enumerate()
                .filter(|(j, s)| *j != i && inside(r[0], s))
                .count()
        })
        .collect();
    let mut vertices = Vec::new();
    let mut triangles = Vec::new();
    for (i, outer) in loops
        .iter()
        .enumerate()
        .filter(|(i, _)| depths[*i] % 2 == 0)
    {
        let mut pts = outer.clone();
        let mut holes = Vec::new();
        let mut boundary_edges = BTreeSet::new();
        let mut add_boundary_edges = |start: usize, end: usize| {
            for k in start..end {
                let a = k as u32;
                let b = (if k + 1 == end { start } else { k + 1 }) as u32;
                boundary_edges.insert((a.min(b), a.max(b)));
            }
        };
        add_boundary_edges(0, pts.len());
        let mut expected = area(outer).abs();
        for (j, h) in loops.iter().enumerate() {
            if depths[j] == depths[i] + 1 && inside(h[0], outer) {
                holes.push(pts.len() as u32);
                let start = pts.len();
                pts.extend_from_slice(h);
                add_boundary_edges(start, pts.len());
                expected -= area(h).abs()
            }
        }
        let mut ts = Vec::<u32>::new();
        earcut::Earcut::new().earcut(pts.iter().copied(), &holes, &mut ts);
        ensure!(
            !ts.is_empty() && ts.len().is_multiple_of(3),
            "triangulation failed"
        );
        let mut earcut_triangles = ts
            .chunks_exact(3)
            .map(|t| [t[0], t[1], t[2]])
            .collect::<Vec<_>>();
        ensure!(
            earcut_triangles
                .iter()
                .flatten()
                .all(|i| (*i as usize) < pts.len()),
            "triangle bounds"
        );
        // Earcut can retain collinear points only in zero-area triangles. Drop
        // those first so boundary restoration does not mistake them for usable
        // incident triangles and subsequently leave a T-junction behind.
        earcut_triangles.retain(|t| {
            cross(pts[t[0] as usize], pts[t[1] as usize], pts[t[2] as usize]).abs() > 1e-12
        });
        retain_boundary_points(&pts, &boundary_edges, &mut earcut_triangles)?;
        let mut sum = 0.;
        let base = vertices.len() as u32;
        for t in &mut earcut_triangles {
            ensure!(
                t.iter().all(|v| (*v as usize) < pts.len()),
                "triangle bounds"
            );
            let a = cross(pts[t[0] as usize], pts[t[1] as usize], pts[t[2] as usize]);
            sum += a.abs() / 2.;
            if a.abs() <= 1e-12 {
                continue;
            }
            if (a > 0.) != orient {
                t.swap(1, 2)
            }
            triangles.push([base + t[0], base + t[1], base + t[2]])
        }
        ensure!(
            (sum - expected).abs() <= 1e-7 * expected.max(1.),
            "triangulated area mismatch: {sum} vs {expected}"
        );
        for uv in pts {
            vertices.push(std::array::from_fn(|k| {
                origin[k] + uv[0] * x[k] + uv[1] * y[k]
            }))
        }
    }
    if triangles.is_empty() {
        bail!("no bounded triangles")
    }
    let vertex_count = vertices.len();
    Ok(Some(FaceMesh {
        face_index: fi,
        face_tag: f.fields["m_GInfo"]["m_tag"].as_i64().unwrap_or(-1),
        render_style_id: crate::native_metadata::identifier(&f.fields["m_renderStyleId"])
            .context("render style id")?,
        vertices,
        triangles,
        normal,
        normals: vec![normal; vertex_count],
        analytic_surface: None,
        trim_uv: Vec::new(),
    }))
}

/// for a convex trim polygon in (u, v), the span of v it covers at
/// draw every trim, convex ones included; this stays for its test.
#[cfg(test)]
fn convex_trim_columns(trim: Vec<[f64; 2]>, tol: f64) -> Option<impl Fn(f64) -> (f64, f64)> {
    let n = trim.len();
    if n < 3 {
        return None;
    }
    let mut sign = 0.0_f64;
    for i in 0..n {
        let (a, b, c) = (trim[i], trim[(i + 1) % n], trim[(i + 2) % n]);
        let cross = (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0]);
        if cross.abs() <= tol {
            continue;
        }
        if sign != 0.0 && cross.signum() != sign {
            return None;
        }
        sign = cross.signum();
    }
    Some(move |u: f64| {
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for i in 0..n {
            let (a, b) = (trim[i], trim[(i + 1) % n]);
            let (u0, u1) = (a[0].min(b[0]), a[0].max(b[0]));
            if u < u0 - tol || u > u1 + tol {
                continue;
            }
            let du = b[0] - a[0];
            let vs = if du.abs() <= tol {
                [a[1], b[1]]
            } else {
                let v = a[1] + (u - a[0]) / du * (b[1] - a[1]);
                [v, v]
            };
            for v in vs {
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
        if lo > hi { (0.0, 0.0) } else { (lo, hi) }
    })
}
/// Tessellate one Face whose saved surface is a bounded cylindrical surface.
/// The analytic surface and UV trim are retained alongside the mesh.
pub fn cylinder_face(
    g: &ObjectGraph,
    fi: usize,
    profile: CylinderTessellationProfile,
) -> Result<FaceMesh> {
    ensure!(
        profile.chord_error.is_finite() && profile.chord_error > 0.,
        "invalid cylinder chord error"
    );
    ensure!(
        profile.max_v_edge.is_finite() && profile.max_v_edge > 0.,
        "invalid cylinder V edge"
    );
    let index = PointerIndex::new(g);
    let face = g.objects.get(fi).context("face index")?;
    ensure!(face.class_name == "Face", "not Face");
    // a split face is drawn whole from its own loops.
    ensure!(face.fields["m_faceRegions"].is_array(), "face regions");
    let surface_index = index
        .resolve(fi, &face.fields["m_pSurf"])?
        .context("face surface")?;
    let surface = &g.objects[surface_index];
    ensure!(
        surface.class_name == "CylSurf",
        "unsupported surface {}",
        surface.class_name
    );
    let center = point::<3>(&surface.fields["m_center"])?;
    let radius = surface.fields["m_radius"]
        .as_f64()
        .context("cylinder radius")?;
    ensure!(radius.is_finite() && radius > 0., "invalid cylinder radius");
    let x_vec = point::<3>(&surface.fields["m_xVec"])?;
    let y_vec = point::<3>(&surface.fields["m_yVec"])?;
    let z_vec = point::<3>(&surface.fields["m_zVec"])?;
    let dot = |a: [f64; 3], b: [f64; 3]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f64>();
    let norm = |a: [f64; 3]| dot(a, a).sqrt();
    ensure!(
        (norm(x_vec) - 1.).abs() < 1e-7
            && (norm(y_vec) - 1.).abs() < 1e-7
            && (norm(z_vec) - 1.).abs() < 1e-7,
        "cylinder basis not unit"
    );
    ensure!(
        dot(x_vec, y_vec).abs() < 1e-7
            && dot(x_vec, z_vec).abs() < 1e-7
            && dot(y_vec, z_vec).abs() < 1e-7,
        "cylinder basis not orthogonal"
    );
    let cross_xy = [
        x_vec[1] * y_vec[2] - x_vec[2] * y_vec[1],
        x_vec[2] * y_vec[0] - x_vec[0] * y_vec[2],
        x_vec[0] * y_vec[1] - x_vec[1] * y_vec[0],
    ];
    // a left-handed basis (z = y x x) is the same cylinder; only
    // the winding of the (u, v) grid against the radial normal turns over.
    let left_handed = dot(cross_xy, z_vec) < -(1. - 1e-7);
    ensure!(
        left_handed || dot(cross_xy, z_vec) > 1. - 1e-7,
        "cylinder basis not orthonormal"
    );
    let envelope = surface.fields["m_Envelope"]["m_corners"]
        .as_array()
        .context("cylinder envelope")?;
    ensure!(envelope.len() == 2, "cylinder envelope bounds");
    let u_range = [
        envelope[0][0].as_f64().context("u min")?,
        envelope[1][0].as_f64().context("u max")?,
    ];
    let v_range = [
        envelope[0][1].as_f64().context("v min")?,
        envelope[1][1].as_f64().context("v max")?,
    ];
    ensure!(
        u_range.iter().chain(v_range.iter()).all(|v| v.is_finite()),
        "nonfinite cylinder envelope"
    );
    ensure!(
        u_range[1] > u_range[0]
            && u_range[1] - u_range[0] <= std::f64::consts::TAU + 1e-7
            && v_range[1] > v_range[0],
        "invalid cylinder envelope or seam crossing"
    );
    let loop_index = index
        .resolve(fi, &face.fields["m_pFirstLoop"])?
        .context("cylinder trim loop")?;
    // Every trim loop of the face (an outer boundary and its holes).
    let mut loops = Vec::new();
    let mut seen_loops = BTreeSet::new();
    let mut li = loop_index;
    loop {
        ensure!(seen_loops.insert(li), "loop-list cycle");
        loops.push(ring(g, &index, fi, li)?);
        match index.resolve(li, &g.objects[li].fields["m_nextLoop"])? {
            Some(next) => li = next,
            None => break,
        }
    }
    let trim_uv = loops[0].clone();
    ensure!(!trim_uv.is_empty(), "empty cylinder trim");
    let mut trim_min = [f64::INFINITY; 2];
    let mut trim_max = [f64::NEG_INFINITY; 2];
    for uv in loops.iter().flatten() {
        for k in 0..2 {
            trim_min[k] = trim_min[k].min(uv[k]);
            trim_max[k] = trim_max[k].max(uv[k]);
        }
    }
    ensure!(
        (trim_min[0] - trim_max[0]).abs() > 1e-12 && (trim_min[1] - trim_max[1]).abs() > 1e-12,
        "degenerate cylinder trim"
    );
    let tol = 1e-7;
    for uv in loops.iter().flatten() {
        ensure!(
            uv[0] >= u_range[0] - tol
                && uv[0] <= u_range[1] + tol
                && uv[1] >= v_range[0] - tol
                && uv[1] <= v_range[1] + tol,
            "cylinder trim outside envelope"
        );
    }
    // A single rectangular trim keeps its regular grid; any other trim
    // (curved cuts, notches, holes) is tessellated in UV strips.
    let rectangular = loops.len() == 1
        && (|| -> Result<()> {
            ensure!(trim_uv.len() >= 4, "nonrectangular cylinder trim");
            let mut covered = [0.; 4];
            let mut perimeter = 0.;
            for (a, b) in trim_uv
                .iter()
                .zip(trim_uv.iter().cycle().skip(1))
                .take(trim_uv.len())
            {
                ensure!(
                    a[0] >= u_range[0] - tol
                        && a[0] <= u_range[1] + tol
                        && a[1] >= v_range[0] - tol
                        && a[1] <= v_range[1] + tol
                        && b[0] >= u_range[0] - tol
                        && b[0] <= u_range[1] + tol
                        && b[1] >= v_range[0] - tol
                        && b[1] <= v_range[1] + tol,
                    "cylinder trim outside envelope"
                );
                let du = b[0] - a[0];
                let dv = b[1] - a[1];
                let length = du.hypot(dv);
                ensure!(length > tol, "repeated cylinder trim vertex");
                ensure!(
                    du.abs() < tol || dv.abs() < tol,
                    "nonrectangular cylinder trim"
                );
                let side = if dv.abs() < tol && (a[1] - trim_min[1]).abs() < tol {
                    0
                } else if du.abs() < tol && (a[0] - trim_max[0]).abs() < tol {
                    1
                } else if dv.abs() < tol && (a[1] - trim_max[1]).abs() < tol {
                    2
                } else if du.abs() < tol && (a[0] - trim_min[0]).abs() < tol {
                    3
                } else {
                    bail!("nonrectangular cylinder trim")
                };
                covered[side] += length;
                perimeter += length;
            }
            let expected_perimeter =
                2. * ((trim_max[0] - trim_min[0]) + (trim_max[1] - trim_min[1]));
            ensure!(
                (perimeter - expected_perimeter).abs() <= tol * expected_perimeter.max(1.),
                "incomplete or self-intersecting cylinder trim"
            );
            ensure!(
                (covered[0] - (trim_max[0] - trim_min[0])).abs() <= tol
                    && (covered[1] - (trim_max[1] - trim_min[1])).abs() <= tol
                    && (covered[2] - (trim_max[0] - trim_min[0])).abs() <= tol
                    && (covered[3] - (trim_max[1] - trim_min[1])).abs() <= tol,
                "incomplete cylinder trim perimeter"
            );
            Ok(())
        })()
        .is_ok();
    // Keep a full-turn trim from collapsing to coincident seam vertices when a
    // deliberately coarse profile is supplied. This is a geometric validity
    // bound, independent of the analytic chord-error policy.
    let sagitta_ratio = (profile.chord_error / (2. * radius)).min(1.);
    let angular_step = (4. * sagitta_ratio.sqrt().asin()).min(std::f64::consts::FRAC_PI_2);
    ensure!(
        angular_step.is_finite() && angular_step > 0.,
        "cylinder tessellation precision underflow"
    );
    let orient = surface.fields["m_orientFlag"]
        .as_bool()
        .context("cylinder orientation")?
        ^ (face.fields["m_faceFlags_v9"]
            .as_u64()
            .context("face flags")?
            & 2
            != 0);
    let eval = |u: f64, v: f64| {
        let (s, c) = u.sin_cos();
        [
            center[0] + radius * (c * x_vec[0] + s * y_vec[0]) + v * z_vec[0],
            center[1] + radius * (c * x_vec[1] + s * y_vec[1]) + v * z_vec[1],
            center[2] + radius * (c * x_vec[2] + s * y_vec[2]) + v * z_vec[2],
        ]
    };
    let radial = |u: f64| {
        let (s, c) = u.sin_cos();
        let n = [
            c * x_vec[0] + s * y_vec[0],
            c * x_vec[1] + s * y_vec[1],
            c * x_vec[2] + s * y_vec[2],
        ];
        if orient { n } else { [-n[0], -n[1], -n[2]] }
    };
    // Counter-clockwise (u, v) triangles face along +radial on a
    // right-handed basis; a left-handed one turns them over.
    let keep_winding = orient ^ left_handed;
    let u_values = tessellation_axis(
        trim_min[0],
        trim_max[0],
        angular_step,
        loops.iter().flatten().map(|p| p[0]),
    )
    .context("cylinder tessellation grid too large")?;
    const MAX_GRID_VERTICES: usize = 2_000_000;
    let mut vertices = Vec::new();
    let mut normals = Vec::new();
    let mut triangles = Vec::new();
    if rectangular {
        let v_values = tessellation_axis(
            trim_min[1],
            trim_max[1],
            profile.max_v_edge,
            trim_uv.iter().map(|p| p[1]),
        )
        .context("cylinder tessellation grid too large")?;
        ensure!(
            u_values.len() <= MAX_GRID_VERTICES && v_values.len() <= MAX_GRID_VERTICES,
            "cylinder tessellation grid too large"
        );
        let nu = u_values.len() - 1;
        let nv = v_values.len() - 1;
        let grid_vertices = (nu + 1)
            .checked_mul(nv + 1)
            .context("cylinder vertex count overflow")?;
        let grid_triangles = nu
            .checked_mul(nv)
            .and_then(|v| v.checked_mul(2))
            .context("cylinder triangle count overflow")?;
        ensure!(
            grid_vertices <= MAX_GRID_VERTICES && grid_triangles <= MAX_GRID_VERTICES * 2,
            "cylinder tessellation grid too large"
        );
        for &v in v_values.iter().take(nv + 1) {
            for &u in u_values.iter().take(nu + 1) {
                vertices.push(eval(u, v));
                normals.push(radial(u));
            }
        }
        let row = nu + 1;
        for j in 0..nv {
            for i in 0..nu {
                let a = (j * row + i) as u32;
                let b = a + 1;
                let c = a + row as u32;
                let d = c + 1;
                if keep_winding {
                    triangles.extend([[a, b, d], [a, d, c]]);
                } else {
                    triangles.extend([[a, d, b], [a, c, d]]);
                }
            }
        }
    } else {
        let (uv, tris) = uv_strip_triangles(&loops, &u_values, profile.max_v_edge)?;
        ensure!(
            uv.len() <= MAX_GRID_VERTICES,
            "cylinder tessellation grid too large"
        );
        for p in &uv {
            vertices.push(eval(p[0], p[1]));
            normals.push(radial(p[0]));
        }
        for t in tris {
            triangles.push(if keep_winding { t } else { [t[0], t[2], t[1]] });
        }
        ensure!(!triangles.is_empty(), "empty cylinder trim");
    }
    Ok(FaceMesh {
        face_index: fi,
        face_tag: face.fields["m_GInfo"]["m_tag"].as_i64().unwrap_or(-1),
        render_style_id: crate::native_metadata::identifier(&face.fields["m_renderStyleId"])?,
        vertices,
        triangles,
        normal: normals[0],
        normals,
        analytic_surface: Some(AnalyticCylinder {
            center,
            radius,
            x_vec,
            y_vec,
            z_vec,
            orient_flag: orient,
            u_range,
            v_range,
        }),
        trim_uv,
    })
}
/// Tessellate one Face on a `ConeSurf`. The surface is
/// `center + v (cos a z + sin a (cos u x + sin u y))`, `a` = `m_halfAngle`:
/// `v` is the slant distance from the apex `m_center` (measured on rme_basic
/// 2024 pipe transitions: at `v` 0.24191 the radius `v sin a` is 0.052083 ft,
/// the radius the adjacent plane's edge samples give). Any trim, any loops.
pub fn cone_face(
    g: &ObjectGraph,
    fi: usize,
    profile: CylinderTessellationProfile,
) -> Result<FaceMesh> {
    let index = PointerIndex::new(g);
    let face = g.objects.get(fi).context("face index")?;
    ensure!(face.class_name == "Face", "not Face");
    ensure!(
        face.fields["m_faceRegions"]
            .as_array()
            .is_some_and(|items| items.is_empty()),
        "face regions need subdivision"
    );
    let surface_index = index
        .resolve(fi, &face.fields["m_pSurf"])?
        .context("face surface")?;
    let surface = &g.objects[surface_index];
    ensure!(surface.class_name == "ConeSurf", "not a cone");
    let center = point::<3>(&surface.fields["m_center"])?;
    let x = point::<3>(&surface.fields["m_xVec"])?;
    let y = point::<3>(&surface.fields["m_yVec"])?;
    let z = point::<3>(&surface.fields["m_zVec"])?;
    let half = surface.fields["m_halfAngle"]
        .as_f64()
        .context("cone half angle")?;
    ensure!(
        half.is_finite() && half > 0. && half < std::f64::consts::FRAC_PI_2,
        "invalid cone half angle"
    );
    let dot = |a: [f64; 3], b: [f64; 3]| a.iter().zip(b).map(|(p, q)| p * q).sum::<f64>();
    let cross = [
        x[1] * y[2] - x[2] * y[1],
        x[2] * y[0] - x[0] * y[2],
        x[0] * y[1] - x[1] * y[0],
    ];
    let left_handed = dot(cross, z) < 0.;
    ensure!(
        (dot(cross, z).abs() - 1.).abs() < 1e-6 && dot(x, y).abs() < 1e-6,
        "cone basis not orthonormal"
    );
    let orient = surface.fields["m_orientFlag"]
        .as_bool()
        .context("cone orientation")?
        ^ (face.fields["m_faceFlags_v9"]
            .as_u64()
            .context("face flags")?
            & 2
            != 0);
    let loops = face_loops_uv(g, fi)?;
    let (sa, ca) = half.sin_cos();
    let v_max = loops
        .iter()
        .flatten()
        .map(|p| p[1].abs())
        .fold(0., f64::max);
    let radius = (v_max * sa).max(1e-6);
    let step = (4. * (profile.chord_error / (2. * radius)).min(1.).sqrt().asin())
        .min(std::f64::consts::FRAC_PI_2);
    let (u_lo, u_hi) = loops
        .iter()
        .flatten()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| {
            (a.min(p[0]), b.max(p[0]))
        });
    let u_grid = tessellation_axis(u_lo, u_hi, step, std::iter::empty())?;
    let (uv, tris) = uv_strip_triangles(&loops, &u_grid, profile.max_v_edge)?;
    ensure!(uv.len() <= 2_000_000, "cone tessellation too large");
    let ruling = |u: f64| {
        let (s, c) = u.sin_cos();
        let r: [f64; 3] = std::array::from_fn(|k| c * x[k] + s * y[k]);
        r
    };
    let mut vertices = Vec::with_capacity(uv.len());
    let mut normals = Vec::with_capacity(uv.len());
    for p in &uv {
        let r = ruling(p[0]);
        vertices.push(std::array::from_fn(|k| {
            center[k] + p[1] * (ca * z[k] + sa * r[k])
        }));
        let n: [f64; 3] = std::array::from_fn(|k| ca * r[k] - sa * z[k]);
        normals.push(if orient { n } else { n.map(|c| -c) });
    }
    let keep_winding = orient ^ left_handed;
    let mut triangles = Vec::with_capacity(tris.len());
    for t in tris {
        let [a, b, c] = t.map(|i| vertices[i as usize]);
        let u: [f64; 3] = std::array::from_fn(|k| b[k] - a[k]);
        let w: [f64; 3] = std::array::from_fn(|k| c[k] - a[k]);
        let n = [
            u[1] * w[2] - u[2] * w[1],
            u[2] * w[0] - u[0] * w[2],
            u[0] * w[1] - u[1] * w[0],
        ];
        if dot(n, n) > 1e-24 {
            triangles.push(if keep_winding { t } else { [t[0], t[2], t[1]] });
        }
    }
    ensure!(!triangles.is_empty(), "empty cone trim");
    Ok(FaceMesh {
        face_index: fi,
        face_tag: face.fields["m_GInfo"]["m_tag"].as_i64().unwrap_or(-1),
        render_style_id: crate::native_metadata::identifier(&face.fields["m_renderStyleId"])?,
        normal: normals[0],
        vertices,
        normals,
        triangles,
        analytic_surface: None,
        trim_uv: loops[0].clone(),
    })
}
/// UV points and their triangles.
pub(crate) type UvTriangles = (Vec<[f64; 2]>, Vec<[u32; 3]>);
/// Triangles covering the region the closed UV `loops` bound (even-odd: an
/// outer boundary and its holes, in any orientation), counter-clockwise in
/// (u, v). The region is cut into strips at every loop vertex's u and at
/// `u_grid`; inside a strip each crossing edge is a straight segment, and
/// the area between the 1st and 2nd, 3rd and 4th... of them is a trapezoid,
/// split along v where it is taller than `max_v`. The u grid
/// bounds the chord error on a curved surface.
pub(crate) fn uv_strip_triangles(
    loops: &[Vec<[f64; 2]>],
    u_grid: &[f64],
    max_v: f64,
) -> Result<UvTriangles> {
    ensure!(max_v.is_finite() && max_v > 0., "invalid v edge");
    let mut breaks: Vec<f64> = loops.iter().flatten().map(|p| p[0]).collect();
    let (lo, hi) = breaks
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &u| {
            (a.min(u), b.max(u))
        });
    breaks.extend(u_grid.iter().copied().filter(|u| *u > lo && *u < hi));
    breaks.sort_by(f64::total_cmp);
    breaks.dedup_by(|a, b| (*a - *b).abs() <= 1e-12);
    ensure!(breaks.len() >= 2, "degenerate trim");
    let edges: Vec<([f64; 2], [f64; 2])> = loops
        .iter()
        .flat_map(|l| l.iter().zip(l.iter().cycle().skip(1)).take(l.len()))
        .map(|(a, b)| if a[0] <= b[0] { (*a, *b) } else { (*b, *a) })
        .filter(|(a, b)| b[0] - a[0] > 1e-12)
        .collect();
    let at =
        |(a, b): &([f64; 2], [f64; 2]), u: f64| a[1] + (b[1] - a[1]) * (u - a[0]) / (b[0] - a[0]);
    let mut uv = Vec::new();
    let mut tris = Vec::new();
    for w in breaks.windows(2) {
        let (u0, u1) = (w[0], w[1]);
        let um = 0.5 * (u0 + u1);
        let mut cross: Vec<(f64, f64, f64)> = edges
            .iter()
            .filter(|(a, b)| a[0] < um && b[0] > um)
            .map(|e| (at(e, um), at(e, u0), at(e, u1)))
            .collect();
        ensure!(cross.len().is_multiple_of(2), "open trim loop");
        cross.sort_by(|a, b| a.0.total_cmp(&b.0));
        for pair in cross.chunks(2) {
            let ((_, b0, b1), (_, t0, t1)) = (pair[0], pair[1]);
            let n = (((t0 - b0).max(t1 - b1)) / max_v).ceil().max(1.) as usize;
            ensure!(
                tris.len() + 2 * n <= 4_000_000,
                "trim tessellation too large"
            );
            for k in 0..n {
                let f0 = k as f64 / n as f64;
                let f1 = (k + 1) as f64 / n as f64;
                let base = uv.len() as u32;
                uv.extend([
                    [u0, b0 + (t0 - b0) * f0],
                    [u1, b1 + (t1 - b1) * f0],
                    [u1, b1 + (t1 - b1) * f1],
                    [u0, b0 + (t0 - b0) * f1],
                ]);
                let q = |i: u32| uv[(base + i) as usize];
                let area = |a: [f64; 2], b: [f64; 2], c: [f64; 2]| {
                    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
                };
                for t in [[0, 1, 2], [0, 2, 3]] {
                    if area(q(t[0]), q(t[1]), q(t[2])) > 1e-18 {
                        tris.push([base + t[0], base + t[1], base + t[2]]);
                    }
                }
            }
        }
    }
    Ok((uv, tris))
}
#[cfg(test)]
mod uv_strip_tests {
    use super::uv_strip_triangles;
    fn area(uv: &[[f64; 2]], tris: &[[u32; 3]]) -> f64 {
        tris.iter()
            .map(|t| {
                let (a, b, c) = (uv[t[0] as usize], uv[t[1] as usize], uv[t[2] as usize]);
                0.5 * ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]))
            })
            .sum()
    }
    #[test]
    fn square_with_square_hole_has_the_ring_area_all_counter_clockwise() {
        let outer = vec![[0., 0.], [4., 0.], [4., 4.], [0., 4.]];
        let hole = vec![[1., 1.], [1., 3.], [3., 3.], [3., 1.]];
        let (uv, tris) = uv_strip_triangles(&[outer, hole], &[0.5, 2.0, 3.5], 10.).unwrap();
        assert!((area(&uv, &tris) - 12.).abs() < 1e-9);
        assert!(tris.iter().all(|t| area(&uv, &[*t]) > 0.));
    }
    #[test]
    fn slanted_cut_triangle_and_v_split() {
        let tri = vec![[0., 0.], [2., 0.], [0., 2.]];
        let (uv, tris) = uv_strip_triangles(&[tri], &[1.0], 0.25).unwrap();
        assert!((area(&uv, &tris) - 2.).abs() < 1e-9);
        assert!(tris.len() > 4);
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Primitive {
    pub source_owner_id: Option<u64>,
    pub object_index: usize,
    pub face_tag: i64,
    pub render_style_id: i64,
    pub material_id: Option<i64>,
    pub vertices: Vec<[f64; 3]>,
    pub normals: Vec<[f64; 3]>,
    pub triangles: Vec<[u32; 3]>,
    /// when set, `vertices` and `normals` are still in the frame
    /// of the graph they were read from (a family symbol's) and this places
    /// them in the document; when `None` they are in the document frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<crate::native_graphics_traversal::GraphicsTransform>,
    /// Whether a branch flagged not visible lies on the way to it. Only a
    /// decode that includes hidden branches ([`graphics_including_hidden`])
    /// holds any.
    #[serde(default, skip_serializing_if = "is_false")]
    pub hidden: bool,
    /// Whether a filter that rejects the context lies on the way to it. Only
    /// a decode that includes filtered branches ([`graphics_including_filtered`])
    /// holds any.
    #[serde(default, skip_serializing_if = "is_false")]
    pub filtered: bool,
}
fn is_false(value: &bool) -> bool {
    !*value
}
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct GraphicsMeshes {
    pub primitives: Vec<Primitive>,
    pub diagnostics: Vec<String>,
    pub unbounded_faces: usize,
    /// Boundary-empty cutout records recognized by an observed saved profile.
    /// Their source records remain in the native graph; no surface is invented.
    #[serde(default)]
    pub empty_trim_faces: usize,
    pub rejected_filters: usize,
    #[serde(default)]
    pub excluded_visibility_branches: usize,
    #[serde(default)]
    pub excluded_non_surface_branches: usize,
    #[serde(default)]
    pub profile_observations: Vec<String>,
}
/// Decode selected saved graphics. Positions and normals are in the owning
/// document frame; length units remain Revit internal feet.
pub fn graphics(g: &ObjectGraph) -> GraphicsMeshes {
    graphics_with_resolver(g, &|_| None)
}
pub fn graphics_with_resolver<'a>(
    g: &'a ObjectGraph,
    resolver: &dyn Fn(u64) -> Option<&'a ObjectGraph>,
) -> GraphicsMeshes {
    graphics_with_resolver_at_detail(g, resolver, 3)
}
/// Decode the explicit saved3D detail representation (native levels1,2,3).
pub fn graphics_with_resolver_at_detail<'a>(
    g: &'a ObjectGraph,
    resolver: &dyn Fn(u64) -> Option<&'a ObjectGraph>,
    detail_level: i64,
) -> GraphicsMeshes {
    graphics_impl(g, resolver, detail_level, false, false, false)
}
/// [`graphics_with_resolver_at_detail`], with `keep_frame` leaving
/// each primitive in its source graph's frame ([`Primitive::frame`]).
pub fn graphics_framed<'a>(
    g: &'a ObjectGraph,
    resolver: &dyn Fn(u64) -> Option<&'a ObjectGraph>,
    detail_level: i64,
    keep_frame: bool,
) -> GraphicsMeshes {
    graphics_impl(g, resolver, detail_level, false, false, keep_frame)
}
/// [`graphics_with_resolver_at_detail`], also decoding the branches whose
/// visibility flags hide them; each [`Primitive`] one lies on the way to is
/// marked [`Primitive::hidden`].
pub fn graphics_including_hidden<'a>(
    g: &'a ObjectGraph,
    resolver: &dyn Fn(u64) -> Option<&'a ObjectGraph>,
    detail_level: i64,
) -> GraphicsMeshes {
    graphics_impl(g, resolver, detail_level, true, false, false)
}
/// [`graphics_including_hidden`], also decoding the children of a filter
/// whose conditions reject the context or are not read; each [`Primitive`]
/// one lies on the way to is marked [`Primitive::filtered`].
pub fn graphics_including_filtered<'a>(
    g: &'a ObjectGraph,
    resolver: &dyn Fn(u64) -> Option<&'a ObjectGraph>,
    detail_level: i64,
) -> GraphicsMeshes {
    graphics_impl(g, resolver, detail_level, true, true, false)
}
fn graphics_impl<'a>(
    g: &'a ObjectGraph,
    resolver: &dyn Fn(u64) -> Option<&'a ObjectGraph>,
    detail_level: i64,
    include_hidden: bool,
    include_filtered: bool,
    keep_frame: bool,
) -> GraphicsMeshes {
    let walk = |keep_light_sources: bool| {
        graphics_from(
            g,
            resolver,
            crate::native_graphics_traversal::select_graphics_impl(
                g,
                resolver,
                detail_level,
                include_hidden,
                include_filtered,
                keep_light_sources,
            ),
            keep_frame,
        )
    };
    let (out, light_source_filters) = walk(false);
    // A light-source filter that is all an element draws is its body, not a
    // light beside it: Snowdon Towers' 17 Honey-Locust trees draw nothing but
    // such a filter, a `GImposter` with the tree's mesh.
    if out.primitives.is_empty() && light_source_filters > 0 {
        return walk(true).0;
    }
    out
}
/// The meshes of `selection`, and how many light-source filters it left out.
fn graphics_from<'a>(
    g: &'a ObjectGraph,
    resolver: &dyn Fn(u64) -> Option<&'a ObjectGraph>,
    selection: crate::native_graphics_traversal::GraphicsSelection,
    keep_frame: bool,
) -> (GraphicsMeshes, usize) {
    let light_source_filters = selection.light_source_filters;
    let mut out = GraphicsMeshes {
        diagnostics: selection
            .diagnostics
            .iter()
            .map(|d| format!("object {}: {}", d.object_index, d.message))
            .collect(),
        rejected_filters: selection.rejected_filters,
        excluded_visibility_branches: selection.excluded_visibility_branches,
        excluded_non_surface_branches: selection.excluded_non_surface_branches,
        profile_observations: selection.profile_observations,
        ..Default::default()
    };
    let mut pointer_indices = BTreeMap::new();
    for selected in selection.selected {
        let g = match selected.source_owner_id {
            None => g,
            Some(id) => match resolver(id) {
                Some(graph) => graph,
                None => {
                    out.diagnostics
                        .push(format!("resolved graphics owner {id} disappeared"));
                    continue;
                }
            },
        };
        let index = pointer_indices
            .entry(selected.source_owner_id)
            .or_insert_with(|| PointerIndex::new(g));
        let oi = selected.object_index;
        let obj = &g.objects[oi];
        let mut decoded = Vec::new();
        let result = (|| -> Result<()> {
            if obj.class_name == "Geometry" {
                for ptr in obj.fields["m_pFaces"]
                    .as_array()
                    .context("geometry faces")?
                {
                    let fi = index.resolve(oi, ptr)?.context("null geometry face")?;
                    match face_indexed(g, index, fi)? {
                        Some(m) => {
                            let normals = if m.normals.len() == m.vertices.len() {
                                m.normals.clone()
                            } else {
                                vec![m.normal; m.vertices.len()]
                            };
                            decoded.push(Primitive {
                                source_owner_id: selected.source_owner_id,
                                object_index: oi,
                                face_tag: m.face_tag,
                                render_style_id: m.render_style_id,
                                material_id: None,
                                vertices: m.vertices,
                                normals,
                                triangles: m.triangles,
                                frame: None,
                                hidden: selected.hidden,
                                filtered: selected.filtered,
                            })
                        }
                        None => {
                            if crate::native_empty_faces::is_observed_empty_trim_face(g, fi) {
                                out.empty_trim_faces += 1;
                            } else {
                                out.unbounded_faces += 1;
                            }
                        }
                    }
                }
            } else if obj.class_name == "GPolyMesh" {
                let ti = index
                    .resolve(oi, &obj.fields["m_pFacetedTopology"])?
                    .context("faceted topology")?;
                let topology = &g.objects[ti];
                // FacetedTopology0 (ordinary meshes) and FacetedTopology8 (RPC
                ensure!(
                    topology.class_name.starts_with("FacetedTopology"),
                    "unsupported faceted topology"
                );
                let f = &topology.fields;
                let vertices = f["m_pointsArr"]
                    .as_array()
                    .context("mesh points")?
                    .iter()
                    .map(point::<3>)
                    .collect::<Result<Vec<_>>>()?;
                let normals = match f["m_normalsArr"].as_array() {
                    Some(a) => a.iter().map(point::<3>).collect::<Result<Vec<_>>>()?,
                    None => Vec::new(),
                };
                // Binding 0 is the ordinary mesh's; binding 2 is one normal
                // per facet, which every RPC mesh (FacetedTopology8) of
                // rac_basic 2024 and Technical School 2026 carries
                // (normals = facets on all of them). Without accepting
                // it no RPC tree, person or car decoded.
                ensure!(
                    normals.is_empty() || matches!(f["m_normalsFlag"].as_u64(), Some(0 | 2)),
                    "unsupported normal binding"
                );
                let triangles = f["m_facetsArr"]
                    .as_array()
                    .context("mesh facets")?
                    .iter()
                    .map(|v| -> Result<[u32; 3]> {
                        let a = v.as_array().context("facet array")?;
                        ensure!(a.len() == 3, "nontriangle facet");
                        let mut t = [0; 3];
                        for i in 0..3 {
                            t[i] = u32::try_from(a[i].as_u64().context("facet index")?)?;
                            ensure!((t[i] as usize) < vertices.len(), "facet index bounds")
                        }
                        Ok(t)
                    })
                    .collect::<Result<Vec<_>>>()?;
                // Normals that do not match the facets are left out; the
                // facet normal is computed from the triangle.
                let normals = if normals.len() == triangles.len() {
                    normals
                } else {
                    Vec::new()
                };
                let facet_normals = !normals.is_empty();

                let mut expanded_vertices = Vec::new();
                let mut expanded_normals = Vec::new();
                let mut expanded_triangles = Vec::new();
                for (i, t) in triangles.iter().enumerate() {
                    let base = expanded_vertices.len() as u32;
                    let normal = if facet_normals {
                        normals[i]
                    } else {
                        let a = vertices[t[0] as usize];
                        let b = vertices[t[1] as usize];
                        let c = vertices[t[2] as usize];
                        let u: [f64; 3] = std::array::from_fn(|k| b[k] - a[k]);
                        let v: [f64; 3] = std::array::from_fn(|k| c[k] - a[k]);
                        [
                            u[1] * v[2] - u[2] * v[1],
                            u[2] * v[0] - u[0] * v[2],
                            u[0] * v[1] - u[1] * v[0],
                        ]
                    };
                    for idx in t {
                        expanded_vertices.push(vertices[*idx as usize]);
                        expanded_normals.push(normal);
                    }
                    expanded_triangles.push([base, base + 1, base + 2]);
                }
                let (vertices, normals, triangles) =
                    (expanded_vertices, expanded_normals, expanded_triangles);
                decoded.push(Primitive {
                    source_owner_id: selected.source_owner_id,
                    object_index: oi,
                    face_tag: obj.fields["m_GInfo"]["m_tag"].as_i64().unwrap_or(-1),
                    render_style_id: crate::native_metadata::identifier(
                        &obj.fields["m_interiorGStyleID"],
                    )
                    .context("polymesh style")?,
                    material_id: Some(
                        crate::native_metadata::identifier(&obj.fields["m_materialID"])
                            .context("polymesh material")?,
                    )
                    .filter(|v| *v >= 0),
                    vertices,
                    normals,
                    triangles,
                    frame: None,
                    hidden: selected.hidden,
                    filtered: selected.filtered,
                })
            }
            for mesh in &mut decoded {
                if keep_frame {
                    mesh.frame = Some(selected.world_transform);
                } else {
                    transform(mesh, selected.world_transform)?
                }
            }
            Ok(())
        })();
        match result {
            Ok(()) => out.primitives.extend(decoded),
            Err(e) => out.diagnostics.push(format!("object {oi}: {e:#}")),
        }
    }
    (out, light_source_filters)
}
/// Place `p` in the document by `m` (positions, normals and, for a mirroring
/// `m`, the winding).
pub fn transform(p: &mut Primitive, m: [[f64; 4]; 4]) -> Result<()> {
    let a = [m[0][0], m[1][0], m[2][0]];
    let b = [m[0][1], m[1][1], m[2][1]];
    let c = [m[0][2], m[1][2], m[2][2]];
    let cp = |a: [f64; 3], b: [f64; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let bc = cp(b, c);
    let ca = cp(c, a);
    let ab = cp(a, b);
    let det = (0..3).map(|i| a[i] * bc[i]).sum::<f64>();
    ensure!(
        det.is_finite() && det.abs() > 1e-12,
        "singular graphics transform"
    );
    for v in &mut p.vertices {
        let old = *v;
        *v = std::array::from_fn(|i| m[i][3] + (0..3).map(|j| m[i][j] * old[j]).sum::<f64>());
        ensure!(
            v.iter().all(|x| x.is_finite()),
            "nonfinite transformed vertex"
        )
    }
    for n in &mut p.normals {
        let old = *n;
        *n = std::array::from_fn(|i| (bc[i] * old[0] + ca[i] * old[1] + ab[i] * old[2]) / det);
        let len = n.iter().map(|v| v * v).sum::<f64>().sqrt();
        ensure!(len > 1e-12 && len.is_finite(), "invalid transformed normal");
        for v in n {
            *v /= len
        }
    }
    if det < 0. {
        for t in &mut p.triangles {
            t.swap(1, 2)
        }
    }
    Ok(())
}

#[cfg(test)]
mod s379_tests {
    use super::convex_trim_columns;

    #[test]
    fn a_mitred_trim_spans_between_its_sloped_ends() {
        // A wall sweep's cylinder face: u across the arc, v along the sweep,
        // both ends mitred.
        let trim = [[0.0, 0.0], [1.0, 1.0], [1.0, 9.0], [0.0, 10.0]];
        let span = convex_trim_columns(trim.to_vec(), 1e-9).expect("convex");
        assert_eq!(span(0.0), (0.0, 10.0));
        assert_eq!(span(0.5), (0.5, 9.5));
        assert_eq!(span(1.0), (1.0, 9.0));
        let notched = [[0.0, 0.0], [2.0, 0.0], [1.0, 1.0], [2.0, 2.0], [0.0, 2.0]];
        assert!(convex_trim_columns(notched.to_vec(), 1e-9).is_none());
    }
}
