//! Bounded UV-grid meshing for decoded parametric surfaces.

use crate::native_parameters::ObjectGraph;
use crate::native_parametric_surface::ParametricSurface;
use crate::native_saved_mesh::FaceMesh;
use crate::vector_math::distance;
use anyhow::{Context, Result, ensure};

#[derive(Debug, Clone)]
pub struct ParametricMesh {
    pub vertices: Vec<[f64; 3]>,
    pub normals: Vec<[f64; 3]>,
    pub triangles: Vec<[u32; 3]>,
}

fn uv_point(v: &crate::graph_value::Fv) -> Result<[f64; 2]> {
    let a = v.as_array().context("UV array")?;
    ensure!(a.len() == 2, "UV dimension");
    let p = [
        a[0].as_f64().context("UV number")?,
        a[1].as_f64().context("UV number")?,
    ];
    ensure!(p.iter().all(|x| x.is_finite()), "nonfinite UV");
    Ok(p)
}

pub(crate) struct FaceGrid {
    pub surface: ParametricSurface,
    pub u: Vec<f64>,
    pub v: Vec<f64>,
    /// `v` without the trim's own values: the equal steps alone.
    pub v_lattice: Vec<f64>,
    pub trim: Vec<[f64; 2]>,
    pub bounds: [[f64; 2]; 2],
    /// Every trim loop in UV when the trim is not one rectangle;
    /// `None` for a rectangular trim, tessellated as a grid.
    pub loops: Option<Vec<Vec<[f64; 2]>>>,
    /// The edges, by object index, that this face and the plane beside it both draw
    /// straight between the edge's saved points (`keeps_saved_chords`).
    pub chord_edges: std::collections::BTreeSet<usize>,
    /// Each span between two saved points of those edges, in the surface's parameters.
    pub chords: Vec<[[f64; 2]; 2]>,
    /// Points `[u, v]` a trimmed face's strip sides at that `u` hold besides its own: those
    /// its neighbours draw on the edges they share (`seam_group`).
    pub at_u_extra: Vec<[f64; 2]>,
}

/// Whether a face on `surface` and a plane draw an edge they share, one that crosses both
/// of the surface's parameters, straight between the edge's saved points, so that the two
/// meet. Each drawing it its own way parts them by the edge's bow, which is what Revit's
/// export of the BIM4LCA timber office's flights and ramps does not: its plane's vertices
/// lie on those lines. A ruled face between plain curves is left out.
fn keeps_saved_chords(surface: &ParametricSurface) -> bool {
    use crate::native_parametric_surface::Curve;
    match surface {
        ParametricSurface::RuledSurf {
            profile1, profile2, ..
        } => matches!(profile1, Curve::Hermite { .. }) || matches!(profile2, Curve::Hermite { .. }),
        ParametricSurface::SurfRev { .. } | ParametricSurface::ConeSurf { .. } => true,
    }
}

/// Whether `surface` is a cone whose x, y and z axes are left-handed. `cone_face` turns
/// such a cone's triangles to keep them outward; this mesher does not, so a cone like that
/// is left to `cone_face` and its plane keeps its saved points.
pub(crate) fn left_handed_cone(surface: &ParametricSurface) -> bool {
    match surface {
        ParametricSurface::ConeSurf { x, y, z, .. } => {
            let cross = [
                x[1] * y[2] - x[2] * y[1],
                x[2] * y[0] - x[0] * y[2],
                x[0] * y[1] - x[1] * y[0],
            ];
            cross[0] * z[0] + cross[1] * z[1] + cross[2] * z[2] < 0.
        }
        _ => false,
    }
}

/// A span between two saved points, in a surface's parameters.
pub(crate) type Chord = [[f64; 2]; 2];

/// The edges that are chords and their spans.
type ChordSpans = (std::collections::BTreeSet<usize>, Vec<Chord>);

/// Points, triangles and beside each point the chord whose span it lies on.
type StripMesh = (Vec<[f64; 2]>, Vec<[u32; 3]>, Vec<Option<Chord>>);

/// The edges of every loop of face `fi` that cross both of the surface's parameters and
/// have a plane for their other face, and their spans between saved points in `fi`'s
/// parameters.
fn chord_spans(g: &ObjectGraph, fi: usize) -> Result<ChordSpans> {
    use crate::native_saved_mesh::pointer;
    let mut edges = std::collections::BTreeSet::new();
    let mut chords = Vec::new();
    let f = g.objects.get(fi).context("face index")?;
    let mut next_loop = pointer(g, fi, &f.fields["m_pFirstLoop"])?;
    let mut seen_loops = std::collections::BTreeSet::new();
    while let Some(li) = next_loop {
        ensure!(seen_loops.insert(li), "loop-list cycle");
        let l = &g.objects[li];
        let mut cur = pointer(g, li, &l.fields["m_next"])?.context("empty trim")?;
        let mut seen = std::collections::BTreeSet::new();
        while cur != li {
            ensure!(seen.insert(cur), "trim edge cycle");
            let e = &g.objects[cur];
            let faces = e.fields["m_pFace"].as_array().context("edge face pair")?;
            ensure!(faces.len() == 2, "edge face pair length");
            let mut side = None;
            for (i, p) in faces.iter().enumerate() {
                if pointer(g, cur, p)? == Some(fi) {
                    side = Some(i);
                }
            }
            let side = side.context("edge does not belong to face")?;
            let ends = e.fields["m_firstAndLastEdgePnts"]
                .as_array()
                .context("edge endpoints")?;
            ensure!(ends.len() == 2, "edge endpoint count");
            let mut points = vec![uv_point(&ends[0]["uv"][side])?];
            for p in e.fields["m_interiorEdgePnts"]
                .as_array()
                .context("edge interior points")?
            {
                points.push(uv_point(&p["uv"][side])?);
            }
            points.push(uv_point(&ends[1]["uv"][side])?);
            let other_is_plane = pointer(g, cur, &faces[1 - side])
                .ok()
                .flatten()
                .and_then(|other| {
                    let surface = g.objects.get(other)?.fields.get("m_pSurf")?;
                    pointer(g, other, surface).ok().flatten()
                })
                .is_some_and(|si| g.objects.get(si).is_some_and(|s| s.class_name == "Plane"));
            if other_is_plane && crate::native_saved_mesh::isoparametric_axis(&points).is_none() {
                edges.insert(cur);
                chords.extend(points.windows(2).map(|w| [w[0], w[1]]));
            }
            cur = pointer(g, cur, &e.fields["m_next"][side])?.context("null trim next")?;
        }
        next_loop = pointer(g, li, &l.fields["m_nextLoop"])?;
    }
    Ok((edges, chords))
}

/// How near a point on a chord must lie to an end of the chord, feet, to be that end: a
/// triangle with a side so short cannot be told from the point, and the plane beside the
/// face would have to leave the point out of its boundary, as the face does by taking it.
pub(crate) const CHORD_SNAP_FEET: f64 = 1e-6;

/// The point at fraction `t` of the straight line between the surface's points at `a`
/// and `b`: where a chord's span lays a vertex of either face. A point within
/// `CHORD_SNAP_FEET` of either end is that end.
pub(crate) fn chord_point(
    surface: &ParametricSurface,
    a: [f64; 2],
    b: [f64; 2],
    t: f64,
) -> Result<[f64; 3]> {
    let e = surface.bounds();
    let at =
        |p: [f64; 2]| surface.evaluate(p[0].clamp(e[0][0], e[1][0]), p[1].clamp(e[0][1], e[1][1]));
    let (from, to) = (at(a)?, at(b)?);
    let p: [f64; 3] = std::array::from_fn(|k| from[k] + t * (to[k] - from[k]));
    let far = |q: [f64; 3]| (0..3).map(|k| (p[k] - q[k]).powi(2)).sum::<f64>().sqrt();
    Ok(if far(from) < CHORD_SNAP_FEET {
        from
    } else if far(to) < CHORD_SNAP_FEET {
        to
    } else {
        p
    })
}

