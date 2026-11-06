//! Revit 2016 project files are read as 2018's (same 32-bit
//! element-record frame and Level records, each release with its own schema
//! tags).
//!
//! Runs against `RVT_REVIT_2016_DIR`: Autodesk's Revit 2016 sample projects
//! (`rac_basic_sample_project.rvt`, `rac_advanced_sample_project.rvt`,
//! `rst_basic_sample_project.rvt`, from Autodesk's download host, see
//! `research/autodesk-sample-projects.tsv`) and, under `same-model-2018/`,
//! the same three samples as Autodesk published them for Revit 2018. Skips
//! element. Now each 2016 sample exports the storeys and elements its 2018
//! copy does, under the same GlobalIds.
//!
//! Revit 2017 the same way, against `RVT_REVIT_2017_DIR` (the
//! same three samples as Autodesk published them for 2017, with the 2018
//! copies under `same-model-2018/`).
//!
//! Revit 2020 and 2021 the same way, against
//! `RVT_AUTODESK_SAMPLES_DIR` (Autodesk's sample projects by release, one
//! folder per year: `2019/`, `2020/`, `2021/`, `2022/`). Every 2020 sample
//! must export what its 2019 copy does, and every 2021 sample what its 2022
//! copy does, under the same GlobalIds.
//! Revit 2014 and 2015 against `RVT_REVIT_2014_DIR` /
//! `RVT_REVIT_2015_DIR` (Autodesk's seven samples of that release, with the
//! 2016 copies under `same-model-2016/`); Revit 2015 against Revit's own IFC
//! export of the same project (`RVT_REVIT_2015_IFC_DIR`: `CIRCLES.rvt` and
//! `CIRCLES.ifc`, GrabCAD "circles-1"); Revit 2011 and 2012 projects in
//! `RVT_REVIT_2011_2012_DIR` (GrabCAD, no Revit answer key: counts only).

use rvt::RevitFile;
use rvt::ifc::{GraphOptions, IfcxGraph, RvtDocExporter, write_step_graph};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

const ELEMENTS: &[&str] = &[
    "IFCBUILDINGSTOREY",
    "IFCWALL",
    "IFCCURTAINWALL",
    "IFCSLAB",
    "IFCCOLUMN",
    "IFCDOOR",
    "IFCWINDOW",
    "IFCBEAM",
    "IFCROOF",
    "IFCSPACE",
    "IFCSTAIR",
    "IFCCOVERING",
    "IFCMEMBER",
    "IFCPLATE",
    "IFCRAILING",
    "IFCBUILDINGELEMENTPROXY",
    // planting and entourage (RPC) are IfcGeographicElement.
    "IFCGEOGRAPHICELEMENT",
];

/// (sample, [storeys, walls, slabs, columns, doors, windows, beams]): the
/// counts of the 2018 copy's export, which the 2016 and 2017 files' must
/// equal.
const EXPECTED: &[(&str, [usize; 7])] = &[
    ("rac_basic_sample_project", [6, 47, 11, 3, 16, 17, 0]),
    (
        "rac_advanced_sample_project",
        [5, 146, 17, 176, 100, 24, 37],
    ),
    ("rst_basic_sample_project", [0, 9, 5, 30, 0, 0, 370]),
];

fn dir(var: &str) -> Option<PathBuf> {
    let dir = std::env::var_os(var).map(PathBuf::from);
    if dir.is_none() {
        eprintln!("skipping: {var} is not set");
    }
    dir
}

/// Element GlobalIds by IFC class of the default export of `path`.
fn export(path: &PathBuf) -> BTreeMap<String, BTreeSet<String>> {
    let mut rf = RevitFile::open(path).expect("open");
    let model = RvtDocExporter
        .export_with_diagnostics(&mut rf)
        .expect("export")
        .model;
    classes(&write_step_graph(&IfcxGraph::from_model(
        &model,
        GraphOptions {
            meshes: false,
            shared_shapes: false,
        },
    )))
}

/// Element GlobalIds by IFC class of a STEP file's text.
fn classes(step: &str) -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for line in step.lines() {
        let Some((_, body)) = line.split_once('=') else {
            continue;
        };
        let Some((class, args)) = body.split_once('(') else {
            continue;
        };
        if !ELEMENTS.contains(&class.trim()) {
            continue;
        }
        let guid = args.split(',').next().unwrap_or("").trim_matches('\'');
        out.entry(class.trim().to_string())
            .or_default()
            .insert(guid.to_string());
    }
    out
}

