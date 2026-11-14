//! Element bodies as triangle meshes and plan outlines, for the viewer's
//! GLB ([`super::gltf`]) and the plan SVG ([`super::sheet`]).
//!
//! They are the shapes the STEP writer emits for the same element, in the
//! same frame. An element's body is, in the writer's order of precedence, its
//! representation map's shape, its solid shape, or its extrusion. An
//! extrusion's profile lies in the element's local XY plane, centred on the
//! placement the way IFC4 centres each profile definition, and rises
//! `height_feet` along local +Z. The placement is `location_feet` turned
//! `rotation_radians` about +Z.
//!
//! Where a shape is approximated it is drawn no smaller than it is:
//! - a boolean difference or intersection draws its first operand;
//! - a swept path sweeps each directrix segment on its own, so segments
//!   overlap at their joins;
//! - a circle is a 24-gon around it, and a revolution turns in 24 steps per
//!   full turn;
//! - a profile that does not triangulate draws its bounding rectangle.

use super::entities::{
    Extrusion, IfcBooleanOp, IfcEntity, ProfileDef, RepresentationMap, SolidShape,
};
use crate::vector_math::{cross as cross3, dot, sub};

/// A profile point, feet.
pub type Point2 = (f64, f64);

/// A closed ring of profile points, without a repeated last point.
pub type Ring = Vec<Point2>;

/// Sides of a full circle or a full revolution.
pub const CIRCLE_SEGMENTS: usize = 24;

/// Points a profile may have, holes included, and still be triangulated.
/// Past it the profile draws its bounding rectangle.
pub const MAX_TRIANGULATED_POINTS: usize = 2048;

/// A triangle mesh. Triangles wind counter-clockwise seen from outside.
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct Mesh {
    pub vertices: Vec<[f64; 3]>,
    pub triangles: Vec<[u32; 3]>,
    /// per triangle, the index in the model's materials of the
    /// material Revit draws that face in ([`NO_FACE_MATERIAL`]: the
    /// element's own colour). Empty, or one per triangle.
    pub face_materials: Vec<usize>,
}

/// A [`Mesh::face_materials`] entry for a face drawn in its element's colour.
pub const NO_FACE_MATERIAL: usize = usize::MAX;

impl Mesh {
    /// Whether the mesh has no triangle.
    pub fn is_empty(&self) -> bool {
        self.triangles.is_empty()
    }

    fn append(&mut self, other: Mesh) {
        let Ok(base) = u32::try_from(self.vertices.len()) else {
            return;
        };
        if !self.face_materials.is_empty() || !other.face_materials.is_empty() {
            let pad = |m: &mut Vec<usize>, n: usize| m.resize(n, NO_FACE_MATERIAL);
            pad(&mut self.face_materials, self.triangles.len());
            let mut theirs = other.face_materials;
            pad(&mut theirs, other.triangles.len());
            self.face_materials.extend(theirs);
        }
        self.vertices.extend(other.vertices);
        self.triangles.extend(
            other
                .triangles
                .into_iter()
                .map(|[a, b, c]| [a + base, b + base, c + base]),
        );
    }

    /// A closed mesh (every edge shared by two triangles that traverse it in
    /// opposite directions) wound so its volume is positive, one side only
    /// (two-siding every saved mesh made 256,164 closed bodies
    /// count as zero volume); any other mesh two-sided.
    fn outward_or_two_sided(mut self) -> Mesh {
        let weld = |v: u32| -> [i64; 3] {
            let p = self.vertices[v as usize];
            std::array::from_fn(|k| (p[k] * 10_000.0).round() as i64)
        };
        let mut edges: std::collections::BTreeMap<([i64; 3], [i64; 3]), u32> = Default::default();
        for &[a, b, c] in &self.triangles {
            let (a, b, c) = (weld(a), weld(b), weld(c));
            for (s, t) in [(a, b), (b, c), (c, a)] {
                if s == t {
                    continue;
                }
                *edges.entry((s, t)).or_default() += 1;
            }
        }
        // Closed: every directed edge is traversed as often as its reverse.
        // A door's frame and panel are closed shells that touch along an
        // found every saved door and window mesh of rac_basic 2024 balanced
        // but failing the old "each edge once each way" test, so drawn twice.
        let closed = edges
            .iter()
            .all(|(&(s, t), &n)| edges.get(&(t, s)) == Some(&n));
        if !closed {
            return self.two_sided();
        }
        // Each edge-connected shell wound so its volume is positive.
        let n = self.triangles.len();
        let mut parent: Vec<usize> = (0..n).collect();
        fn find(parent: &mut [usize], mut i: usize) -> usize {
            while parent[i] != i {
                parent[i] = parent[parent[i]];
                i = parent[i];
            }
            i
        }
        // Shells are joined across manifold edges only (two triangles): an
        // edge where two shells touch carries four.
        let mut users: std::collections::BTreeMap<([i64; 3], [i64; 3]), Vec<usize>> =
            Default::default();
        for (i, &[a, b, c]) in self.triangles.iter().enumerate() {
            let (a, b, c) = (weld(a), weld(b), weld(c));
            for (s, t) in [(a, b), (b, c), (c, a)] {
                if s != t {
                    users
                        .entry(if s <= t { (s, t) } else { (t, s) })
                        .or_default()
                        .push(i);
                }
            }
        }
        for pair in users.values() {
            if let [i, j] = pair[..] {
                let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                parent[ri] = rj;
            }
        }
        let mut volume: std::collections::BTreeMap<usize, f64> = Default::default();
        for i in 0..n {
            let [a, b, c] = self.triangles[i];
            let (a, b, c) = (
                self.vertices[a as usize],
                self.vertices[b as usize],
                self.vertices[c as usize],
            );
            *volume.entry(find(&mut parent, i)).or_default() +=
                (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                    + a[2] * (b[0] * c[1] - b[1] * c[0]))
                    / 6.0;
        }
        for i in 0..n {
            if volume[&find(&mut parent, i)] < 0.0 {
                self.triangles[i].swap(1, 2);
            }
        }
        self
    }

    /// Every triangle also wound the other way, for a shape whose
    /// orientation is not known.
    fn two_sided(mut self) -> Mesh {
        let reversed: Vec<[u32; 3]> = self.triangles.iter().map(|&[a, b, c]| [a, c, b]).collect();
        self.triangles.extend(reversed);
        if !self.face_materials.is_empty() {
            self.face_materials.extend_from_within(..);
        }
        self
    }
}

/// What an element's body is, in the STEP writer's order of precedence.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum Body<'a> {
    Extrusion(&'a Extrusion),
    Solid(&'a SolidShape),
}

/// The body the mesh formats draw for entity `index`: its display body
/// ([`super::IfcModel::display_bodies`]: a host with its openings
/// cut), else [`element_body`].
pub fn drawn_body<'a>(
    model: &'a super::IfcModel,
    index: usize,
    entity: &'a IfcEntity,
) -> Option<Body<'a>> {
    model
        .display_bodies
        .get(&index)
        .map(Body::Solid)
        .or_else(|| element_body(entity, &model.representation_maps))
}

/// The body of `entity`: its representation map's shape, else its solid
/// shape, else its extrusion. `None` for an element with no body and for
/// every other entity.
pub fn element_body<'a>(entity: &'a IfcEntity, maps: &'a [RepresentationMap]) -> Option<Body<'a>> {
    let IfcEntity::BuildingElement {
        extrusion,
        solid_shape,
        representation_map_index,
        ..
    } = entity
    else {
        return None;
    };
    if let Some(map) = representation_map_index.and_then(|index| maps.get(index)) {
        return Some(Body::Solid(&map.shape));
    }
    if let Some(shape) = solid_shape.as_ref() {
        return Some(Body::Solid(shape));
    }
    extrusion.as_ref().map(Body::Extrusion)
}

/// The element placement: `location_feet` (or the origin) turned
/// `rotation_radians` about +Z.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct Placement {
    pub origin: [f64; 3],
    pub cos: f64,
    pub sin: f64,
}

impl Placement {
    /// The placement of an element's `location_feet` and `rotation_radians`.
    pub fn new(location: Option<[f64; 3]>, rotation: Option<f64>) -> Placement {
        let angle = rotation.filter(|r| r.is_finite()).unwrap_or(0.0);
        Placement {
            origin: location.unwrap_or([0.0; 3]),
            cos: angle.cos(),
            sin: angle.sin(),
        }
    }

    /// A local point in model coordinates.
    pub fn apply(&self, p: [f64; 3]) -> [f64; 3] {
        [
            self.origin[0] + self.cos * p[0] - self.sin * p[1],
            self.origin[1] + self.sin * p[0] + self.cos * p[1],
            self.origin[2] + p[2],
        ]
    }
}

fn same(a: Point2, b: Point2) -> bool {
    (a.0 - b.0).abs() <= 1e-9 && (a.1 - b.1).abs() <= 1e-9
}

pub(crate) fn signed_area(ring: &[Point2]) -> f64 {
    let n = ring.len();
    (0..n)
        .map(|i| {
            let (a, b) = (ring[i], ring[(i + 1) % n]);
            a.0 * b.1 - b.0 * a.1
        })
        .sum::<f64>()
        / 2.0
}

