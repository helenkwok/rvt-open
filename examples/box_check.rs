//! the box check. "Everything is a box" was the first thing a
//! viewer of an export saw, while every test passed. This counts, per sample
//! file and Revit category, the elements an export draws as a box, and fails
//! when any count rises above the committed baseline.
//!
//! ```text
//! box_check <baseline.tsv>            # check against the baseline
//! box_check <baseline.tsv> --write    # write the measured counts as the baseline
//! ```
//!
//! Each file is converted as the browser build (rvt-wasm) converts it by
//! default: saved graphics, family geometry and saved materials on. Columns:
//! - `boxed`: elements whose glTF body is 12 triangles (a box: the shared
//!   unit cube or a 12-triangle mesh), as a viewer draws them;
//! - `fallback`: elements whose body is still the bounding-box fallback
//!   (`BodySource` the record box without a resolved profile, or the turned or
//! - `total`: elements with a body.
//!
//! A wall or slab that is a box is drawn as one, so `boxed` is not all
//! fallback; `fallback` is. **The target for `fallback` is zero**: the baseline was
//! This check only measures.
//!
//! The samples are the public ones the gate holds (the directories in
//! `SAMPLES`). With none of the variables set it skips; with one set, a file
//! the baseline names that is absent fails, so the check cannot shrink.
//! Files are converted in parallel, one thread each.

use rvt::RevitFile;
use rvt::convert::{ConvertOptions, export};
use rvt::ifc::IfcModel;
use rvt::ifc::entities::{IfcEntity, PropertyValue};
use rvt::ifc::export_content::REVIT_OTHER_SET;
use rvt::ifc::gltf::build_gltf;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// (directory variable, file): rac_basic, rac_advanced, the structural and MEP
/// samples and the school in 2024 and 2027, the Snowdon Towers parts, and
/// Einhoven (2023).
const SAMPLES: &[(&str, &str)] = &[
    ("RVT_REVIT2024_SAMPLES_DIR", "racbasicsampleproject.rvt"),
    ("RVT_REVIT2024_SAMPLES_DIR", "racadvancedsampleproject.rvt"),
    ("RVT_REVIT2024_SAMPLES_DIR", "rstbasicsampleproject.rvt"),
    ("RVT_REVIT2024_SAMPLES_DIR", "rstadvancedsampleproject.rvt"),
    ("RVT_REVIT2024_SAMPLES_DIR", "rmebasicsampleproject.rvt"),
    ("RVT_REVIT2024_SAMPLES_DIR", "rmeadvancedsampleproject.rvt"),
    ("RVT_REVIT2024_SAMPLES_DIR", "Technicalschoolcurrentm.rvt"),
    (
        "RVT_REVIT2024_SAMPLES_DIR",
        "Snowdon Towers Sample Architectural.rvt",
    ),
    (
        "RVT_REVIT2024_SAMPLES_DIR",
        "Snowdon Towers Sample Structural.rvt",
    ),
    (
        "RVT_REVIT2024_SAMPLES_DIR",
        "Snowdon Towers Sample HVAC.rvt",
    ),
    (
        "RVT_REVIT2024_SAMPLES_DIR",
        "Snowdon Towers Sample Plumbing.rvt",
    ),
    (
        "RVT_REVIT2024_SAMPLES_DIR",
        "Snowdon Towers Sample Electrical.rvt",
    ),
    (
        "RVT_REVIT2024_SAMPLES_DIR",
        "Snowdon Towers Sample Facades.rvt",
    ),
    (
        "RVT_REVIT2024_SAMPLES_DIR",
        "Snowdon Towers Sample Site.rvt",
    ),
    (
        "RVT_AUTODESK_SAMPLES_DIR",
        "2027/rac_basic_sample_project.rvt",
    ),
    (
        "RVT_AUTODESK_SAMPLES_DIR",
        "2027/rac_advanced_sample_project.rvt",
    ),
    (
        "RVT_AUTODESK_SAMPLES_DIR",
        "2027/rst_basic_sample_project.rvt",
    ),
    (
        "RVT_AUTODESK_SAMPLES_DIR",
        "2027/rme_basic_sample_project.rvt",
    ),
    // releases before 2023 draw from their saved graphics too.
    (
        "RVT_AUTODESK_SAMPLES_DIR",
        "2016/rac_basic_sample_project.rvt",
    ),
    (
        "RVT_AUTODESK_SAMPLES_DIR",
        "2019/rac_advanced_sample_project.rvt",
    ),
    (
        "RVT_AUTODESK_SAMPLES_DIR",
        "2021/rme_basic_sample_project.rvt",
    ),
    // The MIT 2023 project whose curtain panels were boxes over walls
    // (tests/fixtures/project-counts/revit-ifc5-einhoven.json).
    ("RVT_PROJECT_CORPUS_DIR", "Revit_IFC5_Einhoven.rvt"),
];

