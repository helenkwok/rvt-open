//! Grids: each grid line's name and ends, from its native record.
//!
//! A `Grid` record names its datum plane (`m_pSurface`): a `Plane` whose
//! origin is the line's start, whose X vector runs along the line and whose
//! envelope's first coordinates span its length; `m_text` is its name. Revit's
//! IFC4 export writes the grids as an `IfcGrid` of `IfcGridAxis` curves, one
//! axis per grid named after it, the lines across the plan in its first list
//! and those along it in its second (tiny reference models set 2, recipe 20,
//! Revit 2024: grids 1 to 3 at x = 0, 6 and 12 m from y = -2 to 8 m, and 4
//! and 5 at y = 0 and 6 m from x = -2 to 14 m, as these ends read).

use super::IfcModel;
use crate::RevitFile;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One grid line: its ElementId, its name and its ends, plan model feet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GridLine {
    pub id: u32,
    pub name: String,
    pub start: [f64; 2],
    pub end: [f64; 2],
}

/// The `BuiltInCategory` of a grid, `OST_Grids`.
pub const OST_GRIDS: i64 = -2_000_220;

/// Every straight grid line of the file whose record is read, by ElementId.
/// A grid in a design option, or on an arc, is left out.
pub fn read(rf: &mut RevitFile) -> Vec<GridLine> {
    let options = crate::native_document::Options {
        selected_classes: ["Grid".to_string()].into(),
        ..Default::default()
    };
    let mut grids = std::collections::BTreeMap::new();
    let _ = crate::native_document::extract_graphs(rf, &options, |record| {
        if record.class_name.as_deref() != Some("Grid") {
            return Ok(());
        }
        let Some(graph) = record.graph.as_ref() else {
            return Ok(());
        };
        let Ok(id) = u32::try_from(record.identity.element_id) else {
            return Ok(());
        };
        if let Some(line) = grid_line(graph, id) {
            grids.insert(id, line);
        }
        Ok(())
    });
    grids.into_values().collect()
}

fn grid_line(graph: &crate::native_parameters::ObjectGraph, id: u32) -> Option<GridLine> {
    let root = &graph.objects.first()?.fields;
    let in_option = root
        .pointer("/m_designOptionId/m_id")
        .and_then(crate::native_json::identifier_i64)
        .is_some_and(|option| option != -1);
    if in_option {
        return None;
    }
    let name = root.get("m_text")?.as_str()?.to_string();
    let token = root.pointer("/m_pSurface/pointer_token")?.as_u64()?;
    let plane = graph
        .objects
        .iter()
        .find(|object| u64::from(object.token) == token && object.class_name == "Plane")?;
    let triple = |key: &str| -> Option<[f64; 3]> {
        let v = plane.fields.get(key)?.as_array()?;
        Some([
            v.first()?.as_f64()?,
            v.get(1)?.as_f64()?,
            v.get(2)?.as_f64()?,
        ])
    };
    let (origin, x) = (triple("m_origin")?, triple("m_xVec")?);
    let corners = plane.fields.pointer("/m_Envelope/m_corners")?.as_array()?;
    let along = |corner: &Value| corner.as_array()?.first()?.as_f64();
    let (a, b) = (along(corners.first()?)?, along(corners.get(1)?)?);
    // A vertical datum plane: its X vector is level.
    if x[2].abs() > 1e-9 || !(b - a).is_finite() || (b - a).abs() < 1e-9 {
        return None;
    }
    let at = |t: f64| [origin[0] + x[0] * t, origin[1] + x[1] * t];
    Some(GridLine {
        id,
        name,
        start: at(a),
        end: at(b),
    })
}

/// The grid lines in the `IfcGrid`'s two lists, by index into
/// `model.grids`: those along the first line, then the others. Where all
/// run one way the last goes in the second list, which may not be empty.
/// `None` for fewer than two lines, which make no `IfcGrid`.
pub(crate) fn axes(model: &IfcModel) -> Option<(Vec<usize>, Vec<usize>)> {
    let first = model.grids.first()?;
    if model.grids.len() < 2 {
        return None;
    }
    let dir = |g: &GridLine| {
        let (dx, dy) = (g.end[0] - g.start[0], g.end[1] - g.start[1]);
        let n = dx.hypot(dy);
        [dx / n, dy / n]
    };
    let d0 = dir(first);
    let (mut u, mut v): (Vec<usize>, Vec<usize>) = (0..model.grids.len()).partition(|&i| {
        let d = dir(&model.grids[i]);
        (d[0] * d0[1] - d[1] * d0[0]).abs() < 1e-6
    });
    if v.is_empty() {
        v.extend(u.pop());
    }
    Some((u, v))
}
