//! Families cut their voids from the solids the voids are joined to, a door
//! family draws its template's sample host wall with the door's opening,
//! deleted forms an older partition still keeps are left out, and a nested
//! instance takes the size of its own stored box.
//!
//! Runs against published course datasets of families
//! (`RVT_FAMILY_SAMPLES_DIR`, searched for each file name below), when
//! present. Skips what is absent.

use rvt::RevitFile;
use rvt::family_documents::{FamilySolid, HOST_DOCUMENT, read_with_host};
use rvt::ifc::RvtDocExporter;
use std::path::{Path, PathBuf};

fn find(dir: &Path, name: &str) -> Option<PathBuf> {
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let p = entry.path();
        if p.is_dir() {
            if let Some(f) = find(&p, name) {
                return Some(f);
            }
        } else if p.file_name().is_some_and(|n| n == name) {
            return Some(p);
        }
    }
    None
}

fn volume(s: &FamilySolid) -> f64 {
    s.triangles
        .iter()
        .map(|t| {
            let [a, b, c] = t.map(|i| s.vertices[i as usize]);
            a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                + a[2] * (b[0] * c[1] - b[1] * c[0])
        })
        .sum::<f64>()
        / 6.0
}

fn open(name: &str) -> Option<RevitFile> {
    let Some(dir) = std::env::var_os("RVT_FAMILY_SAMPLES_DIR").map(PathBuf::from) else {
        eprintln!("skipping: RVT_FAMILY_SAMPLES_DIR is not set");
        return None;
    };
    let Some(path) = find(&dir, name) else {
        eprintln!("skipping: {name} is not under RVT_FAMILY_SAMPLES_DIR");
        return None;
    };
    Some(RevitFile::open(&path).expect("open"))
}

/// (file, voids cut from solids, as measured).
const VOIDS: [(&str, usize); 2] = [("Base Shelf.rfa", 4), ("104 Shelf_B.rfa", 1)];

#[test]
fn voids_are_cut_from_the_solids_they_join() {
    for (name, cuts) in VOIDS {
        let Some(mut rf) = open(name) else { continue };
        let d = read_with_host(&mut rf, true).expect("read");
        assert_eq!(d.stats.void_cuts, cuts, "{name}: solids cut");
        // A file of 2020 or later keeps its forms in its own keyed document.
        let cut: Vec<_> = d
            .docs
            .values()
            .filter(|doc| doc.undecoded.contains_key("void_cut"))
            .collect();
        assert!(!cut.is_empty(), "{name}: no document reads a cut void");
        for doc in cut {
            for s in &doc.solids {
                assert!(volume(s) > 0.0, "{name}: form {} is inside out", s.form_id);
            }
        }
    }
}

#[test]
fn a_door_family_draws_its_sample_wall_with_the_opening() {
    let Some(mut rf) = open("802 Door w Variable Swing.rfa") else {
        return;
    };
    let d = read_with_host(&mut rf, true).expect("read");
    let walls = &d.docs[&HOST_DOCUMENT].sample_walls;
    assert_eq!(walls.len(), 1);
    let w = &walls[0];
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for p in &w.vertices {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let boxed: f64 = (0..3).map(|k| hi[k] - lo[k]).product();
    let v = volume(w);
    // The opening goes through: the wall is less than its box, and solid.
    assert!(v > 0.0 && v < boxed - 1e-6, "wall {v} of box {boxed}");
    // A line across the wall's thickness low at its middle meets no face (the
    // door's opening), and one near its top meets two (solid wall).
    let across = (0..3)
        .min_by(|a, b| (hi[*a] - lo[*a]).total_cmp(&(hi[*b] - lo[*b])))
        .expect("axis");
    let along = if across == 0 { 1 } else { 0 };
    let mid = (lo[along] + hi[along]) / 2.0 + 1e-3;
    assert_eq!(crossings(w, along, mid, lo[2] + 1.003), 0, "opening");
    assert_eq!(crossings(w, along, mid, hi[2] - 0.497), 2, "solid wall");
}

/// Points where a line across the wall through (`u` on axis `along`, height
/// `z`) meets its faces: a hit on an edge two triangles share (or a corner)
/// is one point, counted once, so the count does not depend on how the
/// faces are triangulated.
fn crossings(s: &FamilySolid, along: usize, u: f64, z: f64) -> usize {
    let across = 1 - along;
    let mut depths: Vec<f64> = s
        .triangles
        .iter()
        .filter_map(|t| {
            let v = t.map(|i| s.vertices[i as usize]);
            let p = v.map(|v| (v[along], v[2]));
            let side =
                |a: (f64, f64), b: (f64, f64)| (b.0 - a.0) * (z - a.1) - (b.1 - a.1) * (u - a.0);
            let d = [side(p[1], p[2]), side(p[2], p[0]), side(p[0], p[1])];
            let area: f64 = d.iter().sum();
            let inside =
                (d.iter().all(|x| *x >= 0.0) || d.iter().all(|x| *x <= 0.0)) && area.abs() > 1e-12;
            // Where on the line: the barycentric mix of the corners' depths.
            inside.then(|| (0..3).map(|i| d[i] / area * v[i][across]).sum())
        })
        .collect();
    depths.sort_by(f64::total_cmp);
    depths.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    depths.len()
}

fn family_summary(name: &str) -> Option<rvt::ifc::family_file::FamilySummary> {
    let mut rf = open(name)?;
    let result = RvtDocExporter
        .export_with_diagnostics(&mut rf)
        .expect("export");
    Some(result.diagnostics.input.family.expect("a family summary"))
}

/// (file, deleted forms left out, forms drawn), as measured: each left-out
/// form is in neither the element table nor the newest partition, and Revit's
/// saved display of the family holds none of them.
const DELETED: [(&str, usize, usize); 2] = [
    ("F05_Base Molding.rfa", 3, 3),
    ("Flag Pole w Flag.rfa", 4, 0),
];

#[test]
fn deleted_forms_an_older_partition_keeps_are_left_out() {
    for (name, deleted, drawn) in DELETED {
        let Some(f) = family_summary(name) else {
            continue;
        };
        assert_eq!(f.forms_deleted, deleted, "{name}: deleted forms");
        assert_eq!(
            f.forms_drawn.values().sum::<usize>(),
            drawn,
            "{name}: forms drawn"
        );
    }
}

#[test]
fn nested_instances_take_their_stored_size() {
    // The arch's four column capitals and bases: their family's document draws
    // them 2.27 ft deep, their instances' stored boxes 0.77 ft.
    let Some(f) = family_summary("F04_Three Center Arch_Complete.rfa") else {
        return;
    };
    assert_eq!((f.nested_drawn, f.nested_scaled), (4, 4));
}