// Only complete rectangular boundaries (including analytic collapsed poles)
// are accepted. Merely lying on the rectangle's perimeter is insufficient.
fn ruled_surface_component(
    g: &ObjectGraph,
    start_face: usize,
) -> Result<std::collections::BTreeSet<usize>> {
    // The saved graph does not consistently materialize Edge -> Face pointer
    // records in g.edges.  Resolve the authoritative m_pFace pair on each
    // Edge instead; pointer() also covers token-only references.  Malformed
    // disconnected edges are ignored while building this component index so
    // they cannot taint an otherwise resolvable ruled-surface component.
    let mut adjacency: std::collections::BTreeMap<usize, std::collections::BTreeSet<usize>> =
        std::collections::BTreeMap::new();
    let mut invalid_by_face = std::collections::BTreeSet::new();
    for (edge_index, edge) in g
        .objects
        .iter()
        .enumerate()
        .filter(|(_, object)| object.class_name == "Edge")
    {
        let Some(faces) = edge
            .fields
            .get("m_pFace")
            .and_then(|value| value.as_array())
        else {
            continue;
        };
        let resolved = faces
            .iter()
            .map(|value| {
                crate::native_saved_mesh::pointer(g, edge_index, value)
                    .ok()
                    .flatten()
            })
            .collect::<Vec<_>>();
        let valid_faces = resolved
            .iter()
            .filter_map(|target| *target)
            .filter(|target| {
                g.objects
                    .get(*target)
                    .is_some_and(|object| object.class_name == "Face")
            })
            .collect::<Vec<_>>();
        if faces.len() != 2 || valid_faces.len() != 2 {
            invalid_by_face.extend(valid_faces);
            continue;
        }
        let first = valid_faces[0];
        let second = valid_faces[1];
        adjacency.entry(first).or_default().insert(second);
        adjacency.entry(second).or_default().insert(first);
    }
    let mut component = std::collections::BTreeSet::from([start_face]);
    let mut queue = std::collections::VecDeque::from([start_face]);
    while let Some(face) = queue.pop_front() {
        for &other_face in adjacency.get(&face).into_iter().flatten() {
            if component.insert(other_face) {
                queue.push_back(other_face);
            }
        }
    }
    if let Some(face) = component.iter().find(|face| invalid_by_face.contains(face)) {
        anyhow::bail!("malformed ruled-surface edge is connected to face {face}");
    }
    Ok(component)
}

/// The grid of face `fi`'s surface, its trim one rectangle or loops.
pub(crate) fn face_grid(g: &ObjectGraph, fi: usize) -> Result<FaceGrid> {
    face_grid_impl(g, fi, true, true)
}

