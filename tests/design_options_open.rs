//! under `open` every design option's
//! elements are written, each option an `IfcGroup` of its members inside an
//! `IfcGroup` of its set, each member with a `Design Option` property set;
//! under `revit` the non-primary options stay out and no group is written.
//!
//! Runs against `RVT_REVIT2024_SAMPLES_DIR` (Snowdon Towers architectural:
//! sets "Bandstand Options" and "Bleacher Seating") and `RVT_REVIT_EXTRA_DIR`
//! (`<release>/<label>.rvt`: Pacific Continental Residence 2027, set
//! "Variations"); skips what is absent. IFCX and OpenUSD carry each
//! member's `Design Option` set (groups are IFC4 only). Option names and memberships are the
//! API dump's.

use super::truss_framing::{entities, refs, text};
use rvt::RevitFile;
use rvt::ifc::{ExportConvention, GraphOptions, IfcxGraph, RvtDocExporter, write_ifcx};
use rvt::walker::WalkerLimits;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

fn model(path: &Path, convention: ExportConvention) -> rvt::ifc::IfcModel {
    let mut rf = RevitFile::open(path).expect("open");
    RvtDocExporter
        .export_with_convention(
            &mut rf,
            rvt::ifc::ExportQualityMode::Scaffold,
            WalkerLimits::default(),
            convention,
        )
        .expect("export")
}

fn step(m: &rvt::ifc::IfcModel) -> String {
    rvt::ifc::write_step_graph(&IfcxGraph::from_model(
        m,
        GraphOptions {
            meshes: false,
            shared_shapes: false,
        },
    ))
}

/// Option groups by name: their set's name and their members' Tags.
type Groups = BTreeMap<String, (String, BTreeSet<String>)>;

/// Option groups by name: their set's name and their members' Tags; and
/// every element Tag of the file.
fn option_groups(step: &str) -> (Groups, BTreeSet<String>) {
    let ents = entities(step);
    let name_of = |id: &u64| ents.get(id).and_then(|(_, a)| text(&a[2]));
    let kind = |id: &u64, want: &str| {
        ents.get(id)
            .is_some_and(|(c, a)| c == "IFCGROUP" && text(&a[4]).as_deref() == Some(want))
    };
    let tag = |id: &u64| {
        ents.get(id)
            .and_then(|(_, a)| a.get(7))
            .and_then(|t| text(t))
    };
    let mut out = Groups::new();
    for (class, a) in ents.values() {
        if class != "IFCRELASSIGNSTOGROUP" || a.len() < 7 {
            continue;
        }
        let Some(group) = refs(&a[6]).first().copied() else {
            continue;
        };
        if kind(&group, "Design Option") {
            let members = refs(&a[4]).iter().filter_map(&tag).collect();
            out.entry(name_of(&group).unwrap_or_default())
                .or_default()
                .1 = members;
        } else if kind(&group, "Design Option Set") {
            for option in refs(&a[4]) {
                out.entry(name_of(&option).unwrap_or_default())
                    .or_default()
                    .0 = name_of(&group).unwrap_or_default();
            }
        }
    }
    let all = ents
        .values()
        .filter(|(c, a)| c.starts_with("IFC") && c != "IFCOPENINGELEMENT" && a.len() > 7)
        .filter_map(|(_, a)| text(&a[7]))
        .collect();
    (out, all)
}

/// `(option, set, members, primary)` expected in a file.
type Expected<'a> = &'a [(&'a str, &'a str, usize, bool)];

