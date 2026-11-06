//! the parameters Revit's IFC export writes on a Structural Beam
//! System (`IfcElementAssembly`, `BEAM_GRID`): `Pattern`, `Constraints`,
//! `Identity Data` and the type's `Other(Type)` and `Identity Data(Type)`.
//!
//! Two checks over `RVT_REVIT2024_SAMPLES_DIR` (Autodesk's and the DACH Revit
//! 2024 samples, each with Revit's own IFC beside it; skipped where absent):
//!
//! 1. every value rvt-open writes equals Revit's, assembly by assembly
//!    (matched by `Tag`, the ElementId), lengths compared in metres;
//!    element id by element id), and how many assemblies carry each property.
//!
//! One kind of difference is expected and pinned: Revit's exporter writes some
//! spacings of 2 and 6 feet in the two "advanced" projects and the Technical
//! school divided by 304.8 instead of multiplied (0.0065616798 for 2 ft in a
//! millimetre file). The API says 2 and 6 feet, and so does rvt-open.

use rvt::RevitFile;
use rvt::ifc::{RvtDocExporter, write_step};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const FEET: f64 = 0.3048;

/// The models with a Structural Beam System and Revit's IFC beside them.
const MODELS: &[&str] = &[
    "racadvancedsampleproject",
    "rstadvancedsampleproject",
    "rstbasicsampleproject",
    "Technicalschoolcurrentm",
    "DACHsampleproject",
    "Snowdon Towers Sample Structural",
];

/// A property value: its IFC type and its payload as written.
type Value = (String, String);

/// `set -> property -> value` of one assembly.
type Sets = BTreeMap<String, BTreeMap<String, Value>>;

/// Our assemblies and metres per unit, and Revit's assemblies and metres per unit.
type Both = (BTreeMap<String, Sets>, f64, BTreeMap<String, Sets>, f64);

/// A system the API describes: model, system, and `set.property` with its value.
type Described<'a> = (&'a str, &'a str, &'a [(&'a str, &'a str)]);

/// Top-level comma-separated attributes of a STEP entity's argument list.
fn attributes(args: &str) -> Vec<String> {
    let (mut out, mut cur, mut depth, mut quoted) = (Vec::new(), String::new(), 0i32, false);
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

fn refs(attr: &str) -> Vec<u64> {
    attr.split('#')
        .skip(1)
        .filter_map(|s| {
            s.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse()
                .ok()
        })
        .collect()
}

fn unquote(attr: &str) -> Option<String> {
    let start = attr.find('\'')?;
    let end = attr.rfind('\'')?;
    (end > start).then(|| super::category_type_phase::decode(&attr[start + 1..end]))
}

/// A nominal value `IFCTYPE(payload)` as `(type, payload)`, a string
/// payload decoded.
fn value(attr: &str) -> Option<Value> {
    let attr = attr.trim();
    let (ty, rest) = attr.split_once('(')?;
    let inner = rest.strip_suffix(')')?;
    let payload = if inner.starts_with('\'') {
        unquote(inner)?
    } else {
        inner.to_string()
    };
    Some((ty.to_string(), payload))
}

/// Metres per length unit of a STEP file's project.
fn metres_per_unit(step: &str) -> f64 {
    if step.contains("'FOOT'") {
        FEET
    } else if step.contains("IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.)") {
        0.001
    } else {
        1.0
    }
}

/// The property sets of every `BEAM_GRID` assembly of a STEP file by `Tag`.
fn assemblies(step: &str) -> BTreeMap<String, Sets> {
    let mut ents: BTreeMap<u64, (String, Vec<String>)> = BTreeMap::new();
    for line in step.lines() {
        let Some((id, body)) = line.strip_prefix('#').and_then(|r| r.split_once('=')) else {
            continue;
        };
        let Some((entity, args)) = body.split_once('(') else {
            continue;
        };
        let args = args.trim_end().trim_end_matches(';').trim_end_matches(')');
        if let Ok(id) = id.trim().parse() {
            ents.insert(id, (entity.trim().to_string(), attributes(args)));
        }
    }
    let mut tag_of: BTreeMap<u64, String> = BTreeMap::new();
    for (id, (entity, attrs)) in &ents {
        if entity == "IFCELEMENTASSEMBLY" && attrs.get(9).is_some_and(|p| p.trim() == ".BEAM_GRID.")
        {
            if let Some(tag) = attrs.get(7).and_then(|t| unquote(t)) {
                tag_of.insert(*id, tag);
            }
        }
    }
    let mut out: BTreeMap<String, Sets> =
        tag_of.values().map(|t| (t.clone(), Sets::new())).collect();
    for (entity, attrs) in ents.values() {
        if entity != "IFCRELDEFINESBYPROPERTIES" || attrs.len() < 6 {
            continue;
        }
        let Some((_, pset)) = refs(&attrs[5]).first().and_then(|p| ents.get(p)) else {
            continue;
        };
        let (Some(name), Some(props)) = (pset.get(2).and_then(|n| unquote(n)), pset.get(4)) else {
            continue;
        };
        let mut values = BTreeMap::new();
        for p in refs(props) {
            let Some((e, a)) = ents.get(&p) else { continue };
            if e == "IFCPROPERTYSINGLEVALUE" {
                if let (Some(n), Some(v)) = (unquote(&a[0]), a.get(2).and_then(|v| value(v))) {
                    values.insert(n, v);
                }
            }
        }
        for element in refs(&attrs[4]) {
            if let Some(tag) = tag_of.get(&element) {
                out.get_mut(tag)
                    .expect("tag")
                    .entry(name.clone())
                    .or_default()
                    .extend(values.clone());
            }
        }
    }
    out
}

fn samples_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("RVT_REVIT2024_SAMPLES_DIR").map(PathBuf::from);
    if dir.is_none() {
        eprintln!("skipping: RVT_REVIT2024_SAMPLES_DIR is not set");
    }
    dir
}