/// [`face_grid`]; with `allow_general` a trim that is not one rectangle
/// (several loops, slanted or curved edges) is kept as its UV loops.
///
/// With `exchange`, a ruled face or a surface of revolution also takes the points the faces
/// of its group draw along the edges they share (`seam_group`).
fn face_grid_impl(
    g: &ObjectGraph,
    fi: usize,
    allow_general: bool,
    exchange: bool,
) -> Result<FaceGrid> {
    use crate::native_parametric_surface::Curve;
    use crate::native_saved_mesh::pointer;
    let f = g.objects.get(fi).context("face index")?;
    ensure!(f.class_name == "Face", "not Face");
    ensure!(
        f.fields["m_faceRegions"]
            .as_array()
            .is_some_and(|items| items.is_empty()),
        "face regions need subdivision"
    );
    let si = pointer(g, fi, &f.fields["m_pSurf"])?.context("face surface")?;
    let surface = ParametricSurface::read(g, si)?;
    let li = pointer(g, fi, &f.fields["m_pFirstLoop"])?.context("face loop")?;
    let l = &g.objects[li];
    ensure!(
        ["EdgeLoop", "EdgeLoopWithChainEnvelopes"].contains(&l.class_name.as_str()),
        "unsupported trim loop"
    );
    ensure!(l.fields["m_open"] == false, "open trim loop");
    ensure!(
        pointer(g, li, &l.fields["m_pFace"])? == Some(fi),
        "loop owner mismatch"
    );
    let multiple_loops = pointer(g, li, &l.fields["m_nextLoop"])?.is_some();
    ensure!(
        allow_general || !multiple_loops,
        "multiple parametric trim loops unsupported"
    );
    let mut cur = pointer(g, li, &l.fields["m_next"])?.context("empty trim")?;
    let mut seen = std::collections::BTreeSet::new();
    let mut segments = Vec::new();
    while cur != li {
        ensure!(seen.insert(cur), "trim edge cycle");
        let e = &g.objects[cur];
        ensure!(e.class_name == "Edge", "nonedge trim member");
        let faces = e.fields["m_pFace"].as_array().context("edge face pair")?;
        ensure!(faces.len() == 2, "edge face pair length");
        let mut side = None;
        for (i, p) in faces.iter().enumerate() {
            if pointer(g, cur, p)? == Some(fi) {
                ensure!(side.is_none(), "ambiguous face side");
                side = Some(i);
            }
        }
        let side = side.context("edge does not belong to face")?;
        let ends = e.fields["m_firstAndLastEdgePnts"]
            .as_array()
            .context("edge endpoints")?;
        ensure!(ends.len() == 2, "edge endpoint count");
        let mut points = vec![uv_point(&ends[0]["uv"][side])?];
        for p in e.fields["m_interiorEdgePnts"]
            .as_array()
            .context("edge interior points")?
        {
            points.push(uv_point(&p["uv"][side])?);
        }
        points.push(uv_point(&ends[1]["uv"][side])?);
        segments.push(points);
        cur = pointer(g, cur, &e.fields["m_next"][side])?.context("null trim next")?;
    }
    ensure!(segments.len() >= 2, "too few parametric edges");
    ensure!(
        segments.iter().all(|s: &Vec<_>| !s.is_empty()),
        "empty parametric edge"
    );
    let env = surface.bounds();
    for segment in &mut segments {
        for p in segment {
            for k in 0..2 {
                ensure!(
                    p[k] >= env[0][k] - 1e-7 && p[k] <= env[1][k] + 1e-7,
                    "trim outside surface envelope"
                );
                p[k] = p[k].clamp(env[0][k], env[1][k]);
            }
        }
    }
    let mut closes = false;
    for reverse in [false, true] {
        let first = if reverse {
            *segments[0].last().unwrap()
        } else {
            segments[0][0]
        };
        let mut last = if reverse {
            segments[0][0]
        } else {
            *segments[0].last().unwrap()
        };
        let mut valid = true;
        for seg in &segments[1..] {
            let p = surface.evaluate(last[0], last[1])?;
            let a = seg[0];
            let b = *seg.last().unwrap();
            if distance(p, surface.evaluate(a[0], a[1])?) < 1e-7 {
                last = b;
            } else if distance(p, surface.evaluate(b[0], b[1])?) < 1e-7 {
                last = a;
            } else {
                valid = false;
                break;
            }
        }
        if valid
            && distance(
                surface.evaluate(first[0], first[1])?,
                surface.evaluate(last[0], last[1])?,
            ) < 1e-7
        {
            closes = true;
            break;
        }
    }
    ensure!(closes, "disconnected parametric trim in model coordinates");
    let mut trim = segments.iter().flatten().copied().collect::<Vec<_>>();
    // A face with several loops is gridded over all of them: a loop outside the first
    // one's range would otherwise have no knots of its own.
    let all_loops = if multiple_loops {
        Some(crate::native_saved_mesh::face_loops_uv(g, fi)?)
    } else {
        None
    };
    for p in all_loops.iter().flatten().flatten() {
        trim.push(std::array::from_fn(|k| p[k].clamp(env[0][k], env[1][k])));
    }
    let min = std::array::from_fn(|k| trim.iter().map(|p| p[k]).fold(f64::INFINITY, f64::min));
    let max = std::array::from_fn(|k| trim.iter().map(|p| p[k]).fold(f64::NEG_INFINITY, f64::max));
    ensure!(
        max[0] > min[0] && max[1] > min[1],
        "degenerate parametric domain"
    );
    let mut sides: [Vec<[f64; 2]>; 4] = std::array::from_fn(|_| Vec::new());
    let mut rectangular = !multiple_loops;
    'sides: for seg in &segments {
        for pair in seg.windows(2) {
            let a = pair[0];
            let b = pair[1];
            if (a[0] - b[0]).abs() < 1e-12 && (a[1] - b[1]).abs() < 1e-12 {
                continue;
            }
            let (side, axis) = if (a[1] - min[1]).abs() < 1e-7 && (b[1] - min[1]).abs() < 1e-7 {
                (0, 0)
            } else if (a[0] - max[0]).abs() < 1e-7 && (b[0] - max[0]).abs() < 1e-7 {
                (1, 1)
            } else if (a[1] - max[1]).abs() < 1e-7 && (b[1] - max[1]).abs() < 1e-7 {
                (2, 0)
            } else if (a[0] - min[0]).abs() < 1e-7 && (b[0] - min[0]).abs() < 1e-7 {
                (3, 1)
            } else if allow_general {
                rectangular = false;
                break 'sides;
            } else {
                anyhow::bail!("nonrectangular parametric trim");
            };
            sides[side].push([a[axis].min(b[axis]), a[axis].max(b[axis])]);
        }
    }
    for (side, intervals) in sides.iter_mut().enumerate() {
        if !rectangular {
            break;
        }
        let axis = if side % 2 == 0 { 0 } else { 1 };
        if intervals.is_empty() {
            // A revolution's pole, or the explicitly saved point at one
            // end of a ruled patch, collapses a horizontal UV side.
            let v = if side == 0 { min[1] } else { max[1] };
            let ruled_pole = matches!(&surface,
                ParametricSurface::RuledSurf { profile1: Curve::Point(_), .. } if v == 0.)
                || matches!(&surface,
                    ParametricSurface::RuledSurf { profile2: Curve::Point(_), .. } if v == 1.);
            ensure!(
                axis == 0 && (matches!(surface, ParametricSurface::SurfRev { .. }) || ruled_pole),
                "missing rectangular trim side"
            );
            let a = surface.evaluate(min[0], v)?;
            let b = surface.evaluate((min[0] + max[0]) * 0.5, v)?;
            let c = surface.evaluate(max[0], v)?;
            ensure!(
                distance(a, b) < 1e-9 && distance(a, c) < 1e-9,
                "missing noncollapsed trim side"
            );
            continue;
        }
        intervals.sort_by(|a, b| a[0].total_cmp(&b[0]));
        let mut end = min[axis];
        for p in intervals {
            ensure!(
                (p[0] - end).abs() < 1e-7,
                "overlapping or incomplete trim perimeter"
            );
            end = p[1];
        }
        ensure!((end - max[axis]).abs() < 1e-7, "incomplete trim perimeter");
    }
    const CHORD: f64 = 0.001;
    /// How far, feet, a cell of a ruled face with curved profiles may leave the surface for
    /// the change of its ruling along the profiles.
    const TWIST_FEET: f64 = 0.007;
    let length = |p: [f64; 3]| p.iter().map(|x| x * x).sum::<f64>().sqrt();
    let angular = |r: f64| {
        (4. * (CHORD / (2. * r.max(CHORD))).min(1.).sqrt().asin()).min(std::f64::consts::FRAC_PI_2)
    };
    let (du, dv) = match &surface {
        ParametricSurface::SurfRev { profile, .. } => match profile {
            Curve::Point(_) => anyhow::bail!("point revolution profile is unqualified"),
            Curve::Line {
                origin, direction, ..
            } => {
                let radius = [min[1], max[1]]
                    .into_iter()
                    .map(|v| (origin[0] + v * direction[0]).hypot(origin[1] + v * direction[1]))
                    .fold(0., f64::max);
                (angular(radius), 1. / length(*direction))
            }
            Curve::Hermite { nodes, range } => (
                angular(
                    nodes
                        .iter()
                        .map(|n| n.1[0].hypot(n.1[1]))
                        .fold(0., f64::max),
                ),
                // a spline profile is cut at least as finely as before and as
                // its chord error asks for.
                ((range[1] - range[0]) / (8 * nodes.len()) as f64)
                    .min((max[1] - min[1]) / profile.spline_steps(min[1], max[1], CHORD) as f64),
            ),
            Curve::Arc { center, radius, .. } => (
                angular(center[0].hypot(center[1]) + 2. * radius),
                angular(*radius),
            ),
            // a revolved dome's elliptic meridian, stepped as an arc of its
            // larger radius.
            Curve::Ellipse {
                center,
                x_len,
                y_len,
                ..
            } => {
                let radius = x_len.max(*y_len);
                (
                    angular(center[0].hypot(center[1]) + 2. * radius),
                    angular(radius),
                )
            }
        },
        ParametricSurface::ConeSurf { half_angle, .. } => {
            // The generators are straight: only the angle needs a chord bound.
            let far = min[1].abs().max(max[1].abs());
            (angular(far * half_angle.sin()), 1.)
        }
        ParametricSurface::RuledSurf { .. } => {
            // A shared normalized grid across ruled patches preserves their
            // common straight generator edges even when profile lengths differ.
            let mut n = 1.0_f64;
            let mut n_lines = 1.0_f64;
            let mut curved = false;
            let mut twist = 0.0_f64;
            let component = ruled_surface_component(g, fi)?;
            for i in component.into_iter().filter(|i| {
                g.objects
                    .get(*i)
                    .and_then(|face| face.fields.get("m_pSurf"))
                    .is_some()
            }) {
                let Some(surface_index) = pointer(g, i, &g.objects[i].fields["m_pSurf"])? else {
                    continue;
                };
                if g.objects[surface_index].class_name != "RuledSurf" {
                    continue;
                }
                let s = ParametricSurface::read(g, surface_index)?;
                let ParametricSurface::RuledSurf {
                    profile1:
                        Curve::Line {
                            direction: a,
                            range: ra,
                            ..
                        },
                    profile2:
                        Curve::Line {
                            direction: b,
                            range: rb,
                            ..
                        },
                    ..
                } = s
                else {
                    // curved profiles (arcs, Hermite splines): as
                    // many shared u segments as either profile's turning
                    // (0.1 rad) or length (1 ft) needs.
                    if let ParametricSurface::RuledSurf {
                        profile1, profile2, ..
                    } = &s
                    {
                        n = n
                            .max(curve_segments(profile1))
                            .max(curve_segments(profile2));
                        twist = twist.max(profile_twist(profile1, profile2));
                    }
                    curved = true;
                    continue;
                };
                let da = std::array::from_fn(|k| a[k] * (ra[1] - ra[0]));
                let db = std::array::from_fn(|k| b[k] * (rb[1] - rb[0]));
                let mixed = length(std::array::from_fn(|k| da[k] - db[k]));
                n = n
                    .max((mixed / CHORD).sqrt().ceil())
                    .max(length(da).ceil())
                    .max(length(db).ceil());
                n_lines = n_lines.max(n);
            }
            // The rulings are straight, so a curved profile adds u segments, and v the
            // segments its twist asks for: a cell of the grid is two triangles, which
            // leave the surface by a quarter of the cell's mixed second derivative (the
            // change of the ruling per step along the profiles) times its two steps. The
            // twist is held to `TWIST_FEET`, looser than the profile's chord error: Revit's
            // vertices then lie within 0.0016 ft of a timber stair run, and the duct
            // fittings, whose rulings turn little, are not cut finer for nothing.
            // Never more than keeps the grid's points (the saved values of the trim
            // included) within the mesh budget.
            if curved {
                let asked = (twist / (4. * TWIST_FEET * n)).ceil();
                // The grid also holds the saved values of the trim in each axis: a fixed
                // allowance for them, so that every face of the component takes the
                // same `v` steps and shares its generators' points.
                const KNOTS: f64 = 1000.;
                let room = ((1_500_000. / (n + 1. + KNOTS)).floor() - 1. - KNOTS).max(1.);
                n_lines = n_lines.max(asked.min(2000.).min(room));
            }
            (1. / n, if curved { 1. / n_lines } else { 1. / n })
        }
    };
    // (the equal steps, and those with the trim's values)
    let axis = |k: usize, step: f64| -> Result<(Vec<f64>, Vec<f64>)> {
        // a span a rounding step over a whole number of steps is that many (a trim end that
        // reads 1.0000000000000029 would otherwise cost the face one more step than the
        // face beside it, whose end is 1)
        let count = (((max[k] - min[k]) / step) - 1e-9).ceil().max(1.);
        ensure!(
            count.is_finite() && count <= 10000.,
            "parametric axis budget exceeded"
        );
        let count = count as usize;
        let lattice = (0..=count)
            .map(|i| min[k] + (max[k] - min[k]) * i as f64 / count as f64)
            .collect::<Vec<_>>();
        let mut a = lattice.clone();
        a.extend(trim.iter().map(|p| p[k]));
        a.sort_by(f64::total_cmp);
        a.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
        Ok((lattice, a))
    };
    let (_, mut u_knots) = axis(0, du)?;
    let (v_lattice, mut v_knots) = axis(1, dv)?;
    let mut at_u_extra = Vec::new();
    if exchange
        && matches!(
            surface,
            ParametricSurface::RuledSurf { .. } | ParametricSurface::SurfRev { .. }
        )
    {
        let ex = group_seams(g, fi);
        for (knots, extra) in [(&mut u_knots, &ex.u), (&mut v_knots, &ex.v)] {
            knots.extend(extra);
            knots.sort_by(f64::total_cmp);
            knots.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
        }
        at_u_extra = ex.at_u.clone();
    }
    let loops = if rectangular {
        None
    } else if let Some(all) = all_loops {
        Some(all)
    } else {
        Some(crate::native_saved_mesh::face_loops_uv(g, fi)?)
    };
    // Only a trim that is not one rectangle has an edge across both parameters.
    let (chord_edges, chords) = if loops.is_some() && keeps_saved_chords(&surface) {
        chord_spans(g, fi)?
    } else {
        Default::default()
    };
    Ok(FaceGrid {
        u: u_knots,
        v: v_knots,
        v_lattice,
        surface,
        trim,
        bounds: [min, max],
        loops,
        chord_edges,
        chords,
        at_u_extra,
    })
}

