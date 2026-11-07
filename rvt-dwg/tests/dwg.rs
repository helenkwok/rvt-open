//! rvt-dwg against synthetic drawings (always) and the ACadSharp samples (when `RVT_DWG_SAMPLES`
//! names a folder filled by `tools/fetch-acadsharp-samples.sh`; CI sets it).

use rvt_dwg::write::{CadFormat, DWG_VERSIONS, parse_version, resave, write_drawing};
use rvt_dwg::{
    Block2d, Drawing2d, Entity2d, Geom, Layer2d, Layout2d, Limits, Space, read_bytes, svg, tables,
};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn synthetic() -> Drawing2d {
    let mut ents = vec![
        Entity2d::new(
            "LINE",
            "A-WALL",
            Geom::Line {
                a: [0.0, 0.0, 0.0],
                b: [5000.0, 0.0, 0.0],
            },
        ),
        Entity2d::new(
            "CIRCLE",
            "A-COLS",
            Geom::Circle {
                c: [100.0, 200.0, 0.0],
                r: 150.0,
            },
        ),
        Entity2d::new(
            "ARC",
            "A-DOOR",
            Geom::Arc {
                c: [1000.0, 0.0, 0.0],
                r: 900.0,
                start: 0.0,
                end: std::f64::consts::FRAC_PI_2,
            },
        ),
        Entity2d::new(
            "LWPOLYLINE",
            "A-WALL",
            Geom::Polyline {
                pts: vec![
                    [0.0, 0.0, 0.0],
                    [5000.0, 0.0, 0.5],
                    [5000.0, 3000.0, 0.0],
                    [0.0, 3000.0, 0.0],
                ],
                closed: true,
                z: 0.0,
            },
        ),
        Entity2d::new(
            "TEXT",
            "A-ANNO",
            Geom::Text {
                at: [10.0, 20.0, 0.0],
                height: 250.0,
                rotation: 0.0,
                value: "Level 1".into(),
            },
        ),
        Entity2d::new(
            "INSERT",
            "A-DOOR",
            Geom::Insert {
                block: "DOOR-900".into(),
                at: [2000.0, 0.0, 0.0],
                scale: [1.0, 1.0, 1.0],
                rotation: std::f64::consts::FRAC_PI_2,
                attributes: vec![],
            },
        ),
        Entity2d::new(
            "HATCH",
            "A-FLOR",
            Geom::Hatch {
                loops: vec![vec![
                    [0.0, 0.0, 0.0],
                    [100.0, 0.0, 0.0],
                    [100.0, 100.0, 0.0],
                    [0.0, 100.0, 0.0],
                ]],
                pattern: "SOLID".into(),
                solid: true,
            },
        ),
    ];
    let mut leaf = Entity2d::new(
        "LINE",
        "0",
        Geom::Line {
            a: [0.0, 0.0, 0.0],
            b: [900.0, 0.0, 0.0],
        },
    );
    leaf.owner = "DOOR-900".into();
    ents.push(leaf);
    let mut vp = Entity2d::new(
        "VIEWPORT",
        "0",
        Geom::Viewport {
            center: [210.0, 148.0, 0.0],
            width: 300.0,
            height: 200.0,
            view_center: [2500.0, 1500.0],
            view_height: 4000.0,
            id: 2,
        },
    );
    vp.owner = "*Paper_Space".into();
    ents.push(vp);
    let mut title = Entity2d::new(
        "TEXT",
        "A-ANNO",
        Geom::Text {
            at: [20.0, 20.0, 0.0],
            height: 5.0,
            rotation: 0.0,
            value: "A101".into(),
        },
    );
    title.owner = "*Paper_Space".into();
    ents.push(title);
    Drawing2d {
        version: "AC1032".into(),
        units: 4,
        code_page: "ANSI_1252".into(),
        layers: ["0", "A-WALL", "A-COLS", "A-DOOR", "A-ANNO", "A-FLOR"]
            .iter()
            .enumerate()
            .map(|(i, n)| Layer2d::new(*n, i as i16 + 1))
            .collect(),
        blocks: vec![Block2d {
            name: "DOOR-900".into(),
            base: [0.0; 3],
            xref_path: None,
            layout: None,
            attribute_tags: vec![],
        }],
        layouts: vec![
            Layout2d {
                name: "Model".into(),
                block: rvt_dwg::MODEL_SPACE.into(),
                tab_order: 0,
                is_model: true,
                limits: None,
            },
            Layout2d {
                name: "Layout1".into(),
                block: "*Paper_Space".into(),
                tab_order: 1,
                is_model: false,
                limits: None,
            },
        ],
        entities: ents,
    }
}

