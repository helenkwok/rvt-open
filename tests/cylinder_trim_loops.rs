//! A cylinder face of several trim loops is drawn on points its triangles share.
//!
//! Real file: the BIM4LCA / Nordic LCA timber office as built,
//! `ARK_NordicLCA_Office_Timber_As-Built.rvt` (Revit 2024; VTT, Granlund, Gravicon,
//! Insinööritoimisto Kallinen; CC BY-SA 4.0,
//! <https://cris.vtt.fi/en/publications/bim-files-to-download/>), from
//! `RVT_NORDIC_LCA_DIR`. Skips when absent.
//!
//! The office holds 15 `Stay_Wall_Mounted_:VAR-1` grab bars whose round legs are cylinder
//! faces cut by two trim loops (a hole, or two regions). They are drawn from their saved
//! meshes, and the strips of those faces meet: no directed edge of a bar's triangles lacks
//! the reverse edge (points to 1e-5 ft), as in Revit's own bodies, where strips that did not
//! share points left 300 on each.

use crate::ramp_flight_volume::{body_volumes, parse, unmatched_edges};
use rvt::RevitFile;
use rvt::convert::{ConvertOptions, export};
use rvt::ifc::write_step;
use std::path::PathBuf;

const BARS: [&str; 15] = [
    "1832498", "2633695", "2772001", "2794370", "3015342", "3073846", "3110114", "3112722",
    "3120733", "3121298", "3121314", "3481562", "3481576", "3481585", "3481600",
];

#[test]
fn office_timber_grab_bars_have_no_open_edges() {
    const NAME: &str = "ARK_NordicLCA_Office_Timber_As-Built.rvt";
    let Some(dir) = std::env::var_os("RVT_NORDIC_LCA_DIR").map(PathBuf::from) else {
        eprintln!("skipping: RVT_NORDIC_LCA_DIR is not set");
        return;
    };
    let path = dir.join(NAME);
    if !path.exists() {
        eprintln!("skipping: {NAME} absent");
        return;
    }
    let mut rf = RevitFile::open(&path).expect("open");
    // Every body offered its saved mesh, as `rvt-ifc` does.
    let options = ConvertOptions {
        saved_meshes: true,
        ..ConvertOptions::default()
    };
    let result = export(&mut rf, &options).expect("export");
    let bars = body_volumes(
        &parse(&write_step(&result.model)),
        &["IFCFURNITURE"],
        &BARS,
        1.0,
    );
    assert_eq!(bars.len(), BARS.len(), "rvt-open's bars: {:?}", bars.keys());
    for (tag, body) in &bars {
        let mesh = body
            .mesh
            .as_ref()
            .unwrap_or_else(|| panic!("{tag}: a mesh"));
        let open = unmatched_edges(&mesh.0, &mesh.1);
        eprintln!(
            "{tag}: {} triangles, {open} directed edges without their reverse",
            mesh.1.len()
        );
        assert_eq!(open, 0, "{tag}: edges without their reverse");
    }
}
