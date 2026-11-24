//! Boolean difference of closed triangle meshes, by binary space partitioning
//! (Thibault and Naylor's method: each solid's faces are split by the other's
//! planes and kept by which side they fall on).
//!
//! A family's void forms cut the solids they meet; Revit's saved graphics
//! hold the result, and a family read from its forms (releases whose graphics
//! are not read) cuts them here. The result is a closed mesh where both
//! inputs are. A face split on one side of an edge and not on the other
//! leaves a T-junction (a point inside an edge of the neighbouring face);
//! [`conform`] splits those edges, so every edge of the result is met by its
//! reverse and the mesh reads as closed.

use std::collections::{BTreeMap, HashMap};

/// Points nearer a plane than this (feet) lie on it.
const EPSILON: f64 = 1e-6;

/// Polygons a cut may produce before it gives up (a guard against a split
/// cascade on dense meshes): the uncut solid is kept.
const MAX_POLYGONS: usize = 200_000;

/// Polygon classifications a cut may make in all, building and clipping
/// together, before it gives up (a tree can degenerate to a list, which
/// makes building quadratic while the polygon count stays small).
const MAX_WORK: usize = 20_000_000;

/// The work left; a cut that runs out returns `None`.
struct Budget(std::cell::Cell<usize>);

impl Budget {
    /// Charge one classification; false once nothing is left.
    fn spend(&self) -> bool {
        let left = self.0.get();
        self.0.set(left.saturating_sub(1));
        left > 0
    }
    fn exhausted(&self) -> bool {
        self.0.get() == 0
    }
}