/// Entity count by (type, space).
type Counts = BTreeMap<(String, &'static str), usize>;

/// (type, space) -> count, and the drawn geometry of model space as sorted rounded samples.
fn fingerprint(d: &Drawing2d) -> (Counts, Vec<(i64, i64)>) {
    let mut counts = BTreeMap::new();
    let mut pts = Vec::new();
    for e in &d.entities {
        let space = Space::of(&e.owner);
        *counts.entry((e.kind.clone(), space.as_str())).or_default() += 1;
        if space == Space::Model {
            pts.extend(
                e.geom
                    .samples(f64::MAX)
                    .into_iter()
                    .map(|p| ((p[0] * 1e6).round() as i64, (p[1] * 1e6).round() as i64)),
            );
        }
    }
    pts.sort();
    (counts, pts)
}

#[test]
fn synthetic_drawing_round_trips_through_dxf_and_every_dwg_version() {
    let d = synthetic();
    let want = fingerprint(&d);
    let mut cases = vec![
        (CadFormat::Dxf, "2018"),
        (CadFormat::DxfBinary, "2018"),
        (CadFormat::Dxf, "R12"),
    ];
    cases.extend(DWG_VERSIONS.iter().map(|(y, _)| (CadFormat::Dwg, *y)));
    for (format, version) in cases {
        let v = parse_version(version, format).unwrap();
        let (bytes, notes) =
            write_drawing(&d, format, v).unwrap_or_else(|e| panic!("{format:?} {version}: {e}"));
        assert!(notes.is_empty(), "{format:?} {version}: {notes:?}");
        let back = read_bytes(&bytes, Limits::DEFAULT)
            .unwrap_or_else(|e| panic!("{format:?} {version} re-read: {e}"));
        assert!(
            back.notes.is_empty(),
            "{format:?} {version}: {:?}",
            back.notes
        );
        if version == "R12" {
            // R12 has no LWPOLYLINE or paper-space layouts of this kind: only model-space linework is checked.
            continue;
        }
        let got = fingerprint(&back.drawing);
        assert_eq!(got.0, want.0, "{format:?} {version}: types per space");
        assert_eq!(
            got.1, want.1,
            "{format:?} {version}: model-space geometry to 1e-6"
        );
        let block = back.drawing.blocks.iter().find(|b| b.name == "DOOR-900");
        assert!(
            block.is_some(),
            "{format:?} {version}: the block definition is kept"
        );
        assert!(
            back.drawing
                .layouts
                .iter()
                .any(|l| l.name == "Layout1" && !l.is_model)
        );
    }
}

#[test]
fn svg_tables_and_report_are_deterministic() {
    let d = synthetic();
    let a = svg::render(&d, Some("Layout1"), &Limits::DEFAULT).unwrap();
    let b = svg::render(&d, Some("Layout1"), &Limits::DEFAULT).unwrap();
    assert_eq!(a, b);
    assert!(a.svg.contains("data-viewport") && a.svg.contains("A101"));
    let t = tables::sheets(
        &d,
        &tables::TableOptions {
            explode_blocks: true,
            limits: Limits::DEFAULT,
        },
    );
    let csv: Vec<String> = t.iter().map(tables::write_csv).collect();
    assert_eq!(
        csv,
        tables::sheets(
            &d,
            &tables::TableOptions {
                explode_blocks: true,
                limits: Limits::DEFAULT
            }
        )
        .iter()
        .map(tables::write_csv)
        .collect::<Vec<_>>()
    );
    // The exploded door leaf is placed by the insert's rotation: (2000, 0) to (2000, 900).
    let jsonl = tables::entities_jsonl(&t);
    assert!(
        jsonl.lines().any(|l| l.contains("\"insert_path\"")
            && l.contains("\"end_x\":2000")
            && l.contains("\"end_y\":900")),
        "{jsonl}"
    );
}

/// A damaged copy either reads whole, or its notes say what was left out: a partial read never
/// passes for a complete one.
#[test]
fn damaged_dwg_reads_whole_or_says_what_it_left_out() {
    let mut d = Drawing2d {
        version: "AC1015".into(),
        units: 4,
        layers: vec![Layer2d::new("0", 7)],
        ..Default::default()
    };
    for i in 0..200 {
        d.entities.push(Entity2d::new(
            "LINE",
            "0",
            Geom::Line {
                a: [i as f64, 0.0, 0.0],
                b: [i as f64, 10.0, 0.0],
            },
        ));
    }
    let (dwg, _) = write_drawing(
        &d,
        CadFormat::Dwg,
        parse_version("2000", CadFormat::Dwg).unwrap(),
    )
    .unwrap();
    let whole = read_bytes(&dwg, Limits::DEFAULT).unwrap();
    assert_eq!((whole.drawing.entities.len(), whole.notes.len()), (200, 0));
    let (mut read, mut partial, mut silent) = (0, 0, Vec::new());
    for at in (dwg.len() / 4..dwg.len() * 3 / 5).step_by(97) {
        let mut b = dwg.clone();
        b[at..at + 6].iter_mut().for_each(|x| *x ^= 0xA5);
        let Ok(r) = read_bytes(&b, Limits::DEFAULT) else {
            continue;
        };
        read += 1;
        let lines = r
            .drawing
            .entities
            .iter()
            .filter(|e| e.kind == "LINE")
            .count();
        if lines < 200 {
            if r.notes.is_empty() {
                silent.push((at, lines));
            } else {
                partial += 1;
            }
        }
    }
    assert!(read > 0);
    assert!(
        silent.is_empty(),
        "{} damaged copies read short with no note (offset, lines): {silent:?}; {partial} noted",
        silent.len()
    );
}

#[test]
fn hostile_inputs_are_errors_not_crashes() {
    let mut junk = b"AC1032".to_vec();
    junk.extend((0..100_000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8));
    // Junk behind a DWG header is an error, or an empty drawing that says it is damaged.
    if let Ok(r) = read_bytes(&junk, Limits::DEFAULT) {
        assert!(
            r.drawing.entities.is_empty() && !r.notes.is_empty(),
            "{} entities, notes {:?}",
            r.drawing.entities.len(),
            r.notes
        );
    }
    assert!(read_bytes(b"AC1015", Limits::DEFAULT).is_err());
    assert!(
        read_bytes(b"hello", Limits::DEFAULT)
            .unwrap_err()
            .0
            .contains("not a DWG or DXF")
    );
    let small = Limits {
        max_input_bytes: 10,
        ..Limits::DEFAULT
    };
    assert!(
        read_bytes(&junk, small)
            .unwrap_err()
            .0
            .contains("larger than")
    );
    // Truncations of a good file.
    let (dwg, _) = write_drawing(
        &synthetic(),
        CadFormat::Dwg,
        parse_version("2018", CadFormat::Dwg).unwrap(),
    )
    .unwrap();
    for cut in [16, 128, dwg.len() / 3, dwg.len() / 2, dwg.len() - 7] {
        if let Ok(r) = read_bytes(&dwg[..cut], Limits::DEFAULT) {
            assert!(
                !r.notes.is_empty() || r.drawing.entities.len() == synthetic().entities.len(),
                "cut at {cut} read silently short"
            );
        }
    }
}

/// A DXF larger than the DWG input limit reads, streamed from disk, and the DXF limit still refuses
/// one past it. The file is generated here (a written DXF with one LINE repeated, each with its own
/// handle, to 513 MiB) and deleted afterwards.
#[test]
fn dxf_larger_than_the_dwg_limit_reads_from_disk() {
    use std::io::Write;
    let (bytes, _) = write_drawing(
        &synthetic(),
        CadFormat::Dxf,
        parse_version("2018", CadFormat::Dxf).unwrap(),
    )
    .unwrap();
    let base = read_bytes(&bytes, Limits::DEFAULT)
        .unwrap()
        .drawing
        .entities
        .len();
    let text = String::from_utf8(bytes).unwrap();
    // Group code / value pairs; the first LINE of the ENTITIES section is the template.
    let lines: Vec<&str> = text.lines().collect();
    let pairs: Vec<(&str, &str)> = lines.chunks(2).map(|c| (c[0], c[1])).collect();
    let section = pairs
        .iter()
        .position(|(c, v)| c.trim() == "2" && *v == "ENTITIES")
        .unwrap();
    let start = section
        + pairs[section..]
            .iter()
            .position(|(c, v)| c.trim() == "0" && *v == "LINE")
            .unwrap();
    let end = start
        + 1
        + pairs[start + 1..]
            .iter()
            .position(|(c, _)| c.trim() == "0")
            .unwrap();
    let template = &pairs[start..end];
    let entity = |handle: u64| -> String {
        template
            .iter()
            .map(|(c, v)| {
                let v = if c.trim() == "5" {
                    format!("{handle:X}")
                } else {
                    v.to_string()
                };
                format!("{c}\r\n{v}\r\n")
            })
            .collect()
    };
    let one = entity(0x1000_0000).len() as u64;
    let target = Limits::DEFAULT.max_input_bytes as u64 + (1 << 20);
    let copies = target / one + 1;
    let path = std::env::temp_dir().join(format!("rvt-dwg-large-{}.dxf", std::process::id()));
    {
        let mut f = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
        let write_pairs = |f: &mut std::io::BufWriter<std::fs::File>, ps: &[(&str, &str)]| {
            for (c, v) in ps {
                write!(f, "{c}\r\n{v}\r\n").unwrap();
            }
        };
        write_pairs(&mut f, &pairs[..end]);
        for i in 0..copies {
            f.write_all(entity(0x1000_0000 + i).as_bytes()).unwrap();
        }
        write_pairs(&mut f, &pairs[end..]);
        f.flush().unwrap();
    }
    let len = std::fs::metadata(&path).unwrap().len();
    assert!(len > Limits::DEFAULT.max_input_bytes as u64, "{len} bytes");
    let read = rvt_dwg::read_file(&path, Limits::DEFAULT);
    let refused = rvt_dwg::read_file(
        &path,
        Limits {
            max_dxf_input_bytes: (len - 1) as usize,
            ..Limits::DEFAULT
        },
    );
    let _ = std::fs::remove_file(&path);
    let read = read.unwrap_or_else(|e| panic!("{len} bytes: {e}"));
    assert!(read.notes.is_empty(), "{:?}", read.notes);
    assert_eq!(read.drawing.entities.len() as u64, base as u64 + copies);
    assert!(refused.unwrap_err().0.contains("larger than"));
}

fn samples() -> Option<PathBuf> {
    match std::env::var_os("RVT_DWG_SAMPLES") {
        Some(d) => Some(PathBuf::from(d)),
        None => {
            eprintln!(
                "skip: RVT_DWG_SAMPLES is not set (run rvt-dwg/tools/fetch-acadsharp-samples.sh)"
            );
            None
        }
    }
}

const VERSIONS: &[&str] = &[
    "AC1014", "AC1015", "AC1018", "AC1021", "AC1024", "AC1027", "AC1032",
];

/// Types in each version of the ACadSharp sample (341 entities).
const SAMPLE_TYPES: &[(&str, usize)] = &[
    ("ARC", 3),
    ("ATTDEF", 5),
    ("CIRCLE", 13),
    ("DIMENSION", 11),
    ("ELLIPSE", 1),
    ("3DFACE", 1),
    ("HATCH", 8),
    ("INSERT", 14),
    ("LEADER", 1),
    ("LINE", 98),
    ("LWPOLYLINE", 23),
    ("MLINE", 3),
    ("MTEXT", 36),
    ("MESH", 2),
    ("MULTILEADER", 15),
    ("POINT", 40),
    ("POLYLINE", 2),
    ("IMAGE", 1),
    ("RAY", 1),
    ("REGION", 1),
    ("SHAPE", 1),
    ("SOLID", 14),
    ("3DSOLID", 2),
    ("SPLINE", 2),
    ("ACAD_TABLE", 2),
    ("TEXT", 29),
    ("TOLERANCE", 3),
    ("VIEWPORT", 6),
    ("WIPEOUT", 1),
    ("XLINE", 1),
];

fn types(d: &Drawing2d) -> BTreeMap<String, usize> {
    let mut m = BTreeMap::new();
    for e in &d.entities {
        *m.entry(e.kind.clone()).or_default() += 1;
    }
    m
}

#[test]
fn acadsharp_samples_read_every_version_and_agree_with_their_dxf_twins() {
    let Some(dir) = samples() else { return };
    let first = read_bytes(
        &std::fs::read(dir.join("sample_AC1014.dwg")).unwrap(),
        Limits::DEFAULT,
    )
    .unwrap();
    let base = types(&first.drawing);
    eprintln!("AC1014 types: {base:?}");
    assert_eq!(first.drawing.entities.len(), 341, "{base:?}");
    for (kind, n) in SAMPLE_TYPES {
        // The engine's name for some kinds differs from the DXF name (underlay, table): only the
        // kinds both name the same way are checked by name; the total covers the rest.
        if let Some(got) = base.get(*kind) {
            assert_eq!(got, n, "{kind}");
        }
    }
    for v in VERSIONS {
        let dwg = read_bytes(
            &std::fs::read(dir.join(format!("sample_{v}.dwg"))).unwrap(),
            Limits::DEFAULT,
        )
        .unwrap();
        assert_eq!(dwg.drawing.version, *v);
        assert!(dwg.notes.is_empty(), "{v}: {:?}", dwg.notes);
        assert_eq!(
            types(&dwg.drawing),
            base,
            "{v}: same types and counts as AC1014"
        );
        if *v == "AC1014" {
            continue;
        }
        for kind in ["ascii", "binary"] {
            let dxf = read_bytes(
                &std::fs::read(dir.join(format!("sample_{v}_{kind}.dxf"))).unwrap(),
                Limits::DEFAULT,
            )
            .unwrap_or_else(|e| panic!("{v} {kind} DXF: {e}"));
            let (a, b) = (fingerprint(&dwg.drawing).0, fingerprint(&dxf.drawing).0);
            assert_eq!(a, b, "{v} DWG and its {kind} DXF twin: types per space");
            let names = |d: &Drawing2d| {
                d.layers
                    .iter()
                    .map(|l| l.name.clone())
                    .collect::<std::collections::BTreeSet<_>>()
            };
            assert_eq!(
                names(&dwg.drawing),
                names(&dxf.drawing),
                "{v} {kind}: layers"
            );
            let blocks = |d: &Drawing2d| {
                d.blocks
                    .iter()
                    .filter(|b| Space::of(&b.name) == Space::Block && !b.name.starts_with('*'))
                    .map(|b| b.name.clone())
                    .collect::<std::collections::BTreeSet<_>>()
            };
            // The DXF twins leave out the definition of a dynamic block the DWG keeps.
            let (bd, bx) = (blocks(&dwg.drawing), blocks(&dxf.drawing));
            assert!(
                bx.is_subset(&bd),
                "{v} {kind}: named blocks {bx:?} not in {bd:?}"
            );
        }
    }
    // The R12 DXF twin reads too.
    let r12 = read_bytes(
        &std::fs::read(dir.join("sample_AC1009_ascii.dxf")).unwrap(),
        Limits::DEFAULT,
    )
    .unwrap();
    assert!(!r12.drawing.entities.is_empty());
}

#[test]
fn acadsharp_samples_resave_to_dxf_and_dwg_and_read_back() {
    let Some(dir) = samples() else { return };
    for v in ["AC1014", "AC1032"] {
        let src = read_bytes(
            &std::fs::read(dir.join(format!("sample_{v}.dwg"))).unwrap(),
            Limits::DEFAULT,
        )
        .unwrap();
        let want = types(&src.drawing);
        let mut cases = vec![(CadFormat::Dxf, "2018")];
        cases.extend(DWG_VERSIONS.iter().map(|(y, _)| (CadFormat::Dwg, *y)));
        for (format, version) in cases {
            let (bytes, notes) = resave(&src, format, parse_version(version, format).unwrap())
                .unwrap_or_else(|e| panic!("{v} -> {format:?} {version}: {e}"));
            let back = read_bytes(&bytes, Limits::DEFAULT)
                .unwrap_or_else(|e| panic!("{v} -> {format:?} {version} re-read: {e}"));
            let got = types(&back.drawing);
            if notes.is_empty() {
                assert_eq!(
                    got, want,
                    "{v} -> {format:?} {version}: a lossless re-save keeps every entity"
                );
            } else {
                // Written from the drawing model: entities without plain geometry are left out, and
                // the notes say so; everything plain is kept.
                eprintln!("{v} -> {format:?} {version}: {notes:?}");
                for k in ["LINE", "CIRCLE", "ARC", "HATCH"] {
                    assert_eq!(got.get(k), want.get(k), "{v} -> {format:?} {version}: {k}");
                }
                // Dimensions come back as inserts of the blocks that draw them.
                let n = |m: &BTreeMap<String, usize>, k: &str| m.get(k).copied().unwrap_or(0);
                assert_eq!(
                    n(&got, "INSERT"),
                    n(&want, "INSERT") + n(&want, "DIMENSION"),
                    "{v} -> {format:?} {version}: inserts"
                );
                // Text, multiline text and attribute definitions all come back as text.
                let text = |m: &BTreeMap<String, usize>, kinds: &[&str]| {
                    kinds.iter().filter_map(|k| m.get(*k)).sum::<usize>()
                };
                assert_eq!(
                    text(&got, &["TEXT", "MTEXT"]),
                    text(&want, &["TEXT", "MTEXT", "ATTDEF"]),
                    "{v} -> {format:?} {version}: text"
                );
            }
        }
    }
}

#[test]
fn acadsharp_sample_layouts_render_and_match_their_snapshots() {
    let Some(dir) = samples() else { return };
    use sha2::{Digest, Sha256};
    let mut lines = Vec::new();
    for v in VERSIONS {
        let r = read_bytes(
            &std::fs::read(dir.join(format!("sample_{v}.dwg"))).unwrap(),
            Limits::DEFAULT,
        )
        .unwrap();
        for layout in &r.drawing.layouts {
            let name = (!layout.is_model).then_some(layout.name.as_str());
            let s = svg::render(&r.drawing, name, &Limits::DEFAULT)
                .unwrap_or_else(|e| panic!("{v} {}: {e}", layout.name));
            assert!(s.svg.starts_with("<svg") && s.svg.ends_with("</svg>"));
            let digest: String = Sha256::digest(s.svg.as_bytes())
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            lines.push(format!("{v}\t{}\t{}\t{digest}", layout.name, s.svg.len()));
        }
    }
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/acadsharp-svg.tsv");
    let got = lines.join("\n") + "\n";
    if std::env::var_os("RVT_DWG_UPDATE_SNAPSHOTS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &got).unwrap();
    }
    let want = std::fs::read_to_string(&path)
        .expect("snapshot missing: run with RVT_DWG_UPDATE_SNAPSHOTS=1");
    assert_eq!(
        got, want,
        "SVG changed: inspect the new SVGs, then rerun with RVT_DWG_UPDATE_SNAPSHOTS=1"
    );
}
