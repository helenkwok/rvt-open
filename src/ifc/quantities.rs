//! material quantities for an embodied-carbon takeoff, from the
//! bodies the export draws, without Revit.
//!
//! Per element:
//! - **Volume** of each closed shell of its body (divergence theorem: the sum
//!   of each triangle's signed tetrahedron to the origin). An open shell (a
//!   surface, or one drawn on both sides, whose volume is zero) adds area but
//!   no volume. A boolean difference takes away its second operand's volume
//!   where that lies inside the first's box; an intersection is not measured.
//! - **Material split**: a layered wall, floor, roof or ceiling splits its
//!   net volume across its layers by thickness (each layer is its thickness
//!   times the element's net area, the net volume over the total thickness);
//!   any other element gives each closed shell to the material most of its
//!   faces are drawn in, else to the element's own material.
//! - **Openings**: a wall drawn as an extrusion (whose doors and windows are
//!   IFC openings, not holes in the body) loses each hosted door's and
//!   window's opening box, across the wall's thickness.
//! - **Estimated**: an element still drawn as its record's bounding box (the
//!   box check's fallback) is measured as that box and flagged
//!   [`Basis::Estimated`]; it gets no IFC quantity and its own takeoff rows.
//!
//! [`attach`] writes what is measured into each element's IFC4
//! `Qto_<Class>BaseQuantities` (only names the IFC4 set defines and the
//! element does not already have, so a quantity an earlier pass verified
//! against Revit stays as it is), with one `IfcPhysicalComplexQuantity` per
//! material layer ('Layer', named after its material, holding the layer's
//! `Width` as Revit's export writes it, plus its `NetArea` and `NetVolume`).
//! [`takeoff_csv`] sums the measurements per material and category.

use std::collections::{BTreeMap, BTreeSet};

use super::IfcModel;
use super::body_geometry::{self, Body, Mesh, NO_FACE_MATERIAL};
use super::entities::{IfcBooleanOp, IfcEntity, Property, PropertySet, PropertyValue, SolidShape};
use super::export_content::{
    BEAM_AXIS_BODY_SOURCE, INSTANCE_ORIENTED_BODY_SOURCE, INSTANCE_TURNED_BODY_SOURCE,
    RECORD_BBOX_BODY_SOURCE, REVIT_OTHER_SET,
};
use super::family_bodies::FAMILY_DOCUMENT_BODY_SOURCE;
use super::proxy_shapes::PROXY_BODY_SOURCE;

/// Cubic feet to cubic metres.
const M3_PER_CUFT: f64 = 0.028_316_846_592;
/// Square feet to square metres.
const M2_PER_SQFT: f64 = 0.092_903_04;
/// Feet to metres.
const M_PER_FT: f64 = 0.3048;
/// Vertices closer than this (feet) are one vertex when shells are found.
const WELD_FEET: f64 = 1e-4;

/// How an element's quantities were found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Basis {
    /// From the element's drawn body.
    Measured,
    /// From the bounding box the element is still drawn as.
    Estimated,
}

impl Basis {
    fn label(self) -> &'static str {
        match self {
            Basis::Measured => "measured",
            Basis::Estimated => "estimated",
        }
    }
}

/// One material's share of an element.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct MaterialPart {
    pub material: String,
    /// Cubic feet.
    pub volume_cuft: f64,
    /// Square feet: a layer's net area, else the surface of the shells
    /// drawn in this material.
    pub area_sqft: f64,
    /// A layer's thickness, feet.
    pub layer_width_feet: Option<f64>,
}

/// One element's quantities.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ElementQuantities {
    /// Index into `IfcModel::entities`.
    pub index: usize,
    /// Revit ElementId (the element's tag), where known.
    pub element_id: Option<u32>,
    pub ifc_type: String,
    /// The element's `BodySource` (what its body was drawn from), if named.
    pub body_source: Option<String>,
    /// Revit category (the element's "Other" set), else its IFC class.
    pub category: String,
    pub basis: Basis,
    /// Net volume, cubic feet; `None` where no closed shell was found.
    pub net_volume_cuft: Option<f64>,
    /// Volume before openings were taken away, cubic feet.
    pub gross_volume_cuft: Option<f64>,
    /// Surface of the body, square feet.
    pub surface_sqft: f64,
    /// Total layer thickness, feet (layered elements).
    pub thickness_feet: Option<f64>,
    /// The element's `Length` quantity, feet, where one is written.
    pub length_feet: Option<f64>,
    pub materials: Vec<MaterialPart>,
}

