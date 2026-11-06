//! switch point, in both conventions, against Revit's own IFC export of the
//! same file.
//!
//! Runs against `RVT_REVIT2024_SAMPLES_DIR` and skips gracefully when absent.
//!
//! Switch points (see `rvt::ifc::ExportConvention`):
//! 1. a beam no placing rule placed: `revit` = storey of the lowest-ElementId
//!    Level, `open` = lowest storey by elevation;
//! 2. a model with no building storey: `revit` = no IfcBuilding, the IfcSite
//!    holds the elements, `open` = the IfcBuilding "Default Building";
//! 3. structural framing class: `revit` = IfcBeam / IfcMember / proxy by the
//!    API's StructuralType, `open` = IfcBeam (until a rule is chosen).

use rvt::RevitFile;
use rvt::ifc::{
    ExportConvention, ExportQualityMode, GraphOptions, IfcxGraph, RvtDocExporter, write_step_graph,
};
use rvt::walker::WalkerLimits;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

fn samples() -> Option<PathBuf> {
    let dir = std::env::var_os("RVT_REVIT2024_SAMPLES_DIR").map(PathBuf::from);
    if dir.is_none() {
        eprintln!("skipping: RVT_REVIT2024_SAMPLES_DIR is not set");
    }
    dir
}

/// Our export as STEP text, derived the way `rvt-ifc` derives it.
fn export_step(dir: &std::path::Path, stem: &str, convention: ExportConvention) -> String {
    let mut rf = RevitFile::open(dir.join(format!("{stem}.rvt"))).expect("open rvt");
    let model = RvtDocExporter
        .export_with_convention(
            &mut rf,
            ExportQualityMode::Scaffold,
            WalkerLimits::default(),
            convention,
        )
        .expect("export");
    write_step_graph(&IfcxGraph::from_model(
        &model,
        GraphOptions {
            meshes: false,
            shared_shapes: false,
        },
    ))
}

