//! Sheets and views as drawings: the band, crop and annotation of a plan view, the projection of
//! a section or elevation, and a sheet with its views in viewports, title block and notes. What
//! `rvt-sheet --view` and `--sheet` draw, shared with the browser viewer.

use super::IfcModel;
#[cfg(feature = "dwg")]
use super::sheet::SheetOptions;
use super::sheet::{CropRect, PlanAnnotation, PlanLine, PlanText, ViewWindow};
use crate::plan_views::{LineKind, PlanView, ViewFacts};

/// Millimetres in a foot.
pub const MM_PER_FOOT: f64 = 304.8;
/// BuiltInCategory ids: Lines, Grids.
const LINES: i64 = -2_000_051;
const GRIDS: i64 = -2_000_220;

/// A plan view's band (view depth to cut plane), crop and annotation.
pub fn view_window(
    model: &IfcModel,
    facts: &ViewFacts,
    view: &PlanView,
) -> Result<(ViewWindow, PlanAnnotation), String> {
    let level_elevation = view.level.as_ref().and_then(|name| {
        model
            .building_storeys
            .iter()
            .find(|s| &s.name == name)
            .map(|s| s.elevation_feet)
    });
    let cut = view
        .cut_plane
        .or_else(|| level_elevation.map(|e| e + 1200.0 / MM_PER_FOOT))
        .ok_or_else(|| format!("plan view {:?}: no cut plane or level decoded", view.name))?;
    let bottom = view.bottom.or(level_elevation).unwrap_or(cut).min(cut);
    // The crop applies when the view crops (`on`) and a side face is active.
    let crop = view
        .crop
        .as_ref()
        .filter(|c| c.on && c.active[..2].iter().flatten().any(|a| *a))
        .map(|c| {
            let [x, y, _] = c.axes[0];
            let n = x.hypot(y);
            let x_axis = if n > 1e-9 { [x / n, y / n] } else { [1.0, 0.0] };
            // The view's y axis is x turned a quarter left, or the opposite (a mirrored box):
            // then its y range is negated.
            let left = [-x_axis[1], x_axis[0]];
            let flipped = c.axes[1][0] * left[0] + c.axes[1][1] * left[1] < 0.0;
            let (y0, y1) = if flipped {
                (-c.max[1], -c.min[1])
            } else {
                (c.min[1], c.max[1])
            };
            CropRect {
                origin: [c.origin[0], c.origin[1]],
                x_axis,
                min: [c.min[0], y0],
                max: [c.max[0], y1],
            }
        });
    // The view's own axes (its crop box's x axis, kept whether or not it crops, and that turned
    // a quarter left): a site plan turned to true north draws the model turned.
    let frame = view.crop.as_ref().and_then(|c| {
        let (x, y) = (c.axes[0][0], c.axes[0][1]);
        let n = x.hypot(y);
        (n > 1e-9 && ((x / n - 1.0).abs() > 1e-9 || (y / n).abs() > 1e-9))
            .then(|| [[x / n, y / n], [-y / n, x / n]])
    });
    let scale = view.scale.unwrap_or(100.0);
    // Annotation is drawn out to the view's annotation crop when that is on: the crop box grown
    // by its offsets (sheet sizes, left, right, top, bottom). With it off, text, tags and
    // dimensions are kept wherever they are. Grid lines are cut at the model crop when drawn.
    let in_crop = |p: [f64; 2]| crop.is_none_or(|c| c.contains((p[0], p[1])));
    let annotation_crop = crop.zip(view.crop.as_ref().and_then(|c| c.annotation)).map(
        |(mut c, [left, right, top, bottom])| {
            c.min[0] -= left * scale;
            c.max[0] += right * scale;
            c.max[1] += top * scale;
            c.min[1] -= bottom * scale;
            c
        },
    );
    let in_annotation_crop = |p: [f64; 2]| annotation_crop.is_none_or(|c| c.contains((p[0], p[1])));
    let hidden_element = |id: i64| view.hidden_elements.contains(&id);
    let mut a = PlanAnnotation::default();
    for t in facts
        .texts
        .iter()
        .filter(|t| t.view == view.id && in_annotation_crop([t.at[0], t.at[1]]))
    {
        a.texts.push(PlanText {
            at: [t.at[0], t.at[1]],
            value: t.value.clone(),
            // Text heights are sheet sizes.
            height_feet: t.height.unwrap_or(2.5 / MM_PER_FOOT) * scale,
            rotation: t.rotation,
            layer: "G-ANNO-TEXT".into(),
        });
    }
    for tag in facts.room_tags.iter().filter(|t| t.view == view.id) {
        let Some(room) = tag.room.and_then(|r| facts.rooms.get(&r)) else {
            continue;
        };
        let Some(at) = tag.at.map(|p| [p[0], p[1]]).or(room.point) else {
            continue;
        };
        if !in_annotation_crop(at) {
            continue;
        }
        for (i, value) in [&room.name, &room.number].into_iter().flatten().enumerate() {
            a.texts.push(PlanText {
                at: [at[0], at[1] - i as f64 * 4.0 * scale / MM_PER_FOOT],
                value: value.clone(),
                height_feet: 2.5 * scale / MM_PER_FOOT,
                rotation: 0.0,
                layer: "A-AREA-IDEN".into(),
            });
        }
    }
    let grids_hidden = facts.hidden_categories(view).contains(&GRIDS);
    // A grid head's circle, sheet radius: 1/4" in a view at an imperial scale (1/8" = 1'-0" is
    // 96), else 6.5 mm (the heads of the samples' DWG exports).
    let imperial = (scale.round() - scale).abs() < 1e-6
        && (scale.round() as i64 % 12 == 0 || (scale.round() as i64).count_ones() == 1);
    let radius = if imperial {
        0.25 / 12.0
    } else {
        6.5 / MM_PER_FOOT
    } * scale;
    for g in facts
        .grids
        .iter()
        .filter(|g| !grids_hidden && !hidden_element(g.id))
    {
        // The view's extent of the grid when it has one, else the grid's own (free end first,
        // the head at the bubble end).
        let Some((o, dir)) = g.axis else { continue };
        let at = |t: f64| [o[0] + t * dir[0], o[1] + t * dir[1]];
        let extent = facts
            .grid_extents
            .iter()
            .find(|e| e.view == view.id && e.grid == g.id);
        let (free, head, free_head, bubble_head) = match (extent, g.ends) {
            (Some(e), _) => (at(e.free), at(e.bubble), e.free_head, e.bubble_head),
            (None, Some([f, b])) => ([f[0], f[1]], [b[0], b[1]], false, true),
            (None, None) => continue,
        };
        // Grids run out to the annotation crop when it is on, else to the crop.
        let (free, head) = match annotation_crop.or(crop) {
            Some(c) => match c.clip((free[0], free[1]), (head[0], head[1])) {
                Some((p, q)) => ([p.0, p.1], [q.0, q.1]),
                None => continue,
            },
            None => (free, head),
        };
        a.lines.push(PlanLine {
            a: free,
            b: head,
            layer: "S-GRID".into(),
            outside_crop: true,
        });
        // A head: a circle just beyond its end, the grid's name in it (placed where the export's
        // text box starts, up and left of the centre).
        let mut ends = Vec::new();
        if bubble_head {
            ends.push((head, free));
        }
        if free_head {
            ends.push((free, head));
        }
        for (end, other) in ends {
            let (dx, dy) = (end[0] - other[0], end[1] - other[1]);
            let n = dx.hypot(dy);
            if n < 1e-9 {
                continue;
            }
            let centre = [end[0] + dx / n * radius, end[1] + dy / n * radius];
            let on_circle = |k: usize| {
                let t = std::f64::consts::TAU * k as f64 / 32.0;
                [centre[0] + radius * t.cos(), centre[1] + radius * t.sin()]
            };
            for k in 0..32 {
                a.lines.push(PlanLine {
                    a: on_circle(k),
                    b: on_circle(k + 1),
                    layer: "S-GRID-IDEN".into(),
                    outside_crop: true,
                });
            }
            if !g.name.is_empty() {
                a.texts.push(PlanText {
                    at: [centre[0] - 0.28 * radius, centre[1] + 0.37 * radius],
                    value: g.name.clone(),
                    height_feet: 2.5 * scale / MM_PER_FOOT,
                    rotation: 0.0,
                    layer: "S-GRID-IDEN".into(),
                });
            }
        }
    }
    for d in facts.dimensions.iter().filter(|d| d.view == view.id) {
        for w in d.points.windows(2) {
            let (p, q) = ([w[0][0], w[0][1]], [w[1][0], w[1][1]]);
            if in_annotation_crop([(p[0] + q[0]) / 2.0, (p[1] + q[1]) / 2.0]) {
                a.dimensions.push([p, q]);
            }
        }
    }
    // The project's own lines: the view's detail lines, and model and separation lines in its
    // range, unless their category is hidden.
    let hidden_ids = facts.hidden_categories(view);
    for c in &facts.curves {
        let (lo, hi) = c
            .points
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| {
                (a.min(p[2]), b.max(p[2]))
            });
        // Property lines show in every plan, whatever their height.
        let shown = match c.view {
            Some(v) => v == view.id,
            None if c.kind == LineKind::PropertyLine => true,
            None => lo <= cut + 1e-3 && hi >= bottom - 1e-3,
        };
        // Most are stored with no category (-1); a hidden Lines category hides them all.
        let hidden = hidden_ids.contains(&LINES)
            || c.category
                .is_some_and(|k| k < -1 && hidden_ids.contains(&k));
        if !shown || hidden {
            continue;
        }
        // The export's layers: `Lines` on A-DETL, `<Room Separation>` on A-AREA, `Site: Property
        // Lines` on C-PROP-LINE, a floor's, toposolid's or pad's boundary on its category's.
        let layer = match c.kind {
            LineKind::Line => "A-DETL",
            LineKind::RoomSeparation => "A-AREA",
            LineKind::PropertyLine => "C-PROP-LINE",
            LineKind::Floor => "A-FLOR",
            LineKind::Toposolid => "C-TOPO",
            LineKind::BuildingPad => "C-BLDG-PADS",
        };
        for w in c.points.windows(2) {
            let (p, q) = ([w[0][0], w[0][1]], [w[1][0], w[1][1]]);
            if crop.is_some() && !(in_crop(p) || in_crop(q)) {
                continue;
            }
            a.lines.push(PlanLine {
                a: p,
                b: q,
                layer: layer.into(),
                outside_crop: false,
            });
        }
    }
    // Hidden categories (the view template's when one controls the view), by the names elements
    // carry.
    let hidden_categories: Vec<String> = hidden_ids
        .iter()
        .filter_map(|id| crate::builtin_parameters::category_name(*id))
        .map(String::from)
        .collect();
    Ok((
        ViewWindow {
            bottom_feet: bottom,
            top_feet: cut,
            crop,
            hidden_categories,
            hidden_elements: view.hidden_elements.clone(),
            frame,
        },
        a,
    ))
}