/// Our assemblies of `stem` and Revit's, with each file's metres per unit.
fn both(dir: &Path, stem: &str) -> Option<Both> {
    let rvt = dir.join(format!("{stem}.rvt"));
    let reference = rvt.with_extension("ifc");
    if !rvt.exists() || !reference.exists() {
        eprintln!("skipping: {stem} absent");
        return None;
    }
    let theirs = std::fs::read_to_string(&reference).expect("reference");
    let mut rf = RevitFile::open(&rvt).expect("open");
    let model = RvtDocExporter
        .export_with_diagnostics(&mut rf)
        .expect("export")
        .model;
    let step = write_step(&model);
    Some((
        assemblies(&step),
        metres_per_unit(&step),
        assemblies(&theirs),
        metres_per_unit(&theirs),
    ))
}

/// The properties scored (set, property); `Other`, `Phasing` and the
/// `Pset_*` references were scored by earlier measurements.
const CHECKED: &[(&str, &str)] = &[
    ("Pattern", "Layout Rule"),
    ("Pattern", "Justification"),
    ("Pattern", "Beam Type"),
    ("Pattern", "Beam Type (No Family Name)"),
    ("Pattern", "Centerline Spacing"),
    ("Pattern", "Fixed Spacing"),
    ("Pattern", "Maximum Spacing"),
    ("Constraints", "3D"),
    ("Constraints", "Elevation from Level"),
    ("Constraints", "Reference Level Elevation"),
    ("Constraints", "Work Plane"),
    ("Identity Data", "Comments"),
    ("Other(Type)", "Category"),
    ("Other(Type)", "Family Name"),
    ("Identity Data(Type)", "Type Name"),
    ("Identity Data(Type)", "Description"),
];

