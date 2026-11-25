//! Revit 2008 files and families, checked without a file: objects framed
//! with no id, a 2008 sweep's own profile fields, a sweep along a U-shaped
//! path, and splines drawn through their points.

use rvt::family_documents::{
    Curve, FamilySolid, RawForm, RawPlane, RawSketch, SolveCtx, hermite_at, nurbs_of,
    own_profile_fields, polyline, solve_form,
};
use rvt::formats::{SchemaField, SchemaFieldType, SchemaLayout, SchemaLayouts};
use rvt::schema_objects::{
    Fields, Frame, IdWidth, ObjectHeader, Value, frames, header_records, read_frame,
    take_element_id,
};
use std::collections::BTreeMap;

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dist(a: [f64; 3], b: [f64; 3]) -> f64 {
    let d = sub(a, b);
    dot(d, d).sqrt()
}

/// A schema with one class, tag 7, holding one u32.
fn layouts() -> SchemaLayouts {
    let mut classes = BTreeMap::new();
    classes.insert(
        7,
        SchemaLayout {
            tag: 7,
            name: "ElementHeader".into(),
            base: None,
            fields: vec![SchemaField {
                name: "m_x".into(),
                ty: SchemaFieldType::Scalar {
                    kind: 4,
                    modifier: 0,
                    count: None,
                },
                flags: 0,
            }],
        },
    );
    SchemaLayouts { classes }
}

/// Before Revit 2012 `ElementId` is tag 19 and tag 20 is `Identifier`,
/// the class it holds: an id field still reads as an id.
/// `objects` (class 7 payloads) framed as Revit 2008 writes them, with no
/// id: `Adler-32 · size · payload · size`.
fn id_less(objects: &[u32]) -> Vec<u8> {
    let mut buf = Vec::new();
    for x in objects {
        let mut payload = 7u16.to_le_bytes().to_vec();
        payload.extend(x.to_le_bytes());
        let mut sum = simd_adler32::Adler32::new();
        sum.write(&payload);
        buf.extend(sum.finish().to_le_bytes());
        buf.extend((payload.len() as u32).to_le_bytes());
        buf.extend(&payload);
        buf.extend((payload.len() as u32).to_le_bytes());
    }
    buf
}

#[test]
fn revit_2008_objects_have_no_id() {
    assert_eq!(IdWidth::of_release(2008), IdWidth::Absent);
    assert_eq!(IdWidth::of_release(2010), IdWidth::Narrow);
    assert_eq!(IdWidth::of_release(0), IdWidth::Narrow);
    let buf = id_less(&[1, 2, 3, 4, 5]);
    let (found, gaps) = frames(&buf, IdWidth::Absent);
    assert_eq!(found.len(), 5);
    assert!(gaps.is_empty(), "{gaps:?}");
    for (k, f) in found.iter().enumerate() {
        let o = read_frame(&layouts(), "P", &buf, f.clone());
        assert!(o.exact, "{:?}", o.stopped);
        assert_eq!(
            o.root().and_then(|r| r.get("m_x")),
            Some(&Value::Int(k as i64 + 1))
        );
    }
    // Read with a u32 id, each object takes the one before's trailing
    // size for its id and overlaps it: every second one is lost.
    let (narrow, _) = frames(&buf, IdWidth::Narrow);
    assert_eq!(narrow.len(), 2);
}

#[test]
fn header_records_reads_id_less_records() {
    let mut buf = Vec::new();
    for x in [3u32, 4] {
        let mut payload = 7u16.to_le_bytes().to_vec();
        payload.extend(x.to_le_bytes());
        buf.extend((payload.len() as u32).to_le_bytes());
        buf.extend(&payload);
        buf.extend((payload.len() as u32).to_le_bytes());
    }
    let got = header_records(&layouts(), "P", &buf, 0..buf.len(), IdWidth::Absent);
    assert_eq!(got.records.len(), 2);
    assert!(got.markers.is_empty() && got.rest.is_empty());
    assert_eq!(
        got.records[1].root().unwrap().get("m_x"),
        Some(&Value::Int(4))
    );
}

