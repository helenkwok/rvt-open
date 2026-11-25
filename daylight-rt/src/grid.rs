//! Sensor grids: points on a regular grid over a set of horizontal faces.

use crate::{V, cross, dot, sub};

/// Points of a square grid of `spacing` over the faces' plan, kept where a
/// point (and, with `inset` > 0, the four points `inset` away along x and y)
/// falls in or on a face; at height `z`. The grid starts half a spacing in from
/// the faces' lowest x and y, row by row (x fastest).
pub fn over(faces: &[[V; 3]], z: f64, spacing: f64, inset: f64) -> Vec<V> {
    if faces.is_empty() || spacing.is_nan() || spacing <= 0.0 {
        return Vec::new();
    }
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for f in faces {
        for p in f {
            for k in 0..2 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
    }
    let axis = |a: f64, b: f64| {
        let start = a + spacing / 2.0;
        // as numpy's arange: ceil((stop - start) / step) values start + i delta, delta = (start + step) - start
        let n = ((b - start) / spacing).ceil().max(0.0) as usize;
        let delta = (start + spacing) - start;
        (0..n).map(move |i| start + i as f64 * delta)
    };
    let inside = |x: f64, y: f64| faces.iter().any(|f| in_triangle(x, y, f));
    let mut out = Vec::new();
    for y in axis(lo[1], hi[1]) {
        for x in axis(lo[0], hi[0]) {
            let keep = inside(x, y)
                && (inset == 0.0
                    || [(inset, 0.0), (-inset, 0.0), (0.0, inset), (0.0, -inset)]
                        .iter()
                        .all(|(dx, dy)| inside(x + dx, y + dy)));
            if keep {
                out.push([x, y, z]);
            }
        }
    }
    out
}

/// In or on the triangle's plan (edges included).
fn in_triangle(x: f64, y: f64, t: &[V; 3]) -> bool {
    let c = |o: V, u: V| (u[0] - o[0]) * (y - o[1]) - (u[1] - o[1]) * (x - o[0]);
    let (d1, d2, d3) = (c(t[0], t[1]), c(t[1], t[2]), c(t[2], t[0]));
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
}

/// The z component of a face's unit normal (from its winding), 0 for a degenerate face.
pub fn normal_z(t: &[V; 3]) -> f64 {
    let n = cross(sub(t[1], t[0]), sub(t[2], t[0]));
    n[2] / (dot(n, n).sqrt() + 1e-12)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_two_metre_square_at_half_a_metre_holds_sixteen_points() {
        let sq = [
            [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [2.0, 2.0, 0.0]],
            [[0.0, 0.0, 0.0], [2.0, 2.0, 0.0], [0.0, 2.0, 0.0]],
        ];
        let g = over(&sq, 0.8, 0.5, 0.0);
        assert_eq!(g.len(), 16);
        assert_eq!(g[0], [0.25, 0.25, 0.8]);
        assert_eq!(g[1], [0.75, 0.25, 0.8]);
        assert_eq!(over(&sq, 0.8, 0.5, 0.35).len(), 4);
        assert!(normal_z(&sq[0]) > 0.99);
    }
}