fn split_args(args: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let (mut quoted, mut depth) = (false, 0usize);
    for c in args.chars() {
        match c {
            '\'' => {
                quoted = !quoted;
                current.push(c);
            }
            '(' if !quoted => {
                depth += 1;
                current.push(c);
            }
            ')' if !quoted => {
                depth = depth.saturating_sub(1);
                current.push(c);
            }
            ',' if !quoted && depth == 0 => out.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    out.push(current);
    out
}

fn references(field: &str) -> Vec<u64> {
    split_args(field.trim().trim_start_matches('(').trim_end_matches(')'))
        .iter()
        .filter_map(|f| f.trim().strip_prefix('#')?.parse().ok())
        .collect()
}

/// `#id = ENTITY(args);` lines of a STEP file.
fn entities(step: &str) -> BTreeMap<u64, (String, Vec<String>)> {
    let mut out = BTreeMap::new();
    for line in step.lines() {
        let Some(rest) = line.strip_prefix('#') else {
            continue;
        };
        let Some((id, body)) = rest.split_once('=') else {
            continue;
        };
        let Some((entity, args)) = body.split_once('(') else {
            continue;
        };
        let args = args.trim_end().trim_end_matches(';');
        let args = args.strip_suffix(')').unwrap_or(args);
        if let Ok(id) = id.trim().parse() {
            out.insert(id, (entity.trim().to_string(), split_args(args)));
        }
    }
    out
}

fn text(arg: &str) -> String {
    arg.trim().trim_matches('\'').to_string()
}

struct Step {
    ents: BTreeMap<u64, (String, Vec<String>)>,
}

impl Step {
    fn new(step: &str) -> Self {
        Self {
            ents: entities(step),
        }
    }

    fn count(&self, class: &str) -> usize {
        self.ents.values().filter(|(c, _)| c == class).count()
    }

    /// Tag of an element entity (argument 7 of every IfcElement).
    fn tag(&self, id: u64) -> Option<String> {
        let (_, args) = self.ents.get(&id)?;
        let tag = text(args.get(7)?);
        (!tag.is_empty() && tag.chars().all(|c| c.is_ascii_digit())).then_some(tag)
    }

    /// Entity class by Tag, for the given classes. A Tag two entities of those
    /// classes share is left out: it names no one element.
    fn class_by_tag(&self, classes: &[&str]) -> BTreeMap<String, String> {
        let mut out: BTreeMap<String, Option<String>> = BTreeMap::new();
        for (id, (class, _)) in &self.ents {
            if !classes.contains(&class.as_str()) {
                continue;
            }
            if let Some(tag) = self.tag(*id) {
                out.entry(tag)
                    .and_modify(|c| {
                        if c.as_deref() != Some(class.as_str()) {
                            *c = None
                        }
                    })
                    .or_insert_with(|| Some(class.clone()));
            }
        }
        out.into_iter().filter_map(|(t, c)| Some((t, c?))).collect()
    }

    /// Container of every element by Tag: (container class, container name,
    /// container elevation for a storey).
    fn container_by_tag(&self) -> BTreeMap<String, (String, String, Option<String>)> {
        let mut out = BTreeMap::new();
        for (class, args) in self.ents.values() {
            if class != "IFCRELCONTAINEDINSPATIALSTRUCTURE" {
                continue;
            }
            let Some(structure) = args.get(5).and_then(|s| references(s).first().copied()) else {
                continue;
            };
            let Some((sclass, sargs)) = self.ents.get(&structure) else {
                continue;
            };
            let name = sargs.get(2).map(|n| text(n)).unwrap_or_default();
            let elevation = (sclass == "IFCBUILDINGSTOREY")
                .then(|| sargs.get(9).map(|e| text(e)))
                .flatten();
            for element in references(args.get(4).map(String::as_str).unwrap_or("")) {
                if let Some(tag) = self.tag(element) {
                    out.insert(tag, (sclass.clone(), name.clone(), elevation.clone()));
                }
            }
        }
        out
    }

    fn storeys(&self) -> Vec<(String, f64)> {
        self.ents
            .values()
            .filter(|(c, _)| c == "IFCBUILDINGSTOREY")
            .map(|(_, a)| (text(&a[2]), a[9].trim().parse().unwrap_or(f64::NAN)))
            .collect()
    }
}

/// Switch point 1. Elements no placing rule placed: 6 DACH framing elements
/// (a Reference Level below every storey) and the one framing proxy of
/// racadvancedsampleproject and of Technicalschoolcurrentm (no level at all).
const NO_RULE: &[(&str, &[&str])] = &[
    (
        "DACHsampleproject",
        &[
            "1656244", "1656246", "1656794", "1656796", "1657130", "1657134",
        ],
    ),
    ("racadvancedsampleproject", &["201308"]),
    ("Technicalschoolcurrentm", &["201308"]),
];

fn no_rule_beam_storey_follows_the_convention(k: usize, n: usize) {
    let Some(dir) = samples() else { return };
    for (stem, ids) in crate::shard(NO_RULE, k, n) {
        let reference =
            Step::new(&std::fs::read_to_string(dir.join(format!("{stem}.ifc"))).unwrap());
        let revit_container = reference.container_by_tag();
        let revit_step = Step::new(&export_step(&dir, stem, ExportConvention::Revit));
        let open_step = Step::new(&export_step(&dir, stem, ExportConvention::Open));
        let (revit_ours, open_ours) = (revit_step.container_by_tag(), open_step.container_by_tag());
        // The lowest storey by elevation, from our own storeys.
        let open_storeys = open_step.storeys();
        let lowest = open_storeys
            .iter()
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .expect("storeys")
            .0
            .clone();
        let mut differ = 0;
        for id in *ids {
            // `revit`: the storey Revit's own export names.
            let want = revit_container
                .get(*id)
                .unwrap_or_else(|| panic!("{stem} {id}: Revit has no container"));
            let got = revit_ours
                .get(*id)
                .unwrap_or_else(|| panic!("{stem} {id}: revit convention has no container"));
            assert_eq!(
                (got.0.as_str(), got.1.as_str()),
                (want.0.as_str(), want.1.as_str()),
                "{stem} {id}: revit convention storey"
            );
            // `open`: the lowest storey by elevation.
            let open = open_ours
                .get(*id)
                .unwrap_or_else(|| panic!("{stem} {id}: open convention has no container"));
            assert_eq!(
                (open.0.as_str(), open.1.as_str()),
                ("IFCBUILDINGSTOREY", lowest.as_str()),
                "{stem} {id}: open convention storey"
            );
            if open.1 != got.1 {
                differ += 1;
            }
        }
        eprintln!(
            "{stem}: {} elements, {differ} placed differently by the two conventions",
            ids.len()
        );
        if *stem == "DACHsampleproject" {
            // Revit's lowest-ElementId storey is not the lowest by elevation.
            assert_eq!(differ, ids.len(), "the conventions must differ on DACH");
        }
    }
}

split!(no_rule_beam_storey_follows_the_convention, no_rule_beam_storey_follows_the_convention, s0 0, s1 1, s2 2);

/// Switch point 2: rstbasicsampleproject has no building storey. Revit's own
/// export: 0 IfcBuilding, 0 IfcBuildingStorey, one IfcSite holding every
/// element that is not an aggregate part, and aggregating the one IfcSpace.
#[test]
fn model_without_storeys_follows_the_convention() {
    let Some(dir) = samples() else { return };
    let stem = "rstbasicsampleproject";
    let reference = Step::new(&std::fs::read_to_string(dir.join(format!("{stem}.ifc"))).unwrap());
    assert_eq!(
        reference.count("IFCBUILDING"),
        0,
        "Revit writes no IfcBuilding here"
    );
    assert_eq!(reference.count("IFCBUILDINGSTOREY"), 0);

    let revit = Step::new(&export_step(&dir, stem, ExportConvention::Revit));
    assert_eq!(revit.count("IFCBUILDING"), 0, "revit: no IfcBuilding");
    assert_eq!(
        revit.count("IFCBUILDINGSTOREY"),
        0,
        "revit: no invented storey"
    );
    let revit_containers = revit.container_by_tag();
    assert!(!revit_containers.is_empty());
    assert!(
        revit_containers.values().all(|c| c.0 == "IFCSITE"),
        "revit: every contained element is in the IfcSite"
    );
    // Every element Revit contains in the site that we export is contained
    // in the site too, and every one of ours is one Revit contains.
    let ref_containers = reference.container_by_tag();
    let missing: Vec<_> = revit_containers
        .keys()
        .filter(|t| !ref_containers.contains_key(*t))
        .collect();
    assert!(
        missing.is_empty(),
        "contained here, not in Revit: {missing:?}"
    );

    let open = Step::new(&export_step(&dir, stem, ExportConvention::Open));
    assert_eq!(open.count("IFCBUILDING"), 1, "open: the IfcBuilding stays");
    let open_containers = open.container_by_tag();
    assert!(open_containers.values().any(|c| c.0 == "IFCBUILDING"));
    assert!(!open_containers.values().any(|c| c.0 == "IFCSITE"));
    eprintln!(
        "{stem}: revit contains {} elements in the site; open contains {} in the building, {} in storeys",
        revit_containers.len(),
        open_containers
            .values()
            .filter(|c| c.0 == "IFCBUILDING")
            .count(),
        open_containers
            .values()
            .filter(|c| c.0 == "IFCBUILDINGSTOREY")
            .count(),
    );

    // The IFCX graph follows the same switch.
    let mut rf = RevitFile::open(dir.join(format!("{stem}.rvt"))).unwrap();
    for (convention, buildings) in [(ExportConvention::Revit, 0), (ExportConvention::Open, 1)] {
        let model = RvtDocExporter
            .export_with_convention(
                &mut rf,
                ExportQualityMode::Scaffold,
                WalkerLimits::default(),
                convention,
            )
            .unwrap();
        let ifcx: serde_json::Value =
            serde_json::from_str(&rvt::ifc::write_ifcx(&model).unwrap()).unwrap();
        let n = ifcx["data"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|n| {
                n["attributes"]["bsi::ifc::class"]["code"]
                    .as_str()
                    .is_some_and(|c| c.eq_ignore_ascii_case("IfcBuilding"))
            })
            .count();
        assert_eq!(n, buildings, "{convention:?}: IFCX IFCBUILDING nodes");
    }
}

/// Switch point 3: the framing class. Models where Revit writes IfcMember
/// (DACH 59, rstadvanced 22) or a proxy (id 201308 in racadvanced and
/// Technicalschool) for Structural Framing.
const FRAMING: &[(&str, usize, usize)] = &[
    ("DACHsampleproject", 59, 0),
    ("rstadvancedsampleproject", 22, 0),
    ("racadvancedsampleproject", 0, 1),
    ("Technicalschoolcurrentm", 0, 1),
];

fn framing_class_follows_the_convention(k: usize, n: usize) {
    let Some(dir) = samples() else { return };
    for (stem, members, proxies) in crate::shard(FRAMING, k, n) {
        let reference =
            Step::new(&std::fs::read_to_string(dir.join(format!("{stem}.ifc"))).unwrap());
        let classes = ["IFCBEAM", "IFCMEMBER", "IFCBUILDINGELEMENTPROXY"];
        let revit_class = reference.class_by_tag(&classes);

        let revit = Step::new(&export_step(&dir, stem, ExportConvention::Revit));
        let ours = revit.class_by_tag(&classes);
        // Every Tag ours writes as a beam, member or framing proxy that Revit
        // also writes as one of the three has the same class.
        let framing_tags: BTreeSet<&String> = ours
            .iter()
            .filter(|(_, c)| c.as_str() != "IFCBUILDINGELEMENTPROXY")
            .map(|(t, _)| t)
            .collect();
        let (mut same, mut wrong) = (0, Vec::new());
        for tag in framing_tags {
            if let Some(want) = revit_class.get(tag) {
                if want == &ours[tag] {
                    same += 1;
                } else {
                    wrong.push((tag.clone(), want.clone(), ours[tag].clone()));
                }
            }
        }
        assert!(
            wrong.is_empty(),
            "{stem}: revit convention class differs: {wrong:?}"
        );
        let n_members = ours.values().filter(|c| *c == "IFCMEMBER").count();
        let ref_members = revit_class.values().filter(|c| *c == "IFCMEMBER").count();
        assert!(
            n_members >= *members,
            "{stem}: revit convention writes {n_members} IfcMember, expected at least {members} (Revit {ref_members})"
        );
        if *proxies > 0 {
            assert_eq!(
                ours.get("201308").map(String::as_str),
                Some("IFCBUILDINGELEMENTPROXY"),
                "{stem}: the NonStructural framing element is a proxy"
            );
        }
        eprintln!("{stem}: {same} framing classes equal Revit's; {n_members} IfcMember");

        // `open` writes braces as IfcMember like `revit`, and the
        // NonStructural framing `revit` turns into proxies as IfcBeam;
        // nothing else differs in class.
        let open = Step::new(&export_step(&dir, stem, ExportConvention::Open));
        let open_classes = open.class_by_tag(&classes);
        let count = |m: &BTreeMap<String, String>, c: &str| m.values().filter(|x| *x == c).count();
        assert_eq!(
            count(&ours, "IFCMEMBER"),
            count(&open_classes, "IFCMEMBER"),
            "{stem}: IfcMember revit writes that open does not"
        );
        assert_eq!(
            count(&open_classes, "IFCBEAM") - count(&ours, "IFCBEAM"),
            *proxies,
            "{stem}: IfcBeam open writes that revit does not"
        );
    }
}

split!(framing_class_follows_the_convention, framing_class_follows_the_convention, s0 0, s1 1, s2 2, s3 3);

fn convention_flag_is_accepted_and_checked(k: usize, n: usize) {
    let Some(dir) = samples() else { return };
    let input = dir.join("racadvancedsampleproject.rvt");
    if !input.exists() {
        return;
    }
    let bin = env!("CARGO_BIN_EXE_rvt-ifc");
    let tmp = std::env::temp_dir().join(format!("rvt-ifc-convention-{}-{k}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    // One shard per run: revit, open, and the bogus convention.
    let runs = crate::shard(["revit", "open", "bogus"], k, n);
    for convention in runs.iter().copied().filter(|c| *c != "bogus") {
        let out = tmp.join(format!("{convention}.ifc"));
        let status = std::process::Command::new(bin)
            .arg(&input)
            .args(["--convention", convention, "-o"])
            .arg(&out)
            .output()
            .unwrap();
        assert!(
            status.status.success(),
            "{convention}: {}",
            String::from_utf8_lossy(&status.stderr)
        );
        assert!(std::fs::metadata(&out).unwrap().len() > 0);
    }
    if runs.contains(&"bogus") {
        let bad = std::process::Command::new(bin)
            .arg(&input)
            .args(["--convention", "bogus"])
            .output()
            .unwrap();
        assert!(!bad.status.success());
    }
    let _ = std::fs::remove_dir_all(&tmp);
}

split!(convention_flag_is_accepted_and_checked, convention_flag_is_accepted_and_checked, s0 0, s1 1, s2 2);