#[cfg(feature = "dwg")]
/// A sheet: each plan view on it drawn in model space, side by side, and shown through a
/// viewport where the view sits on the sheet (its internal origin at the viewport's `offset`, at
/// the view's scale); the title blocks' lines, the sheet's text notes and detail lines and each
/// view's title in paper space (`Layout1`). Millimetres.
pub fn sheet_drawing(
    rf: &mut crate::RevitFile,
    model: &IfcModel,
    facts: &ViewFacts,
    sheet: &crate::plan_views::Sheet,
    symbols: &std::collections::BTreeMap<usize, Vec<super::entities::PlanPolyline>>,
    notes: &mut Vec<String>,
) -> Result<rvt_dwg::Drawing2d, String> {
    use rvt_dwg::{Affine, Entity2d, Geom, Layer2d, Layout2d};
    const PAPER: &str = "*Paper_Space";
    let mm = |p: [f64; 2]| [p[0] * MM_PER_FOOT, p[1] * MM_PER_FOOT, 0.0];
    let mut out = rvt_dwg::Drawing2d {
        version: "AC1032".into(),
        units: 4,
        code_page: "ANSI_1252".into(),
        layers: vec![Layer2d::new("0", 7)],
        layouts: vec![
            Layout2d {
                name: "Model".into(),
                block: rvt_dwg::MODEL_SPACE.into(),
                tab_order: 0,
                is_model: true,
                limits: None,
            },
            Layout2d {
                name: "Layout1".into(),
                block: PAPER.into(),
                tab_order: 1,
                is_model: false,
                limits: None,
            },
        ],
        ..Default::default()
    };
    let ensure = |d: &mut rvt_dwg::Drawing2d, layer: &str, aci: i16| {
        if !d.layers.iter().any(|l| l.name == layer) {
            d.layers.push(Layer2d::new(layer, aci));
        }
    };
    let paper = |kind: &str, layer: &str, geom: Geom| {
        let mut e = Entity2d::new(kind, layer, geom);
        e.owner = PAPER.into();
        e
    };
    let mut next_x = 0.0;
    let mut viewport_id = 2;
    // The sheet's own viewport (a view of itself) is not a view placed on it.
    if !facts
        .viewports
        .iter()
        .any(|v| v.sheet == sheet.id && v.view != sheet.id)
    {
        notes.push(format!(
            "sheet {}: no view is placed on it, so only its title block and notes are drawn",
            sheet.number
        ));
    }
    for vp in facts.viewports.iter().filter(|v| v.sheet == sheet.id) {
        let plan = facts.views.iter().find(|v| v.id == vp.view && !v.template);
        let section = facts
            .sections
            .iter()
            .find(|v| v.id == vp.view && !v.template);
        let (d, scale, name) = match (plan, section) {
            (Some(view), _) => {
                let (window, annotation) = view_window(model, facts, view)?;
                let options = SheetOptions {
                    window: Some(window),
                    annotation: Some(annotation),
                    symbols: symbols.clone(),
                    ..SheetOptions::default()
                };
                (
                    super::sheet::plan_drawing(model, &options),
                    view.scale.unwrap_or(100.0),
                    view.name.clone(),
                )
            }
            (None, Some(view)) => {
                let Some(window) = section_window(facts, view) else {
                    notes.push(format!(
                        "sheet {}: view {:?} has no crop box; not drawn",
                        sheet.number, view.name
                    ));
                    continue;
                };
                (
                    super::sheet::projected_drawing(model, &window),
                    view.scale.unwrap_or(100.0),
                    view.name.clone(),
                )
            }
            _ => {
                notes.push(format!(
                    "sheet {}: view {} is not a plan, section or elevation view; not drawn yet",
                    sheet.number, vp.view
                ));
                continue;
            }
        };
        let Some(b) = rvt_dwg::bounds(d.entities.iter().flat_map(|e| e.geom.samples(f64::MAX)))
        else {
            continue;
        };
        // Model space holds the views side by side, each moved along x clear of the last.
        let shift = next_x - b[0];
        next_x += b[2] - b[0] + (20.0 * scale).max(10_000.0);
        let t = Affine {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: shift,
            f: 0.0,
        };
        for l in d.layers {
            if !out.layers.iter().any(|x| x.name == l.name) {
                out.layers.push(l);
            }
        }
        for mut e in d.entities {
            e.geom = e.geom.transformed(&t);
            out.entities.push(e);
        }
        // Paper = offset + view coordinates / scale.
        let c = [(b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0];
        let o = mm(vp.offset);
        // The frame holds what the view draws with 2 mm to spare, so nothing lies on its edge.
        let (w, h) = ((b[2] - b[0]) / scale + 4.0, (b[3] - b[1]) / scale + 4.0);
        ensure(&mut out, "G-ANNO-NPLT", 7);
        out.entities.push(paper(
            "VIEWPORT",
            "G-ANNO-NPLT",
            Geom::Viewport {
                center: [o[0] + c[0] / scale, o[1] + c[1] / scale, 0.0],
                width: w,
                height: h,
                view_center: [c[0] + shift, c[1]],
                view_height: h * scale,
                id: viewport_id,
            },
        ));
        viewport_id += 1;
        // The view's title: its name at the title's origin, over a line of the title's length.
        if vp.label_length > 0.0 {
            ensure(&mut out, "G-ANNO-TTLB", 7);
            let at = mm(vp.label_origin);
            out.entities.push(paper(
                "LINE",
                "G-ANNO-TTLB",
                Geom::Line {
                    a: at,
                    b: [at[0] + vp.label_length * MM_PER_FOOT, at[1], 0.0],
                },
            ));
            out.entities.push(paper(
                "TEXT",
                "G-ANNO-TTLB",
                Geom::Text {
                    at: [at[0], at[1] + 2.0, 0.0],
                    height: 3.5,
                    rotation: 0.0,
                    value: name,
                },
            ));
        }
    }
    // Title blocks: their symbol's lines, placed by the instance.
    let blocks: Vec<&crate::plan_views::TitleBlock> = facts
        .title_blocks
        .iter()
        .filter(|t| t.sheet == sheet.id)
        .collect();
    let wanted: std::collections::BTreeSet<u64> = blocks.iter().map(|t| t.symbol as u64).collect();
    let graphics = if wanted.is_empty() {
        Default::default()
    } else {
        super::plan_maps::symbol_sheet_graphics(rf, &wanted).unwrap_or_else(|e| {
            notes.push(format!("title block lines not read: {e}"));
            Default::default()
        })
    };
    for t in blocks {
        let Some((cs, texts)) = u32::try_from(t.symbol).ok().and_then(|s| graphics.get(&s)) else {
            notes.push(format!(
                "sheet {}: title block {} drew nothing",
                sheet.number, t.id
            ));
            continue;
        };
        ensure(&mut out, "G-ANNO-TTLB", 7);
        let [c, s] = t.x_dir;
        let place = |p: (f64, f64)| {
            mm([
                t.origin[0] + p.0 * c - p.1 * s,
                t.origin[1] + p.0 * s + p.1 * c,
            ])
        };
        for curve in cs {
            for w in super::sheet::symbol_points(curve).windows(2) {
                out.entities.push(paper(
                    "LINE",
                    "G-ANNO-TTLB",
                    Geom::Line {
                        a: place(w[0]),
                        b: place(w[1]),
                    },
                ));
            }
        }
        for text in texts {
            out.entities.push(paper(
                "TEXT",
                "G-ANNO-TTLB",
                Geom::Text {
                    at: place((text.at[0], text.at[1])),
                    height: text.height_feet * MM_PER_FOOT,
                    rotation: text.rotation + s.atan2(c),
                    value: text.value.clone(),
                },
            ));
        }
    }
    // The sheet's own text notes and detail lines, sheet sizes.
    for t in facts.texts.iter().filter(|t| t.view == sheet.id) {
        ensure(&mut out, "G-ANNO-TEXT", 7);
        out.entities.push(paper(
            "TEXT",
            "G-ANNO-TEXT",
            Geom::Text {
                at: mm([t.at[0], t.at[1]]),
                height: t.height.unwrap_or(2.5 / MM_PER_FOOT) * MM_PER_FOOT,
                rotation: t.rotation,
                value: t.value.clone(),
            },
        ));
    }
    for c in facts.curves.iter().filter(|c| c.view == Some(sheet.id)) {
        ensure(&mut out, "A-DETL", 7);
        for w in c.points.windows(2) {
            out.entities.push(paper(
                "LINE",
                "A-DETL",
                Geom::Line {
                    a: mm([w[0][0], w[0][1]]),
                    b: mm([w[1][0], w[1][1]]),
                },
            ));
        }
    }
    let limits = rvt_dwg::bounds(
        out.entities
            .iter()
            .filter(|e| e.owner == PAPER)
            .flat_map(|e| e.geom.samples(f64::MAX)),
    );
    out.layouts[1].limits = limits;
    Ok(out)
}

#[cfg(feature = "dwg")]
/// A section or elevation's axes, crop box and hidden categories and elements.
pub fn section_window(
    facts: &ViewFacts,
    view: &crate::plan_views::SectionView,
) -> Option<super::sheet::ProjectedWindow> {
    let c = view.crop.as_ref()?;
    Some(super::sheet::ProjectedWindow {
        right: view.right,
        up: view.up,
        toward_viewer: view.toward_viewer,
        crop_origin: c.origin,
        crop_axes: c.axes,
        crop_min: c.min,
        crop_max: c.max,
        hidden_categories: facts
            .section_hidden_categories(view)
            .iter()
            .filter_map(|id| crate::builtin_parameters::category_name(*id))
            .map(String::from)
            .collect(),
        hidden_elements: view.hidden_elements.clone(),
    })
}

/// Which sheet or view to draw: a sheet, a plan view, or a section or elevation, by ElementId.
#[cfg(feature = "dwg")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrawingKey {
    Sheet(i64),
    Plan(i64),
    Section(i64),
}

