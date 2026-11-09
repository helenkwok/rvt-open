//! Bodies of RPC content (trees, people, cars) from their family symbol's
//! own saved mesh, read with the schema reader on every release.
//!
//! Revit keeps an RPC symbol's display as a `GImposter` (the image) and a
//! `GPolyMesh` whose `FacetedTopology8` is the mesh Revit's own IFC export
//! writes as the element's body (rac_basic 2024: the point counts of all 10
//! RPC symbols equal those of Revit's `IfcPolygonalFaceSet`s). The symbol's
//! `GElement` (its frame id is the symbol's ElementId) holds it in the host
//! document in 2016, 2019 and 2024 alike, while the saved-graphics reader
//! reads 2023 to 2027 only. An instance places it by its `InstanceInfo`
//! transform, the pose the family-document pass uses.
//!
//! Only a symbol whose every surface is a faceted mesh is taken: a symbol
//! that also holds faced `Geometry` or nested instances is left to the other
//! passes.

use super::IfcModel;
use super::entities::{BrepTriangle, IfcEntity, PropertyValue, SolidShape};
use crate::schema_objects::{Fields, Value, for_each_object_where};
use std::collections::{BTreeMap, BTreeSet};

/// `BodySource` of a body drawn from the family symbol's saved mesh.
pub const SYMBOL_MESH_BODY_SOURCE: &str = "family_symbol_saved_mesh";

/// What [`attach`] did.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SymbolMeshReport {
    /// Boxed elements that are instances with a symbol.
    pub candidates: usize,
    /// Of those, drawn from the symbol's mesh.
    pub replaced: usize,
    pub triangles: usize,
    /// Symbols read that hold only faceted meshes.
    pub mesh_symbols: usize,
    /// Candidates mirrored in X or Y (not placed by this reader).
    pub flipped_skipped: usize,
}

/// A faceted mesh in the symbol's frame (feet).
#[derive(Debug, Default, Clone)]
struct SymbolMesh {
    vertices: Vec<[f64; 3]>,
    triangles: Vec<[u32; 3]>,
}

fn float_of(v: &Value) -> Option<f64> {
    match v {
        Value::Float(f) => Some(*f),
        Value::Int(i) => Some(*i as f64),
        _ => None,
    }
}

fn vec3_of(v: &Value) -> Option<[f64; 3]> {
    let l: Vec<f64> = match v {
        Value::List(l) => l.iter().map(float_of).collect::<Option<_>>()?,
        Value::Object(f) => f
            .values
            .iter()
            .map(|(_, v)| float_of(v))
            .collect::<Option<_>>()?,
        _ => return None,
    };
    (l.len() == 3).then(|| [l[0], l[1], l[2]])
}

fn facet_of(v: &Value) -> Option<[u32; 3]> {
    let Value::List(l) = v else { return None };
    if l.len() != 3 {
        return None;
    }
    let i = |v: &Value| match v {
        Value::Int(i) => u32::try_from(*i).ok(),
        _ => None,
    };
    Some([i(&l[0])?, i(&l[1])?, i(&l[2])?])
}

/// The faceted mesh of one `FacetedTopology*` object.
fn topology_mesh(f: &Fields) -> Option<SymbolMesh> {
    let Some(Value::List(points)) = f.get("m_pointsArr") else {
        return None;
    };
    let Some(Value::List(facets)) = f.get("m_facetsArr") else {
        return None;
    };
    let vertices: Vec<[f64; 3]> = points.iter().map(vec3_of).collect::<Option<_>>()?;
    let triangles: Vec<[u32; 3]> = facets.iter().map(facet_of).collect::<Option<_>>()?;
    if triangles.is_empty()
        || triangles
            .iter()
            .any(|t| t.iter().any(|&i| i as usize >= vertices.len()))
    {
        return None;
    }
    Some(SymbolMesh {
        vertices,
        triangles,
    })
}

