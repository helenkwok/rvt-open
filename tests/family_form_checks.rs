//! Checks a family's solved forms pass before they are drawn: a revolution's
//! stored box against its solved one, and a void cut from a solid.
//!
//! Synthetic forms (generic sizes), so these run everywhere, without sample
//! files.

use rvt::family_documents::{BBOX_TOLERANCE, revolution_box_matches};
use rvt::mesh_csg::difference;

type V = [f64; 3];

fn bounds(vs: &[V]) -> [V; 2] {
    let mut b = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    for v in vs {
        for k in 0..3 {
            b[0][k] = b[0][k].min(v[k]);
            b[1][k] = b[1][k].max(v[k]);
        }
    }
    b
}

/// Points of a ring of `radius` about the z axis from angle 0 through
/// `sweep`, at heights 0 and 1.
fn swept(radius: f64, sweep: f64) -> Vec<V> {
    let n = 64;
    (0..=n)
        .flat_map(|i| {
            let a = sweep * i as f64 / n as f64;
            [0.0, 1.0].map(|z| [radius * a.cos(), radius * a.sin(), z])
        })
        .collect()
}

const Z_AXIS: [V; 2] = [[0.0; 3], [0.0, 0.0, 1.0]];

#[test]
fn a_partial_revolution_gets_only_its_own_chord_sag() {
    // A 100 ft radius turned through 0.005 rad: half a foot across. A stored
    // box 0.3 ft short of it across the turn is another form; one chord
    // across the whole turn falls short by 0.0003 ft.
    let sweep = 0.005;
    let vs = swept(100.0, sweep);
    let solved = bounds(&vs);
    let mut stored = solved;
    stored[1][1] -= 0.3;
    assert!(!revolution_box_matches(
        &stored, &solved, Z_AXIS, sweep, &vs
    ));
    let mut stored = solved;
    stored[1][1] -= 100.0 * (1.0 - (sweep / 2.0).cos()) + BBOX_TOLERANCE * 0.5;
    assert!(revolution_box_matches(&stored, &solved, Z_AXIS, sweep, &vs));
}

#[test]
fn a_whole_turn_keeps_the_six_facet_sag() {
    // A 0.5 ft radius turned whole, shown in six facets: its box lies inside
    // by up to 1 - cos(pi/6) of the radius (0.067 ft) across, never along
    // the axis.
    let tau = std::f64::consts::TAU;
    let vs = swept(0.5, tau);
    let solved = bounds(&vs);
    let mut inside = solved;
    inside[0][0] += 0.06;
    assert!(revolution_box_matches(&inside, &solved, Z_AXIS, tau, &vs));
    let mut beyond = solved;
    beyond[0][0] += 0.08;
    assert!(!revolution_box_matches(&beyond, &solved, Z_AXIS, tau, &vs));
    let mut along = solved;
    along[1][2] -= 0.06;
    assert!(!revolution_box_matches(&along, &solved, Z_AXIS, tau, &vs));
}

/// A closed, outward box mesh.
fn cube(lo: V, hi: V) -> (Vec<V>, Vec<[u32; 3]>) {
    let vs: Vec<V> = (0..8)
        .map(|i| std::array::from_fn(|k| if (i >> k) & 1 == 1 { hi[k] } else { lo[k] }))
        .collect();
    let ts = vec![
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
    ];
    (vs, ts)
}

fn volume(vs: &[V], ts: &[[u32; 3]]) -> f64 {
    ts.iter()
        .map(|t| {
            let [a, b, c] = t.map(|i| vs[i as usize]);
            a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                + a[2] * (b[0] * c[1] - b[1] * c[0])
        })
        .sum::<f64>()
        / 6.0
}

#[test]
fn a_small_solid_keeps_its_faces_through_a_cut() {
    // A pin 0.00002 ft across (faces of real area, though small): a cutter
    // that misses it leaves it whole, and one through half of it halves it.
    let s = 2e-5;
    let (av, at) = cube([0.0; 3], [s; 3]);
    let full = volume(&av, &at);
    let (bv, bt) = cube([1.0; 3], [2.0; 3]);
    let (v, t) = difference((&av, &at), (&bv, &bt)).expect("a cut");
    assert!(
        (volume(&v, &t) - full).abs() < 1e-6 * full,
        "{} of {full}",
        volume(&v, &t)
    );
    let (bv, bt) = cube([s / 2.0, -1.0, -1.0], [1.0, 1.0, 1.0]);
    let (v, t) = difference((&av, &at), (&bv, &bt)).expect("a cut");
    assert!(
        (volume(&v, &t) - full / 2.0).abs() < 1e-6 * full,
        "{} of {full}",
        volume(&v, &t)
    );
}

#[test]
fn a_cut_past_its_piece_limit_gives_up() {
    // More faces than a cut may hold: no cut is made (the caller keeps the
    // uncut solid).
    let (av, at) = cube([0.0; 3], [1.0; 3]);
    let many: Vec<[u32; 3]> = at.iter().copied().cycle().take(900_000).collect();
    let (bv, bt) = cube([0.5; 3], [2.0; 3]);
    assert!(difference((&av, &many), (&bv, &bt)).is_none());
    assert!(difference((&av, &at), (&bv, &bt)).is_some());
}