#[test]
fn an_id_less_object_takes_its_root_m_id() {
    let mut classes = BTreeMap::new();
    let object = |tag| SchemaFieldType::Object {
        tag,
        modifier: 0,
        count: None,
    };
    let field = |name: &str, ty| SchemaField {
        name: name.into(),
        ty,
        flags: 0,
    };
    let int = SchemaFieldType::Scalar {
        kind: 4,
        modifier: 0,
        count: None,
    };
    for (tag, name, fields) in [
        (19, "ElementId", vec![field("m_id", object(20))]),
        (20, "Identifier", vec![field("m_id", int)]),
        (8, "Elem", vec![field("m_id", object(19))]),
    ] {
        classes.insert(
            tag,
            SchemaLayout {
                tag,
                name: name.into(),
                base: None,
                fields,
            },
        );
    }
    let layouts = SchemaLayouts { classes };
    let mut buf = 8u16.to_le_bytes().to_vec();
    buf.extend(2368i32.to_le_bytes());
    for (width, want) in [(IdWidth::Absent, 2368), (IdWidth::Narrow, 5)] {
        let frame = Frame {
            header: ObjectHeader {
                id: if width == IdWidth::Absent { 0 } else { 5 },
                width,
                adler: 0,
                size: buf.len() as u32,
                tag: 8,
            },
            span: 0..buf.len(),
            payload: 0..buf.len(),
        };
        let mut o = read_frame(&layouts, "Partitions/0", &buf, frame);
        take_element_id(&mut o);
        assert_eq!(o.frame.header.id, want);
    }
}

#[test]
fn a_2008_sweep_reads_its_profile_fields_from_itself() {
    let fields = |values: Vec<(&str, Value)>| Fields {
        tag: 0,
        span: 0..0,
        spans: vec![0..0; values.len()],
        values: values
            .into_iter()
            .map(|(n, v)| (n.to_string(), v))
            .collect(),
    };
    let own = fields(vec![
        ("m_id", Value::Id(50)),
        ("m_yOffset", Value::Float(0.5)),
        ("m_pathSketchId", Value::Id(51)),
        ("m_profileSketchId", Value::Id(-1)),
        ("m_profileFamSymId", Value::Id(40)),
        ("m_profileRefPlaneId", Value::Id(52)),
        ("m_flippedX", Value::Bool(true)),
        ("m_maxSegmentAngle", Value::Float(2.5)),
    ]);
    let (mut ids, mut doubles) = (BTreeMap::new(), BTreeMap::new());
    own_profile_fields(&own, &mut ids, &mut doubles);
    assert_eq!(ids.get("m_profileFamSymId"), Some(&40));
    assert_eq!(ids.get("m_profileRefPlaneId"), Some(&52));
    assert_eq!(ids.len(), 2, "{ids:?}");
    assert_eq!(doubles.get("m_yOffset"), Some(&0.5));
    assert_eq!(doubles.get("m_flippedX"), Some(&1.0));
    assert!(!doubles.contains_key("m_maxSegmentAngle"));
    // A later sweep's fields are in `m_profileData`: its own are not read.
    let later = fields(vec![
        ("m_profileData", Value::List(Vec::new())),
        ("m_profileFamSymId", Value::Id(7)),
    ]);
    let (mut ids, mut doubles) = (BTreeMap::new(), BTreeMap::new());
    own_profile_fields(&later, &mut ids, &mut doubles);
    assert!(ids.is_empty() && doubles.is_empty());
}

fn ctx_of(
    sketches: BTreeMap<i64, RawSketch>,
    planes: BTreeMap<i64, RawPlane>,
    curves: BTreeMap<i64, Curve>,
) -> SolveCtx {
    SolveCtx {
        sketches,
        planes,
        curves,
        ..Default::default()
    }
}