/// A sheet or view drawn: the drawing, the layout a PDF draws (`Layout1` for a sheet, `None` for
/// a view's model space) and the scale a view's PDF is drawn at.
#[cfg(feature = "dwg")]
#[derive(Debug, Clone)]
pub struct ViewDrawing {
    pub drawing: rvt_dwg::Drawing2d,
    pub layout: Option<&'static str>,
    pub scale: f64,
}

/// Draw `key` as `rvt-sheet --sheet` or `--view` draws it to DXF, DWG or PDF. `symbols` are the
/// family instances' plan symbols ([`super::plan_maps::instance_plan_symbols`]); what is not
/// drawn is said in `notes`.
#[cfg(feature = "dwg")]
pub fn view_drawing(
    rf: &mut crate::RevitFile,
    model: &IfcModel,
    facts: &ViewFacts,
    key: DrawingKey,
    symbols: &std::collections::BTreeMap<usize, Vec<super::entities::PlanPolyline>>,
    notes: &mut Vec<String>,
) -> Result<ViewDrawing, String> {
    match key {
        DrawingKey::Sheet(id) => {
            let sheet = facts
                .sheets
                .iter()
                .find(|s| s.id == id)
                .ok_or_else(|| format!("no sheet {id}"))?;
            Ok(ViewDrawing {
                drawing: sheet_drawing(rf, model, facts, sheet, symbols, notes)?,
                layout: Some("Layout1"),
                scale: 1.0,
            })
        }
        DrawingKey::Plan(id) => {
            let view = facts
                .views
                .iter()
                .find(|v| v.id == id && !v.template)
                .ok_or_else(|| format!("no plan view {id}"))?;
            let (window, annotation) = view_window(model, facts, view)?;
            let options = SheetOptions {
                window: Some(window),
                annotation: Some(annotation),
                symbols: symbols.clone(),
                ..SheetOptions::default()
            };
            Ok(ViewDrawing {
                drawing: super::sheet::plan_drawing(model, &options),
                layout: None,
                scale: view.scale.unwrap_or(100.0),
            })
        }
        DrawingKey::Section(id) => {
            let view = facts
                .sections
                .iter()
                .find(|v| v.id == id && !v.template)
                .ok_or_else(|| format!("no section or elevation {id}"))?;
            let window = section_window(facts, view)
                .ok_or_else(|| format!("{:?} has no crop box decoded", view.name))?;
            Ok(ViewDrawing {
                drawing: super::sheet::projected_drawing(model, &window),
                layout: None,
                scale: view.scale.unwrap_or(100.0),
            })
        }
    }
}

/// A drawing as `dxf`, `dwg` (in `version`: 2018, 2013, ...) or `pdf` (`layout` drawn: a sheet;
/// `None` for model space at 1:`scale`), with the writer's notes.
#[cfg(feature = "dwg")]
pub fn drawing_bytes(
    drawing: &rvt_dwg::Drawing2d,
    layout: Option<&str>,
    scale: f64,
    format: &str,
    version: &str,
) -> Result<(Vec<u8>, Vec<String>), String> {
    use rvt_dwg::write::{CadFormat, parse_version, write_drawing};
    let format = match format {
        "pdf" => {
            return rvt_dwg::pdf::render(drawing, layout, scale, &rvt_dwg::Limits::DEFAULT)
                .map(|p| (p.bytes, Vec::new()))
                .map_err(|e| e.to_string());
        }
        "dwg" => CadFormat::Dwg,
        "dxf" => CadFormat::Dxf,
        other => return Err(format!("unknown format {other:?}: dxf, dwg or pdf")),
    };
    let version = parse_version(version, format).map_err(|e| e.to_string())?;
    write_drawing(drawing, format, version).map_err(|e| e.to_string())
}
