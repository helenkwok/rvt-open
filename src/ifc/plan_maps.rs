//! The 2D maps a family type holds beside its Body map.
//!
//! Revit 2027.1's IFC4 Reference View export of the 12 Nordic LCA models
//! holds 1,788 `FootPrint` and 155 `Axis` maps beside 6,764 Body maps, each on
//! the type object holding the Body map, and every product drawn by that
//! Body map draws them too. Two kinds of `FootPrint`:
//!
//! - **a family's plan symbol** (1,203 maps: doors, windows, columns,
//!   furniture, terminals, ...), in the `Plan` context's `Annotation`
//!   sub-context: the curves the type's symbol shows in the instance level's
//!   plan view, the free curves at the symbol's own top level (Revit's IFC exporter
//!   `FamilyInstanceExporter.CreateFootprintCurveSet` and
//!   Read here from the symbol's saved graphics
//!   (`crate::native_plan_curves`);
//! - **an extrusion's base** (585 maps, all curtain-panel plates), in the
//!   model's `FootPrint` sub-context: the outline of the body's bottom face.
//!
//! Each map's 2D shape is its users', in the map's frame; a map whose users
//! disagree (a different flip or offset against the same body, or `open`'s
//! sharing across types) gets none and is counted in
//! [`PlanMapReport::users_differing`].
use super::IfcModel;
use super::entities::{IfcEntity, PlanPolyline, PlanShape, SolidShape};
use crate::native_parametric_surface::Curve;
use crate::native_plan_curves::{PlanView, plan_curves};
use std::collections::{BTreeMap, BTreeSet};

/// What [`attach`] did.
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct PlanMapReport {
    pub maps: usize,
    pub symbols_read: usize,
    pub footprints: usize,
    pub plate_footprints: usize,
    pub beam_axes: usize,
    pub users_differing: usize,
    pub no_transform: usize,
    /// Body maps whose users with no Level took a copy without the plan
    /// symbol (`revit`).
    pub levelless_split: usize,
}

const TOLERANCE_FEET: f64 = 1e-4;

/// Give each Body map the 2D maps Revit's export gives its type.
pub fn attach(rf: &mut crate::RevitFile, model: &mut IfcModel) -> anyhow::Result<PlanMapReport> {
    attach_with(rf, model, &PlanView::default())
}

