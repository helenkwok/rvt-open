//! An extrusion's sketch with a nearly full arc. Revit 2011 saves a sketched
//! circle as one arc a thousandth of a turn short of a whole one; alone in its
//! sketch, that arc closes into a circle. With the short straight edge that
//! closes it beside it, the arc is kept as saved and the two make one ring,
//! the chord included. Built from curves in memory, so it needs no model.

use rvt::family_documents::sketch_rings;
use std::f64::consts::{PI, TAU};

const GAP: f64 = PI / 2000.0;

fn at(t: f64) -> [f64; 3] {
    [t.cos(), t.sin(), 0.0]
}

fn area(ring: &[[f64; 3]]) -> f64 {
    let n = ring.len();
    (0..n)
        .map(|i| ring[i][0] * ring[(i + 1) % n][1] - ring[(i + 1) % n][0] * ring[i][1])
        .sum::<f64>()
        / 2.0
}

#[test]
fn a_lone_arc_just_short_of_a_turn_closes() {
    let rings = sketch_rings(&[([0.0; 3], 1.0, [GAP, TAU - GAP])], &[]).expect("a ring");
    assert_eq!(rings.len(), 1);
    assert!((area(&rings[0]) - PI).abs() < 0.03, "{}", area(&rings[0]));
}

#[test]
fn an_arc_and_its_chord_make_one_ring_with_the_chord() {
    let chord = [at(TAU - GAP), at(GAP)];
    for (t, line) in [([GAP, TAU - GAP], chord), ([TAU - GAP, GAP], [chord[1], chord[0]])] {
        let rings = sketch_rings(&[([0.0; 3], 1.0, t)], &[line]).expect("a ring");
        assert_eq!(rings.len(), 1, "{t:?}");
        let ring = &rings[0];
        // 32 steps of the arc, closed by the chord: its two ends are both
        // corners of the ring, with no point of the arc between them.
        assert_eq!(ring.len(), 33, "{t:?}");
        let close = |p: [f64; 3], q: [f64; 3]| (0..3).all(|k| (p[k] - q[k]).abs() < 1e-9);
        let i = ring.iter().position(|p| close(*p, chord[0])).expect("chord end");
        let j = ring.iter().position(|p| close(*p, chord[1])).expect("chord end");
        assert!(i.abs_diff(j) == 1 || i.abs_diff(j) == ring.len() - 1, "{i} {j}");
        assert!((area(ring).abs() - PI).abs() < 0.03, "{}", area(ring));
    }
}

#[test]
fn a_half_circle_alone_draws_nothing() {
    assert!(sketch_rings(&[([0.0; 3], 1.0, [0.0, PI])], &[]).is_none());
}
