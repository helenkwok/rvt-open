//! DXF and DWG layers named and coloured by an export layer table, in the format of the authoring
//! tool's `exportlayers-*.txt` files.
//!
//! - A table reads: a subcategory with no row, or with no layer name, takes its category's row; a
//!   cut line takes the cut layer and colour; a wall takes the row of its type's Function.
//! - `rvt-sheet --view "Level 1"` on rac_basic_sample_project 2023 (`racbasicsampleproject.rvt` in
//!   `RVT_REVIT_2023_DIR`, from the download host, see `research/autodesk-sample-projects.tsv`)
//!   gives each layer it shares with the authoring tool's default DWG export of the same view
//!   (AIA layer names, category properties BYLAYER) that export's colour number.
//! - `--layer-table FILE` renames the layers by the table given, annotation included.
#![cfg(feature = "dwg")]

use rvt::ifc::dwg_layers::{ExportLayers, LayerTable, Line};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

/// A small table of our own, in the authoring tool's format.
const TABLE: &str = "# a comment\r\n\
Walls\t\tWALL-ANY\t5\tWALL-CUT\t6\r\n\
Walls/Interior:1\t\tWALL-INT\t3\tWALL-INT\t3\r\n\
Walls\tCut Pattern\tWALL-PATT\t8\tWALL-PATT\t8\r\n\
Walls\tCommon Edges\t\t-1\t\t-1\r\n\
Stairs\t\tSTAIR\t2\tSTAIR\t2\r\n\
Stairs\t<Above> Outlines\tSTAIR-OVER\t4\tSTAIR-OVER\t4\r\n\
Grids\t\tGRID-LINE\t1\tGRID-LINE\t1\r\n\
Grid Heads\t\tGRID-HEAD\t1\tGRID-HEAD\t1\r\n";

#[test]
fn a_table_names_layers_by_category_subcategory_cut_and_wall_function() {
    let t = LayerTable::parse(TABLE).expect("parses");
    assert_eq!(t.len(), 8);
    assert_eq!(
        t.layer("Walls", "Cut Pattern", false),
        Some(("WALL-PATT", 8))
    );
    // No layer name in its row, and no row: the category's.
    assert_eq!(
        t.layer("Walls", "Common Edges", false),
        Some(("WALL-ANY", 5))
    );
    assert_eq!(
        t.layer("Walls", "Finish 1 [4]", true),
        Some(("WALL-CUT", 6))
    );
    assert_eq!(t.layer("Doors", "", false), None);
    assert!(LayerTable::parse("Walls\tonly two fields").is_err());

    let mut layers = ExportLayers::with_table(t);
    layers.wall_functions.insert(7, 0);
    let wall = |id, external, line| layers.element_layer("Walls", Some(id), external, line);
    // Function 0 (interior) by the wall's id, before its IsExternal.
    assert_eq!(wall(7, Some(true), Line::Cut), Some(("WALL-INT".into(), 3)));
    // No Function read: IsExternal true is an exterior wall, which this table gives no row.
    assert_eq!(wall(8, Some(true), Line::Cut), Some(("WALL-CUT".into(), 6)));
    assert_eq!(
        wall(8, Some(false), Line::Seen),
        Some(("WALL-INT".into(), 3))
    );
    assert_eq!(
        layers.element_layer("Stairs", None, None, Line::Overhead),
        Some(("STAIR-OVER".into(), 4))
    );
    assert_eq!(
        layers.annotation_layer("S-GRID-IDEN", 7),
        ("GRID-HEAD".into(), 1)
    );
    assert_eq!(
        layers.annotation_layer("NOT-A-LAYER", 7),
        ("NOT-A-LAYER".into(), 7)
    );

    // The built-in table: the export's names for interior and exterior walls.
    let builtin = ExportLayers::default();
    assert_eq!(
        builtin.element_layer("Walls", None, Some(false), Line::Cut),
        Some(("I-WALL".into(), 2))
    );
    assert_eq!(
        builtin.element_layer("Walls", None, Some(true), Line::Cut),
        Some(("A-WALL".into(), 113))
    );
}

fn sample() -> Option<PathBuf> {
    let Some(dir) = std::env::var_os("RVT_REVIT_2023_DIR").map(PathBuf::from) else {
        eprintln!("skipping: RVT_REVIT_2023_DIR is not set");
        return None;
    };
    let path = dir.join("racbasicsampleproject.rvt");
    if !path.exists() {
        eprintln!("skipping: {} absent", path.display());
        return None;
    }
    Some(path)
}