/// (boxed, fallback, total) per category.
type Counts = BTreeMap<String, [usize; 3]>;

/// Per category, each fallback element's name and `BodySource`, for the
/// failure message.
type Fallbacks = BTreeMap<String, Vec<String>>;

/// A file's measurement: its counts, its fallback elements and the export's
/// notes.
type Measured = (Counts, Fallbacks, Vec<String>);

/// A file's result: its name, its measurement (`None` when absent) and seconds taken.
type Outcome = (String, Option<Result<Measured, String>>, f64);

fn counts(model: &IfcModel) -> (Counts, Fallbacks) {
    let mut category: BTreeMap<usize, String> = BTreeMap::new();
    for entity in &model.entities {
        if let IfcEntity::ElementPropertySet { element, set } = entity
            && set.name == REVIT_OTHER_SET
            && let Some(PropertyValue::Label(c)) = set
                .properties
                .iter()
                .find(|p| p.name == "Category")
                .map(|p| &p.value)
        {
            category.insert(*element, c.clone());
        }
    }
    let (doc, _) = build_gltf(model);
    let triangles = |mesh: usize| -> usize {
        doc.meshes[mesh]
            .primitives
            .iter()
            .filter_map(|p| p.indices.map(|i| doc.accessors[i].count / 3))
            .sum()
    };
    let mut out = Counts::new();
    let mut fallbacks = Fallbacks::new();
    for node in &doc.nodes {
        let (Some(mesh), Some(extras)) = (node.mesh, &node.extras) else {
            continue;
        };
        let index = extras.entity_index;
        let fallback = model
            .entities
            .get(index)
            .is_some_and(rvt::ifc::quantities::is_box_fallback);
        let name = category
            .get(&index)
            .cloned()
            .unwrap_or_else(|| format!("({})", extras.ifc_type));
        if fallback && let Some(entity) = model.entities.get(index) {
            fallbacks
                .entry(name.clone())
                .or_default()
                .push(describe(entity));
        }
        let row = out.entry(name).or_default();
        row[0] += usize::from(triangles(mesh) == 12);
        row[1] += usize::from(fallback);
        row[2] += 1;
    }
    (out, fallbacks)
}

/// Prints the saved graphics of `ids` in `path` as the saved-mesh pass reads
/// them: status, primitives, branches left out and the first diagnostics.
fn saved_graphics(path: &std::path::Path, ids: &BTreeSet<u64>) {
    use rvt::research::native_document::Options;
    let Ok(mut rf) = RevitFile::open(path) else {
        return;
    };
    let options = Options {
        selected_ids: ids.clone(),
        ..Options::default()
    };
    match rvt::research::native_saved_scene::extract(&mut rf, &options) {
        Ok(scene) => {
            for e in &scene.elements {
                println!(
                    "  saved graphics {}: {}, {} primitives, {} non-surface branches left out, {:?}",
                    e.id,
                    e.status,
                    e.meshes.primitives.len(),
                    e.meshes.excluded_non_surface_branches,
                    e.meshes.diagnostics.iter().take(3).collect::<Vec<_>>()
                );
            }
        }
        Err(err) => println!("  saved graphics not read: {err}"),
    }
}

/// An element's name and `BodySource`, as the failure message lists it.
fn describe(entity: &IfcEntity) -> String {
    let IfcEntity::BuildingElement {
        name, property_set, ..
    } = entity
    else {
        return String::from("(not a building element)");
    };
    let source = property_set
        .as_ref()
        .and_then(|set| {
            set.properties.iter().find_map(|p| match &p.value {
                PropertyValue::Text(t) if p.name == "BodySource" => Some(t.as_str()),
                _ => None,
            })
        })
        .unwrap_or("-");
    format!("{name} ({source})")
}