#[test]
fn revit_2016_samples_export_their_2018_copies_elements() {
    check("RVT_REVIT_2016_DIR", 2016);
}

#[test]
fn revit_2017_samples_export_their_2018_copies_elements() {
    check("RVT_REVIT_2017_DIR", 2017);
}

fn check(var: &str, release: u32) {
    let Some(dir) = dir(var) else { return };
    for &(sample, expected) in EXPECTED {
        let path = dir.join(format!("{sample}.rvt"));
        if !path.exists() {
            eprintln!("skipping: {sample}.rvt absent");
            continue;
        }
        let got = export(&path);
        let n = |c: &str| got.get(c).map_or(0, BTreeSet::len);
        assert_eq!(
            [
                n("IFCBUILDINGSTOREY"),
                n("IFCWALL"),
                n("IFCSLAB"),
                n("IFCCOLUMN"),
                n("IFCDOOR"),
                n("IFCWINDOW"),
                n("IFCBEAM"),
            ],
            expected,
            "{sample} ({release}): storeys, walls, slabs, columns, doors, windows, beams"
        );
        let copy = dir.join("same-model-2018").join(format!("{sample}.rvt"));
        if copy.exists() {
            assert_eq!(got, export(&copy), "{sample}: {release} vs its 2018 copy");
        } else {
            eprintln!("skipping the 2018 comparison: same-model-2018/{sample}.rvt absent");
        }
    }
}

/// (sample, element GlobalIds of the 2014 or 2015 export, of the
/// 2016 copy's). Where both hold an element it has the same IFC class, and
/// the shared count is the smaller set: the differences are the files' own
/// content (rac_basic 2014/2015 hold one more wall record or a few more
/// curtain panels and mullions than 2016; Technical_school 2014 fewer).
/// (3,318 before). The 2015 saves and the 2016 copies of rac_advanced and
/// Technical_school write 3 Areas as spaces (Revit 2015 to 2023), which
/// the 2014 saves do not. rac_basic's 14 rooms sit outside the export phase
/// and are left out on the 2015 save and the 2016 copy, not on 2014.
const OLD_SAMPLES: &[(u32, &str, usize, usize)] = &[
    (2014, "rac_basic_sample_project", 400, 379),
    (2014, "rac_advanced_sample_project", 5466, 5469),
    (2014, "rst_advanced_sample_project", 616, 616),
    (2014, "rme_basic_sample_project", 3405, 3405),
    (2014, "rme_advanced_sample_project", 3405, 3405),
    (2014, "Technical_school-current_m", 5398, 5469),
    (2015, "rac_basic_sample_project", 385, 379),
    (2015, "rac_advanced_sample_project", 5469, 5469),
    (2015, "rst_advanced_sample_project", 616, 616),
    (2015, "rme_basic_sample_project", 3405, 3405),
    (2015, "rme_advanced_sample_project", 3405, 3405),
    (2015, "Technical_school-current_m", 5469, 5469),
];

fn total(set: &BTreeMap<String, BTreeSet<String>>) -> usize {
    set.values().map(BTreeSet::len).sum()
}

fn revit_2014_samples_export_their_2016_copies_elements(k: usize, n: usize) {
    check_old("RVT_REVIT_2014_DIR", 2014, k, n);
}

split!(revit_2014_samples_export_their_2016_copies_elements, revit_2014_samples_export_their_2016_copies_elements, s0 0, s1 1, s2 2, s3 3, s4 4, s5 5, s6 6);

fn revit_2015_samples_export_their_2016_copies_elements(k: usize, n: usize) {
    check_old("RVT_REVIT_2015_DIR", 2015, k, n);
}

split!(revit_2015_samples_export_their_2016_copies_elements, revit_2015_samples_export_their_2016_copies_elements, s0 0, s1 1, s2 2, s3 3, s4 4, s5 5, s6 6);