fn cross(a: Point2, b: Point2, c: Point2) -> f64 {
    (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
}

/// `ring` without repeated points and its closing point, or `None` when
/// that leaves no area or a value is not finite.
fn clean(ring: &[Point2]) -> Option<Ring> {
    let mut out: Ring = Vec::with_capacity(ring.len());
    for &p in ring {
        if !(p.0.is_finite() && p.1.is_finite()) {
            return None;
        }
        if out.last().is_some_and(|&q| same(q, p)) {
            continue;
        }
        out.push(p);
    }
    while out.len() > 1 && same(out[0], out[out.len() - 1]) {
        out.pop();
    }
    (out.len() >= 3 && signed_area(&out).abs() > 1e-12).then_some(out)
}

/// Whether two edges of `ring` that share no end cross each other.
fn crosses_itself(ring: &[Point2]) -> bool {
    let n = ring.len();
    let strictly_apart = |a: Point2, b: Point2, c: Point2, d: Point2| {
        let (d1, d2) = (cross(a, b, c), cross(a, b, d));
        let (d3, d4) = (cross(c, d, a), cross(c, d, b));
        d1 * d2 < 0.0 && d3 * d4 < 0.0
    };
    (0..n).any(|i| {
        ((i + 2)..n)
            .filter(|&j| (j + 1) % n != i)
            .any(|j| strictly_apart(ring[i], ring[(i + 1) % n], ring[j], ring[(j + 1) % n]))
    })
}

fn ring_bounds(ring: &[Point2]) -> (f64, f64, f64, f64) {
    ring.iter().fold(
        (
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ),
        |(x0, y0, x1, y1), &(x, y)| (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
    )
}

fn bounding_rectangle(ring: &[Point2]) -> Ring {
    let (x0, y0, x1, y1) = ring_bounds(ring);
    vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
}

/// Join each hole to the outer ring by a two-way bridge from the hole's
/// rightmost point to an outer point it sees, so one ring bounds the area.
/// `outer` winds counter-clockwise and each hole clockwise.
fn bridge_holes(outer: Ring, mut holes: Vec<Ring>) -> Option<Ring> {
    let max_x = |ring: &Ring| ring.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
    holes.sort_by(|a, b| max_x(b).total_cmp(&max_x(a)));
    let mut poly = outer;
    for hole in holes {
        let (start, m) = hole
            .iter()
            .copied()
            .enumerate()
            .max_by(|a, b| a.1.0.total_cmp(&b.1.0))?;
        // The nearest outer edge the ray from `m` towards +X crosses, and
        // that edge's rightmost end.
        let n = poly.len();
        let mut hit: Option<(f64, usize)> = None;
        for i in 0..n {
            let (a, b) = (poly[i], poly[(i + 1) % n]);
            if a.1 == b.1 || m.1 < a.1.min(b.1) || m.1 > a.1.max(b.1) {
                continue;
            }
            let x = a.0 + (m.1 - a.1) * (b.0 - a.0) / (b.1 - a.1);
            if x < m.0 || hit.is_some_and(|(best, _)| x >= best) {
                continue;
            }
            hit = Some((x, if a.0 >= b.0 { i } else { (i + 1) % n }));
        }
        let (x, mut join) = hit?;
        // An outer point inside the triangle (m, crossing, join end) could
        // hide the join end; the one nearest the ray's direction is seen.
        let crossing = (x, m.1);
        let end = poly[join];
        if !same(end, crossing) {
            let (a, b, c) = if cross(m, crossing, end) >= 0.0 {
                (m, crossing, end)
            } else {
                (m, end, crossing)
            };
            let mut best = f64::NEG_INFINITY;
            for (i, &p) in poly.iter().enumerate() {
                if i == join || same(p, m) {
                    continue;
                }
                if cross(a, b, p) > 0.0 && cross(b, c, p) > 0.0 && cross(c, a, p) > 0.0 {
                    let d = ((p.0 - m.0).powi(2) + (p.1 - m.1).powi(2)).sqrt();
                    let alignment = (p.0 - m.0) / d.max(f64::MIN_POSITIVE);
                    if alignment > best {
                        best = alignment;
                        join = i;
                    }
                }
            }
        }
        let mut merged = Vec::with_capacity(poly.len() + hole.len() + 2);
        merged.extend_from_slice(&poly[..=join]);
        merged.extend(hole[start..].iter().copied());
        merged.extend(hole[..=start].iter().copied());
        merged.push(poly[join]);
        merged.extend_from_slice(&poly[join + 1..]);
        poly = merged;
    }
    Some(poly)
}

/// Triangles covering the counter-clockwise polygon `poly`, as indices into
/// it. `None` when no ear can be cut, which a self-intersecting polygon
/// leads to.
fn ear_clip(poly: &[Point2]) -> Option<Vec<[u32; 3]>> {
    let mut left: Vec<usize> = (0..poly.len()).collect();
    let mut triangles = Vec::with_capacity(poly.len().saturating_sub(2));
    let mut from = 0;
    while left.len() > 3 {
        let m = left.len();
        let ear = (0..m).map(|k| (from + k) % m).find(|&k| {
            let (ia, ib, ic) = (left[(k + m - 1) % m], left[k], left[(k + 1) % m]);
            let (a, b, c) = (poly[ia], poly[ib], poly[ic]);
            cross(a, b, c) > 0.0
                && !left.iter().any(|&j| {
                    let p = poly[j];
                    j != ia
                        && j != ib
                        && j != ic
                        && !same(p, a)
                        && !same(p, b)
                        && !same(p, c)
                        && cross(a, b, p) >= 0.0
                        && cross(b, c, p) >= 0.0
                        && cross(c, a, p) >= 0.0
                })
        });
        match ear {
            Some(k) => {
                let (ia, ib, ic) = (left[(k + m - 1) % m], left[k], left[(k + 1) % m]);
                triangles.push([
                    u32::try_from(ia).ok()?,
                    u32::try_from(ib).ok()?,
                    u32::try_from(ic).ok()?,
                ]);
                left.remove(k);
                from = k;
            }
            None => {
                // A point on the line through its neighbours bounds no
                // area; dropping it can free an ear.
                let k = (0..m).find(|&k| {
                    let (a, b, c) = (
                        poly[left[(k + m - 1) % m]],
                        poly[left[k]],
                        poly[left[(k + 1) % m]],
                    );
                    cross(a, b, c).abs() <= 1e-12
                })?;
                left.remove(k);
            }
        }
    }
    if let [ia, ib, ic] = left[..] {
        if cross(poly[ia], poly[ib], poly[ic]) > 0.0 {
            triangles.push([
                u32::try_from(ia).ok()?,
                u32::try_from(ib).ok()?,
                u32::try_from(ic).ok()?,
            ]);
        }
    }
    Some(triangles)
}

/// A profile ready to extrude or sweep: its rings, outer counter-clockwise
/// and holes clockwise, and a triangulation of the area between them.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Section {
    pub rings: Vec<Ring>,
    pub cap_points: Ring,
    pub cap_triangles: Vec<[u32; 3]>,
}

/// Triangulate the area inside `outer` and outside `holes`. A hole that
/// cannot be joined drops out, and a profile that still does not
/// triangulate (or has more than [`MAX_TRIANGULATED_POINTS`]) becomes its
/// bounding rectangle, so the section is never smaller than the profile.
pub fn section(outer: &[Point2], holes: &[Ring]) -> Option<Section> {
    let mut outer = clean(outer)?;
    if signed_area(&outer) < 0.0 {
        outer.reverse();
    }
    let holes: Vec<Ring> = holes
        .iter()
        .filter_map(|hole| clean(hole))
        .map(|mut hole| {
            if signed_area(&hole) > 0.0 {
                hole.reverse();
            }
            hole
        })
        .collect();
    let points = outer.len() + holes.iter().map(|h| h.len() + 2).sum::<usize>();
    if points <= MAX_TRIANGULATED_POINTS && !crosses_itself(&outer) {
        let holes: Vec<Ring> = holes.into_iter().filter(|h| !crosses_itself(h)).collect();
        if let Some(poly) = bridge_holes(outer.clone(), holes.clone()) {
            if let Some(triangles) = ear_clip(&poly) {
                let mut rings = vec![outer];
                rings.extend(holes);
                return Some(Section {
                    rings,
                    cap_points: poly,
                    cap_triangles: triangles,
                });
            }
        }
        if let Some(triangles) = ear_clip(&outer) {
            return Some(Section {
                rings: vec![outer.clone()],
                cap_points: outer,
                cap_triangles: triangles,
            });
        }
    }
    let rectangle = bounding_rectangle(&outer);
    Some(Section {
        rings: vec![rectangle.clone()],
        cap_points: rectangle,
        cap_triangles: vec![[0, 1, 2], [0, 2, 3]],
    })
}

fn circle(radius: f64) -> Ring {
    // Around the circle: the polygon's sides touch it.
    let r = radius / (std::f64::consts::PI / CIRCLE_SEGMENTS as f64).cos();
    (0..CIRCLE_SEGMENTS)
        .map(|i| {
            let a = std::f64::consts::TAU * i as f64 / CIRCLE_SEGMENTS as f64;
            (r * a.cos(), r * a.sin())
        })
        .collect()
}

fn rectangle(width: f64, depth: f64) -> Ring {
    let (w, d) = (width / 2.0, depth / 2.0);
    vec![(-w, -d), (w, -d), (w, d), (-w, d)]
}

/// The outer ring and holes of a profile, centred as IFC4 centres it: a
/// parameterised profile on the centre of its bounding box, an arbitrary
/// one at its own coordinates. `None` for a profile with a non-positive or
/// non-finite dimension.
pub fn profile_rings(profile: &ProfileDef) -> Option<(Ring, Vec<Ring>)> {
    let positive = |values: &[f64]| values.iter().all(|v| v.is_finite() && *v > 0.0);
    let rings = match profile {
        ProfileDef::Rectangle {
            width_feet,
            depth_feet,
        } => {
            positive(&[*width_feet, *depth_feet]).then_some(())?;
            (rectangle(*width_feet, *depth_feet), Vec::new())
        }
        ProfileDef::Circle { radius_feet } => {
            positive(&[*radius_feet]).then_some(())?;
            (circle(*radius_feet), Vec::new())
        }
        ProfileDef::IShape {
            overall_width_feet: w,
            overall_depth_feet: d,
            web_thickness_feet: tw,
            flange_thickness_feet: tf,
        } => {
            positive(&[*w, *d, *tw, *tf]).then_some(())?;
            let (x, y, t, f) = (w / 2.0, d / 2.0, tw / 2.0, *tf);
            (
                vec![
                    (-x, -y),
                    (x, -y),
                    (x, -y + f),
                    (t, -y + f),
                    (t, y - f),
                    (x, y - f),
                    (x, y),
                    (-x, y),
                    (-x, y - f),
                    (-t, y - f),
                    (-t, -y + f),
                    (-x, -y + f),
                ],
                Vec::new(),
            )
        }
        ProfileDef::TShape {
            overall_depth_feet: d,
            flange_width_feet: w,
            web_thickness_feet: tw,
            flange_thickness_feet: tf,
        } => {
            positive(&[*w, *d, *tw, *tf]).then_some(())?;
            let (x, y, t, f) = (w / 2.0, d / 2.0, tw / 2.0, *tf);
            (
                vec![
                    (-t, -y),
                    (t, -y),
                    (t, y - f),
                    (x, y - f),
                    (x, y),
                    (-x, y),
                    (-x, y - f),
                    (-t, y - f),
                ],
                Vec::new(),
            )
        }
        ProfileDef::LShape {
            overall_depth_feet: d,
            overall_width_feet: w,
            thickness_feet: t,
        } => {
            positive(&[*w, *d, *t]).then_some(())?;
            let (x, y) = (w / 2.0, d / 2.0);
            (
                vec![
                    (-x, -y),
                    (x, -y),
                    (x, -y + t),
                    (-x + t, -y + t),
                    (-x + t, y),
                    (-x, y),
                ],
                Vec::new(),
            )
        }
        ProfileDef::UShape {
            overall_depth_feet: d,
            flange_width_feet: w,
            web_thickness_feet: tw,
            flange_thickness_feet: tf,
        } => {
            positive(&[*w, *d, *tw, *tf]).then_some(())?;
            let (x, y) = (w / 2.0, d / 2.0);
            (
                vec![
                    (-x, -y),
                    (x, -y),
                    (x, -y + tf),
                    (-x + tw, -y + tf),
                    (-x + tw, y - tf),
                    (x, y - tf),
                    (x, y),
                    (-x, y),
                ],
                Vec::new(),
            )
        }
        ProfileDef::RectangleHollow {
            overall_width_feet: w,
            overall_depth_feet: d,
            wall_thickness_feet: t,
        } => {
            positive(&[*w, *d, *t]).then_some(())?;
            let inner = (w - 2.0 * t, d - 2.0 * t);
            let holes = if inner.0 > 0.0 && inner.1 > 0.0 {
                vec![rectangle(inner.0, inner.1)]
            } else {
                Vec::new()
            };
            (rectangle(*w, *d), holes)
        }
        ProfileDef::CircleHollow {
            radius_feet: r,
            wall_thickness_feet: t,
        } => {
            positive(&[*r, *t]).then_some(())?;
            // The hole is inside its polygon, so the tube is not drawn
            // thinner than it is.
            let holes = if r - t > 0.0 {
                let inner = (r - t) * (std::f64::consts::PI / CIRCLE_SEGMENTS as f64).cos();
                vec![circle(inner)]
            } else {
                Vec::new()
            };
            (circle(*r), holes)
        }
        ProfileDef::ArbitraryClosed { points } => (points.clone(), Vec::new()),
        ProfileDef::ArbitraryWithVoids { points, voids } => (points.clone(), voids.clone()),
        ProfileDef::IndexedPolyCurve { points, segments } => {
            (indexed_poly_curve_ring(points, segments)?, Vec::new())
        }
        ProfileDef::IndexedPolyCurveWithVoids { outer, voids } => (
            indexed_poly_curve_ring(&outer.points, &outer.segments)?,
            voids
                .iter()
                .map(|void| indexed_poly_curve_ring(&void.points, &void.segments))
                .collect::<Option<Vec<_>>>()?,
        ),
    };
    Some(rings)
}

/// An indexed poly-curve's ring: each line's start, and each arc from its
/// start towards its end in steps of at most a [`CIRCLE_SEGMENTS`]th of a
/// turn about the circle through its three points.
fn indexed_poly_curve_ring(
    points: &[Point2],
    segments: &[super::entities::CurveSegment],
) -> Option<Ring> {
    use super::entities::CurveSegment;
    let mut ring = Ring::new();
    for segment in segments {
        match *segment {
            CurveSegment::Line([a, _]) => ring.push(*points.get(a)?),
            CurveSegment::Arc([a, m, b]) => {
                let (p, q, r) = (*points.get(a)?, *points.get(m)?, *points.get(b)?);
                // The circle through p, q and r.
                let d = 2.0 * (p.0 * (q.1 - r.1) + q.0 * (r.1 - p.1) + r.0 * (p.1 - q.1));
                if d.abs() < 1e-12 {
                    ring.push(p);
                    continue;
                }
                let n = |v: Point2| v.0 * v.0 + v.1 * v.1;
                let c = (
                    (n(p) * (q.1 - r.1) + n(q) * (r.1 - p.1) + n(r) * (p.1 - q.1)) / d,
                    (n(p) * (r.0 - q.0) + n(q) * (p.0 - r.0) + n(r) * (q.0 - p.0)) / d,
                );
                let angle = |v: Point2| (v.1 - c.1).atan2(v.0 - c.0);
                let radius = (p.0 - c.0).hypot(p.1 - c.1);
                // Anticlockwise when r lies to the left of p to q.
                let turn = std::f64::consts::TAU;
                let anticlockwise = d > 0.0;
                let sweep = |from: f64, to: f64| {
                    let s = (to - from).rem_euclid(turn);
                    if anticlockwise { s } else { s - turn }
                };
                let total = sweep(angle(p), angle(r));
                let steps =
                    ((total.abs() / (turn / CIRCLE_SEGMENTS as f64)).ceil() as usize).max(1);
                let start = angle(p);
                for i in 0..steps {
                    let a = start + total * i as f64 / steps as f64;
                    ring.push((c.0 + radius * a.cos(), c.1 + radius * a.sin()));
                }
            }
        }
    }
    (ring.len() >= 3).then_some(ring)
}

/// The rings of an extrusion's profile: its override, else its width ×
/// depth rectangle.
pub fn extrusion_rings(extrusion: &Extrusion) -> Option<(Ring, Vec<Ring>)> {
    match &extrusion.profile_override {
        Some(profile) => profile_rings(profile),
        None => profile_rings(&ProfileDef::Rectangle {
            width_feet: extrusion.width_feet,
            depth_feet: extrusion.depth_feet,
        }),
    }
}

/// The prism of `section` between two copies of it: `at(point, 0.0)` and
/// `at(point, 1.0)`. `at` must keep orientation (a right-handed map), so
/// the second copy faces away from the first.
fn prism(section: &Section, at: impl Fn(Point2, f64) -> [f64; 3]) -> Mesh {
    let mut mesh = Mesh::default();
    let n = section.cap_points.len() as u32;
    for t in [0.0, 1.0] {
        mesh.vertices
            .extend(section.cap_points.iter().map(|&p| at(p, t)));
    }
    for &[a, b, c] in &section.cap_triangles {
        mesh.triangles.push([a, c, b]);
        mesh.triangles.push([a + n, b + n, c + n]);
    }
    for ring in &section.rings {
        for i in 0..ring.len() {
            let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
            let base = mesh.vertices.len() as u32;
            mesh.vertices
                .extend([at(a, 0.0), at(b, 0.0), at(b, 1.0), at(a, 1.0)]);
            mesh.triangles.push([base, base + 1, base + 2]);
            mesh.triangles.push([base, base + 2, base + 3]);
        }
    }
    mesh
}

/// A slab over the plan area inside `outer` and outside `holes` whose
/// underside lies at `bottom(point)` and whose top lies `thickness` above
/// it, with upright sides.
pub fn sloped_slab_mesh(
    outer: &[Point2],
    holes: &[Ring],
    bottom: impl Fn(Point2) -> f64,
    thickness: f64,
) -> Option<Mesh> {
    if !(thickness.is_finite() && thickness > 0.0) {
        return None;
    }
    let section = section(outer, holes)?;
    let mesh = prism(&section, |p, t| [p.0, p.1, bottom(p) + t * thickness]);
    mesh.vertices
        .iter()
        .flatten()
        .all(|v| v.is_finite())
        .then_some(mesh)
}

/// A solid over the plan area inside `outer` and outside `holes` between the
/// surfaces `bottom(point)` and `top(point)`, with upright sides: a
/// wall whose base follows the top of the footing it stands on. `None`
/// unless `top` lies above `bottom` at every vertex of the plan.
pub fn between_surfaces_mesh(
    outer: &[Point2],
    holes: &[Ring],
    bottom: impl Fn(Point2) -> f64,
    top: impl Fn(Point2) -> f64,
) -> Option<Mesh> {
    let section = section(outer, holes)?;
    let rises = section
        .cap_points
        .iter()
        .all(|&p| top(p) - bottom(p) > 1e-9);
    if !rises {
        return None;
    }
    let mesh = prism(&section, |p, t| {
        let low = bottom(p);
        [p.0, p.1, low + t * (top(p) - low)]
    });
    mesh.vertices
        .iter()
        .flatten()
        .all(|v| v.is_finite())
        .then_some(mesh)
}

/// A plane `z = a·x + b·y + c` over the plan, as `[a, b, c]`.
pub type PlanPlane = [f64; 3];

fn plane_at([a, b, c]: PlanPlane, (x, y): Point2) -> f64 {
    a * x + b * y + c
}

/// How close to a splitting line a point counts as on it, feet.
const ENVELOPE_SPLIT_EPS: f64 = 1e-9;

/// Most planes [`envelope_slab_mesh`] takes; a roof sketch has far fewer
/// defining edges.
pub const MAX_ENVELOPE_PLANES: usize = 32;

/// Most pieces [`envelope_slab_mesh`] cuts the plan into, and most side
/// segments it draws: past either it draws nothing, which bounds the work
/// and memory a hostile sketch can ask for.
pub const MAX_ENVELOPE_PIECES: usize = 1 << 16;

/// Most points those pieces hold in all: sixteen a piece at the piece cap,
/// where a convex piece of a roof's plan has a handful.
pub const MAX_ENVELOPE_POINTS: usize = 16 * MAX_ENVELOPE_PIECES;

/// The point where the segment `p`–`q` crosses the line `f = 0`, from the
/// values `fp` and `fq` of `f` at its ends. The ends are taken in a fixed
/// order, so two triangles sharing the segment get the same point.
fn crossing(p: Point2, fp: f64, q: Point2, fq: f64) -> Point2 {
    let ((p, fp), (q, fq)) = if p <= q {
        ((p, fp), (q, fq))
    } else {
        ((q, fq), (p, fp))
    };
    let t = fp / (fp - fq);
    (p.0 + t * (q.0 - p.0), p.1 + t * (q.1 - p.1))
}

/// The part of the convex polygon `poly` where `f >= 0` (`keep_positive`)
/// or `f <= 0`.
fn clip_convex(poly: &[Point2], line: PlanPlane, keep_positive: bool) -> Vec<Point2> {
    let side = |p: Point2| {
        let f = plane_at(line, p);
        if keep_positive { f } else { -f }
    };
    let mut out = Vec::with_capacity(poly.len() + 1);
    for i in 0..poly.len() {
        let (p, q) = (poly[i], poly[(i + 1) % poly.len()]);
        let (fp, fq) = (side(p), side(q));
        if fp >= -ENVELOPE_SPLIT_EPS {
            out.push(p);
        }
        if (fp > ENVELOPE_SPLIT_EPS && fq < -ENVELOPE_SPLIT_EPS)
            || (fp < -ENVELOPE_SPLIT_EPS && fq > ENVELOPE_SPLIT_EPS)
        {
            out.push(crossing(p, plane_at(line, p), q, plane_at(line, q)));
        }
    }
    out
}

/// The plan inside `outer` and outside `holes`, cut into convex pieces,
/// counter-clockwise, along every line where two of the `tops` meet, two of
/// the `undersides` meet, an underside meets its floor, or a domain ends:
/// top and underside are flat on each piece. With the section and the lines.
/// `None` past [`MAX_ENVELOPE_PIECES`] pieces.
fn envelope_pieces(
    outer: &[Point2],
    holes: &[Ring],
    tops: &[PlanPlane],
    undersides: &[(PlanPlane, f64)],
    domains: &[Vec<PlanPlane>],
    bottom_domains: &[Vec<PlanPlane>],
) -> Option<(Section, Vec<PlanPlane>, Vec<Vec<Point2>>)> {
    let section = section(outer, holes)?;
    let difference = |p: PlanPlane, q: PlanPlane| [p[0] - q[0], p[1] - q[1], p[2] - q[2]];
    let mut lines: Vec<PlanPlane> = Vec::new();
    for (i, &p) in tops.iter().enumerate() {
        lines.extend(tops[i + 1..].iter().map(|&q| difference(p, q)));
    }
    for (i, &(p, _)) in undersides.iter().enumerate() {
        lines.extend(undersides[i + 1..].iter().map(|&(q, _)| difference(p, q)));
        lines.extend(
            undersides
                .iter()
                .map(|&(_, floor)| difference(p, [0.0, 0.0, floor])),
        );
    }
    lines.extend(domains.iter().flatten().copied());
    if !std::ptr::eq(bottom_domains, domains) {
        lines.extend(bottom_domains.iter().flatten().copied());
    }
    lines.retain(|l| l[0].hypot(l[1]) > ENVELOPE_SPLIT_EPS);
    // The cap's triangles, counter-clockwise, cut by every line.
    let mut pieces: Vec<Vec<Point2>> = section
        .cap_triangles
        .iter()
        .map(|&[a, b, c]| {
            let tri = [a, b, c].map(|i| section.cap_points[i as usize]);
            let mut tri = tri.to_vec();
            if signed_area(&tri) < 0.0 {
                tri.reverse();
            }
            tri
        })
        .collect();
    for &line in &lines {
        pieces = pieces
            .into_iter()
            .flat_map(|poly| {
                let values: Vec<f64> = poly.iter().map(|&p| plane_at(line, p)).collect();
                let above = values.iter().any(|&f| f > ENVELOPE_SPLIT_EPS);
                let below = values.iter().any(|&f| f < -ENVELOPE_SPLIT_EPS);
                if above && below {
                    vec![
                        clip_convex(&poly, line, true),
                        clip_convex(&poly, line, false),
                    ]
                } else {
                    vec![poly]
                }
            })
            .filter(|poly| poly.len() >= 3 && signed_area(poly).abs() > 1e-12)
            .collect();
        // A split whose other half is a sliver adds two points to a piece and
        // no piece, so the count alone let a crafted sketch reach 8,000 points
        // a piece at the cap. Summing them costs what the split did.
        if pieces.len() > MAX_ENVELOPE_PIECES
            || pieces.iter().map(Vec::len).sum::<usize>() > MAX_ENVELOPE_POINTS
        {
            crate::budget::stopped("envelope_pieces");
            return None;
        }
    }
    Some((section, lines, pieces))
}

/// A slab over the plan area inside `outer` and outside `holes` whose top
/// is the lowest of the `tops` planes at each point and whose underside is
/// the lowest of the `undersides` planes, each never below its own floor
/// height, with upright sides: a gable or hip roof, each plane
/// rising from its own edge and cut level at its floor. Plane `i` counts
/// only where every half-plane in `domains[i]` is not negative: its edge's
/// side, and the wedge between the lines where it meets its neighbouring
/// edges' planes, which converge at a hip and part at a valley. The plan is
/// cut along every line where two of the planes it compares meet, or a
/// domain ends, so top and underside are flat on each piece. `None` for no
/// plane, more than [`MAX_ENVELOPE_PLANES`], more than
/// [`MAX_ENVELOPE_PIECES`] pieces or sides, a point no plane covers, or a top
/// below the underside anywhere.
pub fn envelope_slab_mesh(
    outer: &[Point2],
    holes: &[Ring],
    tops: &[PlanPlane],
    undersides: &[(PlanPlane, f64)],
    domains: &[Vec<PlanPlane>],
) -> Option<Mesh> {
    envelope_slab_mesh_split(outer, holes, tops, undersides, domains, None)
}

/// [`envelope_slab_mesh`] with the underside's planes counting within
/// `bottom_domains`, where given, rather than the same domains as the top's
///: where each plane's top lies a different height above its
/// underside, the lines where the tops meet and the lines where the
/// undersides meet differ, and so do the wedges each surface is cut into.
pub(super) fn envelope_slab_mesh_split(
    outer: &[Point2],
    holes: &[Ring],
    tops: &[PlanPlane],
    undersides: &[(PlanPlane, f64)],
    domains: &[Vec<PlanPlane>],
    bottom_domains: Option<&[Vec<PlanPlane>]>,
) -> Option<Mesh> {
    if tops.is_empty()
        || tops.len() != undersides.len()
        || tops.len() != domains.len()
        || bottom_domains.is_some_and(|b| b.len() != tops.len())
        || tops.len() > MAX_ENVELOPE_PLANES
    {
        return None;
    }
    let bottom_domains = bottom_domains.unwrap_or(domains);
    let (section, lines, pieces) =
        envelope_pieces(outer, holes, tops, undersides, domains, bottom_domains)?;
    let covers_in = |dom: &[Vec<PlanPlane>], i: usize, p: Point2| {
        dom[i].iter().all(|&d| plane_at(d, p) >= -1e-7)
    };
    let top = |p: Point2| {
        (0..tops.len())
            .filter(|&i| covers_in(domains, i, p))
            .map(|i| plane_at(tops[i], p))
            .fold(f64::INFINITY, f64::min)
    };
    let bottom = |p: Point2| {
        (0..undersides.len())
            .filter(|&i| covers_in(bottom_domains, i, p))
            .map(|i| plane_at(undersides[i].0, p).max(undersides[i].1))
            .fold(f64::INFINITY, f64::min)
    };
    envelope_mesh(&section, &lines, &pieces, top, bottom)
}

/// A slab over the plan area inside `outer` made of `parts`, each a set of
/// planes rising from a pair of facing edges over a region: `(domain, tops,
/// undersides)`, the region where every half-plane of `domain` is not
/// negative. A part's top is the lowest of its `tops` and its underside the
/// lowest of its `undersides`, never below 0; where parts overlap, the
/// highest part's top and the highest part's underside win, as gables run
/// into one another. `None` as [`envelope_slab_mesh`], or where a point lies
/// in no part.
pub(super) fn envelope_parts_mesh(
    outer: &[Point2],
    parts: &[(Vec<PlanPlane>, Vec<PlanPlane>, Vec<PlanPlane>)],
) -> Option<Mesh> {
    let tops: Vec<PlanPlane> = parts.iter().flat_map(|part| part.1.clone()).collect();
    let undersides: Vec<(PlanPlane, f64)> = parts
        .iter()
        .flat_map(|part| part.2.iter().map(|&plane| (plane, 0.0)))
        .collect();
    if parts.is_empty()
        || parts
            .iter()
            .any(|part| part.1.is_empty() || part.2.is_empty())
        || tops.len() > MAX_ENVELOPE_PLANES
        || undersides.len() > MAX_ENVELOPE_PLANES
    {
        return None;
    }
    let domains: Vec<Vec<PlanPlane>> = parts.iter().map(|part| part.0.clone()).collect();
    let (section, lines, pieces) =
        envelope_pieces(outer, &[], &tops, &undersides, &domains, &domains)?;
    // The highest part's lowest plane: of its tops (`under` false) or its
    // undersides.
    let highest = |p: Point2, under: bool| {
        parts
            .iter()
            .filter(|part| part.0.iter().all(|&d| plane_at(d, p) >= -1e-7))
            .map(|part| {
                let planes = if under { &part.2 } else { &part.1 };
                planes
                    .iter()
                    .map(|&plane| plane_at(plane, p))
                    .fold(f64::INFINITY, f64::min)
            })
            .fold(f64::NEG_INFINITY, f64::max)
    };
    let top = |p: Point2| highest(p, false);
    let bottom = |p: Point2| highest(p, true).max(0.0);
    envelope_mesh(&section, &lines, &pieces, top, bottom)
}

/// The slab over `pieces` and up the sides of `section`'s rings, cut where
/// `lines` cross them, between `bottom` and `top`. `None` past
/// [`MAX_ENVELOPE_PIECES`] sides, at a point with no finite top, or with a top
/// below the underside anywhere.
fn envelope_mesh(
    section: &Section,
    lines: &[PlanPlane],
    pieces: &[Vec<Point2>],
    top: impl Fn(Point2) -> f64,
    bottom: impl Fn(Point2) -> f64,
) -> Option<Mesh> {
    let mut mesh = Mesh::default();
    let push = |mesh: &mut Mesh, points: [[f64; 3]; 3]| {
        let base = mesh.vertices.len() as u32;
        mesh.vertices.extend(points);
        mesh.triangles.push([base, base + 1, base + 2]);
    };
    for poly in pieces {
        for i in 1..poly.len() - 1 {
            let [a, b, c] = [poly[0], poly[i], poly[i + 1]];
            let lift = |p: Point2, z: f64| [p.0, p.1, z];
            push(
                &mut mesh,
                [lift(a, top(a)), lift(b, top(b)), lift(c, top(c))],
            );
            push(
                &mut mesh,
                [lift(a, bottom(a)), lift(c, bottom(c)), lift(b, bottom(b))],
            );
        }
    }
    // The sides, each ring edge cut where a line crosses it.
    let mut sides = 0usize;
    for ring in &section.rings {
        for i in 0..ring.len() {
            let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
            let mut cuts: Vec<f64> = vec![0.0, 1.0];
            for &line in lines {
                let (fa, fb) = (plane_at(line, a), plane_at(line, b));
                if (fa > ENVELOPE_SPLIT_EPS && fb < -ENVELOPE_SPLIT_EPS)
                    || (fa < -ENVELOPE_SPLIT_EPS && fb > ENVELOPE_SPLIT_EPS)
                {
                    cuts.push(fa / (fa - fb));
                }
            }
            cuts.sort_by(f64::total_cmp);
            cuts.dedup_by(|x, y| (*x - *y).abs() <= 1e-12);
            sides += cuts.len() - 1;
            if sides > MAX_ENVELOPE_PIECES {
                crate::budget::stopped("envelope_pieces");
                return None;
            }
            let at = |t: f64| (a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1));
            for pair in cuts.windows(2) {
                let (p, q) = (at(pair[0]), at(pair[1]));
                let base = mesh.vertices.len() as u32;
                mesh.vertices.extend([
                    [p.0, p.1, bottom(p)],
                    [q.0, q.1, bottom(q)],
                    [q.0, q.1, top(q)],
                    [p.0, p.1, top(p)],
                ]);
                mesh.triangles.push([base, base + 1, base + 2]);
                mesh.triangles.push([base, base + 2, base + 3]);
            }
        }
    }
    let thick_enough = pieces
        .iter()
        .flatten()
        .chain(section.rings.iter().flatten())
        .all(|&p| top(p).is_finite() && top(p) >= bottom(p) - 1e-9);
    (thick_enough && mesh.vertices.iter().flatten().all(|v| v.is_finite())).then_some(mesh)
}