fn measure(path: &PathBuf) -> Result<Measured, String> {
    let mut rf = RevitFile::open(path).map_err(|e| format!("open: {e}"))?;
    let opts = ConvertOptions {
        family_geometry: true,
        saved_meshes: true,
        saved_materials: true,
        ..ConvertOptions::default()
    };
    let exported = export(&mut rf, &opts).map_err(|e| format!("export: {e}"))?;
    let (counts, fallbacks) = counts(&exported.model);
    Ok((counts, fallbacks, exported.notes))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(baseline_path) = args.first().map(PathBuf::from) else {
        eprintln!("usage: box_check <baseline.tsv> [--write]");
        std::process::exit(2);
    };
    let write = args.iter().any(|a| a == "--write");
    let files: Vec<(String, Option<PathBuf>)> = SAMPLES
        .iter()
        .map(|(var, file)| {
            let path = std::env::var_os(var).map(|d| PathBuf::from(d).join(file));
            (file.to_string(), path)
        })
        .collect();
    if files.iter().all(|(_, p)| p.is_none()) {
        println!("box check: skipped, no sample directory variable is set");
        return;
    }
    let start = std::time::Instant::now();
    // BOX_CHECK_JOBS caps how many files convert at once (memory: small CI runners);
    // unset = all files at once.
    let jobs = std::env::var("BOX_CHECK_JOBS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(files.len().max(1));
    let mut results: Vec<Outcome> = Vec::new();
    for chunk in files.chunks(jobs) {
        results.extend(std::thread::scope(|s| {
            let handles: Vec<_> = chunk
                .iter()
                .map(|(name, path)| {
                    s.spawn(move || {
                        let t = std::time::Instant::now();
                        let r = path.as_ref().filter(|p| p.exists()).map(measure);
                        (name.clone(), r, t.elapsed().as_secs_f64())
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().expect("box check thread"))
                .collect::<Vec<_>>()
        }));
    }
    let mut measured: BTreeMap<(String, String), [usize; 3]> = BTreeMap::new();
    let mut failures = Vec::new();
    for (file, result, secs) in &results {
        match result {
            None => println!("{file}: absent"),
            Some(Err(e)) => failures.push(format!("{file}: {e}")),
            Some(Ok((c, _, _))) => {
                let [b, f, t] = c
                    .values()
                    .fold([0; 3], |a, r| [a[0] + r[0], a[1] + r[1], a[2] + r[2]]);
                println!("{file}: boxed {b} / {t}, fallback {f} / {t} ({secs:.1} s)");
                for (category, row) in c {
                    if row[0] + row[1] > 0 {
                        println!(
                            "  {category}: boxed {} fallback {} / {}",
                            row[0], row[1], row[2]
                        );
                    }
                    measured.insert((file.clone(), category.clone()), *row);
                }
            }
        }
    }
    println!(
        "box check: {} files in {:.1} s",
        results.len(),
        start.elapsed().as_secs_f64()
    );
    if write {
        let mut out = String::from(
            "# Box check baseline: per sample file and Revit category, elements drawn as a\n\
             # 12-triangle box (boxed), still on the bounding-box fallback (fallback), with a body (total).\n\
             # The gate fails when boxed or fallback rises above these. Target for fallback: zero.\n\
             file\tcategory\tboxed\tfallback\ttotal\n",
        );
        for ((file, category), [b, f, t]) in &measured {
            out.push_str(&format!("{file}\t{category}\t{b}\t{f}\t{t}\n"));
        }
        std::fs::write(&baseline_path, out).expect("write baseline");
        println!("baseline written: {}", baseline_path.display());
    } else {
        let text = std::fs::read_to_string(&baseline_path).expect("read baseline");
        let mut baseline: BTreeMap<(String, String), [usize; 2]> = BTreeMap::new();
        for line in text.lines().filter(|l| !l.starts_with('#')).skip(1) {
            let f: Vec<&str> = line.split('\t').collect();
            let n = |i: usize| f[i].parse::<usize>().expect("baseline count");
            baseline.insert((f[0].to_string(), f[1].to_string()), [n(2), n(3)]);
        }
        let present = |file: &str| {
            results
                .iter()
                .any(|(f, r, _)| f == file && matches!(r, Some(Ok(_))))
        };
        for (file, _) in baseline.keys() {
            if !present(file) && !failures.iter().any(|m| m.starts_with(file.as_str())) {
                let e = format!("{file}: in the baseline but not measured (absent or failed)");
                if !failures.contains(&e) {
                    failures.push(e);
                }
            }
        }
        for ((file, category), [b, f, _]) in &measured {
            let [bb, bf] = baseline
                .get(&(file.clone(), category.clone()))
                .copied()
                .unwrap_or([0, 0]);
            if *b > bb || *f > bf {
                failures.push(format!(
                    "{file} {category}: boxed {b} (baseline {bb}), fallback {f} (baseline {bf})"
                ));
                // What is on the fallback, and what the export's passes said,
                // so a regression shows its cause in the log.
                if let Some((_, Some(Ok((_, fallbacks, notes))), _)) =
                    results.iter().find(|(name, _, _)| name == file)
                {
                    for element in fallbacks.get(category).into_iter().flatten().take(20) {
                        println!("  {file} {category} fallback: {element}");
                    }
                    // Each one's saved graphics, as the saved-mesh pass reads them.
                    let ids: BTreeSet<u64> = fallbacks
                        .get(category)
                        .into_iter()
                        .flatten()
                        .take(20)
                        .filter_map(|e| e.split(" (").next()?.rsplit(':').next()?.parse().ok())
                        .collect();
                    if let Some(path) = files
                        .iter()
                        .find(|(name, _)| name == file)
                        .and_then(|(_, path)| path.as_ref())
                    {
                        saved_graphics(path, &ids);
                    }
                    for note in notes {
                        println!("  {file} note: {note}");
                    }
                }
            }
        }
    }
    if failures.is_empty() {
        println!("box check: PASS");
    } else {
        for f in &failures {
            println!("box check FAIL: {f}");
        }
        std::process::exit(1);
    }
}