fn check(path: &Path, expected: Expected<'_>, required: &[(&str, &[u32])]) {
    let open_model = model(path, ExportConvention::Open);
    let (groups, open_tags) = option_groups(&step(&open_model));
    let got: BTreeMap<&str, (&str, usize)> = groups
        .iter()
        .map(|(n, (s, m))| (n.as_str(), (s.as_str(), m.len())))
        .collect();
    let want: BTreeMap<&str, (&str, usize)> =
        expected.iter().map(|(o, s, n, _)| (*o, (*s, *n))).collect();
    assert_eq!(
        got,
        want,
        "{}: option groups (set, members)",
        path.display()
    );
    let (revit_groups, revit_tags) = option_groups(&step(&model(path, ExportConvention::Revit)));
    assert!(
        revit_groups.is_empty(),
        "revit writes no design option group"
    );
    for (option, set, _, primary) in expected {
        // Element Tags only: a room's 8th attribute is its long name, which
        // rooms of other options share (rooms: `rooms_in_design_options_2023`).
        let members: BTreeSet<String> = groups[*option]
            .1
            .iter()
            .filter(|m| m.parse::<u64>().is_ok())
            .cloned()
            .collect();
        let members = &members;
        if *primary {
            assert!(
                members.is_subset(&revit_tags),
                "{option}: primary members in revit"
            );
        } else {
            assert!(
                members.is_disjoint(&revit_tags),
                "{option} ({set}): non-primary members left out by revit"
            );
        }
    }
    for (option, ids) in required {
        for id in *ids {
            assert!(
                open_tags.contains(&id.to_string()),
                "{option}: {id} written"
            );
            assert!(
                groups[*option].1.contains(&id.to_string()),
                "{option}: {id} grouped"
            );
        }
    }
    // IFCX carries each member's set, option and primary flag.
    let ifcx = write_ifcx(&open_model).expect("ifcx");
    let grouped: usize = groups.values().map(|(_, m)| m.len()).sum();
    let carried = ifcx
        .matches("\"rvtopen::revit::Design Option::Design Option Set\"")
        .count();
    assert!(
        carried >= grouped,
        "IFCX Design Option sets {carried} < {grouped}"
    );
}

#[test]
fn open_groups_snowdon_design_options() {
    let Some(dir) = std::env::var_os("RVT_REVIT2024_SAMPLES_DIR") else {
        eprintln!("skipping: RVT_REVIT2024_SAMPLES_DIR is not set");
        return;
    };
    let path = Path::new(&dir).join("Snowdon Towers Sample Architectural.rvt");
    if !path.exists() {
        eprintln!("skipping: Snowdon Towers Sample Architectural absent");
        return;
    }
    check(
        &path,
        &[
            ("Twisty Trellis", "Bandstand Options", 33, true),
            ("Simple Dome", "Bandstand Options", 1, false),
            ("Blank", "Bandstand Options", 0, false),
            ("Generative", "Bandstand Options", 0, false),
            ("Middle Stair", "Bleacher Seating", 29, true),
            (
                "Continuous Bleachers no Stairs",
                "Bleacher Seating",
                26,
                false,
            ),
        ],
        &[],
    );
}

#[test]
fn open_writes_pacific_2027_options() {
    let Some(dir) = std::env::var_os("RVT_REVIT_EXTRA_DIR") else {
        eprintln!("skipping: RVT_REVIT_EXTRA_DIR is not set");
        return;
    };
    let path = Path::new(&dir).join("2027/Pacific_Continental_Residence_Sample__pf2027.rvt");
    if !path.exists() {
        eprintln!("skipping: Pacific Continental Residence 2027 absent");
        return;
    }
    check(
        &path,
        PACIFIC,
        &[(
            "The Charmant",
            &[
                2162435, 2165459, 2165466, 2165468, 2165470, 2165472, 2165474, 2165476,
            ],
        )],
    );
}

/// Pacific Continental Residence 2027: the API's set "Variations" and its
/// options, with the members open writes (every written member
/// readers writes two more members in each option (209, 221, 202 before);
/// those two are not yet scored against the API.
const PACIFIC: Expected<'static> = &[
    // 211, 223 and 204 before gutters were recovered from their element
    // records: each variation's gutters are its members too.
    ("El Cabrillo", "Variations", 224, true),
    ("The Charmant", "Variations", 241, false),
    ("The Dover", "Variations", 220, false),
];

/// The `RVT_REVIT_EXTRA_DIR` file `<release>/<label>.rvt`, or `None` (with
/// a skip message) when the directory or the file is absent.
fn extra(release_and_label: &str) -> Option<std::path::PathBuf> {
    let Some(dir) = std::env::var_os("RVT_REVIT_EXTRA_DIR") else {
        eprintln!("skipping: RVT_REVIT_EXTRA_DIR is not set");
        return None;
    };
    let path = Path::new(&dir).join(format!("{release_and_label}.rvt"));
    if !path.exists() {
        eprintln!("skipping: {release_and_label} absent");
        return None;
    }
    Some(path)
}