/// [`envelope_slab_mesh`]'s slab cut into one part per plane: plane `i`'s
/// part is the plan where its top is the lowest of the planes that count,
/// with its own top and underside (never below its floor) over it and upright
/// sides all round, as Revit's export writes a gable or hip roof's slabs. A
/// plane that is nowhere the lowest gets an empty part. `None` where
/// [`envelope_slab_mesh`] would be.
pub fn envelope_part_meshes(
    outer: &[Point2],
    holes: &[Ring],
    tops: &[PlanPlane],
    undersides: &[(PlanPlane, f64)],
    domains: &[Vec<PlanPlane>],
) -> Option<Vec<Mesh>> {
    if tops.is_empty()
        || tops.len() != undersides.len()
        || tops.len() != domains.len()
        || tops.len() > MAX_ENVELOPE_PLANES
    {
        return None;
    }
    let (_, _, pieces) = envelope_pieces(outer, holes, tops, undersides, domains, domains)?;
    let covers = |i: usize, p: Point2| domains[i].iter().all(|&d| plane_at(d, p) >= -1e-7);
    let mut parts: Vec<Vec<&Vec<Point2>>> = vec![Vec::new(); tops.len()];
    for poly in &pieces {
        let n = poly.len() as f64;
        let centre = poly
            .iter()
            .fold((0.0, 0.0), |(x, y), p| (x + p.0 / n, y + p.1 / n));
        let owner = (0..tops.len())
            .filter(|&i| covers(i, centre))
            .min_by(|&i, &j| plane_at(tops[i], centre).total_cmp(&plane_at(tops[j], centre)))?;
        parts[owner].push(poly);
    }
    let key = |p: Point2| (p.0.to_bits(), p.1.to_bits());
    let mut meshes = Vec::with_capacity(tops.len());
    for (i, polys) in parts.iter().enumerate() {
        let top = |p: Point2| plane_at(tops[i], p);
        let bottom = |p: Point2| plane_at(undersides[i].0, p).max(undersides[i].1);
        let mut mesh = Mesh::default();
        let mut push = |points: [[f64; 3]; 3]| {
            let base = mesh.vertices.len() as u32;
            mesh.vertices.extend(points);
            mesh.triangles.push([base, base + 1, base + 2]);
        };
        // Edges two of the part's pieces share are inside it; the pieces
        // were cut along whole lines, so a shared edge has the same ends in
        // both.
        let mut edges: std::collections::HashSet<_> = std::collections::HashSet::new();
        for poly in polys {
            for k in 0..poly.len() {
                edges.insert((key(poly[k]), key(poly[(k + 1) % poly.len()])));
            }
        }
        for poly in polys {
            let lift = |p: Point2, z: f64| [p.0, p.1, z];
            for k in 1..poly.len() - 1 {
                let [a, b, c] = [poly[0], poly[k], poly[k + 1]];
                push([lift(a, top(a)), lift(b, top(b)), lift(c, top(c))]);
                push([lift(a, bottom(a)), lift(c, bottom(c)), lift(b, bottom(b))]);
            }
            for k in 0..poly.len() {
                let (p, q) = (poly[k], poly[(k + 1) % poly.len()]);
                if edges.contains(&(key(q), key(p))) {
                    continue;
                }
                push([lift(p, bottom(p)), lift(q, bottom(q)), lift(q, top(q))]);
                push([lift(p, bottom(p)), lift(q, top(q)), lift(p, top(p))]);
            }
        }
        let sound = polys
            .iter()
            .flat_map(|poly| poly.iter())
            .all(|&p| top(p) >= bottom(p) - 1e-9)
            && mesh.vertices.iter().flatten().all(|v| v.is_finite());
        if !sound {
            return None;
        }
        meshes.push(mesh);
    }
    Some(meshes)
}