/// Shard `k` of `n` of the checks: one per sample of `release`, then rst_basic.
fn check_old(var: &str, release: u32, k: usize, n: usize) {
    let Some(dir) = dir(var) else { return };
    let jobs = OLD_SAMPLES
        .iter()
        .filter(|s| s.0 == release)
        .map(Some)
        .chain([None]);
    for job in crate::shard(jobs, k, n) {
        let Some(&(_, sample, ours, key)) = job else {
            check_rst_basic(&dir, release);
            continue;
        };
        let path = dir.join(format!("{sample}.rvt"));
        let copy = dir.join("same-model-2016").join(format!("{sample}.rvt"));
        if !path.exists() || !copy.exists() {
            eprintln!("skipping: {sample}.rvt or its 2016 copy absent");
            continue;
        }
        let (got, want) = (export(&path), export(&copy));
        let shared: usize = got
            .iter()
            .map(|(class, ids)| want.get(class).map_or(0, |w| ids.intersection(w).count()))
            .sum();
        assert_eq!(
            (total(&got), total(&want), shared),
            (ours, key, ours.min(key)),
            "{sample} ({release}): element GlobalIds (ours, 2016 copy, shared by class)"
        );
    }
}

/// rst_basic is a different model in 2016 (no shared GlobalId): its 2014 and
/// 2015 saves export the same 239 elements.
fn check_rst_basic(dir: &std::path::Path, release: u32) {
    let path = dir.join("rst_basic_sample_project.rvt");
    if path.exists() {
        let got = export(&path);
        let n = |c: &str| got.get(c).map_or(0, BTreeSet::len);
        assert_eq!(
            [
                n("IFCBUILDINGSTOREY"),
                n("IFCWALL"),
                n("IFCSLAB"),
                n("IFCCOLUMN"),
                n("IFCBEAM")
            ],
            [4, 4, 2, 70, 137],
            "rst_basic ({release})"
        );
        assert_eq!(total(&got), 239, "rst_basic ({release})");
    }
}

/// Revit 2015's own IFC2X3 export of CIRCLES (Exporter 2015.0.2014.0323):
/// these classes' GlobalIds are exactly Revit's.
const CIRCLES_SAME: &[(&str, usize)] = &[
    ("IFCMEMBER", 573),
    ("IFCPLATE", 72),
    ("IFCBEAM", 43),
    ("IFCBUILDINGELEMENTPROXY", 28),
    ("IFCWINDOW", 15),
    ("IFCCOVERING", 11),
    ("IFCROOF", 8),
    ("IFCDOOR", 6),
];

#[test]
fn revit_2015_circles_matches_revits_own_ifc() {
    let Some(dir) = dir("RVT_REVIT_2015_IFC_DIR") else {
        return;
    };
    let (rvt, ifc) = (dir.join("CIRCLES.rvt"), dir.join("CIRCLES.ifc"));
    if !rvt.exists() || !ifc.exists() {
        eprintln!("skipping: CIRCLES.rvt or CIRCLES.ifc absent");
        return;
    }
    let got = export(&rvt);
    let key = classes(&std::fs::read_to_string(&ifc).expect("read CIRCLES.ifc"));
    for &(class, n) in CIRCLES_SAME {
        assert_eq!(got.get(class).map_or(0, BTreeSet::len), n, "{class}");
        assert_eq!(
            got.get(class),
            key.get(class),
            "{class}: GlobalIds vs Revit's IFC"
        );
    }
}

/// Revit 2011 and 2012 projects (GrabCAD; no Revit export of
/// them exists). (file stem, release, [storeys, walls, doors, windows]):
/// the storeys are every placed Level, the walls every distinct wall record
/// that is not a curtain wall,
/// the doors and windows every distinct `FamilyInstance` record of the
/// category (the others are `FamilySymbol` types).
const PRE_2014: &[(&str, u32, [usize; 4])] = &[
    ("Mosque-Baitul Aman_Render_2011", 2011, [3, 65, 4, 18]),
    // 8: Revit 2012 writes no Building Story setting, so all 8 placed Levels
    ("SEM 2 HOUSE DESIGN", 2012, [8, 73, 16, 51]),
];

#[test]
fn revit_2011_and_2012_projects_export_levels_and_elements() {
    let Some(dir) = dir("RVT_REVIT_2011_2012_DIR") else {
        return;
    };
    for &(stem, release, expected) in PRE_2014 {
        let path = dir.join(format!("{stem}.rvt"));
        if !path.exists() {
            eprintln!("skipping: {stem}.rvt absent");
            continue;
        }
        let mut rf = RevitFile::open(&path).expect("open");
        assert_eq!(
            rf.basic_file_info().expect("info").version,
            release,
            "{stem}"
        );
        let got = export(&path);
        let n = |c: &str| got.get(c).map_or(0, BTreeSet::len);
        assert_eq!(
            [
                n("IFCBUILDINGSTOREY"),
                n("IFCWALL"),
                n("IFCDOOR"),
                n("IFCWINDOW")
            ],
            expected,
            "{stem} ({release}): storeys, walls, doors, windows"
        );
    }
}