type V3 = [f64; 3];

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn lerp(a: V3, b: V3, t: f64) -> V3 {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

#[derive(Clone, Copy)]
struct Plane {
    n: V3,
    w: f64,
}

impl Plane {
    fn of(p: &[V3]) -> Option<Plane> {
        // Newell's normal: robust for any planar polygon.
        let mut n = [0.0; 3];
        for (i, a) in p.iter().enumerate() {
            let b = p[(i + 1) % p.len()];
            n[0] += (a[1] - b[1]) * (a[2] + b[2]);
            n[1] += (a[2] - b[2]) * (a[0] + b[0]);
            n[2] += (a[0] - b[0]) * (a[1] + b[1]);
        }
        let len = dot(n, n).sqrt();
        if len < 1e-12 {
            return None;
        }
        let n = [n[0] / len, n[1] / len, n[2] / len];
        Some(Plane { n, w: dot(n, p[0]) })
    }
    fn flip(&mut self) {
        self.n = [-self.n[0], -self.n[1], -self.n[2]];
        self.w = -self.w;
    }
}

#[derive(Clone)]
struct Polygon {
    points: Vec<V3>,
    plane: Plane,
}

impl Polygon {
    fn flip(&mut self) {
        self.points.reverse();
        self.plane.flip();
    }
}

const COPLANAR: u8 = 0;
const FRONT: u8 = 1;
const BACK: u8 = 2;

/// Split `p` by `plane` into the four lists.
fn split(
    plane: &Plane,
    p: Polygon,
    coplanar_front: &mut Vec<Polygon>,
    coplanar_back: &mut Vec<Polygon>,
    front: &mut Vec<Polygon>,
    back: &mut Vec<Polygon>,
) {
    let mut kind = 0u8;
    let sides: Vec<u8> = p
        .points
        .iter()
        .map(|v| {
            let t = dot(plane.n, *v) - plane.w;
            let s = if t < -EPSILON {
                BACK
            } else if t > EPSILON {
                FRONT
            } else {
                COPLANAR
            };
            kind |= s;
            s
        })
        .collect();
    match kind {
        COPLANAR => {
            if dot(plane.n, p.plane.n) > 0.0 {
                coplanar_front.push(p)
            } else {
                coplanar_back.push(p)
            }
        }
        FRONT => front.push(p),
        BACK => back.push(p),
        _ => {
            let (mut f, mut b) = (Vec::new(), Vec::new());
            let n = p.points.len();
            for i in 0..n {
                let j = (i + 1) % n;
                let (si, sj) = (sides[i], sides[j]);
                let (vi, vj) = (p.points[i], p.points[j]);
                if si != BACK {
                    f.push(vi);
                }
                if si != FRONT {
                    b.push(vi);
                }
                if (si | sj) == (FRONT | BACK) {
                    let t = (plane.w - dot(plane.n, vi)) / dot(plane.n, sub(vj, vi));
                    let v = lerp(vi, vj, t);
                    f.push(v);
                    b.push(v);
                }
            }
            if f.len() >= 3 {
                front.push(Polygon {
                    points: f,
                    plane: p.plane,
                });
            }
            if b.len() >= 3 {
                back.push(Polygon {
                    points: b,
                    plane: p.plane,
                });
            }
        }
    }
}

/// A BSP tree, nodes in an arena (no recursion on build depth).
#[derive(Default)]
struct Tree {
    nodes: Vec<Node>,
}

struct Node {
    plane: Option<Plane>,
    polygons: Vec<Polygon>,
    front: Option<usize>,
    back: Option<usize>,
}

impl Tree {
    fn new(polygons: Vec<Polygon>, budget: &Budget) -> Tree {
        let mut t = Tree::default();
        t.nodes.push(Node {
            plane: None,
            polygons: Vec::new(),
            front: None,
            back: None,
        });
        t.build(0, polygons, budget);
        t
    }

    fn build(&mut self, root: usize, polygons: Vec<Polygon>, budget: &Budget) {
        let mut stack = vec![(root, polygons)];
        while let Some((at, polys)) = stack.pop() {
            if polys.is_empty() || budget.exhausted() {
                continue;
            }
            let plane = *self.nodes[at].plane.get_or_insert(polys[0].plane);
            let (mut cf, mut cb, mut f, mut b) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
            for p in polys {
                if !budget.spend() {
                    break;
                }
                split(&plane, p, &mut cf, &mut cb, &mut f, &mut b);
            }
            self.nodes[at].polygons.extend(cf);
            self.nodes[at].polygons.extend(cb);
            for (list, front) in [(f, true), (b, false)] {
                if list.is_empty() {
                    continue;
                }
                let child = if front {
                    self.nodes[at].front
                } else {
                    self.nodes[at].back
                };
                let child = child.unwrap_or_else(|| {
                    self.nodes.push(Node {
                        plane: None,
                        polygons: Vec::new(),
                        front: None,
                        back: None,
                    });
                    let c = self.nodes.len() - 1;
                    if front {
                        self.nodes[at].front = Some(c);
                    } else {
                        self.nodes[at].back = Some(c);
                    }
                    c
                });
                stack.push((child, list));
            }
        }
    }

    /// Turn the solid inside out.
    fn invert(&mut self) {
        for n in &mut self.nodes {
            for p in &mut n.polygons {
                p.flip();
            }
            if let Some(pl) = &mut n.plane {
                pl.flip();
            }
            std::mem::swap(&mut n.front, &mut n.back);
        }
    }

    /// The parts of `polygons` outside this solid.
    fn clip_polygons(&self, polygons: Vec<Polygon>, budget: &Budget) -> Vec<Polygon> {
        let mut out = Vec::new();
        let mut stack = vec![(0usize, polygons)];
        while let Some((at, polys)) = stack.pop() {
            let node = &self.nodes[at];
            let Some(plane) = node.plane else {
                out.extend(polys);
                continue;
            };
            let (mut cf, mut cb, mut f, mut b) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
            for p in polys {
                if !budget.spend() {
                    break;
                }
                split(&plane, p, &mut cf, &mut cb, &mut f, &mut b);
            }
            f.extend(cf);
            b.extend(cb);
            match node.front {
                Some(c) => stack.push((c, f)),
                None => out.extend(f),
            }
            // Behind a leaf is inside the solid: dropped.
            if let Some(c) = node.back {
                stack.push((c, b));
            }
        }
        out
    }

    /// Remove from this tree's polygons the parts inside `other`.
    fn clip_to(&mut self, other: &Tree, budget: &Budget) {
        for i in 0..self.nodes.len() {
            let polys = std::mem::take(&mut self.nodes[i].polygons);
            self.nodes[i].polygons = other.clip_polygons(polys, budget);
        }
    }

    fn all_polygons(&self) -> Vec<Polygon> {
        self.nodes
            .iter()
            .flat_map(|n| n.polygons.iter().cloned())
            .collect()
    }

    fn count(&self) -> usize {
        self.nodes.iter().map(|n| n.polygons.len()).sum()
    }
}

fn polygons(vertices: &[V3], triangles: &[[u32; 3]]) -> Vec<Polygon> {
    triangles
        .iter()
        .filter_map(|t| {
            let points: Vec<V3> = t.iter().map(|i| vertices[*i as usize]).collect();
            let plane = Plane::of(&points)?;
            Some(Polygon { points, plane })
        })
        .collect()
}

/// `a` minus `b`, as vertices and triangles, or `None` when the cut gives
/// up (see [`MAX_POLYGONS`]). Both must be closed, outward-wound meshes.
pub fn difference(
    a: (&[V3], &[[u32; 3]]),
    b: (&[V3], &[[u32; 3]]),
) -> Option<(Vec<V3>, Vec<[u32; 3]>)> {
    // Nothing minus anything is nothing.
    if a.1.is_empty() {
        return Some((Vec::new(), Vec::new()));
    }
    let budget = Budget(std::cell::Cell::new(MAX_WORK));
    let mut ta = Tree::new(polygons(a.0, a.1), &budget);
    let mut tb = Tree::new(polygons(b.0, b.1), &budget);
    ta.invert();
    ta.clip_to(&tb, &budget);
    tb.clip_to(&ta, &budget);
    tb.invert();
    tb.clip_to(&ta, &budget);
    tb.invert();
    if ta.count() + tb.count() > MAX_POLYGONS {
        return None;
    }
    ta.build(0, tb.all_polygons(), &budget);
    ta.invert();
    // The final merge splits the cutter's faces again: the result is
    // bounded too, and a cut that ran out of work is not a cut.
    if budget.exhausted() || ta.count() > MAX_POLYGONS {
        return None;
    }
    let mut vertices = Vec::new();
    let mut triangles = Vec::new();
    for p in ta.all_polygons() {
        let base = vertices.len() as u32;
        vertices.extend(p.points.iter().copied());
        for k in 1..p.points.len() as u32 - 1 {
            triangles.push([base, base + k, base + k + 1]);
        }
    }
    // Drop slivers the splits left with no area.
    triangles.retain(|t| {
        let [p, q, r] = t.map(|i| vertices[i as usize]);
        dot(cross(sub(q, p), sub(r, p)), cross(sub(q, p), sub(r, p))) > 1e-18
    });
    let (vertices, triangles, _) = conform(&vertices, &triangles, EPSILON);
    Some((vertices, triangles))
}

/// `triangles` with their T-junctions split: points nearer than `tolerance`
/// (feet) are welded into one, and each edge not met by as many reversed
/// edges is split at every point lying on it (within `tolerance`), its
/// triangle fanned around the new points (from the opposite corner where
/// one edge is split, else from its centroid). Returns the vertices, the
/// triangles and, per triangle, the index of the triangle it came from.
/// Edges that no point splits (a real gap) stay unmatched.
pub fn conform(
    vertices: &[V3],
    triangles: &[[u32; 3]],
    tolerance: f64,
) -> (Vec<V3>, Vec<[u32; 3]>, Vec<usize>) {
    let mut points: Vec<V3> = Vec::new();
    let mut welded: HashMap<[i64; 3], u32> = HashMap::new();
    let mut tris: Vec<[u32; 3]> = Vec::with_capacity(triangles.len());
    let mut from: Vec<usize> = Vec::with_capacity(triangles.len());
    for (i, t) in triangles.iter().enumerate() {
        let w = t.map(|v| {
            let p = vertices[v as usize];
            *welded
                .entry(p.map(|c| (c / tolerance).round() as i64))
                .or_insert_with(|| {
                    points.push(p);
                    (points.len() - 1) as u32
                })
        });
        if w[0] != w[1] && w[1] != w[2] && w[2] != w[0] {
            tris.push(w);
            from.push(i);
        }
    }
    drop(welded);
    // A split can expose another (two faces split the same edge at
    // different points): a few passes settle it.
    for _ in 0..4 {
        let mut count: HashMap<(u32, u32), i64> = HashMap::new();
        for t in &tris {
            for k in 0..3 {
                *count.entry((t[k], t[(k + 1) % 3])).or_default() += 1;
            }
        }
        let open = |a: u32, b: u32| count.get(&(a, b)) > count.get(&(b, a));
        let mut unmatched: Vec<(u32, u32)> =
            count.keys().copied().filter(|&(a, b)| open(a, b)).collect();
        if unmatched.is_empty() {
            break;
        }
        unmatched.sort_unstable();
        // Only an end of another unmatched edge can lie inside one.
        let mut ends: Vec<u32> = unmatched.iter().flat_map(|&(a, b)| [a, b]).collect();
        ends.sort_unstable();
        ends.dedup();
        ends.sort_unstable_by(|&a, &b| points[a as usize][0].total_cmp(&points[b as usize][0]));
        let mut splits: BTreeMap<(u32, u32), Vec<u32>> = BTreeMap::new();
        for &(a, b) in &unmatched {
            let (pa, pb) = (points[a as usize], points[b as usize]);
            let d = sub(pb, pa);
            let len2 = dot(d, d);
            let lo = pa[0].min(pb[0]) - tolerance;
            let hi = pa[0].max(pb[0]) + tolerance;
            let start = ends.partition_point(|&v| points[v as usize][0] < lo);
            let mut on: Vec<(f64, u32)> = ends[start..]
                .iter()
                .take_while(|&&v| points[v as usize][0] <= hi)
                .filter(|&&v| v != a && v != b)
                .filter_map(|&v| {
                    let pv = points[v as usize];
                    let t = dot(sub(pv, pa), d) / len2;
                    let foot = lerp(pa, pb, t);
                    let off = sub(pv, foot);
                    let margin = tolerance / len2.sqrt();
                    (t > margin && t < 1.0 - margin && dot(off, off) < tolerance * tolerance)
                        .then_some((t, v))
                })
                .collect();
            if !on.is_empty() {
                on.sort_by(|x, y| x.0.total_cmp(&y.0));
                splits.insert((a, b), on.into_iter().map(|(_, v)| v).collect());
            }
        }
        if splits.is_empty() {
            break;
        }
        let mut next = Vec::with_capacity(tris.len());
        let mut next_from = Vec::with_capacity(tris.len());
        for (t, &f) in tris.iter().zip(&from) {
            let inner: [Option<&Vec<u32>>; 3] =
                std::array::from_fn(|k| splits.get(&(t[k], t[(k + 1) % 3])));
            let split_edges: Vec<usize> = (0..3).filter(|&k| inner[k].is_some()).collect();
            if split_edges.is_empty() {
                next.push(*t);
                next_from.push(f);
                continue;
            }
            // The triangle's outline with the points on its edges, in order.
            let mut ring: Vec<u32> = Vec::new();
            for k in 0..3 {
                ring.push(t[k]);
                ring.extend(inner[k].into_iter().flatten());
            }
            let fan: Vec<[u32; 3]> = if let [k] = split_edges[..] {
                // From the corner opposite the one split edge.
                let apex = t[(k + 2) % 3];
                let at = ring.iter().position(|&v| v == apex).unwrap_or(0);
                let n = ring.len();
                (1..n - 1)
                    .map(|j| [apex, ring[(at + j) % n], ring[(at + j + 1) % n]])
                    .collect()
            } else {
                let c = [0, 1, 2].map(|k| points[t[k] as usize]);
                points.push(std::array::from_fn(|i| (c[0][i] + c[1][i] + c[2][i]) / 3.0));
                let centre = (points.len() - 1) as u32;
                (0..ring.len())
                    .map(|j| [centre, ring[j], ring[(j + 1) % ring.len()]])
                    .collect()
            };
            next_from.extend(std::iter::repeat_n(f, fan.len()));
            next.extend(fan);
        }
        tris = next;
        from = next_from;
    }
    (points, tris, from)
}