/// An extruded body cut into layers stacked from its top down: for
/// each layer width, top first, the prism of the body's plan outline over
/// that layer's heights. `None` unless the widths add up to the body's
/// height within [`LAYER_THICKNESS_TOLERANCE_FEET`].
pub fn stacked_extrusion_meshes(extrusion: &Extrusion, widths: &[f64]) -> Option<Vec<Mesh>> {
    let height = extrusion.height_feet;
    let total: f64 = widths.iter().sum();
    if !(height.is_finite() && height > 0.0 && total.is_finite() && total > 0.0)
        || (height - total).abs() > LAYER_THICKNESS_TOLERANCE_FEET
    {
        return None;
    }
    let (outer, holes) = extrusion_rings(extrusion)?;
    let section = section(&outer, &holes)?;
    let mut top = height;
    Some(
        widths
            .iter()
            .map(|&width| {
                let bottom = top - width;
                let band = top;
                top = bottom;
                prism(&section, |(x, y), t| [x, y, bottom + t * (band - bottom)])
            })
            .collect(),
    )
}

/// An extrusion's body in element-local feet.
pub fn extrusion_mesh(extrusion: &Extrusion) -> Option<Mesh> {
    let height = extrusion.height_feet;
    if !(height.is_finite() && height > 0.0) {
        return None;
    }
    let (outer, holes) = extrusion_rings(extrusion)?;
    let section = section(&outer, &holes)?;
    Some(prism(&section, |(x, y), t| [x, y, t * height]))
}

