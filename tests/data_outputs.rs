//! The data outputs, end to end through the built CLIs: the element table
//! (`rvt-schedule --format jsonl|xlsx|parquet|sqlite`), the IFC options of `rvt-ifc`
//! (`--ifc-profile`, `--ifc-units`, `--ifc-origin`, `--ifc-categories`,
//! `--phase`), the OBJ export (`rvt-gltf --format obj`), the run report
//! (`rvt-ifc --report`) and `rvt-batch` with its output manifest.
//!
//! The tier-1 synthetic fixtures always run; Autodesk's public Revit 2024
//! sample project (`RVT_REVIT2024_SAMPLES_DIR`, fetched by CI) runs where
//! present and is skipped, saying so, where not.

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn tier1(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("corpus/tier1")
        .join(name)
        .join(format!("{name}.rvt"))
}

fn sample() -> Option<PathBuf> {
    let p = std::env::var_os("RVT_REVIT2024_SAMPLES_DIR")
        .map(PathBuf::from)?
        .join("racbasicsampleproject.rvt");
    if p.exists() {
        Some(p)
    } else {
        eprintln!("skipping: racbasicsampleproject.rvt not under RVT_REVIT2024_SAMPLES_DIR");
        None
    }
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rvt-data-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(bin: &str, args: &[&std::ffi::OsStr]) -> Output {
    Command::new(bin)
        .args(args)
        .env("SOURCE_DATE_EPOCH", "1700000000")
        .output()
        .expect("spawn CLI")
}

fn ok(out: &Output) {
    assert!(
        out.status.success(),
        "exit {:?}\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn os(s: &str) -> &std::ffi::OsStr {
    std::ffi::OsStr::new(s)
}

/// `rvt-ifc <input> -o <out> <args>`, returning the IFC text.
fn ifc(input: &Path, out: &Path, args: &[&str]) -> String {
    let mut a: Vec<&std::ffi::OsStr> = vec![input.as_os_str(), os("-o"), out.as_os_str()];
    a.extend(args.iter().map(|s| os(s)));
    ok(&run(env!("CARGO_BIN_EXE_rvt-ifc"), &a));
    std::fs::read_to_string(out).expect("IFC written")
}

/// The DATA section's instances: id to (entity name, attribute text).
fn instances(step: &str) -> BTreeMap<u32, (String, String)> {
    let mut out = BTreeMap::new();
    for line in step.lines() {
        let Some(rest) = line.strip_prefix('#') else {
            continue;
        };
        let Some((id, body)) = rest.split_once('=') else {
            continue;
        };
        let (Some(open), Some(close)) = (body.find('('), body.rfind(')')) else {
            continue;
        };
        out.insert(
            id.parse().expect("instance id"),
            (body[..open].to_string(), body[open + 1..close].to_string()),
        );
    }
    out
}

/// Top-level attributes of an instance's attribute text.
fn attributes(args: &str) -> Vec<String> {
    let (mut out, mut cur, mut depth, mut quoted) = (Vec::new(), String::new(), 0, false);
    for c in args.chars() {
        match c {
            '\'' => quoted = !quoted,
            '(' if !quoted => depth += 1,
            ')' if !quoted => depth -= 1,
            ',' if !quoted && depth == 0 => {
                out.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    out.push(cur);
    out
}

/// The `#n` references outside strings.
fn refs(args: &str) -> Vec<u32> {
    let mut out = Vec::new();
    let mut quoted = false;
    let b = args.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\'' {
            quoted = !quoted;
        } else if b[i] == b'#' && !quoted {
            let j = (i + 1..b.len())
                .find(|&j| !b[j].is_ascii_digit())
                .unwrap_or(b.len());
            out.push(args[i + 1..j].parse().unwrap());
            i = j;
            continue;
        }
        i += 1;
    }
    out
}

/// Every reference names an instance of the file.
fn assert_no_dangling(step: &str) {
    let inst = instances(step);
    for (id, (name, args)) in &inst {
        for r in refs(args) {
            assert!(
                inst.contains_key(&r),
                "#{id}={name} names #{r}, which is not in the file"
            );
        }
    }
}

fn count(step: &str, entity: &str) -> usize {
    instances(step)
        .values()
        .filter(|(n, _)| n == entity)
        .count()
}

fn property_set_names(step: &str) -> BTreeSet<String> {
    instances(step)
        .values()
        .filter(|(n, _)| n == "IFCPROPERTYSET")
        .map(|(_, a)| attributes(a)[2].trim_matches('\'').to_string())
        .collect()
}

/// The building elements (products with a placement and a GlobalId), by
/// entity name.
fn products(step: &str) -> BTreeMap<String, usize> {
    let inst = instances(step);
    let mut out = BTreeMap::new();
    for (name, args) in inst.values() {
        if name.starts_with("IFCREL") || name.ends_with("TYPE") || !args.starts_with('\'') {
            continue;
        }
        let a = attributes(args);
        if a.len() > 6
            && a[5].starts_with('#')
            && !matches!(
                name.as_str(),
                "IFCPROJECT" | "IFCSITE" | "IFCBUILDING" | "IFCBUILDINGSTOREY"
            )
        {
            *out.entry(name.clone()).or_insert(0) += 1;
        }
    }
    out
}

fn real(s: &str) -> f64 {
    s.trim().replace(".E", ".0E").parse().expect("a real")
}

#[test]
fn element_table_has_one_record_per_element_with_parameters() {
    for input in [tier1("architectural-2024")].into_iter().chain(sample()) {
        let dir = scratch("table");
        let out = dir.join("t.jsonl");
        ok(&run(
            env!("CARGO_BIN_EXE_rvt-schedule"),
            &[
                input.as_os_str(),
                os("--format"),
                os("jsonl"),
                os("-o"),
                out.as_os_str(),
            ],
        ));
        let text = std::fs::read_to_string(&out).unwrap();
        assert!(text.ends_with('\n') || text.is_empty());
        let records: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).expect("each line is one JSON object"))
            .collect();
        for r in &records {
            for key in [
                "element_id",
                "global_id",
                "ifc_class",
                "category",
                "family",
                "type",
                "level",
                "area_m2",
                "volume_m3",
                "length_m",
                "parameters",
            ] {
                assert!(r.get(key).is_some(), "record lacks {key}: {r}");
            }
            assert!(r["global_id"].as_str().is_some_and(|g| g.len() == 22));
            for p in r["parameters"].as_array().unwrap() {
                assert!(p["set"].is_string() && p["name"].is_string());
            }
        }
        if input == tier1("architectural-2024") {
            continue;
        }
        // The real sample: the GlobalIds are the IFC file's, and most
        // elements have a category and parameters.
        assert!(records.len() > 400, "{} records", records.len());
        let step = ifc(&input, &dir.join("full.ifc"), &[]);
        let gids: BTreeSet<String> = instances(&step)
            .values()
            .filter_map(|(_, a)| {
                a.strip_prefix('\'')
                    .and_then(|a| a.get(..22))
                    .map(str::to_string)
            })
            .collect();
        let missing = records
            .iter()
            .filter(|r| !gids.contains(r["global_id"].as_str().unwrap()))
            .count();
        assert_eq!(
            missing, 0,
            "records whose GlobalId the IFC file does not have"
        );
        let with_category = records.iter().filter(|r| r["category"].is_string()).count();
        let with_parameters = records
            .iter()
            .filter(|r| r["parameters"].as_array().is_some_and(|p| p.len() > 3))
            .count();
        assert!(
            with_category * 10 >= records.len() * 9,
            "{with_category} of {}",
            records.len()
        );
        assert!(
            with_parameters * 10 >= records.len() * 9,
            "{with_parameters} of {}",
            records.len()
        );
        let walls: Vec<&Value> = records
            .iter()
            .filter(|r| r["category"] == "Walls")
            .collect();
        assert!(!walls.is_empty());
        assert!(
            walls
                .iter()
                .filter(|w| w["volume_m3"].as_f64().is_some_and(|v| v > 0.0))
                .count()
                * 2
                > walls.len(),
            "most walls have a volume"
        );
    }
}

/// `rvt-schedule <input> --format <format> -o <out>`.
fn schedule(input: &Path, format: &str, out: &Path) {
    ok(&run(
        env!("CARGO_BIN_EXE_rvt-schedule"),
        &[
            input.as_os_str(),
            os("--format"),
            os(format),
            os("-o"),
            out.as_os_str(),
        ],
    ));
}

#[test]
fn element_table_formats_hold_the_jsonl_rows() {
    use arrow_array::Array;
    use arrow_array::cast::AsArray;
    for input in [tier1("architectural-2024")].into_iter().chain(sample()) {
        let dir = scratch("formats");
        schedule(&input, "jsonl", &dir.join("t.jsonl"));
        let records: Vec<Value> = std::fs::read_to_string(dir.join("t.jsonl"))
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        let parameters: usize = records
            .iter()
            .map(|r| r["parameters"].as_array().unwrap().len())
            .sum();

        schedule(&input, "parquet", &dir.join("t.parquet"));
        let reader = parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::try_new(
            std::fs::File::open(dir.join("t.parquet")).unwrap(),
        )
        .unwrap()
        .build()
        .unwrap();
        let (mut rows, mut params) = (0, 0);
        for batch in reader {
            let batch = batch.unwrap();
            let ids = batch
                .column_by_name("global_id")
                .unwrap()
                .as_string::<i32>();
            for i in 0..batch.num_rows() {
                assert_eq!(
                    records[rows + i]["global_id"].as_str(),
                    (!ids.is_null(i)).then(|| ids.value(i))
                );
            }
            rows += batch.num_rows();
            params += batch
                .column_by_name("parameters")
                .unwrap()
                .as_list::<i32>()
                .values()
                .len();
        }
        assert_eq!((rows, params), (records.len(), parameters));

        schedule(&input, "sqlite", &dir.join("t.sqlite"));
        let db = rusqlite::Connection::open(dir.join("t.sqlite")).unwrap();
        let count = |sql: &str| -> usize {
            db.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap() as usize
        };
        assert_eq!(count("SELECT count(*) FROM elements"), records.len());
        assert_eq!(count("SELECT count(*) FROM parameters"), parameters);
        assert_eq!(
            count(
                "SELECT count(*) FROM parameters p LEFT JOIN elements e USING (element_row) WHERE e.element_row IS NULL"
            ),
            0
        );

        schedule(&input, "xlsx", &dir.join("t.xlsx"));
        let xlsx = std::fs::read(dir.join("t.xlsx")).unwrap();
        assert_eq!(&xlsx[..2], b"PK");
    }
}

#[test]
fn ifc_profiles_leave_out_the_sets_they_define() {
    let Some(input) = sample().or_else(|| Some(tier1("architectural-2024"))) else {
        return;
    };
    let dir = scratch("profile");
    let full = ifc(&input, &dir.join("full.ifc"), &[]);
    let coordination = ifc(
        &input,
        &dir.join("c.ifc"),
        &["--ifc-profile", "coordination"],
    );
    let geometry = ifc(&input, &dir.join("g.ifc"), &["--ifc-profile", "geometry"]);
    for step in [&coordination, &geometry] {
        assert_no_dangling(step);
        assert_eq!(
            products(step),
            products(&full),
            "a profile keeps every element"
        );
        assert_eq!(count(step, "IFCELEMENTQUANTITY"), 0);
    }
    for name in property_set_names(&coordination) {
        assert!(
            name.starts_with("Pset_") || name == "Revit",
            "coordination kept {name}"
        );
    }
    for name in property_set_names(&geometry) {
        assert_eq!(name, "Revit", "geometry kept {name}");
    }
    assert!(
        count(&coordination, "IFCPROPERTYSET") <= count(&full, "IFCPROPERTYSET"),
        "the profile only removes"
    );
    if sample().is_some() {
        assert!(
            property_set_names(&coordination)
                .iter()
                .any(|n| n.starts_with("Pset_"))
        );
        assert!(count(&full, "IFCELEMENTQUANTITY") > 0);
    }
}

/// Entity classes whose real numbers are not lengths, areas or volumes in
/// the project's units (directions, ratios, colours, angles, counts, other
/// measures, conversion factors), with the ones rescaled: every class with a
/// real number in a written file must be in one of the two lists, so a
/// writer change that adds a length somewhere new fails here.
const NOT_LENGTHS: &[&str] = &[
    "IFCDIRECTION",
    "IFCCOLOURRGB",
    "IFCCOLOURRGBLIST",
    "IFCSURFACESTYLERENDERING",
    "IFCPROPERTYSINGLEVALUE",
    "IFCPROPERTYLISTVALUE",
    "IFCQUANTITYCOUNT",
    "IFCQUANTITYWEIGHT",
    "IFCQUANTITYTIME",
    "IFCCARTESIANTRANSFORMATIONOPERATOR3D",
    "IFCREVOLVEDAREASOLID",
    "IFCMEASUREWITHUNIT",
    "IFCMAPCONVERSION",
    "IFCINDEXEDCOLOURMAP",
    "IFCGEOMETRICREPRESENTATIONSUBCONTEXT",
    "IFCOWNERHISTORY",
    "IFCSITE",
];
const LENGTHS: &[&str] = &[
    "IFCCARTESIANPOINT",
    "IFCCARTESIANPOINTLIST2D",
    "IFCCARTESIANPOINTLIST3D",
    "IFCEXTRUDEDAREASOLID",
    "IFCRECTANGLEPROFILEDEF",
    "IFCRECTANGLEHOLLOWPROFILEDEF",
    "IFCCIRCLEPROFILEDEF",
    "IFCCIRCLEHOLLOWPROFILEDEF",
    "IFCISHAPEPROFILEDEF",
    "IFCLSHAPEPROFILEDEF",
    "IFCUSHAPEPROFILEDEF",
    "IFCTSHAPEPROFILEDEF",
    "IFCBUILDINGSTOREY",
    "IFCBUILDING",
    "IFCSPACE",
    "IFCDOOR",
    "IFCWINDOW",
    "IFCSTAIRFLIGHT",
    "IFCMATERIALLAYER",
    "IFCMATERIALLAYERSETUSAGE",
    "IFCMATERIALPROFILESETUSAGE",
    "IFCGEOMETRICREPRESENTATIONCONTEXT",
    "IFCQUANTITYLENGTH",
    "IFCQUANTITYAREA",
    "IFCQUANTITYVOLUME",
];

/// Whether `args` holds a real number outside strings and references.
fn has_real(args: &str) -> bool {
    let mut quoted = false;
    let mut prev = ',';
    let b: Vec<char> = args.chars().collect();
    for (i, &c) in b.iter().enumerate() {
        if c == '\'' {
            quoted = !quoted;
        }
        if !quoted && c == '.' && i > 0 && b[i - 1].is_ascii_digit() && prev != '#' {
            // A digit run before the point that is not a reference.
            let mut j = i;
            while j > 0 && b[j - 1].is_ascii_digit() {
                j -= 1;
            }
            if j == 0 || b[j - 1] != '#' {
                return true;
            }
        }
        if !c.is_ascii_digit() {
            prev = c;
        }
    }
    false
}

#[test]
fn ifc_units_rescale_every_length() {
    let Some(input) = sample().or_else(|| Some(tier1("architectural-2024"))) else {
        return;
    };
    let dir = scratch("units");
    let metres = ifc(&input, &dir.join("m.ifc"), &[]);
    let m = instances(&metres);
    for (name, args) in m.values() {
        if has_real(args) {
            assert!(
                LENGTHS.contains(&name.as_str()) || NOT_LENGTHS.contains(&name.as_str()),
                "{name} holds a real number and is in neither list: decide whether it is a length"
            );
        }
    }
    for (unit, k) in [("mm", 1000.0), ("ft", 1.0 / 0.3048), ("in", 1.0 / 0.0254)] {
        let step = ifc(
            &input,
            &dir.join(format!("{unit}.ifc")),
            &["--ifc-units", unit],
        );
        assert_no_dangling(&step);
        let s = instances(&step);
        let mut points = 0;
        for (id, (name, args)) in &m {
            let Some((_, scaled)) = s.get(id) else {
                continue;
            };
            match name.as_str() {
                "IFCCARTESIANPOINT" => {
                    let a: Vec<f64> = args.trim_matches(['(', ')']).split(',').map(real).collect();
                    let b: Vec<f64> = scaled
                        .trim_matches(['(', ')'])
                        .split(',')
                        .map(real)
                        .collect();
                    for (x, y) in a.iter().zip(&b) {
                        assert!(
                            (x * k - y).abs() <= 1e-6 * (1.0 + y.abs()),
                            "#{id}: {x} -> {y} in {unit}"
                        );
                    }
                    points += 1;
                }
                "IFCBUILDINGSTOREY" => {
                    let (x, y) = (real(&attributes(args)[9]), real(&attributes(scaled)[9]));
                    assert!((x * k - y).abs() <= 1e-6 * (1.0 + y.abs()));
                }
                _ => {}
            }
        }
        assert!(points > 0);
        let unit_word = match unit {
            "mm" => ".MILLI.,.METRE.",
            "ft" => "'FOOT'",
            _ => "'INCH'",
        };
        assert!(
            step.contains(unit_word),
            "{unit}: the unit assignment names {unit_word}"
        );
    }
}

#[test]
fn ifc_origin_moves_the_site_or_says_why_not() {
    // The synthetic fixture has no base points: the option fails, writing
    // nothing, and says which point is missing.
    let dir = scratch("origin");
    let out = dir.join("o.ifc");
    let fixture = tier1("architectural-2024");
    let r = run(
        env!("CARGO_BIN_EXE_rvt-ifc"),
        &[
            fixture.as_os_str(),
            os("-o"),
            out.as_os_str(),
            os("--ifc-origin"),
            os("survey-point"),
        ],
    );
    assert!(!r.status.success());
    assert!(String::from_utf8_lossy(&r.stderr).contains("survey point was not read"));
    assert!(!out.exists());

    let Some(input) = sample() else { return };
    let internal = ifc(&input, &dir.join("i.ifc"), &[]);
    for origin in ["project-base", "survey-point"] {
        let step = ifc(
            &input,
            &dir.join(format!("{origin}.ifc")),
            &["--ifc-origin", origin],
        );
        assert_no_dangling(&step);
        let inst = instances(&step);
        let site = inst.values().find(|(n, _)| n == "IFCSITE").expect("site");
        let placement = refs(&attributes(&site.1)[5])[0];
        let axis = refs(&attributes(&inst[&placement].1)[1])[0];
        let location = refs(&attributes(&inst[&axis].1)[0])[0];
        let xyz: Vec<f64> = inst[&location]
            .1
            .trim_matches(['(', ')'])
            .split(',')
            .map(real)
            .collect();
        assert_eq!(xyz.len(), 3);
        // Everything else is the internal export's.
        assert_eq!(products(&step), products(&internal));
        eprintln!("{origin}: site at {xyz:?}");
    }
}

#[test]
fn ifc_categories_keep_only_the_named_elements() {
    let Some(input) = sample() else { return };
    let dir = scratch("categories");
    let full = ifc(&input, &dir.join("full.ifc"), &[]);
    let out = dir.join("walls.ifc");
    let r = run(
        env!("CARGO_BIN_EXE_rvt-ifc"),
        &[
            input.as_os_str(),
            os("-o"),
            out.as_os_str(),
            os("--ifc-categories"),
            os("walls,IfcSlab,Nonsense"),
        ],
    );
    ok(&r);
    assert!(String::from_utf8_lossy(&r.stderr).contains("matched no element for: Nonsense"));
    let walls = std::fs::read_to_string(&out).unwrap();
    assert_no_dangling(&walls);
    let kept = products(&walls);
    let all = products(&full);
    assert_eq!(kept.get("IFCDOOR"), None, "doors left out: {kept:?}");
    assert_eq!(kept.get("IFCSLAB"), all.get("IFCSLAB"), "every slab kept");
    assert!(kept.contains_key("IFCWALL"), "{kept:?}");
    assert!(walls.len() < full.len());
}

#[test]
fn phase_is_checked_against_the_files_phases() {
    let Some(input) = sample() else { return };
    let dir = scratch("phase");
    let out = dir.join("p.ifc");
    let r = run(
        env!("CARGO_BIN_EXE_rvt-ifc"),
        &[
            input.as_os_str(),
            os("-o"),
            out.as_os_str(),
            os("--phase"),
            os("No Such Phase"),
        ],
    );
    assert!(!r.status.success());
    let err = String::from_utf8_lossy(&r.stderr).into_owned();
    assert!(err.contains("phases are"), "{err}");
    // Each named phase converts; the last is the default export.
    let phases: Vec<String> = err
        .split("phases are ")
        .nth(1)
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .split(", ")
        .map(|s| s.trim().to_string())
        .collect();
    let mut default = products(&ifc(&input, &dir.join("default.ifc"), &[]));
    let last = phases.last().unwrap().clone();
    let mut at_last = products(&ifc(&input, &dir.join("last.ifc"), &["--phase", &last]));
    // Where the last phase holds no room the default writes the rooms of the
    // latest phase that holds any, and naming the last writes none; every
    // other element is the same.
    let rooms = (default.remove("IFCSPACE"), at_last.remove("IFCSPACE"));
    assert!(
        rooms.0 >= rooms.1,
        "the default keeps at least the named phase's rooms: {rooms:?}"
    );
    assert_eq!(
        at_last, default,
        "--phase <last phase> is the default export but for its rooms"
    );
    for phase in &phases {
        let step = ifc(&input, &dir.join("each.ifc"), &["--phase", phase]);
        assert_no_dangling(&step);
    }
}

#[test]
fn obj_export_has_a_group_per_element_and_its_materials() {
    for input in [tier1("architectural-2024")].into_iter().chain(sample()) {
        let dir = scratch("obj");
        let out = dir.join("m.obj");
        ok(&run(
            env!("CARGO_BIN_EXE_rvt-gltf"),
            &[
                input.as_os_str(),
                os("--format"),
                os("obj"),
                os("-o"),
                out.as_os_str(),
            ],
        ));
        let obj = std::fs::read_to_string(&out).unwrap();
        let mtl = std::fs::read_to_string(dir.join("m.mtl")).expect("m.mtl beside it");
        assert!(obj.contains("mtllib m.mtl"));
        let defined: BTreeSet<&str> = mtl
            .lines()
            .filter_map(|l| l.strip_prefix("newmtl "))
            .collect();
        let (mut vertices, mut faces, mut groups) = (0usize, 0usize, 0usize);
        for line in obj.lines() {
            if line.starts_with("v ") {
                vertices += 1;
                assert_eq!(line.split(' ').skip(1).map(real).count(), 3);
            } else if let Some(f) = line.strip_prefix("f ") {
                faces += 1;
                for i in f.split(' ') {
                    let i: usize = i.parse().unwrap();
                    assert!(
                        i >= 1 && i <= vertices,
                        "face index {i} of {vertices} vertices"
                    );
                }
            } else if let Some(m) = line.strip_prefix("usemtl ") {
                assert!(defined.contains(m), "usemtl {m} not in the .mtl");
            } else if let Some(g) = line.strip_prefix("g ") {
                groups += 1;
                assert!(!g.contains(' '));
            }
        }
        if input != tier1("architectural-2024") {
            assert!(
                groups > 100 && faces > 1000,
                "{groups} groups, {faces} faces"
            );
            assert!(
                obj.lines()
                    .filter(|l| l.starts_with("g ") && l.ends_with(']'))
                    .count()
                    * 2
                    > groups
            );
        }
    }
}

#[test]
fn run_report_names_outputs_and_embeds_the_diagnostics() {
    let dir = scratch("report");
    let input = tier1("architectural-2024");
    let report = dir.join("run.json");
    ok(&run(
        env!("CARGO_BIN_EXE_rvt-ifc"),
        &[
            input.as_os_str(),
            os("-o"),
            dir.join("m.ifc").as_os_str(),
            os("--report"),
            report.as_os_str(),
        ],
    ));
    let r: Value = serde_json::from_str(&std::fs::read_to_string(&report).unwrap()).unwrap();
    assert_eq!(r["schema_version"], 1);
    assert_eq!(r["tool"], "rvt-ifc");
    assert_eq!(r["status"], "ok");
    assert_eq!(r["outputs"][0]["format"], "ifc");
    assert!(r["outputs"][0]["bytes"].as_u64().unwrap() > 0);
    assert!(r["diagnostics"]["schema_version"].is_number());
    assert!(r["elements_by_category"].is_object());
}

#[test]
fn batch_converts_a_tree_once_and_never_overwrites() {
    let root = scratch("batch");
    let input = root.join("in");
    std::fs::create_dir_all(input.join("sub")).unwrap();
    std::fs::copy(tier1("architectural-2024"), input.join("a.rvt")).unwrap();
    std::fs::copy(tier1("mep-2024"), input.join("sub").join("b.RVT")).unwrap();
    std::fs::write(input.join("notes.txt"), "not a model").unwrap();
    let out = root.join("out");
    let batch = |extra: &[&str]| {
        let mut a: Vec<&std::ffi::OsStr> = vec![input.as_os_str(), os("-o"), out.as_os_str()];
        a.extend(extra.iter().map(|s| os(s)));
        run(env!("CARGO_BIN_EXE_rvt-batch"), &a)
    };
    let first = batch(&["--formats", "ifc,table,glb,obj,xlsx,parquet,sqlite"]);
    ok(&first);
    let lines: Vec<Value> = String::from_utf8_lossy(&first.stdout)
        .lines()
        .map(|l| serde_json::from_str(l).expect("one JSON status line per file"))
        .collect();
    assert_eq!(lines.len(), 2, "two Revit files, the text file ignored");
    let statuses: HashMap<String, String> = lines
        .iter()
        .map(|l| {
            (
                l["input"].as_str().unwrap().to_string(),
                l["status"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert!(statuses.values().all(|s| s == "ok"), "{statuses:?}");
    for (folder, stem) in [
        (out.join("a.rvt"), "a"),
        (out.join("sub").join("b.RVT"), "b"),
    ] {
        for file in [
            format!("{stem}.ifc"),
            format!("{stem}.elements.jsonl"),
            format!("{stem}.glb"),
            format!("{stem}.obj"),
            format!("{stem}.mtl"),
            format!("{stem}.elements.xlsx"),
            format!("{stem}.elements.parquet"),
            format!("{stem}.elements.sqlite"),
            "report.json".to_string(),
            "manifest.json".to_string(),
        ] {
            assert!(
                folder.join(&file).exists(),
                "{} missing",
                folder.join(&file).display()
            );
        }
        let r: Value =
            serde_json::from_str(&std::fs::read_to_string(folder.join("report.json")).unwrap())
                .unwrap();
        assert_eq!(r["status"], "ok");
        assert_eq!(r["tool"], "rvt-batch");
        // The manifest is written last, names every file with its size and
        // SHA-256, and the folder is no longer marked unfinished.
        assert!(!folder.join(".incomplete").exists());
        let m = rvt::ifc::run_report::OutputManifest::read(
            &std::fs::read_to_string(folder.join("manifest.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(m.report.as_deref(), Some("report.json"));
        assert_eq!(m.outputs.len(), 9);
        for o in &m.outputs {
            use sha2::Digest;
            let bytes = std::fs::read(folder.join(&o.file)).unwrap();
            assert_eq!(bytes.len() as u64, o.bytes, "{}", o.file);
            let hex: String = sha2::Sha256::digest(&bytes)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            assert_eq!(hex, o.sha256, "{}", o.file);
        }
    }
    // A second run finds both folders and writes nothing.
    let before = std::fs::read(out.join("a.rvt").join("a.ifc")).unwrap();
    let second = batch(&[]);
    ok(&second);
    assert!(
        String::from_utf8_lossy(&second.stdout)
            .lines()
            .all(|l| l.contains("\"skipped\""))
    );
    assert_eq!(
        std::fs::read(out.join("a.rvt").join("a.ifc")).unwrap(),
        before
    );
    // A folder a crash left unfinished is converted again.
    std::fs::write(out.join("a.rvt").join(".incomplete"), b"").unwrap();
    std::fs::write(out.join("a.rvt").join("a.ifc"), b"half").unwrap();
    let again = batch(&["--formats", "ifc"]);
    ok(&again);
    assert!(String::from_utf8_lossy(&again.stdout).contains("\"ok\""));
    assert!(
        std::fs::read(out.join("a.rvt").join("a.ifc"))
            .unwrap()
            .len()
            > 4
    );
    assert!(!out.join("a.rvt").join("a.glb").exists());
    // A file that is not a Revit file fails on its own, exit status 2.
    std::fs::write(input.join("broken.rvt"), b"not a compound file").unwrap();
    let third = batch(&[]);
    assert_eq!(third.status.code(), Some(2));
    let r: Value = serde_json::from_str(
        &std::fs::read_to_string(out.join("broken.rvt").join("report.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(r["status"], "failed");
    assert!(r["error"].is_string());
}
