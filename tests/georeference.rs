//! the georeference and the standard property-set names in every
//! output (IFC4 STEP, IFCX, OpenUSD, ThatOpen Fragments).
//!
//! Real file: `ARK_NordicLCA_Housing_Terrain-Concrete_BuildingPermit.rvt`
//! (Nordic LCA / BIM4LCA, Revit 2024; VTT, Granlund, Gravicon,
//! Insinööritoimisto Kallinen; CC BY-SA 4.0,
//! <https://cris.vtt.fi/en/publications/bim-files-to-download/>), from
//! `RVT_NORDIC_LCA_DIR`. Skips when absent. Expected values are the Revit
//! True North) and Revit's own IFC4 export of the model (`EPSG:3879`).

use rvt::RevitFile;
use rvt::ifc::{
    GraphOptions, IfcxGraph, RvtDocExporter, write_ifcx_graph, write_step_graph, write_usda_graph,
    write_usdc_graph,
};
use serde_json::Value;
use std::io::Read;
use std::path::PathBuf;

fn sample() -> Option<PathBuf> {
    let p = PathBuf::from(std::env::var_os("RVT_NORDIC_LCA_DIR")?)
        .join("ARK_NordicLCA_Housing_Terrain-Concrete_BuildingPermit.rvt");
    p.exists().then_some(p)
}

#[test]
fn nordic_housing_terrain_carries_its_georeference_and_set_names_everywhere() {
    let Some(path) = sample() else {
        eprintln!("skip: RVT_NORDIC_LCA_DIR not set");
        return;
    };
    let mut rf = RevitFile::open(&path).expect("open");

    // The decoder, against the Revit API.
    let g = rvt::georeference::read(&mut rf).expect("georeference");
    assert_eq!(g.location_name, "Internal");
    assert!((g.latitude - 1.0906860396912101f64.to_degrees()).abs() < 1e-9);
    assert!((g.longitude - 0.48499246059304696f64.to_degrees()).abs() < 1e-9);
    assert!((g.elevation - 335.0 * 0.3048).abs() < 1e-9);
    assert!((g.true_north - 1.0582236430374001f64.to_degrees()).abs() < 1e-9);
    let crs = g.crs.clone().expect("crs");
    assert_eq!(crs.epsg, Some(3879));
    for p in [g.survey_point.unwrap(), g.project_base_point.unwrap()] {
        assert!(p.internal.iter().chain(&p.shared).all(|v| v.abs() < 1e-9));
    }
    // survey vs project base point from `BasePoint::m_locationType`
    // (the API: 111426 `IsShared` true, category Survey Point; 111429 false,
    // Project Base Point).
    assert_eq!(g.survey_point.unwrap().element_id, 111426);
    assert_eq!(g.project_base_point.unwrap().element_id, 111429);

    let model = RvtDocExporter
        .export_with_diagnostics(&mut rf)
        .expect("export")
        .model;
    assert_eq!(model.georeference.as_ref(), Some(&g));
    let graph = IfcxGraph::from_model(&model, GraphOptions::default());

    // IFC4.
    let step = write_step_graph(&graph);
    assert!(step.contains("IFCPROJECTEDCRS('EPSG:3879','ETRS89 / ETRS-GK25FIN'"));
    assert!(step.contains("IFCMAPCONVERSION("));
    assert!(step.contains(".ELEMENT.,(62,29,30,144653),(27,47,16,875916),102.108,"));
    let ctx = step
        .lines()
        .find(|l| l.contains("IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model'"))
        .unwrap();
    assert!(!ctx.ends_with(",$);"), "true north is written: {ctx}");

    // IFCX: site, reference points, and a set name for every flat property.
    let ifcx: Value = serde_json::from_str(&write_ifcx_graph(&graph).expect("ifcx")).unwrap();
    let data = ifcx["data"].as_array().unwrap();
    let site = data
        .iter()
        .find(|n| n["attributes"].get("rvtopen::georeference").is_some())
        .expect("site georeference");
    assert!(site["attributes"]["ifc-src::attr::RefLatitude"].is_number());
    let points: Vec<&Value> = data
        .iter()
        .filter(|n| n["attributes"].get("epsg3879").is_some())
        .collect();
    assert_eq!(
        points.len(),
        2,
        "two reference points with EPSG:3879 coordinates"
    );
    assert!(ifcx["schemas"].get("epsg3879").is_some());
    let (mut flat, mut named) = (0, 0);
    for n in data {
        let Some(a) = n["attributes"].as_object() else {
            continue;
        };
        for k in a.keys() {
            let Some(p) = k.strip_prefix("bsi::ifc::prop::") else {
                continue;
            };
            if matches!(p, "Name" | "Description") {
                continue;
            }
            flat += 1;
            let sets = a[&format!("rvtopen::ifc::psetOf::{p}")].as_array().unwrap();
            assert!(!sets.is_empty());
            assert!(sets.iter().all(|s| {
                let s = s.as_str().unwrap();
                s.starts_with("Pset_") || s.starts_with("Qto_")
            }));
            named += 1;
        }
    }
    assert!(
        flat > 0 && named == flat,
        "{named} of {flat} flat properties named"
    );

    // OpenUSD.
    let usda = write_usda_graph(&graph).expect("usda");
    let head = &usda[..usda.find("\n)\n").unwrap()];
    assert!(head.contains("dictionary georeference = {"));
    assert!(head.contains("int64 epsg = 3879"));
    assert_eq!(usda.matches("matrix4d xformOp:transform").count(), 2 + 507);
    assert!(usda.contains("rvtopen:ifc:psetOf:"));
    // The binary layer: the reference points' transforms are specs there too
    let usdc = write_usdc_graph(&graph).expect("usdc");
    let ops = crate::usd_writer::read_usdc(&usdc)
        .iter()
        .filter(|(p, _)| p.ends_with(".xformOp:transform"))
        .count();
    assert_eq!(ops, 2 + 507, "xformOp:transform specs in the .usdc");

    // Fragments: no set named `bsi::ifc::prop`, georeference in metadata.
    let (frag, _) = rvt::ifc::frag_writer::write_frag_graph(&graph).expect("frag");
    let mut raw = Vec::new();
    flate2::read::ZlibDecoder::new(&frag[..])
        .read_to_end(&mut raw)
        .unwrap();
    let text = String::from_utf8_lossy(&raw);
    assert!(text.contains("\"georeference\":{"));
    assert!(text.contains("\"epsg\":3879"));
    assert!(!text.contains("bsi::ifc::prop"));
}