/// How far a layered element's layers may add up to other than its body's
/// thickness across them before the layers are not drawn, feet.
pub const LAYER_THICKNESS_TOLERANCE_FEET: f64 = 0.02;

/// The part of `ring` whose offset along `normal` lies within `lo..=hi`
/// (Sutherland–Hodgman against the two half-planes).
pub fn clip_ring_to_band(ring: &[Point2], normal: [f64; 2], lo: f64, hi: f64) -> Ring {
    let offset = |p: Point2| p.0 * normal[0] + p.1 * normal[1];
    let clip = |ring: &[Point2], keep: &dyn Fn(f64) -> bool, edge: f64| -> Ring {
        let mut out = Ring::new();
        for (i, &p) in ring.iter().enumerate() {
            let q = ring[(i + 1) % ring.len()];
            let (sp, sq) = (offset(p), offset(q));
            if keep(sp) {
                out.push(p);
            }
            if keep(sp) != keep(sq) {
                let t = (edge - sp) / (sq - sp);
                out.push((p.0 + t * (q.0 - p.0), p.1 + t * (q.1 - p.1)));
            }
        }
        out
    };
    let below = clip(ring, &|s| s <= hi, hi);
    without_spikes(clip(&below, &|s| s >= lo, lo))
}

/// `ring` without the vertices that lie on a straight line through their
/// neighbours. Clipping an outline with steps on the band's edges (a wall
/// whose layers end on different lines) leaves zero-width spikes
/// along those edges, out to where the neighbouring layer ends; they draw
/// nothing but carry vertices at the wrong length.
fn without_spikes(mut ring: Ring) -> Ring {
    let mut index = 0;
    while ring.len() > 2 && index < ring.len() {
        let previous = ring[(index + ring.len() - 1) % ring.len()];
        let point = ring[index];
        let next = ring[(index + 1) % ring.len()];
        let cross = (point.0 - previous.0) * (next.1 - point.1)
            - (point.1 - previous.1) * (next.0 - point.0);
        if cross.abs() <= 1e-12 {
            ring.remove(index);
            index = index.saturating_sub(1);
        } else {
            index += 1;
        }
    }
    ring
}

/// An extruded body cut into its layers across the thickness: for each
/// layer width, exterior first, the prism of the body's plan outline that
/// lies within it. `exterior` is the unit plan direction to the exterior
/// face in the extrusion's own axes. `None` unless the body is one outline
/// with no voids and the widths add up to its thickness along `exterior`
/// within [`LAYER_THICKNESS_TOLERANCE_FEET`].
pub fn layered_extrusion_meshes(
    extrusion: &Extrusion,
    exterior: [f64; 2],
    widths: &[f64],
) -> Option<Vec<Mesh>> {
    let height = extrusion.height_feet;
    if !(height.is_finite() && height > 0.0) || widths.is_empty() {
        return None;
    }
    let (outer, holes) = extrusion_rings(extrusion)?;
    if !holes.is_empty() || outer.len() < 3 {
        return None;
    }
    let offsets: Vec<f64> = outer
        .iter()
        .map(|p| p.0 * exterior[0] + p.1 * exterior[1])
        .collect();
    let (min, max) = offsets
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &s| {
            (lo.min(s), hi.max(s))
        });
    let total: f64 = widths.iter().sum();
    if !total.is_finite()
        || total <= 0.0
        || ((max - min) - total).abs() > LAYER_THICKNESS_TOLERANCE_FEET
    {
        return None;
    }
    let mut meshes = Vec::with_capacity(widths.len());
    let mut face = max;
    for &width in widths {
        let band = clip_ring_to_band(&outer, exterior, face - width, face);
        face -= width;
        let mesh = section(&band, &[])
            .map(|section| prism(&section, |(x, y), t| [x, y, t * height]))
            .unwrap_or_default();
        meshes.push(mesh);
    }
    Some(meshes)
}

/// A stepped wall's body: a [`SolidShape::Compound`] of the wall's
/// plan to a height and pieces of that plan standing on it, cut into its
/// layers like [`layered_extrusion_meshes`] cuts an extrusion. The layers are
/// laid across the first item's plan, and every piece is clipped to them.
/// `None` for any other shape.
pub fn layered_stepped_meshes(
    shape: &SolidShape,
    exterior: [f64; 2],
    widths: &[f64],
) -> Option<Vec<Mesh>> {
    let SolidShape::Compound(items) = shape else {
        return None;
    };
    let Some(SolidShape::ExtrudedArea(base)) = items.first() else {
        return None;
    };
    let (plan, holes) = extrusion_rings(base)?;
    if !holes.is_empty() || plan.len() < 3 || widths.is_empty() {
        return None;
    }
    let (min, max) = plan
        .iter()
        .map(|p| p.0 * exterior[0] + p.1 * exterior[1])
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), s| {
            (lo.min(s), hi.max(s))
        });
    let total: f64 = widths.iter().sum();
    if !total.is_finite()
        || total <= 0.0
        || ((max - min) - total).abs() > LAYER_THICKNESS_TOLERANCE_FEET
    {
        return None;
    }
    let mut parts: Vec<(Ring, f64, f64)> = vec![(plan, 0.0, base.height_feet)];
    for item in &items[1..] {
        let SolidShape::PlacedExtrusion {
            profile,
            origin_feet,
            axis,
            ref_direction,
            depth_feet,
        } = item
        else {
            return None;
        };
        if *axis != [0.0, 0.0, 1.0] || *ref_direction != [1.0, 0.0, 0.0] {
            return None;
        }
        let (outer, holes) = profile_rings(profile)?;
        if !holes.is_empty() {
            return None;
        }
        parts.push((outer, origin_feet[2], *depth_feet));
    }
    if parts
        .iter()
        .any(|(_, z, height)| !(z.is_finite() && height.is_finite() && *height > 0.0))
    {
        return None;
    }
    let mut meshes = Vec::with_capacity(widths.len());
    let mut face = max;
    for &width in widths {
        let mut mesh = Mesh::default();
        for (ring, z, height) in &parts {
            let band = clip_ring_to_band(ring, exterior, face - width, face);
            if let Some(section) = section(&band, &[]) {
                mesh.append(prism(&section, |(x, y), t| [x, y, z + t * height]));
            }
        }
        face -= width;
        meshes.push(mesh);
    }
    Some(meshes)
}

fn unit(v: [f64; 3]) -> Option<[f64; 3]> {
    let n = dot(v, v).sqrt();
    (n.is_finite() && n > 1e-12).then(|| [v[0] / n, v[1] / n, v[2] / n])
}

/// A profile swept along a polyline, as `IfcFixedReferenceSweptAreaSolid`
/// places it: along each segment the profile's X axis is `fixed_reference`
/// with its component along the segment removed, and its Z axis the
/// segment's direction.
fn swept_mesh(
    profile: &ProfileDef,
    points: &[[f64; 3]],
    fixed_reference: [f64; 3],
) -> Option<Mesh> {
    let (outer, holes) = profile_rings(profile)?;
    let section = section(&outer, &holes)?;
    let reference = unit(fixed_reference)?;
    let mut mesh = Mesh::default();
    for pair in points.windows(2) {
        let (p, q) = (pair[0], pair[1]);
        let Some(z) = unit(sub(q, p)) else {
            continue;
        };
        let along = dot(reference, z);
        let x = unit(sub(reference, [z[0] * along, z[1] * along, z[2] * along]))
            .or_else(|| unit(cross3(z, [1.0, 0.0, 0.0])))
            .or_else(|| unit(cross3(z, [0.0, 1.0, 0.0])))?;
        let y = cross3(z, x);
        let run = sub(q, p);
        mesh.append(prism(&section, |(u, v), t| {
            [
                p[0] + t * run[0] + u * x[0] + v * y[0],
                p[1] + t * run[1] + u * x[1] + v * y[1],
                p[2] + t * run[2] + u * x[2] + v * y[2],
            ]
        }));
    }
    (!mesh.is_empty()).then_some(mesh)
}

/// A profile extruded along the Z axis of its own placement, as
/// `IfcExtrudedAreaSolid` with an `IfcAxis2Placement3D` does: X is the
/// reference direction projected off the axis, Y is the axis crossed with
/// X.
fn placed_extrusion_mesh(
    profile: &ProfileDef,
    origin: [f64; 3],
    axis: [f64; 3],
    reference: [f64; 3],
    depth: f64,
) -> Option<Mesh> {
    if !(depth.is_finite() && depth > 0.0) {
        return None;
    }
    let (outer, holes) = profile_rings(profile)?;
    let section = section(&outer, &holes)?;
    let z = unit(axis)?;
    let along = dot(reference, z);
    let x = unit(sub(reference, [z[0] * along, z[1] * along, z[2] * along]))?;
    let y = cross3(z, x);
    let mesh = prism(&section, |(u, v), t| {
        [
            origin[0] + u * x[0] + v * y[0] + t * depth * z[0],
            origin[1] + u * x[1] + v * y[1] + t * depth * z[1],
            origin[2] + u * x[2] + v * y[2] + t * depth * z[2],
        ]
    });
    (!mesh.is_empty()).then_some(mesh)
}

/// `p` turned `angle` about the axis through `origin` along the unit `axis`.
fn rotate(p: [f64; 3], origin: [f64; 3], axis: [f64; 3], angle: f64) -> [f64; 3] {
    let v = sub(p, origin);
    let (s, c) = angle.sin_cos();
    let k = cross3(axis, v);
    let d = dot(axis, v) * (1.0 - c);
    [
        origin[0] + v[0] * c + k[0] * s + axis[0] * d,
        origin[1] + v[1] * c + k[1] * s + axis[1] * d,
        origin[2] + v[2] * c + k[2] * s + axis[2] * d,
    ]
}