/// What [`attach`] did.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct QuantityReport {
    /// Elements with a body.
    pub elements: usize,
    /// Of them, with a measured net volume.
    pub measured: usize,
    /// Of them, still boxes (estimated).
    pub estimated: usize,
    /// Of them, with no closed shell (no volume).
    pub no_volume: usize,
    /// Simple quantities written.
    pub quantities_written: usize,
    /// Layer complex quantities written.
    pub layer_quantities_written: usize,
}

/// A closed (or open) shell: its volume (cubic feet, signed by winding), its
/// surface (square feet) and the face material that covers most of it.
#[derive(Debug, Clone, Copy)]
struct Shell {
    volume: f64,
    area: f64,
    material: usize,
}

/// The shells of `mesh`, joined across edges two triangles share; `None`
/// where the mesh is not closed (a directed edge not matched by as many
/// reversed).
fn mesh_shells(mesh: &Mesh) -> Option<Vec<Shell>> {
    let weld = |v: u32| -> [i64; 3] {
        let p = mesh.vertices[v as usize];
        std::array::from_fn(|k| (p[k] / WELD_FEET).round() as i64)
    };
    let tris: Vec<[[i64; 3]; 3]> = mesh
        .triangles
        .iter()
        .map(|&[a, b, c]| [weld(a), weld(b), weld(c)])
        .collect();
    let mut directed: BTreeMap<([i64; 3], [i64; 3]), i64> = BTreeMap::new();
    let mut users: BTreeMap<([i64; 3], [i64; 3]), Vec<usize>> = BTreeMap::new();
    for (i, t) in tris.iter().enumerate() {
        for (s, e) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            if s == e {
                continue;
            }
            *directed.entry((s, e)).or_default() += 1;
            users
                .entry(if s <= e { (s, e) } else { (e, s) })
                .or_default()
                .push(i);
        }
    }
    if directed
        .iter()
        .any(|(&(s, e), &n)| directed.get(&(e, s)) != Some(&n))
    {
        return None;
    }
    let mut parent: Vec<usize> = (0..tris.len()).collect();
    fn find(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for list in users.values() {
        if let [a, b] = list[..] {
            let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
            parent[ra] = rb;
        }
    }
    let mut groups: BTreeMap<usize, (f64, f64, BTreeMap<usize, f64>)> = BTreeMap::new();
    for (i, &[a, b, c]) in mesh.triangles.iter().enumerate() {
        let [p, q, r] = [a, b, c].map(|v| mesh.vertices[v as usize]);
        let cross = [
            q[1] * r[2] - q[2] * r[1],
            q[2] * r[0] - q[0] * r[2],
            q[0] * r[1] - q[1] * r[0],
        ];
        let volume = (p[0] * cross[0] + p[1] * cross[1] + p[2] * cross[2]) / 6.0;
        let u = [q[0] - p[0], q[1] - p[1], q[2] - p[2]];
        let w = [r[0] - p[0], r[1] - p[1], r[2] - p[2]];
        let n = [
            u[1] * w[2] - u[2] * w[1],
            u[2] * w[0] - u[0] * w[2],
            u[0] * w[1] - u[1] * w[0],
        ];
        let area = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt() / 2.0;
        let material = mesh
            .face_materials
            .get(i)
            .copied()
            .unwrap_or(NO_FACE_MATERIAL);
        let g = groups.entry(find(&mut parent, i)).or_default();
        g.0 += volume;
        g.1 += area;
        *g.2.entry(material).or_default() += area;
    }
    Some(
        groups
            .into_values()
            .map(|(volume, area, by)| Shell {
                volume,
                area,
                material: by
                    .into_iter()
                    .max_by(|a, b| a.1.total_cmp(&b.1))
                    .map_or(NO_FACE_MATERIAL, |(m, _)| m),
            })
            .collect(),
    )
}

/// The box enclosing `mesh`'s vertices.
fn mesh_box(mesh: &Mesh) -> Option<([f64; 3], [f64; 3])> {
    let mut it = mesh.vertices.iter();
    let first = *it.next()?;
    Some(it.fold((first, first), |(lo, hi), p| {
        (
            std::array::from_fn(|k| lo[k].min(p[k])),
            std::array::from_fn(|k| hi[k].max(p[k])),
        )
    }))
}