/// design options on the `u32` record releases 2018-2023 (the
/// option 26 bytes before an element record's marker; sets, primaries and
/// option names with `u32` ids). Every option group named as the API,
/// with the members open writes (every written API member grouped as the
/// API, `score318.py`); revit leaves the non-primary members out.
fn open_groups_design_options_2018_to_2023(k: usize, n: usize) {
    const SET_GN_HVAC: &str = "Klimatisierung Hofgebäude";
    const SET_CJK: &str = "南側ファサード";
    let cases: &[(&str, Expected<'static>)] = &[
        (
            "2018/rst_basic_sample_project__pf2018",
            &[
                ("Option 1", "Option Set 1", 0, true),
                ("Option 2", "Option Set 1", 0, false),
                ("Option 3", "Option Set 1", 0, false),
            ],
        ),
        (
            "2019/BIM_Projekt_Golden_Nugget-Gebaeudetechnik__pf2019",
            // with pipe insulation and pipe and duct accessories
            // exported, 1,797 and 290 members (were 1,084 and 258), every one
            // the API dump's member of that option (635 pipe insulations, 62
            // pipe accessories, 47 duct accessories added).
            &[
                ("Dezentrale Klimatisierung", SET_GN_HVAC, 1797, true),
                ("Klimatisierung auf Luftbasis", SET_GN_HVAC, 290, false),
            ],
        ),
        (
            "2022/cjk_9480d97c__pf2022",
            &[("B", SET_CJK, 1129, true), ("A", SET_CJK, 872, false)],
        ),
        (
            "2023/BIM_Projekt_Golden_Nugget-Architektur_und_Ingenieurbau__pf2023",
            GN_BALCONY,
        ),
    ];
    for (file, expected) in crate::shard(cases, k, n) {
        if let Some(path) = extra(file) {
            check(&path, expected, &[]);
        }
    }
}

split!(open_groups_design_options_2018_to_2023, open_groups_design_options_2018_to_2023, s0 0, s1 1, s2 2, s3 3);

/// Golden Nugget Architektur 2023, set "Balkonvarianten Hofseite": members
/// are the distinct Tags (and room long names) of each group; each option
/// holds two rooms.
const GN_BALCONY: Expected<'static> = &[
    (
        "Beton und Stahl kombiniert",
        "Balkonvarianten Hofseite",
        31,
        true,
    ),
    ("Stahlgeländer", "Balkonvarianten Hofseite", 19, false),
    ("Glasgeländer", "Balkonvarianten Hofseite", 19, false),
];

/// Rooms in a design option: under open each of the three
/// balcony options of Golden Nugget Architektur 2023 holds two `IfcSpace`
/// members; revit writes the primary's and leaves the other four out, as
/// Revit's own export of the same project (DACH 2024, 86 spaces) does.
fn rooms_in_design_options_2023(k: usize, n: usize) {
    let Some(path) = extra("2023/BIM_Projekt_Golden_Nugget-Architektur_und_Ingenieurbau__pf2023")
    else {
        return;
    };
    let count = |s: &str| {
        entities(s)
            .values()
            .filter(|(c, _)| c == "IFCSPACE")
            .count()
    };
    // One shard per export: open's checks, then revit's.
    for convention in crate::shard([ExportConvention::Open, ExportConvention::Revit], k, n) {
        if convention == ExportConvention::Revit {
            let revit_step = step(&model(&path, ExportConvention::Revit));
            assert_eq!(
                count(&revit_step),
                86,
                "revit leaves 4 non-primary rooms out"
            );
            continue;
        }
        let open_step = step(&model(&path, ExportConvention::Open));
        let ents = entities(&open_step);
        let mut spaces_in_options = 0;
        for (class, a) in ents.values() {
            if class == "IFCRELASSIGNSTOGROUP" && a.len() > 6 {
                let group = refs(&a[6]).first().copied();
                let is_option = group.and_then(|g| ents.get(&g)).is_some_and(|(c, ga)| {
                    c == "IFCGROUP" && text(&ga[4]).as_deref() == Some("Design Option")
                });
                if is_option {
                    spaces_in_options += refs(&a[4])
                        .iter()
                        .filter(|m| ents.get(*m).is_some_and(|(c, _)| c == "IFCSPACE"))
                        .count();
                }
            }
        }
        assert_eq!(spaces_in_options, 6, "rooms grouped under open");
        assert_eq!(count(&open_step), 90, "open writes every room");
    }
}

split!(rooms_in_design_options_2023, rooms_in_design_options_2023, s0 0, s1 1);

/// Sets outside the partitions' leading record chain: cjk
/// 2024 and 2026 resolve their set by the set list's marker.
#[test]
fn open_groups_unchained_option_sets() {
    const SET_CJK: &str = "南側ファサード";
    for (file, primary) in [
        ("2024/cjk_138949ac__pf2024", 1130),
        ("2026/cjk_1706fd8d__pf2026", 1130),
    ] {
        if let Some(path) = extra(file) {
            check(
                &path,
                &[("B", SET_CJK, primary, true), ("A", SET_CJK, 872, false)],
                &[],
            );
        }
    }
}
