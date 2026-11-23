//! A nested instance takes the size of its own stored box, however it is
//! turned or mirrored. Revit stores a nested instance's box as the box, in
//! the host's frame, of the instance's own box turned by its placement; a
//! body larger than that is shrunk along its own axes onto it.
//!
//! Synthetic bodies and placements (generic sizes: a door panel, a column
//! capital, a lever handle), so these run everywhere, without sample files.

use rvt::ifc::family_file::{Affine, NestedStretch, stretch_nested_to_box};

type V = [f64; 3];
type M = [[f64; 3]; 3];

const TURNS: [f64; 9] = [10.0, 25.0, 30.0, 60.0, 75.0, 90.0, 120.0, 200.0, 300.0];

fn ap(m: &M, v: &V) -> V {
    std::array::from_fn(|k| (0..3).map(|j| m[k][j] * v[j]).sum())
}

fn mul(a: &M, b: &M) -> M {
    std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| a[i][k] * b[k][j]).sum()))
}

fn bbox(vs: &[V]) -> [V; 2] {
    let mut b = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    for v in vs {
        for k in 0..3 {
            b[0][k] = b[0][k].min(v[k]);
            b[1][k] = b[1][k].max(v[k]);
        }
    }
    b
}

/// A closed, outward box mesh.
fn box_mesh(lo: V, hi: V, vs: &mut Vec<V>, ts: &mut Vec<[usize; 3]>) {
    let b = vs.len();
    for i in 0..8 {
        vs.push(std::array::from_fn(|k| {
            if (i >> k) & 1 == 1 { hi[k] } else { lo[k] }
        }));
    }
    for t in [
        [0, 2, 1],
        [1, 2, 3],
        [4, 5, 6],
        [5, 7, 6],
        [0, 1, 4],
        [1, 5, 4],
        [2, 6, 3],
        [3, 6, 7],
        [0, 4, 2],
        [2, 4, 6],
        [1, 3, 5],
        [3, 7, 5],
    ] {
        ts.push([b + t[0], b + t[1], b + t[2]]);
    }
}

fn signed_volume(vs: &[V], ts: &[[usize; 3]]) -> f64 {
    ts.iter()
        .map(|t| {
            let (a, b, c) = (vs[t[0]], vs[t[1]], vs[t[2]]);
            (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                + a[2] * (b[0] * c[1] - b[1] * c[0]))
                / 6.0
        })
        .sum()
}

/// A nested body: its own (family-frame) mesh, its placement, and the factors
/// its type's size is of the family's saved one.
struct Case {
    name: &'static str,
    own: Vec<V>,
    tris: Vec<[usize; 3]>,
    map: Affine,
    factors: V,
}

impl Case {
    fn new(name: &'static str, boxes: &[(V, V)], l: M, t: V, factors: V) -> Case {
        let (mut own, mut tris) = (Vec::new(), Vec::new());
        for (lo, hi) in boxes {
            box_mesh(*lo, *hi, &mut own, &mut tris);
        }
        Case {
            name,
            own,
            tris,
            map: (l, t),
            factors,
        }
    }

    fn placed(&self, map: &Affine) -> Vec<V> {
        self.own
            .iter()
            .map(|p| {
                let q = ap(&map.0, p);
                std::array::from_fn(|k| q[k] + map.1[k])
            })
            .collect()
    }

    /// The stored box: the own box shrunk by `factors` about its centre,
    /// turned by `map`, boxed in the host frame.
    fn stored(&self, map: &Affine) -> [V; 2] {
        let [lo, hi] = bbox(&self.own);
        let c: V = std::array::from_fn(|k| (lo[k] + hi[k]) / 2.0);
        let corners: Vec<V> = (0..8)
            .map(|i| {
                let p: V = std::array::from_fn(|k| {
                    let e = if (i >> k) & 1 == 1 { hi[k] } else { lo[k] };
                    c[k] + (e - c[k]) * self.factors[k]
                });
                let q = ap(&map.0, &p);
                std::array::from_fn(|k| q[k] + map.1[k])
            })
            .collect();
        bbox(&corners)
    }
}