/// The points a ruled face or a surface of revolution takes from the neighbours across its
/// edges.
#[derive(Debug, Default)]
struct SeamExtras {
    u: Vec<f64>,
    v: Vec<f64>,
    /// `[u, v]`: a point on the strip side at `u`.
    at_u: Vec<[f64; 2]>,
}

/// Per graph, the points each face takes from its group (`seam_group`), computed once for
/// every face of a group.
#[derive(Default)]
pub(crate) struct SeamCache(
    std::sync::Mutex<std::collections::HashMap<usize, std::sync::Arc<SeamExtras>>>,
);

impl Clone for SeamCache {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl std::fmt::Debug for SeamCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SeamCache")
    }
}

/// The points face `fi` takes from the connected group of faces it exchanges with.
fn group_seams(g: &ObjectGraph, fi: usize) -> std::sync::Arc<SeamExtras> {
    let lock = || {
        g.seams
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    };
    if let Some(found) = lock().get(&fi) {
        return found.clone();
    }
    let group = seam_group(g, fi).unwrap_or_default();
    let mut cache = lock();
    for (face, extras) in group {
        cache
            .entry(face)
            .or_insert_with(|| std::sync::Arc::new(extras));
    }
    cache.entry(fi).or_default().clone()
}

/// The edges of face `fi`'s trim loops: `(edge, side of fi, the face on the other side)`.
fn face_links(g: &ObjectGraph, fi: usize) -> Result<Vec<(usize, usize, usize)>> {
    use crate::native_saved_mesh::pointer;
    let mut links = Vec::new();
    let f = g.objects.get(fi).context("face index")?;
    let mut next_loop = pointer(g, fi, &f.fields["m_pFirstLoop"])?;
    let mut seen_loops = std::collections::BTreeSet::new();
    while let Some(li) = next_loop {
        ensure!(seen_loops.insert(li), "loop-list cycle");
        let l = &g.objects[li];
        let mut cur = pointer(g, li, &l.fields["m_next"])?.context("empty trim")?;
        let mut seen = std::collections::BTreeSet::new();
        while cur != li {
            ensure!(seen.insert(cur), "trim edge cycle");
            let e = &g.objects[cur];
            let faces = e.fields["m_pFace"].as_array().context("edge face pair")?;
            ensure!(faces.len() == 2, "edge face pair length");
            let mut side = None;
            for (i, p) in faces.iter().enumerate() {
                if pointer(g, cur, p)? == Some(fi) {
                    side = Some(i);
                }
            }
            let side = side.context("edge does not belong to face")?;
            if let Some(other) = pointer(g, cur, &faces[1 - side]).ok().flatten() {
                if other != fi {
                    links.push((cur, side, other));
                }
            }
            cur = pointer(g, cur, &e.fields["m_next"][side])?.context("null trim next")?;
        }
        next_loop = pointer(g, li, &l.fields["m_nextLoop"])?;
    }
    Ok(links)
}

/// Whether two faces take each other's points along an edge they share: ruled faces and
/// surfaces of revolution trimmed to a rectangle, except two ruled faces trimmed alike,
/// which share their normalized grid instead (`face_grid_impl`). A trimmed surface of
/// revolution keeps its own points.
fn exchanges(a: &FaceGrid, b: &FaceGrid) -> bool {
    use ParametricSurface::{RuledSurf, SurfRev};
    let trimmed_revolution =
        |f: &FaceGrid| matches!(f.surface, SurfRev { .. }) && f.loops.is_some();
    if trimmed_revolution(a) || trimmed_revolution(b) {
        return false;
    }
    match (&a.surface, &b.surface) {
        (RuledSurf { .. }, RuledSurf { .. }) => a.loops.is_some() != b.loops.is_some(),
        (RuledSurf { .. } | SurfRev { .. }, RuledSurf { .. } | SurfRev { .. }) => true,
        _ => false,
    }
}