/// [`attach`] for the plan view `view` selects.
pub(crate) fn attach_with(
    rf: &mut crate::RevitFile,
    model: &mut IfcModel,
    view: &PlanView,
) -> anyhow::Result<PlanMapReport> {
    let mut report = PlanMapReport {
        maps: model.representation_maps.len(),
        ..Default::default()
    };
    let mut users: Vec<Vec<usize>> = vec![Vec::new(); model.representation_maps.len()];
    for (i, e) in model.entities.iter().enumerate() {
        if let IfcEntity::BuildingElement {
            representation_map_index: Some(m),
            ..
        } = e
        {
            if let Some(u) = users.get_mut(*m) {
                u.push(i);
            }
        }
    }
    let element_id = |e: &IfcEntity| match e {
        IfcEntity::BuildingElement { type_guid, .. } => {
            type_guid.as_ref().and_then(|t| t.parse::<u32>().ok())
        }
        _ => None,
    };
    // Plates: their extrusion base.
    for (m, us) in users.iter().enumerate() {
        let Some(&first) = us.first() else { continue };
        let IfcEntity::BuildingElement { ifc_type, .. } = &model.entities[first] else {
            continue;
        };
        if ifc_type != "IFCPLATE" {
            continue;
        }
        if let Some(curves) = base_outline(&model.representation_maps[m].shape) {
            model.representation_maps[m].plan.push(PlanShape {
                identifier: "FootPrint".into(),
                context: "FootPrint".into(),
                curves,
            });
            report.plate_footprints += 1;
        }
    }
    // Beams: their location line (Revit's IFC exporter, `BeamExporter.CreateBeamAxis`).
    let beam_ids: BTreeSet<u32> = users
        .iter()
        .flatten()
        .filter_map(|&u| match &model.entities[u] {
            IfcEntity::BuildingElement { ifc_type, .. } if ifc_type == "IFCBEAM" => {
                element_id(&model.entities[u])
            }
            _ => None,
        })
        .collect();
    if !beam_ids.is_empty() {
        let version = rf.basic_file_info().map(|b| b.version).unwrap_or(0);
        let lines = crate::partition_beam_axes::scan_bounded_lines(rf, version, &beam_ids)
            .unwrap_or_default();
        for (m, us) in users.iter().enumerate() {
            let mut shape: Option<Vec<PlanPolyline>> = None;
            let mut differs = false;
            for &u in us {
                let IfcEntity::BuildingElement {
                    ifc_type,
                    location_feet: Some(location),
                    rotation_radians,
                    ..
                } = &model.entities[u]
                else {
                    continue;
                };
                if ifc_type != "IFCBEAM" {
                    continue;
                }
                let Some(line) = element_id(&model.entities[u]).and_then(|id| lines.get(&id))
                else {
                    continue;
                };
                let local = vec![PlanPolyline {
                    points_feet: [line.start(), line.end()]
                        .iter()
                        .map(|w| to_local(*w, location, rotation_radians.unwrap_or(0.0)))
                        .collect(),
                    arc: false,
                }];
                match &shape {
                    None => shape = Some(local),
                    Some(held) if !same(held, &local) => differs = true,
                    _ => {}
                }
            }
            if differs {
                report.users_differing += 1;
                continue;
            }
            if let Some(curves) = shape {
                model.representation_maps[m].plan.push(PlanShape {
                    identifier: "Axis".into(),
                    context: "Axis".into(),
                    curves,
                });
                report.beam_axes += 1;
            }
        }
    }
    // Family types: their symbol's plan curves.
    let family = super::body_geometry::revit_family_instances(model);
    let mut wanted_symbols = BTreeSet::new();
    let mut instance_ids = BTreeSet::new();
    for (m, us) in users.iter().enumerate() {
        if !model.representation_maps[m].plan.is_empty() {
            continue;
        }
        for &u in us.iter().filter(|&&u| family[u]) {
            if let Some(id) = element_id(&model.entities[u]) {
                if let Some(t) = model.element_type_ids.get(&id) {
                    wanted_symbols.insert(u64::from(*t));
                    instance_ids.insert(id);
                }
            }
        }
    }
    if wanted_symbols.is_empty() {
        return Ok(report);
    }
    let (symbol_curves, transforms) = read_symbols(
        rf,
        view,
        wanted_symbols,
        &instance_ids,
        &mut report.symbols_read,
    )?;
    // an instance with no Level draws no plan
    // symbol in Revit's export ([`crate::schema_spatial::levelless`]).
    let levelless = if model.convention == super::ExportConvention::Revit {
        crate::schema_spatial::levelless(rf, &instance_ids).unwrap_or_default()
    } else {
        BTreeSet::new()
    };
    let mut split: Vec<(usize, Vec<usize>)> = Vec::new();
    for (m, us) in users.iter().enumerate() {
        if !model.representation_maps[m].plan.is_empty() {
            continue;
        }
        let mut shape: Option<Vec<PlanPolyline>> = None;
        let mut differs = false;
        let mut without_level = Vec::new();
        for &u in us {
            if !family[u] {
                continue;
            }
            let Some(id) = element_id(&model.entities[u]) else {
                continue;
            };
            if levelless.contains(&id) {
                without_level.push(u);
                continue;
            }
            let Some(curves) = model
                .element_type_ids
                .get(&id)
                .and_then(|t| symbol_curves.get(t))
            else {
                continue;
            };
            let IfcEntity::BuildingElement {
                location_feet: Some(location),
                rotation_radians,
                ..
            } = &model.entities[u]
            else {
                continue;
            };
            let Some(t) = transforms.get(&id) else {
                report.no_transform += 1;
                continue;
            };
            let local: Vec<PlanPolyline> = curves
                .iter()
                .map(|c| PlanPolyline {
                    arc: c.arc,
                    points_feet: c
                        .points_feet
                        .iter()
                        .map(|p| {
                            let w: [f64; 3] = [0, 1, 2].map(|i| {
                                t.axes[0][i] * p[0]
                                    + t.axes[1][i] * p[1]
                                    + t.axes[2][i] * p[2]
                                    + t.origin[i]
                            });
                            to_local(w, location, rotation_radians.unwrap_or(0.0))
                        })
                        .collect(),
                })
                .collect();
            match &shape {
                None => shape = Some(local),
                Some(held) if !same(held, &local) => differs = true,
                _ => {}
            }
        }
        if differs {
            // Users of one Body map whose plan symbols differ (open's sharing
            // across types, or a flip the body does not show): no FootPrint
            // rather than one that is wrong for some of them.
            report.users_differing += 1;
            continue;
        }
        if shape.is_some() && !without_level.is_empty() {
            report.levelless_split += 1;
            split.push((m, without_level));
        }
        if let Some(curves) = shape {
            model.representation_maps[m].plan.push(PlanShape {
                identifier: "FootPrint".into(),
                context: "Annotation".into(),
                curves,
            });
            report.footprints += 1;
        }
    }
    // The users with no Level take a copy of the Body map without the plan
    // symbol, so the others keep it.
    for (m, users) in split {
        let mut copy = model.representation_maps[m].clone();
        copy.plan.clear();
        let index = model.representation_maps.len();
        model.representation_maps.push(copy);
        for u in users {
            if let IfcEntity::BuildingElement {
                representation_map_index,
                ..
            } = &mut model.entities[u]
            {
                *representation_map_index = Some(index);
            }
        }
    }
    Ok(report)
}

