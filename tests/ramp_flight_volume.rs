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
//!
//! The curved faces of those bodies meet their neighbours: no directed edge of a
//! body's triangles lacks the reverse edge (points to 1e-5 ft), as in Revit's,
//! and Revit's vertices lie within 0.002 ft of the bodies' surfaces (the ramp
//! flight's the other way too), where the faces drawn on the curve and the planes
//! drawn between its saved points parted the ramp flight and Revit's by 0.013 ft.

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

/// A mesh: its points (metres) and its triangles by their indices.
type MeshData = (Vec<[f64; 3]>, Vec<[usize; 3]>);

/// A body: its volume in cubic metres and, where it is a mesh, its points (metres) and triangles.
struct Body {
    volume: f64,
    mesh: Option<MeshData>,
}

/// Tag -> the one `Body` of each element of the classes that has a tessellated one, in
/// cubic metres. Each body counts once: Revit's flight and the ramp that aggregates it have the
/// same Tag, and the ramp holds no body of its own.
fn body_volumes(
    step: &Step,
    classes: &[&str],
    tags: &[&str],
    metres_per_unit: f64,
) -> BTreeMap<String, Body> {
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
        let mut mesh = None;
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
                let cubic = volume(&points, &triangles);
                mesh = Some((points, triangles));
                cubic
            }
            other => panic!("tag {tag}: body of kind {other}"),
        };
        let unit = metres_per_unit.powi(3);
        let mesh = mesh.map(|(points, triangles)| {
            let metres = points
                .iter()
                .map(|p| p.map(|x| x * metres_per_unit))
                .collect();
            (metres, triangles)
        });
        assert!(
            out.insert(
                tag.clone(),
                Body {
                    volume: cubic * unit,
                    mesh
                }
            )
            .is_none(),
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
) -> Option<(BTreeMap<String, Body>, BTreeMap<String, Body>)> {
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

/// Directed edges of the triangles that no triangle has the reverse of, points within 1e-5 ft
/// (3.048e-6 m) of each other taken as one point, as the body scorer counts them (it rounds
/// the points to 1e-5 ft; here two points that straddle a rounding step are still one).
fn unmatched_edges(points: &[[f64; 3]], triangles: &[[usize; 3]]) -> usize {
    const TOL: f64 = 0.3048e-5;
    let cell = |p: [f64; 3]| p.map(|x| (x / TOL).floor() as i64);
    let mut grid: std::collections::HashMap<[i64; 3], Vec<usize>> =
        std::collections::HashMap::new();
    for (i, p) in points.iter().enumerate() {
        grid.entry(cell(*p)).or_default().push(i);
    }
    // each point is the lowest index of the points within TOL of it, and of theirs in turn
    let mut root: Vec<usize> = (0..points.len()).collect();
    fn find(root: &mut [usize], mut i: usize) -> usize {
        while root[i] != i {
            root[i] = root[root[i]];
            i = root[i];
        }
        i
    }
    for (i, p) in points.iter().enumerate() {
        let c = cell(*p);
        for d in 0..27 {
            let k = [c[0] + d / 9 - 1, c[1] + d / 3 % 3 - 1, c[2] + d % 3 - 1];
            for &j in grid.get(&k).into_iter().flatten() {
                let q = points[j];
                if j != i && dot(sub(*p, q), sub(*p, q)).sqrt() < TOL {
                    let (a, b) = (find(&mut root, i), find(&mut root, j));
                    root[a.max(b)] = a.min(b);
                }
            }
        }
    }
    let mut edges = std::collections::HashSet::new();
    for t in triangles {
        for k in 0..3 {
            let (a, b) = (find(&mut root, t[k]), find(&mut root, t[(k + 1) % 3]));
            if a != b {
                edges.insert((a, b));
            }
        }
    }
    edges
        .iter()
        .filter(|(a, b)| !edges.contains(&(*b, *a)))
        .count()
}

type Mesh<'a> = &'a MeshData;

/// The volume's centroid of a closed mesh, metres.
fn centroid(mesh: Mesh) -> [f64; 3] {
    let (points, triangles) = mesh;
    let n = points.len() as f64;
    let c = points.iter().fold([0.0; 3], |a, p| {
        [a[0] + p[0] / n, a[1] + p[1] / n, a[2] + p[2] / n]
    });
    let (mut total, mut moment) = (0.0, [0.0; 3]);
    for t in triangles {
        let [a, b, d] = t.map(|i| {
            [
                points[i][0] - c[0],
                points[i][1] - c[1],
                points[i][2] - c[2],
            ]
        });
        let v = (a[0] * (b[1] * d[2] - b[2] * d[1]) - a[1] * (b[0] * d[2] - b[2] * d[0])
            + a[2] * (b[0] * d[1] - b[1] * d[0]))
            / 6.0;
        total += v;
        for (k, m) in moment.iter_mut().enumerate() {
            *m += v * (a[k] + b[k] + d[k]) / 4.0;
        }
    }
    std::array::from_fn(|k| c[k] + moment[k] / total)
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// The distance from `p` to the triangle `a b c`.
fn point_triangle(p: [f64; 3], a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> f64 {
    let segment = |u: [f64; 3], v: [f64; 3]| {
        let d = sub(v, u);
        let t = (dot(sub(p, u), d) / dot(d, d).max(1e-30)).clamp(0.0, 1.0);
        let q = [u[0] + t * d[0], u[1] + t * d[1], u[2] + t * d[2]];
        dot(sub(p, q), sub(p, q)).sqrt()
    };
    let (ab, ac) = (sub(b, a), sub(c, a));
    let n = [
        ab[1] * ac[2] - ab[2] * ac[1],
        ab[2] * ac[0] - ab[0] * ac[2],
        ab[0] * ac[1] - ab[1] * ac[0],
    ];
    let edges = segment(a, b).min(segment(b, c)).min(segment(c, a));
    let len2 = dot(n, n);
    if len2 < 1e-30 {
        return edges;
    }
    let ap = sub(p, a);
    let h = dot(ap, n) / len2;
    let q = sub(ap, [h * n[0], h * n[1], h * n[2]]);
    let (d00, d01, d11) = (dot(ab, ab), dot(ab, ac), dot(ac, ac));
    let (d20, d21) = (dot(q, ab), dot(q, ac));
    let denom = d00 * d11 - d01 * d01;
    let v = (d11 * d20 - d01 * d21) / denom;
    let w = (d00 * d21 - d01 * d20) / denom;
    if v >= 0.0 && w >= 0.0 && v + w <= 1.0 {
        edges.min(h.abs() * len2.sqrt())
    } else {
        edges
    }
}

/// The farthest any of `from`'s points lies from `to`'s surface, metres.
fn surface_gap(from: Mesh, to: Mesh) -> f64 {
    from.0
        .iter()
        .map(|p| {
            to.1.iter()
                .map(|t| point_triangle(*p, to.0[t[0]], to.0[t[1]], to.0[t[2]]))
                .fold(f64::INFINITY, f64::min)
        })
        .fold(0.0, f64::max)
}

/// Each body holds Revit's volume and, drawn as a mesh, meets itself: the volume within 0.01%,
/// no directed edge without its reverse (where Revit's own bodies have none; points within
/// 1e-5 ft are one), and the vertices
/// of each within 0.002 ft of the other's surface once both are put at one centroid (the two
/// files place the body in different frames, with the same turn).
fn same_bodies(
    ours: &BTreeMap<String, Body>,
    revit: &BTreeMap<String, Body>,
    count: usize,
    meshed: &[&str],
    both_ways: &[&str],
) {
    assert_eq!(ours.len(), count, "rvt-open's bodies: {:?}", ours.keys());
    assert_eq!(
        revit.keys().collect::<Vec<_>>(),
        ours.keys().collect::<Vec<_>>()
    );
    for (tag, body) in ours {
        let ratio = body.volume / revit[tag].volume;
        eprintln!(
            "{tag}: rvt-open {:.6} m3, Revit {:.6} m3, ratio {ratio:.6}",
            body.volume, revit[tag].volume
        );
        assert!((ratio - 1.0).abs() < 1e-4, "{tag}: volume ratio {ratio}");
        if meshed.contains(&tag.as_str()) {
            assert!(
                body.mesh.is_some() && revit[tag].mesh.is_some(),
                "{tag}: both bodies are meshes"
            );
        }
        let Some(mesh) = &body.mesh else { continue };
        let open = unmatched_edges(&mesh.0, &mesh.1);
        eprintln!(
            "{tag}: {open} directed edges of {} triangles without their reverse",
            mesh.1.len()
        );
        assert_eq!(open, 0, "{tag}: edges without their reverse");
        let Some(reference) = &revit[tag].mesh else {
            continue;
        };
        let (c, r) = (centroid(mesh), centroid(reference));
        let moved = (
            reference
                .0
                .iter()
                .map(|p| [p[0] + c[0] - r[0], p[1] + c[1] - r[1], p[2] + c[2] - r[2]])
                .collect::<Vec<_>>(),
            reference.1.clone(),
        );
        let (to_ours, to_revit) = (surface_gap(&moved, mesh), surface_gap(mesh, &moved));
        eprintln!(
            "{tag}: Revit's vertices {:.5} ft from rvt-open's surface, rvt-open's {:.5} ft from Revit's",
            to_ours / 0.3048,
            to_revit / 0.3048
        );
        assert!(
            to_ours / 0.3048 < 0.002,
            "{tag}: Revit's vertices {to_ours} m from ours"
        );
        // Revit's export draws the timber stair runs' curved faces in far fewer triangles
        // than rvt-open (610 against 10,904), so rvt-open's vertices on the true surface
        // stand 0.0095 ft off its facets: the other way holds only where Revit's body is as fine.
        if both_ways.contains(&tag.as_str()) {
            assert!(
                to_revit / 0.3048 < 0.002,
                "{tag}: ours {to_revit} m from Revit's"
            );
        }
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
    same_bodies(&ours, &revit, 6, &["1386050"], &["1386050"]);
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
    same_bodies(&ours, &revit, 2, &tags, &[]);
}
