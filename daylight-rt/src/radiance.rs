//! The Radiance scene text both tiers read: `plastic` and `glass` modifiers and
//! triangles as `polygon`s, and sensor files (`x y z dx dy dz` per line).
//!
//! The writer prints materials to 4 decimals and coordinates to 5 (0.01 mm), and
//! [`Model`]s the viewer traces are rounded the same way ([`round_trip`]), so
//! the instant tier and Radiance see the same numbers.

use crate::{Model, Modifier, Sensor, Surface, Triangle, V, norm};
use std::collections::HashMap;
use std::fmt::Write;

/// Reads `plastic`/`glass` modifiers and `polygon`s (fanned into triangles) from
/// Radiance scene texts, in order; other primitives are skipped. A polygon whose
/// modifier is unknown gets a grey plastic.
pub fn parse(texts: &[&str]) -> Result<Model, String> {
    let mut model = Model::default();
    let mut by_id: HashMap<String, u32> = HashMap::new();
    for text in texts {
        let tok: Vec<&str> = text
            .lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .flat_map(str::split_whitespace)
            .collect();
        let mut i = 0;
        while i + 2 < tok.len() {
            let (modname, ty, id) = (tok[i], tok[i + 1], tok[i + 2]);
            i += 3;
            let mut reals: Vec<f64> = Vec::new();
            for k in 0..3 {
                let n: usize = tok
                    .get(i)
                    .and_then(|s| s.parse().ok())
                    .ok_or_else(|| format!("{id}: argument count expected"))?;
                i += 1;
                if i + n > tok.len() {
                    return Err(format!("{id}: {n} arguments expected"));
                }
                if k == 2 {
                    reals = tok[i..i + n]
                        .iter()
                        .map(|s| s.parse().map_err(|_| format!("{id}: bad number {s}")))
                        .collect::<Result<_, _>>()?;
                }
                i += n;
            }
            let r3 = |o: usize| [reals[o], reals[o + 1], reals[o + 2]];
            match ty {
                "plastic" if reals.len() >= 5 => {
                    let surface = Surface::Plastic {
                        rgb: r3(0),
                        spec: reals[3],
                        rough: reals[4],
                    };
                    push_modifier(&mut model, &mut by_id, id, surface);
                }
                "glass" if reals.len() >= 3 => {
                    push_modifier(&mut model, &mut by_id, id, Surface::Glass { tn: r3(0) });
                }
                "polygon" if reals.len() >= 9 && reals.len().is_multiple_of(3) => {
                    let m = match by_id.get(modname) {
                        Some(&m) => m,
                        None => push_modifier(
                            &mut model,
                            &mut by_id,
                            modname,
                            Surface::Plastic {
                                rgb: [0.5; 3],
                                spec: 0.0,
                                rough: 0.0,
                            },
                        ),
                    };
                    let p: Vec<V> = reals.chunks(3).map(|c| [c[0], c[1], c[2]]).collect();
                    if !convex(&p) {
                        return Err(format!(
                            "{id}: a concave polygon is not read (only triangles and convex polygons)"
                        ));
                    }
                    for k in 1..p.len() - 1 {
                        model.triangles.push(Triangle {
                            v: [p[0], p[k], p[k + 1]],
                            modifier: m,
                        });
                    }
                }
                _ => {}
            }
        }
    }
    Ok(model)
}

/// True when every corner turns the same way about the polygon's normal (a
/// fan of its first vertex then covers exactly the polygon).
fn convex(p: &[V]) -> bool {
    if p.len() <= 3 {
        return true;
    }
    let mut n = [0.0; 3];
    for k in 0..p.len() {
        let c = crate::cross(p[k], p[(k + 1) % p.len()]);
        n = [n[0] + c[0], n[1] + c[1], n[2] + c[2]];
    }
    (0..p.len()).all(|k| {
        let (a, b, c) = (p[k], p[(k + 1) % p.len()], p[(k + 2) % p.len()]);
        crate::dot(crate::cross(crate::sub(b, a), crate::sub(c, b)), n) >= -1e-12
    })
}

fn push_modifier(
    model: &mut Model,
    by_id: &mut HashMap<String, u32>,
    id: &str,
    surface: Surface,
) -> u32 {
    let m = model.modifiers.len() as u32;
    model.modifiers.push(Modifier {
        id: id.to_string(),
        surface,
        label: id.to_string(),
        assumed: false,
    });
    by_id.insert(id.to_string(), m);
    m
}

