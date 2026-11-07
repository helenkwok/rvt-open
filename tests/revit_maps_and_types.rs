//! `--convention revit` maps family instances as Revit's export
//! does and writes only the types its elements use; `--convention open`
//! also writes every type the file defines, placed or not.
//!
//! Revit 2027's own IFC4 exports of the Nordic models draw every family
//! instance (doors, windows, members, columns, beams, plates, furniture,
//! terminals) through an `IfcMappedItem` of a map its own type object holds,
//! a type placed once included, with at most one Body map per type object,
//! and never a wall, slab, covering, railing, stair, roof or opening. This
//! checks the `revit` sharing against that rule on the library's model and
//! its IFC4, and pins the counts, including the unplaced types (which
//! Revit's export leaves out, and rvt-open writes under `open` only).
//!
//! Real file: `ARK_NordicLCA_Office_Concrete_BuildingPermit.rvt` (Nordic
//! LCA / BIM4LCA, Revit 2024; VTT, Granlund, Gravicon, Insinööritoimisto
//! Kallinen; CC BY-SA 4.0), from `RVT_NORDIC_LCA_DIR`. Skips when absent.

use rvt::RevitFile;
use rvt::ifc::body_geometry::{revit_family_instances, share_shapes_by_convention};
use rvt::ifc::entities::IfcEntity;
use rvt::ifc::{GraphOptions, IfcxGraph, RvtDocExporter, write_step_graph};
use std::path::PathBuf;

/// Classes Revit's export never draws through a mapped item.
const NEVER_MAPPED: &[&str] = &[
    "IFCWALL",
    "IFCSLAB",
    "IFCCOVERING",
    "IFCRAILING",
    "IFCSTAIR",
    "IFCSTAIRFLIGHT",
    "IFCROOF",
    "IFCRAMP",
    "IFCOPENINGELEMENT",
    "IFCCURTAINWALL",
    "IFCPIPESEGMENT",
];

#[test]
fn nordic_office_concrete_maps_family_instances_and_writes_used_types() {
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
    let family = revit_family_instances(&model);
    let maps = share_shapes_by_convention(&mut model);

    let mut mapped = 0;
    for (i, entity) in model.entities.iter().enumerate() {
        let IfcEntity::BuildingElement {
            ifc_type,
            representation_map_index,
            ..
        } = entity
        else {
            continue;
        };
        if representation_map_index.is_some() {
            mapped += 1;
            assert!(family[i], "{ifc_type}: mapped but no family instance");
            assert!(
                !NEVER_MAPPED.contains(&ifc_type.to_ascii_uppercase().as_str()),
                "{ifc_type} mapped"
            );
        }
    }

    let step = write_step_graph(&IfcxGraph::from_model(
        &model,
        GraphOptions {
            meshes: false,
            shared_shapes: true,
        },
    ));
    let lines: Vec<&str> = step.lines().collect();
    let count = |needle: &str| lines.iter().filter(|l| l.contains(needle)).count();
    assert_eq!(count("=IFCREPRESENTATIONMAP("), maps);
    assert_eq!(count("=IFCMAPPEDITEM("), mapped);
    // Type objects: those holding a map hold exactly one, as in Revit's
    // export (no type object of it holds two Body maps).
    let types: Vec<&&str> = lines
        .iter()
        .filter(|l| {
            l.split_once('=').is_some_and(|(_, e)| {
                e.starts_with("IFC")
                    && !e.starts_with("IFCREL")
                    && e.split('(').next().is_some_and(|n| n.ends_with("TYPE"))
            })
        })
        .collect();
    let mut holding = 0;
    for t in &types {
        let held = t.matches(",(#").count();
        // HasPropertySets is the first `(#..)` list; RepresentationMaps the second.
        if held >= 2 {
            holding += 1;
            let maps_list = t
                .split(",(#")
                .nth(2)
                .and_then(|s| s.split(')').next())
                .unwrap_or("");
            assert!(
                !maps_list.contains(','),
                "a type object holds two maps: {t}"
            );
        }
    }
    assert_eq!(holding, maps, "every map is held by one type object");
    let typed = count("=IFCRELDEFINESBYTYPE(");
    let unplaced = types.len() - typed;
    assert_eq!(
        (maps, mapped, types.len(), typed, unplaced),
        PINS,
        "(maps, mapped items, type objects, typed, unplaced type objects)"
    );

    // `open` also writes the types no element uses.
    model.convention = rvt::ifc::ExportConvention::Open;
    let open = write_step_graph(&IfcxGraph::from_model(
        &model,
        GraphOptions {
            meshes: false,
            shared_shapes: true,
        },
    ));
    let open_lines: Vec<&str> = open.lines().collect();
    let open_types = open_lines
        .iter()
        .filter(|l| {
            l.split_once('=').is_some_and(|(_, e)| {
                e.starts_with("IFC")
                    && !e.starts_with("IFCREL")
                    && e.split('(').next().is_some_and(|n| n.ends_with("TYPE"))
            })
        })
        .count();
    let open_typed = open_lines
        .iter()
        .filter(|l| l.contains("=IFCRELDEFINESBYTYPE("))
        .count();
    assert_eq!(
        (open_types, open_typed),
        OPEN_TYPES,
        "(type objects, typed) under open"
    );
}

/// Pinned: 1,944 maps (Revit 2027's own export:
/// 1,338 Body maps), 11,689 mapped items, 2,240 type objects, 2,204 of them
/// written as `IfcSpace` ("Export to IFC As") are typed by their family
/// types, 30 more type objects holding their maps: 2,270 and 2,234.
// IfcFlowTerminal; the same three gained plan footprints in plan_maps_and_spaces.
// against the 340 pin, measured by the gate.
// The revit convention writes no unplaced type (34 before it left them out).
const PINS: (usize, usize, usize, usize, usize) = (1_935, 11_691, 2_226, 2_226, 0);

/// Under `open`: type objects are one per Revit type, not split by map
/// (1,302 typing elements), plus the 34 types no element uses.
const OPEN_TYPES: (usize, usize) = (1_336, 1_302);
