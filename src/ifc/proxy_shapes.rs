//! Shaped stand-ins for elements that would otherwise be their record's
//! bounding box: a tree is a trunk and a canopy, a person a body
//! and a head, a car a hull, a cabin and four wheels, each fitted to the
//! element's own box. They are *approximations*, labelled so in
//! `BodySource` and counted in the export notes and diagnostics: they are
//! used only where neither the saved graphics nor a family document gave a
//! real shape (releases before 2023, instances whose graphics do not decode).

use super::IfcModel;
use super::entities::{BrepTriangle, IfcEntity, PropertyValue, SolidShape};
use std::collections::BTreeMap;

/// What a stand-in shape draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Tree,
    Conifer,
    Palm,
    Person,
    Car,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Kind::Tree => "tree",
            Kind::Conifer => "conifer",
            Kind::Palm => "palm",
            Kind::Person => "person",
            Kind::Car => "car",
        }
    }
}

/// `BodySource` of a stand-in body.
pub const PROXY_BODY_SOURCE: &str = "approximate_proxy_shape";

/// The kind of thing an element's name says it is.
pub fn kind_of(name: &str) -> Option<Kind> {
    let n = name.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| n.contains(w));
    if has(&["palm"]) {
        Some(Kind::Palm)
    } else if has(&[
        "pine",
        "spruce",
        "fir ",
        "fir-",
        "conifer",
        "cedar",
        "cypress",
        "evergreen",
    ]) {
        Some(Kind::Conifer)
    } else if has(&[
        "tree", "shrub", "bush", "hedge", "planting", "plant", "canopy",
    ]) {
        Some(Kind::Tree)
    } else if has(&[
        "person",
        "people",
        "female",
        "male",
        "woman",
        "man ",
        "child",
        "pedestrian",
    ]) {
        Some(Kind::Person)
    } else if has(&[
        "beetle",
        " car",
        "car ",
        "vehicle",
        "automobile",
        "sedan",
        "truck",
        "suv",
        "bus ",
    ]) {
        Some(Kind::Car)
    } else {
        None
    }
}

#[derive(Default)]
struct Mesh {
    v: Vec<[f64; 3]>,
    t: Vec<BrepTriangle>,
}

impl Mesh {
    fn quad(&mut self, a: u32, b: u32, c: u32, d: u32) {
        self.t.push(BrepTriangle(a, b, c));
        self.t.push(BrepTriangle(a, c, d));
    }

    /// A closed solid of revolution about the z axis through (cx, cy): rings
    /// of (z, radius), bottom to top, capped where the radius is not 0.
    fn lathe(&mut self, c: [f64; 2], rx: f64, ry: f64, rings: &[(f64, f64)], seg: u32) {
        let base = self.v.len() as u32;
        let n = rings.len() as u32;
        for &(z, r) in rings {
            for k in 0..seg {
                let a = std::f64::consts::TAU * f64::from(k) / f64::from(seg);
                self.v
                    .push([c[0] + rx * r * a.cos(), c[1] + ry * r * a.sin(), z]);
            }
        }
        let at = |ring: u32, k: u32| base + ring * seg + k % seg;
        for ring in 0..n - 1 {
            for k in 0..seg {
                self.quad(
                    at(ring, k),
                    at(ring, k + 1),
                    at(ring + 1, k + 1),
                    at(ring + 1, k),
                );
            }
        }
        for (ring, top) in [(0, false), (n - 1, true)] {
            if rings[ring as usize].1 > 0.0 {
                let centre = self.v.len() as u32;
                self.v.push([c[0], c[1], rings[ring as usize].0]);
                for k in 0..seg {
                    if top {
                        self.t
                            .push(BrepTriangle(centre, at(ring, k), at(ring, k + 1)));
                    } else {
                        self.t
                            .push(BrepTriangle(centre, at(ring, k + 1), at(ring, k)));
                    }
                }
            }
        }
    }

    fn ellipsoid(&mut self, c: [f64; 3], r: [f64; 3], seg: u32, stacks: u32) {
        // Rings from the south pole to the north pole; a pole ring has
        // radius 0, so it collapses to a point of repeated vertices.
        let rings: Vec<(f64, f64)> = (0..=stacks)
            .map(|i| {
                let a = std::f64::consts::PI * f64::from(i) / f64::from(stacks);
                (c[2] - r[2] * a.cos(), a.sin().max(1e-4))
            })
            .collect();
        self.lathe([c[0], c[1]], r[0], r[1], &rings, seg);
    }

