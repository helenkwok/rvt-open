//! The layers DXF and DWG output draws each element on: an export layer table in the format of
//! the authoring tool's own `exportlayers-*.txt` files (category, subcategory, layer name, colour
//! number, cut layer name, cut colour number, tab-separated), looked up by an element's category,
//! its subcategory and whether the line is cut or seen.
//!
//! The built-in table holds the rows the drawings use, with the layer names and colour numbers
//! the authoring tool's default DWG export (AIA layer names, category properties BYLAYER) gives
//! the public sample projects, read from its exports of them. A row whose colour those exports
//! do not show has colour `-1`, and the drawing takes a colour from the element's IFC type. The
//! tool's own complete tables ship with it and are not included here: pass one
//! (`rvt-sheet --layer-table FILE`), or let [`LayerTable::from_revit_install`] find the copy an
//! installation keeps.
//!
//! Lookups follow the export's rules, read from its layer table for the samples: a subcategory
//! with no row, or a row with no layer name, is drawn on its category's layer; a wall is drawn on
//! the row for its type's Function (`Walls/Interior:1`, `Walls/Exterior:2`, `Walls/Foundation:3`,
//! `Walls/Retaining:4`) where the table has one; a cut line takes the cut layer and colour, and a
//! row with no cut layer its projection layer.
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, LazyLock};

/// One row of an export layer table.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    layer: String,
    colour: i16,
    cut_layer: String,
    cut_colour: i16,
}

/// An export layer table: the layer and colour each category and subcategory is drawn with.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LayerTable {
    rows: HashMap<(String, String), Row>,
}

/// The rows the drawings use, from the authoring tool's default export of the public samples.
const BUILTIN: &str = include_str!("dwg_layers_builtin.txt");

static BUILTIN_TABLE: LazyLock<LayerTable> =
    LazyLock::new(|| LayerTable::parse(BUILTIN).expect("built-in layer table parses"));

/// The layer a wall of type Function `function` (`WallFunction`: 0 interior, 1 exterior, 2
/// foundation, 3 retaining) is drawn on: the category name the table's row for it carries.
fn wall_function_category(function: i64) -> Option<&'static str> {
    Some(match function {
        0 => "Walls/Interior:1",
        1 => "Walls/Exterior:2",
        2 => "Walls/Foundation:3",
        3 => "Walls/Retaining:4",
        _ => return None,
    })
}

impl LayerTable {
    /// The built-in table.
    pub fn builtin() -> &'static LayerTable {
        &BUILTIN_TABLE
    }

    /// Read a table in the `exportlayers-*.txt` format: one row per line, `#` lines are
    /// comments, fields separated by tabs. A colour that is not a number is `-1`; a row with
    /// fewer than three fields is refused.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut rows = HashMap::new();
        for (n, line) in text.lines().enumerate() {
            let line = line.trim_end_matches('\r');
            if line.trim().is_empty() || line.starts_with('#') {
                continue;
            }
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() < 3 {
                return Err(format!(
                    "line {}: {} fields; a row is: category, subcategory, layer name, colour \
                     number, cut layer name, cut colour number",
                    n + 1,
                    f.len()
                ));
            }
            let colour = |i: usize| {
                f.get(i)
                    .and_then(|s| s.trim().parse::<i16>().ok())
                    .unwrap_or(-1)
            };
            let text = |i: usize| f.get(i).map_or(String::new(), |s| s.trim().to_string());
            rows.insert(
                (text(0), text(1)),
                Row {
                    layer: text(2),
                    colour: colour(3),
                    cut_layer: text(4),
                    cut_colour: colour(5),
                },
            );
        }
        Ok(Self { rows })
    }

    /// The table an installation of the authoring tool keeps for its default export
    /// (`%ProgramData%\Autodesk\RVT <release>\exportlayers-dwg-AIA.txt`), the newest release
    /// found, with its path. `None` where there is none (always, off Windows).
    pub fn from_revit_install() -> Option<(PathBuf, Self)> {
        let base = PathBuf::from(std::env::var_os("PROGRAMDATA")?).join("Autodesk");
        (2015..=2040).rev().find_map(|release| {
            let path = base
                .join(format!("RVT {release}"))
                .join("exportlayers-dwg-AIA.txt");
            let bytes = std::fs::read(&path).ok()?;
            let table = Self::parse(&String::from_utf8_lossy(&bytes)).ok()?;
            Some((path, table))
        })
    }

    /// How many rows the table has.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether the table has no row.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The colour number of the first row (by category, then subcategory) drawn on `layer`, where
    /// one gives a colour.
    pub fn colour_of(&self, layer: &str) -> Option<i16> {
        let mut rows: Vec<(&(String, String), &Row)> = self.rows.iter().collect();
        rows.sort_by(|a, b| a.0.cmp(b.0));
        rows.into_iter()
            .find(|(_, r)| r.layer == layer && r.colour >= 0)
            .map(|(_, r)| r.colour)
    }

    /// The layer and colour of `category`'s `subcategory` (`""` for the category itself), cut or
    /// seen: its own row where that names a layer, else its category's. `None` where neither
    /// row names one.
    pub fn layer(&self, category: &str, subcategory: &str, cut: bool) -> Option<(&str, i16)> {
        let pick = |key: (&str, &str)| {
            let row = self.rows.get(&(key.0.to_string(), key.1.to_string()))?;
            let (layer, colour) = if cut && !row.cut_layer.is_empty() {
                (&row.cut_layer, row.cut_colour)
            } else {
                (&row.layer, row.colour)
            };
            (!layer.is_empty()).then_some((layer.as_str(), colour))
        };
        pick((category, subcategory)).or_else(|| pick((category, "")))
    }
}