/// The shells of `shape` (see the module docs), `None` where it cannot be
/// measured.
fn shape_shells(shape: &SolidShape) -> Option<Vec<Shell>> {
    match shape {
        SolidShape::Compound(items) => {
            let mut all = Vec::new();
            for item in items {
                all.extend(shape_shells(item)?);
            }
            Some(all)
        }
        SolidShape::BooleanResult {
            op,
            operand_a,
            operand_b,
        } => {
            let mut a = shape_shells(operand_a)?;
            match op {
                IfcBooleanOp::Union => a.extend(shape_shells(operand_b)?),
                IfcBooleanOp::Difference => {
                    let inside = match (
                        body_geometry::solid_mesh(operand_a)
                            .as_ref()
                            .and_then(mesh_box),
                        body_geometry::solid_mesh(operand_b)
                            .as_ref()
                            .and_then(mesh_box),
                    ) {
                        (Some((alo, ahi)), Some((blo, bhi))) => {
                            (0..3).all(|k| blo[k] >= alo[k] - 1e-6 && bhi[k] <= ahi[k] + 1e-6)
                        }
                        _ => false,
                    };
                    if !inside {
                        return None;
                    }
                    let removed: f64 = shape_shells(operand_b)?
                        .iter()
                        .map(|s| s.volume.abs())
                        .sum();
                    let material = a.first().map_or(NO_FACE_MATERIAL, |s| s.material);
                    a.push(Shell {
                        volume: -removed,
                        area: 0.0,
                        material,
                    });
                }
                _ => return None,
            }
            Some(a)
        }
        other => mesh_shells(&body_geometry::solid_mesh(other)?),
    }
}

fn body_shells(body: Body<'_>) -> Option<Vec<Shell>> {
    match body {
        Body::Extrusion(e) => mesh_shells(&body_geometry::extrusion_mesh(e)?),
        Body::Solid(s) => shape_shells(s),
    }
}

/// Whether `entity` is still drawn as its record's bounding box: the box
/// check's `fallback`.
pub fn is_box_fallback(entity: &IfcEntity) -> bool {
    let IfcEntity::BuildingElement {
        property_set: Some(set),
        ..
    } = entity
    else {
        return false;
    };
    let resolved = set
        .properties
        .iter()
        .any(|p| p.name == "ProfileResolved" && matches!(p.value, PropertyValue::Boolean(true)));
    match set.properties.iter().find_map(|p| match &p.value {
        PropertyValue::Text(t) if p.name == "BodySource" => Some(t.as_str()),
        _ => None,
    }) {
        Some(s) if s == RECORD_BBOX_BODY_SOURCE => !resolved,
        Some(s) => s == INSTANCE_TURNED_BODY_SOURCE || s == INSTANCE_ORIENTED_BODY_SOURCE,
        None => false,
    }
}

/// The element's `BodySource`, where it names one.
fn body_source(entity: &IfcEntity) -> Option<&str> {
    let IfcEntity::BuildingElement {
        property_set: Some(set),
        ..
    } = entity
    else {
        return None;
    };
    set.properties.iter().find_map(|p| match &p.value {
        PropertyValue::Text(t) if p.name == "BodySource" => Some(t.as_str()),
        _ => None,
    })
}

/// Whether `entity`'s body is no measure of its size: still its box
/// ([`is_box_fallback`]), a beam's location-line box (not its section),
/// an approximate stand-in, or a family document drawn
/// at its saved type's size (right for about 5% of instances).
pub fn is_estimated_body(entity: &IfcEntity) -> bool {
    is_box_fallback(entity)
        || matches!(
            body_source(entity),
            Some(s) if s == BEAM_AXIS_BODY_SOURCE
                || s == PROXY_BODY_SOURCE
                || s == FAMILY_DOCUMENT_BODY_SOURCE
        )
}

