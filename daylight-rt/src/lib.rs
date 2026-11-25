//! Daylight and sun metrics for the browser viewer's instant tier.
//!
//! A small ray tracer over a triangle scene with two surface kinds, Radiance's
//! `plastic` (a diffuse reflectance) and `glass` (thin glass, two-interface
//! Fresnel at n = 1.52 as Radiance's reference manual describes its glass type):
//!
//! - [`sun_hours`]: for each sensor, how many of the given sun directions reach
//!   it directly (through glass, not through anything opaque);
//! - [`sky_view`]: the sky view factor, the share of a uniform sky above the
//!   horizon a sensor sees, glass weighted by its transmittance;
//! - [`daylight_factor`]: the daylight factor (%) under the CIE overcast sky with
//!   `ab` diffuse bounces (Radiance's `-ab` meaning, no ambient value), by
//!   cosine-weighted Monte Carlo paths. A preview: it has no irradiance cache and
//!   no sampling toward windows, so per sensor it is noisier than Radiance at the
//!   same time; the validated numbers come from Radiance itself.
//!
//! The scene, the sensors and the solar position are shared with the validated
//! tier: [`radiance`] writes and reads the Radiance scene text, [`sun`] is the
//! solar position as `gensky` computes it, and [`grid`] lays sensor grids.
//! Coordinates are metres, Z up, in the model's own (project) frame; the sun is
//! turned into that frame by the angle to true north ([`sun::to_project`]).

#![forbid(unsafe_code)]

pub mod grid;
pub mod radiance;
pub mod sun;

#[cfg(feature = "glb")]
pub mod glb;

/// A point or direction, metres, Z up.
pub type V = [f64; 3];

pub(crate) fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub(crate) fn cross(a: V, b: V) -> V {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
pub(crate) fn dot(a: V, b: V) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub(crate) fn norm(a: V) -> V {
    let l = dot(a, a).sqrt();
    [a[0] / l, a[1] / l, a[2] / l]
}
fn madd(a: V, b: V, s: f64) -> V {
    [a[0] + b[0] * s, a[1] + b[1] * s, a[2] + b[2] * s]
}

/// Radiance's grey value of a colour (`bright()` in its sources: 0.265 R + 0.670 G + 0.065 B).
pub fn grey(c: [f64; 3]) -> f64 {
    0.265 * c[0] + 0.670 * c[1] + 0.065 * c[2]
}

/// A surface material, as Radiance's primitive of the same name.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Surface {
    /// `plastic`: diffuse reflectance per channel, specularity, roughness.
    Plastic {
        rgb: [f64; 3],
        spec: f64,
        rough: f64,
    },
    /// `glass`: transmissivity per channel (Radiance's glass takes transmissivity, not transmittance).
    Glass { tn: [f64; 3] },
}

/// A named material (Radiance modifier).
#[derive(Debug, Clone, PartialEq)]
pub struct Modifier {
    /// The Radiance identifier (`m0`, `m1`, ...).
    pub id: String,
    pub surface: Surface,
    /// The material's name in the model, for reports.
    pub label: String,
    /// The colour is not the file's own (a placeholder the reader chose), so the
    /// reflectance is assumed.
    pub assumed: bool,
}

/// One triangle and the index of its modifier.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Triangle {
    pub v: [V; 3],
    pub modifier: u32,
}

/// The scene both tiers read: modifiers and triangles.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Model {
    pub modifiers: Vec<Modifier>,
    pub triangles: Vec<Triangle>,
}

/// A sensor: position and the direction its surface faces.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sensor {
    pub p: V,
    pub n: V,
}

#[derive(Clone, Copy)]
enum Mat {
    Diffuse(f64),
    Glass(f64),
}

struct Tri {
    p0: V,
    e1: V,
    e2: V,
    n: V,
    m: Mat,
}

/// A BVH node. `count > 0`: a leaf over triangles `[first, first + count)`;
/// otherwise the left child is the next node and `first` is the right child.
struct Node {
    lo: V,
    hi: V,
    first: u32,
    count: u32,
}

/// The scene ready for tracing: triangles with their material values, in a BVH.
pub struct Tracer {
    tris: Vec<Tri>,
    nodes: Vec<Node>,
}

