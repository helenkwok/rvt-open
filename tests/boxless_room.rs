//! a room whose element record carries no box is still a room.
//!
//! BIM4LCA / Nordic LCA `ARK_NordicLCA_Office_Timber_As-Built.rvt` (Revit
//! 2024; VTT, Granlund, Gravicon, Insinööritoimisto Kallinen; CC BY-SA 4.0,
//! <https://cris.vtt.fi/en/publications/bim-files-to-download/>) holds 232
//! rooms: Revit's own IFC4 export of the file has an `IfcSpace` per room
//! (its `IfcSpaceType` tags them by ElementId). Room 3128929 (`CHASE`) is
//! one of them, and its record has no `Outline` box, so it used to be
//! dropped.
//!
//! Runs against `RVT_NORDIC_LCA_DIR`, a folder holding the `.rvt`. Skips when
//! absent.

use rvt::RevitFile;
use std::collections::BTreeSet;
use std::path::PathBuf;

#[test]
fn nordic_office_timber_keeps_its_boxless_room() {
    const NAME: &str = "ARK_NordicLCA_Office_Timber_As-Built.rvt";
    let Some(dir) = std::env::var_os("RVT_NORDIC_LCA_DIR").map(PathBuf::from) else {
        eprintln!("skipping: RVT_NORDIC_LCA_DIR is not set");
        return;
    };
    let rvt = dir.join(NAME);
    if !rvt.exists() {
        eprintln!("skipping: {NAME} absent");
        return;
    }
    let mut rf = RevitFile::open(&rvt).expect("open");
    let rooms = rvt::research::partition_schema_mvp::rooms_from_partition_category_records(
        &mut rf,
        2024,
        &BTreeSet::new(),
    )
    .expect("rooms");
    let ids: BTreeSet<u32> = rooms.iter().filter_map(|room| room.id).collect();
    assert_eq!(ids.len(), rooms.len(), "a room emitted twice");
    assert!(ids.contains(&3_128_929), "room 3128929 not recovered");
    // the same records give the model's areas (Revit's export
    // writes them as `IfcSpace` too).
    let count = |class: &str| rooms.iter().filter(|r| r.class == class).count();
    assert_eq!(count("Room"), 232, "Revit's export holds 232 rooms");
    assert_eq!(count("Area"), 30, "Revit's export holds 30 areas");
}