/// A face's points during the exchange: its knots with those it took, sorted, and the points
/// its strip sides took.
struct SeamWork {
    u: Vec<f64>,
    v: Vec<f64>,
    at_u: Vec<[f64; 2]>,
    extras: SeamExtras,
}

/// Whether sorted `values` hold `x` to 1e-9.
fn holds(values: &[f64], x: f64) -> bool {
    let i = values.partition_point(|y| *y < x);
    (i > 0 && x - values[i - 1] < 1e-9) || (i < values.len() && values[i] - x < 1e-9)
}

/// The `v` values a trimmed face's strip side at `u` holds: the equal steps, the loops' own
/// points there and the points it took.
fn strip_side(grid: &FaceGrid, work: &SeamWork, u: f64) -> Vec<f64> {
    let mut values: Vec<f64> = grid
        .v_lattice
        .iter()
        .copied()
        .chain(
            grid.loops
                .iter()
                .flatten()
                .flatten()
                .chain(&work.at_u)
                .filter(|p| (p[0] - u).abs() < 1e-9)
                .map(|p| p[1]),
        )
        .collect();
    values.sort_by(f64::total_cmp);
    values
}

/// One shared edge, along one `u` or one `v` in both faces: the points the face on side
/// `1 - side` (`og`, with the points it took so far, `ow`) draws on it, mapped into the
/// parameters of the face on `side` by the edge's saved points; with the axis they lie
/// along there and that face's fixed parameter. A rectangular face draws every knot of its
/// grid there, a trimmed one the strips' breaks (along `u`) or its strip side (along `v`).
fn edge_values(
    g: &ObjectGraph,
    edge: usize,
    side: usize,
    og: &FaceGrid,
    ow: &SeamWork,
) -> Result<(usize, f64, Vec<f64>)> {
    use crate::native_saved_mesh::isoparametric_axis;
    let (mine, theirs) = (edge_uv(g, edge, side)?, edge_uv(g, edge, 1 - side)?);
    ensure!(mine.len() == theirs.len(), "edge sample count");
    let (ax, ay) = (
        isoparametric_axis(&mine).context("edge across both parameters")?,
        isoparametric_axis(&theirs).context("edge across both parameters")?,
    );
    let (first, last) = (theirs[0][ay], theirs[theirs.len() - 1][ay]);
    let (lo, hi) = (first.min(last), first.max(last));
    let fixed = theirs[0][1 - ay];
    let mut values: Vec<f64> = match (&og.loops, ay) {
        (None, 0) => ow.u.clone(),
        (None, _) => ow.v.clone(),
        (Some(loops), 0) => strip_breaks(loops, &ow.u),
        (Some(_), _) => strip_side(og, ow, fixed),
    };
    values.retain(|v| *v > lo + 1e-9 && *v < hi - 1e-9);
    // the same edge in this face's parameters: the saved points must agree under one
    // straight map
    let (x0, x1) = (mine[0][ax], mine[mine.len() - 1][ax]);
    ensure!((last - first).abs() > 1e-12, "collapsed edge");
    let map = |y: f64| x0 + (y - first) / (last - first) * (x1 - x0);
    ensure!(
        mine.iter()
            .zip(&theirs)
            .all(|(a, b)| (map(b[ay]) - a[ax]).abs() < 1e-7),
        "edge parameters differ"
    );
    Ok((ax, mine[0][1 - ax], values.into_iter().map(map).collect()))
}

/// Rounds of the exchange at most. A round in which no face takes a new point ends it, and
/// a point crosses one edge per round, so a group whose edges map the parameters the same
/// way whichever path a point takes settles within as many rounds as it has faces. The
/// cap stops a group whose maps do not compose exactly round a cycle of faces, which could
/// otherwise keep making points.
const SEAM_ROUNDS: usize = 32;

/// The connected group of ruled faces and surfaces of revolution that `fi` exchanges with
/// (`exchanges`), and the points each takes: every face takes what each neighbour draws
/// along their shared edge, the neighbour's own takings included, until no face takes a
/// new point, so that the faces of a chain (a revolution, a ruled face, a revolution) or of
/// a ring of revolutions all draw a shared edge through the same points. An edge that does
/// not run along one `u` or one `v` in both faces, or whose saved points do not agree,
/// carries nothing.
fn seam_group(g: &ObjectGraph, fi: usize) -> Result<std::collections::BTreeMap<usize, SeamExtras>> {
    use crate::native_saved_mesh::pointer;
    use std::collections::BTreeMap;
    // a face's grid without the exchange, or `None` when it is no ruled face or surface of
    // revolution or has no grid
    let mut grids: BTreeMap<usize, Option<FaceGrid>> = BTreeMap::new();
    let grid_of = |grids: &mut BTreeMap<usize, Option<FaceGrid>>, face: usize| -> bool {
        grids
            .entry(face)
            .or_insert_with(|| {
                let surface = g.objects.get(face)?.fields.get("m_pSurf")?;
                let surface = pointer(g, face, surface).ok()??;
                if !matches!(
                    g.objects.get(surface)?.class_name.as_str(),
                    "RuledSurf" | "SurfRev"
                ) {
                    return None;
                }
                face_grid_impl(g, face, true, false).ok()
            })
            .is_some()
    };
    ensure!(
        grid_of(&mut grids, fi),
        "face without a ruled or revolved grid"
    );
    let mut links: BTreeMap<usize, Vec<(usize, usize, usize)>> = BTreeMap::new();
    let mut stack = vec![fi];
    links.insert(fi, Vec::new());
    while let Some(f) = stack.pop() {
        let mut mine = Vec::new();
        for (edge, side, other) in face_links(g, f).unwrap_or_default() {
            if !grid_of(&mut grids, other) {
                continue;
            }
            let (a, b) = (grids[&f].as_ref(), grids[&other].as_ref());
            if !a.zip(b).is_some_and(|(a, b)| exchanges(a, b)) {
                continue;
            }
            mine.push((edge, side, other));
            if let std::collections::btree_map::Entry::Vacant(e) = links.entry(other) {
                e.insert(Vec::new());
                stack.push(other);
            }
        }
        links.insert(f, mine);
    }
    let grid = |face: usize| grids[&face].as_ref().expect("group member has a grid");
    let mut work: BTreeMap<usize, SeamWork> = links
        .keys()
        .map(|&face| {
            let grid = grid(face);
            let work = SeamWork {
                u: grid.u.clone(),
                v: grid.v.clone(),
                at_u: Vec::new(),
                extras: SeamExtras::default(),
            };
            (face, work)
        })
        .collect();
    for _ in 0..SEAM_ROUNDS {
        let mut grew = false;
        for (&f, mine) in &links {
            for &(edge, side, other) in mine {
                let Ok((ax, fixed, values)) =
                    edge_values(g, edge, side, grid(other), &work[&other])
                else {
                    continue;
                };
                let rectangular = grid(f).loops.is_none();
                let side_values =
                    (ax == 1 && !rectangular).then(|| strip_side(grid(f), &work[&f], fixed));
                let w = work.get_mut(&f).expect("group member");
                for x in values {
                    let taken = match (ax, &side_values) {
                        (0, _) => &mut w.u,
                        (_, None) => &mut w.v,
                        (_, Some(side)) => {
                            if !holds(side, x)
                                && !w
                                    .at_u
                                    .iter()
                                    .any(|p| (p[0] - fixed).abs() < 1e-9 && (p[1] - x).abs() < 1e-9)
                            {
                                w.at_u.push([fixed, x]);
                                w.extras.at_u.push([fixed, x]);
                                grew = true;
                            }
                            continue;
                        }
                    };
                    if !holds(taken, x) {
                        taken.insert(taken.partition_point(|y| *y < x), x);
                        if ax == 0 {
                            &mut w.extras.u
                        } else {
                            &mut w.extras.v
                        }
                        .push(x);
                        grew = true;
                    }
                }
            }
        }
        if !grew {
            break;
        }
    }
    Ok(work.into_iter().map(|(face, w)| (face, w.extras)).collect())
}