/// `rvt-sheet` on plan view "Level 1" to DXF, `extra` added, read back: each layer's colour number
/// and how many entities it holds.
fn level_1(
    rvt: &std::path::Path,
    tag: &str,
    extra: &[&std::ffi::OsStr],
) -> BTreeMap<String, (i16, usize)> {
    let dir = std::env::temp_dir().join(format!("rvt-dwg-layers-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dxf = dir.join("level-1.dxf");
    let out = Command::new(env!("CARGO_BIN_EXE_rvt-sheet"))
        .arg(rvt)
        .args(["--view", "Level 1", "--format", "dxf", "-o"])
        .arg(&dxf)
        .args(extra)
        .output()
        .expect("spawn rvt-sheet");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let read = rvt_dwg::read_file(&dxf, rvt_dwg::Limits::DEFAULT).expect("read back");
    let _ = std::fs::remove_dir_all(&dir);
    let mut layers: BTreeMap<String, (i16, usize)> = read
        .drawing
        .layers
        .iter()
        .map(|l| (l.name.clone(), (l.colour.aci.unwrap_or(-1), 0)))
        .collect();
    for e in &read.drawing.entities {
        if let Some(l) = layers.get_mut(&e.layer) {
            l.1 += 1;
        }
    }
    layers.retain(|_, (_, n)| *n > 0);
    layers
}

/// The colour number of each layer the authoring tool's default DWG export of rac_basic's plan view
/// "Level 1" draws on (AIA layer names, category properties BYLAYER, Revit 2023).
const EXPORT_COLOURS: [(&str, i16); 28] = [
    ("A-ANNO-DIMS", 211),
    ("A-AREA", 32),
    ("A-AREA-IDEN", 2),
    ("A-DOOR", 31),
    ("A-FLOR", 192),
    ("A-FLOR-HRAL", 11),
    ("A-FLOR-HRAL-MBND", 5),
    ("A-GENM", 13),
    ("A-GLAZ", 151),
    ("A-GLAZ-CURT", 52),
    ("A-GLAZ-CWMG", 51),
    ("A-WALL", 113),
    ("C-BLDG-PADS", 91),
    ("G-ANNO-TEXT", 231),
    ("I-FURN", 30),
    ("I-WALL", 2),
    ("P-SANR-FIXT", 6),
    ("Q-CASE", 31),
    ("Q-SPCQ", 91),
    ("S-COLS", 52),
    ("S-FNDN", 32),
    ("S-GRID", 1),
    ("S-GRID-IDEN", 1),
    ("S-STRS", 31),
    ("S-STRS-MBND", 31),
    ("A-ANNO-NOTE", 131),
    ("G-ANNO-SYMB", 111),
    ("A-WALL-PATT", 111),
];

#[test]
fn level_1_layers_take_the_exports_colours() {
    let Some(rvt) = sample() else { return };
    let layers = level_1(&rvt, "builtin", &[]);
    let mut shared = 0;
    for (name, colour) in EXPORT_COLOURS {
        if let Some((ours, _)) = layers.get(name) {
            assert_eq!(*ours, colour, "{name}: {layers:?}");
            shared += 1;
        }
    }
    // Before the table, none of the 25 layers both drew had the export's colour.
    assert!(shared >= 24, "{shared} shared layers: {layers:?}");
}

#[test]
fn a_supplied_table_renames_the_layers() {
    let Some(rvt) = sample() else { return };
    let dir = std::env::temp_dir().join(format!("rvt-dwg-layers-table-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let table = dir.join("exportlayers-test.txt");
    std::fs::write(&table, TABLE).unwrap();
    let layers = level_1(
        &rvt,
        "table",
        &["--layer-table".as_ref(), table.as_os_str()],
    );
    let _ = std::fs::remove_dir_all(&dir);
    for (name, colour) in [
        ("WALL-CUT", 6),
        ("WALL-INT", 3),
        ("STAIR", 2),
        ("STAIR-OVER", 4),
        ("GRID-LINE", 1),
        ("GRID-HEAD", 1),
    ] {
        assert_eq!(
            layers.get(name).map(|l| l.0),
            Some(colour),
            "{name}: {layers:?}"
        );
    }
    for gone in [
        "A-WALL",
        "I-WALL",
        "S-STRS",
        "S-STRS-MBND",
        "S-GRID",
        "S-GRID-IDEN",
    ] {
        assert!(!layers.contains_key(gone), "{gone}: {layers:?}");
    }
}
