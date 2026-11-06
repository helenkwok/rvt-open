//! Structural Truss assemblies and the framing class on every
//! release.
//!
//! 1. Revit's IFC export aggregates a Structural Truss's members into an
//!    `IfcElementAssembly` `.TRUSS.` contained in the storey of the truss's
//!    Reference Level. Checked against Revit's own IFC of rstadvanced
//!    (`RVT_REVIT2024_SAMPLES_DIR`): the 2 trusses, their GlobalId, Name,
//!    ObjectType, 26 members, storey, and every property rvt-open writes on
//!    them equal to Revit's.
//! 2. The same trusses and members on the 2018 to 2023 saves of that sample
//!    (`RVT_REVIT_EXTRA_DIR`, `<release>/<label>.rvt`, the files of the API
//! 3. The framing class by the API's `StructuralType` on 2018 to 2027
//!    (`RVT_REVIT_EXTRA_DIR`): Brace is `IfcMember` `.BRACE.`, NonStructural a
//!    proxy, counted against the API dump of each file.
//!
//! Each check skips the files that are absent.

use rvt::RevitFile;
use rvt::ifc::{RvtDocExporter, write_step};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// One STEP entity: its class and top-level attributes.
pub(crate) type Entities = BTreeMap<u64, (String, Vec<String>)>;

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

pub(crate) fn entities(step: &str) -> Entities {
    let mut out = Entities::new();
    for line in step.lines() {
        let Some((id, body)) = line.strip_prefix('#').and_then(|r| r.split_once('=')) else {
            continue;
        };
        let Some((class, args)) = body.split_once('(') else {
            continue;
        };
        let args = args.trim_end().trim_end_matches(';');
        let args = args.strip_suffix(')').unwrap_or(args);
        if let Ok(id) = id.trim().parse() {
            out.insert(id, (class.trim().to_string(), attributes(args)));
        }
    }
    out
}