/// The saved points of `edge` in the parameters of its face on `side`.
fn edge_uv(g: &ObjectGraph, edge: usize, side: usize) -> Result<Vec<[f64; 2]>> {
    let e = &g.objects[edge];
    let ends = e.fields["m_firstAndLastEdgePnts"]
        .as_array()
        .context("edge endpoints")?;
    ensure!(ends.len() == 2, "edge endpoint count");
    let mut points = vec![uv_point(&ends[0]["uv"][side])?];
    for p in e.fields["m_interiorEdgePnts"]
        .as_array()
        .context("edge interior points")?
    {
        points.push(uv_point(&p["uv"][side])?);
    }
    points.push(uv_point(&ends[1]["uv"][side])?);
    Ok(points)
}

/// The greatest length of the change of a ruling per unit of `u` between two profiles
/// (the mixed second derivative of the ruled surface), at 65 equal steps of `u`.
fn profile_twist(
    a: &crate::native_parametric_surface::Curve,
    b: &crate::native_parametric_surface::Curve,
) -> f64 {
    let (ra, rb) = (a.range(), b.range());
    (0..=64)
        .map(|i| {
            let u = f64::from(i) / 64.;
            let da = a.derivative(ra[0] + u * (ra[1] - ra[0]));
            let db = b.derivative(rb[0] + u * (rb[1] - rb[0]));
            let d: [f64; 3] =
                std::array::from_fn(|k| db[k] * (rb[1] - rb[0]) - da[k] * (ra[1] - ra[0]));
            d.iter().map(|x| x * x).sum::<f64>().sqrt()
        })
        .fold(0., f64::max)
}

/// Segments a curved profile needs: its turning in 0.1 rad steps or its
/// length in 1 ft steps, whichever is more (sampled at 64 points).
fn curve_segments(c: &crate::native_parametric_surface::Curve) -> f64 {
    let range = c.range();
    let at = |i: usize| range[0] + (range[1] - range[0]) * i as f64 / 64.;
    let norm = |p: [f64; 3]| p.iter().map(|x| x * x).sum::<f64>().sqrt();
    let (mut turn, mut length) = (0., 0.);
    for i in 0..64 {
        let (a, b) = (c.derivative(at(i)), c.derivative(at(i + 1)));
        let (na, nb) = (norm(a), norm(b));
        if na > 1e-12 && nb > 1e-12 {
            let cos = (a[0] * b[0] + a[1] * b[1] + a[2] * b[2]) / (na * nb);
            turn += cos.clamp(-1., 1.).acos();
        }
        let (p, q) = (c.eval(at(i)), c.eval(at(i + 1)));
        length += norm([q[0] - p[0], q[1] - p[1], q[2] - p[2]]);
    }
    // a spline is cut at least as finely as its chord error asks for (a ruled
    // face's twist is bounded by the turning and length alone, so neither
    // count replaces the other).
    let splined = c.spline_steps(range[0], range[1], 0.001) as f64;
    (turn / 0.1)
        .max(length)
        .max(splined)
        .ceil()
        .clamp(1., 2000.)
}

pub fn face(g: &ObjectGraph, fi: usize) -> Result<FaceMesh> {
    let grid = face_grid_impl(g, fi, true, true)?;
    let f = &g.objects[fi];
    let flags = f.fields["m_faceFlags_v9"].as_u64().context("face flags")?;
    if let Some(loops) = &grid.loops {
        let mesh = tessellate_loops(&grid, loops, flags & 2 == 0)?;
        return Ok(FaceMesh {
            face_index: fi,
            face_tag: f.fields["m_GInfo"]["m_tag"].as_i64().unwrap_or(-1),
            render_style_id: crate::native_metadata::identifier(&f.fields["m_renderStyleId"])?,
            normal: mesh.normals[0],
            vertices: mesh.vertices,
            normals: mesh.normals,
            triangles: mesh.triangles,
            analytic_surface: None,
            trim_uv: grid.trim.clone(),
        });
    }
    let mesh = tessellate_uv(
        &grid.surface,
        &grid.u,
        &grid.v,
        grid.bounds,
        flags & 2 == 0,
        2_000_000,
    )?;
    Ok(FaceMesh {
        face_index: fi,
        face_tag: f.fields["m_GInfo"]["m_tag"].as_i64().unwrap_or(-1),
        render_style_id: crate::native_metadata::identifier(&f.fields["m_renderStyleId"])?,
        normal: mesh.normals[0],
        vertices: mesh.vertices,
        normals: mesh.normals,
        triangles: mesh.triangles,
        analytic_surface: None,
        trim_uv: grid.trim,
    })
}

/// The `u` values a trimmed face is cut into strips at: every loop point's, and the grid's
/// between the loops' least and greatest.
pub(crate) fn strip_breaks(loops: &[Vec<[f64; 2]>], u_grid: &[f64]) -> Vec<f64> {
    let mut breaks: Vec<f64> = loops.iter().flatten().map(|p| p[0]).collect();
    let (lo, hi) = breaks
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &u| {
            (a.min(u), b.max(u))
        });
    breaks.extend(u_grid.iter().copied().filter(|u| *u > lo && *u < hi));
    breaks.sort_by(f64::total_cmp);
    // points a rounding step apart are one point (`key`), so are their breaks
    breaks.dedup_by(|a, b| (*a - *b).abs() <= 1e-9);
    breaks
}

/// A parameter rounded to 1e-9, as the strips number their points.
fn key(x: f64) -> i64 {
    (x * 1e9).round() as i64
}

/// One span of a trim loop between two saved points, the end of lower `u` first.
struct Span {
    a: [f64; 2],
    b: [f64; 2],
    /// Where the span is one of the chords (`FaceGrid::chords`), its two ends as saved.
    chord: Option<[[f64; 2]; 2]>,
}

