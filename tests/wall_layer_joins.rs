//! Layered walls at butt joins whose walls' heights differ end layer by layer
//! as Revit's IFC4 export draws them.
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
];

const NORDIC: &[Case] = &[(
    "ARK_NordicLCA_Housing_Concrete_BuildingPermit.rvt",
    1346777,
    0,
    &[0.88583; 5],
)];

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