pub(crate) fn refs(attr: &str) -> Vec<u64> {
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

pub(crate) fn text(attr: &str) -> Option<String> {
    let (start, end) = (attr.find('\'')?, attr.rfind('\'')?);
    (end > start).then(|| super::category_type_phase::decode(&attr[start + 1..end]))
}

/// Metres per length unit of a STEP file's project.
fn metres_per_unit(step: &str) -> f64 {
    if step.contains("'FOOT'") {
        0.3048
    } else if step.contains("IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.)") {
        0.001
    } else {
        1.0
    }
}

/// What a file says about one truss assembly.
#[derive(Debug, Default, PartialEq)]
struct Assembly {
    global_id: String,
    name: Option<String>,
    object_type: Option<String>,
    members: BTreeSet<String>,
    storey: Option<String>,
    /// `set.property` -> value as written (a length in metres).
    properties: BTreeMap<String, String>,
}

/// Every `.TRUSS.` assembly of a STEP file by `Tag`.
fn trusses(step: &str) -> BTreeMap<String, Assembly> {
    let ents = entities(step);
    let scale = metres_per_unit(step);
    let tag = |id: &u64| {
        ents.get(id)
            .and_then(|(_, a)| a.get(7))
            .and_then(|t| text(t))
    };
    let mut out: BTreeMap<u64, Assembly> = ents
        .iter()
        .filter(|(_, (c, a))| {
            c == "IFCELEMENTASSEMBLY" && a.get(9).is_some_and(|p| p.trim() == ".TRUSS.")
        })
        .map(|(id, (_, a))| {
            (
                *id,
                Assembly {
                    global_id: text(&a[0]).unwrap_or_default(),
                    name: text(&a[2]),
                    object_type: text(&a[4]),
                    ..Assembly::default()
                },
            )
        })
        .collect();
    for (class, a) in ents.values() {
        match class.as_str() {
            "IFCRELAGGREGATES" if a.len() >= 6 => {
                if let Some(whole) = refs(&a[4]).first().and_then(|w| out.get_mut(w)) {
                    whole.members = refs(&a[5]).iter().filter_map(&tag).collect();
                }
            }
            "IFCRELCONTAINEDINSPATIALSTRUCTURE" if a.len() >= 6 => {
                let storey = refs(&a[5])
                    .first()
                    .and_then(|s| ents.get(s))
                    .and_then(|(_, sa)| text(&sa[2]));
                for e in refs(&a[4]) {
                    if let Some(whole) = out.get_mut(&e) {
                        whole.storey = storey.clone();
                    }
                }
            }
            "IFCRELDEFINESBYPROPERTIES" if a.len() >= 6 => {
                let Some((_, set)) = refs(&a[5]).first().and_then(|s| ents.get(s)) else {
                    continue;
                };
                let set_name = text(&set[2]).unwrap_or_default();
                let props: Vec<(String, String)> = refs(&set[4])
                    .iter()
                    .filter_map(|p| ents.get(p))
                    .filter(|(c, _)| c == "IFCPROPERTYSINGLEVALUE")
                    .filter_map(|(_, pa)| {
                        let v = pa[2].trim();
                        let (ty, rest) = v.split_once('(')?;
                        let inner = rest.strip_suffix(')')?;
                        let value = if ty == "IFCLENGTHMEASURE" {
                            format!("{:.4}", inner.parse::<f64>().ok()? * scale)
                        } else if inner.starts_with('\'') {
                            text(inner)?
                        } else {
                            inner.to_string()
                        };
                        Some((
                            format!("{set_name}.{}", text(&pa[0])?),
                            format!("{ty} {value}"),
                        ))
                    })
                    .collect();
                for e in refs(&a[4]) {
                    if let Some(whole) = out.get_mut(&e) {
                        whole.properties.extend(props.iter().cloned());
                    }
                }
            }
            _ => {}
        }
    }
    out.into_iter()
        .filter_map(|(id, a)| Some((tag(&id)?, a)))
        .collect()
}

fn export(path: &Path) -> String {
    let mut rf = RevitFile::open(path).expect("open");
    let model = RvtDocExporter
        .export_with_diagnostics(&mut rf)
        .expect("export")
        .model;
    write_step(&model)
}

/// The members of the two trusses of rstadvanced, the API's `Truss.Members`
const TRUSSES: &[(&str, &[u32])] = &[
    (
        "132598",
        &[
            132740, 132741, 132742, 132744, 132745, 132746, 132748, 132749, 132750, 132751, 132752,
            132753, 132754,
        ],
    ),
    (
        "133226",
        &[
            133227, 133228, 133229, 133230, 133231, 133232, 133233, 133234, 133235, 133236, 133237,
            133238, 133239,
        ],
    ),
];

fn api_trusses() -> BTreeMap<String, BTreeSet<String>> {
    TRUSSES
        .iter()
        .map(|(t, m)| (t.to_string(), m.iter().map(u32::to_string).collect()))
        .collect()
}

/// The properties rvt-open writes on a truss assembly that Revit writes too.
const WRITTEN: &[&str] = &[
    "Other.Category",
    "Other.Family",
    "Other.Family and Type",
    "Other.Type",
    "Other.Type Id",
    "Phasing.Phase Created",
    "Pset_ElementAssemblyCommon.Reference",
    "Pset_EnvironmentalImpactIndicators.Reference",
    "Constraints.Reference Level",
    "Other(Type).Category",
    "Other(Type).Family Name",
    "Identity Data(Type).Type Name",
];

#[test]
fn truss_assemblies_match_revits_export() {
    let Some(dir) = std::env::var_os("RVT_REVIT2024_SAMPLES_DIR").map(PathBuf::from) else {
        eprintln!("skipping: RVT_REVIT2024_SAMPLES_DIR is not set");
        return;
    };
    let rvt = dir.join("rstadvancedsampleproject.rvt");
    let reference = dir.join("rstadvancedsampleproject.ifc");
    if !rvt.exists() || !reference.exists() {
        eprintln!("skipping: rstadvancedsampleproject absent");
        return;
    }
    let theirs = trusses(&std::fs::read_to_string(&reference).expect("reference"));
    let ours = trusses(&export(&rvt));
    assert_eq!(
        ours.keys().collect::<Vec<_>>(),
        theirs.keys().collect::<Vec<_>>(),
        "truss assemblies by Tag"
    );
    let api = api_trusses();
    for (tag, revit) in &theirs {
        let mine = &ours[tag];
        assert_eq!(
            revit.members, api[tag],
            "{tag}: Revit's members are the API's"
        );
        assert_eq!(mine.members, revit.members, "{tag}: members");
        assert_eq!(mine.global_id, revit.global_id, "{tag}: GlobalId");
        assert_eq!(mine.name, revit.name, "{tag}: Name");
        assert_eq!(mine.object_type, revit.object_type, "{tag}: ObjectType");
        assert_eq!(mine.storey, revit.storey, "{tag}: storey");
        for key in WRITTEN {
            assert_eq!(
                mine.properties.get(*key),
                revit.properties.get(*key),
                "{tag}: {key}"
            );
            assert!(
                revit.properties.contains_key(*key),
                "{tag}: Revit writes {key}"
            );
        }
        // Nothing else rvt-open writes in a set Revit writes differs.
        for (key, value) in &mine.properties {
            if let Some(theirs) = revit.properties.get(key) {
                assert_eq!(value, theirs, "{tag}: {key}");
            }
        }
    }
}

fn extra(file: &str) -> Option<PathBuf> {
    let Some(dir) = std::env::var_os("RVT_REVIT_EXTRA_DIR").map(PathBuf::from) else {
        eprintln!("skipping: RVT_REVIT_EXTRA_DIR is not set");
        return None;
    };
    let path = dir.join(file);
    path.exists().then_some(path).or_else(|| {
        eprintln!("skipping: {file} absent");
        None
    })
}

#[test]
fn truss_members_on_older_releases() {
    for year in [2018, 2019, 2022, 2023] {
        let file = format!("{year}/rst_advanced_sample_project__pf{year}.rvt");
        let Some(path) = extra(&file) else { continue };
        let ours: BTreeMap<String, BTreeSet<String>> = trusses(&export(&path))
            .into_iter()
            .map(|(t, a)| (t, a.members))
            .collect();
        assert_eq!(ours, api_trusses(), "{file}: trusses and members");
    }
}

/// dump): Brace count, and the NonStructural ids. One file per release with
/// braces and one with NonStructural framing where the corpus has them.
const FRAMING: &[(&str, usize, &[u32])] = &[
    ("2018/rst_advanced_sample_project__pf2018.rvt", 22, &[]),
    ("2018/rac_advanced_sample_project__pf2018.rvt", 0, &[201308]),
    (
        "2019/BIM_Projekt_Golden_Nugget-Architektur_und_Ingenieurbau__pf2019.rvt",
        36,
        &[],
    ),
    ("2022/rst_advanced_sample_project__pf2022.rvt", 22, &[]),
    ("2022/rac_advanced_sample_project__pf2022.rvt", 0, &[201308]),
    (
        "2023/BIM_Projekt_Golden_Nugget-Architektur_und_Ingenieurbau__pf2023.rvt",
        59,
        &[],
    ),
    ("2023/rac_advanced_sample_project__pf2023.rvt", 0, &[201308]),
    (
        "2026/BIM_Projekt_Golden_Nugget-Architektur_und_Ingenieurbau__pf2026.rvt",
        59,
        &[],
    ),
    (
        "2026/cjk_c267da0d__pf2026.rvt",
        0,
        &[
            4590681, 4590769, 4590789, 4590811, 4590831, 4590851, 4669416, 4669809, 4669897,
            4669919, 4669941, 4669963, 4669987, 4670427, 4670448, 4670467, 4670492, 4670513,
        ],
    ),
    (
        "2027/BIM_Projekt_Golden_Nugget-Architektur_und_Ingenieurbau__pf2027.rvt",
        59,
        &[],
    ),
];

fn framing_class_on_every_release(k: usize, n: usize) {
    for (file, braces, non_structural) in crate::shard(FRAMING, k, n) {
        let Some(path) = extra(file) else { continue };
        let step = export(&path);
        let ents = entities(&step);
        let by_class = |class: &str, predefined: &str| -> BTreeSet<String> {
            ents.values()
                .filter(|(c, a)| c == class && a.last().is_some_and(|p| p.trim() == predefined))
                .filter_map(|(_, a)| text(&a[7]))
                .collect()
        };
        assert_eq!(
            by_class("IFCMEMBER", ".BRACE.").len(),
            *braces,
            "{file}: IfcMember .BRACE."
        );
        let proxies = by_class("IFCBUILDINGELEMENTPROXY", ".NOTDEFINED.");
        for id in *non_structural {
            assert!(proxies.contains(&id.to_string()), "{file}: {id} is a proxy");
        }
    }
}

split!(framing_class_on_every_release, framing_class_on_every_release, s0 0, s1 1, s2 2, s3 3, s4 4, s5 5, s6 6, s7 7, s8 8, s9 9);