/// Triangles covering the region the closed `loops` bound (even-odd), counter-clockwise in
/// (u, v), on points shared between the triangles that use them. The region is cut into
/// strips at the `strip_breaks`; inside a strip each crossing span is straight, and the
/// area between the 1st and 2nd, 3rd and 4th... of them is a trapezoid. A side of a
/// trapezoid, at one `u`, holds the `v_lattice` values between its ends and the `v` of every
/// point of the loops at that `u`, so that the strips beside each other, and a plane beside
/// a vertical edge of the trim (which takes the same values,
/// `native_saved_mesh::refined_plane_edge`), have the same points along it; the
/// two sides are joined by a zipper of triangles. A point on a chord's span is tagged
/// with the chord's ends.
fn conforming_strips(
    loops: &[Vec<[f64; 2]>],
    u_grid: &[f64],
    v_lattice: &[f64],
    chords: &[[[f64; 2]; 2]],
    extra_at_u: &[[f64; 2]],
) -> Result<StripMesh> {
    let breaks = strip_breaks(loops, u_grid);
    ensure!(breaks.len() >= 2, "degenerate trim");
    // A loop joins nearly meeting edges at one of their ends, so a span's ends can be
    // a hair from the saved points of the chord it is: find it by nearness.
    let cell = |p: [f64; 2]| [(p[0] * 1e6).floor() as i64, (p[1] * 1e6).floor() as i64];
    let mut cells: std::collections::HashMap<[i64; 2], Vec<usize>> =
        std::collections::HashMap::new();
    for (i, c) in chords.iter().enumerate() {
        cells.entry(cell(c[0])).or_default().push(i);
        cells.entry(cell(c[1])).or_default().push(i);
    }
    let near = |p: [f64; 2], q: [f64; 2]| (p[0] - q[0]).hypot(p[1] - q[1]) < 1e-7;
    let find_chord = |a: [f64; 2], b: [f64; 2]| -> Option<Chord> {
        let c = cell(a);
        (-1..=1)
            .flat_map(|dx| (-1..=1).map(move |dy| [c[0] + dx, c[1] + dy]))
            .filter_map(|k| cells.get(&k))
            .flatten()
            .map(|&i| chords[i])
            .find(|ch| (near(ch[0], a) && near(ch[1], b)) || (near(ch[1], a) && near(ch[0], b)))
    };
    let spans: Vec<Span> = loops
        .iter()
        .flat_map(|l| l.iter().zip(l.iter().cycle().skip(1)).take(l.len()))
        .map(|(a, b)| {
            let chord = find_chord(*a, *b);
            if a[0] <= b[0] {
                Span {
                    a: *a,
                    b: *b,
                    chord,
                }
            } else {
                Span {
                    a: *b,
                    b: *a,
                    chord,
                }
            }
        })
        .filter(|s| s.b[0] - s.a[0] > 1e-12)
        .collect();
    let at = |s: &Span, u: f64| s.a[1] + (s.b[1] - s.a[1]) * (u - s.a[0]) / (s.b[0] - s.a[0]);
    let mut uv: Vec<[f64; 2]> = Vec::new();
    let mut tags: Vec<Option<[[f64; 2]; 2]>> = Vec::new();
    let mut tris: Vec<[u32; 3]> = Vec::new();
    let mut index: std::collections::HashMap<(i64, i64), u32> = std::collections::HashMap::new();
    let mut vertex =
        |p: [f64; 2], tag: Option<[[f64; 2]; 2]>, uv: &mut Vec<[f64; 2]>| -> Result<u32> {
            if let Some(&i) = index.get(&(key(p[0]), key(p[1]))) {
                if tags[i as usize].is_none() {
                    tags[i as usize] = tag;
                }
                return Ok(i);
            }
            let i = u32::try_from(uv.len()).context("trim vertex count overflow")?;
            uv.push(p);
            tags.push(tag);
            index.insert((key(p[0]), key(p[1])), i);
            Ok(i)
        };
    // The v of every loop point, by its u.
    let mut at_u: std::collections::HashMap<i64, Vec<f64>> = std::collections::HashMap::new();
    for p in loops.iter().flatten().chain(extra_at_u) {
        at_u.entry(key(p[0])).or_default().push(p[1]);
    }
    // The v values along one side at `u`, from `lo` to `hi`.
    let chain = |u: f64, lo: f64, hi: f64| -> Vec<f64> {
        let mut c = vec![lo];
        if hi - lo > 1e-9 {
            let k = key(u);
            let mut inside: Vec<f64> = v_lattice
                .iter()
                .copied()
                .chain(
                    (k - 1..=k + 1)
                        .filter_map(|k| at_u.get(&k))
                        .flatten()
                        .copied(),
                )
                .filter(|v| *v > lo + 1e-9 && *v < hi - 1e-9)
                .collect();
            inside.sort_by(f64::total_cmp);
            inside.dedup_by(|a, b| (*a - *b).abs() <= 1e-9);
            c.extend(inside);
            c.push(hi);
        }
        c
    };
    for w in breaks.windows(2) {
        let (u0, u1) = (w[0], w[1]);
        let um = 0.5 * (u0 + u1);
        let mut cross: Vec<(f64, &Span)> = spans
            .iter()
            .filter(|s| s.a[0] < um && s.b[0] > um)
            .map(|s| (at(s, um), s))
            .collect();
        ensure!(cross.len().is_multiple_of(2), "open trim loop");
        cross.sort_by(|x, y| x.0.total_cmp(&y.0));
        for pair in cross.chunks(2) {
            let (bottom, top) = (pair[0].1, pair[1].1);
            let left = chain(u0, at(bottom, u0), at(top, u0));
            let right = chain(u1, at(bottom, u1), at(top, u1));
            ensure!(
                tris.len() + 2 * (left.len() + right.len()) <= 4_000_000,
                "trim tessellation too large"
            );
            let frac = |c: &[f64], i: usize| {
                let (lo, hi) = (c[0], c[c.len() - 1]);
                if hi - lo > 0. {
                    (c[i] - lo) / (hi - lo)
                } else {
                    0.
                }
            };
            let mut ids = |c: &[f64], u: f64, uv: &mut Vec<[f64; 2]>| -> Result<Vec<u32>> {
                let last = c.len() - 1;
                (0..c.len())
                    .map(|i| {
                        let tag = if i == 0 {
                            bottom.chord
                        } else if i == last {
                            top.chord
                        } else {
                            None
                        };
                        vertex([u, c[i]], tag, uv)
                    })
                    .collect()
            };
            let (l, r) = (ids(&left, u0, &mut uv)?, ids(&right, u1, &mut uv)?);
            let (mut i, mut j) = (0, 0);
            while i + 1 < l.len() || j + 1 < r.len() {
                let advance_left = i + 1 < l.len()
                    && (j + 1 == r.len() || frac(&left, i + 1) <= frac(&right, j + 1));
                let t = if advance_left {
                    let t = [l[i], r[j], l[i + 1]];
                    i += 1;
                    t
                } else {
                    let t = [l[i], r[j], r[j + 1]];
                    j += 1;
                    t
                };
                let (p, q, s) = (uv[t[0] as usize], uv[t[1] as usize], uv[t[2] as usize]);
                if (q[0] - p[0]) * (s[1] - p[1]) - (q[1] - p[1]) * (s[0] - p[0]) > 1e-18 {
                    tris.push(t);
                }
            }
        }
    }
    Ok((uv, tris, tags))
}