/// Meshes of the symbols `wanted`, keyed by symbol id; only symbols whose
/// surfaces are all faceted meshes.
fn read_symbol_meshes(
    rf: &mut crate::RevitFile,
    wanted: &BTreeSet<u64>,
) -> anyhow::Result<BTreeMap<u64, SymbolMesh>> {
    let names: BTreeMap<u16, String> = rf
        .schema_layouts()?
        .classes
        .iter()
        .map(|(t, c)| (*t, c.name.clone()))
        .collect();
    let Some(gelement) = names
        .iter()
        .find_map(|(t, n)| (n == "GElement").then_some(*t))
    else {
        return Ok(BTreeMap::new());
    };
    // Section starts of the partitions: an object after a section header
    // lies in an embedded family document, not the host.
    let mut sections: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for name in rf.partition_stream_names() {
        if let Ok(inflated) = rf.inflated_partition(&name) {
            sections.insert(
                name,
                inflated.sections().iter().map(|(at, _)| *at).collect(),
            );
        }
    }
    let mut out = BTreeMap::new();
    for_each_object_where(
        rf,
        |frame, _| frame.header.tag == gelement && wanted.contains(&frame.header.id),
        |x| {
            let in_document = sections
                .get(&x.stream)
                .is_some_and(|s| s.partition_point(|at| *at <= x.frame.span.start) > 0);
            if in_document {
                return;
            }
            let class = |f: &Fields| names.get(&f.tag).map(String::as_str).unwrap_or("");
            let mut mesh = SymbolMesh::default();
            for o in &x.objects {
                match class(o) {
                    c if c.starts_with("FacetedTopology") => {
                        let Some(m) = topology_mesh(o) else {
                            return;
                        };
                        let base = mesh.vertices.len() as u32;
                        mesh.vertices.extend(m.vertices);
                        mesh.triangles.extend(
                            m.triangles
                                .iter()
                                .map(|t| [base + t[0], base + t[1], base + t[2]]),
                        );
                    }
                    // A faced surface or a nested symbol: not this pass's.
                    "Geometry" => {
                        if matches!(o.get("m_pFaces"), Some(Value::List(l)) if !l.is_empty()) {
                            return;
                        }
                    }
                    "GInstance" | "GComponentRef" => return,
                    _ => {}
                }
            }
            if !mesh.triangles.is_empty() {
                out.insert(x.frame.header.id, mesh);
            }
        },
    )?;
    Ok(out)
}