/// The modifiers as Radiance text (4 decimals).
pub fn write_materials(model: &Model) -> String {
    let mut s = String::new();
    for m in &model.modifiers {
        match m.surface {
            Surface::Plastic { rgb, spec, rough } => {
                let _ = write!(
                    s,
                    "void plastic {}\n0\n0\n5 {:.4} {:.4} {:.4} {} {}\n\n",
                    m.id, rgb[0], rgb[1], rgb[2], spec, rough
                );
            }
            Surface::Glass { tn } => {
                let _ = write!(
                    s,
                    "void glass {}\n0\n0\n3 {:.4} {:.4} {:.4}\n\n",
                    m.id, tn[0], tn[1], tn[2]
                );
            }
        }
    }
    s
}

/// The triangles as Radiance `polygon`s (5 decimals), named `t<n>`.
pub fn write_triangles(model: &Model) -> String {
    let mut s = String::with_capacity(model.triangles.len() * 100);
    for (n, t) in model.triangles.iter().enumerate() {
        let id = model
            .modifiers
            .get(t.modifier as usize)
            .map_or("void", |m| m.id.as_str());
        let _ = write!(s, "{id} polygon t{n}\n0\n0\n9");
        for v in t.v {
            let _ = write!(s, " {:.5} {:.5} {:.5}", v[0], v[1], v[2]);
        }
        s.push('\n');
    }
    s
}

/// Sensors as `x y z dx dy dz` lines (4 decimals).
pub fn write_sensors(sensors: &[Sensor]) -> String {
    let mut s = String::new();
    for p in sensors {
        let _ = writeln!(
            s,
            "{:.4} {:.4} {:.4} {} {} {}",
            p.p[0], p.p[1], p.p[2], p.n[0], p.n[1], p.n[2]
        );
    }
    s
}

/// Reads `x y z dx dy dz` lines; shorter lines are skipped.
pub fn parse_sensors(text: &str) -> Vec<Sensor> {
    text.lines()
        .filter_map(|l| {
            let v: Vec<f64> = l
                .split_whitespace()
                .filter_map(|x| x.parse().ok())
                .collect();
            (v.len() >= 6).then(|| Sensor {
                p: [v[0], v[1], v[2]],
                n: norm([v[3], v[4], v[5]]),
            })
        })
        .collect()
}

/// `x` as the writer prints it with `places` decimals, read back.
pub fn round(x: f64, places: usize) -> f64 {
    format!("{x:.places$}").parse().unwrap_or(x)
}

/// The model as the writer would print and the reader read it back: materials
/// to 4 decimals, coordinates to 5.
pub fn round_trip(model: &mut Model) {
    for m in &mut model.modifiers {
        match &mut m.surface {
            Surface::Plastic { rgb, .. } => rgb.iter_mut().for_each(|c| *c = round(*c, 4)),
            Surface::Glass { tn } => tn.iter_mut().for_each(|c| *c = round(*c, 4)),
        }
    }
    for t in &mut model.triangles {
        for v in &mut t.v {
            v.iter_mut().for_each(|c| *c = round(*c, 5));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_then_read_gives_the_same_model() {
        let text = "void plastic m0\n0\n0\n5 0.2122 0.2122 0.2122 0 0\n\nvoid glass m1\n0\n0\n3 0.9432 0.9432 0.9432\n\n\
                    m1 polygon a\n0\n0\n12 0 0 0 1 0 0 1 1 0 0 1 0\n";
        let m = parse(&[text]).unwrap();
        assert_eq!(m.triangles.len(), 2);
        assert_eq!(m.triangles[1].modifier, 1);
        let again = parse(&[&write_materials(&m), &write_triangles(&m)]).unwrap();
        assert_eq!(again.triangles.len(), 2);
        assert_eq!(again.modifiers[1].surface, m.modifiers[1].surface);
        assert_eq!(round(1.234_565_1, 5), 1.23457);
        let s = parse_sensors("1 2 3 0 0 2\nbad\n");
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].n, [0.0, 0.0, 1.0]);
        // a U-shaped polygon is refused rather than fanned across its notch
        let u = "void plastic m\n0\n0\n5 .5 .5 .5 0 0\nm polygon u\n0\n0\n24 0 0 0 3 0 0 3 3 0 2 3 0 2 1 0 1 1 0 1 3 0 0 3 0\n";
        assert!(parse(&[u]).is_err());
    }
}
