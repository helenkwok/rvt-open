//! Family-instance bodies from the family's own embedded document.
//!
//! Where a `FamilyInstance` carries only its element record's bounding box,
//! [`attach`] looks up the instance's symbol, the symbol's family and the
//! family's content document ([`crate::family_documents`]) and, when every
//! solid form of that document decoded, replaces the box with those solids
//! placed by the instance's own transform. Order of preference in the
//! export: this, then the saved graphics ([`super::saved_meshes`]), then the
//! box.

use super::IfcModel;
use super::entities::{BrepTriangle, IfcEntity, PropertyValue, SolidShape};
use crate::family_documents::{FamilyDocuments, Route};
use std::collections::{BTreeMap, BTreeSet};

/// `BodySource` of a body drawn from the family's embedded document.
pub const FAMILY_DOCUMENT_BODY_SOURCE: &str = "family_document_geometry";
/// `BodySource` of a family-document body stretched to the instance's own
/// record box (the document draws the family's saved type).
pub const FAMILY_DOCUMENT_SCALED_BODY_SOURCE: &str = "family_document_geometry_scaled";
/// A stretch beyond these factors is another shape, not another size.
const SCALE_LIMITS: (f64, f64) = (0.2, 5.0);

/// What [`attach`] did.
#[derive(Debug, Default, Clone, PartialEq, serde::Serialize)]
pub struct FamilyBodyReport {
    /// Elements whose body was the record's bounding box.
    pub candidates: usize,
    /// Of those, drawn from their family document.
    pub replaced: usize,
    pub triangles: usize,
    /// Candidates that are not instances of a loaded family.
    pub no_family_document: usize,
    /// Candidates whose family document holds a form not decoded, by form.
    pub incomplete: BTreeMap<String, usize>,
    /// Candidates whose family document decoded but whose body is not the
    /// size of the instance's record box: the document draws the family's
    /// saved type, and the instance's type can be another size.
    pub size_rejected: usize,
    /// Candidates mirrored in X or Y, which this reader does not place.
    pub flipped_skipped: usize,
    /// Elements that took a material from a solid's material element.
    pub materials_set: usize,
    /// Of `replaced`, bodies stretched to the instance's record box.
    pub scaled: usize,
    /// Of `scaled`, bodies stretched beyond the usual limits because every
    /// solid is an extrusion or blend running along one box axis and the two
    /// cross factors are equal: the same ruled form between proportionally
    /// scaled profiles over another length (an approximation only where the
    /// instance's profiles are not in the document's proportions).
    pub scaled_ruled_along_axis: usize,
    /// Every size rejection: element id and the stretch per body axis it
    /// would have needed.
    pub size_rejected_factors: BTreeMap<i64, Vec<f64>>,
    /// The first size rejection of each family symbol (up to 64): element id, the drawn body's size,
    /// the record box, the stretch it would need and each document solid's
    /// own size (feet), so a rejection can be read without a debugger.
    pub size_rejected_examples: Vec<String>,
}

/// Replace each bounding-box body with the instance's family-document
/// solids, where that document decoded completely.
pub fn attach(rf: &mut crate::RevitFile, model: &mut IfcModel) -> anyhow::Result<FamilyBodyReport> {
    attach_oriented(rf, model, false)
}

