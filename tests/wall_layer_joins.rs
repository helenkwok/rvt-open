//! Layered walls at butt joins end layer by layer as Revit's IFC4 export
//! draws them: walls of different heights, walls whose layer functions differ
//! from the outside of the corner, T joints whose layers nest, and walls whose
//! line only their native record gives.
//!
//! Each case is a wall end and, per layer (exterior first), how far its body
//! reaches past the end of its centreline, in feet, read from Revit's own IFC4
//! export of the same model (the body cut along each layer's middle line,
//! matched by ElementId). Before this, each of these ends was drawn square.
//!
//! - Autodesk's `racbasicsampleproject.rvt` (Revit 2024): wall 428588 stops
//!   at both ends against a taller wall of its type, and its six layers nest
//!   into the corner.
//! - Autodesk's Snowdon Towers Architectural (2024): wall 1055451 runs through
//!   a lower wall at its end and reaches that wall's far face with every
//!   layer.
//! - BIM4LCA `ARK_NordicLCA_Housing_Concrete_BuildingPermit.rvt` (Revit 2024;
//!   CC BY-SA 4.0, <https://cris.vtt.fi/en/publications/bim-files-to-download/>):
//!   wall 1346777 does the same.
//! - Snowdon Towers Architectural: wall 1699930 stops at a T joint, and its
//!   layers outside the core on the side whose functions match the other
//!   wall's near side step into that wall's like layers; wall 685081 stops at
//!   a T joint against a wall whose line only its native record gives;
//!   wall 2115148, a structure and finish wall, runs through one whose
//!   finish is on the other side, and its finish stops at the other's core.
//! - BIM4LCA Housing Concrete Building Permit: walls 1499172 and 1501553 meet
//!   at an L joint, each with a finish 1 on one face and a finish 2 on the
//!   other; their cores nest, the finish 2 of the wall that stops wraps the
//!   corner and the other's finish 1 stops at it.
//! - BIM4LCA Housing Timber Building Permit (wall 1346757) and Office
//!   Concrete As-Built (wall 1348463): walls joined to walls whose data copies
//!   disagree on their line, which now take their native record's line, so
//!   the joins are read and the layers nest.
//!
//! Runs against `RVT_REVIT2024_SAMPLES_DIR` and `RVT_NORDIC_LCA_DIR`; each
//! case skips when its model is absent.

use rvt::RevitFile;
use rvt::research::partition_schema_mvp as mvp;
use std::path::PathBuf;

/// (model file, ElementId, end of its centreline (0 start, 1 end), Revit's
/// reach per layer, exterior first, feet).
type Case = (&'static str, u32, usize, &'static [f64]);

const SAMPLES_2024: &[Case] = &[
    (
        "racbasicsampleproject.rvt",
        428588,
        0,
        &[0.25919, 0.17717, 0.12795, -0.2395, -0.28871, -0.33136],
    ),
    (
        "racbasicsampleproject.rvt",
        428588,
        1,
        &[0.25919, 0.17716, 0.12795, -0.2395, -0.28872, -0.33137],
    ),
    (
        "Snowdon Towers Sample Architectural.rvt",
        1055451,
        1,
        &[0.52604; 6],
    ),
    (
        "Snowdon Towers Sample Architectural.rvt",
        1699930,
        1,
        &[-0.60937, -0.46354, -0.32812, -0.16146, -0.10937, -0.60937],
    ),
    (
        "Snowdon Towers Sample Architectural.rvt",
        685081,
        0,
        &[
            -0.52604, -0.47396, -0.47396, -0.47396, -0.47396, -0.47396, -0.47396, -0.52604,
        ],
    ),
    (
        "Snowdon Towers Sample Architectural.rvt",
        2115148,
        1,
        &[0.125, -0.17708],
    ),
];

const NORDIC: &[Case] = &[
    (
        "ARK_NordicLCA_Housing_Concrete_BuildingPermit.rvt",
        1346777,
        0,
        &[0.88583; 5],
    ),
    (
        "ARK_NordicLCA_Housing_Concrete_BuildingPermit.rvt",
        1499172,
        1,
        &[0.10827, 0.10827, -0.15092],
    ),
    (
        "ARK_NordicLCA_Housing_Concrete_BuildingPermit.rvt",
        1501553,
        0,
        &[-0.10827, -0.10827, 0.15092],
    ),
    (
        "ARK_NordicLCA_Housing_Timber_BuildingPermit.rvt",
        1346757,
        0,
        &[
            0.6332, 0.54134, 0.46916, 0.33137, 0.30184, -0.51837, -0.58399,
        ],
    ),
    (
        "ARK_NordicLCA_Office_Concrete_As-Built.rvt",
        1348463,
        1,
        &[0.71358, 0.61516, 0.45112, 0.35269, -0.22146],
    ),
];

fn check(dir: &str, cases: &[Case]) {
    let Some(dir) = std::env::var_os(dir).map(PathBuf::from) else {
        eprintln!("skipping: {dir} is not set");
        return;
    };
    let mut models: Vec<&str> = cases.iter().map(|case| case.0).collect();
    models.dedup();
    for model in models {
        let path = dir.join(model);
        if !path.exists() {
            eprintln!("skipping: {model} absent");
            continue;
        }
        let mut rf = RevitFile::open(&path).expect("open");
        let version = rf.basic_file_info().expect("basic file info").version;
        let walls = mvp::recover_partition_schema_mvp(
            &mut rf,
            version,
            rvt::walker::WalkerLimits::default(),
        )
        .expect("partition MVP")
        .walls;
        for (_, id, end, revit) in cases.iter().filter(|case| case.0 == model) {
            let wall = walls
                .iter()
                .find(|wall| wall.id == Some(*id))
                .unwrap_or_else(|| panic!("{model}: wall {id} not read"));
            let ends = mvp::wall_layer_ends_from_fields(&wall.fields)
                .unwrap_or_else(|| panic!("{model}: wall {id} has no layered ends"));
            let ours = ends.reach_feet[*end]
                .as_ref()
                .unwrap_or_else(|| panic!("{model}: wall {id} end {end} is not layered"));
            assert_eq!(ours.len(), revit.len(), "{model}: wall {id} layers");
            for (layer, ([exterior, interior], want)) in ours.iter().zip(revit.iter()).enumerate() {
                for edge in [exterior, interior] {
                    assert!(
                        (edge - want).abs() <= 0.001,
                        "{model}: wall {id} end {end} layer {layer} reaches {edge}, Revit {want}"
                    );
                }
            }
        }
    }
}

#[test]
fn layered_ends_of_walls_of_different_heights_2024_samples() {
    check("RVT_REVIT2024_SAMPLES_DIR", SAMPLES_2024);
}

#[test]
fn layered_ends_of_walls_of_different_heights_nordic() {
    check("RVT_NORDIC_LCA_DIR", NORDIC);
}