type SymbolCurves = BTreeMap<u32, Vec<PlanPolyline>>;
type Transforms = BTreeMap<u32, crate::partition_instance_transforms::InstanceTransform>;

/// Each wanted family symbol's plan curves (in its own frame) and each wanted instance's
/// transform. `read` counts the symbols whose saved graphics were read.
fn read_symbols(
    rf: &mut crate::RevitFile,
    view: &PlanView,
    wanted_symbols: BTreeSet<u64>,
    instance_ids: &BTreeSet<u32>,
    read: &mut usize,
) -> anyhow::Result<(SymbolCurves, Transforms)> {
    let mut opts = crate::native_document::Options {
        selected_ids: wanted_symbols,
        max_graph_values: 1_000_000,
        max_graph_objects: 1_000_000,
        ..Default::default()
    };
    opts.channels = BTreeSet::from([103]);
    let mut symbol_curves = SymbolCurves::new();
    crate::native_document::extract(rf, &opts, |r| {
        if let (Some(g), Ok(id)) = (&r.graph, u32::try_from(r.identity.element_id)) {
            *read += 1;
            let curves: Vec<PlanPolyline> = plan_curves(g, view)
                .curves
                .iter()
                .map(|c| symbol_polyline(&c.curve, &c.transform))
                .collect();
            if !curves.is_empty() {
                symbol_curves.insert(id, curves);
            }
        }
        Ok(())
    })?;
    let version = rf.basic_file_info().map(|b| b.version).unwrap_or(0);
    let transforms =
        crate::partition_instance_transforms::scan_instance_transforms(rf, version, instance_ids)
            .unwrap_or_default();
    Ok((symbol_curves, transforms))
}