/// Every value written equals Revit's; the one known exporter quirk is counted.
fn beam_system_values_equal_revits(k: usize, n: usize) {
    let Some(dir) = samples_dir() else { return };
    let mut quirk = 0;
    let mut equal: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    for stem in crate::shard(MODELS, k, n) {
        let Some((ours, our_unit, theirs, their_unit)) = both(&dir, stem) else {
            continue;
        };
        for (tag, sets) in &ours {
            let Some(want) = theirs.get(tag) else {
                continue;
            };
            for (set, prop) in CHECKED {
                let (Some(mine), Some(rev)) = (
                    sets.get(*set).and_then(|s| s.get(*prop)),
                    want.get(*set).and_then(|s| s.get(*prop)),
                ) else {
                    continue;
                };
                let same = if mine.0 == "IFCLENGTHMEASURE" {
                    let m = mine.1.parse::<f64>().expect("number") * our_unit;
                    let r = rev.1.parse::<f64>().expect("number") * their_unit;
                    if (m - r).abs() <= 1.5e-6 {
                        true
                    } else {
                        // Revit's quirk: whole feet divided by 304.8 in a millimetre file
                        assert!(
                            (r - m / FEET / 304_800.0).abs() < 1e-9,
                            "{stem} {tag} {set}.{prop}: ours {m} m, Revit {r} m"
                        );
                        quirk += 1;
                        continue;
                    }
                } else {
                    mine == rev
                };
                assert!(
                    same,
                    "{stem} {tag} {set}.{prop}: ours {mine:?}, Revit {rev:?}"
                );
                *equal.entry((set, prop)).or_default() += 1;
            }
        }
    }
    // The pins are on the totals over all shards, checked once all have run:
    // a record is the shard's quirk count, then `set\tprop\tequal` lines.
    let mut record = quirk.to_string();
    for ((set, prop), count) in &equal {
        record.push_str(&format!("\n{set}\t{prop}\t{count}"));
    }
    let Some(all) = crate::all_shards("beam_system_params-equal_revits", k, n, &record) else {
        return;
    };
    let mut quirk = 0usize;
    let mut equal: BTreeMap<(String, String), usize> = BTreeMap::new();
    for record in &all {
        let mut lines = record.lines();
        quirk += lines.next().unwrap().parse::<usize>().unwrap();
        for line in lines {
            let f: Vec<&str> = line.split('\t').collect();
            *equal.entry((f[0].into(), f[1].into())).or_default() += f[2].parse::<usize>().unwrap();
        }
    }
    eprintln!("equal {equal:?}, Revit unit quirk {quirk}");
    assert_eq!(quirk, QUIRK_VALUES, "Revit's spacing quirk");
    for ((set, prop), least) in EQUAL_AT_LEAST {
        let got = equal
            .get(&(set.to_string(), prop.to_string()))
            .copied()
            .unwrap_or(0);
        assert!(got >= *least, "{set}.{prop}: {got} equal, want {least}");
    }
}

split!(beam_system_values_equal_revits, beam_system_values_equal_revits, s0 0, s1 1, s2 2, s3 3, s4 4, s5 5);

/// Spacings (Centerline, Fixed, Maximum) Revit's exporter divides by 304.8:
/// 14 + 2 + 12 on the advanced samples and the Technical school.
const QUIRK_VALUES: usize = 28;

/// Assemblies on which a property equals Revit's, at least (measured
/// on the six models, 109 assemblies; the spacings exclude the 28
/// of [`QUIRK_VALUES`]: 109, 37 and 72 assemblies carry them).
const EQUAL_AT_LEAST: &[((&str, &str), usize)] = &[
    (("Pattern", "Layout Rule"), 109),
    (("Pattern", "Justification"), 109),
    (("Pattern", "Beam Type"), 109),
    (("Pattern", "Beam Type (No Family Name)"), 109),
    (("Pattern", "Centerline Spacing"), 95),
    (("Pattern", "Fixed Spacing"), 35),
    (("Pattern", "Maximum Spacing"), 60),
    (("Constraints", "3D"), 109),
    (("Constraints", "Elevation from Level"), 105),
    (("Constraints", "Reference Level Elevation"), 104),
    (("Constraints", "Work Plane"), 109),
    (("Identity Data", "Comments"), 76),
    (("Identity Data(Type)", "Description"), 2),
    (("Other(Type)", "Category"), 109),
    (("Other(Type)", "Family Name"), 109),
    (("Identity Data(Type)", "Type Name"), 109),
];