/// the two BIM4LCA Office As-Built models save two IFC export
/// configurations (`IFC4 Bim4LCA`, EPSG 3879; `IFC4 Reference View RAVA`,
/// EPSG 3857) in one partition, and nothing in the file marks either as the
/// one in use (Revit's IFC exporter keeps the selection in session only): the CRS stays
/// unknown, while the rest of the georeference and the base points (by
/// `m_locationType`: 111426 survey, 111429 project base point, as the API) are
/// read.
#[test]
fn nordic_office_as_built_leaves_two_saved_crs_unknown() {
    let Some(dir) = std::env::var_os("RVT_NORDIC_LCA_DIR") else {
        eprintln!("skip: RVT_NORDIC_LCA_DIR not set");
        return;
    };
    for name in [
        "ARK_NordicLCA_Office_Concrete_As-Built.rvt",
        "ARK_NordicLCA_Office_Timber_As-Built.rvt",
    ] {
        let path = PathBuf::from(&dir).join(name);
        if !path.exists() {
            eprintln!("skip: {name} absent");
            continue;
        }
        let mut rf = RevitFile::open(&path).expect("open");
        let g = rvt::georeference::read(&mut rf).expect("georeference");
        assert_eq!(g.crs, None, "{name}");
        assert_eq!(g.location_name, "Internal", "{name}");
        assert_eq!(g.survey_point.unwrap().element_id, 111426, "{name}");
        assert_eq!(g.project_base_point.unwrap().element_id, 111429, "{name}");
    }
}

