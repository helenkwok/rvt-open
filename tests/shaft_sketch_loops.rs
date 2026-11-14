//! A shaft opening cuts the floors and ceilings it crosses along its sketch's
//! loops, not along its record box.
//!
//! The public sample `racadvancedsampleproject.rvt` (Revit 2024 to 2027) has
//! two shaft openings sketched as several rectangles: 156935, four in a box of
//! 189.5 by 176.1 ft, and 157484, three in a box of 9.3 by 98.0 ft. Cut as
//! their boxes they took 71% of the hollow-core floors 139803 and 139829
//! (4,800.9 ft² left of each) and left the ceilings 242937 and 247617 whole
//! (Revit's export writes an opening only in floors). Revit's own Area for
//! each, read through its API on the 2024 file, is the plan area left once the
//! loops are cut: 16,746.18, 16,684.36, 4,843.56 and 3,833.69 ft².
//!
//! Runs against `RVT_AUTODESK_SAMPLES_DIR` (`<year>/racadvancedsampleproject.rvt`);
//! a release whose file is absent is skipped.

use rvt::RevitFile;
use rvt::convert::{ConvertOptions, export};
use rvt::ifc::entities::{IfcEntity, ProfileDef};
use std::path::PathBuf;

/// Revit's Area of each element, ft².
const REVIT_AREA_FT2: [(&str, f64); 4] = [
    ("139803", 16746.177),
    ("139829", 16684.365),
    ("242937", 4843.556),
    ("247617", 3833.694),
];

fn area(points: &[(f64, f64)]) -> f64 {
    let n = points.len();
    (0..n)
        .map(|i| {
            let (a, b) = (points[i], points[(i + 1) % n]);
            a.0 * b.1 - b.0 * a.1
        })
        .sum::<f64>()
        .abs()
        / 2.0
}

#[test]
fn rac_advanced_shafts_cut_their_loops() {
    let Some(dir) = std::env::var_os("RVT_AUTODESK_SAMPLES_DIR").map(PathBuf::from) else {
        eprintln!("skipping: RVT_AUTODESK_SAMPLES_DIR is not set");
        return;
    };
    for year in 2024..=2027 {
        let path = dir.join(format!("{year}/racadvancedsampleproject.rvt"));
        if !path.exists() {
            eprintln!("skipping: {} absent", path.display());
            continue;
        }
        let mut rf = RevitFile::open(&path).expect("open");
        let model = export(&mut rf, &ConvertOptions::default())
            .expect("export")
            .model;
        // No opening of shaft 156935 is its 189.5 by 176.1 ft box.
        for entity in &model.entities {
            if let IfcEntity::VoidOpening {
                tag, outline_feet, ..
            } = entity
                && tag == "156935"
            {
                let cut = area(outline_feet);
                assert!(
                    cut < 0.3 * 189.49 * 176.06,
                    "{year}: 156935 cuts {cut:.1} ft²"
                );
            }
        }
        for (tag, revit) in REVIT_AREA_FT2 {
            let (index, profile) = model
                .entities
                .iter()
                .enumerate()
                .find_map(|(index, entity)| match entity {
                    IfcEntity::BuildingElement {
                        type_guid: Some(t),
                        extrusion: Some(extrusion),
                        ..
                    } if t == tag => Some((index, extrusion.profile_override.clone())),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{year}: {tag} has no extruded body"));
            let outline = match profile {
                Some(ProfileDef::ArbitraryClosed { points }) => area(&points),
                Some(ProfileDef::ArbitraryWithVoids { points, voids }) => {
                    area(&points) - voids.iter().map(|ring| area(ring.as_slice())).sum::<f64>()
                }
                other => panic!("{year}: {tag}'s profile is {other:?}"),
            };
            let cut: f64 = model
                .entities
                .iter()
                .filter_map(|entity| match entity {
                    IfcEntity::VoidOpening {
                        host, outline_feet, ..
                    } if *host == index => Some(area(outline_feet)),
                    _ => None,
                })
                .sum();
            // A loop that runs past the outline is counted whole here, so what
            // is left may fall short of Revit's Area (by 44 ft² on 247617, whose
            // drawn body is Revit's Area to 0.1 ft²); it may not exceed it.
            let left = outline - cut;
            assert!(
                left <= revit * 1.005 && left >= revit * 0.98,
                "{year}: {tag} is {left:.2} ft² once cut ({outline:.2} less {cut:.2}), Revit's Area {revit}"
            );
        }
    }
}