impl Tracer {
    /// Builds the BVH. Degenerate triangles are skipped.
    pub fn new(model: &Model) -> Tracer {
        let mats: Vec<Mat> = model
            .modifiers
            .iter()
            .map(|m| match m.surface {
                Surface::Plastic { rgb, spec, .. } => Mat::Diffuse(grey(rgb) * (1.0 - spec)),
                Surface::Glass { tn } => Mat::Glass(grey(tn)),
            })
            .collect();
        let mut tris = Vec::with_capacity(model.triangles.len());
        for t in &model.triangles {
            let (e1, e2) = (sub(t.v[1], t.v[0]), sub(t.v[2], t.v[0]));
            let c = cross(e1, e2);
            if dot(c, c) > 1e-24 {
                let m = mats
                    .get(t.modifier as usize)
                    .copied()
                    .unwrap_or(Mat::Diffuse(0.5));
                tris.push(Tri {
                    p0: t.v[0],
                    e1,
                    e2,
                    n: norm(c),
                    m,
                });
            }
        }
        let mut tracer = Tracer {
            tris,
            nodes: Vec::new(),
        };
        let n = tracer.tris.len();
        if n > 0 {
            tracer.build(0, n);
        }
        tracer
    }

    /// Number of triangles traced.
    pub fn len(&self) -> usize {
        self.tris.len()
    }

    /// True when the scene holds no triangle.
    pub fn is_empty(&self) -> bool {
        self.tris.is_empty()
    }

    fn bounds(&self, a: usize, b: usize) -> (V, V) {
        let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
        for tr in &self.tris[a..b] {
            for p in [tr.p0, madd(tr.p0, tr.e1, 1.0), madd(tr.p0, tr.e2, 1.0)] {
                for k in 0..3 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
            }
        }
        (lo, hi)
    }

    // ponytail: median split; a surface-area heuristic if large models trace slowly.
    fn build(&mut self, a: usize, b: usize) -> u32 {
        let (lo, hi) = self.bounds(a, b);
        let id = self.nodes.len() as u32;
        self.nodes.push(Node {
            lo,
            hi,
            first: a as u32,
            count: (b - a) as u32,
        });
        if b - a <= 4 {
            return id;
        }
        let ext = sub(hi, lo);
        let ax = if ext[0] > ext[1] && ext[0] > ext[2] {
            0
        } else if ext[1] > ext[2] {
            1
        } else {
            2
        };
        let key = |t: &Tri| 3.0 * t.p0[ax] + t.e1[ax] + t.e2[ax];
        self.tris[a..b].sort_by(|x, y| key(x).total_cmp(&key(y)));
        let mid = (a + b) / 2;
        self.build(a, mid);
        let right = self.build(mid, b);
        let node = &mut self.nodes[id as usize];
        node.count = 0;
        node.first = right;
        id
    }

    /// The nearest hit along `o + t d`, t > 1e-6: distance and triangle index.
    fn hit(&self, o: V, d: V) -> Option<(f64, usize)> {
        if self.nodes.is_empty() {
            return None;
        }
        let inv = [1.0 / d[0], 1.0 / d[1], 1.0 / d[2]];
        let mut best: Option<(f64, usize)> = None;
        let mut stack = [0u32; 128];
        let mut sp = 1;
        while sp > 0 {
            sp -= 1;
            let id = stack[sp] as usize;
            let nd = &self.nodes[id];
            let (mut t0, mut t1) = (0.0f64, best.map_or(f64::MAX, |b| b.0));
            for k in 0..3 {
                let (mut u, mut v) = ((nd.lo[k] - o[k]) * inv[k], (nd.hi[k] - o[k]) * inv[k]);
                if u > v {
                    std::mem::swap(&mut u, &mut v);
                }
                t0 = t0.max(u);
                t1 = t1.min(v);
            }
            if t0 > t1 {
                continue;
            }
            if nd.count > 0 {
                for k in nd.first as usize..(nd.first + nd.count) as usize {
                    let tr = &self.tris[k];
                    let p = cross(d, tr.e2);
                    let det = dot(tr.e1, p);
                    if det.abs() < 1e-14 {
                        continue;
                    }
                    let id = 1.0 / det;
                    let s = sub(o, tr.p0);
                    let u = dot(s, p) * id;
                    if !(0.0..=1.0).contains(&u) {
                        continue;
                    }
                    let q = cross(s, tr.e1);
                    let v = dot(d, q) * id;
                    if v < 0.0 || u + v > 1.0 {
                        continue;
                    }
                    let t = dot(tr.e2, q) * id;
                    if t > 1e-6 && t < best.map_or(f64::MAX, |b| b.0) {
                        best = Some((t, k));
                    }
                }
            } else if sp + 2 <= stack.len() {
                stack[sp] = id as u32 + 1;
                stack[sp + 1] = nd.first;
                sp += 2;
            }
        }
        best
    }

    /// True when nothing opaque lies along `o + t d` (glass lets the ray on).
    fn sees(&self, mut o: V, d: V) -> bool {
        loop {
            match self.hit(o, d) {
                None => return true,
                Some((t, k)) => match self.tris[k].m {
                    Mat::Glass(_) => o = madd(o, d, t),
                    Mat::Diffuse(_) => return false,
                },
            }
        }
    }
}