/// [`attach`] where `rows_are_axes` reads the instance matrix's rows, not
/// its columns, as the instance's axes (the probe measures which is right).
pub fn attach_oriented(
    rf: &mut crate::RevitFile,
    model: &mut IfcModel,
    rows_are_axes: bool,
) -> anyhow::Result<FamilyBodyReport> {
    let mut report = FamilyBodyReport::default();
    let wholes: BTreeSet<usize> = model
        .entities
        .iter()
        .filter_map(|entity| match entity {
            IfcEntity::Aggregate { whole, .. } => Some(*whole),
            _ => None,
        })
        .collect();
    let mut candidates: Vec<(usize, i64)> = Vec::new();
    for (index, entity) in model.entities.iter().enumerate() {
        if wholes.contains(&index) {
            continue;
        }
        let IfcEntity::BuildingElement {
            type_guid: Some(tag),
            property_set: Some(set),
            location_feet: Some(_),
            ..
        } = entity
        else {
            continue;
        };
        let text = |name: &str| {
            set.properties.iter().find_map(|p| match &p.value {
                PropertyValue::Text(t) if p.name == name => Some(t.as_str()),
                _ => None,
            })
        };
        let profile_resolved = set.properties.iter().any(|p| {
            p.name == "ProfileResolved" && matches!(p.value, PropertyValue::Boolean(true))
        });
        if !text("BodySource").is_some_and(super::export_content::is_plan_box_source)
            || profile_resolved
        {
            continue;
        }
        if let Ok(id) = tag.parse::<i64>() {
            candidates.push((index, id));
        }
    }
    report.candidates = candidates.len();
    if candidates.is_empty() {
        return Ok(report);
    }
    let docs: FamilyDocuments = crate::family_documents::read(rf)?;
    let mut resolver = None;
    let mut pending: Vec<(usize, crate::native_saved_materials::RenderMaterial)> = Vec::new();
    let mut rejected_symbols: BTreeSet<i64> = BTreeSet::new();
    for (index, id) in candidates {
        match docs.route(id) {
            Route::NoFamilyDocument => {
                report.no_family_document += 1;
                continue;
            }
            Route::Incomplete(why) => {
                for w in why {
                    *report.incomplete.entry(w.to_string()).or_default() += 1;
                }
                continue;
            }
            Route::FamilyDocument => {}
        }
        let Some((pose, doc)) = docs.solids_for(id) else {
            continue;
        };
        if pose.flipped_x || pose.flipped_y {
            report.flipped_skipped += 1;
            continue;
        }
        let m = pose.matrix;
        let axis = |j: usize| {
            if rows_are_axes {
                m[j]
            } else {
                [m[0][j], m[1][j], m[2][j]]
            }
        };
        let (x, y, z) = (axis(0), axis(1), axis(2));
        let IfcEntity::BuildingElement {
            location_feet: Some(origin),
            rotation_radians,
            extrusion,
            solid_shape,
            property_set: Some(set),
            material_index,
            material_layer_set_index,
            material_profile_set_index,
            ..
        } = &mut model.entities[index]
        else {
            continue;
        };
        let (sin, cos) = (-rotation_radians.unwrap_or(0.0)).sin_cos();
        let mut vertices = Vec::new();
        let mut triangles = Vec::new();
        let mut dominant: Option<(usize, i64)> = None;
        for solid in &doc.solids {
            let base = vertices.len() as u32;
            vertices.extend(solid.vertices.iter().map(|p| {
                let model_point: [f64; 3] = std::array::from_fn(|k| {
                    pose.origin[k] + x[k] * p[0] + y[k] * p[1] + z[k] * p[2]
                });
                let (dx, dy) = (model_point[0] - origin[0], model_point[1] - origin[1]);
                [
                    dx * cos - dy * sin,
                    dx * sin + dy * cos,
                    model_point[2] - origin[2],
                ]
            }));
            triangles.extend(
                solid
                    .triangles
                    .iter()
                    .map(|t| BrepTriangle(base + t[0], base + t[1], base + t[2])),
            );
            if let Some(m) = solid.material_id
                && dominant.is_none_or(|(n, _)| solid.triangles.len() > n)
            {
                dominant = Some((solid.triangles.len(), m));
            }
        }
        if triangles.is_empty() {
            continue;
        }
        // The body must be the size of the record box (either way round in
        // plan); when it is not, the document drew the family's saved type,
        // so stretch it to the instance's own box, unless that
        // would make it another shape.
        let mut source = FAMILY_DOCUMENT_BODY_SOURCE;
        if let Some(record) = extrusion.as_ref() {
            let mut lo = [f64::INFINITY; 3];
            let mut hi = [f64::NEG_INFINITY; 3];
            for v in &vertices {
                for k in 0..3 {
                    lo[k] = lo[k].min(v[k]);
                    hi[k] = hi[k].max(v[k]);
                }
            }
            let size = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
            let close = |a: f64, b: f64| (a - b).abs() <= 0.1_f64.max(0.15 * b.abs());
            let straight = close(size[0], record.width_feet) && close(size[1], record.depth_feet);
            let turned = close(size[0], record.depth_feet) && close(size[1], record.width_feet);
            if !((straight || turned) && close(size[2], record.height_feet)) {
                // Plan axes the way round the body is most nearly the box.
                let gap = |a: f64, b: f64| (a / b.max(1e-9)).ln().abs();
                let want_xy = if gap(size[0], record.width_feet) + gap(size[1], record.depth_feet)
                    <= gap(size[0], record.depth_feet) + gap(size[1], record.width_feet)
                {
                    [record.width_feet, record.depth_feet]
                } else {
                    [record.depth_feet, record.width_feet]
                };
                let want = [want_xy[0], want_xy[1], record.height_feet];
                let factor: Vec<f64> = (0..3)
                    .map(|k| {
                        if size[k] > 1e-6 {
                            want[k] / size[k]
                        } else {
                            f64::NAN
                        }
                    })
                    .collect();
                let within = factor
                    .iter()
                    .all(|f| f.is_finite() && (SCALE_LIMITS.0..=SCALE_LIMITS.1).contains(f));
                // Outside the limits, a body of extrusions and blends that all
                // run along one box axis k, with equal factors across it,
                // scales exactly along k: circles stay round, squares square.
                let ruled_along_axis = !within && {
                    let local = |a: [f64; 3]| -> [f64; 3] {
                        let w: [f64; 3] =
                            std::array::from_fn(|k| x[k] * a[0] + y[k] * a[1] + z[k] * a[2]);
                        [w[0] * cos - w[1] * sin, w[0] * sin + w[1] * cos, w[2]]
                    };
                    let axes: Option<Vec<[f64; 3]>> =
                        doc.solids.iter().map(|s| s.ruled_axis.map(local)).collect();
                    let along = axes.and_then(|axes| {
                        let first = *axes.first()?;
                        let k = (0..3).find(|&k| first[k].abs() > 1.0 - 1e-6)?;
                        axes.iter().all(|a| a[k].abs() > 1.0 - 1e-6).then_some(k)
                    });
                    along.is_some_and(|k| {
                        let cross: Vec<f64> =
                            (0..3).filter(|&i| i != k).map(|i| factor[i]).collect();
                        factor.iter().all(|f| f.is_finite() && *f > 0.0)
                            && (cross[0] - cross[1]).abs() <= 0.01 * cross[0].max(cross[1])
                    })
                };
                if ruled_along_axis {
                    report.scaled_ruled_along_axis += 1;
                }
                if !within && !ruled_along_axis {
                    report.size_rejected += 1;
                    report.size_rejected_factors.insert(id, factor.clone());
                    if report.size_rejected_examples.len() < 64
                        && rejected_symbols.insert(pose.symbol)
                    {
                        let solids: Vec<String> = doc
                            .solids
                            .iter()
                            .map(|s| {
                                let (mut a, mut b) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
                                for v in &s.vertices {
                                    for k in 0..3 {
                                        a[k] = a[k].min(v[k]);
                                        b[k] = b[k].max(v[k]);
                                    }
                                }
                                format!(
                                    "form {} {} tris [{:.4},{:.4},{:.4}]..[{:.4},{:.4},{:.4}]",
                                    s.form_id,
                                    s.triangles.len(),
                                    a[0],
                                    a[1],
                                    a[2],
                                    b[0],
                                    b[1],
                                    b[2]
                                )
                            })
                            .collect();
                        report.size_rejected_examples.push(format!(
                            "{id}: body {size:.4?}, record [{:.4},{:.4},{:.4}], factor {factor:.3?}; solids {solids:?}",
                            record.width_feet, record.depth_feet, record.height_feet
                        ));
                    }
                    continue;
                }
                // About the plan centre and the lowest point, so the base
                // stays where the instance puts it.
                let pivot = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, lo[2]];
                for v in &mut vertices {
                    for k in 0..3 {
                        v[k] = pivot[k] + (v[k] - pivot[k]) * factor[k];
                    }
                }
                source = FAMILY_DOCUMENT_SCALED_BODY_SOURCE;
                report.scaled += 1;
            }
        }
        let (vertices, triangles) = super::saved_meshes::tidy_mesh(vertices, triangles);
        report.replaced += 1;
        report.triangles += triangles.len();
        *solid_shape = Some(SolidShape::TriangulatedFaceSet {
            vertices_feet: vertices,
            triangles,
            face_materials: Vec::new(),
        });
        for property in &mut set.properties {
            if property.name == "BodySource" {
                property.value = PropertyValue::Text(source.into());
            }
            super::export_content::mark_space_family_body(property);
        }
        let wants_material = material_index.is_none()
            && material_layer_set_index.is_none()
            && material_profile_set_index.is_none();
        if let (true, Some((_, material_id))) = (wants_material, dominant) {
            if resolver.is_none() {
                resolver =
                    crate::native_saved_scene::material_resolver(rf, &Default::default()).ok();
            }
            if let Some(found) = resolver
                .as_ref()
                .and_then(|r| r.resolve(id as u64, None, 0, 0, Some(material_id)))
            {
                pending.push((index, found));
            }
        }
    }
    for (index, found) in pending {
        let chosen = super::saved_meshes::material_index_for(&mut model.materials, &found);
        if let IfcEntity::BuildingElement { material_index, .. } = &mut model.entities[index] {
            *material_index = Some(chosen);
            report.materials_set += 1;
        }
    }
    Ok(report)
}
