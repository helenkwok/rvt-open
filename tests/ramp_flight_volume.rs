//! The body of a ramp or of a stair flight has Revit's volume.
//!
//! Real file: the BIM4LCA / Nordic LCA office with terrain,
//! `ARK_NordicLCA_Office_Terrain_Concrete_BuildingPermit.rvt` (Revit 2024;
//! VTT, Granlund, Gravicon, Insinööritoimisto Kallinen; CC BY-SA 4.0,
//! <https://cris.vtt.fi/en/publications/bim-files-to-download/>), and Revit's own
//! IFC4 export of it, from `RVT_NORDIC_LCA_DIR`. Skips when either is absent.
//!
//! The office holds six ramps, among them `Ramp:AP-ajoramppi` 1386050, a ruled
//! flight whose profile is a Hermite spline. Revit writes five of them as an
//! `IfcRamp` that aggregates an `IfcRampFlight`, both carrying the ramp's id as
//! `Tag`, so the body shows once, under the flight. A scorer that meshes the
//! parts of every element with a Tag counts that body under both and reads half
//! of rvt-open's volume (ratio 0.5000): the volume of each body below is taken
//! from the one body Revit writes, and equals rvt-open's.

use rvt::RevitFile;
use rvt::convert::{ConvertOptions, export};
use rvt::ifc::write_step;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// A STEP file by instance id: the entity's name and the text between its outer parentheses.
type Step = BTreeMap<u64, (String, String)>;

fn parse(step: &str) -> Step {
    let mut out = Step::new();
    for line in step.lines() {
        let Some((id, rest)) = line
            .trim()
            .strip_prefix('#')
            .and_then(|l| l.split_once('='))
        else {
            continue;
        };
        let (Ok(id), Some(open), Some(close)) = (id.parse(), rest.find('('), rest.rfind(')'))
        else {
            continue;
        };
        out.insert(
            id,
            (rest[..open].to_string(), rest[open + 1..close].to_string()),
        );
    }
    out
}

/// The top-level fields of an entity's arguments: commas inside quotes or parentheses do not split.
fn fields(args: &str) -> Vec<&str> {
    let (mut out, mut depth, mut quoted, mut from) = (Vec::new(), 0, false, 0);
    for (i, c) in args.char_indices() {
        match c {
            '\'' => quoted = !quoted,
            '(' if !quoted => depth += 1,
            ')' if !quoted => depth -= 1,
            ',' if !quoted && depth == 0 => {
                out.push(args[from..i].trim());
                from = i + 1;
            }
            _ => {}
        }
    }
    out.push(args[from..].trim());
    out
}

fn reference(text: &str) -> u64 {
    text.trim()
        .trim_start_matches('#')
        .parse()
        .unwrap_or_else(|_| panic!("a #id, not {text:?}"))
}

fn numbers(text: &str) -> Vec<f64> {
    text.split(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | 'E' | 'e' | '+')))
        .filter_map(|t| t.parse().ok())
        .collect()
}

/// The volume enclosed by triangles about the points' centre (the origin's part of a mesh with
/// cracks between its faces is taken away), in the points' unit cubed.
fn volume(points: &[[f64; 3]], triangles: &[[usize; 3]]) -> f64 {
    let n = points.len() as f64;
    let c = points.iter().fold([0.0; 3], |a, p| {
        [a[0] + p[0] / n, a[1] + p[1] / n, a[2] + p[2] / n]
    });
    let at = |i: usize| {
        [
            points[i][0] - c[0],
            points[i][1] - c[1],
            points[i][2] - c[2],
        ]
    };
    triangles
        .iter()
        .map(|t| {
            let (a, b, d) = (at(t[0]), at(t[1]), at(t[2]));
            (a[0] * (b[1] * d[2] - b[2] * d[1]) - a[1] * (b[0] * d[2] - b[2] * d[0])
                + a[2] * (b[0] * d[1] - b[1] * d[0]))
                / 6.0
        })
        .sum::<f64>()
        .abs()
}