fn solid_volume(s: &FamilySolid) -> f64 {
    s.triangles
        .iter()
        .map(|t| {
            let [a, b, c] = t.map(|i| s.vertices[i as usize]);
            dot(a, cross(b, c)) / 6.0
        })
        .sum()
}

fn line(a: [f64; 3], b: [f64; 3]) -> Curve {
    Curve::Line {
        origin: a,
        dir: sub(b, a),
        t: [0.0, 1.0],
    }
}

/// Sketch `sid` on plane `pid` (normal `n`) holding `lines`; curve ids
/// start at `first`.
#[allow(clippy::too_many_arguments)]
fn sketch_of(
    sid: i64,
    pid: i64,
    n: [f64; 3],
    lines: &[([f64; 3], [f64; 3])],
    first: i64,
    sketches: &mut BTreeMap<i64, RawSketch>,
    planes: &mut BTreeMap<i64, RawPlane>,
    curves: &mut BTreeMap<i64, Curve>,
) {
    let ids: Vec<i64> = (0..lines.len() as i64).map(|i| first + i).collect();
    for (id, (a, b)) in ids.iter().zip(lines) {
        curves.insert(*id, line(*a, *b));
    }
    sketches.insert(
        sid,
        RawSketch {
            plane: pid,
            curves: ids,
        },
    );
    planes.insert(
        pid,
        RawPlane {
            m: [[0.0, 0.0, n[0]], [0.0, 0.0, n[1]], [0.0, 0.0, n[2]]],
            flip_z: false,
        },
    );
}

fn ring(points: &[[f64; 3]]) -> Vec<([f64; 3], [f64; 3])> {
    (0..points.len())
        .map(|i| (points[i], points[(i + 1) % points.len()]))
        .collect()
}

fn form(kind: &'static str, ids: &[(&str, i64)], doubles: &[(&str, f64)]) -> RawForm {
    RawForm {
        guid: [0; 16],
        id: 1,
        tag: 0,
        part: 0,
        kind,
        material: -1,
        cutting: false,
        sketch_ids: ids.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        doubles: doubles.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        path3d: false,
    }
}

#[test]
fn a_u_shaped_path_is_swept_from_the_segment_beside_its_profile() {
    let (mut sk, mut pl, mut cv) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
    // The plane x = 1 crosses the path's first and last segments; the
    // profile sits on the last.
    let profile = ring(&[
        [1.0, 1.5, -0.5],
        [1.0, 2.5, -0.5],
        [1.0, 2.5, 0.5],
        [1.0, 1.5, 0.5],
    ]);
    sketch_of(
        10,
        20,
        [1.0, 0.0, 0.0],
        &profile,
        100,
        &mut sk,
        &mut pl,
        &mut cv,
    );
    let path = [
        ([0.0, 0.0, 0.0], [4.0, 0.0, 0.0]),
        ([4.0, 0.0, 0.0], [4.0, 2.0, 0.0]),
        ([4.0, 2.0, 0.0], [0.0, 2.0, 0.0]),
    ];
    sketch_of(
        11,
        21,
        [0.0, 0.0, 1.0],
        &path,
        200,
        &mut sk,
        &mut pl,
        &mut cv,
    );
    let f = form(
        "sweep",
        &[("m_profileSketchId", 10), ("m_pathSketchId", 11)],
        &[],
    );
    let (solid, bbox) = solve_form(&f, &ctx_of(sk, pl, cv)).unwrap();
    let v = solid_volume(&solid).abs();
    assert!((v - 10.0).abs() < 1e-9, "volume {v}");
    assert!((bbox[0][1] + 0.5).abs() < 1e-9 && (bbox[1][1] - 2.5).abs() < 1e-9);
    assert!((bbox[0][0]).abs() < 1e-9 && (bbox[1][0] - 4.5).abs() < 1e-9);
}