/// Each case is on its family's axes (identity or 90-degree turns).
fn cases() -> Vec<Case> {
    let id = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    vec![
        // A panel saved 4 ft wide, placed turned 90 degrees in a 3 ft opening.
        Case::new(
            "panel",
            &[
                ([0.0, 0.0, 0.0], [0.15, 4.0, 7.0]),
                ([0.05, 0.3, 3.0], [0.1, 0.6, 3.3]),
            ],
            [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            [-1.5, 0.24, 0.0],
            [1.0, 0.75, 1.0],
        ),
        // A capital saved 2.2 ft across, placed at 0.77 ft.
        Case::new(
            "capital",
            &[
                ([-1.1, -1.1, 0.0], [1.1, 1.1, 0.5]),
                ([-0.6, -0.6, 0.5], [0.6, 0.6, 0.9]),
            ],
            id,
            [4.0, 2.0, 9.0],
            [0.35, 0.35, 1.0],
        ),
        // A lever handle stood on its side (its own z along the host's y).
        Case::new(
            "handle",
            &[
                ([-0.15, -0.15, 0.0], [0.15, 0.15, 0.2]),
                ([0.0, -0.05, 0.05], [0.4, 0.05, 0.15]),
            ],
            [[-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]],
            [1.0, 0.25, 3.2],
            [0.8, 1.0, 0.7],
        ),
    ]
}

/// A turn about the host's z by `deg`, after a mirror across the host's x when
/// `mirror`.
fn turn(deg: f64, mirror: bool) -> M {
    let (c, s) = (deg.to_radians().cos(), deg.to_radians().sin());
    let m = if mirror { -1.0 } else { 1.0 };
    [[c * m, -s, 0.0], [s * m, c, 0.0], [0.0, 0.0, 1.0]]
}

fn max_diff(a: &[V], b: &[V]) -> f64 {
    a.iter()
        .zip(b)
        .map(|(p, q)| (0..3).map(|k| (p[k] - q[k]).abs()).fold(0.0, f64::max))
        .fold(0.0, f64::max)
}

#[test]
fn on_axes_stretch_is_the_per_axis_map() {
    for c in cases() {
        let mut vs = c.placed(&c.map);
        let stored = c.stored(&c.map);
        let [lo, hi] = bbox(&vs);
        // The per-axis map of the host frame: the body's box onto the stored box.
        let want: Vec<V> = vs
            .iter()
            .map(|v| {
                std::array::from_fn(|k| {
                    stored[0][k] + (v[k] - lo[k]) * (stored[1][k] - stored[0][k]) / (hi[k] - lo[k])
                })
            })
            .collect();
        assert_eq!(
            stretch_nested_to_box(&mut vs, stored, &c.map),
            NestedStretch::Stretched,
            "{}",
            c.name
        );
        let d = max_diff(&vs, &want);
        assert!(d < 1e-9, "{}: {d} ft from the per-axis map", c.name);
    }
}

#[test]
fn turned_and_mirrored_match_the_on_axes_result() {
    let mut n = 0;
    for c in cases() {
        let mut on_axes = c.placed(&c.map);
        assert_eq!(
            stretch_nested_to_box(&mut on_axes, c.stored(&c.map), &c.map),
            NestedStretch::Stretched
        );
        for mirror in [false, true] {
            for deg in TURNS {
                let r = turn(deg, mirror);
                let map: Affine = (mul(&r, &c.map.0), ap(&r, &c.map.1));
                let mut vs = c.placed(&map);
                let before = signed_volume(&vs, &c.tris);
                let got = stretch_nested_to_box(&mut vs, c.stored(&map), &map);
                assert_eq!(
                    got,
                    NestedStretch::Stretched,
                    "{} {deg} mirror {mirror}",
                    c.name
                );
                // The same body as on the axes, turned (and mirrored) likewise.
                let want: Vec<V> = on_axes.iter().map(|v| ap(&r, v)).collect();
                let d = max_diff(&vs, &want);
                assert!(d < 1e-6, "{} {deg} mirror {mirror}: {d} ft", c.name);
                // The reflection and the faces' winding are kept: the signed
                // volume keeps its sign and scales by the factors.
                let after = signed_volume(&vs, &c.tris);
                let f: f64 = c.factors.iter().product();
                assert!(
                    before.signum() == after.signum(),
                    "{} {deg}: reflection lost",
                    c.name
                );
                assert!(
                    (after / before - f).abs() < 1e-9,
                    "{} {deg}: volume {after} of {before}",
                    c.name
                );
                assert_eq!(before < 0.0, mirror, "{} {deg}: mirrored body", c.name);
                n += 1;
            }
        }
    }
    assert_eq!(n, 3 * 2 * TURNS.len());
}

#[test]
fn a_turn_near_45_degrees_is_undetermined() {
    for c in cases() {
        for mirror in [false, true] {
            let r = turn(44.0, mirror);
            let map: Affine = (mul(&r, &c.map.0), ap(&r, &c.map.1));
            let mut vs = c.placed(&map);
            let before = vs.clone();
            assert_eq!(
                stretch_nested_to_box(&mut vs, c.stored(&map), &map),
                NestedStretch::Undetermined,
                "{}",
                c.name
            );
            assert_eq!(
                vs, before,
                "{}: an undetermined body keeps its size",
                c.name
            );
        }
    }
}

#[test]
fn never_stretches_up() {
    let id = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let mut grow = cases().remove(1);
    // A stored box larger than the body (lines and reference planes count in
    // it): the body keeps its size.
    grow.factors = [1.4, 1.4, 1.0];
    for map in [grow.map, (mul(&turn(30.0, true), &id), [1.0, 2.0, 0.0])] {
        let mut vs = grow.placed(&map);
        let before = vs.clone();
        assert_eq!(
            stretch_nested_to_box(&mut vs, grow.stored(&map), &map),
            NestedStretch::Fits
        );
        assert_eq!(vs, before);
    }
    // Turned by 30 degrees, a stored box inside the body's turned box that
    // only an own axis grown by 30% (and another shrunk) gives back: another
    // shape, not another size.
    let plank = Case::new(
        "plank",
        &[([0.0, 0.0, 0.0], [1.0, 4.0, 0.5])],
        mul(&turn(30.0, false), &id),
        [0.0; 3],
        [1.3, 0.75, 1.0],
    );
    let mut vs = plank.placed(&plank.map);
    let before = vs.clone();
    let stored = plank.stored(&plank.map);
    let turned_body = bbox(&plank.stored_unshrunk());
    assert!((0..3).all(|k| stored[0][k] >= turned_body[0][k] && stored[1][k] <= turned_body[1][k]));
    assert_eq!(
        stretch_nested_to_box(&mut vs, stored, &plank.map),
        NestedStretch::BeyondLimits
    );
    assert_eq!(vs, before);
}

impl Case {
    /// The corners of the own box turned by the placement (factors 1).
    fn stored_unshrunk(&self) -> Vec<V> {
        let s = Case {
            name: self.name,
            own: self.own.clone(),
            tris: Vec::new(),
            map: self.map,
            factors: [1.0; 3],
        };
        let b = s.stored(&s.map);
        b.to_vec()
    }
}