/// Each family instance's plan symbol in model coordinates (feet), by its index in
/// `model.entities`: the curves its symbol shows in a plan view (doors' swings, windows' glass
/// and sill lines, fixtures' outlines), placed by the instance's own transform, so its flips and
/// hand are the instance's.
pub fn instance_plan_symbols(
    rf: &mut crate::RevitFile,
    model: &IfcModel,
) -> anyhow::Result<BTreeMap<usize, Vec<PlanPolyline>>> {
    let family = super::body_geometry::revit_family_instances(model);
    let mut by_id: BTreeMap<u32, usize> = BTreeMap::new();
    let mut wanted_symbols = BTreeSet::new();
    for (u, e) in model.entities.iter().enumerate() {
        let IfcEntity::BuildingElement { type_guid, .. } = e else {
            continue;
        };
        let Some(id) = type_guid.as_ref().and_then(|t| t.parse::<u32>().ok()) else {
            continue;
        };
        if !family[u] {
            continue;
        }
        if let Some(t) = model.element_type_ids.get(&id) {
            wanted_symbols.insert(u64::from(*t));
            by_id.insert(id, u);
        }
    }
    let mut out = BTreeMap::new();
    if wanted_symbols.is_empty() {
        return Ok(out);
    }
    let ids: BTreeSet<u32> = by_id.keys().copied().collect();
    let mut read = 0;
    let (symbol_curves, mut transforms) =
        read_symbols(rf, &PlanView::default(), wanted_symbols, &ids, &mut read)?;
    // Where the element data gives no transform (before 2024), the instance's own `InstanceInfo`
    // matrix, whose columns are its axes.
    if transforms.len() < ids.len() {
        let docs = crate::family_documents::read_memoised(rf)?;
        for id in &ids {
            if transforms.contains_key(id) {
                continue;
            }
            let Some(pose) = docs.instances.get(&i64::from(*id)) else {
                continue;
            };
            let m = pose.matrix;
            let axes = [0, 1, 2].map(|j| [m[0][j], m[1][j], m[2][j]]);
            transforms.insert(
                *id,
                crate::partition_instance_transforms::InstanceTransform {
                    axes,
                    origin: pose.origin,
                },
            );
        }
    }
    for (id, u) in by_id {
        let (Some(curves), Some(t)) = (
            model
                .element_type_ids
                .get(&id)
                .and_then(|s| symbol_curves.get(s)),
            transforms.get(&id),
        ) else {
            continue;
        };
        let world = curves
            .iter()
            .map(|c| PlanPolyline {
                arc: c.arc,
                points_feet: c
                    .points_feet
                    .iter()
                    .map(|p| {
                        [0, 1, 2].map(|i| {
                            t.axes[0][i] * p[0]
                                + t.axes[1][i] * p[1]
                                + t.axes[2][i] * p[2]
                                + t.origin[i]
                        })
                    })
                    .collect(),
            })
            .collect();
        out.insert(u, world);
    }
    Ok(out)
}

/// A text a family symbol draws (a title block's fixed labels), in the symbol's frame: where the
/// text's box starts on the left of its base line, feet.
#[derive(Debug, Clone, PartialEq)]
pub struct SymbolText {
    pub at: [f64; 2],
    pub height_feet: f64,
    pub rotation: f64,
    pub value: String,
}

/// A symbol's sheet curves and texts, see [`symbol_sheet_graphics`].
pub type SymbolGraphics = (Vec<PlanPolyline>, Vec<SymbolText>);

/// What a family symbol draws on a sheet: the curves it shows in a plan view and its texts
/// (`GRichText`: rich text at `m_modelOrigin`, moved right by `m_textHorzOffsetHimetric` and up by
/// `m_textVertOffsetHimetric`, both 1/100 mm, `m_defaultFontSize` high, along `m_modelXUnitVec`).
/// A title block's border, lines and fixed labels, by its symbol id.
pub fn symbol_sheet_graphics(
    rf: &mut crate::RevitFile,
    symbols: &BTreeSet<u64>,
) -> anyhow::Result<BTreeMap<u32, SymbolGraphics>> {
    let mut read = 0;
    let (mut curves, _) = read_symbols(
        rf,
        &PlanView::default(),
        symbols.clone(),
        &BTreeSet::new(),
        &mut read,
    )?;
    let mut opts = crate::native_document::Options {
        selected_ids: symbols.clone(),
        max_graph_values: 1_000_000,
        max_graph_objects: 1_000_000,
        ..Default::default()
    };
    opts.channels = BTreeSet::from([103]);
    let mut texts: BTreeMap<u32, Vec<SymbolText>> = BTreeMap::new();
    crate::native_document::extract(rf, &opts, |r| {
        if let (Some(g), Ok(id)) = (&r.graph, u32::try_from(r.identity.element_id)) {
            for o in g.objects.iter().filter(|o| o.class_name == "GRichText") {
                if let Some(t) = rich_text(&o.fields) {
                    texts.entry(id).or_default().push(t);
                }
            }
        }
        Ok(())
    })?;
    let ids: BTreeSet<u32> = curves.keys().chain(texts.keys()).copied().collect();
    Ok(ids
        .into_iter()
        .map(|id| {
            (
                id,
                (
                    curves.remove(&id).unwrap_or_default(),
                    texts.remove(&id).unwrap_or_default(),
                ),
            )
        })
        .collect())
}