/// What DXF and DWG output names its layers by: the table, and each wall's type Function (by the
/// wall's ElementId) where it was read.
#[derive(Debug, Clone, Default)]
pub struct ExportLayers {
    /// The table; `None` is the built-in one.
    pub table: Option<Arc<LayerTable>>,
    /// Each wall's type Function (`WallFunction` code), by the wall's ElementId.
    pub wall_functions: BTreeMap<u64, i64>,
}

/// Which line of an element is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Line {
    /// Where the view's cut plane crosses it.
    Cut,
    /// Its edges seen beyond the cut plane.
    Seen,
    /// Its edges above the cut plane, drawn as overhead lines (a plan's stairs and railings).
    Overhead,
}

/// The subcategories whose rows give a category's overhead lines, in the order tried.
const OVERHEAD_SUBCATEGORIES: [&str; 2] = ["<Above> Outlines", "<Above> Railings Cut Line"];

impl ExportLayers {
    /// With `table` in place of the built-in one.
    pub fn with_table(table: LayerTable) -> Self {
        Self {
            table: Some(Arc::new(table)),
            ..Self::default()
        }
    }

    /// The table in use.
    pub fn table(&self) -> &LayerTable {
        self.table.as_deref().unwrap_or(LayerTable::builtin())
    }

    /// Read the type Function of `model`'s walls from `rf`, for walls drawn on their Function's
    /// row. Walls whose Function is not read keep their `IsExternal` choice.
    pub fn read_wall_functions(
        &mut self,
        rf: &mut crate::RevitFile,
        model: &super::IfcModel,
    ) -> anyhow::Result<()> {
        let walls: BTreeSet<u64> = model
            .entities
            .iter()
            .filter_map(|e| match e {
                super::entities::IfcEntity::BuildingElement {
                    ifc_type,
                    type_guid,
                    ..
                } if ifc_type.starts_with("IFCWALL") || ifc_type == "IFCCURTAINWALL" => {
                    type_guid.as_deref()?.parse().ok()
                }
                _ => None,
            })
            .collect();
        self.wall_functions = crate::native_wall_types::wall_type_functions(rf, &walls)?;
        Ok(())
    }

    /// The layer and colour an element of `category` (`id` its ElementId, `external` its
    /// `IsExternal`) draws `line` on. `None` where the table has no row for it.
    pub fn element_layer(
        &self,
        category: &str,
        id: Option<u64>,
        external: Option<bool>,
        line: Line,
    ) -> Option<(String, i16)> {
        let table = self.table();
        let cut = line == Line::Cut;
        // Rooms are drawn as the export's room polylines.
        let category = if category == "Rooms" {
            "Room Polylines"
        } else {
            category
        };
        if line == Line::Overhead {
            if let Some((l, c)) = OVERHEAD_SUBCATEGORIES.iter().find_map(|s| {
                table
                    .rows
                    .get(&(category.to_string(), s.to_string()))
                    .filter(|r| !r.layer.is_empty())
                    .map(|r| (r.layer.clone(), r.colour))
            }) {
                return Some((l, c));
            }
            let (l, c) = self.element_layer(category, id, external, Line::Seen)?;
            return Some((format!("{l}-MBND"), c));
        }
        if category == "Walls" {
            let function =
                id.and_then(|id| self.wall_functions.get(&id).copied())
                    .or(match external {
                        Some(true) => Some(1),
                        Some(false) => Some(0),
                        None => None,
                    });
            if let Some(found) = function
                .and_then(wall_function_category)
                .and_then(|special| table.layer(special, "", cut))
            {
                return Some((found.0.to_string(), found.1));
            }
        }
        table
            .layer(category, "", cut)
            .map(|(l, c)| (l.to_string(), c))
    }

    /// The layer and colour of an annotation drawn on `builtin` (one of the built-in table's
    /// layers: `S-GRID`, `G-ANNO-TEXT`, …), from the row of the category it stands for. Other
    /// layers keep their name and `colour`.
    pub fn annotation_layer(&self, builtin: &str, colour: i16) -> (String, i16) {
        const ANNOTATION: [(&str, &str, &str); 15] = [
            ("A-DETL", "Lines", ""),
            ("A-AREA", "Lines", "<Room Separation>"),
            ("C-PROP-LINE", "Site", "Property Lines"),
            ("C-BLDG-PADS", "Site", "Pads"),
            ("A-FLOR", "Floors", ""),
            ("C-TOPO", "Toposolid", ""),
            ("G-ANNO-TEXT", "Text Notes", ""),
            ("A-AREA-IDEN", "Room Tags", ""),
            ("S-GRID", "Grids", ""),
            ("S-GRID-IDEN", "Grid Heads", ""),
            ("A-ANNO-DIMS", "Dimensions", ""),
            ("A-FLOR-LEVL", "Levels", ""),
            ("G-ANNO-TTLB", "Title Blocks", ""),
            ("G-ANNO-NPLT", "Viewports", ""),
            ("A-AREA-BNDY", "Room Polylines", ""),
        ];
        ANNOTATION
            .iter()
            .find(|(b, _, _)| *b == builtin)
            .and_then(|(_, category, sub)| self.table().layer(category, sub, false))
            .map_or((builtin.to_string(), colour), |(l, c)| {
                (l.to_string(), if c < 0 { colour } else { c })
            })
    }
}
