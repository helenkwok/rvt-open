//! the 2D maps Revit's export gives a family type beside its
//! Body map, and every room, area and family-as-space as an `IfcSpace`.
//!
//! Revit 2027.1's IFC4 export of the Nordic models holds a `FootPrint` map
//! (the type's plan symbol, in the `Plan` context's `Annotation`
//! sub-context; or a curtain panel's extrusion base, in the model's
//! `FootPrint` sub-context) or an `Axis` map (a beam's location line) on the
//! type object that holds the Body map, and every product drawn by the Body
//! map draws it too. It writes rooms and areas as aggregated `IfcSpace`s,
//! with the "IFC Predefined Type" parameter or `NOTDEFINED`, and family
//! instances set to `IfcSpace` as contained ones. This pins those counts on
//! one model under the library's default (`revit`) convention.
//!
//! Real file: `ARK_NordicLCA_Office_Concrete_BuildingPermit.rvt` (Nordic
//! LCA / BIM4LCA, Revit 2024; CC BY-SA 4.0), from `RVT_NORDIC_LCA_DIR`.
//! Skips when absent.

use rvt::RevitFile;
use rvt::ifc::body_geometry::share_shapes_by_convention;
use rvt::ifc::{GraphOptions, IfcxGraph, RvtDocExporter, write_step_graph};
use std::path::PathBuf;

fn count(step: &str, needle: &str) -> usize {
    step.lines().filter(|l| l.contains(needle)).count()
}

#[test]
fn nordic_office_concrete_plan_maps_and_spaces() {
    let Some(dir) = std::env::var_os("RVT_NORDIC_LCA_DIR").map(PathBuf::from) else {
        eprintln!("skipping: RVT_NORDIC_LCA_DIR is not set");
        return;
    };
    let rvt = dir.join("ARK_NordicLCA_Office_Concrete_BuildingPermit.rvt");
    if !rvt.exists() {
        eprintln!("skipping: model absent");
        return;
    }
    let mut rf = RevitFile::open(&rvt).expect("open");
    let mut model = RvtDocExporter
        .export_with_diagnostics(&mut rf)
        .expect("export")
        .model;
    share_shapes_by_convention(&mut model);
    let report = rvt::ifc::plan_maps::attach(&mut rf, &mut model).expect("plan maps");
    let step = write_step_graph(&IfcxGraph::from_model(
        &model,
        GraphOptions {
            meshes: false,
            shared_shapes: true,
        },
    ));

    // Every 2D map is written once, as its own representation map, and each
    // product of its Body map draws it.
    let plan_maps: usize = model.representation_maps.iter().map(|m| m.plan.len()).sum();
    assert_eq!(
        count(&step, "IFCREPRESENTATIONMAP("),
        model.representation_maps.len() + plan_maps
    );
    assert!(count(&step, "'FootPrint','MappedRepresentation'") > 0 && plan_maps > 0);
    let pinned = (
        report.footprints,
        report.plate_footprints,
        report.beam_axes,
        report.users_differing,
    );
    assert_eq!(
        pinned, PINNED_PLAN,
        "footprints, plate footprints, beam axes, maps whose users differ"
    );

    // Spaces: rooms, areas and family instances set to IfcSpace.
    let spaces = count(&step, "=IFCSPACE(");
    let notdefined = step
        .lines()
        .filter(|l| l.contains("=IFCSPACE(") && l.contains(".NOTDEFINED."))
        .count();
    assert_eq!(
        (spaces, notdefined),
        PINNED_SPACES,
        "IfcSpace, of which NOTDEFINED"
    );
}

/// Level stopped drawing one, as in Revit's export), 138 plates' bases, 2
// 241 -> 243 footprints and 16 -> 14 maps whose users differ: the walls that
// embed a curtain wall, drawn from their saved mesh where Revit raised their
// base, are no longer the plain prisms that shared those maps.
const PINNED_PLAN: (usize, usize, usize, usize) = (243, 137, 2, 14);
/// 306 spaces: 234 rooms, 30 areas and 42 family instances (`USERDEFINED`);
/// 74 `NOTDEFINED`: the 30 areas and the 44 rooms whose "IFC Predefined
/// Revit 2027.1 writes them).
// written as `SPACE` (as Revit's export does on the BIM4LCA
// office), 29 fewer `NOTDEFINED`, and two spaces more.
const PINNED_SPACES: (usize, usize) = (308, 45);