    fn boxed(&mut self, lo: [f64; 3], hi: [f64; 3]) {
        let base = self.v.len() as u32;
        for i in 0..8u32 {
            self.v.push([
                if i & 1 == 0 { lo[0] } else { hi[0] },
                if i & 2 == 0 { lo[1] } else { hi[1] },
                if i & 4 == 0 { lo[2] } else { hi[2] },
            ]);
        }
        for q in [
            [0, 2, 3, 1],
            [4, 5, 7, 6],
            [0, 1, 5, 4],
            [2, 6, 7, 3],
            [0, 4, 6, 2],
            [1, 3, 7, 5],
        ] {
            self.quad(base + q[0], base + q[1], base + q[2], base + q[3]);
        }
    }

    /// A cylinder along y from y0 to y1 (a wheel), centre x, z.
    fn wheel(&mut self, x: f64, z: f64, r: f64, y0: f64, y1: f64) {
        let seg = 12u32;
        let base = self.v.len() as u32;
        for y in [y0, y1] {
            for k in 0..seg {
                let a = std::f64::consts::TAU * f64::from(k) / f64::from(seg);
                self.v.push([x + r * a.cos(), y, z + r * a.sin()]);
            }
        }
        for k in 0..seg {
            let (k1, k2) = (k, (k + 1) % seg);
            self.quad(base + k1, base + seg + k1, base + seg + k2, base + k2);
        }
        for (off, rev) in [(0, true), (seg, false)] {
            let centre = self.v.len() as u32;
            self.v.push([x, if rev { y0 } else { y1 }, z]);
            for k in 0..seg {
                let (a, b) = (base + off + k, base + off + (k + 1) % seg);
                if rev {
                    self.t.push(BrepTriangle(centre, a, b));
                } else {
                    self.t.push(BrepTriangle(centre, b, a));
                }
            }
        }
    }
}

/// The stand-in for `kind` in a box of `w` x `d` x `h` feet, centred on the
/// origin in plan and standing on z = 0 (the record box's own frame).
fn shape(kind: Kind, w: f64, d: f64, h: f64) -> Mesh {
    let mut m = Mesh::default();
    let (rx, ry) = (w / 2.0, d / 2.0);
    match kind {
        Kind::Tree => {
            let trunk = 0.04 * w.min(d).max(h * 0.1);
            m.lathe([0.0, 0.0], trunk, trunk, &[(0.0, 1.0), (h * 0.45, 1.0)], 8);
            m.ellipsoid([0.0, 0.0, h * 0.65], [rx, ry, h * 0.35], 16, 10);
        }
        Kind::Conifer => {
            let trunk = 0.03 * w.min(d).max(h * 0.05);
            m.lathe([0.0, 0.0], trunk, trunk, &[(0.0, 1.0), (h * 0.2, 1.0)], 8);
            m.lathe([0.0, 0.0], rx, ry, &[(h * 0.1, 1.0), (h, 0.0)], 16);
        }
        Kind::Palm => {
            let trunk = 0.05 * w.min(d).max(h * 0.04);
            m.lathe([0.0, 0.0], trunk, trunk, &[(0.0, 1.0), (h * 0.8, 0.7)], 8);
            m.ellipsoid([0.0, 0.0, h * 0.88], [rx, ry, h * 0.12], 16, 6);
        }
        Kind::Person => {
            let r = w.min(d) / 2.0;
            m.lathe(
                [0.0, 0.0],
                r * 0.8,
                r * 0.8,
                &[(0.0, 0.6), (h * 0.45, 0.8)],
                10,
            );
            m.lathe(
                [0.0, 0.0],
                r,
                r * 0.8,
                &[(h * 0.45, 1.0), (h * 0.8, 0.9)],
                10,
            );
            m.ellipsoid([0.0, 0.0, h * 0.9], [r * 0.5, r * 0.5, h * 0.1], 10, 6);
        }
        Kind::Car => {
            // Length along the longer plan side (x after the swap below).
            let (l, b) = if w >= d { (w, d) } else { (d, w) };
            let (hl, hb) = (l / 2.0, b / 2.0);
            m.boxed([-hl, -hb, h * 0.2], [hl, hb, h * 0.55]);
            m.boxed([-hl * 0.5, -hb * 0.9, h * 0.55], [hl * 0.4, hb * 0.9, h]);
            let r = (h * 0.17).min(l * 0.1);
            for x in [-hl * 0.6, hl * 0.6] {
                m.wheel(x, r, r, -hb, -hb * 0.8);
                m.wheel(x, r, r, hb * 0.8, hb);
            }
            if w < d {
                for p in &mut m.v {
                    *p = [p[1], p[0], p[2]];
                }
                for t in &mut m.t {
                    *t = BrepTriangle(t.0, t.2, t.1);
                }
            }
        }
    }
    m
}

