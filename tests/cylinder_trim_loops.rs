//! A cylinder face of two trim loops is drawn from the saved graphics.
//!
//! Real file: the BIM4LCA / Nordic LCA timber office as built,
//! `ARK_NordicLCA_Office_Timber_As-Built.rvt` (Revit 2024; VTT, Granlund, Gravicon,
//! Insinööritoimisto Kallinen; CC BY-SA 4.0,
//! <https://cris.vtt.fi/en/publications/bim-files-to-download/>), from
//! `RVT_NORDIC_LCA_DIR`. Skips when absent.
//!
//! The office holds 15 `Stay_Wall_Mounted_:VAR-1` grab bars whose round legs are cylinder
//! faces cut by two trim loops (a hole, or two regions). Each is drawn from its saved mesh
//! (about 7,700 triangles), not left as its record's box.

use crate::ramp_flight_volume::{body_volumes, parse};
use rvt::RevitFile;
use rvt::convert::{ConvertOptions, export};
use rvt::ifc::write_step;
use std::path::PathBuf;

const BARS: [&str; 15] = [
    "1832498", "2633695", "2772001", "2794370", "3015342", "3073846", "3110114", "3112722",
    "3120733", "3121298", "3121314", "3481562", "3481576", "3481585", "3481600",
];

#[test]
fn office_timber_grab_bars_are_drawn_from_their_meshes() {
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
        eprintln!("{tag}: {} triangles", mesh.1.len());
        assert!(mesh.1.len() > 7_000, "{tag}: {} triangles", mesh.1.len());
    }
}
