//! "Export to IFC As" and the IFC predefined type are read on Revit
//! reads them on 2024 and later.
//!
//! On Revit 2022 the values are type parameters of the shared parameters
//! `IfcExportAs` and `IfcExportType`, held in each family type's
//! `FamilyParams`; Revit 2023 keeps those and adds the built-ins
//! `IFC_EXPORT_ELEMENT_TYPE_AS` / `IFC_EXPORT_PREDEFINEDTYPE_TYPE`. On both
//! saves of the same model the Revit API dump lists 308 elements
//! with each: 307 export-as values and all 308 predefined types are read the
//! same (the 308th export-as, `IfcElectric DistributionBoardType`, is no
//! entity name and is not taken). Runs against `RVT_REVIT_EXTRA_DIR`
//! (`<release>/<label>.rvt`) and skips what is absent.

use rvt::RevitFile;
use rvt::ifc::{RvtDocExporter, write_step};
use std::path::PathBuf;

fn sample(file: &str) -> Option<PathBuf> {
    let Some(dir) = std::env::var_os("RVT_REVIT_EXTRA_DIR").map(PathBuf::from) else {
        eprintln!("skipping: RVT_REVIT_EXTRA_DIR is not set");
        return None;
    };
    let path = dir.join(file);
    path.exists().then_some(path).or_else(|| {
        eprintln!("skipping: {file} absent");
        None
    })
}

#[test]
fn export_parameters_are_read_on_2022_and_2023() {
    for file in [
        "2022/cjk_cf74146d__pf2022.rvt",
        "2023/cjk_e618e87d__pf2023.rvt",
    ] {
        let Some(path) = sample(file) else { continue };
        let mut rf = RevitFile::open(&path).expect("open");
        let declared =
            rvt::elem_table::declared_ids(&rvt::elem_table::parse_records(&mut rf).expect("elems"));
        let map = rvt::research::partition_ifc_export_overrides::scan_export_parameters_schema(
            &mut rf, &declared,
        )
        .expect("scan");
        let with_entity = map.values().filter(|p| p.export_as.is_some()).count();
        let with_predefined = map.values().filter(|p| p.predefined_type.is_some()).count();
        assert_eq!(
            (map.len(), with_entity, with_predefined),
            (1141, 1137, 1141),
            "{file}"
        );
        let heat_recovery = &map[&17_090_623];
        assert_eq!(
            (
                heat_recovery.export_as.as_deref(),
                heat_recovery.predefined_type.as_deref()
            ),
            (Some("IfcAirToAirHeatRecovery"), Some("ROTARYWHEEL")),
            "{file}"
        );
    }
}

/// The exporter writes the elements as the entities their types name: the
/// 2022 and 2023 saves give every element the class the 2024 save
/// `IfcValve` instead of `IfcWasteTerminal` and two air terminals `IfcValve`.
#[test]
fn the_2022_and_2023_saves_export_the_entities_their_types_name() {
    for file in [
        "2022/cjk_cf74146d__pf2022.rvt",
        "2023/cjk_e618e87d__pf2023.rvt",
    ] {
        let Some(path) = sample(file) else { continue };
        let mut rf = RevitFile::open(&path).expect("open");
        let model = RvtDocExporter
            .export_with_diagnostics(&mut rf)
            .expect("export")
            .model;
        let step = write_step(&model);
        let count = |entity: &str| {
            let open = format!("{entity}(");
            step.lines()
                .filter(|l| {
                    l.split_once('=')
                        .is_some_and(|(_, b)| b.trim_start().starts_with(&open))
                })
                .count()
        };
        assert_eq!(
            ["IFCAIRTERMINAL", "IFCVALVE", "IFCWASTETERMINAL"].map(count),
            [133, 30, 2],
            "{file}"
        );
    }
}