/// Replace each bounding-box body of an instance whose symbol holds only a
/// faceted mesh (RPC content) with that mesh, placed by the instance.
pub fn attach(rf: &mut crate::RevitFile, model: &mut IfcModel) -> anyhow::Result<SymbolMeshReport> {
    let mut report = SymbolMeshReport::default();
    let pre_2023 = rf
        .basic_file_info()
        .map(|b| b.version < 2023)
        .unwrap_or(false);
    let mut candidates: Vec<(usize, i64)> = Vec::new();
    for (index, entity) in model.entities.iter().enumerate() {
        let IfcEntity::BuildingElement {
            type_guid: Some(tag),
            property_set: Some(set),
            location_feet: Some(_),
            ..
        } = entity
        else {
            continue;
        };
        let boxed = set.properties.iter().any(|p| {
            p.name == "BodySource"
                && matches!(&p.value, PropertyValue::Text(t) if super::export_content::is_plan_box_source(t))
        });
        if let (true, Ok(id)) = (boxed, tag.parse::<i64>()) {
            candidates.push((index, id));
        }
    }
    if candidates.is_empty() {
        return Ok(report);
    }
    let docs = crate::family_documents::read_memoised(rf)?;
    let symbol_of = |id: i64| {
        let pose = docs.instances.get(&id)?;
        let symbol = pose.info_symbol.unwrap_or(pose.symbol);
        u64::try_from(symbol).ok().map(|s| (pose, s))
    };
    let wanted: BTreeSet<u64> = candidates
        .iter()
        .filter_map(|(_, id)| symbol_of(*id).map(|(_, s)| s))
        .collect();
    report.candidates = candidates
        .iter()
        .filter(|(_, id)| symbol_of(*id).is_some())
        .count();
    let meshes = read_symbol_meshes(rf, &wanted)?;
    report.mesh_symbols = meshes.len();
    for (index, id) in candidates {
        let Some((pose, symbol)) = symbol_of(id) else {
            continue;
        };
        let Some(mesh) = meshes.get(&symbol) else {
            continue;
        };
        if pose.flipped_x || pose.flipped_y {
            report.flipped_skipped += 1;
            continue;
        }
        // The instance matrix's columns are its axes.
        let m = pose.matrix;
        let axis = |j: usize| [m[0][j], m[1][j], m[2][j]];
        let (x, y, z) = (axis(0), axis(1), axis(2));
        // the symbol mesh is kept in its own frame and the
        // instance drawn through its matrix (exact instancing).
        let triangles: Vec<BrepTriangle> = mesh
            .triangles
            .iter()
            .map(|t| BrepTriangle(t[0], t[1], t[2]))
            .collect();
        let (vertices, triangles) =
            super::saved_meshes::tidy_mesh(mesh.vertices.clone(), triangles);
        if triangles.is_empty() {
            continue;
        }
        // before 2023 the element frame has no plan rotation;
        // the element takes the one its instance turns it by.
        if pre_2023
            && let IfcEntity::BuildingElement {
                rotation_radians, ..
            } = &mut model.entities[index]
            && rotation_radians.is_none()
        {
            *rotation_radians = plan_rotation(pose);
        }
        let world: super::instancing::Frame =
            std::array::from_fn(|i| [x[i], y[i], z[i], pose.origin[i]]);
        let count = triangles.len();
        let symbol = super::instancing::SymbolMesh {
            vertices_feet: vertices.clone(),
            triangles: triangles.clone(),
            face_materials: Vec::new(),
        };
        if !super::instancing::register(model, index, symbol, world) {
            let IfcEntity::BuildingElement {
                location_feet: Some(origin),
                rotation_radians,
                solid_shape,
                ..
            } = &mut model.entities[index]
            else {
                continue;
            };
            let (sin, cos) = (-rotation_radians.unwrap_or(0.0)).sin_cos();
            let vertices: Vec<[f64; 3]> = vertices
                .iter()
                .map(|p| {
                    let w = super::instancing::apply(&world, *p);
                    let (dx, dy) = (w[0] - origin[0], w[1] - origin[1]);
                    [dx * cos - dy * sin, dx * sin + dy * cos, w[2] - origin[2]]
                })
                .collect();
            let (vertices, triangles) = super::saved_meshes::tidy_mesh(vertices, triangles);
            *solid_shape = Some(SolidShape::TriangulatedFaceSet {
                vertices_feet: vertices,
                triangles,
                face_materials: Vec::new(),
            });
        }
        report.replaced += 1;
        report.triangles += count;
        let IfcEntity::BuildingElement {
            property_set: Some(set),
            ..
        } = &mut model.entities[index]
        else {
            continue;
        };
        for property in &mut set.properties {
            if property.name == "BodySource" {
                property.value = PropertyValue::Text(SYMBOL_MESH_BODY_SOURCE.into());
            }
        }
    }
    Ok(report)
}

/// The plan rotation an instance's `InstanceInfo` turns it by (its X axis,
/// the matrix's first column, about Z), when it is a plain plan rotation:
/// no tilt, no mirror.
pub(crate) fn plan_rotation(pose: &crate::family_documents::InstancePose) -> Option<f64> {
    let m = pose.matrix;
    let det = m[0][0] * m[1][1] - m[0][1] * m[1][0];
    ((m[2][2] - 1.0).abs() < 1e-6
        && m[0][2].abs() < 1e-6
        && m[1][2].abs() < 1e-6
        && (det - 1.0).abs() < 1e-6
        && !pose.flipped_x
        && !pose.flipped_y)
        .then(|| m[1][0].atan2(m[0][0]))
}