/// `rvt-ifc` on `file` with `extra` arguments, every output into a fresh
/// temporary directory: (stderr, IFC4, IFCX, .usda, .usdc bytes, raw .frag).
fn rvt_ifc(
    file: &std::path::Path,
    tag: &str,
    extra: &[&str],
) -> (String, String, String, String, Vec<u8>, String) {
    let dir = std::env::temp_dir().join(format!("rvt-georef-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = |e: &str| dir.join(format!("m.{e}"));
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_rvt-ifc"))
        .arg(file)
        .arg("-o")
        .arg(p("ifc"))
        .args(["--ifcx", p("ifcx").to_str().unwrap()])
        .args(["--usda", p("usda").to_str().unwrap()])
        .args(["--usdc", p("usdc").to_str().unwrap()])
        .args(["--frag", p("frag").to_str().unwrap()])
        .args(extra)
        .output()
        .expect("spawn rvt-ifc");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let read = |e: &str| std::fs::read_to_string(p(e)).unwrap();
    let mut frag = Vec::new();
    flate2::read::ZlibDecoder::new(&std::fs::read(p("frag")).unwrap()[..])
        .read_to_end(&mut frag)
        .unwrap();
    let r = (
        String::from_utf8_lossy(&out.stderr).into_owned(),
        read("ifc"),
        read("ifcx"),
        read("usda"),
        std::fs::read(p("usdc")).unwrap(),
        String::from_utf8_lossy(&frag).into_owned(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    r
}

/// on an Office As-Built model (two saved configurations, 3879 and
/// 3857, CRS unknown), `rvt-ifc --epsg 3879` writes EPSG 3879 into all four
/// outputs, marked user-supplied; without it the CRS stays unknown and stderr
/// names both candidates.
#[test]
fn nordic_office_as_built_takes_the_crs_from_epsg_override() {
    let Some(dir) = std::env::var_os("RVT_NORDIC_LCA_DIR") else {
        eprintln!("skip: RVT_NORDIC_LCA_DIR not set");
        return;
    };
    let path = PathBuf::from(&dir).join("ARK_NordicLCA_Office_Concrete_As-Built.rvt");
    if !path.exists() {
        eprintln!("skip: Office Concrete As-Built absent");
        return;
    }
    let (err, step, ifcx, usda, usdc, frag) = rvt_ifc(&path, "with", &["--epsg", "3879"]);
    assert!(!err.contains("CRS left unknown"), "{err}");
    let user = "EPSG:3879, user-supplied (rvt-ifc --epsg), not stored in the file";
    assert!(
        step.contains(&format!("IFCPROJECTEDCRS('EPSG:3879','{user}'")),
        "IFC4 CRS"
    );
    assert!(step.contains("IFCMAPCONVERSION("));
    assert!(step.contains("IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.)"));
    let x: Value = serde_json::from_str(&ifcx).unwrap();
    let data = x["data"].as_array().unwrap();
    let site = data
        .iter()
        .find_map(|n| n["attributes"].get("rvtopen::georeference"))
        .expect("site georeference");
    assert_eq!(site["crs"]["epsg"], 3879);
    assert_eq!(site["crs"]["source"], "UserOverride");
    assert_eq!(
        data.iter()
            .filter(|n| n["attributes"].get("epsg3879").is_some())
            .count(),
        2
    );
    let head = &usda[..usda.find("\n)\n").unwrap()];
    assert!(head.contains("int64 epsg = 3879") && head.contains("\"UserOverride\""));
    assert!(
        usdc.windows(12).any(|w| w == b"UserOverride")
            && usdc.windows(9).any(|w| w == b"epsg3879:"),
        ".usdc carries the override and the epsg3879 reference points"
    );
    assert!(frag.contains("\"epsg\":3879") && frag.contains("\"source\":\"UserOverride\""));

    // Without the flag: unknown, and both candidates named on stderr.
    let (err, step, ..) = rvt_ifc(&path, "without", &[]);
    assert!(!step.contains("IFCPROJECTEDCRS"));
    assert!(
        err.contains("EPSG 3879")
            && err.contains("EPSG 3857")
            && err.contains("CRS left unknown; pass --epsg to choose"),
        "{err}"
    );
}

/// `--epsg` must be a positive integer.
#[test]
fn epsg_override_rejects_non_positive_codes() {
    for bad in ["0", "-3879", "abc", "3879.5"] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_rvt-ifc"))
            .args(["missing.rvt", &format!("--epsg={bad}")])
            .output()
            .expect("spawn rvt-ifc");
        assert!(!out.status.success(), "--epsg {bad} accepted");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("--epsg"),
            "{bad}"
        );
    }
}
