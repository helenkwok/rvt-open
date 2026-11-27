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
/// BuiltInCategory ids: Lines, Grids, Property Lines.
const LINES: i64 = -2_000_051;
const GRIDS: i64 = -2_000_220;
const PROPERTY_LINES: i64 = -2_001_265;

/// Whether any part of segment `p`-`q` lies in `crop` (always, with no crop): a line crossing
/// the crop with both ends outside it still shows its middle, which the drawing clips to.
fn segment_in_crop(crop: Option<&CropRect>, p: [f64; 2], q: [f64; 2]) -> bool {
    crop.is_none_or(|c| c.clip((p[0], p[1]), (q[0], q[1])).is_some())
}

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
            element: Some(t.id),
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
                element: Some(tag.id),
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
            element: Some(g.id),
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
                    element: Some(g.id),
                });
            }
            if !g.name.is_empty() {
                a.texts.push(PlanText {
                    at: [centre[0] - 0.28 * radius, centre[1] + 0.37 * radius],
                    value: g.name.clone(),
                    height_feet: 2.5 * scale / MM_PER_FOOT,
                    rotation: 0.0,
                    layer: "S-GRID-IDEN".into(),
                    element: Some(g.id),
                });
            }
        }
    }
    for d in facts.dimensions.iter().filter(|d| d.view == view.id) {
        for w in d.points.windows(2) {
            let (p, q) = ([w[0][0], w[0][1]], [w[1][0], w[1][1]]);
            if in_annotation_crop([(p[0] + q[0]) / 2.0, (p[1] + q[1]) / 2.0]) {
                a.dimensions.push([p, q]);
                a.dimension_elements.push(d.id);
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
        // Most are stored with no category (-1); a hidden Lines category hides them all, and a
        // hidden Property Lines category a property's (the Technical School sample's floor plan
        // hides it, and its sheet A1 in the authoring tool's PDF shows none).
        let hidden = hidden_ids.contains(&LINES)
            || (c.kind == LineKind::PropertyLine && hidden_ids.contains(&PROPERTY_LINES))
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
            if !segment_in_crop(crop.as_ref(), p, q) {
                continue;
            }
            a.lines.push(PlanLine {
                a: p,
                b: q,
                layer: layer.into(),
                outside_crop: false,
                element: Some(c.id),
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
    layers: &super::dwg_layers::ExportLayers,
    notes: &mut Vec<String>,
) -> Result<rvt_dwg::Drawing2d, String> {
    let nplt = layers.annotation_layer("G-ANNO-NPLT", 7);
    let ttlb = layers.annotation_layer("G-ANNO-TTLB", 7);
    let notes_layer = layers.annotation_layer("G-ANNO-TEXT", 7);
    let detail = layers.annotation_layer("A-DETL", 7);
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
    // The scales of the views drawn on the sheet, for its Scale label.
    let mut scales: Vec<f64> = Vec::new();
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
        // `origin`: where the view's own origin lies in what it draws (millimetres), which the
        // viewport's offset places on the sheet; `frame`: the crop region (with the annotation
        // crop when on) the viewport shows, `[x0, y0, x1, y1]` in the same millimetres.
        let (d, scale, name, origin, frame) = match (plan, section) {
            (Some(view), _) => {
                let (window, annotation) = view_window(model, facts, view)?;
                let options = SheetOptions {
                    window: Some(window),
                    annotation: Some(annotation),
                    symbols: symbols.clone(),
                    layers: layers.clone(),
                    ..SheetOptions::default()
                };
                (
                    super::sheet::plan_drawing(model, &options),
                    view.scale.unwrap_or(100.0),
                    view.name.clone(),
                    [0.0, 0.0],
                    None,
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
                let scale = view.scale.unwrap_or(100.0);
                let mut d = super::sheet::projected_drawing(model, &window, scale, layers);
                section_annotation(&mut d, model, facts, view, &window, scale, layers);
                let (origin, mut frame, annotation_crop) = section_frame(view, &window, scale);
                // The viewport frames the crop and what the view draws beyond it (level and grid
                // heads, notes), cut at the annotation crop when that is on: the outlines the
                // authoring tool reports for sheet A2's viewports frame what they show, not
                // their whole annotation crops.
                if let Some(f) = frame.as_mut() {
                    // A text reaches its end (its width at its em, 1.4 capital heights).
                    let ends = d.entities.iter().flat_map(|e| match &e.geom {
                        Geom::Text {
                            at,
                            height,
                            rotation,
                            value,
                        } => {
                            let w = value
                                .lines()
                                .map(|l| crate::plan_views::text_width(l, 1.4 * height))
                                .fold(0.0, f64::max);
                            // Its box: the baseline's ends and the top of its first line.
                            let (c, s) = (rotation.cos(), rotation.sin());
                            let up = 1.4 * height;
                            vec![
                                [at[0] + w * c, at[1] + w * s],
                                [at[0] - up * s, at[1] + up * c],
                                [at[0] + w * c - up * s, at[1] + w * s + up * c],
                            ]
                        }
                        _ => Vec::new(),
                    });
                    if let Some(b) = rvt_dwg::bounds(
                        d.entities
                            .iter()
                            .flat_map(|e| e.geom.samples(f64::MAX))
                            .chain(ends),
                    ) {
                        *f = [
                            f[0].min(b[0]),
                            f[1].min(b[1]),
                            f[2].max(b[2]),
                            f[3].max(b[3]),
                        ];
                    }
                    if let Some(a) = annotation_crop {
                        *f = [
                            f[0].max(a[0]),
                            f[1].max(a[1]),
                            f[2].min(a[2]),
                            f[3].min(a[3]),
                        ];
                    }
                }
                (d, scale, view.name.clone(), origin, frame)
            }
            _ => match facts.other_views.iter().find(|v| v.id == vp.view) {
                Some(v) if v.schedule => {
                    notes.push(format!(
                        "sheet {}: {:?} is a schedule; schedules are not drawn yet",
                        sheet.number, v.name
                    ));
                    continue;
                }
                Some(v) => {
                    let scale = v.scale.unwrap_or(1.0);
                    let d = drafting_drawing(rf, facts, v.id, scale, layers, notes);
                    if d.entities.is_empty() {
                        notes.push(format!(
                            "sheet {}: drafting view {:?} drew nothing",
                            sheet.number, v.name
                        ));
                    }
                    (d, scale, v.name.clone(), [0.0, 0.0], None)
                }
                None => {
                    notes.push(format!(
                        "sheet {}: view {} is not a plan, section, elevation or drafting view; not drawn yet",
                        sheet.number, vp.view
                    ));
                    continue;
                }
            },
        };
        if !scales.iter().any(|s| (s - scale).abs() < 1e-9) {
            scales.push(scale);
        }
        let drawn = rvt_dwg::bounds(d.entities.iter().flat_map(|e| e.geom.samples(f64::MAX)));
        // The viewport shows the view's crop region when it crops, else what the view draws.
        let Some(b) = frame.or(drawn) else {
            continue;
        };
        // Model space holds the views side by side, each moved along x clear of the last.
        let room = drawn.map_or(b, |d| [d[0].min(b[0]), d[1], d[2].max(b[2]), d[3]]);
        let shift = next_x - room[0];
        next_x += room[2] - room[0] + (20.0 * scale).max(10_000.0);
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
        // Paper = offset + view coordinates (from the view's origin) / scale.
        let c = [(b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0];
        let o = mm(vp.offset);
        // A crop frame is the crop itself; one around what the view draws has 2 mm to spare, so
        // nothing lies on its edge.
        let spare = if frame.is_some() { 0.0 } else { 4.0 };
        let (w, h) = ((b[2] - b[0]) / scale + spare, (b[3] - b[1]) / scale + spare);
        ensure(&mut out, &nplt.0, nplt.1);
        out.entities.push(
            paper(
                "VIEWPORT",
                &nplt.0,
                Geom::Viewport {
                    center: [
                        o[0] + (c[0] - origin[0]) / scale,
                        o[1] + (c[1] - origin[1]) / scale,
                        0.0,
                    ],
                    width: w,
                    height: h,
                    view_center: [c[0] + shift, c[1]],
                    view_height: h * scale,
                    id: viewport_id,
                },
            )
            .of_element(Some(vp.id)),
        );
        viewport_id += 1;
        // The view's title: its name at the title's origin, over a line of the title's length.
        if vp.label_length > 0.0 {
            ensure(&mut out, &ttlb.0, ttlb.1);
            let at = mm(vp.label_origin);
            out.entities.push(
                paper(
                    "LINE",
                    &ttlb.0,
                    Geom::Line {
                        a: at,
                        b: [at[0] + vp.label_length * MM_PER_FOOT, at[1], 0.0],
                    },
                )
                .of_element(Some(vp.id)),
            );
            out.entities.push(
                paper(
                    "TEXT",
                    &ttlb.0,
                    Geom::Text {
                        at: [at[0], at[1] + 2.0, 0.0],
                        height: 3.5,
                        rotation: 0.0,
                        value: name,
                    },
                )
                .of_element(Some(vp.id)),
            );
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
    // SHEET_SCALE: the views' one scale, or "As indicated" when they differ.
    let scale_text = match scales.as_slice() {
        [] => None,
        [s] => Some(format!("1 : {}", (s * 1000.0).round() / 1000.0)),
        _ => Some("As indicated".to_string()),
    };
    for t in &blocks {
        let [c, s] = t.x_dir;
        for l in facts.labels.iter().filter(|l| l.symbol == t.symbol) {
            let value: String = l
                .parts
                .iter()
                .filter_map(|(param, prefix, suffix)| {
                    let v = match *param {
                        -1_007_402 => scale_text.clone(),
                        p => sheet
                            .params
                            .get(&p)
                            .or_else(|| facts.project_info.get(&p))
                            .cloned(),
                    }?;
                    Some(format!("{prefix}{v}{suffix}"))
                })
                .collect::<Vec<_>>()
                .join(" ");
            if value.trim().is_empty() {
                continue;
            }
            ensure(&mut out, &ttlb.0, ttlb.1);
            let (text, at) = label_text(l, &value);
            // In the sheet: placed and turned by the title block.
            let at = [
                t.origin[0] + at[0] * c - at[1] * s,
                t.origin[1] + at[0] * s + at[1] * c,
            ];
            out.entities.push(
                paper(
                    "TEXT",
                    &ttlb.0,
                    Geom::Text {
                        at: mm(at),
                        height: text.0 * MM_PER_FOOT,
                        rotation: text.1 + s.atan2(c),
                        value,
                    },
                )
                .of_element(Some(t.id)),
            );
        }
    }
    for t in blocks {
        let Some((cs, texts)) = u32::try_from(t.symbol).ok().and_then(|s| graphics.get(&s)) else {
            notes.push(format!(
                "sheet {}: title block {} drew nothing",
                sheet.number, t.id
            ));
            continue;
        };
        ensure(&mut out, &ttlb.0, ttlb.1);
        let [c, s] = t.x_dir;
        let place = |p: (f64, f64)| {
            mm([
                t.origin[0] + p.0 * c - p.1 * s,
                t.origin[1] + p.0 * s + p.1 * c,
            ])
        };
        for curve in cs {
            for w in super::sheet::symbol_points(curve).windows(2) {
                out.entities.push(
                    paper(
                        "LINE",
                        &ttlb.0,
                        Geom::Line {
                            a: place(w[0]),
                            b: place(w[1]),
                        },
                    )
                    .of_element(Some(t.id)),
                );
            }
        }
        for text in texts {
            out.entities.push(
                paper(
                    "TEXT",
                    &ttlb.0,
                    Geom::Text {
                        at: place((text.at[0], text.at[1])),
                        height: text.height_feet * MM_PER_FOOT,
                        rotation: text.rotation + s.atan2(c),
                        value: text.value.clone(),
                    },
                )
                .of_element(Some(t.id)),
            );
        }
    }
    // The sheet's own text notes and detail lines, sheet sizes.
    for t in facts.texts.iter().filter(|t| t.view == sheet.id) {
        ensure(&mut out, &notes_layer.0, notes_layer.1);
        out.entities.push(
            paper(
                "TEXT",
                &notes_layer.0,
                Geom::Text {
                    at: mm([t.at[0], t.at[1]]),
                    height: t.height.unwrap_or(2.5 / MM_PER_FOOT) * MM_PER_FOOT,
                    rotation: t.rotation,
                    value: t.value.clone(),
                },
            )
            .of_element(Some(t.id)),
        );
    }
    for c in facts.curves.iter().filter(|c| c.view == Some(sheet.id)) {
        ensure(&mut out, &detail.0, detail.1);
        for w in c.points.windows(2) {
            out.entities.push(
                paper(
                    "LINE",
                    &detail.0,
                    Geom::Line {
                        a: mm([w[0][0], w[0][1]]),
                        b: mm([w[1][0], w[1][1]]),
                    },
                )
                .of_element(Some(c.id)),
            );
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

#[cfg(feature = "dwg")]
/// Where a label's `value` starts (its baseline's left end) and its height and angle, in the
/// symbol's frame: the label's point is the text's left, centre or right and its top, middle or
/// bottom.
fn label_text(l: &crate::plan_views::SymbolLabel, value: &str) -> ((f64, f64), [f64; 2]) {
    let n = l.base[0].hypot(l.base[1]);
    let (bx, by) = if n > 1e-9 {
        (l.base[0] / n, l.base[1] / n)
    } else {
        (1.0, 0.0)
    };
    // The label's height is its text's box.
    let h = crate::plan_views::text_em(l.height);
    let along = match l.align.0 {
        1 => -crate::plan_views::text_width(value, h),
        0 => -crate::plan_views::text_width(value, h) / 2.0,
        _ => 0.0,
    };
    // The baseline: an ascent below a top, a descent above a bottom, between for a middle.
    let up = match l.align.1 {
        1 => -0.905 * h,
        0 => -0.3465 * h,
        _ => 0.212 * h,
    };
    (
        (crate::plan_views::cap_height(h), by.atan2(bx)),
        [
            l.at[0] + along * bx - up * by,
            l.at[1] + along * by + up * bx,
        ],
    )
}

#[cfg(feature = "dwg")]
fn dot3(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[cfg(feature = "dwg")]
/// Where a section's own origin lies in what [`super::sheet::projected_drawing`] draws (its crop
/// box's origin, millimetres; a viewport's offset places that point on the sheet, measured on the
/// samples' sheets against the authoring tool's viewport outlines), its crop rectangle and its
/// annotation crop when that is on (the crop grown by sheet sizes, left, right, top, bottom).
/// `None` when the view does not crop.
fn section_frame(
    view: &crate::plan_views::SectionView,
    w: &super::sheet::ProjectedWindow,
    scale: f64,
) -> ([f64; 2], Option<[f64; 4]>, Option<[f64; 4]>) {
    let on_view = |p: [f64; 3]| [dot3(p, w.right) * MM_PER_FOOT, dot3(p, w.up) * MM_PER_FOOT];
    let origin = on_view(w.crop_origin);
    let Some(c) = view
        .crop
        .as_ref()
        .filter(|c| c.on && c.active[..2].iter().flatten().any(|a| *a))
    else {
        return (origin, None, None);
    };
    let corner = |x: f64, y: f64| {
        on_view([0, 1, 2].map(|i| w.crop_origin[i] + x * c.axes[0][i] + y * c.axes[1][i]))
    };
    let frame = rvt_dwg::bounds(
        [
            corner(c.min[0], c.min[1]),
            corner(c.max[0], c.min[1]),
            corner(c.min[0], c.max[1]),
            corner(c.max[0], c.max[1]),
        ]
        .into_iter(),
    );
    let annotation = frame
        .zip(c.annotation)
        .map(|(f, [left, right, top, bottom])| {
            let k = scale * MM_PER_FOOT;
            [
                f[0] - left * k,
                f[1] - bottom * k,
                f[2] + right * k,
                f[3] + top * k,
            ]
        });
    (origin, frame, annotation)
}

#[cfg(feature = "dwg")]
/// A section's own annotation, drawn into `d` in the view's axes like its elements: the text
/// notes and detail lines it owns (model feet; text heights are sheet sizes).
fn section_annotation(
    d: &mut rvt_dwg::Drawing2d,
    model: &IfcModel,
    facts: &ViewFacts,
    view: &crate::plan_views::SectionView,
    w: &super::sheet::ProjectedWindow,
    scale: f64,
    layers: &super::dwg_layers::ExportLayers,
) {
    let notes_layer = layers.annotation_layer("G-ANNO-TEXT", 7);
    let grid = layers.annotation_layer("S-GRID", 7);
    let grid_head = layers.annotation_layer("S-GRID-IDEN", 7);
    let level = layers.annotation_layer("A-FLOR-LEVL", 7);
    let detail = layers.annotation_layer("A-DETL", 7);
    use rvt_dwg::{Entity2d, Geom, Layer2d};
    let on_view = |p: [f64; 3]| {
        [
            dot3(p, w.right) * MM_PER_FOOT,
            dot3(p, w.up) * MM_PER_FOOT,
            0.0,
        ]
    };
    let layer = |d: &mut rvt_dwg::Drawing2d, (name, aci): &(String, i16)| {
        if !d.layers.iter().any(|l| &l.name == name) {
            d.layers.push(Layer2d::new(name.as_str(), *aci));
        }
    };
    // Notes and detail lines outside the annotation crop, when that is on, are not shown.
    let annotation_crop = section_frame(view, w, scale).2;
    let shown = |p: [f64; 3]| {
        annotation_crop.is_none_or(|a| p[0] >= a[0] && p[0] <= a[2] && p[1] >= a[1] && p[1] <= a[3])
    };
    let hidden_ids = facts.section_hidden_categories(view);
    for t in facts
        .texts
        .iter()
        .filter(|t| t.view == view.id && shown(on_view(t.at)))
    {
        layer(d, &notes_layer);
        d.entities.push(
            Entity2d::new(
                "TEXT",
                &notes_layer.0,
                Geom::Text {
                    at: on_view(t.at),
                    height: t.height.unwrap_or(2.5 / MM_PER_FOOT) * scale * MM_PER_FOOT,
                    rotation: dot3(t.base, w.up).atan2(dot3(t.base, w.right)),
                    value: t.value.clone(),
                },
            )
            .of_element(Some(t.id)),
        );
    }
    // Levels (the model's storeys) and grids, across the crop: a level where its height meets the
    // view (its name above and its height below the line's right end), a grid where it crosses
    // the section plane (its head above the crop). Drawn in a view whose up is the model's.
    let crop = view.crop.as_ref().filter(|_| w.up[2].abs() > 0.99);
    if let Some(c) = crop {
        let at =
            |x: f64, y: f64, z: f64| {
                on_view([0, 1, 2].map(|i| {
                    w.crop_origin[i] + x * c.axes[0][i] + y * c.axes[1][i] + z * c.axes[2][i]
                }))
            };
        let near = if dot3(c.axes[2], w.toward_viewer) >= 0.0 {
            c.max[2]
        } else {
            c.min[2]
        };
        let k = scale * MM_PER_FOOT;
        let text = |d: &mut rvt_dwg::Drawing2d, at: [f64; 3], value: String, layer: &str| {
            d.entities.push(Entity2d::new(
                "TEXT",
                layer,
                Geom::Text {
                    at,
                    height: 2.5 / MM_PER_FOOT * k,
                    rotation: 0.0,
                    value,
                },
            ));
        };
        let line = |d: &mut rvt_dwg::Drawing2d, a: [f64; 3], b: [f64; 3], layer: &str| {
            d.entities
                .push(Entity2d::new("LINE", layer, Geom::Line { a, b }));
        };
        // BuiltInCategory Levels.
        if !hidden_ids.contains(&-2_000_240) {
            for l in &model.building_storeys {
                let z = l.elevation_feet;
                // The level's height in the crop's y (the view's up is the model's z).
                let y = (z - w.crop_origin[2]) * c.axes[1][2];
                if y < c.min[1] || y > c.max[1] {
                    continue;
                }
                layer(d, &level);
                let first = d.entities.len();
                let (a, b) = (at(c.min[0], y, near), at(c.max[0], y, near));
                line(d, a, b, &level.0);
                let right = if a[0] > b[0] { a } else { b };
                let gap = 1.5 / MM_PER_FOOT * k;
                text(
                    d,
                    [right[0] + gap, right[1] + gap, 0.0],
                    l.name.clone(),
                    &level.0,
                );
                text(
                    d,
                    [
                        right[0] + gap,
                        right[1] - 2.0 * gap - 2.5 / MM_PER_FOOT * k,
                        0.0,
                    ],
                    format!("{}", (z * MM_PER_FOOT).round()),
                    &level.0,
                );
                // The level's ElementId, by its name.
                let id = facts.levels.iter().find(|v| v.name == l.name).map(|v| v.id);
                for e in &mut d.entities[first..] {
                    e.element = id;
                }
            }
        }
        if !hidden_ids.contains(&GRIDS) {
            let plane = [0, 1, 2].map(|i| w.crop_origin[i] + near * c.axes[2][i]);
            let radius = 6.5 / MM_PER_FOOT * k;
            for g in facts
                .grids
                .iter()
                .filter(|g| !view.hidden_elements.contains(&g.id))
            {
                let Some((o, dir)) = g.axis else { continue };
                let along = dot3(dir, c.axes[2]);
                if along.abs() < 0.2 {
                    continue;
                }
                let t = dot3([0, 1, 2].map(|i| plane[i] - o[i]), c.axes[2]) / along;
                let p = [0, 1, 2].map(|i| o[i] + t * dir[i] - w.crop_origin[i]);
                let x = dot3(p, c.axes[0]);
                if x < c.min[0] || x > c.max[0] {
                    continue;
                }
                layer(d, &grid);
                layer(d, &grid_head);
                let first = d.entities.len();
                let (a, top) = (at(x, c.min[1], near), at(x, c.max[1], near));
                line(d, a, top, &grid.0);
                let centre = [top[0], top[1] + radius];
                for i in 0..32 {
                    let p = |i: usize| {
                        let t = std::f64::consts::TAU * i as f64 / 32.0;
                        [
                            centre[0] + radius * t.cos(),
                            centre[1] + radius * t.sin(),
                            0.0,
                        ]
                    };
                    line(d, p(i), p(i + 1), &grid_head.0);
                }
                let tw = crate::plan_views::text_width(&g.name, 1.4 * 2.5 / MM_PER_FOOT * k);
                text(
                    d,
                    [
                        centre[0] - tw / 2.0,
                        centre[1] - 0.36 * 2.5 / MM_PER_FOOT * k,
                        0.0,
                    ],
                    g.name.clone(),
                    &grid_head.0,
                );
                for e in &mut d.entities[first..] {
                    e.element = Some(g.id);
                }
            }
        }
    }
    // As in a plan: a hidden Lines category hides them all, a hidden category its own.
    let lines_hidden = hidden_ids.contains(&LINES);
    for c in facts.curves.iter().filter(|c| {
        c.view == Some(view.id)
            && !lines_hidden
            && !c
                .category
                .is_some_and(|k| k < -1 && hidden_ids.contains(&k))
    }) {
        layer(d, &detail);
        for s in c.points.windows(2) {
            // The part inside the annotation crop.
            let Some((a, b)) = clip_to_rect(on_view(s[0]), on_view(s[1]), annotation_crop) else {
                continue;
            };
            d.entities
                .push(Entity2d::new("LINE", &detail.0, Geom::Line { a, b }).of_element(Some(c.id)));
        }
    }
}

#[cfg(feature = "dwg")]
/// The part of segment `a`-`b` inside `rect` (`[x0, y0, x1, y1]`; all of it with no rect), by
/// Liang-Barsky.
fn clip_to_rect(a: [f64; 3], b: [f64; 3], rect: Option<[f64; 4]>) -> Option<([f64; 3], [f64; 3])> {
    let Some(r) = rect else {
        return Some((a, b));
    };
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for (num, den) in [
        (a[0] - r[0], -dx),
        (r[2] - a[0], dx),
        (a[1] - r[1], -dy),
        (r[3] - a[1], dy),
    ] {
        if den == 0.0 {
            if num < 0.0 {
                return None;
            }
        } else if den < 0.0 {
            t0 = t0.max(num / den);
        } else {
            t1 = t1.min(num / den);
        }
    }
    let at = |t: f64| [a[0] + t * dx, a[1] + t * dy, 0.0];
    (t0 < t1).then(|| (at(t0), at(t1)))
}

#[cfg(feature = "dwg")]
/// A drafting view: the text notes and detail lines it owns and the lines of the drawings
/// imported into it, in its own 2D coordinates (model feet at its scale; text heights are sheet
/// sizes), millimetres. An imported drawing's texts are not read yet.
fn drafting_drawing(
    rf: &mut crate::RevitFile,
    facts: &ViewFacts,
    id: i64,
    scale: f64,
    layers: &super::dwg_layers::ExportLayers,
    notes: &mut Vec<String>,
) -> rvt_dwg::Drawing2d {
    let notes_layer = layers.annotation_layer("G-ANNO-TEXT", 7);
    let detail = layers.annotation_layer("A-DETL", 7);
    let import = layers.annotation_layer("G-ANNO-IMPT", 7);
    use rvt_dwg::{Drawing2d, Entity2d, Geom, Layer2d, Layout2d, MODEL_SPACE};
    let mm = |p: [f64; 3]| [p[0] * MM_PER_FOOT, p[1] * MM_PER_FOOT, 0.0];
    let mut d = Drawing2d {
        version: "AC1032".into(),
        units: 4,
        code_page: "ANSI_1252".into(),
        layers: vec![Layer2d::new("0", 7)],
        layouts: vec![Layout2d {
            name: "Model".into(),
            block: MODEL_SPACE.into(),
            tab_order: 0,
            is_model: true,
            limits: None,
        }],
        ..Default::default()
    };
    for t in facts.texts.iter().filter(|t| t.view == id) {
        d.entities.push(
            Entity2d::new(
                "TEXT",
                &notes_layer.0,
                Geom::Text {
                    at: mm(t.at),
                    height: t.height.unwrap_or(2.5 / MM_PER_FOOT) * scale * MM_PER_FOOT,
                    rotation: t.rotation,
                    value: t.value.clone(),
                },
            )
            .of_element(Some(t.id)),
        );
    }
    for c in facts.curves.iter().filter(|c| c.view == Some(id)) {
        for s in c.points.windows(2) {
            d.entities.push(
                Entity2d::new(
                    "LINE",
                    &detail.0,
                    Geom::Line {
                        a: mm(s[0]),
                        b: mm(s[1]),
                    },
                )
                .of_element(Some(c.id)),
            );
        }
    }
    if facts.imports.iter().any(|i| i.0 == id) {
        imported_drawings(rf, id, &import, &mut d, notes);
    }
    for (name, aci) in [&notes_layer, &detail, &import] {
        if d.entities.iter().any(|e| &e.layer == name) && !d.layers.iter().any(|l| &l.name == name)
        {
            d.layers.push(Layer2d::new(name.as_str(), *aci));
        }
    }
    d
}

#[cfg(feature = "dwg")]
/// The CAD drawings imported or linked into view `id`, drawn into `d` as the authoring tool's
/// DWG export draws them: each curve and text on its own layer of the drawing (`Thin`, `Text`,
/// ...: the import's layer subcategories, coloured as their line styles are), each entity
/// carrying the import's ElementId, its blocks drawn in place. `fallback` is the layer for
/// geometry on no named layer.
fn imported_drawings(
    rf: &mut crate::RevitFile,
    id: i64,
    fallback: &(String, i16),
    d: &mut rvt_dwg::Drawing2d,
    notes: &mut Vec<String>,
) {
    use rvt_dwg::{Colour, Entity2d, Geom, Layer2d};
    let links = match crate::links::read(rf) {
        Ok(l) => l,
        Err(e) => {
            notes.push(format!("imported drawing not read: {e}"));
            return;
        }
    };
    let placed: Vec<_> = links
        .instances
        .iter()
        .filter(|i| i.view == Some(id) && i.class == "ImportInstance")
        .collect();
    let wanted = placed.iter().map(|i| i.type_id as u64).collect();
    let graphics = crate::links::import_graphics(rf, &wanted).unwrap_or_else(|e| {
        notes.push(format!("imported drawing not read: {e}"));
        Default::default()
    });
    let mm = |p: [f64; 3]| [p[0] * MM_PER_FOOT, p[1] * MM_PER_FOOT, 0.0];
    for i in placed {
        let (Some(t), Some(g)) = (
            links.types.iter().find(|t| t.id == i.type_id),
            graphics.get(&i.type_id),
        ) else {
            continue;
        };
        let [x, y, _, o] = i.transform;
        let place = |p: [f64; 3]| mm([0, 1, 2].map(|k| o[k] + p[0] * x[k] + p[1] * y[k]));
        // Graphics name the line style they are drawn with, which belongs to one layer.
        if g.truncated {
            notes.push(format!(
                "imported drawing {}: more than {} curves and texts through its blocks; the rest is left out",
                t.name,
                crate::links::IMPORT_DRAW_BUDGET
            ));
        }
        // The layer, and the colour an entity carries itself when another import already
        // defined a layer of that name in another colour (layers are one table per drawing).
        let mut layer_of = |style: i64| -> (String, Option<Colour>) {
            let Some(l) = t
                .layers
                .iter()
                .find(|l| l.id == style || l.styles.contains(&style))
            else {
                return (fallback.0.clone(), None);
            };
            let mut layer = Layer2d::new(l.name.as_str(), 7);
            // A line style colour is 0xBBGGRR; black draws as colour 7 (white on black).
            if let Some(c) = l.color.filter(|c| *c > 0) {
                let c = c as u32;
                layer.colour = Colour {
                    aci: None,
                    rgb: Some([
                        (c & 0xff) as u8,
                        (c >> 8 & 0xff) as u8,
                        (c >> 16 & 0xff) as u8,
                    ]),
                };
            }
            match d.layers.iter().find(|x| x.name == l.name) {
                None => {
                    d.layers.push(layer);
                    (l.name.clone(), None)
                }
                Some(x) if x.colour == layer.colour => (l.name.clone(), None),
                Some(_) => (l.name.clone(), Some(layer.colour)),
            }
        };
        for (points, category) in &g.curves {
            let (layer, colour) = layer_of(*category);
            for s in points.windows(2) {
                let mut e = Entity2d::new(
                    "LINE",
                    &layer,
                    Geom::Line {
                        a: place(s[0]),
                        b: place(s[1]),
                    },
                )
                .of_element(Some(i.id));
                if let Some(c) = colour {
                    e.colour = c;
                }
                d.entities.push(e);
            }
        }
        for text in &g.texts {
            let (layer, colour) = layer_of(text.layer);
            let mut e = Entity2d::new(
                "TEXT",
                &layer,
                Geom::Text {
                    at: place(text.at),
                    height: text.height * x[0].hypot(x[1]) * MM_PER_FOOT,
                    // The baseline taken through both axes, so a mirrored placement
                    // keeps an upward text upward.
                    rotation: {
                        let (c, s) = (text.rotation.cos(), text.rotation.sin());
                        (x[1] * c + y[1] * s).atan2(x[0] * c + y[0] * s)
                    },
                    value: text.value.clone(),
                },
            )
            .of_element(Some(i.id));
            if let Some(c) = colour {
                e.colour = c;
            }
            d.entities.push(e);
        }
    }
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
    layers: &super::dwg_layers::ExportLayers,
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
                drawing: sheet_drawing(rf, model, facts, sheet, symbols, layers, notes)?,
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
                layers: layers.clone(),
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
            let scale = view.scale.unwrap_or(100.0);
            let mut drawing = super::sheet::projected_drawing(model, &window, scale, layers);
            section_annotation(&mut drawing, model, facts, view, &window, scale, layers);
            Ok(ViewDrawing {
                drawing,
                layout: None,
                scale,
            })
        }
    }
}

/// A drawing as `dxf`, `dwg` (in `version`: 2018, 2013, ...), `pdf` or `svg` (`layout` drawn: a
/// sheet; `None` for model space, a PDF's at 1:`scale`), with the writer's notes.
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
        "svg" => {
            return rvt_dwg::svg::render(drawing, layout, &rvt_dwg::Limits::DEFAULT)
                .map(|s| (s.svg.into_bytes(), Vec::new()))
                .map_err(|e| e.to_string());
        }
        "dwg" => CadFormat::Dwg,
        "dxf" => CadFormat::Dxf,
        other => return Err(format!("unknown format {other:?}: dxf, dwg, pdf or svg")),
    };
    let version = parse_version(version, format).map_err(|e| e.to_string())?;
    write_drawing(drawing, format, version).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_segment_crossing_the_crop_is_kept() {
        let crop = CropRect {
            origin: [0.0, 0.0],
            x_axis: [1.0, 0.0],
            min: [0.0, 0.0],
            max: [10.0, 10.0],
        };
        assert!(segment_in_crop(Some(&crop), [-1.0, 5.0], [11.0, 5.0]));
        assert!(segment_in_crop(Some(&crop), [2.0, 2.0], [3.0, 3.0]));
        assert!(!segment_in_crop(Some(&crop), [-5.0, -1.0], [20.0, -1.0]));
        assert!(segment_in_crop(None, [-5.0, -1.0], [20.0, -1.0]));
    }
}