/// Revit 2010 and 2013 projects from Paul Aubin's published
/// course datasets (`RVT_REVIT_2010_2013_DIR`). No Revit export of them
/// exists. (file stem, release, [storeys, walls, doors, windows]) as the
/// measurement found them; on main before it, none of them exported anything.
const REVIT_2010_2013: &[(&str, u32, [usize; 4])] = &[
    // 2010 and 2012 write no Building Story setting; the 2012 files
    // and the 2010 one's third Level read as "not a storey" before. Their
    // storeys are those of the 2011/2013 saves of the same models.
    ("Detailing Sandbox 2010", 2010, [3, 11, 4, 11]),
    ("Detailing Sandbox 2012", 2012, [3, 17, 4, 14]),
    ("100 Sandbox", 2012, [4, 9, 5, 0]),
    ("Workset-TeamX", 2012, [2, 22, 13, 26]),
    ("Stair and Railing Sandbox", 2013, [7, 66, 6, 0]),
    ("Chapel", 2013, [3, 8, 7, 14]),
    ("04_Timeline", 2013, [3, 30, 8, 21]),
    ("100 Sandbox (01)", 2013, [4, 9, 6, 0]),
];

/// Same models saved by two releases (older stem, newer stem): every element
/// GlobalId the older save exports is in the newer save's export, with the
/// same IFC class, and the storeys are the same. GlobalIds come
/// from the elements' own unique ids, which an upgrade keeps.
const REVIT_2010_2013_COPIES: &[(&str, &str)] = &[
    ("Detailing Sandbox 2010", "Detailing Sandbox 2011"),
    ("100 Sandbox", "100 Sandbox (01)"),
];

/// saves of one model whose storeys (GlobalIds) are the same.
/// The 2012 Detailing Sandbox is a later edit of the 2011 one (one proxy of
/// 2011 is gone), so only its storeys are compared.
const REVIT_2012_STOREY_COPIES: &[(&str, &str)] =
    &[("Detailing Sandbox 2012", "Detailing Sandbox 2011")];