/// A parametric face whose trim is not one rectangle: the UV region its
/// loops bound, cut into strips at the grid's u values and split along v at
/// the grid's v values (`conforming_strips`), evaluated on the surface, with a vertex on a
/// chord's span (`FaceGrid::chords`) laid on the straight line between the span's
/// two points.
fn tessellate_loops(
    grid: &FaceGrid,
    loops: &[Vec<[f64; 2]>],
    orient: bool,
) -> Result<ParametricMesh> {
    let (uv, tris, tags) = conforming_strips(
        loops,
        &grid.u,
        &grid.v_lattice,
        &grid.chords,
        &grid.at_u_extra,
    )?;
    ensure!(
        uv.len() <= 2_000_000,
        "parametric mesh vertex budget exceeded"
    );
    let surface_orient = grid.surface.orientation();
    let winding = surface_orient == orient;
    let mut vertices = Vec::with_capacity(uv.len());
    let mut normals = Vec::with_capacity(uv.len());
    for (p, tag) in uv.iter().zip(&tags) {
        let mut v = grid.surface.evaluate(p[0], p[1])?;
        if let Some([a, b]) = tag {
            let d = [b[0] - a[0], b[1] - a[1]];
            let length = d[0] * d[0] + d[1] * d[1];
            let t = ((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / length;
            if length > 0. && t > 0. && t < 1. {
                v = chord_point(&grid.surface, *a, *b, t)?;
            }
        }
        vertices.push(v);
        let mut n = grid.surface.normal(p[0], p[1])?;
        if !orient {
            for value in &mut n {
                *value = -*value;
            }
        }
        normals.push(n);
    }
    let mut triangles = Vec::with_capacity(tris.len());
    for t in tris {
        let [p, q, r] = t.map(|i| vertices[i as usize]);
        let u = [q[0] - p[0], q[1] - p[1], q[2] - p[2]];
        let v = [r[0] - p[0], r[1] - p[1], r[2] - p[2]];
        let area = (u[1] * v[2] - u[2] * v[1])
            .hypot((u[2] * v[0] - u[0] * v[2]).hypot(u[0] * v[1] - u[1] * v[0]));
        if area > 1e-12 {
            triangles.push(if winding { t } else { [t[0], t[2], t[1]] });
        }
    }
    ensure!(
        !triangles.is_empty(),
        "parametric surface produced no triangles"
    );
    Ok(ParametricMesh {
        vertices,
        normals,
        triangles,
    })
}

pub fn tessellate(
    surface: &ParametricSurface,
    u_segments: usize,
    v_segments: usize,
    max_vertices: usize,
) -> Result<ParametricMesh> {
    ensure!(
        u_segments > 0 && v_segments > 0 && max_vertices > 3 && max_vertices <= u32::MAX as usize,
        "invalid parametric mesh budget"
    );
    let e = surface.bounds();
    let u_count = u_segments
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("U grid overflow"))?;
    let v_count = v_segments
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("V grid overflow"))?;
    let grid_count = u_count
        .checked_mul(v_count)
        .ok_or_else(|| anyhow::anyhow!("parametric mesh vertex count overflow"))?;
    ensure!(
        grid_count <= max_vertices,
        "parametric mesh vertex budget exceeded"
    );
    ensure!(
        grid_count <= u32::MAX as usize,
        "parametric mesh exceeds index range"
    );
    let mut us = Vec::with_capacity(u_count);
    let mut vs = Vec::with_capacity(v_count);
    for i in 0..=u_segments {
        us.push(e[0][0] + (e[1][0] - e[0][0]) * (i as f64 / u_segments as f64));
    }
    for j in 0..=v_segments {
        vs.push(e[0][1] + (e[1][1] - e[0][1]) * (j as f64 / v_segments as f64));
    }
    tessellate_uv(surface, &us, &vs, e, true, max_vertices)
}

pub fn tessellate_uv(
    surface: &ParametricSurface,
    u_knots: &[f64],
    v_knots: &[f64],
    trim: [[f64; 2]; 2],
    orient: bool,
    max_vertices: usize,
) -> Result<ParametricMesh> {
    ensure!(
        u_knots.len() >= 2
            && v_knots.len() >= 2
            && max_vertices > 3
            && max_vertices <= u32::MAX as usize,
        "invalid parametric mesh budget"
    );
    ensure!(
        u_knots.iter().all(|x| x.is_finite())
            && v_knots.iter().all(|x| x.is_finite())
            && u_knots.windows(2).all(|w| w[1] > w[0])
            && v_knots.windows(2).all(|w| w[1] > w[0]),
        "knots must be finite and strictly increasing"
    );
    ensure!(
        trim.iter().flatten().all(|x| x.is_finite())
            && trim[1][0] > trim[0][0]
            && trim[1][1] > trim[0][1],
        "invalid trim rectangle"
    );
    let bounds = surface.bounds();
    ensure!(
        trim[0][0] >= bounds[0][0]
            && trim[1][0] <= bounds[1][0]
            && trim[0][1] >= bounds[0][1]
            && trim[1][1] <= bounds[1][1],
        "trim outside surface envelope"
    );
    let (Some(u_first), Some(u_last), Some(v_first), Some(v_last)) = (
        u_knots.first(),
        u_knots.last(),
        v_knots.first(),
        v_knots.last(),
    ) else {
        anyhow::bail!("empty knot vector");
    };
    ensure!(
        *u_first >= trim[0][0]
            && *u_last <= trim[1][0]
            && *v_first >= trim[0][1]
            && *v_last <= trim[1][1],
        "knots outside trim rectangle"
    );
    let count = u_knots
        .len()
        .checked_mul(v_knots.len())
        .ok_or_else(|| anyhow::anyhow!("parametric mesh vertex count overflow"))?;
    ensure!(
        count <= max_vertices && count <= u32::MAX as usize,
        "parametric mesh vertex budget exceeded"
    );
    let mut vertices = Vec::with_capacity(count);
    let mut normals = Vec::with_capacity(count);
    for &v in v_knots {
        for &u in u_knots {
            vertices.push(surface.evaluate(u, v)?);
            let mut n = surface.normal(u, v)?;
            if !orient {
                for value in &mut n {
                    *value = -*value;
                }
            }
            normals.push(n);
        }
    }
    let cells = u_knots
        .len()
        .checked_sub(1)
        .zip(v_knots.len().checked_sub(1))
        .and_then(|(u, v)| u.checked_mul(v))
        .ok_or_else(|| anyhow::anyhow!("parametric mesh cell count overflow"))?;
    let mut triangles = Vec::with_capacity(
        cells
            .checked_mul(2)
            .ok_or_else(|| anyhow::anyhow!("parametric triangle count overflow"))?,
    );
    let surface_orient = surface.orientation();
    let winding = surface_orient == orient;
    let stride = u_knots.len();
    for j in 0..v_knots.len() - 1 {
        for i in 0..u_knots.len() - 1 {
            let a = (j * stride + i) as u32;
            let b = a + 1;
            let c = ((j + 1) * stride + i + 1) as u32;
            let d = ((j + 1) * stride + i) as u32;
            let area = |x: u32, y: u32, z: u32| {
                let p = vertices[x as usize];
                let q = vertices[y as usize];
                let r = vertices[z as usize];
                let u = [q[0] - p[0], q[1] - p[1], q[2] - p[2]];
                let v = [r[0] - p[0], r[1] - p[1], r[2] - p[2]];
                (u[1] * v[2] - u[2] * v[1])
                    .hypot((u[2] * v[0] - u[0] * v[2]).hypot(u[0] * v[1] - u[1] * v[0]))
            };
            let collapsed = |x: u32, y: u32| {
                let p = vertices[x as usize];
                let q = vertices[y as usize];
                let d = [q[0] - p[0], q[1] - p[1], q[2] - p[2]];
                d[0] * d[0] + d[1] * d[1] + d[2] * d[2] <= 1e-20
            };
            let add_triangle =
                |x: u32, y: u32, z: u32, triangles: &mut Vec<[u32; 3]>| -> Result<()> {
                    let triangle_area = area(x, y, z);
                    if triangle_area <= 1e-12 {
                        ensure!(
                            collapsed(x, y) || collapsed(y, z) || collapsed(z, x),
                            "noncollapsed degenerate parametric triangle"
                        );
                    } else {
                        triangles.push(if winding { [x, y, z] } else { [x, z, y] });
                    }
                    Ok(())
                };
            add_triangle(a, b, c, &mut triangles)?;
            add_triangle(a, c, d, &mut triangles)?;
        }
    }
    ensure!(
        !triangles.is_empty(),
        "parametric surface produced no triangles"
    );
    Ok(ParametricMesh {
        vertices,
        normals,
        triangles,
    })
}