/// A `GRichText` graphics object as a text, see [`symbol_sheet_graphics`].
fn rich_text(f: &crate::graph_value::Fv) -> Option<SymbolText> {
    let xyz = |name: &str| -> Option<[f64; 3]> {
        let v = f.get(name)?.get("m_xyz")?.as_array()?;
        let p: Vec<f64> = v.iter().filter_map(|x| x.as_f64()).collect();
        (p.len() == 3).then(|| [p[0], p[1], p[2]])
    };
    let num = |name: &str| f.get(name).and_then(|v| v.as_f64()).unwrap_or(0.0);
    let raw: Vec<u8> = f
        .get("m_arrRtfRawData")?
        .as_array()?
        .iter()
        .filter_map(|x| x.as_u64())
        .map(|x| x as u8)
        .take_while(|b| *b != 0)
        .collect();
    let value = crate::plan_views::rtf_plain(&String::from_utf8_lossy(&raw));
    if value.trim().is_empty() {
        return None;
    }
    let origin = xyz("m_modelOrigin")?;
    let x = xyz("m_modelXUnitVec").unwrap_or([1.0, 0.0, 0.0]);
    let (dx, dy) = (
        num("m_textHorzOffsetHimetric") / 100.0 / 304.8,
        num("m_textVertOffsetHimetric") / 100.0 / 304.8,
    );
    Some(SymbolText {
        at: [
            origin[0] + dx * x[0] - dy * x[1],
            origin[1] + dx * x[1] + dy * x[0],
        ],
        height_feet: num("m_defaultFontSize"),
        rotation: x[1].atan2(x[0]),
        value,
    })
}

/// A model point in an element's frame: its location, turned by its rotation.
fn to_local(w: [f64; 3], location: &[f64; 3], rotation: f64) -> [f64; 3] {
    let (sin, cos) = (-rotation).sin_cos();
    let (dx, dy) = (w[0] - location[0], w[1] - location[1]);
    [dx * cos - dy * sin, dx * sin + dy * cos, w[2] - location[2]]
}

fn same(a: &[PlanPolyline], b: &[PlanPolyline]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| {
            x.points_feet.len() == y.points_feet.len()
                && x.points_feet
                    .iter()
                    .zip(&y.points_feet)
                    .all(|(p, q)| (0..3).all(|i| (p[i] - q[i]).abs() <= TOLERANCE_FEET))
        })
}

/// A symbol curve in the symbol's frame, laid on its plane z = 0 as Revit's
/// export lays each curve: a line by its ends, an arc by its ends and middle.
fn symbol_polyline(
    curve: &Curve,
    m: &crate::native_graphics_traversal::GraphicsTransform,
) -> PlanPolyline {
    let (range, arc) = match curve {
        Curve::Line { range, .. } => (*range, false),
        Curve::Arc { range, .. } | Curve::Hermite { range, .. } | Curve::Ellipse { range, .. } => {
            (*range, true)
        }
    };
    let at = |t: f64| {
        let p = curve.eval(t);
        let w = [0, 1, 2].map(|i| m[i][0] * p[0] + m[i][1] * p[1] + m[i][2] * p[2] + m[i][3]);
        [w[0], w[1], 0.0]
    };
    let ts: Vec<f64> = if arc {
        vec![range[0], (range[0] + range[1]) / 2.0, range[1]]
    } else {
        vec![range[0], range[1]]
    };
    PlanPolyline {
        points_feet: ts.into_iter().map(at).collect(),
        arc,
    }
}