fn revit_2010_and_2013_projects_export_levels_and_elements(k: usize, n: usize) {
    let Some(dir) = dir("RVT_REVIT_2010_2013_DIR") else {
        return;
    };
    // One check per model, then the workshared file, the storey copies and
    // the copies, each a job of its own.
    enum Job {
        Model(&'static (&'static str, u32, [usize; 4])),
        Workshared,
        Storeys(&'static (&'static str, &'static str)),
        Copies(&'static (&'static str, &'static str)),
    }
    let jobs = (REVIT_2010_2013.iter().map(Job::Model))
        .chain([Job::Workshared])
        .chain(REVIT_2012_STOREY_COPIES.iter().map(Job::Storeys))
        .chain(REVIT_2010_2013_COPIES.iter().map(Job::Copies));
    for job in crate::shard(jobs, k, n) {
        match job {
            Job::Model(&(stem, release, expected)) => {
                let path = dir.join(format!("{stem}.rvt"));
                if !path.exists() {
                    eprintln!("skipping: {stem}.rvt absent");
                    continue;
                }
                let mut rf = RevitFile::open(&path).expect("open");
                assert_eq!(
                    rf.basic_file_info().expect("info").version,
                    release,
                    "{stem}"
                );
                let got = export(&path);
                let n = |c: &str| got.get(c).map_or(0, BTreeSet::len);
                assert_eq!(
                    [
                        n("IFCBUILDINGSTOREY"),
                        n("IFCWALL"),
                        n("IFCDOOR"),
                        n("IFCWINDOW")
                    ],
                    expected,
                    "{stem} ({release}): storeys, walls, doors, windows"
                );
            }
            // A workshared Revit 2012 file whose central path names 2011 reads as
            // 2012, the release its schema belongs to.
            Job::Workshared => {
                let workshared = dir.join("Workset-TeamX.rvt");
                if workshared.exists() {
                    let mut rf = RevitFile::open(&workshared).expect("open");
                    assert_eq!(rf.basic_file_info().expect("info").version, 2012);
                }
            }
            Job::Storeys(&(a, b)) => {
                let (pa, pb) = (dir.join(format!("{a}.rvt")), dir.join(format!("{b}.rvt")));
                if !pa.exists() || !pb.exists() {
                    eprintln!("skipping: {a} / {b} absent");
                    continue;
                }
                let (ea, eb) = (export(&pa), export(&pb));
                assert!(
                    ea.get("IFCBUILDINGSTOREY").is_some_and(|s| !s.is_empty()),
                    "{a}: storeys"
                );
                assert_eq!(
                    ea.get("IFCBUILDINGSTOREY"),
                    eb.get("IFCBUILDINGSTOREY"),
                    "{a} / {b}: storeys"
                );
            }
            Job::Copies(&(older, newer)) => {
                let (a, b) = (
                    dir.join(format!("{older}.rvt")),
                    dir.join(format!("{newer}.rvt")),
                );
                if !a.exists() || !b.exists() {
                    eprintln!("skipping: {older} / {newer} absent");
                    continue;
                }
                let (old, new) = (export(&a), export(&b));
                assert!(
                    old.values().map(BTreeSet::len).sum::<usize>() > 0,
                    "{older} exports elements"
                );
                assert_eq!(
                    old.get("IFCBUILDINGSTOREY"),
                    new.get("IFCBUILDINGSTOREY"),
                    "{older} -> {newer}: storeys"
                );
                for (class, ids) in &old {
                    if class == "IFCBUILDING" {
                        continue;
                    }
                    let missing: Vec<_> = ids
                        .iter()
                        .filter(|id| !new.get(class).is_some_and(|n| n.contains(*id)))
                        .collect();
                    assert!(
                        missing.is_empty(),
                        "{older} -> {newer}: {class} {missing:?}"
                    );
                }
            }
        }
    }
}

split!(revit_2010_and_2013_projects_export_levels_and_elements, revit_2010_and_2013_projects_export_levels_and_elements, s0 0, s1 1, s2 2, s3 3, s4 4, s5 5, s6 6, s7 7, s8 8, s9 9, s10 10, s11 11);

fn revit_2020_and_2021_samples_export_their_neighbours_elements(k: usize, n: usize) {
    let Some(root) = dir("RVT_AUTODESK_SAMPLES_DIR") else {
        return;
    };
    // Every (release, its neighbour, sample), checked by shard.
    let mut jobs = Vec::new();
    for (release, key) in [(2020, 2019), (2021, 2022)] {
        let Ok(entries) = std::fs::read_dir(root.join(release.to_string())) else {
            eprintln!("skipping: {release}/ absent");
            continue;
        };
        let mut samples: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "rvt"))
            .collect();
        samples.sort();
        jobs.extend(samples.into_iter().map(|path| (release, key, path)));
    }
    {
        for (release, key, path) in crate::shard(jobs, k, n) {
            let copy = root
                .join(key.to_string())
                .join(path.file_name().expect("name"));
            if !copy.exists() {
                eprintln!("skipping: {} has no {key} copy", path.display());
                continue;
            }
            let got = export(&path);
            let mut expected = export(&copy);
            // rac_advanced's 2021 save writes the Areas its 2022 copy writes
            // from the plan topology alone (marked unchecked), so none is
            // its copy's only. Held exactly so a change is seen.
            if release == 2021
                && path
                    .file_stem()
                    // named with or without underscores, as the samples' folders name it
                    .is_some_and(|s| {
                        s.to_string_lossy().replace('_', "") == "racadvancedsampleproject"
                    })
            {
                let ours = got.get("IFCSPACE").cloned().unwrap_or_default();
                if let Some(spaces) = expected.get_mut("IFCSPACE") {
                    let before = spaces.len();
                    spaces.retain(|id| ours.contains(id));
                    assert_eq!(
                        before - spaces.len(),
                        0,
                        "rac_advanced 2022: Areas only its copy writes"
                    );
                }
            }
            assert!(
                expected.values().map(BTreeSet::len).sum::<usize>() > 0,
                "{}: the {key} copy exports elements",
                path.display()
            );
            assert_eq!(
                got,
                expected,
                "{}: {release} vs its {key} copy",
                path.display()
            );
        }
    }
}

split!(revit_2020_and_2021_samples_export_their_neighbours_elements, revit_2020_and_2021_samples_export_their_neighbours_elements, s0 0, s1 1, s2 2, s3 3, s4 4, s5 5, s6 6, s7 7, s8 8, s9 9, s10 10, s11 11, s12 12, s13 13, s14 14);