/// Systems the Revit API dump describes, by model and ElementId: what the
/// API gave for the parameters (feet there, metres here).
fn beam_system_values_equal_the_api(k: usize, n: usize) {
    let Some(dir) = samples_dir() else { return };
    // (stem, system, set.property -> expected)
    let cases: &[Described] = &[
        (
            "rstbasicsampleproject",
            "1175801",
            &[
                ("Pattern.Layout Rule", "Fixed Distance"),
                ("Pattern.Justification", "Center"),
                (
                    "Pattern.Beam Type",
                    "Rectangular(Timber)-Beam : Timber_50 x200",
                ),
                ("Pattern.Beam Type (No Family Name)", "Timber_50 x200"),
                ("Constraints.Work Plane", "Level : Level 2"),
                ("Other(Type).Family Name", "Structural Beam System"),
                ("Identity Data(Type).Type Name", "Structural Framing System"),
            ],
        ),
        (
            "rstbasicsampleproject",
            "1301742",
            &[("Constraints.Work Plane", "<not associated>")],
        ),
        (
            "Snowdon Towers Sample Structural",
            "651718",
            &[
                ("Pattern.Layout Rule", "Maximum Spacing"),
                ("Identity Data.Comments", "@ Max 4'-0\""),
                ("Constraints.Work Plane", "Level : L3 TOS"),
            ],
        ),
        (
            "DACHsampleproject",
            "3364550",
            &[
                ("Pattern.Justification", "Direction Line"),
                ("Constraints.Work Plane", "<not associated>"),
                ("Identity Data(Type).Description", "Dachstuhl Hofgebäude"),
            ],
        ),
    ];
    // lengths in feet from the API: (stem, system, set.property, feet)
    let lengths: &[(&str, &str, &str, f64)] = &[
        (
            "rstbasicsampleproject",
            "1175801",
            "Pattern.Fixed Spacing",
            1.476_377_952_755_905_6,
        ),
        (
            "rstbasicsampleproject",
            "1175801",
            "Pattern.Centerline Spacing",
            1.476_377_952_755_905_6,
        ),
        (
            "rstbasicsampleproject",
            "1175801",
            "Constraints.Elevation from Level",
            0.183_727_034_120_734_9,
        ),
        (
            "rstbasicsampleproject",
            "1175801",
            "Constraints.Reference Level Elevation",
            9.842_519_685_039_37,
        ),
        (
            "rstbasicsampleproject",
            "1301742",
            "Constraints.Elevation from Level",
            0.131_233_595_800_524_93,
        ),
        (
            "Snowdon Towers Sample Structural",
            "651718",
            "Pattern.Maximum Spacing",
            6.0,
        ),
        (
            "Snowdon Towers Sample Structural",
            "651718",
            "Pattern.Centerline Spacing",
            5.183_953_952_909_033,
        ),
        (
            "DACHsampleproject",
            "3364550",
            "Pattern.Fixed Spacing",
            1.968_503_937_007_873_8,
        ),
    ];
    // A shard exports its own models; each case is checked by the shard
    // that exports its model.
    let mut run = BTreeMap::new();
    for stem in crate::shard(MODELS, k, n) {
        if let Some((ours, unit, _, _)) = both(&dir, stem) {
            run.insert(*stem, (ours, unit));
        }
    }
    for (stem, system, wanted) in cases {
        let Some((ours, _)) = run.get(stem) else {
            continue;
        };
        let sets = ours
            .get(*system)
            .unwrap_or_else(|| panic!("{stem} {system}: no assembly"));
        for (key, want) in *wanted {
            let (set, prop) = key.split_once('.').expect("set.property");
            let got = sets.get(set).and_then(|s| s.get(prop));
            assert_eq!(
                got.map(|v| v.1.as_str()),
                Some(*want),
                "{stem} {system} {key}"
            );
        }
    }
    for (stem, system, key, feet) in lengths {
        let Some((ours, unit)) = run.get(stem) else {
            continue;
        };
        let (set, prop) = key.split_once('.').expect("set.property");
        let got = ours
            .get(*system)
            .and_then(|s| s.get(set))
            .and_then(|s| s.get(prop))
            .unwrap_or_else(|| panic!("{stem} {system} {key}: absent"));
        let metres = got.1.parse::<f64>().expect("number") * unit;
        assert!(
            (metres - feet * FEET).abs() <= 1.5e-6,
            "{stem} {system} {key}: {metres} m, API {feet} ft"
        );
    }
}

split!(beam_system_values_equal_the_api, beam_system_values_equal_the_api, s0 0, s1 1, s2 2, s3 3, s4 4, s5 5);