/// Each undirected edge (its two end points, quantised) with how many
/// triangles use it and its two vertex indices.
type Edges = BTreeMap<([i64; 3], [i64; 3]), (usize, u32, u32)>;

/// The outline of a body's bottom face, one straight curve per edge as
/// Revit's export writes an extrusion base loop: a rectangle extrusion's
/// profile, or the boundary of the triangles lying at the body's lowest z,
/// with the points a triangulation adds along a straight edge dropped.
fn base_outline(shape: &SolidShape) -> Option<Vec<PlanPolyline>> {
    let segment = |a: [f64; 3], b: [f64; 3]| PlanPolyline {
        points_feet: vec![a, b],
        arc: false,
    };
    match shape {
        SolidShape::ExtrudedArea(e) if e.profile_override.is_none() => {
            let (w, d) = (e.width_feet / 2.0, e.depth_feet / 2.0);
            let c = [[-w, -d, 0.], [w, -d, 0.], [w, d, 0.], [-w, d, 0.]];
            Some((0..4).map(|i| segment(c[i], c[(i + 1) % 4])).collect())
        }
        SolidShape::FacetedBrep {
            vertices_feet,
            triangles,
        }
        | SolidShape::TriangulatedFaceSet {
            vertices_feet,
            triangles,
            ..
        } => {
            let zmin = vertices_feet
                .iter()
                .map(|v| v[2])
                .fold(f64::INFINITY, f64::min);
            let low = |i: u32| (vertices_feet[i as usize][2] - zmin).abs() <= TOLERANCE_FEET;
            let key =
                |i: u32| vertices_feet[i as usize].map(|x| (x / TOLERANCE_FEET).round() as i64);
            let mut edges = Edges::new();
            for t in triangles {
                let (a, b, c) = (t.0, t.1, t.2);
                if !(low(a) && low(b) && low(c)) {
                    continue;
                }
                for (p, q) in [(a, b), (b, c), (c, a)] {
                    let (kp, kq) = (key(p), key(q));
                    let k = if kp <= kq { (kp, kq) } else { (kq, kp) };
                    edges.entry(k).or_insert((0, p, q)).0 += 1;
                }
            }
            // Boundary edges, chained into loops by shared end points.
            let boundary: Vec<(u32, u32)> = edges
                .values()
                .filter(|(n, ..)| *n == 1)
                .map(|&(_, p, q)| (p, q))
                .collect();
            let mut next: BTreeMap<[i64; 3], Vec<usize>> = BTreeMap::new();
            for (i, &(p, q)) in boundary.iter().enumerate() {
                next.entry(key(p)).or_default().push(i);
                next.entry(key(q)).or_default().push(i);
            }
            let mut used = vec![false; boundary.len()];
            let mut out = Vec::new();
            for start in 0..boundary.len() {
                if used[start] {
                    continue;
                }
                used[start] = true;
                let (p0, mut cur) = boundary[start];
                let mut ring = vec![vertices_feet[p0 as usize], vertices_feet[cur as usize]];
                while let Some(&e) = next
                    .get(&key(cur))
                    .and_then(|v| v.iter().find(|&&e| !used[e]))
                {
                    used[e] = true;
                    let (p, q) = boundary[e];
                    cur = if key(p) == key(cur) { q } else { p };
                    ring.push(vertices_feet[cur as usize]);
                }
                // Drop points on a straight run.
                let mut pts: Vec<[f64; 3]> = Vec::new();
                for (i, &p) in ring.iter().enumerate() {
                    let interior = i > 0 && i + 1 < ring.len();
                    if interior {
                        let (a, b) = (*pts.last().unwrap(), ring[i + 1]);
                        let cross = (p[0] - a[0]) * (b[1] - a[1]) - (p[1] - a[1]) * (b[0] - a[0]);
                        let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2))
                            .sqrt()
                            .max(1e-12);
                        if (cross / len).abs() <= TOLERANCE_FEET {
                            continue;
                        }
                    }
                    pts.push(p);
                }
                out.extend(pts.windows(2).map(|w| segment(w[0], w[1])));
            }
            (!out.is_empty()).then_some(out)
        }
        _ => None,
    }
}
