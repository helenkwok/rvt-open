//! A Hermite spline profile of a ruled surface or a surface of revolution is
//! cut into the equal parameter steps its curvature asks for, aiming at a chord
//! error and capped.
//!
//! `Curve::spline_steps` gives the number of equal steps between two
//! parameters that aim to hold the chord error of the spline to a distance (one
//! for a curve that is not a spline, at most 128, which holds the distance only
//! as far as that many steps do), and `Curve::speed` the greatest
//! length of its derivative. Built from nodes in memory, so it needs no model.

use rvt::research::native_parametric_surface::Curve;

const CHORD: f64 = 0.001;

/// A quarter circle of radius `r` as two nodes (parameter 0 and 1) with the
/// tangents of the circle, per unit parameter.
fn quarter(r: f64) -> Curve {
    let k = r * std::f64::consts::FRAC_PI_2;
    Curve::Hermite {
        nodes: vec![
            (0., [r, 0., 0.], [0., k, 0.]),
            (1., [0., r, 0.], [-k, 0., 0.]),
        ],
        range: [0., 1.],
    }
}

fn dist(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// The greatest distance of the spline from the polyline of `steps` equal
/// parameter steps, tested at 16 points of each step.
fn deviation(curve: &Curve, steps: usize) -> f64 {
    let mut worst = 0.0_f64;
    for i in 0..steps {
        let (a, b) = (i as f64 / steps as f64, (i + 1) as f64 / steps as f64);
        let (pa, pb) = (curve.eval(a), curve.eval(b));
        for j in 1..16 {
            let f = j as f64 / 16.;
            let on_chord = std::array::from_fn(|k| pa[k] + f * (pb[k] - pa[k]));
            worst = worst.max(dist(curve.eval(a + f * (b - a)), on_chord));
        }
    }
    worst
}

#[test]
fn a_straight_spline_is_one_step() {
    let line = Curve::Hermite {
        nodes: vec![
            (0., [0., 0., 0.], [4., 0., 0.]),
            (1., [4., 0., 0.], [4., 0., 0.]),
        ],
        range: [0., 1.],
    };
    assert_eq!(line.spline_steps(0., 1., CHORD), 1);
    assert!((line.speed(0., 1.) - 4.).abs() < 1e-9);
}

#[test]
fn a_curved_spline_is_cut_to_hold_its_chord() {
    let curve = quarter(5.);
    let steps = curve.spline_steps(0., 1., CHORD);
    assert!((2..=128).contains(&steps), "{steps} steps");
    // The chord error of the steps asked for is the one asked: within the
    // distance, and the step count one fewer is not (the bound is the largest
    // second difference, so it holds with some slack).
    let held = deviation(&curve, steps);
    assert!(held <= CHORD * 1.05, "{steps} steps leave {held} ft");
    assert!(
        deviation(&curve, steps / 3) > CHORD,
        "a third of the steps would have held it too"
    );
    // A tighter curve takes more steps.
    assert!(quarter(0.5).spline_steps(0., 1., CHORD) < steps);
    assert!(quarter(50.).spline_steps(0., 1., CHORD) > steps);
}

#[test]
fn the_steps_of_a_spline_stop_at_128() {
    assert_eq!(quarter(1e6).spline_steps(0., 1., CHORD), 128);
}

#[test]
fn a_curve_that_is_not_a_spline_is_one_step() {
    let line = Curve::line([0., 0., 0.], [3., 4., 0.], [0., 2.]);
    assert_eq!(line.spline_steps(0., 2., CHORD), 1);
    assert!((line.speed(0., 2.) - 5.).abs() < 1e-9);
}

/// Every node of this spline lies on `x = 1` and the samples of a fixed grid
/// fall on the nodes, but between two nodes `x` reaches 1.25: a bound taken at
/// the samples alone would read it as straight.
#[test]
fn a_bend_between_two_nodes_is_not_missed() {
    let nodes = (0..=64)
        .map(|i| {
            let t = i as f64 / 64.;
            let tangent = if i % 2 == 0 { 64. } else { -64. };
            (t, [1., 0., t], [tangent, 0., 1.])
        })
        .collect();
    let curve = Curve::Hermite {
        nodes,
        range: [0., 1.],
    };
    assert_eq!(curve.spline_steps(0., 1., CHORD), 128);
}