/// A profile in the local XY plane turned about an axis, as
/// `IfcRevolvedAreaSolid` does. Drawn two-sided.
fn revolved_mesh(
    profile: &ProfileDef,
    origin: [f64; 3],
    axis: [f64; 3],
    angle: f64,
) -> Option<Mesh> {
    if !(angle.is_finite() && angle != 0.0) || !origin.iter().all(|v| v.is_finite()) {
        return None;
    }
    let axis = unit(axis)?;
    let (outer, holes) = profile_rings(profile)?;
    let section = section(&outer, &holes)?;
    let turn = angle.clamp(-std::f64::consts::TAU, std::f64::consts::TAU);
    let steps =
        ((CIRCLE_SEGMENTS as f64 * turn.abs() / std::f64::consts::TAU).ceil() as usize).max(1);
    let at = |p: Point2, step: usize| {
        rotate(
            [p.0, p.1, 0.0],
            origin,
            axis,
            turn * step as f64 / steps as f64,
        )
    };
    let mut mesh = Mesh::default();
    for ring in &section.rings {
        for i in 0..ring.len() {
            let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
            for step in 0..steps {
                let base = mesh.vertices.len() as u32;
                mesh.vertices
                    .extend([at(a, step), at(b, step), at(b, step + 1), at(a, step + 1)]);
                mesh.triangles.push([base, base + 1, base + 2]);
                mesh.triangles.push([base, base + 2, base + 3]);
            }
        }
    }
    if turn.abs() < std::f64::consts::TAU - 1e-9 {
        for step in [0, steps] {
            let base = mesh.vertices.len() as u32;
            mesh.vertices
                .extend(section.cap_points.iter().map(|&p| at(p, step)));
            mesh.triangles.extend(
                section
                    .cap_triangles
                    .iter()
                    .map(|&[a, b, c]| [base + a, base + b, base + c]),
            );
        }
    }
    Some(mesh.two_sided())
}

/// The one layer of an element whose type has a single layer and whose body
/// is a solid the layered splitters do not take apart (a wall whose base is cut
/// at a shape-edited or level floor): the whole solid, where its extent along the
/// exterior normal `exterior`, in the body's frame, is the layer's width within
/// [`LAYER_THICKNESS_TOLERANCE_FEET`]. `None` for several layers, a solid that
/// does not mesh, or a thickness that is not the layer's.
pub fn one_layer_solid_meshes(
    shape: &SolidShape,
    exterior: [f64; 2],
    widths: &[f64],
) -> Option<Vec<Mesh>> {
    let [width] = widths else {
        return None;
    };
    let mesh = solid_mesh(shape)?;
    let (min, max) = mesh
        .vertices
        .iter()
        .map(|p| p[0] * exterior[0] + p[1] * exterior[1])
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), s| {
            (lo.min(s), hi.max(s))
        });
    let thickness = max - min;
    (width.is_finite()
        && *width > 0.0
        && thickness.is_finite()
        && (thickness - width).abs() <= LAYER_THICKNESS_TOLERANCE_FEET)
        .then(|| vec![mesh])
}

/// A solid shape's body in element-local feet.
pub fn solid_mesh(shape: &SolidShape) -> Option<Mesh> {
    match shape {
        SolidShape::ExtrudedArea(extrusion) => extrusion_mesh(extrusion),
        SolidShape::FacetedBrep {
            vertices_feet,
            triangles,
        }
        | SolidShape::TriangulatedFaceSet {
            vertices_feet,
            triangles,
            ..
        } => {
            if !vertices_feet.iter().flatten().all(|v| v.is_finite()) {
                return None;
            }
            let faces: &[usize] = match shape {
                SolidShape::TriangulatedFaceSet { face_materials, .. }
                    if face_materials.len() == triangles.len() =>
                {
                    face_materials
                }
                _ => &[],
            };
            let count = vertices_feet.len();
            let (triangles, face_materials): (Vec<[u32; 3]>, Vec<usize>) = triangles
                .iter()
                .enumerate()
                .map(|(i, t)| ([t.0, t.1, t.2], faces.get(i).copied()))
                .filter(|(t, _)| t.iter().all(|&i| (i as usize) < count))
                .map(|(t, m)| (t, m.unwrap_or(NO_FACE_MATERIAL)))
                .unzip();
            let face_materials = if faces.is_empty() {
                Vec::new()
            } else {
                face_materials
            };
            let mesh = Mesh {
                vertices: vertices_feet.clone(),
                triangles,
                face_materials,
            };
            (!mesh.is_empty()).then(|| mesh.outward_or_two_sided())
        }
        SolidShape::SweptPath {
            profile,
            directrix_points_feet,
            fixed_reference,
        } => swept_mesh(profile, directrix_points_feet, *fixed_reference),
        SolidShape::PlacedExtrusion {
            profile,
            origin_feet,
            axis,
            ref_direction,
            depth_feet,
        } => placed_extrusion_mesh(profile, *origin_feet, *axis, *ref_direction, *depth_feet),
        SolidShape::RevolvedArea {
            profile,
            axis_origin_feet,
            axis_direction,
            angle_radians,
        } => revolved_mesh(profile, *axis_origin_feet, *axis_direction, *angle_radians),
        SolidShape::PlacedRevolvedArea {
            profile,
            origin_feet,
            axis_direction,
            radial_direction,
            angle_radians,
        } => {
            let y = unit(*axis_direction)?;
            let z = unit(cross3(*radial_direction, y))?;
            let x = cross3(y, z);
            let mut mesh = revolved_mesh(profile, [0.0; 3], [0.0, 1.0, 0.0], *angle_radians)?;
            for p in &mut mesh.vertices {
                *p = std::array::from_fn(|i| {
                    origin_feet[i] + x[i] * p[0] + y[i] * p[1] + z[i] * p[2]
                });
            }
            Some(mesh)
        }
        SolidShape::Compound(items) => {
            let mut meshes = items.iter().filter_map(solid_mesh);
            let mut mesh = meshes.next()?;
            for other in meshes {
                mesh.append(other);
            }
            Some(mesh)
        }
        SolidShape::BooleanResult { .. } => {
            // A wall's profile cuts nest one level per cut, down the first
            // operand: walked as a loop, so the file's cut count is not
            // the stack's depth. A difference keeps its first operand's mesh; a
            // union adds its second's, innermost first, as recursion did.
            let mut unions = Vec::new();
            let mut at = shape;
            while let SolidShape::BooleanResult {
                op,
                operand_a,
                operand_b,
            } = at
            {
                if matches!(op, IfcBooleanOp::Union) {
                    unions.push(operand_b.as_ref());
                }
                at = operand_a.as_ref();
            }
            let mut mesh = solid_mesh(at)?;
            for other in unions.into_iter().rev().filter_map(solid_mesh) {
                mesh.append(other);
            }
            Some(mesh)
        }
    }
}

/// A body in element-local feet.
pub fn body_mesh(body: Body<'_>) -> Option<Mesh> {
    match body {
        Body::Extrusion(extrusion) => extrusion_mesh(extrusion),
        Body::Solid(shape) => solid_mesh(shape),
    }
}