/// The IFC4 base quantity set of an IFC class and the quantities it
/// defines (IFC4 ADD2 TC1); `None` for a class with no such set.
pub fn qto_set(ifc_type: &str) -> Option<(&'static str, &'static [&'static str])> {
    const LINEAR: &[&str] = &[
        "Length",
        "CrossSectionArea",
        "OuterSurfaceArea",
        "GrossSurfaceArea",
        "NetSurfaceArea",
        "GrossVolume",
        "NetVolume",
    ];
    Some(match ifc_type {
        "IFCWALL" | "IFCWALLSTANDARDCASE" => (
            "Qto_WallBaseQuantities",
            &[
                "Length",
                "Width",
                "Height",
                "GrossFootprintArea",
                "NetFootprintArea",
                "GrossSideArea",
                "NetSideArea",
                "GrossVolume",
                "NetVolume",
            ],
        ),
        "IFCSLAB" => (
            "Qto_SlabBaseQuantities",
            &[
                "Width",
                "Length",
                "Depth",
                "Perimeter",
                "GrossArea",
                "NetArea",
                "GrossVolume",
                "NetVolume",
            ],
        ),
        "IFCROOF" => (
            "Qto_RoofBaseQuantities",
            &["GrossArea", "NetArea", "ProjectedArea"],
        ),
        "IFCCOVERING" => (
            "Qto_CoveringBaseQuantities",
            &["Width", "GrossArea", "NetArea"],
        ),
        "IFCCOLUMN" => ("Qto_ColumnBaseQuantities", LINEAR),
        "IFCBEAM" => ("Qto_BeamBaseQuantities", LINEAR),
        "IFCMEMBER" => ("Qto_MemberBaseQuantities", LINEAR),
        "IFCPLATE" => (
            "Qto_PlateBaseQuantities",
            &[
                "Width",
                "Perimeter",
                "GrossArea",
                "NetArea",
                "GrossVolume",
                "NetVolume",
            ],
        ),
        "IFCFOOTING" => (
            "Qto_FootingBaseQuantities",
            &[
                "Length",
                "Width",
                "Height",
                "CrossSectionArea",
                "OuterSurfaceArea",
                "GrossSurfaceArea",
                "GrossVolume",
                "NetVolume",
            ],
        ),
        "IFCSTAIRFLIGHT" => (
            "Qto_StairFlightBaseQuantities",
            &["Length", "GrossVolume", "NetVolume"],
        ),
        "IFCRAMPFLIGHT" => (
            "Qto_RampFlightBaseQuantities",
            &[
                "Length",
                "Width",
                "GrossArea",
                "NetArea",
                "GrossVolume",
                "NetVolume",
            ],
        ),
        "IFCBUILDINGELEMENTPROXY" => (
            "Qto_BuildingElementProxyQuantities",
            &["NetSurfaceArea", "NetVolume"],
        ),
        _ => return None,
    })
}

/// Classes that are no material (rooms, openings, annotation).
fn is_material_class(ifc_type: &str) -> bool {
    !matches!(
        ifc_type,
        "IFCSPACE" | "IFCOPENINGELEMENT" | "IFCANNOTATION" | "IFCGRID" | "IFCVIRTUALELEMENT"
    )
}

/// What one element's layers are: (material name, thickness feet), exterior
/// or top first.
fn layers_of(model: &IfcModel, entity: &IfcEntity) -> Vec<(String, f64)> {
    let IfcEntity::BuildingElement {
        material_layer_set_index,
        type_guid,
        ..
    } = entity
    else {
        return Vec::new();
    };
    let mut out: Vec<(String, f64)> = material_layer_set_index
        .and_then(|i| model.material_layer_sets.get(i))
        .map(|set| {
            set.layers
                .iter()
                .map(|l| {
                    let name = l
                        .material_index
                        .and_then(|m| model.materials.get(m))
                        .map(|m| m.name.clone())
                        .or_else(|| l.name.clone())
                        .unwrap_or_else(|| "(unnamed layer)".into());
                    (name, l.thickness_feet)
                })
                .collect()
        })
        .unwrap_or_default();
    if out.is_empty()
        && let Some(layers) = type_guid
            .as_deref()
            .and_then(|t| t.parse::<u32>().ok())
            .and_then(|id| model.element_layers.get(&id))
    {
        out = layers
            .layers
            .iter()
            .map(|l| {
                (
                    l.name.clone().unwrap_or_else(|| "(unnamed layer)".into()),
                    l.width_feet,
                )
            })
            .collect();
    }
    out.retain(|(_, t)| t.is_finite() && *t > 0.0);
    out
}

/// The element's own material: its single material, its only constituent,
/// or its profile's.
fn own_material(model: &IfcModel, index: usize, entity: &IfcEntity) -> String {
    let IfcEntity::BuildingElement {
        material_index,
        material_profile_set_index,
        ..
    } = entity
    else {
        return "(no material)".into();
    };
    let name = |m: usize| model.materials.get(m).map(|m| m.name.clone());
    material_index
        .and_then(name)
        .or_else(|| {
            model
                .material_constituent_sets
                .iter()
                .find(|s| s.elements.contains(&index))
                .and_then(|s| match s.material_indices[..] {
                    [m] => name(m),
                    _ => None,
                })
        })
        .or_else(|| {
            material_profile_set_index
                .and_then(|i| model.material_profile_sets.get(i))
                .and_then(|s| s.profiles.first())
                .and_then(|p| name(p.material_index))
        })
        .unwrap_or_else(|| "(no material)".into())
}

/// The volume (cubic feet) a hosted door or window takes out of a wall of
/// `thickness`: its opening cut, else its body's box with the axis across
/// the wall at the wall's thickness.
fn opening_volume(model: &IfcModel, filler: usize, thickness: f64) -> f64 {
    let entity = &model.entities[filler];
    let body = body_geometry::element_body(entity, &model.representation_maps);
    let mesh = body.and_then(body_geometry::body_mesh);
    let bbox = mesh.as_ref().and_then(mesh_box);
    if let Some(cut) = model.opening_cuts.get(&filler) {
        let height = cut
            .z_range_feet
            .map(|[lo, hi]| hi - lo)
            .or_else(|| bbox.map(|(lo, hi)| hi[2] - lo[2]))
            .unwrap_or(0.0);
        return (cut.x_dim_feet * cut.y_dim_feet * height).max(0.0);
    }
    bbox.map_or(0.0, |(lo, hi)| {
        (hi[0] - lo[0]).max(hi[1] - lo[1]) * (hi[2] - lo[2]) * thickness
    })
    .max(0.0)
}

/// Measure every building element of `model` (see the module docs).
pub fn measure(model: &IfcModel) -> Vec<ElementQuantities> {
    let mut category: BTreeMap<usize, String> = BTreeMap::new();
    let mut length: BTreeMap<usize, f64> = BTreeMap::new();
    for entity in &model.entities {
        if let IfcEntity::ElementPropertySet { element, set } = entity {
            if set.name == REVIT_OTHER_SET
                && let Some(PropertyValue::Label(c)) = set
                    .properties
                    .iter()
                    .find(|p| p.name == "Category")
                    .map(|p| &p.value)
            {
                category.insert(*element, c.clone());
            }
            if set.name.starts_with("Qto_")
                && let Some(l) = set.properties.iter().find_map(|p| match p.value {
                    PropertyValue::LengthFeet(l) | PropertyValue::PositiveLengthFeet(l)
                        if p.name == "Length" =>
                    {
                        Some(l)
                    }
                    _ => None,
                })
            {
                length.insert(*element, l);
            }
        }
    }
    // A body piece's shells count for the element it is part of.
    let piece_of: BTreeMap<usize, usize> = model
        .body_pieces
        .iter()
        .flat_map(|(&owner, pieces)| pieces.iter().map(move |&p| (p, owner)))
        .collect();
    let mut hosted: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (i, entity) in model.entities.iter().enumerate() {
        if let IfcEntity::BuildingElement {
            ifc_type,
            host_element_index: Some(host),
            ..
        } = entity
            && matches!(ifc_type.as_str(), "IFCDOOR" | "IFCWINDOW")
        {
            hosted.entry(*host).or_default().push(i);
        }
    }
    // Each element is measured on its own: share them out among threads,
    // keeping the model's order.
    let candidates: Vec<usize> = model
        .entities
        .iter()
        .enumerate()
        .filter(|(index, entity)| {
            matches!(entity, IfcEntity::BuildingElement { ifc_type, .. }
                if !piece_of.contains_key(index) && is_material_class(ifc_type))
        })
        .map(|(index, _)| index)
        .collect();
    let measure_one = |&index: &usize| -> Option<ElementQuantities> {
        let entity = &model.entities[index];
        let IfcEntity::BuildingElement {
            ifc_type,
            type_guid,
            ..
        } = entity
        else {
            return None;
        };
        let body = body_geometry::element_body(entity, &model.representation_maps)?;
        let extruded = matches!(
            body,
            Body::Extrusion(_) | Body::Solid(SolidShape::ExtrudedArea(_))
        );
        let mut shells = body_shells(body);
        for &piece in model.body_pieces.get(&index).into_iter().flatten() {
            let more = model
                .entities
                .get(piece)
                .and_then(|p| body_geometry::element_body(p, &model.representation_maps))
                .and_then(body_shells);
            shells = match (shells, more) {
                (Some(mut a), Some(b)) => {
                    a.extend(b);
                    Some(a)
                }
                _ => None,
            };
        }
        let own = own_material(model, index, entity);
        let basis = if is_estimated_body(entity) {
            Basis::Estimated
        } else {
            Basis::Measured
        };
        let mut q = ElementQuantities {
            index,
            element_id: type_guid.as_deref().and_then(|t| t.parse().ok()),
            body_source: body_source(entity).map(str::to_string),
            ifc_type: ifc_type.clone(),
            category: category
                .get(&index)
                .cloned()
                .unwrap_or_else(|| ifc_type.clone()),
            basis,
            net_volume_cuft: None,
            gross_volume_cuft: None,
            surface_sqft: 0.0,
            thickness_feet: None,
            length_feet: length.get(&index).copied(),
            materials: Vec::new(),
        };
        if let Some(shells) = shells {
            q.surface_sqft = shells.iter().map(|s| s.area).sum();
            // Each shell the right way out; a surface (no volume) is no
            // solid.
            let solid: Vec<Shell> = shells
                .iter()
                .copied()
                .filter(|s| s.area == 0.0 || s.volume.abs() > 1e-6 * s.area.powf(1.5))
                .collect();
            let gross: f64 = solid
                .iter()
                .map(|s| {
                    if s.area == 0.0 {
                        s.volume
                    } else {
                        s.volume.abs()
                    }
                })
                .sum();
            if !solid.is_empty() && gross > 0.0 {
                let layers = layers_of(model, entity);
                let thickness: f64 = layers.iter().map(|(_, t)| t).sum();
                let wall = matches!(ifc_type.as_str(), "IFCWALL" | "IFCWALLSTANDARDCASE");
                let removed: f64 = if wall && extruded && thickness > 0.0 {
                    hosted
                        .get(&index)
                        .into_iter()
                        .flatten()
                        .map(|&f| opening_volume(model, f, thickness))
                        .sum()
                } else {
                    0.0
                };
                let net = (gross - removed).max(0.0);
                q.net_volume_cuft = Some(net);
                q.gross_volume_cuft = Some(gross);
                if thickness > 0.0
                    && matches!(
                        ifc_type.as_str(),
                        "IFCWALL" | "IFCWALLSTANDARDCASE" | "IFCSLAB" | "IFCROOF" | "IFCCOVERING"
                    )
                {
                    q.thickness_feet = Some(thickness);
                    let area = net / thickness;
                    q.materials = layers
                        .into_iter()
                        .map(|(material, t)| MaterialPart {
                            material,
                            volume_cuft: net * t / thickness,
                            area_sqft: area,
                            layer_width_feet: Some(t),
                        })
                        .collect();
                } else {
                    let mut by: BTreeMap<String, (f64, f64)> = BTreeMap::new();
                    let scale = if gross > 0.0 { net / gross } else { 0.0 };
                    for s in &solid {
                        let name = model
                            .materials
                            .get(s.material)
                            .map_or_else(|| own.clone(), |m| m.name.clone());
                        let e = by.entry(name).or_default();
                        e.0 += if s.area == 0.0 {
                            s.volume
                        } else {
                            s.volume.abs()
                        } * scale;
                        e.1 += s.area;
                    }
                    q.materials = by
                        .into_iter()
                        .map(|(material, (v, a))| MaterialPart {
                            material,
                            volume_cuft: v.max(0.0),
                            area_sqft: a,
                            layer_width_feet: None,
                        })
                        .collect();
                }
            }
        }
        if q.materials.is_empty() {
            q.materials.push(MaterialPart {
                material: own,
                volume_cuft: 0.0,
                area_sqft: q.surface_sqft,
                layer_width_feet: None,
            });
        }
        Some(q)
    };
    crate::control::map_in_order(&candidates, measure_one)
        .into_iter()
        .flatten()
        .collect()
}

/// The IFC4 quantities `q` gives its class's set: (name, value).
fn base_quantities(q: &ElementQuantities) -> Vec<(&'static str, PropertyValue)> {
    let Some(net) = q.net_volume_cuft else {
        return Vec::new();
    };
    let mut v = vec![("NetVolume", PropertyValue::VolumeCubicFeet(net))];
    let t = q.thickness_feet.filter(|t| *t > 0.0);
    match q.ifc_type.as_str() {
        "IFCWALL" | "IFCWALLSTANDARDCASE" => {
            if let Some(gross) = q.gross_volume_cuft {
                v.push(("GrossVolume", PropertyValue::VolumeCubicFeet(gross)));
            }
            if let Some(t) = t {
                v.push(("Width", PropertyValue::PositiveLengthFeet(t)));
                v.push(("NetSideArea", PropertyValue::AreaSquareFeet(net / t)));
                if let Some(gross) = q.gross_volume_cuft {
                    v.push(("GrossSideArea", PropertyValue::AreaSquareFeet(gross / t)));
                }
            }
        }
        "IFCSLAB" | "IFCPLATE" => {
            if let Some(t) = t {
                let name = if q.ifc_type == "IFCSLAB" {
                    "Depth"
                } else {
                    "Width"
                };
                v.push((name, PropertyValue::PositiveLengthFeet(t)));
                v.push(("NetArea", PropertyValue::AreaSquareFeet(net / t)));
            }
        }
        "IFCROOF" | "IFCCOVERING" => {
            if let Some(t) = t {
                v.push(("Width", PropertyValue::PositiveLengthFeet(t)));
                v.push(("NetArea", PropertyValue::AreaSquareFeet(net / t)));
            }
        }
        _ => v.push((
            "NetSurfaceArea",
            PropertyValue::AreaSquareFeet(q.surface_sqft),
        )),
    }
    v
}

/// Measure `model` and write its quantities into each element's IFC4 base
/// quantity set (see the module docs). Returns the measurements and a report.
pub fn attach(model: &mut IfcModel) -> (Vec<ElementQuantities>, QuantityReport) {
    let measured = measure(model);
    let mut report = QuantityReport::default();
    // Existing Qto sets, by element.
    let mut existing: BTreeMap<(usize, String), usize> = BTreeMap::new();
    for (i, entity) in model.entities.iter().enumerate() {
        if let IfcEntity::ElementPropertySet { element, set } = entity
            && set.name.starts_with("Qto_")
        {
            existing.insert((*element, set.name.clone()), i);
        }
    }
    let mut new_sets: Vec<IfcEntity> = Vec::new();
    for q in &measured {
        report.elements += 1;
        match (q.basis, q.net_volume_cuft) {
            (Basis::Estimated, _) => {
                report.estimated += 1;
                continue;
            }
            (_, Some(_)) => report.measured += 1,
            (_, None) => {
                report.no_volume += 1;
                continue;
            }
        }
        let Some((set_name, allowed)) = qto_set(&q.ifc_type) else {
            continue;
        };
        let mut add: Vec<Property> = base_quantities(q)
            .into_iter()
            .filter(|(n, _)| allowed.contains(n))
            .map(|(n, value)| Property {
                name: n.into(),
                value,
            })
            .collect();
        // One complex quantity per layer, named after its material (a
        // repeated name numbered as Revit's export numbers it).
        let mut seen: BTreeMap<String, usize> = BTreeMap::new();
        for part in &q.materials {
            let Some(width) = part.layer_width_feet else {
                continue;
            };
            let n = seen.entry(part.material.clone()).or_default();
            *n += 1;
            let name = if *n == 1 {
                part.material.clone()
            } else {
                format!("{} ({n})", part.material)
            };
            add.push(Property {
                name,
                value: PropertyValue::PhysicalComplex {
                    discrimination: "Layer".into(),
                    quantities: vec![
                        Property {
                            name: "Width".into(),
                            value: PropertyValue::PositiveLengthFeet(width),
                        },
                        Property {
                            name: "NetArea".into(),
                            value: PropertyValue::AreaSquareFeet(part.area_sqft),
                        },
                        Property {
                            name: "NetVolume".into(),
                            value: PropertyValue::VolumeCubicFeet(part.volume_cuft),
                        },
                    ],
                },
            });
        }
        let set: &mut PropertySet = match existing.get(&(q.index, set_name.to_string())) {
            Some(&i) => match &mut model.entities[i] {
                IfcEntity::ElementPropertySet { set, .. } => set,
                _ => continue,
            },
            None => {
                new_sets.push(IfcEntity::ElementPropertySet {
                    element: q.index,
                    set: PropertySet {
                        name: set_name.into(),
                        properties: Vec::new(),
                    },
                });
                match new_sets.last_mut() {
                    Some(IfcEntity::ElementPropertySet { set, .. }) => set,
                    _ => continue,
                }
            }
        };
        let have: BTreeSet<String> = set.properties.iter().map(|p| p.name.clone()).collect();
        for p in add {
            if have.contains(&p.name) {
                continue;
            }
            if matches!(p.value, PropertyValue::PhysicalComplex { .. }) {
                report.layer_quantities_written += 1;
            } else {
                report.quantities_written += 1;
            }
            set.properties.push(p);
        }
    }
    new_sets.retain(
        |e| matches!(e, IfcEntity::ElementPropertySet { set, .. } if !set.properties.is_empty()),
    );
    model.entities.extend(new_sets);
    (measured, report)
}

/// CSV-quote a field.
fn field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// The materials takeoff: per material, category and basis, the volume
/// (m3), area (m2) and length (m) its elements hold and how many elements.
/// `include_estimated` false leaves the estimated (box) elements out.
pub fn takeoff_csv(measured: &[ElementQuantities], include_estimated: bool) -> String {
    #[derive(Default)]
    struct Sum {
        volume: f64,
        area: f64,
        length: f64,
        elements: BTreeSet<usize>,
        volume_elements: BTreeSet<usize>,
        length_elements: BTreeSet<usize>,
    }
    let mut rows: BTreeMap<(String, String, Basis), Sum> = BTreeMap::new();
    for q in measured {
        if q.basis == Basis::Estimated && !include_estimated {
            continue;
        }
        for part in &q.materials {
            let s = rows
                .entry((part.material.clone(), q.category.clone(), q.basis))
                .or_default();
            s.elements.insert(q.index);
            if q.net_volume_cuft.is_some() {
                s.volume += part.volume_cuft;
                s.volume_elements.insert(q.index);
            }
            s.area += part.area_sqft;
            if let Some(l) = q.length_feet {
                s.length += l;
                s.length_elements.insert(q.index);
            }
        }
    }
    let mut out = String::from("material,category,quantity,value,unit,elements,basis\n");
    for ((material, category, basis), s) in rows {
        let mut line = |quantity: &str, value: f64, unit: &str, n: usize| {
            out.push_str(&format!(
                "{},{},{quantity},{value:.6},{unit},{n},{}\n",
                field(&material),
                field(&category),
                basis.label()
            ));
        };
        if !s.volume_elements.is_empty() {
            line(
                "volume",
                s.volume * M3_PER_CUFT,
                "m3",
                s.volume_elements.len(),
            );
        }
        line("area", s.area * M2_PER_SQFT, "m2", s.elements.len());
        if !s.length_elements.is_empty() {
            line("length", s.length * M_PER_FT, "m", s.length_elements.len());
        }
    }
    out
}

/// Each element's quantities, one row per material: ElementId, class,
/// category, basis, the element's net and gross volume (m3), surface (m2),
/// layer thickness (m) and length (m), then the material's volume (m3), area
/// (m2) and layer width (m). Empty where not measured.
pub fn element_quantities_csv(measured: &[ElementQuantities]) -> String {
    let opt = |v: Option<f64>, k: f64| v.map(|v| format!("{:.6}", v * k)).unwrap_or_default();
    let mut out = String::from(
        "element_id,ifc_class,category,basis,body_source,net_volume_m3,gross_volume_m3,surface_m2,thickness_m,length_m,material,material_volume_m3,material_area_m2,layer_width_m\n",
    );
    for q in measured {
        for part in &q.materials {
            out.push_str(&format!(
                "{},{},{},{},{},{},{},{:.6},{},{},{},{},{:.6},{}\n",
                q.element_id.map(|i| i.to_string()).unwrap_or_default(),
                q.ifc_type,
                field(&q.category),
                q.basis.label(),
                field(q.body_source.as_deref().unwrap_or_default()),
                opt(q.net_volume_cuft, M3_PER_CUFT),
                opt(q.gross_volume_cuft, M3_PER_CUFT),
                q.surface_sqft * M2_PER_SQFT,
                opt(q.thickness_feet, M_PER_FT),
                opt(q.length_feet, M_PER_FT),
                field(&part.material),
                opt(q.net_volume_cuft.map(|_| part.volume_cuft), M3_PER_CUFT),
                part.area_sqft * M2_PER_SQFT,
                opt(part.layer_width_feet, M_PER_FT),
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube(size: f64) -> Mesh {
        let v = |x: f64, y: f64, z: f64| [x * size, y * size, z * size];
        let vertices = vec![
            v(0., 0., 0.),
            v(1., 0., 0.),
            v(1., 1., 0.),
            v(0., 1., 0.),
            v(0., 0., 1.),
            v(1., 0., 1.),
            v(1., 1., 1.),
            v(0., 1., 1.),
        ];
        let triangles = vec![
            [0, 2, 1],
            [0, 3, 2],
            [4, 5, 6],
            [4, 6, 7],
            [0, 1, 5],
            [0, 5, 4],
            [1, 2, 6],
            [1, 6, 5],
            [2, 3, 7],
            [2, 7, 6],
            [3, 0, 4],
            [3, 4, 7],
        ];
        Mesh {
            vertices,
            triangles,
            face_materials: Vec::new(),
        }
    }

    #[test]
    fn closed_cube_volume_and_area() {
        let shells = mesh_shells(&cube(2.0)).expect("closed");
        assert_eq!(shells.len(), 1);
        assert!((shells[0].volume - 8.0).abs() < 1e-9);
        assert!((shells[0].area - 24.0).abs() < 1e-9);
    }

    #[test]
    fn open_mesh_is_not_measured() {
        let mut m = cube(1.0);
        m.triangles.pop();
        assert!(mesh_shells(&m).is_none());
    }

    #[test]
    fn two_sided_sheet_has_no_volume() {
        let m = Mesh {
            vertices: vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
            triangles: vec![[0, 1, 2], [0, 2, 1]],
            face_materials: Vec::new(),
        };
        let shells = mesh_shells(&m).expect("balanced");
        assert!(shells.iter().all(|s| s.volume.abs() < 1e-12));
    }

    #[test]
    fn takeoff_keeps_estimated_apart() {
        let q = |index, basis| ElementQuantities {
            index,
            element_id: None,
            body_source: None,
            ifc_type: "IFCFURNISHINGELEMENT".into(),
            category: "Furniture".into(),
            basis,
            net_volume_cuft: Some(1.0 / M3_PER_CUFT),
            gross_volume_cuft: None,
            surface_sqft: 0.0,
            thickness_feet: None,
            length_feet: None,
            materials: vec![MaterialPart {
                material: "Oak, \"red\"".into(),
                volume_cuft: 1.0 / M3_PER_CUFT,
                area_sqft: 0.0,
                layer_width_feet: None,
            }],
        };
        let all = [q(1, Basis::Measured), q(2, Basis::Estimated)];
        let csv = takeoff_csv(&all, true);
        assert!(csv.contains("\"Oak, \"\"red\"\"\",Furniture,volume,1.000000,m3,1,measured"));
        assert!(csv.contains(",volume,1.000000,m3,1,estimated"));
        assert!(!takeoff_csv(&all, false).contains("estimated"));
    }
}