fn points_of(step: &Step, list: u64) -> Vec<[f64; 3]> {
    let (name, args) = &step[&list];
    assert_eq!(name, "IFCCARTESIANPOINTLIST3D");
    numbers(args)
        .chunks(3)
        .map(|p| [p[0], p[1], p[2]])
        .collect()
}

/// The area of a profile of an extruded solid, in the square of the file's length unit.
fn profile_area(step: &Step, profile: u64, tag: &str) -> f64 {
    let (kind, args) = &step[&profile];
    let f = fields(args);
    let polygon = |p: Vec<[f64; 2]>| {
        (0..p.len())
            .map(|i| p[i][0] * p[(i + 1) % p.len()][1] - p[(i + 1) % p.len()][0] * p[i][1])
            .sum::<f64>()
            .abs()
            / 2.0
    };
    match kind.as_str() {
        // (type, name, position, x dimension, y dimension)
        "IFCRECTANGLEPROFILEDEF" => numbers(f[3])[0] * numbers(f[4])[0],
        "IFCARBITRARYCLOSEDPROFILEDEF" => {
            let (curve, args) = &step[&reference(f[2])];
            let f = fields(args);
            match curve.as_str() {
                "IFCINDEXEDPOLYCURVE" => {
                    assert_eq!(f[1], "$", "tag {tag}: a profile with arcs");
                    let (list, args) = &step[&reference(f[0])];
                    assert_eq!(list, "IFCCARTESIANPOINTLIST2D");
                    polygon(numbers(args).chunks(2).map(|p| [p[0], p[1]]).collect())
                }
                "IFCPOLYLINE" => polygon(
                    f[0].trim_matches(|c| c == '(' || c == ')')
                        .split(',')
                        .map(|r| {
                            let p = numbers(&step[&reference(r)].1);
                            [p[0], p[1]]
                        })
                        .collect(),
                ),
                other => panic!("tag {tag}: profile curve {other}"),
            }
        }
        other => panic!("tag {tag}: profile {other}"),
    }
}

/// Tag -> volume of the one `Body` of each element of the classes that has a tessellated one, in
/// cubic metres. Each body counts once: Revit's flight and the ramp that aggregates it have the
/// same Tag, and the ramp holds no body of its own.
fn body_volumes(
    step: &Step,
    classes: &[&str],
    tags: &[&str],
    metres_per_unit: f64,
) -> BTreeMap<String, f64> {
    let mut out = BTreeMap::new();
    for (name, args) in step.values() {
        if !classes.contains(&name.as_str()) {
            continue;
        }
        let f = fields(args);
        if f[6] == "$" {
            continue;
        }
        let tag = f[7].trim_matches('\'').to_string();
        if !tags.is_empty() && !tags.contains(&tag.as_str()) {
            continue;
        }
        let (_, shapes) = &step[&reference(f[6])];
        let body = fields(shapes)[2]
            .trim_matches(|c| c == '(' || c == ')')
            .split(',')
            .map(reference)
            .map(|r| &step[&r])
            .find(|(_, a)| fields(a)[1] == "'Body'")
            .expect("a Body representation");
        let item = reference(fields(&body.1)[3].trim_matches(|c| c == '(' || c == ')'));
        let (kind, a) = &step[&item];
        let f = fields(a);
        let cubic = match kind.as_str() {
            // (profile, position, direction, depth): the profile's polygon area times the depth,
            // along the profile's own normal.
            "IFCEXTRUDEDAREASOLID" => {
                let area = profile_area(step, reference(f[0]), &tag);
                let (_, direction) = &step[&reference(f[2])];
                assert_eq!(
                    numbers(direction),
                    [0.0, 0.0, 1.0],
                    "tag {tag}: a slanted extrusion"
                );
                area * numbers(f[3])[0]
            }
            "IFCTRIANGULATEDFACESET" | "IFCPOLYGONALFACESET" => {
                let points = points_of(step, reference(f[0]));
                let triangles: Vec<[usize; 3]> = if kind == "IFCTRIANGULATEDFACESET" {
                    // (coordinates, normals, closed, ((a, b, c), ...), pn index): one-based.
                    numbers(f[3])
                        .chunks(3)
                        .map(|t| [t[0] as usize - 1, t[1] as usize - 1, t[2] as usize - 1])
                        .collect()
                } else {
                    // (coordinates, closed, (#face, ...), pn index): each face fanned from its first point.
                    f[2].trim_matches(|c| c == '(' || c == ')')
                        .split(',')
                        .flat_map(|r| {
                            let (face, a) = &step[&reference(r)];
                            assert_eq!(face, "IFCINDEXEDPOLYGONALFACE");
                            let v: Vec<usize> =
                                numbers(a).iter().map(|i| *i as usize - 1).collect();
                            (1..v.len() - 1).map(move |k| [v[0], v[k], v[k + 1]])
                        })
                        .collect()
                };
                volume(&points, &triangles)
            }
            other => panic!("tag {tag}: body of kind {other}"),
        };
        let unit = metres_per_unit.powi(3);
        assert!(
            out.insert(tag.clone(), cubic * unit).is_none(),
            "tag {tag} has two bodies"
        );
    }
    out
}

