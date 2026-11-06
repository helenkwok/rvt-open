//! every object of a partition read through the file's own
//! schema.
//!
//! Runs against `RVT_AUTODESK_SAMPLES_DIR` (Autodesk's sample projects by
//! release, one folder per year). Skips what is absent. The answer key for
//! `rac_basic_sample_project`, Revit 2024): wall 599841's Unconnected Height
//! (BuiltInParameter -1001105) is 3400 mm and its Base Offset (-1001108)
//! -1500 mm.

use rvt::RevitFile;
use rvt::schema_objects::{ParamValue, read_elements, read_partitions};
use std::path::PathBuf;

fn sample(release: u32, name: &str) -> Option<PathBuf> {
    let root = PathBuf::from(std::env::var_os("RVT_AUTODESK_SAMPLES_DIR")?);
    let path = root.join(release.to_string()).join(format!("{name}.rvt"));
    if path.exists() {
        Some(path)
    } else {
        eprintln!("skipping: {} absent", path.display());
        None
    }
}

const FOOT: f64 = 0.3048;

#[test]
fn rac_basic_2024_every_object_reads_to_its_end() {
    let Some(path) = sample(2024, "rac_basic_sample_project") else {
        return;
    };
    let mut rf = RevitFile::open(&path).expect("open");
    let parts = read_partitions(&mut rf).expect("read");
    let objects: Vec<_> = parts.iter().flat_map(|p| &p.objects).collect();
    let exact = objects.iter().filter(|o| o.exact).count();
    assert!(objects.len() > 170_000, "{} objects", objects.len());
    assert_eq!(exact, objects.len(), "objects read to their end");
    // Spans tile each object: root first, every object inside the payload.
    for o in &objects {
        let root = o.root().expect("root");
        assert_eq!(root.span.start, o.frame.payload.start + 2);
        assert_eq!(
            o.objects.last().expect("last").span.end,
            o.frame.payload.end
        );
        assert!(o.unparsed.is_none());
    }
}

#[test]
fn rac_basic_2024_wall_599841_heights() {
    let Some(path) = sample(2024, "rac_basic_sample_project") else {
        return;
    };
    let mut rf = RevitFile::open(&path).expect("open");
    let elements = read_elements(&mut rf).expect("read");
    let wall = elements.get(&599_841).expect("wall 599841");
    assert_eq!(wall.class, "SWall");
    let feet = |p: i64| match wall.parameters.get(&p) {
        Some(ParamValue::Double(v)) => *v,
        other => panic!("{p}: {other:?}"),
    };
    assert!((feet(-1_001_105) * FOOT * 1000.0 - 3400.0).abs() < 1e-6);
    assert!((feet(-1_001_108) * FOOT * 1000.0 + 1500.0).abs() < 1e-6);
    let id = |name: &str| match wall.fields.get(name) {
        Some(rvt::schema_objects::Value::Id(v)) => *v,
        other => panic!("{name}: {other:?}"),
    };
    assert_eq!(id("m_assocLevelId"), 245_423, "Level 2");
    assert_eq!(id("m_WallAttributesId"), 711_906, "wall type");
    assert_eq!(id("m_upToLevelId"), -1, "unconnected");
}

/// Every release from 2016 reads its data objects by its own schema: almost
/// all to their end, and every element root with its own id.
fn rac_basic_every_release_reads_by_its_schema(k: usize, n: usize) {
    for release in crate::shard(2016..=2027, k, n) {
        let Some(path) = sample(release, "rac_basic_sample_project") else {
            continue;
        };
        let mut rf = RevitFile::open(&path).expect("open");
        let objects: Vec<_> = read_partitions(&mut rf)
            .expect("read")
            .into_iter()
            .flat_map(|p| p.objects)
            .collect();
        let exact = objects.iter().filter(|o| o.exact).count();
        let elements = objects.iter().filter(|o| o.element().is_some()).count();
        assert!(
            objects.len() > 90_000,
            "{release}: {} objects",
            objects.len()
        );
        assert!(
            exact * 100 >= objects.len() * 98,
            "{release}: {exact} of {} exact",
            objects.len()
        );
        assert!(elements > 75_000, "{release}: {elements} element roots");
    }
}

split!(rac_basic_every_release_reads_by_its_schema, rac_basic_every_release_reads_by_its_schema, s0 0, s1 1, s2 2, s3 3, s4 4, s5 5, s6 6, s7 7, s8 8, s9 9, s10 10, s11 11);

#[test]
fn schema_first_iteration_keeps_production_elements_as_cross_checks() {
    let Some(path) = sample(2024, "rac_basic_sample_project") else {
        return;
    };
    let mut rf = RevitFile::open(&path).expect("open");
    let all = rvt::walker::iter_elements_schema_first(&mut rf).expect("iterate");
    let wall = all
        .iter()
        .find(|e| e.id == Some(599_841))
        .expect("wall 599841");
    assert_eq!(wall.class, "SWall");
    assert_eq!(wall.provenance.decoder.as_deref(), Some("schema_objects"));
    assert!(wall.fields.iter().any(|(n, _)| n == "param:-1001105"));
    let ids: std::collections::BTreeSet<_> = all.iter().filter_map(|e| e.id).collect();
    assert_eq!(
        ids.len(),
        all.iter().filter(|e| e.id.is_some()).count(),
        "one row per id"
    );
}

/// A stored value Revit recalculates is reported, marked.
/// On rac_basic 2024, 51 walls with a top constraint store an Unconnected
/// Height (the 48 of them the API lists all differ from the API, which
/// reports the constrained height) and are marked; the 13 unconnected walls
/// are not (the 8 the API lists all equal it).
#[test]
fn recalculated_wall_heights_are_marked_not_dropped() {
    use rvt::schema_objects::{RECALCULATED_NOTE, Value, WALL_UNCONNECTED_HEIGHT};
    let Some(path) = sample(2024, "rac_basic_sample_project") else {
        return;
    };
    let mut rf = RevitFile::open(&path).expect("open");
    let elements = read_elements(&mut rf).expect("read");
    let (mut marked, mut unmarked) = (0, 0);
    for e in elements.values().filter(|e| e.class == "SWall") {
        if !e.parameters.contains_key(&WALL_UNCONNECTED_HEIGHT) {
            continue;
        }
        let constrained = matches!(e.fields.get("m_upToLevelId"), Some(Value::Id(id)) if *id != -1);
        assert_eq!(
            e.recalculated.contains(&WALL_UNCONNECTED_HEIGHT),
            constrained,
            "wall {}",
            e.id
        );
        if constrained {
            marked += 1;
        } else {
            unmarked += 1;
        }
    }
    assert_eq!((marked, unmarked), (51, 13));
    let all = rvt::walker::iter_elements_schema_first(&mut rf).expect("iterate");
    let notes = all
        .iter()
        .flat_map(|e| &e.fields)
        .filter(|(n, v)| {
            n == "param:-1001105:note"
                && matches!(v, rvt::walker::InstanceField::String(s) if s == RECALCULATED_NOTE)
        })
        .count();
    assert_eq!(notes, 51);
}
