//! A body whose shared symbol is read but not decoded says why.
//!
//! Real file: the BIM4LCA / Nordic LCA concrete housing at building permit,
//! `ARK_NordicLCA_Housing_Concrete_BuildingPermit.rvt` (Revit 2024; VTT, Granlund, Gravicon,
//! Insinööritoimisto Kallinen; CC BY-SA 4.0,
//! <https://cris.vtt.fi/en/publications/bim-files-to-download/>), from
//! `RVT_NORDIC_LCA_DIR`. Skips when absent.
//!
//! The house holds 12 bodies (7 doors and 5 sanitary terminals) whose saved graphics name a
//! shared symbol that the file holds and the graph reader refuses (it runs out of its field
//! value budget). Their box stays, and the refusal used to read as if the file lacked the
//! symbol (`missing current symbol graphics N`); it now reads `current symbol graphics N not
//! decoded:` and the reader's own reason. The bodies drawn do not change.

use rvt::RevitFile;
use rvt::ifc::{ExportOptions, run_export};
use std::path::PathBuf;

#[test]
fn concrete_housing_names_the_refusal_of_a_symbol_it_holds() {
    const NAME: &str = "ARK_NordicLCA_Housing_Concrete_BuildingPermit.rvt";
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
    let exported = run_export(&mut rf, &ExportOptions::default()).expect("export");
    let refusals = &exported.diagnostics.exported.saved_mesh_refusals;
    eprintln!("{refusals:#?}");
    let count = |text: &str| -> usize {
        refusals
            .iter()
            .filter(|(reason, _)| reason.contains(text))
            .map(|(_, n)| n)
            .sum()
    };
    assert_eq!(
        count("missing current symbol graphics"),
        0,
        "a symbol the file holds is not missing: {refusals:?}"
    );
    assert_eq!(
        count("current symbol graphics #######"),
        12,
        "12 bodies name their symbol: {refusals:?}"
    );
    assert_eq!(
        count("not decoded: "),
        12,
        "each says the reader's own reason: {refusals:?}"
    );
    assert_eq!(
        count("native field value budget exceeded"),
        12,
        "each names the budget the reader ran out of: {refusals:?}"
    );
}