/// The convex hull of `points`, counter-clockwise.
fn convex_hull(mut points: Vec<Point2>) -> Ring {
    points.retain(|p| p.0.is_finite() && p.1.is_finite());
    points.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    points.dedup_by(|a, b| same(*a, *b));
    if points.len() < 3 {
        return points;
    }
    let mut lower: Ring = Vec::new();
    for &p in &points {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], p) <= 0.0 {
            lower.pop();
        }
        lower.push(p);
    }
    let mut upper: Ring = Vec::new();
    for &p in points.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], p) <= 0.0 {
            upper.pop();
        }
        upper.push(p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// What an element covers in plan, in model feet: an extrusion's profile
/// (holes kept) where it stands, the profile of a stepped wall's first item
/// where the rest stand within it, and the convex hull of any other body.
pub fn plan_outline(body: Body<'_>, placement: &Placement) -> Option<(Ring, Vec<Ring>)> {
    let to_model = |ring: &Ring| -> Ring {
        ring.iter()
            .map(|&(x, y)| {
                let p = placement.apply([x, y, 0.0]);
                (p[0], p[1])
            })
            .collect()
    };
    match body {
        Body::Extrusion(extrusion) => {
            let (outer, holes) = extrusion_rings(extrusion)?;
            let outer = clean(&outer)?;
            let holes = holes.iter().filter_map(|h| clean(h)).collect::<Vec<_>>();
            Some((to_model(&outer), holes.iter().map(to_model).collect()))
        }
        Body::Solid(shape) => {
            let mesh = solid_mesh(shape)?;
            let hull = convex_hull(
                mesh.vertices
                    .iter()
                    .map(|&v| {
                        let p = placement.apply(v);
                        (p[0], p[1])
                    })
                    .collect(),
            );
            if hull.len() < 3 {
                return None;
            }
            // A stepped wall is its own plan to a height and pieces of
            // it above: its first item is that plan, and the rest stand
            // within it, so the hull of the first item is the hull of all.
            if let SolidShape::Compound(items) = shape {
                if let Some(SolidShape::ExtrudedArea(first)) = items.first() {
                    if let Some((outer, holes)) = plan_outline(Body::Extrusion(first), placement) {
                        let first_hull = convex_hull(outer.clone());
                        if (signed_area(&first_hull).abs() - signed_area(&hull).abs()).abs() <= 1e-6
                        {
                            return Some((outer, holes));
                        }
                    }
                }
            }
            Some((hull, Vec::new()))
        }
    }
}

/// an element-local mesh's points on a 1e-7 m grid (`feet_to_m`
/// converts the mesh's unit) and a key two meshes share exactly when those
/// points and their triangles are equal.
pub fn mesh_key(mesh: &Mesh, feet_to_m: f64) -> (Vec<[i64; 3]>, String) {
    use std::fmt::Write;
    let q: Vec<[i64; 3]> = mesh
        .vertices
        .iter()
        .map(|v| v.map(|c| (c * feet_to_m * 1e7).round() as i64))
        .collect();
    let mut key = String::with_capacity(q.len() * 24 + mesh.triangles.len() * 12);
    for p in &q {
        let _ = write!(key, "{},{},{};", p[0], p[1], p[2]);
    }
    key.push('|');
    for t in &mesh.triangles {
        let _ = write!(key, "{},{},{};", t[0], t[1], t[2]);
    }
    (q, key)
}

/// The exact body definition of a building element that can share a map:
/// its solid shape, else its extrusion, as text. `None` for an
/// opening (the IFC4 writer gives its extrusion its host's cut), an element
/// already mapped, and one with no drawable body.
fn shareable_body_key(entity: &IfcEntity) -> Option<String> {
    let IfcEntity::BuildingElement {
        ifc_type,
        representation_map_index: None,
        ..
    } = entity
    else {
        return None;
    };
    if ifc_type == "IFCOPENINGELEMENT" {
        return None;
    }
    // Keyed by the exact solid definition, not its mesh: the mesh draws a
    // profile past MAX_TRIANGULATED_POINTS (or one it cannot triangulate)
    // as a stand-in, while IFC4 writes the profile itself, so two equal
    // meshes can be two different IFC4 solids.
    let body = element_body(entity, &[])?;
    if body_mesh(body).is_none_or(|m| m.is_empty()) {
        return None;
    }
    match body {
        Body::Solid(shape) => serde_json::to_string(shape),
        Body::Extrusion(ex) => serde_json::to_string(&SolidShape::ExtrudedArea(ex.clone())),
    }
    .ok()
}

/// the sharing key of entity `i`: its symbol mesh where it is an
/// exact instance of one (whatever its placement), else
/// [`shareable_body_key`].
fn body_key(model: &super::IfcModel, i: usize) -> Option<String> {
    let entity = &model.entities[i];
    if matches!(entity, IfcEntity::BuildingElement { ifc_type, .. } if ifc_type == "IFCOPENINGELEMENT")
    {
        return None;
    }
    match super::instancing::parts_of(model, i) {
        Some([(mesh, _)]) => Some(format!("symbol-mesh:{mesh}")),
        // Several parts: the writers share each part.
        Some(_) => None,
        None => shareable_body_key(entity),
    }
}

/// Draw each group of elements (indices into `model.entities`, all with an
/// equal [`shareable_body_key`]) through one new `RepresentationMap`, in the
/// order of each group's first element. Returns the number of maps added.
fn map_groups(model: &mut super::IfcModel, groups: Vec<Vec<usize>>) -> usize {
    let mut groups = groups;
    groups.sort_by_key(|g| g[0]);
    let added = groups.len();
    for members in groups {
        let index = model.representation_maps.len();
        // instances of one symbol mesh map it in its own frame,
        // each through its transform (the key made every member one).
        let symbol: Vec<Option<(usize, super::instancing::Frame)>> = members
            .iter()
            .map(|&i| super::instancing::instance_of(model, i))
            .collect();
        if let Some(Some((mesh, _))) = symbol.first() {
            if symbol.iter().all(|s| s.is_some_and(|(m, _)| m == *mesh)) {
                let shape = model.instancing.meshes[*mesh].shape();
                for (&i, s) in members.iter().zip(&symbol) {
                    if let IfcEntity::BuildingElement {
                        solid_shape,
                        representation_map_index,
                        ..
                    } = &mut model.entities[i]
                    {
                        *solid_shape = None;
                        *representation_map_index = Some(index);
                    }
                    let target = s.expect("checked").1;
                    model.instancing.targets.insert(i, target);
                }
                model.representation_maps.push(RepresentationMap {
                    name: None,
                    shape,
                    origin_feet: [0.0; 3],
                    plan: Vec::new(),
                });
                continue;
            }
        }
        let mut shape = None;
        for &i in &members {
            if let IfcEntity::BuildingElement {
                solid_shape,
                extrusion,
                representation_map_index,
                ..
            } = &mut model.entities[i]
            {
                // The solid shape moves into the map; an extrusion stays (it
                // is small, and the map takes precedence in every writer).
                let own = solid_shape
                    .take()
                    .or_else(|| extrusion.clone().map(SolidShape::ExtrudedArea));
                if shape.is_none() {
                    shape = own;
                }
                *representation_map_index = Some(index);
            }
        }
        if let Some(shape) = shape {
            model.representation_maps.push(RepresentationMap {
                name: None,
                shape,
                origin_feet: [0.0; 3],
                plan: Vec::new(),
            });
        }
    }
    added
}

/// every building element (openings aside)
/// whose body (solid shape, else extrusion) is exactly equal to at least one
/// other's draws it through one shared `RepresentationMap` (IFC4
/// `IfcRepresentationMap` + a per-instance `IfcMappedItem`); the element's
/// own placement is unchanged. Returns the number of maps added.
pub fn share_shapes(model: &mut super::IfcModel) -> usize {
    let mut groups: std::collections::BTreeMap<String, Vec<usize>> = Default::default();
    for i in 0..model.entities.len() {
        if let Some(key) = body_key(model, i) {
            groups.entry(key).or_default().push(i);
        }
    }
    let shared = groups.into_values().filter(|g| g.len() > 1).collect();
    map_groups(model, shared)
}

/// IFC classes Revit 2027's own IFC4 exports draw through a mapped item
/// (the 12 Nordic models): the classes of family instances. The
/// fallback of [`revit_family_instances`] for an element whose type record
/// was not read.
pub const REVIT_MAPPED_CLASSES: &[&str] = &[
    "IFCMEMBER",
    "IFCDOOR",
    "IFCWINDOW",
    "IFCCOLUMN",
    "IFCBEAM",
    "IFCPLATE",
    "IFCFURNITURE",
    "IFCSANITARYTERMINAL",
    "IFCELECTRICAPPLIANCE",
    "IFCLIGHTFIXTURE",
    "IFCWASTETERMINAL",
    "IFCFIRESUPPRESSIONTERMINAL",
    "IFCGEOGRAPHICELEMENT",
];

/// IFC classes Revit 2027's own IFC4 exports never draw through a mapped
/// item (the 12 Nordic models), even where the element's type
/// is a family type (coverings and stairs of loadable families).
pub const REVIT_NEVER_MAPPED_CLASSES: &[&str] = &[
    "IFCWALL",
    "IFCSLAB",
    "IFCCOVERING",
    "IFCCURTAINWALL",
    "IFCROOF",
    "IFCRAILING",
    "IFCRAMP",
    "IFCRAMPFLIGHT",
    "IFCSTAIR",
    "IFCSTAIRFLIGHT",
    "IFCOPENINGELEMENT",
    "IFCPIPESEGMENT",
    "IFCSHADINGDEVICE",
    "IFCSOLARDEVICE",
];

/// Whether Revit's export draws each entity of `model` as a family
/// instance, by index: its type is a family type
/// ([`crate::file_types::FileType::family_symbol`]), or, where its type
/// record was not read, its class is one Revit maps; never one of
/// [`REVIT_NEVER_MAPPED_CLASSES`].
pub fn revit_family_instances(model: &super::IfcModel) -> Vec<bool> {
    let family: std::collections::HashMap<u32, bool> = model
        .file_types
        .iter()
        .map(|t| (t.id, t.family_symbol))
        .collect();
    model
        .entities
        .iter()
        .map(|entity| {
            let IfcEntity::BuildingElement {
                ifc_type,
                type_guid,
                ..
            } = entity
            else {
                return false;
            };
            let type_id = type_guid
                .as_deref()
                .and_then(|t| t.parse::<u32>().ok())
                .and_then(|id| model.element_type_ids.get(&id));
            let class = ifc_type.to_ascii_uppercase();
            if REVIT_NEVER_MAPPED_CLASSES.contains(&class.as_str()) {
                return false;
            }
            match type_id.and_then(|id| family.get(id)) {
                Some(&f) => f,
                None => REVIT_MAPPED_CLASSES.contains(&class.as_str()),
            }
        })
        .collect()
}

/// family instances as Revit's export does ([`share_shapes_revit`]),
/// `open` every exactly equal definition ([`share_shapes`]). What `rvt-ifc` runs unless `--per-element-meshes`. Returns the
/// number of maps added.
pub fn share_shapes_by_convention(model: &mut super::IfcModel) -> usize {
    match model.convention {
        super::convention::ExportConvention::Revit => {
            let family = revit_family_instances(model);
            share_shapes_revit(model, |i, _| family[i])
        }
        _ => share_shapes(model),
    }
}

/// Revit's export draws every family
/// instance through an `IfcMappedItem` of a map its type object holds, and
/// never a wall, slab, covering, railing, stair or opening. Its exporter
/// keys a type's map by the instance's original symbol, entity and flip,
/// and gives an instance whose own geometry differs from its symbol's a map
/// of its own (Revit's IFC exporter, `FamilyInstanceExporter`, read, not copied; the
/// element `family_instance` admits gets a map, a type used once included:
/// the elements of one type, entity and predefined type share a map where
/// their body definitions are exactly equal. Returns the number of maps
/// added.
pub fn share_shapes_revit(
    model: &mut super::IfcModel,
    family_instance: impl Fn(usize, &IfcEntity) -> bool,
) -> usize {
    // The type object an element is typed by: its type, entity, predefined
    // type and, where Revit gives it its original symbol's GlobalId,
    // that GlobalId, as the IFC4 writer groups type objects.
    type Key = (Option<u32>, String, Option<String>, Option<String>, String);
    let mut groups: std::collections::BTreeMap<Key, Vec<usize>> = Default::default();
    for (i, entity) in model.entities.iter().enumerate() {
        if !family_instance(i, entity) {
            continue;
        }
        let Some(body) = body_key(model, i) else {
            continue;
        };
        let IfcEntity::BuildingElement {
            ifc_type,
            type_guid,
            predefined_type,
            ..
        } = entity
        else {
            continue;
        };
        let element_id = type_guid.as_deref().and_then(|t| t.parse::<u32>().ok());
        let type_id = element_id.and_then(|id| model.element_type_ids.get(&id).copied());
        let symbol_gid = element_id.and_then(|id| model.global_ids.element_types.get(&id).cloned());
        groups
            .entry((
                type_id,
                ifc_type.clone(),
                predefined_type.clone(),
                symbol_gid,
                body,
            ))
            .or_default()
            .push(i);
    }
    map_groups(model, groups.into_values().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ifc::entities::BrepTriangle;

    fn area(section: &Section) -> f64 {
        section
            .cap_triangles
            .iter()
            .map(|&[a, b, c]| {
                let p = &section.cap_points;
                cross(p[a as usize], p[b as usize], p[c as usize]) / 2.0
            })
            .sum()
    }

    /// Signed volume of a closed mesh (divergence theorem).
    fn volume(mesh: &Mesh) -> f64 {
        mesh.triangles
            .iter()
            .map(|&[a, b, c]| {
                let (a, b, c) = (
                    mesh.vertices[a as usize],
                    mesh.vertices[b as usize],
                    mesh.vertices[c as usize],
                );
                dot(a, cross3(b, c)) / 6.0
            })
            .sum()
    }

    #[test]
    fn a_concave_profile_triangulates_to_its_area() {
        // An L: 4 x 4 less a 2 x 2 corner.
        let l = vec![
            (0.0, 0.0),
            (4.0, 0.0),
            (4.0, 2.0),
            (2.0, 2.0),
            (2.0, 4.0),
            (0.0, 4.0),
        ];
        let s = section(&l, &[]).expect("section");
        assert_eq!(s.cap_triangles.len(), 4);
        assert!((area(&s) - 12.0).abs() < 1e-12);
        // Clockwise input is turned counter-clockwise.
        let mut cw = l.clone();
        cw.reverse();
        assert!((area(&section(&cw, &[]).expect("section")) - 12.0).abs() < 1e-12);
    }

    #[test]
    fn holes_are_left_out_of_the_section() {
        let outer = rectangle(10.0, 10.0);
        let holes = vec![rectangle(2.0, 2.0), {
            let r = rectangle(1.0, 1.0);
            r.iter().map(|&(x, y)| (x + 3.0, y + 3.0)).collect()
        }];
        let s = section(&outer, &holes).expect("section");
        assert_eq!(s.rings.len(), 3);
        assert!((area(&s) - (100.0 - 4.0 - 1.0)).abs() < 1e-9);
    }

    #[test]
    fn a_self_intersecting_profile_draws_its_bounding_rectangle() {
        // The last edge crosses the first; the ring still encloses area.
        let crossing = vec![(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (1.0, -1.0)];
        let s = section(&crossing, &[]).expect("section");
        assert!((area(&s) - 20.0).abs() < 1e-12);
    }

    #[test]
    fn degenerate_profiles_have_no_section() {
        assert!(section(&[(0.0, 0.0), (1.0, 1.0), (2.0, 2.0)], &[]).is_none());
        assert!(section(&[(0.0, 0.0), (1.0, 0.0)], &[]).is_none());
        assert!(section(&[(0.0, 0.0), (f64::NAN, 0.0), (1.0, 1.0)], &[]).is_none());
        assert!(
            profile_rings(&ProfileDef::Rectangle {
                width_feet: 0.0,
                depth_feet: 1.0
            })
            .is_none()
        );
    }

    #[test]
    fn an_extrusion_is_a_closed_prism_of_its_profile() {
        let mesh = extrusion_mesh(&Extrusion::rectangle(4.0, 2.0, 3.0)).expect("mesh");
        assert!((volume(&mesh) - 24.0).abs() < 1e-9);
        let hollow = Extrusion {
            width_feet: 0.0,
            depth_feet: 0.0,
            height_feet: 2.0,
            profile_override: Some(ProfileDef::RectangleHollow {
                overall_width_feet: 4.0,
                overall_depth_feet: 4.0,
                wall_thickness_feet: 1.0,
            }),
        };
        let mesh = extrusion_mesh(&hollow).expect("mesh");
        assert!((volume(&mesh) - (16.0 - 4.0) * 2.0).abs() < 1e-9);
        assert!(extrusion_mesh(&Extrusion::rectangle(1.0, 1.0, 0.0)).is_none());
    }

    #[test]
    fn steel_profiles_have_their_area() {
        let cases = [
            (
                ProfileDef::IShape {
                    overall_width_feet: 1.0,
                    overall_depth_feet: 2.0,
                    web_thickness_feet: 0.1,
                    flange_thickness_feet: 0.2,
                },
                2.0 * 1.0 * 0.2 + 0.1 * 1.6,
            ),
            (
                ProfileDef::TShape {
                    overall_depth_feet: 2.0,
                    flange_width_feet: 1.0,
                    web_thickness_feet: 0.1,
                    flange_thickness_feet: 0.2,
                },
                1.0 * 0.2 + 0.1 * 1.8,
            ),
            (
                ProfileDef::LShape {
                    overall_depth_feet: 2.0,
                    overall_width_feet: 1.0,
                    thickness_feet: 0.1,
                },
                2.0 * 0.1 + 0.9 * 0.1,
            ),
            (
                ProfileDef::UShape {
                    overall_depth_feet: 2.0,
                    flange_width_feet: 1.0,
                    web_thickness_feet: 0.1,
                    flange_thickness_feet: 0.2,
                },
                2.0 * 0.2 * 1.0 + 0.1 * 1.6,
            ),
        ];
        for (profile, expected) in cases {
            let (outer, holes) = profile_rings(&profile).expect("rings");
            let (x0, y0, x1, y1) = ring_bounds(&outer);
            assert!(
                (x0 + x1).abs() < 1e-12 && (y0 + y1).abs() < 1e-12,
                "centred on its box"
            );
            let s = section(&outer, &holes).expect("section");
            assert!((area(&s) - expected).abs() < 1e-9, "{profile:?}");
        }
    }

    #[test]
    fn a_circle_is_drawn_around_itself() {
        let (outer, _) = profile_rings(&ProfileDef::Circle { radius_feet: 1.0 }).expect("rings");
        let s = section(&outer, &[]).expect("section");
        assert!(area(&s) > std::f64::consts::PI);
        assert!(area(&s) < std::f64::consts::PI * 1.02);
    }

    #[test]
    fn a_swept_beam_lies_along_its_directrix_with_its_depth_up() {
        // A 0.3 deep, 0.25 wide section along a 2 degree slope: the fixed
        // reference +Z sets the profile's X axis, so X is the depth.
        let slope = 2f64.to_radians();
        let d = [0.0, slope.cos(), -slope.sin()];
        let shape = SolidShape::SweptPath {
            profile: ProfileDef::Rectangle {
                width_feet: 0.3,
                depth_feet: 0.25,
            },
            directrix_points_feet: vec![[0.0, 0.0, 0.0], [0.0, 20.0 * d[1], 20.0 * d[2]]],
            fixed_reference: [0.0, 0.0, 1.0],
        };
        let mesh = solid_mesh(&shape).expect("mesh");
        assert!((volume(&mesh) - 20.0 * 0.3 * 0.25).abs() < 1e-9);
        let across = |axis: [f64; 3]| {
            let values: Vec<f64> = mesh.vertices.iter().map(|&v| dot(v, axis)).collect();
            values.iter().copied().fold(f64::NEG_INFINITY, f64::max)
                - values.iter().copied().fold(f64::INFINITY, f64::min)
        };
        assert!((across([1.0, 0.0, 0.0]) - 0.25).abs() < 1e-9);
        assert!((across([0.0, slope.sin(), slope.cos()]) - 0.3).abs() < 1e-9);
        assert!((across(d) - 20.0).abs() < 1e-9);
    }

    /// A placed extrusion stands its profile in the plane its axis
    /// is normal to, X along the reference, Y the axis crossed with X.
    #[test]
    fn a_placed_extrusion_stands_its_profile_up() {
        // An L-shaped step, X up and Y along, extruded 4 ft along -X.
        let points = vec![
            (0.0, 0.0),
            (0.5, 0.0),
            (0.5, 1.0),
            (0.25, 1.0),
            (0.25, 0.5),
            (0.0, 0.5),
        ];
        let shape = SolidShape::PlacedExtrusion {
            profile: ProfileDef::ArbitraryClosed { points },
            origin_feet: [1.0, 2.0, 3.0],
            axis: [-1.0, 0.0, 0.0],
            ref_direction: [0.0, 0.0, 1.0],
            depth_feet: 4.0,
        };
        let mesh = solid_mesh(&shape).expect("mesh");
        assert!((volume(&mesh).abs() - 4.0 * 0.375).abs() < 1e-9);
        // Y is (-1, 0, 0) × (0, 0, 1) = (0, 1, 0): profile (x, y) lands at
        // origin + x up + y along +Y, swept to x = 1 - 4.
        for (x, y) in [(0.5, 1.0), (0.25, 0.5)] {
            for along in [1.0, -3.0] {
                let want = [along, 2.0 + y, 3.0 + x];
                assert!(
                    mesh.vertices
                        .iter()
                        .any(|p| (0..3).all(|i| (p[i] - want[i]).abs() < 1e-12)),
                    "missing {want:?}"
                );
            }
        }
        // A zero depth draws nothing.
        let SolidShape::PlacedExtrusion {
            profile,
            origin_feet,
            axis,
            ref_direction,
            ..
        } = shape
        else {
            unreachable!()
        };
        let flat = SolidShape::PlacedExtrusion {
            profile,
            origin_feet,
            axis,
            ref_direction,
            depth_feet: 0.0,
        };
        assert!(solid_mesh(&flat).is_none());
    }

    #[test]
    fn a_revolution_and_a_brep_are_drawn() {
        let shape = SolidShape::RevolvedArea {
            profile: ProfileDef::Rectangle {
                width_feet: 1.0,
                depth_feet: 2.0,
            },
            axis_origin_feet: [2.0, 0.0, 0.0],
            axis_direction: [0.0, 1.0, 0.0],
            angle_radians: std::f64::consts::TAU,
        };
        let mesh = solid_mesh(&shape).expect("mesh");
        assert!(!mesh.is_empty());
        let brep = SolidShape::FacetedBrep {
            vertices_feet: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            triangles: vec![
                BrepTriangle(0, 2, 1),
                BrepTriangle(0, 1, 3),
                BrepTriangle(9, 9, 9),
            ],
        };
        let mesh = solid_mesh(&brep).expect("mesh");
        assert_eq!(
            mesh.triangles.len(),
            4,
            "the bad triangle is dropped, the rest two-sided"
        );
    }

    #[test]
    fn a_display_body_is_drawn_instead_of_the_element_body() {
        // the mesh formats draw a host's display body (its saved
        // graphics, openings cut); everything else keeps its own body.
        let wall: IfcEntity = serde_json::from_value(serde_json::json!({
            "kind": "BuildingElement",
            "ifc_type": "IFCWALL",
            "name": "w",
            "type_guid": null,
            "extrusion": Extrusion::rectangle(2.0, 1.0, 1.0),
        }))
        .expect("entity");
        let mut model = super::super::IfcModel {
            entities: vec![wall.clone(), wall],
            ..Default::default()
        };
        let cut = SolidShape::FacetedBrep {
            vertices_feet: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            triangles: vec![BrepTriangle(0, 1, 2)],
        };
        model.display_bodies.insert(1, cut);
        let drawn = |i: usize| {
            body_mesh(drawn_body(&model, i, &model.entities[i]).expect("body"))
                .expect("mesh")
                .triangles
                .len()
        };
        assert_eq!(drawn(0), 12, "the box");
        assert_eq!(drawn(1), 2, "the display body, two-sided");
    }

    #[test]
    fn a_difference_draws_its_first_operand() {
        let shape = SolidShape::BooleanResult {
            op: IfcBooleanOp::Difference,
            operand_a: Box::new(SolidShape::ExtrudedArea(Extrusion::rectangle(
                2.0, 2.0, 1.0,
            ))),
            operand_b: Box::new(SolidShape::ExtrudedArea(Extrusion::rectangle(
                1.0, 1.0, 1.0,
            ))),
        };
        assert!((volume(&solid_mesh(&shape).expect("mesh")) - 4.0).abs() < 1e-9);
    }

    #[test]
    fn plan_outlines_turn_with_the_placement() {
        let extrusion = Extrusion::rectangle(10.0, 1.0, 3.0);
        let placement = Placement::new(Some([5.0, 5.0, 0.0]), Some(std::f64::consts::FRAC_PI_2));
        let (outer, holes) =
            plan_outline(Body::Extrusion(&extrusion), &placement).expect("outline");
        assert!(holes.is_empty());
        let (x0, y0, x1, y1) = ring_bounds(&outer);
        assert!((x1 - x0 - 1.0).abs() < 1e-9 && (y1 - y0 - 10.0).abs() < 1e-9);
        assert!(((x0 + x1) / 2.0 - 5.0).abs() < 1e-9 && ((y0 + y1) / 2.0 - 5.0).abs() < 1e-9);
        let swept = SolidShape::SweptPath {
            profile: ProfileDef::Rectangle {
                width_feet: 1.0,
                depth_feet: 1.0,
            },
            directrix_points_feet: vec![[0.0, 0.0, 0.0], [3.0, 4.0, 0.0]],
            fixed_reference: [0.0, 0.0, 1.0],
        };
        let (hull, _) =
            plan_outline(Body::Solid(&swept), &Placement::new(None, None)).expect("outline");
        assert_eq!(hull.len(), 4);
        assert!((signed_area(&hull) - 5.0).abs() < 1e-9);
    }

    /// a door's frame and panel are closed boxes touching along
    /// an edge. The mesh is closed (every directed edge as often as its
    /// reverse), so it is drawn one side, each box outward, and each face
    /// keeps its material.
    #[test]
    fn touching_closed_boxes_stay_one_sided_with_face_materials() {
        let cube = |o: f64, flip: bool| -> (Vec<[f64; 3]>, Vec<BrepTriangle>) {
            let v: Vec<[f64; 3]> = (0..8)
                .map(|i| {
                    [
                        o + (i & 1) as f64,
                        ((i >> 1) & 1) as f64,
                        ((i >> 2) & 1) as f64,
                    ]
                })
                .collect();
            let quads = [
                [0, 2, 3, 1],
                [4, 5, 7, 6],
                [0, 1, 5, 4],
                [2, 6, 7, 3],
                [0, 4, 6, 2],
                [1, 3, 7, 5],
            ];
            let mut t = Vec::new();
            for q in quads {
                for tri in [[q[0], q[1], q[2]], [q[0], q[2], q[3]]] {
                    t.push(if flip {
                        BrepTriangle(tri[0], tri[2], tri[1])
                    } else {
                        BrepTriangle(tri[0], tri[1], tri[2])
                    });
                }
            }
            (v, t)
        };
        // Two unit cubes meeting only along the edge x = 1, y = 1 (the
        // second spans x and y 1..2); the second is wound inward.
        let (mut vertices, mut triangles) = cube(0.0, false);
        let (v2, t2) = cube(1.0, true);
        let v2: Vec<[f64; 3]> = v2.iter().map(|p| [p[0], p[1] + 1.0, p[2]]).collect();
        let base = vertices.len() as u32;
        vertices.extend(v2);
        triangles.extend(
            t2.iter()
                .map(|t| BrepTriangle(t.0 + base, t.1 + base, t.2 + base)),
        );
        let face_materials: Vec<usize> = (0..24).map(|i| if i < 12 { 3 } else { 5 }).collect();
        let shape = SolidShape::TriangulatedFaceSet {
            vertices_feet: vertices,
            triangles,
            face_materials,
        };
        let mesh = solid_mesh(&shape).expect("mesh");
        assert_eq!(mesh.triangles.len(), 24, "drawn once, not two-sided");
        assert!((volume(&mesh) - 2.0).abs() < 1e-9, "each box outward");
        assert_eq!(mesh.face_materials.iter().filter(|m| **m == 3).count(), 12);
        assert_eq!(mesh.face_materials.iter().filter(|m| **m == 5).count(), 12);
    }
}