/// `BodySource` of a toposurface drawn from its saved mesh.
pub const SITE_SURFACE_BODY_SOURCE: &str = "site_surface_saved_mesh";
/// Revit's built-in Site category (`OST_Site`).
const SITE_CATEGORY: i64 = -2001260;

/// What [`add_site_content`] added.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SiteContentReport {
    /// Toposurfaces (`SiteSurface`) added as terrain, with their triangles.
    pub terrains: usize,
    pub terrain_triangles: usize,
    /// Site-category family instances (wind turbines, site furniture) the
    /// element walk does not yield, added for the body passes.
    pub site_instances: usize,
    /// Category ids of the host instances the export lacked (for reports).
    pub missing_instance_categories: BTreeMap<i64, usize>,
}

/// The first ElementId value under `v` (an id field in any of its stored shapes).
fn first_id(v: &Value) -> Option<i64> {
    match v {
        Value::Id(i) | Value::Int(i) => Some(*i),
        Value::Object(f) => f.values.iter().find_map(|(_, v)| first_id(v)),
        Value::List(l) => l.iter().find_map(first_id),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn site_entity(
    ifc_type: &str,
    name: String,
    tag: u64,
    predefined: &str,
    location: [f64; 3],
    rotation: f64,
    body_source: &str,
    solid_shape: Option<SolidShape>,
) -> IfcEntity {
    IfcEntity::BuildingElement {
        ifc_type: ifc_type.into(),
        name,
        type_guid: Some(tag.to_string()),
        predefined_type: Some(predefined.into()),
        storey_index: None,
        material_index: None,
        property_set: Some(super::entities::PropertySet {
            name: super::export_content::ELEMENT_RECORD_PROPERTY_SET.into(),
            properties: vec![super::entities::Property {
                name: "BodySource".into(),
                value: PropertyValue::Text(body_source.into()),
            }],
        }),
        location_feet: Some(location),
        rotation_radians: Some(rotation),
        extrusion: None,
        host_element_index: None,
        material_layer_set_index: None,
        material_profile_set_index: None,
        solid_shape,
        representation_map_index: None,
    }
}

/// The toposurfaces (`SiteSurface`) of a file, as Revit's export draws them.
struct SiteSurfaces {
    /// Those it draws: not owned by another (`m_unplacedOwnerId` unset; a
    /// graded copy is owned and not drawn).
    drawn: BTreeSet<u64>,
    /// Each subregion's surface: a subregion (whose id + 1 is in a surface's
    /// `m_dividingBoundaries`) is part of that surface's body (rac_basic:
    /// 607 + 144 + 160 points = Revit's 909).
    parent_of: BTreeMap<u64, u64>,
    /// The owned ones.
    owned: BTreeSet<u64>,
}

fn site_surfaces(
    rf: &mut crate::RevitFile,
    site_surface: u16,
    in_host: &dyn Fn(&str, usize) -> bool,
) -> anyhow::Result<SiteSurfaces> {
    let mut out = SiteSurfaces {
        drawn: BTreeSet::new(),
        parent_of: BTreeMap::new(),
        owned: BTreeSet::new(),
    };
    for_each_object_where(
        rf,
        |frame, _| frame.header.tag == site_surface,
        |x| {
            if !in_host(&x.stream, x.frame.span.start) {
                return;
            }
            let Some(root) = x.objects.first() else {
                return;
            };
            if matches!(root.get("m_unplacedOwnerId"), Some(Value::Id(o)) if *o != -1) {
                out.owned.insert(x.frame.header.id);
                return;
            }
            out.drawn.insert(x.frame.header.id);
            if let Some(Value::List(b)) = root.get("m_dividingBoundaries") {
                for v in b {
                    if let Value::Id(boundary) = v
                        && let Ok(sub) = u64::try_from(*boundary - 1)
                    {
                        out.parent_of.insert(sub, x.frame.header.id);
                    }
                }
            }
        },
    )?;
    Ok(out)
}

/// The topography surfaces Revit's export writes no element of their own
/// for: the owned ones (graded copies) and the subregions, which are part of
/// their surface's body. Empty where the file names no toposurface class.
pub fn hidden_site_surfaces(rf: &mut crate::RevitFile) -> BTreeSet<u64> {
    let site_surface = rf.schema_layouts().ok().and_then(|layouts| {
        layouts
            .classes
            .iter()
            .find_map(|(t, c)| (c.name == "SiteSurface").then_some(*t))
    });
    let Some(site_surface) = site_surface else {
        return BTreeSet::new();
    };
    let mut sections: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for name in rf.partition_stream_names() {
        if let Ok(inflated) = rf.inflated_partition(&name) {
            sections.insert(
                name,
                inflated.sections().iter().map(|(at, _)| *at).collect(),
            );
        }
    }
    let in_host = |stream: &str, start: usize| {
        !sections
            .get(stream)
            .is_some_and(|s| s.partition_point(|at| *at <= start) > 0)
    };
    let Ok(site) = site_surfaces(rf, site_surface, &in_host) else {
        return BTreeSet::new();
    };
    let mut hidden = site.owned;
    hidden.extend(
        site.parent_of
            .into_iter()
            .filter(|(sub, parent)| site.drawn.contains(parent) && site.drawn.contains(sub))
            .map(|(sub, _)| sub),
    );
    hidden
}

/// Site content the element walk does not yield (rac_basic: the
/// toposurface and three wind turbines that Revit's own export writes):
/// - every toposurface (`SiteSurface`) becomes an `IfcGeographicElement
///   .TERRAIN.` drawn from its saved graphics' faceted mesh, on every
///   release (model coordinates);
/// - every Site-category family instance not already exported is added
///   (Revit 2023 to 2027 only, where the saved-graphics pass then draws it),
///   as `IfcGeographicElement .USERDEFINED.`, placed by its instance.
pub fn add_site_content(
    rf: &mut crate::RevitFile,
    model: &mut IfcModel,
) -> anyhow::Result<SiteContentReport> {
    let mut report = SiteContentReport::default();
    let release = rf.basic_file_info().map(|b| b.version).unwrap_or(0);
    let names: BTreeMap<u16, String> = rf
        .schema_layouts()?
        .classes
        .iter()
        .map(|(t, c)| (*t, c.name.clone()))
        .collect();
    let tag_of = |n: &str| names.iter().find_map(|(t, c)| (c == n).then_some(*t));
    let (Some(gelement), Some(site_surface)) = (tag_of("GElement"), tag_of("SiteSurface")) else {
        return Ok(report);
    };
    let mut sections: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for name in rf.partition_stream_names() {
        if let Ok(inflated) = rf.inflated_partition(&name) {
            sections.insert(
                name,
                inflated.sections().iter().map(|(at, _)| *at).collect(),
            );
        }
    }
    let in_host = |stream: &str, start: usize| {
        !sections
            .get(stream)
            .is_some_and(|s| s.partition_point(|at| *at <= start) > 0)
    };
    // A topography surface read from its element record has its record box
    // for a body: its saved mesh replaces it below.
    let terrain_record = |e: &IfcEntity| matches!(e, IfcEntity::BuildingElement { predefined_type: Some(p), .. } if p == "TERRAIN");
    let exported: BTreeSet<u64> = model
        .entities
        .iter()
        .filter(|e| !terrain_record(e))
        .filter_map(|e| match e {
            IfcEntity::BuildingElement {
                type_guid: Some(t), ..
            } => t.parse().ok(),
            _ => None,
        })
        .collect();
    let site = site_surfaces(rf, site_surface, &in_host)?;
    let surfaces: BTreeSet<u64> = site
        .drawn
        .iter()
        .copied()
        .filter(|id| !exported.contains(id))
        .collect();
    let parent_of: BTreeMap<u64, u64> = site
        .parent_of
        .into_iter()
        .filter(|(_, parent)| surfaces.contains(parent))
        .collect();
    let docs = crate::family_documents::read_memoised(rf)?;
    let missing: BTreeMap<u64, &crate::family_documents::InstancePose> = docs
        .instances
        .iter()
        .filter_map(|(id, pose)| u64::try_from(*id).ok().map(|i| (i, pose)))
        .filter(|(id, _)| !exported.contains(id))
        .collect();
    let mut terrain: BTreeMap<u64, SymbolMesh> = BTreeMap::new();
    // The material each toposurface's mesh is drawn in (`GPolyMesh.m_materialID`).
    let mut terrain_material: BTreeMap<u64, i64> = BTreeMap::new();
    for_each_object_where(
        rf,
        |frame, _| frame.header.tag == gelement && surfaces.contains(&frame.header.id),
        |x| {
            if !in_host(&x.stream, x.frame.span.start) {
                return;
            }
            let id = x.frame.header.id;
            if surfaces.contains(&id) {
                let class = |f: &Fields| names.get(&f.tag).map(String::as_str).unwrap_or("");
                if let Some(m) = x
                    .objects
                    .iter()
                    .filter(|o| class(o) == "GPolyMesh")
                    .find_map(|o| o.get("m_materialID").and_then(first_id))
                    .filter(|m| *m > 0)
                {
                    terrain_material.entry(id).or_insert(m);
                }
                let mesh = terrain.entry(id).or_default();
                for o in x
                    .objects
                    .iter()
                    .filter(|o| class(o).starts_with("FacetedTopology"))
                {
                    if let Some(m) = topology_mesh(o) {
                        let base = mesh.vertices.len() as u32;
                        mesh.vertices.extend(m.vertices);
                        mesh.triangles.extend(
                            m.triangles
                                .iter()
                                .map(|t| [base + t[0], base + t[1], base + t[2]]),
                        );
                    }
                }
            }
        },
    )?;
    let mut resolver: Option<Option<crate::native_saved_materials::Resolver>> = None;
    let subregions: Vec<u64> = terrain
        .keys()
        .copied()
        .filter(|id| parent_of.get(id).is_some_and(|p| terrain.contains_key(p)))
        .collect();
    // each triangle's own mesh material, so a subregion keeps its
    // colour inside its surface's body.
    let mut groups: BTreeMap<u64, Vec<Option<i64>>> = terrain
        .iter()
        .map(|(id, m)| (*id, vec![None; m.triangles.len()]))
        .collect();
    for sub in subregions {
        let mesh = terrain.remove(&sub).unwrap_or_default();
        let parent_id = parent_of[&sub];
        groups.remove(&sub);
        if let Some(g) = groups.get_mut(&parent_id) {
            g.extend(std::iter::repeat_n(
                terrain_material.get(&sub).copied(),
                mesh.triangles.len(),
            ));
        }
        let parent = terrain.get_mut(&parent_id).expect("parent present");
        let base = parent.vertices.len() as u32;
        parent.vertices.extend(mesh.vertices);
        parent.triangles.extend(
            mesh.triangles
                .iter()
                .map(|t| [base + t[0], base + t[1], base + t[2]]),
        );
    }
    for (id, mesh) in terrain {
        if mesh.triangles.is_empty() {
            continue;
        }
        let triangles: Vec<BrepTriangle> = mesh
            .triangles
            .iter()
            .map(|t| BrepTriangle(t[0], t[1], t[2]))
            .collect();
        // Shaded view draws it (its graphics colour).
        let mut resolve = |model: &mut IfcModel, m: i64| {
            if resolver.is_none() {
                resolver = Some(
                    crate::native_saved_scene::material_resolver(rf, &Default::default()).ok(),
                );
            }
            resolver
                .as_ref()
                .and_then(|r| r.as_ref())
                .and_then(|r| r.resolve(id, None, 0, 0, Some(m)))
                .map(|found| super::saved_meshes::material_index_for(&mut model.materials, &found))
        };
        let material_index = terrain_material.get(&id).and_then(|m| resolve(model, *m));
        let triangle_groups: Vec<Option<usize>> = groups
            .remove(&id)
            .unwrap_or_default()
            .into_iter()
            .map(|m| m.and_then(|m| resolve(model, m)))
            .collect();
        let lookup = if triangle_groups
            .iter()
            .any(|g| g.is_some() && *g != material_index)
        {
            super::saved_meshes::face_material_lookup(&mesh.vertices, &triangles, &triangle_groups)
        } else {
            BTreeMap::new()
        };
        let (vertices, triangles) = super::saved_meshes::tidy_mesh(mesh.vertices, triangles);
        let face_materials = super::saved_meshes::face_materials_of(&lookup, &vertices, &triangles);
        report.terrains += 1;
        report.terrain_triangles += triangles.len();
        let entity = site_entity(
            "IFCGEOGRAPHICELEMENT",
            format!("Surface:{id}"),
            id,
            "TERRAIN",
            [0.0; 3],
            0.0,
            SITE_SURFACE_BODY_SOURCE,
            Some(SolidShape::TriangulatedFaceSet {
                vertices_feet: vertices,
                triangles,
                face_materials,
            }),
        );
        let tag = id.to_string();
        let at = match model.entities.iter().position(|e| {
            terrain_record(e)
                && matches!(e, IfcEntity::BuildingElement { type_guid: Some(t), .. } if *t == tag)
        }) {
            Some(at) => {
                model.entities[at] = entity;
                at
            }
            None => {
                model.entities.push(entity);
                model.entities.len() - 1
            }
        };
        if let Some(IfcEntity::BuildingElement {
            material_index: slot,
            ..
        }) = model.entities.get_mut(at)
        {
            *slot = material_index;
        }
    }
    // The category of an instance the walk missed is its family's
    // (`Family.m_categoryId`; rac_basic's wind turbines: -2001260, Site).
    let family_of = |id: &u64| {
        missing
            .get(id)
            .and_then(|p| docs.symbol_family.get(&p.info_symbol.unwrap_or(p.symbol)))
            .copied()
    };
    let wanted_families: BTreeSet<u64> = missing
        .keys()
        .filter_map(|id| family_of(id).and_then(|f| u64::try_from(f).ok()))
        .collect();
    let mut family_category: BTreeMap<u64, i64> = BTreeMap::new();
    if let Some(family_tag) = tag_of("Family") {
        for_each_object_where(
            rf,
            |frame, _| frame.header.tag == family_tag && wanted_families.contains(&frame.header.id),
            |x| {
                if in_host(&x.stream, x.frame.span.start)
                    && let Some(Value::Id(c)) =
                        x.objects.first().and_then(|r| r.get("m_categoryId"))
                {
                    family_category.insert(x.frame.header.id, *c);
                }
            },
        )?;
    }
    let mut site_ids = Vec::new();
    for id in missing.keys() {
        let category = family_of(id)
            .and_then(|f| u64::try_from(f).ok())
            .and_then(|f| family_category.get(&f).copied())
            .unwrap_or(0);
        *report
            .missing_instance_categories
            .entry(category)
            .or_default() += 1;
        if category == SITE_CATEGORY {
            site_ids.push(*id);
        }
    }
    if (2014..=2027).contains(&release) {
        for id in site_ids {
            let pose = missing[&id];
            let m = pose.matrix;
            report.site_instances += 1;
            model.entities.push(site_entity(
                "IFCGEOGRAPHICELEMENT",
                format!("Site-{id}"),
                id,
                "USERDEFINED",
                pose.origin,
                m[1][0].atan2(m[0][0]),
                super::export_content::RECORD_BBOX_BODY_SOURCE,
                None,
            ));
        }
    }
    Ok(report)
}