/// xorshift64: the paths' random numbers, seeded per sensor so a sensor's value
/// does not depend on how the sensors are split between workers.
struct Rng(u64);
impl Rng {
    fn for_sensor(seed: u64, index: u64) -> Rng {
        Rng(seed ^ 0x9E37_79B9_7F4A_7C15u64.wrapping_mul(index + 1))
    }
    fn f(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
}

fn cos_dir(n: V, r: &mut Rng) -> V {
    let (u1, u2) = (r.f(), r.f());
    let (rr, ph) = (u1.sqrt(), 2.0 * std::f64::consts::PI * u2);
    let a = if n[0].abs() > 0.9 {
        [0.0, 1.0, 0.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let t = norm(cross(a, n));
    let b = cross(n, t);
    let (x, y, z) = (rr * ph.cos(), rr * ph.sin(), (1.0 - u1).sqrt());
    [
        t[0] * x + b[0] * y + n[0] * z,
        t[1] * x + b[1] * y + n[1] * z,
        t[2] * x + b[2] * y + n[2] * z,
    ]
}

/// Thin-glass reflectance and transmittance at incidence cosine `cos1` for
/// transmissivity `tn` (two interfaces, n = 1.52, absorption `tn^(1/cos2)`).
pub fn glass_rt(tn: f64, cos1: f64) -> (f64, f64) {
    const N: f64 = 1.52;
    let c1 = cos1.abs().max(1e-6);
    let c2 = (1.0 - (1.0 - c1 * c1) / (N * N)).sqrt();
    let d = tn.powf(1.0 / c2);
    let re = ((c1 - N * c2) / (c1 + N * c2)).powi(2);
    let rm = ((1.0 / c1 - N / c2) / (1.0 / c1 + N / c2)).powi(2);
    let (mut r, mut t) = (0.0, 0.0);
    for x in [re, rm] {
        t += 0.5 * (1.0 - x) * (1.0 - x) * d / (1.0 - x * x * d * d);
        r += 0.5 * x * (1.0 + (1.0 - 2.0 * x) * d * d) / (1.0 - x * x * d * d);
    }
    (r, t)
}

/// Radiance's transmissivity for a glass of normal transmittance `t` (its reference manual's formula).
pub fn transmissivity(t: f64) -> f64 {
    ((0.840_252_843_5 + 0.007_252_223_9 * t * t).sqrt() - 0.916_653_066_1) / 0.003_626_111_9 / t
}

/// A sky: radiance seen in direction `d` (unit vector, Z up).
pub type Sky<'a> = &'a dyn Fn(V) -> f64;

/// The CIE overcast sky as `gensky -c` writes it, with skybright.cal's blend
/// into the ground: zenith radiance `lz`, ground radiance `lg`.
pub fn cie_overcast(d: V, lz: f64, lg: f64) -> f64 {
    let a = (d[2] + 1.01).powi(10);
    let b = (d[2] + 1.01).powf(-10.0);
    (a * lz * (1.0 + 2.0 * d[2]) / 3.0 + b * lg) / (a + b)
}

fn trace(s: &Tracer, mut o: V, mut d: V, mut ab: u32, sky: Sky, r: &mut Rng) -> f64 {
    let mut w = 1.0;
    for _ in 0..64 {
        let Some((t, k)) = s.hit(o, d) else {
            return w * sky(d);
        };
        let tr = &s.tris[k];
        o = madd(o, d, t);
        let cosi = dot(d, tr.n);
        match tr.m {
            Mat::Diffuse(rho) => {
                if ab == 0 {
                    return 0.0;
                }
                ab -= 1;
                w *= rho;
                let n = if cosi < 0.0 {
                    tr.n
                } else {
                    [-tr.n[0], -tr.n[1], -tr.n[2]]
                };
                d = cos_dir(n, r);
            }
            Mat::Glass(tn) => {
                let (rf, tt) = glass_rt(tn, cosi);
                if r.f() * (rf + tt) >= tt {
                    d = madd(d, tr.n, -2.0 * cosi);
                }
                w *= rf + tt;
            }
        }
        if w < 1e-4 {
            return 0.0;
        }
    }
    0.0
}

/// Irradiance per sensor by cosine-weighted Monte Carlo (E = pi * mean radiance),
/// `paths` per sensor, `ab` >= 1 diffuse bounces. `first` is the index of
/// `sensors[0]` in the whole grid (the random sequence follows it).
pub fn irradiance(
    s: &Tracer,
    sensors: &[Sensor],
    first: usize,
    paths: u32,
    ab: u32,
    sky: Sky,
    seed: u64,
) -> Vec<f64> {
    let paths = paths.max(1);
    sensors
        .iter()
        .enumerate()
        .map(|(i, sn)| {
            let mut r = Rng::for_sensor(seed, (first + i) as u64);
            let mut sum = 0.0;
            for _ in 0..paths {
                let d = cos_dir(sn.n, &mut r);
                sum += trace(s, sn.p, d, ab.max(1) - 1, sky, &mut r);
            }
            std::f64::consts::PI * sum / f64::from(paths)
        })
        .collect()
}

/// For each sensor, how many of `suns` (unit vectors toward the sun, project
/// frame) reach it: above the sensor's plane and blocked by nothing opaque.
pub fn sun_hours(s: &Tracer, sensors: &[Sensor], suns: &[V]) -> Vec<u32> {
    sensors
        .iter()
        .map(|sn| {
            suns.iter()
                .filter(|&&d| dot(d, sn.n) > 0.0 && s.sees(sn.p, d))
                .count() as u32
        })
        .collect()
}

/// Sky view factor per sensor: the irradiance from a uniform sky of radiance
/// 1/pi above the horizon (ground black), one bounce, so 1 = open sky.
pub fn sky_view(s: &Tracer, sensors: &[Sensor], first: usize, paths: u32) -> Vec<f64> {
    let sky = |d: V| {
        if d[2] > 0.0 {
            1.0 / std::f64::consts::PI
        } else {
            0.0
        }
    };
    irradiance(s, sensors, first, paths, 1, &sky, 7)
}

/// Daylight factor (%) per sensor under the CIE overcast sky with ground
/// reflectance `ground` (gensky's default 0.2), `ab` diffuse bounces.
pub fn daylight_factor(
    s: &Tracer,
    sensors: &[Sensor],
    first: usize,
    ab: u32,
    paths: u32,
    ground: f64,
) -> Vec<f64> {
    // gensky -c -B b: zenith radiance b / (7 pi / 9), ground radiance ground * b / pi.
    let b = 55.866;
    let (lz, lg) = (
        b * 9.0 / (7.0 * std::f64::consts::PI),
        ground * b / std::f64::consts::PI,
    );
    irradiance(
        s,
        sensors,
        first,
        paths,
        ab,
        &move |d: V| cie_overcast(d, lz, lg),
        11,
    )
    .into_iter()
    .map(|e| 100.0 * e / b)
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> Model {
        Model {
            modifiers: vec![Modifier {
                id: "m".into(),
                surface: Surface::Plastic {
                    rgb: [0.5; 3],
                    spec: 0.0,
                    rough: 0.0,
                },
                label: "m".into(),
                assumed: false,
            }],
            triangles: Vec::new(),
        }
    }

    #[test]
    fn open_sky() {
        let s = Tracer::new(&model());
        let p = [Sensor {
            p: [0.0; 3],
            n: [0.0, 0.0, 1.0],
        }];
        let v = sky_view(&s, &p, 0, 20_000);
        assert!((v[0] - 1.0).abs() < 0.01, "{v:?}");
        let df = daylight_factor(&s, &p, 0, 1, 200_000, 0.2);
        assert!((df[0] - 100.0).abs() < 1.5, "{df:?}");
        let (r, t) = glass_rt(0.9432, 1.0);
        assert!((t - 0.8611).abs() < 0.01 && r > 0.07 && r < 0.09, "{r} {t}");
        assert!((transmissivity(0.75f64.sqrt()) - 0.9432).abs() < 1e-4);
    }

    #[test]
    fn roof_blocks_the_sun_and_glass_does_not() {
        let mut m = model();
        m.modifiers.push(Modifier {
            id: "g".into(),
            surface: Surface::Glass { tn: [0.9; 3] },
            label: "g".into(),
            assumed: false,
        });
        let quad = |z: f64, modifier: u32| {
            [
                Triangle {
                    v: [[-1.0, -1.0, z], [1.0, -1.0, z], [1.0, 1.0, z]],
                    modifier,
                },
                Triangle {
                    v: [[-1.0, -1.0, z], [1.0, 1.0, z], [-1.0, 1.0, z]],
                    modifier,
                },
            ]
        };
        m.triangles.extend(quad(2.0, 1));
        let p = [Sensor {
            p: [0.0; 3],
            n: [0.0, 0.0, 1.0],
        }];
        let suns = [[0.0, 0.0, 1.0], norm([1.0, 0.0, 1.0]), [0.0, 0.0, -1.0]];
        assert_eq!(sun_hours(&Tracer::new(&m), &p, &suns), [2]);
        m.triangles.extend(quad(3.0, 0));
        // the opaque roof above the glass covers the sun overhead; the 45 degree sun passes beside it
        assert_eq!(sun_hours(&Tracer::new(&m), &p, &suns), [1]);
    }
}