/// What [`attach`] did, by kind.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ProxyReport {
    /// Elements still drawn as their record's box that a proxy shape replaced.
    pub replaced: BTreeMap<String, usize>,
    /// Elements still drawn as their record's box that no shape fits.
    pub still_boxes: usize,
}

/// Replace each element that is still its record's bounding box and whose
/// name says tree, person or car with a shaped stand-in of its box size.
pub fn attach(model: &mut IfcModel) -> ProxyReport {
    let mut report = ProxyReport::default();
    for entity in &mut model.entities {
        let IfcEntity::BuildingElement {
            name,
            extrusion: Some(ex),
            solid_shape,
            property_set: Some(set),
            ..
        } = entity
        else {
            continue;
        };
        let is_box = set.properties.iter().any(|p| {
            p.name == "BodySource"
                && matches!(&p.value, PropertyValue::Text(t) if super::export_content::is_plan_box_source(t))
        }) && !set.properties.iter().any(|p| {
            p.name == "ProfileResolved" && matches!(p.value, PropertyValue::Boolean(true))
        });
        if !is_box || ex.profile_override.is_some() {
            continue;
        }
        let Some(kind) = kind_of(name) else {
            report.still_boxes += 1;
            continue;
        };
        let (w, d, h) = (ex.width_feet, ex.depth_feet, ex.height_feet);
        if !(w > 0.0 && d > 0.0 && h > 0.0 && w.is_finite() && d.is_finite() && h.is_finite()) {
            continue;
        }
        let m = shape(kind, w, d, h);
        *solid_shape = Some(SolidShape::TriangulatedFaceSet {
            vertices_feet: m.v,
            triangles: m.t,
            face_materials: Vec::new(),
        });
        for p in &mut set.properties {
            if p.name == "BodySource" {
                p.value = PropertyValue::Text(PROXY_BODY_SOURCE.into());
            }
        }
        set.properties.push(super::entities::Property {
            name: "ApproximateShape".into(),
            value: PropertyValue::Text(kind.label().into()),
        });
        *report.replaced.entry(kind.label().into()).or_default() += 1;
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn volume(m: &Mesh) -> f64 {
        m.t.iter()
            .map(|t| {
                let [a, b, c] = [t.0, t.1, t.2].map(|i| m.v[i as usize]);
                (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                    + a[2] * (b[0] * c[1] - b[1] * c[0]))
                    / 6.0
            })
            .sum()
    }

    #[test]
    fn every_shape_is_a_positive_volume_inside_its_box() {
        for kind in [
            Kind::Tree,
            Kind::Conifer,
            Kind::Palm,
            Kind::Person,
            Kind::Car,
        ] {
            let m = shape(kind, 20.0, 18.0, 30.0);
            assert!(m.t.len() > 12, "{kind:?} is no more than a box");
            assert!(volume(&m) > 1.0, "{kind:?} volume {}", volume(&m));
            for p in &m.v {
                assert!(p[0].abs() <= 10.0 + 1e-9 && p[1].abs() <= 10.0 + 1e-9);
                assert!((-1e-9..=30.0 + 1e-9).contains(&p[2]), "{kind:?} z {}", p[2]);
            }
        }
    }

    #[test]
    fn names_pick_the_kind() {
        assert_eq!(
            kind_of("RPC Tree - Deciduous:Scarlet Oak - 42':946835"),
            Some(Kind::Tree)
        );
        assert_eq!(kind_of("RPC Female:YinYin:954641"), Some(Kind::Person));
        assert_eq!(kind_of("M_RPC Beetle:M_RPC Beetle:950367"), Some(Kind::Car));
        assert_eq!(kind_of("Basic Wall:Generic - 8\":1"), None);
    }
}