#[test]
fn splines_are_drawn_through_their_points() {
    // A quarter circle as a rational quadratic NURBS.
    let w = std::f64::consts::FRAC_1_SQRT_2;
    let quarter = Curve::Nurbs {
        degree: 2,
        points: vec![
            ([1.0, 0.0, 0.0], 1.0),
            ([1.0, 1.0, 0.0], w),
            ([0.0, 1.0, 0.0], 1.0),
        ],
        knots: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        t: [0.0, 1.0],
    };
    let (pts, closed, _) = polyline(&quarter);
    assert!(!closed && pts.len() > 4, "{}", pts.len());
    for p in &pts {
        assert!((dot(*p, *p).sqrt() - 1.0).abs() < 1e-9, "{p:?}");
    }
    assert!(dist(pts[0], [1.0, 0.0, 0.0]) < 1e-12);
    assert!(dist(*pts.last().unwrap(), [0.0, 1.0, 0.0]) < 1e-12);
    // A Hermite spline with its tangents along its chord is that line,
    // kept as its two ends.
    let straight = Curve::Hermite {
        nodes: vec![
            (0.0, [0.0, 0.0, 0.0], [2.0, 0.0, 0.0]),
            (1.0, [2.0, 0.0, 0.0], [2.0, 0.0, 0.0]),
            (2.0, [4.0, 0.0, 0.0], [2.0, 0.0, 0.0]),
        ],
        t: [0.0, 2.0],
    };
    let (pts, _, _) = polyline(&straight);
    assert_eq!(pts, vec![[0.0, 0.0, 0.0], [4.0, 0.0, 0.0]]);
    // Between nodes it follows the cubic: tangents (0, 3) and (0, -3)
    // over a unit span bulge 0.75 at the middle.
    // A short span between two long ones is sampled too.
    let spike = Curve::Hermite {
        nodes: vec![
            (0.0, [0.0, 0.0, 0.0], [1.0, 0.0, 0.0]),
            (0.001, [0.001, 1.0, 0.0], [1.0, 0.0, 0.0]),
            (0.002, [0.002, 0.0, 0.0], [1.0, 0.0, 0.0]),
            (1.0, [1.0, 0.0, 0.0], [1.0, 0.0, 0.0]),
        ],
        t: [0.0, 1.0],
    };
    let (pts, _, ext) = polyline(&spike);
    assert!(pts.iter().any(|p| (p[1] - 1.0).abs() < 1e-12), "{pts:?}");
    assert!(ext.len() >= pts.len());
    // A degree beyond the limit keeps the straight line.
    let names: BTreeMap<u16, String> = [(1, "GNurbSpline".to_string())].into();
    let nurbs = |degree: i64| Fields {
        tag: 1,
        span: 0..0,
        spans: vec![0..0; 5],
        values: vec![
            (
                "m_endParams".into(),
                Value::List(vec![Value::Float(0.0), Value::Float(1.0)]),
            ),
            ("m_Degree".into(), Value::Int(degree)),
            (
                "m_ctrlPoints".into(),
                Value::List(
                    (0..=degree)
                        .map(|i| {
                            Value::Object(Fields {
                                tag: 2,
                                span: 0..0,
                                spans: vec![0..0; 2],
                                values: vec![
                                    ("m_w".into(), Value::Float(1.0)),
                                    (
                                        "m_Point".into(),
                                        Value::List(vec![Value::Float(i as f64); 3]),
                                    ),
                                ],
                            })
                        })
                        .collect(),
                ),
            ),
            (
                "m_Knots".into(),
                Value::List(
                    (0..2 * (degree + 1))
                        .map(|i| Value::Float(if i <= degree { 0.0 } else { 1.0 }))
                        .collect(),
                ),
            ),
            ("m_Closed".into(), Value::Bool(false)),
        ],
    };
    assert!(nurbs_of(&nurbs(3), &names).is_some());
    assert!(nurbs_of(&nurbs(8), &names).is_none());
    let bulge = [
        (0.0, [0.0; 3], [0.0, 3.0, 0.0]),
        (1.0, [1.0, 0.0, 0.0], [0.0, -3.0, 0.0]),
    ];
    assert!(dist(hermite_at(&bulge, 0.5), [0.5, 0.75, 0.0]) < 1e-12);
}