/// Both exports of `model` (and Revit's `_Revit.ifc` beside it), from `RVT_NORDIC_LCA_DIR`:
/// the volume of each body of the classes by Tag, rvt-open's and Revit's, in cubic metres.
fn volumes(
    model: &str,
    classes: &[&str],
    tags: &[&str],
) -> Option<(BTreeMap<String, f64>, BTreeMap<String, f64>)> {
    let Some(dir) = std::env::var_os("RVT_NORDIC_LCA_DIR").map(PathBuf::from) else {
        eprintln!("skipping: RVT_NORDIC_LCA_DIR is not set");
        return None;
    };
    let rvt = dir.join(format!("{model}.rvt"));
    let ifc = dir.join(format!("{model}_Revit.ifc"));
    if !rvt.exists() || !ifc.exists() {
        eprintln!("skipping: {model} or Revit's export absent");
        return None;
    }
    let mut rf = RevitFile::open(&rvt).expect("open");
    // Every body offered its saved mesh, as `rvt-ifc` does.
    let options = ConvertOptions {
        saved_meshes: true,
        ..ConvertOptions::default()
    };
    let result = export(&mut rf, &options).expect("export");
    // rvt-open writes metres; Revit's export writes millimetres.
    let ours = body_volumes(&parse(&write_step(&result.model)), classes, tags, 1.0);
    let revit = body_volumes(
        &parse(&std::fs::read_to_string(&ifc).expect("read")),
        classes,
        tags,
        0.001,
    );
    Some((ours, revit))
}

fn same_volumes(ours: &BTreeMap<String, f64>, revit: &BTreeMap<String, f64>, count: usize) {
    assert_eq!(ours.len(), count, "rvt-open's bodies: {ours:?}");
    assert_eq!(
        revit.keys().collect::<Vec<_>>(),
        ours.keys().collect::<Vec<_>>()
    );
    for (tag, volume) in ours {
        let ratio = volume / revit[tag];
        eprintln!(
            "{tag}: rvt-open {volume:.6} m3, Revit {:.6} m3, ratio {ratio:.6}",
            revit[tag]
        );
        assert!((ratio - 1.0).abs() < 1e-4, "{tag}: volume ratio {ratio}");
    }
}

#[test]
fn office_terrain_ramps_have_revits_volume() {
    let Some((ours, revit)) = volumes(
        "ARK_NordicLCA_Office_Terrain_Concrete_BuildingPermit",
        &["IFCRAMP", "IFCRAMPFLIGHT"],
        &[],
    ) else {
        return;
    };
    // 1386050 is the ruled flight with a Hermite spline profile.
    assert!(ours.contains_key("1386050"));
    same_volumes(&ours, &revit, 6);
}

#[test]
fn office_timber_stair_flights_have_revits_volume() {
    // The two monolithic runs of the cast-in-place stairs, ruled like the ramp.
    let tags = ["3558983", "3561904"];
    let Some((ours, revit)) = volumes(
        "ARK_NordicLCA_Office_Timber_As-Built",
        &["IFCSTAIRFLIGHT"],
        &tags,
    ) else {
        return;
    };
    same_volumes(&ours, &revit, 2);
}
