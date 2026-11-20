//! A cylinder face of several trim loops is drawn on points its triangles share.
//!
//! Real file: the BIM4LCA / Nordic LCA timber office as built,
//! `ARK_NordicLCA_Office_Timber_As-Built.rvt` (Revit 2024; VTT, Granlund, Gravicon,
//! Insinööritoimisto Kallinen; CC BY-SA 4.0,
//! <https://cris.vtt.fi/en/publications/bim-files-to-download/>), and Revit's own IFC4
//! export of it, from `RVT_NORDIC_LCA_DIR`. Skips when either is absent.
//!
//! The office holds 15 `Stay_Wall_Mounted_:VAR-1` grab bars whose round legs are cylinder
//! faces cut by two trim loops (a hole, or two regions). They are drawn from their saved
//! meshes, and the strips of those faces meet: no directed edge of a bar's triangles lacks
//! the reverse edge (points to 1e-5 ft), as in Revit's own body, where strips that did not
//! share points left 300 on each. Revit's export draws the legs as a square whose corners
//! lie on the round, so the bars' volume is 1.11 of Revit's, as the file saves them.

use crate::ramp_flight_volume::{unmatched_edges, volumes};

const BARS: [&str; 15] = [
    "1832498", "2633695", "2772001", "2794370", "3015342", "3073846", "3110114", "3112722",
    "3120733", "3121298", "3121314", "3481562", "3481576", "3481585", "3481600",
];

#[test]
fn office_timber_grab_bars_have_no_open_edges() {
    let Some((ours, revit)) = volumes(
        "ARK_NordicLCA_Office_Timber_As-Built",
        &["IFCFURNITURE"],
        &BARS,
    ) else {
        return;
    };
    assert_eq!(ours.len(), BARS.len(), "rvt-open's bars: {:?}", ours.keys());
    assert_eq!(revit.len(), BARS.len(), "Revit's bars: {:?}", revit.keys());
    for (tag, body) in &ours {
        let mesh = body
            .mesh
            .as_ref()
            .unwrap_or_else(|| panic!("{tag}: a mesh"));
        let open = unmatched_edges(&mesh.0, &mesh.1);
        let ratio = body.volume / revit[tag].volume;
        eprintln!(
            "{tag}: {} triangles, {open} edges without their reverse, volume {ratio:.6} of Revit's",
            mesh.1.len()
        );
        assert_eq!(open, 0, "{tag}: edges without their reverse");
        assert!(
            (1.10..1.12).contains(&ratio),
            "{tag}: the round legs against Revit's square ones, ratio {ratio}"
        );
    }
}
