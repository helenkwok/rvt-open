//! Family-instance bodies from the meshes Revit saves for display.
//!
//! A Revit file keeps the graphics it last drew for each element. The
//! native reader (`native_saved_scene`, `native_saved_mesh` and the
//! schema, graph and identity code under them) decodes them in document
//! feet. Where rvt-open has only an element record's bounding box, as for
//! doors, windows and furniture, [`attach`] replaces that box with the
//! element's saved mesh.
//!
//! On RE1's four models (Revit 2025) 207 of the 214 bodies replaced are
//! within 0.01 ft of Revit's export.
//!
//! On Core Interior the saved meshes coincide with the bodies rvt-open already
//! draws exactly (walls 351 of 360, columns 256 of 256, slabs 80 of 80), and
//! a door's mesh is its panel and frame, 3.50 × 0.83 × 8.25 ft, where its
//! record box is 3.75 ft deep because it includes the swing: the 2.92 ft
//! difference measured against Revit's own export.
//!
//! Every element drawn as its record's box is drawn from its saved mesh by
//! default on Revit 2014 to 2027 ([`default_scope`], [`SavedMeshScope::All`],
//! for the CLIs, Python and the viewer), and only where all of the mesh
//! decodes. Of the bodies scored on 31 models, the meshes of 17 classes have
//! every vertex of Revit's export within 0.001 ft, windows among them; doors
//! are 675 of 681, sanitary terminals 68 of 94, flow fittings 71 of 89, flow
//! terminals 30 of 35 and walls 14 of 18. [`SavedMeshScope::DoorsAndWindows`]
//! reads less of the file: every vertex is Revit's within 0.001 ft on Core
//! Interior's 132 doors and 6 windows and the BIM4LCA office's 269 windows.
//!
//! A roof drawn as the level block of its sketch outline at its record box's
//! full height is replaced too: the record box holds the roof's
//! reference surface, not its slope or its thickness, and its saved mesh is
//! what Revit drew. The outline's own properties (`ProfileSource`,
//! `ProfileVertexCount`, `ProfileVoidCount`) and the `SlabShapeRefusal` of
//! any body replaced go with the body they described. A member, plate or
//! column drawn from a closed mesh of flat faces gets that mesh's volume as
//! its `NetVolume`.
//!
//! One kind of wall is drawn from its saved mesh in every scope that reads
//! graphics, on Revit 2024 to 2027, the releases whose wall records name what
//! embeds in them: a wall that embeds a curtain wall (`WallEmbedWallGStep`) whose saved mesh starts at least half a foot above the wall's base
//!. Revit draws such a wall less the curtain wall's prism and less the
//! volumes of the elements that cut its end stubs, which no record read so far
//! tells apart from a wall that keeps them; the mesh Revit last drew is the
//! body, and its lowest vertex is Revit's lowest point on all 113 walls
//! measured. A wall with an opening hosted in it, a Tag shared with another
//! entity, a body that is neither a plain extrusion nor one with a stepped top
//! or a mesh that did not decode whole keeps the body it had, as does
//! one whose mesh starts at its base.

use super::entities::{
    BrepTriangle, IfcEntity, ProfileDef, Property, PropertySet, PropertyValue, SolidShape,
};
use super::export_content::{
    BEAM_AXIS_BODY_SOURCE, BEAM_BOX_BODY_SOURCE, INSTANCE_ORIENTED_BODY_SOURCE,
    INSTANCE_TURNED_BODY_SOURCE, RECORD_BBOX_BODY_SOURCE, SAVED_MESH_ELEMENT_ID_SHARED,
    SAVED_MESH_NO_GRAPHICS, SAVED_MESH_NO_MESH, SAVED_MESH_NO_TRIANGLES,
    SAVED_MESH_NOT_CANDIDATE_PREFIX, SAVED_MESH_NOT_OFFERED_PREFIX, SAVED_MESH_PARTIAL_PREFIX,
    SAVED_MESH_REFUSAL_PROPERTY, SLAB_SHAPE_REFUSAL_PROPERTY, SPACE_OUTLINE_SAVED_MESH,
    SPACE_OUTLINE_SOURCE_PROPERTY, WALL_BASE_SOURCE_PROPERTY, WALL_TOP_SLOPED_BODY_HEIGHT_SOURCE,
    WALL_TOP_STEPPED_BODY_HEIGHT_SOURCE,
};
use super::mesh_volume::{
    BeamQuantities, ColumnQuantities, MESH_LENGTH_REACH, Placed, Prism, all_faces_flat,
    beam_quantities, box_plan_width, closed_mesh_volume, column_quantities, mullion_surface_area,
    partner_ends, partners_the_boxes_missed, prism_along,
};
use super::mesh_wall_quantities::{WallFrame, WallMeshQuantities, wall_mesh_quantities};
use super::{IfcModel, MaterialInfo};
use crate::native_saved_mesh::{GraphicsMeshes, Primitive};
use crate::native_wall_cuts::WallCutKind;
use std::collections::{BTreeMap, BTreeSet};

/// `BodySource` of a body drawn from the element's saved graphics.
pub const SAVED_MESH_BODY_SOURCE: &str = "saved_graphics_mesh";

/// only duct fittings meet the measured native-reference body bound.
const COLLAPSED_RULED_CATEGORY_REFUSAL: &str =
    "collapsed ruled profile outside the measured duct fittings";

/// How far above its base a wall's saved mesh must start, and below its top,
/// for [`attach_scoped_with_control`] to draw a wall that embeds a curtain
/// wall from it, feet. On the BIM4LCA office as built and at
/// building permit the lowest vertex of such a wall's saved mesh is Revit's
/// lowest point within 0.003 ft on 113 of 113 walls: 14 stand 8.2021 or
/// 9.514 ft above their base, the curtain wall's height, and the other 99 on
/// it to within 0.001 ft.
const EMBED_RAISE_MIN_FEET: f64 = 0.5;

/// The releases whose walls' native records name what embeds in them, and so
/// the releases a wall that embeds a curtain wall is drawn from its saved mesh
/// on: the walls' records of 2023 name no embed step, so the rule
/// found none there and the scope's 2023 was never exercised.
const EMBED_HOST_REVIT_VERSIONS: std::ops::RangeInclusive<u32> = 2024..=2027;

/// What [`attach`] did.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize)]
#[non_exhaustive]
pub struct SavedMeshReport {
    /// Elements whose body was the record's bounding box.
    pub candidates: usize,
    /// Of those, elements now drawn from their saved mesh.
    pub replaced: usize,
    /// Of those, elements keeping their box because part of their saved
    /// graphics did not decode or is outside the measured category.
    pub partial: usize,
    /// Triangles written in all.
    pub triangles: usize,
    /// Instances whose mesh matched an earlier instance's within 2 mm and
    /// now carries exactly its vertices, so `share_shapes` gives them one map.
    pub unified_instances: usize,
    /// Instances with the same triangles as an earlier instance but vertices
    /// more than 2 mm from it, and the closest such miss in millimetres
    /// (why a type's instances are not one map).
    pub unify_misses: usize,
    pub unify_closest_miss_mm: usize,
    /// Triangles dropped because the saved graphics hold every face twice,
    /// once per side.
    pub back_faces_dropped: usize,
    /// Continuous rails (top rails, handrails) drawn into their railing's
    /// body: Revit keeps each as its own element, outside the railing's
    /// saved graphics.
    pub continuous_rails: usize,
    /// Walls that embed a curtain wall, drawn as plain or stepped extrusions with nothing
    /// hosted in them.
    pub embed_hosts: usize,
    /// Of those, walls now drawn from their saved mesh, Revit having raised
    /// their base to the top of the curtain wall.
    pub embed_raised: usize,
    /// Of those, walls keeping their body because part of their saved
    /// graphics did not decode or is outside the measured category.
    pub embed_partial: usize,
    /// Why the walls' native records were not read, where they were not: no
    /// wall that embeds a curtain wall is then drawn from its mesh.
    pub embed_error: Option<String>,
}

/// Each railing's continuous rails (`BaseRailing.m_continuousRailIds`): top
/// rails and handrails are elements of their own (`TopRail`, `HandRail`) with
/// their own saved graphics, which the railing's graphics (balusters, posts,
/// other rails) leave out.
fn continuous_rails(rf: &mut crate::RevitFile) -> BTreeMap<u64, Vec<u64>> {
    use crate::schema_objects::Value;
    let mut out: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
    let Ok(layouts) = rf.schema_layouts() else {
        return out;
    };
    let Some(tag) = layouts
        .classes
        .iter()
        .find(|(_, c)| c.name == "BaseRailing")
        .map(|(t, _)| *t)
    else {
        return out;
    };
    let _ = crate::schema_objects::for_each_object_where(
        rf,
        |frame, _| frame.header.tag == tag,
        |o| {
            let Some(Value::List(list)) = o.root().and_then(|r| r.get("m_continuousRailIds"))
            else {
                return;
            };
            let rails: Vec<u64> = list
                .iter()
                .filter_map(|item| match item {
                    Value::Object(pair) => match pair.get("second") {
                        Some(Value::Id(id)) => u64::try_from(*id).ok().filter(|id| *id > 0),
                        _ => None,
                    },
                    _ => None,
                })
                .collect();
            if !rails.is_empty() {
                out.insert(o.frame.header.id, rails);
            }
        },
    );
    out
}

/// Whether some face or branch of `meshes` did not decode: on the
/// BIM4LCA office 357 such meshes, none of them Revit's body. Only branches
/// left out for their view semantics leave a mesh whole.
fn meshes_partial(meshes: &GraphicsMeshes) -> bool {
    meshes
        .diagnostics
        .iter()
        .any(|diagnostic| leaves_partial(diagnostic))
}

/// Whether a decoder diagnostic leaves its mesh partial (see [`meshes_partial`]).
fn leaves_partial(diagnostic: &str) -> bool {
    !diagnostic.ends_with(crate::native_graphics_traversal::NESTED_DETAIL_FILTER_DIAGNOSTIC)
        && !diagnostic.ends_with(crate::native_graphics_traversal::VIEW_COMPONENT_DIAGNOSTIC)
}

/// A decoder diagnostic as [`SAVED_MESH_REFUSAL_PROPERTY`] says it: without the
/// object it names (the text before its first `: `) and with its digits written
/// as `#`, so that two elements with the same fault say it alike.
fn diagnostic_kind(message: &str) -> String {
    let text = message.split_once(": ").map_or(message, |(_, rest)| rest);
    text.chars()
        .map(|c| if c.is_ascii_digit() { '#' } else { c })
        .collect()
}

/// What [`SAVED_MESH_REFUSAL_PROPERTY`] says of a mesh that did not decode
/// whole: [`SAVED_MESH_PARTIAL_PREFIX`] and the distinct kinds of the
/// diagnostics that left it partial, in order, joined by `; `. None where
/// [`meshes_partial`] is false.
fn partial_refusal(meshes: &GraphicsMeshes) -> Option<String> {
    let kinds: BTreeSet<String> = meshes
        .diagnostics
        .iter()
        .filter(|diagnostic| leaves_partial(diagnostic))
        .map(|diagnostic| diagnostic_kind(diagnostic))
        .collect();
    (!kinds.is_empty()).then(|| {
        format!(
            "{SAVED_MESH_PARTIAL_PREFIX}{}",
            kinds.into_iter().collect::<Vec<_>>().join("; ")
        )
    })
}

/// `meshes` with every primitive kept in its source graph's frame
/// ([`Primitive::frame`]) moved into the model's, as the walls that embed a
/// curtain wall are measured.
fn in_world(meshes: &GraphicsMeshes) -> GraphicsMeshes {
    let mut primitives = meshes.primitives.clone();
    for primitive in &mut primitives {
        let Some(m) = primitive.frame.take() else {
            continue;
        };
        let frame = [m[0], m[1], m[2]];
        for vertex in &mut primitive.vertices {
            *vertex = super::instancing::apply(&frame, *vertex);
        }
        if super::instancing::det(&frame) < 0.0 {
            for triangle in &mut primitive.triangles {
                triangle.swap(1, 2);
            }
        }
    }
    GraphicsMeshes {
        primitives,
        diagnostics: meshes.diagnostics.clone(),
        ..Default::default()
    }
}

/// Whether an element of `ifc_type` with `properties` is drawn as a box that
/// the pass can replace with its saved mesh: its record's box, a beam's box,
/// a family instance's box turned to its angle or a curtain
/// mullion's box along its own axes (327 mullions of the BIM4LCA
/// timber office at permit, whose Revit solids have mitred ends), each a
/// stand-in for the mesh; and a roof's level block, whose record box holds the
/// reference surface and not the slope or thickness. A resolved outline
/// is the body of any other class (a floor's plan, a ceiling's).
fn is_replaceable_box(ifc_type: &str, properties: &[Property]) -> bool {
    let source = properties
        .iter()
        .find_map(|property| match &property.value {
            PropertyValue::Text(text) if property.name == "BodySource" => Some(text.as_str()),
            _ => None,
        });
    let profile_resolved = properties.iter().any(|property| {
        property.name == "ProfileResolved" && matches!(property.value, PropertyValue::Boolean(true))
    });
    matches!(
        source,
        Some(
            RECORD_BBOX_BODY_SOURCE
                | BEAM_BOX_BODY_SOURCE
                | INSTANCE_TURNED_BODY_SOURCE
                | INSTANCE_ORIENTED_BODY_SOURCE
        )
    ) && !(profile_resolved && ifc_type != "IFCROOF")
}

/// Say on each box body that no pass offered a saved mesh why none did:
/// `why` follows [`SAVED_MESH_NOT_OFFERED_PREFIX`]. Returns how many
/// elements it said it on.
pub fn note_not_offered(model: &mut IfcModel, why: &str) -> usize {
    let boxed: Vec<usize> = model
        .entities
        .iter()
        .enumerate()
        .filter_map(|(index, entity)| match entity {
            IfcEntity::BuildingElement {
                ifc_type,
                property_set: Some(set),
                ..
            } if is_replaceable_box(ifc_type, &set.properties) => Some(index),
            _ => None,
        })
        .collect();
    for &index in &boxed {
        note_refusal(
            model,
            index,
            format!("{SAVED_MESH_NOT_OFFERED_PREFIX}{why}"),
        );
    }
    boxed.len()
}

/// Say on the element at `index` why its box was not replaced by the mesh Revit
/// saved for it.
fn note_refusal(model: &mut IfcModel, index: usize, reason: String) {
    let Some(IfcEntity::BuildingElement {
        property_set: Some(set),
        ..
    }) = model.entities.get_mut(index)
    else {
        return;
    };
    set.properties
        .retain(|property| property.name != SAVED_MESH_REFUSAL_PROPERTY);
    set.properties.push(Property {
        name: SAVED_MESH_REFUSAL_PROPERTY.into(),
        value: PropertyValue::Text(reason),
    });
}

/// The vertices and triangles of `primitives`, in the frame of an element at
/// `origin` turned `rotation_radians`, feet.
fn local_mesh(
    primitives: &[Primitive],
    origin: [f64; 3],
    rotation_radians: Option<f64>,
) -> (Vec<[f64; 3]>, Vec<BrepTriangle>) {
    let (sin, cos) = (-rotation_radians.unwrap_or(0.0)).sin_cos();
    let mut vertices = Vec::new();
    let mut triangles = Vec::new();
    for primitive in primitives {
        let base = vertices.len() as u32;
        vertices.extend(primitive.vertices.iter().map(|p| {
            let (dx, dy) = (p[0] - origin[0], p[1] - origin[1]);
            [dx * cos - dy * sin, dx * sin + dy * cos, p[2] - origin[2]]
        }));
        triangles.extend(
            primitive
                .triangles
                .iter()
                .filter(|t| t.iter().all(|&v| (v as usize) < primitive.vertices.len()))
                .map(|t| BrepTriangle(base + t[0], base + t[1], base + t[2])),
        );
    }
    (vertices, triangles)
}

/// Which bounding-box bodies [`attach_scoped_with_control`] replaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SavedMeshScope {
    /// Every element drawn as its record's box (`rvt-ifc --saved-meshes`).
    All,
    /// Doors and windows only, as `rvt-ifc` and `rvt-gltf` do by default.
    DoorsAndWindows,
}

impl SavedMeshScope {
    fn includes(self, ifc_type: &str) -> bool {
        match self {
            Self::All => true,
            Self::DoorsAndWindows => matches!(ifc_type, "IFCDOOR" | "IFCWINDOW"),
        }
    }
}

/// The bodies drawn from saved graphics by default, by `rvt-ifc`, `rvt-gltf`,
/// the Python package and the viewer alike: every boxed element on Revit 2014
/// to 2027, the releases whose saved graphics are read, and none on any other
/// release.
pub fn default_scope(rf: &mut crate::RevitFile) -> Option<SavedMeshScope> {
    let release = rf.basic_file_info().ok()?.version;
    (2014..=2027)
        .contains(&release)
        .then_some(SavedMeshScope::All)
}

/// Replace each bounding-box body with the element's saved mesh, where the
/// file holds one and all of it decodes.
pub fn attach(rf: &mut crate::RevitFile, model: &mut IfcModel) -> anyhow::Result<SavedMeshReport> {
    attach_with_control(rf, model, &crate::control::WalkerControl::default())
}

/// Same as [`attach`], reading the saved graphics through
/// `research::native_saved_scene::extract_at_detail_with_control`.
pub fn attach_with_control(
    rf: &mut crate::RevitFile,
    model: &mut IfcModel,
    control: &crate::control::WalkerControl,
) -> anyhow::Result<SavedMeshReport> {
    attach_scoped_with_control(rf, model, SavedMeshScope::All, control)
}

/// Elements whose saved graphics are extracted and attached at once, so a
/// large file's scene never sits in memory whole: the browser build's 4 GiB
/// (16 GiB on wasm64) does not hold it (DACH 2024: over 2.8 GB), and natively
/// it was the largest stage (BIM4LCA housing models: about 1.6 of 2.8 GB).
/// The material pass runs once for all batches. Each element meshes from its
/// own graphs, so the result does not depend on the batch.
/// 256 natively too: against 1,024 it cut the peak by up to 47% on the
/// Memory workflow's six models for at most 9% more time.
const SAVED_SCENE_BATCH: usize = 256;

/// The saved-graphics material resolver every pass here uses, built from
/// every element once per file and kept on `rf`.
fn material_resolver(
    rf: &mut crate::RevitFile,
) -> anyhow::Result<std::sync::Arc<crate::native_saved_materials::Resolver>> {
    if let Some(resolver) = rf.cached_saved_material_resolver() {
        return Ok(resolver);
    }
    let resolver = std::sync::Arc::new(crate::native_saved_scene::saved_material_resolver(
        rf,
        &crate::native_document::Options {
            max_graph_values: 1_000_000,
            max_graph_objects: 1_000_000,
            ..Default::default()
        },
    )?);
    rf.cache_saved_material_resolver(std::sync::Arc::clone(&resolver));
    Ok(resolver)
}

/// [`SAVED_SCENE_BATCH`], or on native builds `RVT_SAVED_SCENE_BATCH` when it
/// is set, to measure the memory and time of other batch sizes. The output
/// is the same for any batch size.
#[cfg(not(target_family = "wasm"))]
fn saved_scene_batch() -> usize {
    std::env::var("RVT_SAVED_SCENE_BATCH")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(SAVED_SCENE_BATCH)
}
#[cfg(target_family = "wasm")]
fn saved_scene_batch() -> usize {
    SAVED_SCENE_BATCH
}

/// Same as [`attach_with_control`], for the bodies `scope` names only, and
/// for the walls that embed a curtain wall whatever the scope (see the
/// module).
pub fn attach_scoped_with_control(
    rf: &mut crate::RevitFile,
    model: &mut IfcModel,
    scope: SavedMeshScope,
    control: &crate::control::WalkerControl,
) -> anyhow::Result<SavedMeshReport> {
    attach_scoped_in_batches(rf, model, scope, control, saved_scene_batch())
}

/// [`attach_scoped_with_control`] extracting the saved graphics of at most
/// `batch_size` elements at a time, so memory follows the batch, not the file.
/// The result is the same for any batch size.
pub fn attach_scoped_in_batches(
    rf: &mut crate::RevitFile,
    model: &mut IfcModel,
    scope: SavedMeshScope,
    control: &crate::control::WalkerControl,
    batch_size: usize,
) -> anyhow::Result<SavedMeshReport> {
    let mut report = SavedMeshReport::default();
    // Read the source record's category, never the possibly overridden IFC class.
    // An attach without a production walk cannot qualify a collapsed body.
    let categories = rf.walked_builtin_categories();

    // the volume of each body drawn from a closed mesh of flat faces.
    let mut volumes: Vec<(usize, f64)> = Vec::new();
    let mut by_id: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
    // A stair or curtain wall is drawn by its parts; its own saved graphics
    // would draw it twice.
    let wholes: BTreeSet<usize> = model
        .entities
        .iter()
        .filter_map(|entity| match entity {
            IfcEntity::Aggregate { whole, .. } => Some(*whole),
            _ => None,
        })
        .collect();
    // why each box that keeps its box does; a box the pass does not take
    // as a candidate says so before it looks at any saved graphics.
    let mut refusals: Vec<(usize, String)> = Vec::new();
    for (index, entity) in model.entities.iter().enumerate() {
        let IfcEntity::BuildingElement {
            ifc_type,
            type_guid,
            property_set: Some(set),
            location_feet,
            ..
        } = entity
        else {
            continue;
        };
        // On Snowdon Towers Electrical 116 of 130 receptacles, outlets, conduit
        // elbows and pendants drawn as a turned box have a whole saved mesh.
        if !is_replaceable_box(ifc_type, &set.properties) {
            continue;
        }
        let id = type_guid.as_deref().and_then(|tag| tag.parse::<u64>().ok());
        let not_taken = if wholes.contains(&index) {
            Some("whole_drawn_by_its_parts")
        } else if location_feet.is_none() {
            Some("no_location")
        } else if id.is_none() {
            Some("no_element_id")
        } else if !scope.includes(ifc_type) {
            Some("class_not_in_scope")
        } else {
            None
        };
        match (id, not_taken) {
            (Some(id), None) => {
                report.candidates += 1;
                by_id.entry(id).or_default().push(index);
            }
            (_, Some(why)) => {
                refusals.push((index, format!("{SAVED_MESH_NOT_CANDIDATE_PREFIX}{why}")));
            }
            (None, None) => {}
        }
    }
    // the walls whose quantities their saved mesh gives, whether or not
    // the wall's own body is drawn from it.
    let mut walls: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
    for (index, entity) in model.entities.iter().enumerate() {
        if wholes.contains(&index) {
            continue;
        }
        let IfcEntity::BuildingElement {
            ifc_type,
            type_guid: Some(tag),
            ..
        } = entity
        else {
            continue;
        };
        if !matches!(ifc_type.as_str(), "IFCWALL" | "IFCWALLSTANDARDCASE")
            || !scope.includes(ifc_type)
        {
            continue;
        }
        if let Ok(id) = tag.parse::<u64>() {
            walls.entry(id).or_default().push(index);
        }
    }
    // the beams drawn along their location line, whose base quantities
    // their saved mesh gives, whether or not the beam's own body is drawn from
    // it: that body is the extrusion of a resolved section and stays so, as a
    // wall's does.
    let mut beam_lines: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
    for (index, entity) in model.entities.iter().enumerate() {
        if wholes.contains(&index) {
            continue;
        }
        let IfcEntity::BuildingElement {
            ifc_type,
            type_guid: Some(tag),
            property_set: Some(set),
            ..
        } = entity
        else {
            continue;
        };
        let along_its_line = set.properties.iter().any(|property| {
            property.name == "BodySource"
                && matches!(&property.value, PropertyValue::Text(text) if text == BEAM_AXIS_BODY_SOURCE)
        });
        if !ifc_type.eq_ignore_ascii_case("IFCBEAM") || !along_its_line || !scope.includes(ifc_type)
        {
            continue;
        }
        if let Ok(id) = tag.parse::<u64>() {
            beam_lines.entry(id).or_default().push(index);
        }
    }
    // the columns whose body is the rectangle their type's section gives
    // over the record's height (a resolved profile, which the pass leaves alone:
    // `is_replaceable_box`), whose base quantities their saved mesh gives all the
    // same. That body is Revit's own form for a plain section and stays so, as a
    // beam's does, unless the mesh shows another section.
    let mut column_rectangles: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
    for (index, entity) in model.entities.iter().enumerate() {
        if wholes.contains(&index) {
            continue;
        }
        let IfcEntity::BuildingElement {
            ifc_type,
            type_guid: Some(tag),
            property_set: Some(set),
            ..
        } = entity
        else {
            continue;
        };
        let boxed = set.properties.iter().any(|property| {
            property.name == "BodySource"
                && matches!(&property.value, PropertyValue::Text(source) if source == RECORD_BBOX_BODY_SOURCE)
        });
        let resolved = set.properties.iter().any(|property| {
            property.name == "ProfileResolved"
                && matches!(property.value, PropertyValue::Boolean(true))
        });
        if !ifc_type.eq_ignore_ascii_case("IFCCOLUMN")
            || !resolved
            || !boxed
            || !scope.includes(ifc_type)
        {
            continue;
        }
        if let Ok(id) = tag.parse::<u64>() {
            column_rectangles.entry(id).or_default().push(index);
        }
    }
    let outline_checks = outline_checks(model, &wholes, scope);
    let hosts = match embed_hosts(rf, model, &by_id) {
        Ok(hosts) => hosts,
        Err(error) => {
            report.embed_error = Some(error);
            BTreeMap::new()
        }
    };
    report.embed_hosts = hosts.len();
    if by_id.is_empty()
        && hosts.is_empty()
        && walls.is_empty()
        && beam_lines.is_empty()
        && column_rectangles.is_empty()
        && outline_checks.is_empty()
    {
        for (index, reason) in refusals {
            note_refusal(model, index, reason);
        }
        return Ok(report);
    }
    let railings: BTreeSet<u64> = by_id
        .iter()
        .filter(|(_, indices)| {
            indices.iter().any(|&i| {
                matches!(&model.entities[i], IfcEntity::BuildingElement { ifc_type, .. } if ifc_type == "IFCRAILING")
            })
        })
        .map(|(id, _)| *id)
        .collect();
    let rails = if railings.is_empty() {
        BTreeMap::new()
    } else {
        let mut all = continuous_rails(rf);
        all.retain(|id, _| railings.contains(id));
        all
    };
    // a large file's whole saved scene does not fit the browser
    // build's 4 GiB; extract and attach it a batch of elements at a time.
    // Each element meshes from its own graphs, so batches change nothing.
    let ids: Vec<u64> = by_id
        .keys()
        .chain(hosts.keys())
        .chain(walls.keys())
        .chain(beam_lines.keys())
        .chain(column_rectangles.keys())
        .chain(outline_checks.keys())
        .copied()
        .collect::<BTreeSet<u64>>()
        .into_iter()
        .collect();
    // each curtain mullion's mesh read as a prism along its own axis.
    let mut members: Vec<MemberMesh> = Vec::new();
    // the axis each curtain mullion runs along, by ElementId, in the model's frame.
    let mullion_axes = rf.mullion_axes();
    // the end faces each curtain mullion's solid keeps, by ElementId.
    let kept_ends = rf.mullion_kept_ends();
    // what each beam's mesh gives of its base quantities.
    let mut beams: Vec<(usize, BeamQuantities)> = Vec::new();
    // what each column's mesh gives of its base quantities.
    let mut columns: Vec<(usize, ColumnQuantities)> = Vec::new();
    // the volume of each column's mesh, whatever its faces.
    let mut column_volumes: Vec<(usize, f64)> = Vec::new();
    // what the mesh of each wall gives of its quantities.
    let mut wall_quantities: Vec<(usize, WallMeshQuantities)> = Vec::new();
    // what a wall's mesh needs besides itself to be read as a vertical
    // prism less the openings rvt-open cuts in it, the wall's own axes and the
    // boxes of those openings, and whether its base is joined to a roof, floor
    // or footing, which takes the areas away.
    let wall_openings = wall_frame_and_openings(model);
    let wall_ids: BTreeSet<u64> = walls.keys().copied().collect();
    let no_base_join =
        crate::native_wall_cuts::walls_with_no_base_join(rf, &wall_ids).unwrap_or_default();
    // the width of each curtain panel drawn from a mesh that is a vertical box.
    let mut plate_widths: Vec<(usize, f64)> = Vec::new();
    // the candidates the scene held graphics for.
    let mut seen: BTreeSet<u64> = BTreeSet::new();
    // before 2023 an instance's element frame carries no plan
    // rotation (the record gives none); its saved mesh is placed in the
    // frame its InstanceInfo turns it to, as Revit's export places it.
    let poses = pre_2023_instance_poses(rf);
    // The material pass reads every element whatever the batch selects:
    // run it once, not once per batch.
    let materials = material_resolver(rf)?;
    for batch in ids.chunks(batch_size.max(1)) {
        let mut selected: BTreeSet<u64> = batch.iter().copied().collect();
        for id in batch {
            selected.extend(rails.get(id).into_iter().flatten());
        }
        let options = crate::native_document::Options {
            selected_ids: selected,
            max_graph_values: 1_000_000,
            max_graph_objects: 1_000_000,
            value_source: Some("saved_graphics"),
            ..Default::default()
        };
        // primitives stay in their symbol's frame, so an instance
        // can share its symbol's mesh exactly (see `instancing`).
        let scene = crate::native_saved_scene::extract_framed_with_materials(
            rf,
            &options,
            3,
            control,
            true,
            Some(&*materials),
        )?;
        crate::control::stage("saved graphics: attach");
        // A railing's continuous rails, set aside to join its body.
        let rail_ids: BTreeSet<u64> = batch
            .iter()
            .flat_map(|id| rails.get(id).into_iter().flatten().copied())
            .collect();
        let mut rail_elements: BTreeMap<u64, crate::native_saved_scene::SavedElement> =
            BTreeMap::new();
        let mut elements = Vec::with_capacity(scene.elements.len());
        for element in scene.elements {
            if rail_ids.contains(&element.identity.element_id)
                && !by_id.contains_key(&element.identity.element_id)
            {
                rail_elements.insert(element.identity.element_id, element);
            } else {
                elements.push(element);
            }
        }
        // By value: each element's saved mesh is freed once its body is built,
        // so the scene shrinks as the model grows.
        for element in elements {
            let element_id = element.identity.element_id;
            if by_id.contains_key(&element_id) {
                seen.insert(element_id);
            }
            let category = u32::try_from(element_id)
                .ok()
                .and_then(|id| categories.get(&id).copied().flatten());
            if element.meshes.has_collapsed_ruled_profile
                && category != Some(crate::partition_element_records::OST_DUCT_FITTING)
            {
                // Preserve an existing decoder refusal for a body already partial.
                // Complete bodies outside the measured category keep their boxes;
                // their meshes must not supply quantities or raised wall bodies either.
                let reason = partial_refusal(&element.meshes)
                    .unwrap_or_else(|| COLLAPSED_RULED_CATEGORY_REFUSAL.into());
                if let Some(indices) = by_id.get(&element_id) {
                    report.partial += indices.len();
                    refusals.extend(indices.iter().map(|index| (*index, reason.clone())));
                }
                if let Some(&index) = hosts.get(&element_id) {
                    report.embed_partial += 1;
                    refusals.push((index, reason));
                }
                continue;
            }
            // a column drawn from its mesh rather than its rectangle.
            let mut mesh_body: Option<[usize; 1]> = None;
            // a wall's, a beam's or a column's quantities from its saved mesh,
            // whether or not its body is drawn from it; the mesh is moved into
            // the model's frame first, as these read it.
            if walls.contains_key(&element_id)
                || beam_lines.contains_key(&element_id)
                || column_rectangles.contains_key(&element_id)
            {
                let world = in_world(&element.meshes);
                if let Some([index]) = walls.get(&element_id).map(Vec::as_slice) {
                    let held = wall_openings.get(index);
                    let frame = held.map_or(WallFrame { turn: 0.0 }, |held| held.frame);
                    let boxes: &[[f64; 6]] = held.map_or(&[], |held| held.boxes.as_slice());
                    let found = wall_mesh_quantities(
                        &world.primitives,
                        &frame,
                        boxes,
                        !no_base_join.contains(&element_id),
                    );
                    if let Some(mut found) = found {
                        // The frames a batch composes can move a sum over the
                        // faces in its last bit; to 1e-9 (far below the saved
                        // corners' single precision) every batch size agrees.
                        let tidy = |x: f64| (x * 1e9).round() / 1e9;
                        found.net_volume = tidy(found.net_volume);
                        found.height_feet = tidy(found.height_feet);
                        if let Some(sides) = &mut found.sides {
                            sides.net = tidy(sides.net);
                            sides.gross = sides.gross.map(tidy);
                        }
                        wall_quantities.push((*index, found));
                    }
                }
                let whole = partial_refusal(&element.meshes).is_none();
                if let Some([index]) = beam_lines.get(&element_id).map(Vec::as_slice)
                    && whole
                    && let Some(found) = beam_mesh_quantities(&world.primitives)
                {
                    beams.push((*index, found));
                }
                if let Some([index]) = column_rectangles.get(&element_id).map(Vec::as_slice)
                    && whole
                    && let Some(found) = column_mesh_quantities(&world.primitives)
                {
                    // a column whose mesh is a vertical prism of another
                    // section than its type's rectangle is drawn from its
                    // mesh, as a box is.
                    if section_not_rectangle(&model.entities[*index], found.cross_section_area) {
                        report.candidates += 1;
                        mesh_body = Some([*index]);
                    } else {
                        columns.push((*index, found));
                    }
                }
            }
            if let Some(&index) = hosts.get(&element.identity.element_id) {
                raise_embed_host(model, index, &in_world(&element.meshes), &mut report);
                continue;
            }
            // A floor or ceiling drawn as its outline whose saved mesh, whole,
            // covers another plan area is drawn from the mesh.
            let outline_index: Vec<usize>;
            let indices: &[usize] = match (&mesh_body, outline_checks.get(&element_id)) {
                // a column drawn from its non-rectangular mesh
                (Some(index), _) => index,
                (None, Some(&(index, outline_area, cut_area))) => {
                    if element.meshes.primitives.is_empty()
                        || partial_refusal(&element.meshes).is_some()
                    {
                        continue;
                    }
                    // The openings may overlap each other or run past the
                    // outline, so the outline is drawn right wherever the mesh
                    // lies between it less all of them and it whole.
                    let mesh_area = up_area(&in_world(&element.meshes).primitives);
                    let tolerance = OUTLINE_MESH_AREA_TOLERANCE;
                    if mesh_area <= outline_area * (1.0 + tolerance)
                        && mesh_area >= (outline_area - cut_area) * (1.0 - tolerance)
                    {
                        continue;
                    }
                    if let IfcEntity::BuildingElement {
                        property_set: Some(set),
                        ..
                    } = &mut model.entities[index]
                    {
                        set.properties.push(Property {
                            name: OUTLINE_AREA_REPLACED_PROPERTY.into(),
                            value: PropertyValue::AreaSquareFeet(outline_area),
                        });
                    }
                    outline_index = vec![index];
                    &outline_index
                }
                (None, None) => match by_id.get(&element.identity.element_id) {
                    Some(indices) => indices,
                    None => continue,
                },
            };
            // A Tag carried by several entities (a slab's pieces) is ambiguous:
            // leave them as they are.
            let [index] = indices else {
                refusals.extend(
                    indices
                        .iter()
                        .map(|index| (*index, SAVED_MESH_ELEMENT_ID_SHARED.to_string())),
                );
                continue;
            };
            let wanted: BTreeSet<u64> = rails
                .get(&element.identity.element_id)
                .into_iter()
                .flatten()
                .copied()
                .collect();
            let mut own_rails: Vec<&mut crate::native_saved_scene::SavedElement> = rail_elements
                .iter_mut()
                .filter(|(id, _)| wanted.contains(id))
                .map(|(_, r)| r)
                // A rail whose graphics did not all decode is left out; the
                // railing keeps its own body (the box check counts it).
                .filter(|r| !is_partial(r))
                .collect();
            let partial = partial_refusal(&element.meshes);
            if element.meshes.primitives.is_empty() {
                refusals.push((
                    *index,
                    partial.unwrap_or_else(|| SAVED_MESH_NO_MESH.to_string()),
                ));
                continue;
            }
            // A mesh some face or branch of which did not decode is only
            // part of the body (on the BIM4LCA office 357 such meshes, none of
            // them Revit's body). Only branches left out for their view semantics
            // leave it whole.
            if let Some(partial) = partial {
                report.partial += 1;
                refusals.push((*index, partial));
                continue;
            }
            // before 2023 an instance's element frame carries no
            // plan rotation; it takes the one its InstanceInfo turns it by,
            // kept only when its saved mesh replaces its body.
            let IfcEntity::BuildingElement {
                rotation_radians, ..
            } = &mut model.entities[*index]
            else {
                continue;
            };
            let unturned = *rotation_radians;
            if unturned.is_none()
                && let Some(turned) = poses
                    .get(&(element.identity.element_id as i64))
                    .and_then(super::symbol_meshes::plan_rotation)
            {
                *rotation_radians = Some(turned);
            }
            // a curtain mullion's, a curtain panel's, a beam's and a column's
            // quantities from its mesh in the element's frame, kept where the
            // mesh replaces its body.
            let quantities = element_quantities(
                &model.entities[*index],
                &element,
                mullion_axes.as_deref(),
                kept_ends.as_deref(),
                *index,
            );
            // A body of flat faces only (no rails joined to it) has its mesh's
            // volume as its `NetVolume`, where the mesh closes.
            let flat = own_rails.is_empty() && all_faces_flat(&element.meshes.primitives);
            let replaced = 'element: {
                // each face group's material, as the model's material.
                // a railing's continuous rails join its body, after
                // its own groups.
                let group_materials: Vec<Option<usize>> = std::iter::once(&element)
                    .chain(own_rails.iter().map(|r| &**r))
                    .flat_map(|e| {
                        (0..e.meshes.primitives.len()).map(move |i| e.render_materials.get(&i))
                    })
                    .map(|m| m.map(|m| material_index_for(&mut model.materials, m)))
                    .collect();
                let primitives: Vec<&Primitive> = std::iter::once(&element)
                    .chain(own_rails.iter().map(|r| &**r))
                    .flat_map(|e| e.meshes.primitives.iter())
                    .collect();
                // each group of primitives under one placement (the
                // element's symbol, or a nested family instance such as a
                // railing's baluster) is a part, meshed in its own frame, so equal
                // parts are shared exactly. A scaled placement is placed in the
                // document as before.
                let mut dropped = 0;
                let mut done = false;
                if primitives.iter().all(|p| p.frame.is_some()) {
                    type Bits = (bool, [[u64; 4]; 4]);
                    let mut order: Vec<Bits> = Vec::new();
                    let mut groups: BTreeMap<Bits, Vec<usize>> = BTreeMap::new();
                    for (i, p) in primitives.iter().enumerate() {
                        let bits = (
                            p.source_owner_id.is_none(),
                            p.frame.expect("checked").map(|r| r.map(f64::to_bits)),
                        );
                        let g = groups.entry(bits).or_default();
                        if g.is_empty() {
                            order.push(bits);
                        }
                        g.push(i);
                    }
                    // Each part tidied in its own frame; back faces are dropped
                    // over the whole element, as before (a part's twin face can
                    // lie in another part, and the outward side is the element's).
                    let IfcEntity::BuildingElement {
                        location_feet,
                        rotation_radians,
                        ..
                    } = &model.entities[*index]
                    else {
                        break 'element false;
                    };
                    let inverse = super::instancing::inverse_placement(
                        &super::body_geometry::Placement::new(*location_feet, *rotation_radians),
                    );
                    let mut tidied = Vec::with_capacity(order.len());
                    for bits in &order {
                        let members = &groups[bits];
                        let m = primitives[members[0]].frame.expect("checked");
                        let (mut vertices, mut triangles, mut triangle_groups) =
                            (Vec::new(), Vec::new(), Vec::new());
                        for &i in members {
                            let primitive = primitives[i];
                            let base = vertices.len() as u32;
                            vertices.extend_from_slice(&primitive.vertices);
                            triangles.extend(
                                primitive
                                    .triangles
                                    .iter()
                                    .filter(|t| {
                                        t.iter().all(|&v| (v as usize) < primitive.vertices.len())
                                    })
                                    .map(|t| BrepTriangle(base + t[0], base + t[1], base + t[2])),
                            );
                            triangle_groups.resize(triangles.len(), group_materials[i]);
                        }
                        // The element's own graphics (no symbol) go into its frame
                        // first, as before: only a symbol's frame is shared.
                        let world = [m[0], m[1], m[2]];
                        let world = if primitives[members[0]].source_owner_id.is_none() {
                            let to_local = super::instancing::compose(&inverse, &world);
                            for v in &mut vertices {
                                *v = super::instancing::apply(&to_local, *v);
                            }
                            if super::instancing::det(&world) < 0.0 {
                                for t in &mut triangles {
                                    std::mem::swap(&mut t.1, &mut t.2);
                                }
                            }
                            None
                        } else {
                            Some(world)
                        };
                        let face_of = face_material_lookup(&vertices, &triangles, &triangle_groups);
                        let (vertices, triangles) = tidy_mesh(vertices, triangles);
                        tidied.push((vertices, triangles, face_of, world));
                    }
                    let kept = {
                        let (mut union, mut union_triangles) = (Vec::new(), Vec::new());
                        for (vertices, triangles, _, world) in &tidied {
                            let to_local = world.map_or(super::instancing::IDENTITY, |w| {
                                super::instancing::compose(&inverse, &w)
                            });
                            let mirrored = world.is_some_and(|w| super::instancing::det(&w) < 0.0);
                            let base = union.len() as u32;
                            union.extend(
                                vertices
                                    .iter()
                                    .map(|v| super::instancing::apply(&to_local, *v)),
                            );
                            union_triangles.extend(triangles.iter().map(|t| match mirrored {
                                true => BrepTriangle(base + t.0, base + t.2, base + t.1),
                                false => BrepTriangle(base + t.0, base + t.1, base + t.2),
                            }));
                        }
                        let before = union_triangles.len();
                        let kept = drop_back_faces(&union, union_triangles.clone());
                        dropped += before - kept.len();
                        let mut count: std::collections::HashMap<(u32, u32, u32), usize> =
                            Default::default();
                        for t in &kept {
                            *count.entry((t.0, t.1, t.2)).or_default() += 1;
                        }
                        union_triangles
                            .iter()
                            .map(|t| match count.get_mut(&(t.0, t.1, t.2)) {
                                Some(n) if *n > 0 => {
                                    *n -= 1;
                                    true
                                }
                                _ => false,
                            })
                            .collect::<Vec<bool>>()
                    };
                    let mut parts = Vec::with_capacity(tidied.len());
                    let mut at = 0;
                    for (vertices, triangles, face_of, world) in tidied {
                        let n = triangles.len();
                        let triangles: Vec<BrepTriangle> = triangles
                            .into_iter()
                            .zip(&kept[at..at + n])
                            .filter_map(|(t, k)| k.then_some(t))
                            .collect();
                        at += n;
                        if triangles.is_empty() {
                            continue;
                        }
                        let face_materials = face_materials_of(&face_of, &vertices, &triangles);
                        parts.push((
                            super::instancing::SymbolMesh {
                                vertices_feet: vertices,
                                triangles,
                                face_materials,
                            },
                            world,
                        ));
                    }
                    if parts.is_empty() {
                        break 'element false;
                    }
                    done = super::instancing::register_parts(model, *index, parts);
                    if !done {
                        dropped = 0;
                    }
                }
                if !done {
                    // The element and its rails placed in the model frame.
                    let mut placed = true;
                    let mut element = element;
                    for p in std::iter::once(&mut element)
                        .chain(own_rails.iter_mut().map(|r| &mut **r))
                        .flat_map(|e| e.meshes.primitives.iter_mut())
                    {
                        if let Some(m) = p.frame.take() {
                            placed &= crate::native_saved_mesh::transform(p, m).is_ok();
                        }
                    }
                    if !placed {
                        report.partial += 1;
                        break 'element false;
                    }
                    let rails: Vec<&crate::native_saved_scene::SavedElement> =
                        own_rails.iter().map(|r| &**r).collect();
                    let IfcEntity::BuildingElement {
                        location_feet: Some(origin),
                        rotation_radians,
                        ..
                    } = &model.entities[*index]
                    else {
                        break 'element false;
                    };
                    let (origin, rotation) = (*origin, *rotation_radians);
                    let Some((shape, _, d)) =
                        local_body(&element, &rails, origin, rotation, &mut model.materials)
                    else {
                        break 'element false;
                    };
                    dropped = d;
                    if let IfcEntity::BuildingElement { solid_shape, .. } =
                        &mut model.entities[*index]
                    {
                        *solid_shape = Some(shape);
                    }
                }
                report.back_faces_dropped += dropped;
                report.replaced += 1;
                report.continuous_rails += own_rails.len();
                if let IfcEntity::BuildingElement {
                    solid_shape: Some(SolidShape::TriangulatedFaceSet { triangles, .. }),
                    property_set: Some(set),
                    ..
                } = &mut model.entities[*index]
                {
                    report.triangles += triangles.len();
                    for property in &mut set.properties {
                        if property.name == "BodySource" {
                            property.value = PropertyValue::Text(SAVED_MESH_BODY_SOURCE.into());
                        } else if property.name == SPACE_OUTLINE_SOURCE_PROPERTY {
                            property.value = PropertyValue::Text(SPACE_OUTLINE_SAVED_MESH.into());
                        } else if property.name == "ProfileResolved" {
                            property.value = PropertyValue::Boolean(false);
                        }
                    }
                    // What describes a body that is no longer the outline block
                    // or the box: where its outline came from and why its shape
                    // was not drawn.
                    set.properties.retain(|property| {
                        !matches!(
                            property.name.as_str(),
                            "ProfileSource"
                                | "ProfileVertexCount"
                                | "ProfileVoidCount"
                                | SLAB_SHAPE_REFUSAL_PROPERTY
                        )
                    });
                }
                if flat
                    && let IfcEntity::BuildingElement {
                        solid_shape:
                            Some(SolidShape::TriangulatedFaceSet {
                                vertices_feet,
                                triangles,
                                ..
                            }),
                        ..
                    } = &model.entities[*index]
                    && let Some(volume) = closed_mesh_volume(vertices_feet, triangles)
                {
                    volumes.push((*index, volume));
                }
                true
            };
            if replaced {
                let MeshQuantities {
                    member,
                    beam,
                    column,
                    column_volume,
                    plate_width,
                } = quantities;
                members.extend(member);
                beams.extend(beam.map(|beam| (*index, beam)));
                columns.extend(column.map(|column| (*index, column)));
                column_volumes.extend(column_volume.map(|volume| (*index, volume)));
                plate_widths.extend(plate_width.map(|width| (*index, width)));
            } else {
                refusals.push((*index, SAVED_MESH_NO_TRIANGLES.to_string()));
                if let IfcEntity::BuildingElement {
                    rotation_radians, ..
                } = &mut model.entities[*index]
                {
                    *rotation_radians = unturned;
                }
            }
        }
    }
    // The elements arrive batch by batch: what they found is applied in the
    // order of the entities, so the sets appended are the same for any batch
    // size.
    volumes.sort_by_key(|(index, _)| *index);
    members.sort_by_key(|member| member.index);
    column_volumes.sort_by_key(|(index, _)| *index);
    wall_quantities.sort_by_key(|(index, _)| *index);
    plate_widths.sort_by_key(|(index, _)| *index);
    refusals.sort_by_key(|(index, _)| *index);
    drop_column_box_quantities(model, &column_volumes);
    let moved = add_mesh_volumes(model, &volumes, &members);
    follow_mesh_span(model, &moved);
    add_mullion_surface_areas(model, &members);
    add_beam_quantities(model, &mut beams);
    add_column_quantities(model, &mut columns);
    add_wall_quantities(model, &wall_quantities);
    add_plate_widths(model, &plate_widths);
    // a candidate the scene held no graphics for keeps its box too.
    for (id, indices) in &by_id {
        if !seen.contains(id) {
            refusals.extend(
                indices
                    .iter()
                    .map(|index| (*index, SAVED_MESH_NO_GRAPHICS.to_string())),
            );
        }
    }
    for (index, reason) in refusals {
        note_refusal(model, index, reason);
    }
    (
        report.unified_instances,
        report.unify_misses,
        report.unify_closest_miss_mm,
    ) = unify_instances(model);
    Ok(report)
}

/// How far, as a share, the plan area of a floor's or ceiling's saved mesh may
/// lie outside its outline's, whole and less every opening cut in it, before
/// the mesh is drawn instead ([`outline_checks`]).
const OUTLINE_MESH_AREA_TOLERANCE: f64 = 0.02;

/// Property of a floor or ceiling drawn from its saved mesh in place of its
/// outline: the outline's plan area, square feet.
pub const OUTLINE_AREA_REPLACED_PROPERTY: &str = "ReplacedOutlineArea";

/// The floors and ceilings drawn as the extrusion of their sketch outline whose
/// saved mesh is checked against it, by ElementId: the entity, the outline's
/// plan area and the sum of the areas of the openings cut in it, square feet.
/// Where the mesh decodes whole and its upward faces cover more than the
/// outline or less than the outline less every opening, by more than
/// [`OUTLINE_MESH_AREA_TOLERANCE`], the mesh is Revit's body and is drawn: on Snowdon Towers (2024) lawns 1227304 and 1235943 and slab 2238597
/// are cut by what they join (21%, 16% and 7% over Revit's Area as outlines)
/// and slab 727212 is 11% under it, where each saved mesh is Revit's Area
/// within 1%.
fn outline_checks(
    model: &IfcModel,
    wholes: &BTreeSet<usize>,
    scope: SavedMeshScope,
) -> BTreeMap<u64, (usize, f64, f64)> {
    use super::entities::ProfileDef;
    let ring = |points: &[(f64, f64)]| {
        let n = points.len();
        (0..n)
            .map(|i| {
                let (a, b) = (points[i], points[(i + 1) % n]);
                a.0 * b.1 - b.0 * a.1
            })
            .sum::<f64>()
            .abs()
            / 2.0
    };
    let mut cut: BTreeMap<usize, f64> = BTreeMap::new();
    for entity in &model.entities {
        if let IfcEntity::VoidOpening {
            host, outline_feet, ..
        } = entity
        {
            *cut.entry(*host).or_default() += ring(outline_feet);
        }
    }
    let mut tags: BTreeMap<u64, Vec<(usize, f64, f64)>> = BTreeMap::new();
    for (index, entity) in model.entities.iter().enumerate() {
        let IfcEntity::BuildingElement {
            ifc_type,
            type_guid: Some(tag),
            extrusion,
            solid_shape,
            property_set,
            ..
        } = entity
        else {
            continue;
        };
        let Ok(id) = tag.parse::<u64>() else {
            continue;
        };
        let outline = match extrusion.as_ref().and_then(|e| e.profile_override.as_ref()) {
            Some(ProfileDef::ArbitraryClosed { points }) => Some(ring(points)),
            Some(ProfileDef::ArbitraryWithVoids { points, voids }) => {
                Some(ring(points) - voids.iter().map(|void| ring(void.as_slice())).sum::<f64>())
            }
            _ => None,
        };
        let outlined = property_set.as_ref().is_some_and(|set| {
            set.properties.iter().any(|p| {
                p.name == "ProfileResolved" && matches!(p.value, PropertyValue::Boolean(true))
            }) && set.properties.iter().any(|p| {
                p.name == "BodySource"
                    && matches!(&p.value, PropertyValue::Text(t) if t == RECORD_BBOX_BODY_SOURCE)
            })
        });
        let area = outline
            .filter(|_| {
                matches!(ifc_type.as_str(), "IFCSLAB" | "IFCCOVERING")
                    && outlined
                    && solid_shape.is_none()
                    && !wholes.contains(&index)
                    && scope.includes(ifc_type)
            })
            .unwrap_or(f64::NAN);
        // every entity of a tag, so a floor written in pieces is left alone.
        let openings = cut.get(&index).copied().unwrap_or(0.0);
        tags.entry(id).or_default().push((index, area, openings));
    }
    tags.into_iter()
        .filter_map(|(id, entries)| match entries.as_slice() {
            [(index, area, openings)] if area.is_finite() && *area > 0.0 => {
                Some((id, (*index, *area, *openings)))
            }
            _ => None,
        })
        .collect()
}

/// The area, square feet, of the faces of `primitives` that look up (their
/// normal more than 0.1 up), in the model's frame.
fn up_area(primitives: &[Primitive]) -> f64 {
    primitives
        .iter()
        .flat_map(|p| {
            p.triangles.iter().filter_map(|t| {
                let [a, b, c] = t.map(|i| p.vertices.get(i as usize).copied());
                let (a, b, c) = (a?, b?, c?);
                let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
                let n = [
                    u[1] * v[2] - u[2] * v[1],
                    u[2] * v[0] - u[0] * v[2],
                    u[0] * v[1] - u[1] * v[0],
                ];
                let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                (length > 0.0 && n[2] / length > 0.1).then_some(length / 2.0)
            })
        })
        .sum()
}

/// What a mesh that replaces a body gives of its element's quantities
/// besides its `NetVolume` ([`element_quantities`]).
#[derive(Default)]
struct MeshQuantities {
    member: Option<MemberMesh>,
    beam: Option<BeamQuantities>,
    column: Option<ColumnQuantities>,
    column_volume: Option<f64>,
    plate_width: Option<f64>,
}

/// The quantities `element`'s saved mesh gives the entity at `index` (a curtain
/// mullion's prism along its own axis, a curtain panel's width, a beam's and a
/// column's base quantities, a column's mesh volume), read from the mesh in the
/// element's frame: each primitive placed in the model's frame
/// ([`in_world`]), then moved into the element's. Nothing for any other entity.
fn element_quantities(
    entity: &IfcEntity,
    element: &crate::native_saved_scene::SavedElement,
    mullion_axes: Option<&BTreeMap<u32, [f64; 3]>>,
    kept_ends: Option<&BTreeMap<u32, crate::partition_schema_mvp::KeptEnds>>,
    index: usize,
) -> MeshQuantities {
    let IfcEntity::BuildingElement {
        ifc_type,
        location_feet: Some(origin),
        rotation_radians,
        ..
    } = entity
    else {
        return MeshQuantities::default();
    };
    let kind = ifc_type.to_ascii_uppercase();
    if !matches!(
        kind.as_str(),
        "IFCMEMBER" | "IFCPLATE" | "IFCBEAM" | "IFCCOLUMN"
    ) {
        return MeshQuantities::default();
    }
    let world = in_world(&element.meshes);
    let primitives = &world.primitives;
    let (vertices, triangles) = local_mesh(primitives, *origin, *rotation_radians);
    if triangles.is_empty() {
        return MeshQuantities::default();
    }
    let volume = all_faces_flat(primitives)
        .then(|| closed_mesh_volume(&vertices, &triangles))
        .flatten();
    let plate_width = volume
        .filter(|_| kind == "IFCPLATE")
        .and_then(|volume| box_plan_width(&vertices, &triangles, volume));
    let column_volume = (kind == "IFCCOLUMN")
        .then(|| oriented_mesh_volume(primitives, *origin))
        .flatten();
    // the mullion's own axis, from its record, turned into the element's
    // frame as its mesh is.
    let member = volume.filter(|_| kind == "IFCMEMBER").and_then(|_| {
        let id = u32::try_from(element.identity.element_id).ok()?;
        let axis = *mullion_axes?.get(&id)?;
        let turn = rotation_radians.unwrap_or(0.0);
        let (sin, cos) = (-turn).sin_cos();
        let local = [
            axis[0] * cos - axis[1] * sin,
            axis[0] * sin + axis[1] * cos,
            axis[2],
        ];
        let prism = prism_along(&vertices, &triangles, local)?;
        // The end faces' centres in the model's frame, where mullions are
        // compared with one another.
        let (sin, cos) = turn.sin_cos();
        let world = |p: [f64; 3]| {
            [
                origin[0] + p[0] * cos - p[1] * sin,
                origin[1] + p[0] * sin + p[1] * cos,
                origin[2] + p[2],
            ]
        };
        let placed = Placed {
            axis,
            ends: prism.ends.map(|end| world(end.centre)),
            areas: prism.ends.map(|end| end.area),
        };
        let kept = kept_ends.and_then(|kept| kept.get(&id).copied());
        Some(MemberMesh {
            index,
            kept,
            prism,
            placed,
        })
    });
    // a beam's mesh, where it is a closed solid of flat faces, as the prism
    // Revit's export extrudes its profile into.
    let beam = volume
        .filter(|_| kind == "IFCBEAM")
        .and_then(|_| beam_quantities(&vertices, &triangles));
    // a column's mesh, where it is a closed solid of flat faces, as the
    // vertical prism Revit's export extrudes its profile into.
    let column = volume
        .filter(|_| kind == "IFCCOLUMN")
        .and_then(|_| column_quantities(&vertices, &triangles));
    MeshQuantities {
        member,
        beam,
        column,
        column_volume,
        plate_width,
    }
}

/// How far, relative to the rectangle's, the section of a column's mesh may
/// differ from its type's rectangle and the rectangle still be the column's
/// section: the mesh's single-precision corners move a section's area
/// by far less, and the BIM4LCA housing's chamfered columns are 0.3% and 1.1%
/// under their rectangles.
const COLUMN_SECTION_TOLERANCE: f64 = 1e-3;

/// Whether `entity`, a column drawn as its type's rectangle, has a mesh whose
/// section, of area `mesh_area` (square feet), is not that rectangle:
/// a chamfered or thin-walled section, which the mesh draws and the rectangle
/// does not. False where the body is not a rectangle's extrusion.
fn section_not_rectangle(entity: &IfcEntity, mesh_area: f64) -> bool {
    let IfcEntity::BuildingElement {
        extrusion: Some(extrusion),
        ..
    } = entity
    else {
        return false;
    };
    let (width, depth) = match &extrusion.profile_override {
        None => (extrusion.width_feet, extrusion.depth_feet),
        Some(ProfileDef::Rectangle {
            width_feet,
            depth_feet,
        }) => (*width_feet, *depth_feet),
        Some(_) => return false,
    };
    let area = width * depth;
    area.is_finite()
        && area > 0.0
        && mesh_area.is_finite()
        && (mesh_area - area).abs() > COLUMN_SECTION_TOLERANCE * area
}

/// How far a column's box volume may differ from the volume of the mesh Revit
/// saved for it, as a fraction of the mesh's, and still be its solid:
/// the mesh of a cylinder drawn with chords is within 1% of it, and a round
/// column's box (the circle's bounding square) is 27% over it.
const COLUMN_BOX_MESH_TOLERANCE: f64 = 0.02;

/// The volume, cubic feet, of the surface `primitives` make, each triangle
/// turned outward by its vertices' normals and measured from `origin`, or none
/// where a primitive has no normals for its vertices or the volume is not
/// positive. It does not need the surface closed or its faces flat, so it reads
/// the chord-drawn side of a cylinder to the accuracy of its chords.
fn oriented_mesh_volume(primitives: &[Primitive], origin: [f64; 3]) -> Option<f64> {
    let sub = |a: [f64; 3], b: [f64; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let cross = |a: [f64; 3], b: [f64; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let mut total = 0.0;
    for primitive in primitives {
        if primitive.normals.len() != primitive.vertices.len() {
            return None;
        }
        for triangle in &primitive.triangles {
            let [i, j, k] = triangle.map(|index| index as usize);
            let a = sub(*primitive.vertices.get(i)?, origin);
            let mut b = sub(*primitive.vertices.get(j)?, origin);
            let mut c = sub(*primitive.vertices.get(k)?, origin);
            let outward = [i, j, k].map(|index| primitive.normals[index]);
            let sum = [
                outward[0][0] + outward[1][0] + outward[2][0],
                outward[0][1] + outward[1][1] + outward[2][1],
                outward[0][2] + outward[1][2] + outward[2][2],
            ];
            if dot(cross(sub(b, a), sub(c, a)), sum) < 0.0 {
                std::mem::swap(&mut b, &mut c);
            }
            total += dot(a, cross(b, c)) / 6.0;
        }
    }
    (total.is_finite() && total > 0.0).then_some(total)
}

/// Take from each column of `found` (an index into the entities and the volume
/// of the mesh its body was drawn from) the quantities its record's box gave
/// where the box is not its solid: its `CrossSectionArea`,
/// `OuterSurfaceArea`, `GrossVolume` and `NetVolume`, written for a type whose
/// length parameter `b` is both sides of its box, which a round column
/// of that type is as well. Its `Length` stays. The box is its solid where its
/// held `NetVolume`, or its `GrossVolume` where it holds no other, is within
/// [`COLUMN_BOX_MESH_TOLERANCE`] of its mesh's volume. On Autodesk's
/// rac_advanced sample 6 round columns of 450 mm and as many in each of its
/// three later saves hold the square's areas and volumes, 4/pi of the circle's.
fn drop_column_box_quantities(model: &mut IfcModel, found: &[(usize, f64)]) {
    if found.is_empty() {
        return;
    }
    let mut held: BTreeMap<usize, usize> = BTreeMap::new();
    for (position, entity) in model.entities.iter().enumerate() {
        if let IfcEntity::ElementPropertySet { element, set } = entity {
            if set.name == "Qto_ColumnBaseQuantities" {
                held.entry(*element).or_insert(position);
            }
        }
    }
    for &(index, mesh_volume) in found {
        let Some(&position) = held.get(&index) else {
            continue;
        };
        let Some(IfcEntity::ElementPropertySet { set, .. }) = model.entities.get_mut(position)
        else {
            continue;
        };
        let volume_of = |wanted: &str| {
            set.properties
                .iter()
                .find_map(|property| match &property.value {
                    PropertyValue::VolumeCubicFeet(volume) if property.name == wanted => {
                        Some(*volume)
                    }
                    _ => None,
                })
        };
        let Some(box_volume) = volume_of("NetVolume").or_else(|| volume_of("GrossVolume")) else {
            continue;
        };
        if (box_volume - mesh_volume).abs() > COLUMN_BOX_MESH_TOLERANCE * mesh_volume {
            set.properties.retain(|property| {
                !matches!(
                    property.name.as_str(),
                    "CrossSectionArea" | "OuterSurfaceArea" | "GrossVolume" | "NetVolume"
                )
            });
        }
    }
}

/// Give each element of `volumes` (an index into the entities and the volume,
/// cubic feet, of the closed mesh of flat faces its body was drawn from) the
/// `NetVolume` that Revit's export writes for it, in the base quantity set of
/// its entity, where it holds none: Revit's IFC exporter's `NetVolumeCalculator`
/// takes the volume of the solid it writes (CLEANROOM.md source 5),
/// and the mesh is that solid, which a body of flat faces gives exactly. A
/// column gets no `GrossVolume` here: Revit's export omits it on the 18 columns of
/// the BIM4LCA office at building permit that it draws as a cut mesh, and the
/// volume alone does not tell them from the rest ([`add_column_quantities`]
/// gives it where the mesh is a vertical prism). A curtain mullion, a curtain
/// panel and a column are the entities measured equal to Revit's on every element,
/// and a building element proxy since (256 values on 26 models with a
/// Revit export, eight held-out pairs of 2024 to 2027 among them, none different);
/// a body of another entity gets none. A proxy's saved
/// mesh is a tessellation wherever its solid has a curved face, which
/// [`all_faces_flat`] refuses, so on the BIM4LCA office as built 8 of its 12
/// proxies take Revit's `NetVolume` and the ones drawn from a profile with rounded
/// folds get none. A curtain mullion
/// whose `NetVolume` the box gave and whose mesh has another volume is not an
/// extrusion of its box, and takes the mesh's and the mesh's length:
/// the mullions of `members` whose `Length` it changed are returned with
/// the new one, for [`follow_mesh_span`].
fn add_mesh_volumes(
    model: &mut IfcModel,
    volumes: &[(usize, f64)],
    members: &[MemberMesh],
) -> Vec<(usize, f64)> {
    let mut moved = Vec::new();
    if volumes.is_empty() {
        return moved;
    }
    let extents: BTreeMap<usize, f64> = members
        .iter()
        .map(|member| (member.index, member.prism.extent))
        .collect();
    let mut held: BTreeMap<(usize, String), usize> = BTreeMap::new();
    for (position, entity) in model.entities.iter().enumerate() {
        if let IfcEntity::ElementPropertySet { element, set } = entity {
            if set.name.starts_with("Qto_") {
                held.entry((*element, set.name.clone())).or_insert(position);
            }
        }
    }
    for &(index, volume) in volumes {
        let Some(IfcEntity::BuildingElement { ifc_type, .. }) = model.entities.get(index) else {
            continue;
        };
        let (set_name, names): (&str, &[&str]) = match ifc_type.to_ascii_uppercase().as_str() {
            "IFCMEMBER" => ("Qto_MemberBaseQuantities", &["NetVolume"]),
            "IFCPLATE" => ("Qto_PlateBaseQuantities", &["NetVolume"]),
            "IFCCOLUMN" => ("Qto_ColumnBaseQuantities", &["NetVolume"]),
            "IFCBUILDINGELEMENTPROXY" => ("Qto_BuildingElementProxyQuantities", &["NetVolume"]),
            _ => continue,
        };
        let volume_property = |name: &str| Property {
            name: name.into(),
            value: PropertyValue::VolumeCubicFeet(volume),
        };
        let key = (index, set_name.to_string());
        match held.get(&key) {
            Some(&position) => {
                if let Some(IfcEntity::ElementPropertySet { set, .. }) =
                    model.entities.get_mut(position)
                {
                    for &name in names {
                        if !has_property(set, name) {
                            set.properties.push(volume_property(name));
                        }
                    }
                    if set_name == "Qto_MemberBaseQuantities" {
                        let extent = extents.get(&index).copied();
                        if let Some(length) = follow_mesh_over_box(set, volume, extent) {
                            moved.push((index, length));
                        }
                    }
                }
            }
            None => {
                held.insert(key, model.entities.len());
                model.entities.push(IfcEntity::ElementPropertySet {
                    element: index,
                    set: PropertySet {
                        name: set_name.into(),
                        properties: names.iter().map(|&name| volume_property(name)).collect(),
                    },
                });
            }
        }
    }
    moved
}

/// How far a curtain mullion's box volume may differ from the volume of the
/// mesh Revit saved for it, as a fraction of the mesh's, before the box is not
/// its solid. A mullion whose `NetVolume` is Revit's from its box has
/// the mesh's volume to within 1e-4; one with a 45 degree end differs by 2.6 to
/// 3.7%.
const BOX_MESH_VOLUME_TOLERANCE: f64 = 1e-3;

/// How far, feet, the extent of a mullion's mesh must differ from its written
/// `Length` to replace it: the mesh's corners are single precision,
/// which moves an extent by 1e-5 ft, and the box overruns the solid by 0.03 ft
/// or more where it does.
const MESH_LENGTH_MOVES_FEET: f64 = 1e-4;

/// A curtain mullion whose `NetVolume` was written from its box and
/// whose saved mesh has another volume is not an extrusion of that box, as a
/// mullion with a 45 degree end is not: Revit's `NetVolume` is the mesh's,
/// and the box's `NetSurfaceArea`, its sides and the end faces its
/// neighbours leave, is not Revit's, so it is left out. Its `Length` is
/// the extent of the mesh along its own axis: Revit's `Length` is the
/// extent of its own body, which the saved mesh is, and a box that overruns the
/// solid gives a longer one. `extent` is that of the mesh along the mullion's own
/// axis, where it is a prism along it ([`prism_along`]); it replaces the `Length`
/// where it is within `MESH_LENGTH_REACH` of it, and otherwise the written
/// `Length` stays. Returns the new `Length`, where there is one.
fn follow_mesh_over_box(
    set: &mut PropertySet,
    mesh_volume: f64,
    extent: Option<f64>,
) -> Option<f64> {
    if !(mesh_volume.is_finite() && mesh_volume > 0.0) {
        return None;
    }
    let box_volume = set
        .properties
        .iter()
        .find_map(|property| match &property.value {
            PropertyValue::VolumeCubicFeet(volume) if property.name == "NetVolume" => Some(*volume),
            _ => None,
        })?;
    if (box_volume - mesh_volume).abs() <= BOX_MESH_VOLUME_TOLERANCE * mesh_volume {
        return None;
    }
    for property in &mut set.properties {
        if property.name == "NetVolume" {
            property.value = PropertyValue::VolumeCubicFeet(mesh_volume);
        }
    }
    set.properties
        .retain(|property| property.name != "NetSurfaceArea");
    let length = set
        .properties
        .iter_mut()
        .find(|property| property.name == "Length")?;
    let PropertyValue::PositiveLengthFeet(written) = &mut length.value else {
        return None;
    };
    let extent = extent?;
    let off = (extent - *written).abs();
    if off > MESH_LENGTH_REACH * *written || off <= MESH_LENGTH_MOVES_FEET {
        return None;
    }
    *written = extent;
    Some(extent)
}

/// A curtain mullion drawn from its saved mesh, closed and of flat faces, read as
/// a prism along its own axis.
struct MemberMesh {
    /// The mullion's index into the entities.
    index: usize,
    /// How many of its two end faces its Revit solid keeps, and which of those
    /// ends have a partner, where its curtain wall's boxes say.
    kept: Option<crate::partition_schema_mvp::KeptEnds>,
    /// The mesh as a prism along the mullion's axis.
    prism: Prism,
    /// Where its two end faces are, in the model's frame.
    placed: Placed,
}

/// Give each curtain mullion of `members` that holds no `NetSurfaceArea` the one
/// Revit's export writes, from its mesh: the area of its faces along
/// its own axis and of the end faces its solid keeps
/// ([`mullion_surface_area`]). The mesh is Revit's solid, so its lateral and end
/// areas are Revit's; `kept` is rule on the boxes of the mullion's
/// curtain wall, which is read for a mullion on any host, where the box's own
/// `NetSurfaceArea` is written only for one on a plain host. A mullion whose
/// box gave a `NetSurfaceArea` that its mesh did not contradict keeps
/// it. Where a mullion has a partner at an end that `kept` counts, the count
/// misses a continuation on another curtain wall, and the mullion gets none:
/// `kept` must leave every end with a partner (`partner_ends`) among the ends it
/// does not count, but for the ends at which the boxes saw the partner and kept
/// the end, as an oblique mullion or one across inside keeps it.
/// The mesh's two ends are matched to the boxes' by which side of the
/// mullion's axis each lies on ([`partners_the_boxes_missed`]), so that a partner
/// at the end the boxes saw none at is missed whatever lies at the other.
fn add_mullion_surface_areas(model: &mut IfcModel, members: &[MemberMesh]) {
    let mut held: BTreeMap<usize, usize> = BTreeMap::new();
    for (position, entity) in model.entities.iter().enumerate() {
        if let IfcEntity::ElementPropertySet { element, set } = entity {
            if set.name == "Qto_MemberBaseQuantities" {
                held.entry(*element).or_insert(position);
            }
        }
    }
    let placed: Vec<Placed> = members.iter().map(|member| member.placed).collect();
    let partners = partner_ends(&placed);
    for (member, partners) in members.iter().zip(partners) {
        let (Some(ends), Some(&position)) = (member.kept, held.get(&member.index)) else {
            continue;
        };
        let kept = ends.kept;
        // The partners the boxes saw at the ends they keep are no continuation they
        // missed; a partner at any other end is.
        let missed = partners_the_boxes_missed(&member.placed, partners, ends.partnered);
        if missed > usize::from(2 - kept.min(2)) {
            continue;
        }
        let Some(IfcEntity::ElementPropertySet { set, .. }) = model.entities.get_mut(position)
        else {
            continue;
        };
        if has_property(set, "NetSurfaceArea") {
            continue;
        }
        let Some(area) = mullion_surface_area(&member.prism, kept) else {
            continue;
        };
        set.properties.push(Property {
            name: "NetSurfaceArea".into(),
            value: PropertyValue::AreaSquareFeet(area),
        });
    }
}

/// What the saved mesh of a beam drawn along its line gives of its base
/// quantities, where all its faces are planes and it is one closed
/// solid that is a prism. The quantities are lengths, areas and a volume, which
/// where the mesh is placed does not change.
fn beam_mesh_quantities(primitives: &[Primitive]) -> Option<BeamQuantities> {
    if !all_faces_flat(primitives) {
        return None;
    }
    let (vertices, triangles) = local_mesh(primitives, [0.0; 3], None);
    beam_quantities(&vertices, &triangles)
}

/// What the saved mesh of a column drawn as its type's rectangle gives of its
/// base quantities, where all its faces are planes and it is one closed
/// solid that is a prism along the vertical. Where the mesh is placed
/// does not change the lengths, areas and volume.
fn column_mesh_quantities(primitives: &[Primitive]) -> Option<ColumnQuantities> {
    if !all_faces_flat(primitives) {
        return None;
    }
    let (vertices, triangles) = local_mesh(primitives, [0.0; 3], None);
    column_quantities(&vertices, &triangles)
}

/// Give each beam of `found` (an index into the entities and what its mesh gives
/// of its base quantities) the quantities Revit's export writes for it, in the
/// base quantity set it holds or a new one, where that holds none of them:
/// `CrossSectionArea`, `OuterSurfaceArea`, `GrossSurfaceArea`,
/// `NetSurfaceArea`, `GrossVolume` and `NetVolume`. They are taken in the order
/// of the entities, so that the sets a beam that has none gets are appended in
/// one order whichever order the elements were meshed in.
fn add_beam_quantities(model: &mut IfcModel, found: &mut [(usize, BeamQuantities)]) {
    if found.is_empty() {
        return;
    }
    found.sort_by_key(|(index, _)| *index);
    let mut held: BTreeMap<usize, usize> = BTreeMap::new();
    for (position, entity) in model.entities.iter().enumerate() {
        if let IfcEntity::ElementPropertySet { element, set } = entity {
            if set.name == "Qto_BeamBaseQuantities" {
                held.entry(*element).or_insert(position);
            }
        }
    }
    for (index, quantities) in found.iter() {
        let values = [
            (
                "CrossSectionArea",
                PropertyValue::AreaSquareFeet(quantities.cross_section_area),
            ),
            (
                "OuterSurfaceArea",
                PropertyValue::AreaSquareFeet(quantities.outer_surface_area),
            ),
            (
                "GrossSurfaceArea",
                PropertyValue::AreaSquareFeet(quantities.surface_area),
            ),
            (
                "NetSurfaceArea",
                PropertyValue::AreaSquareFeet(quantities.surface_area),
            ),
            (
                "GrossVolume",
                PropertyValue::VolumeCubicFeet(quantities.volume),
            ),
            (
                "NetVolume",
                PropertyValue::VolumeCubicFeet(quantities.volume),
            ),
        ];
        match held.get(index) {
            Some(&position) => {
                if let Some(IfcEntity::ElementPropertySet { set, .. }) =
                    model.entities.get_mut(position)
                {
                    for (name, value) in values {
                        if !has_property(set, name) {
                            set.properties.push(Property {
                                name: name.into(),
                                value,
                            });
                        }
                    }
                }
            }
            None => {
                held.insert(*index, model.entities.len());
                model.entities.push(IfcEntity::ElementPropertySet {
                    element: *index,
                    set: PropertySet {
                        name: "Qto_BeamBaseQuantities".into(),
                        properties: values
                            .into_iter()
                            .map(|(name, value)| Property {
                                name: name.into(),
                                value,
                            })
                            .collect(),
                    },
                });
            }
        }
    }
}

/// Give each column of `found` (an index into the entities and what its mesh
/// gives of its base quantities) the quantities Revit's export writes for a
/// column it draws as an extrusion: `CrossSectionArea`,
/// `OuterSurfaceArea` and `GrossVolume`, and `Length`, in the base quantity set it
/// holds. A column whose set holds none of the three gets all four, and one
/// that holds all three gets the `Length` it lacks; one that holds some of the
/// three is left as it is, because the cut-column rule writes a
/// `CrossSectionArea` and no more for a column Revit draws as a cut mesh, which has
/// no `OuterSurfaceArea` or `GrossVolume`. A column drawn from
/// its mesh holds the set: [`add_mesh_volumes`] gave every column whose mesh is
/// a closed solid of flat faces its `NetVolume`. A column that keeps its body,
/// the rectangle its type gives, may hold none, and then gets the set
/// with all of them, its `NetVolume` the volume of the prism as Revit's is of an
/// extrusion. A value a column holds is never replaced.
fn add_column_quantities(model: &mut IfcModel, found: &mut [(usize, ColumnQuantities)]) {
    if found.is_empty() {
        return;
    }
    found.sort_by_key(|(index, _)| *index);
    let mut held: BTreeMap<usize, usize> = BTreeMap::new();
    for (position, entity) in model.entities.iter().enumerate() {
        if let IfcEntity::ElementPropertySet { element, set } = entity {
            if set.name == "Qto_ColumnBaseQuantities" {
                held.entry(*element).or_insert(position);
            }
        }
    }
    for (index, quantities) in found.iter() {
        let section = [
            (
                "CrossSectionArea",
                PropertyValue::AreaSquareFeet(quantities.cross_section_area),
            ),
            (
                "OuterSurfaceArea",
                PropertyValue::AreaSquareFeet(quantities.outer_surface_area),
            ),
            (
                "GrossVolume",
                PropertyValue::VolumeCubicFeet(quantities.volume),
            ),
        ];
        let rest = [
            (
                "Length",
                PropertyValue::PositiveLengthFeet(quantities.length),
            ),
            (
                "NetVolume",
                PropertyValue::VolumeCubicFeet(quantities.volume),
            ),
        ];
        let Some(&position) = held.get(index) else {
            held.insert(*index, model.entities.len());
            model.entities.push(IfcEntity::ElementPropertySet {
                element: *index,
                set: PropertySet {
                    name: "Qto_ColumnBaseQuantities".into(),
                    properties: section
                        .into_iter()
                        .chain(rest)
                        .map(|(name, value)| Property {
                            name: name.into(),
                            value,
                        })
                        .collect(),
                },
            });
            continue;
        };
        let Some(IfcEntity::ElementPropertySet { set, .. }) = model.entities.get_mut(position)
        else {
            continue;
        };
        let held_of_three = section
            .iter()
            .filter(|(name, _)| has_property(set, name))
            .count();
        let wanted = match held_of_three {
            0 => section.into_iter().chain(rest).collect::<Vec<_>>(),
            3 => rest.into_iter().collect::<Vec<_>>(),
            _ => continue,
        };
        for (name, value) in wanted {
            if !has_property(set, name) {
                set.properties.push(Property {
                    name: name.into(),
                    value,
                });
            }
        }
    }
}

/// Give the `Span` of each mullion of `moved` (an index into the entities and its
/// new `Length`, feet) the same length, as it has at the record.
fn follow_mesh_span(model: &mut IfcModel, moved: &[(usize, f64)]) {
    if moved.is_empty() {
        return;
    }
    let lengths: BTreeMap<usize, f64> = moved.iter().copied().collect();
    for entity in &mut model.entities {
        let IfcEntity::ElementPropertySet { element, set } = entity else {
            continue;
        };
        let Some(&length) = lengths.get(&*element) else {
            continue;
        };
        if set.name != "Pset_MemberCommon" {
            continue;
        }
        for property in &mut set.properties {
            if property.name == "Span" {
                property.value = PropertyValue::PositiveLengthFeet(length);
            }
        }
    }
}

/// How far a wall's held `NetVolume` may differ from the volume of its mesh,
/// relative to the mesh's, and still be the solid's: the mesh's corners
/// are single precision, which moves the volume of a small wall at 190 ft by
/// about 1e-4 of it.
const WALL_BOX_MESH_TOLERANCE: f64 = 1e-3;

/// How far the vertical span of a wall's mesh may differ from the height of its
/// body, feet, and still confirm it: the span is within 1e-4 ft of
/// Revit's `Height` on every wall of the BIM4LCA office, and the
/// smallest difference between a body's height and Revit's measured on the
/// BIM4LCA housing is 0.11 ft.
const WALL_HEIGHT_AGREE_FEET: f64 = 5e-4;

/// How far, radians, an element's turn may lie off its wall's own axes for its
/// world box, which is axis aligned, to be that element's extent in the wall's
/// frame.
const OPENING_TURN_TOLERANCE: f64 = 1e-6;

/// A wall's own axes and the world boxes, feet, of the openings rvt-open cuts in
/// it, `[min_x, min_y, min_z, max_x, max_y, max_z]`.
struct WallOpenings {
    frame: WallFrame,
    boxes: Vec<[f64; 6]>,
}

/// each wall's frame, by its index in `model.entities`, and the boxes of
/// the openings written in it: the rectangle of every element the wall hosts,
/// with the axis across the wall cut to the wall's thickness, placed on that
/// element's own placement and extruded up its height, as the writer emits it.
/// A wall whose element is not read, and an opening whose turn is
/// not one of its wall's, is left out, so the wall gets no areas of the mesh for
/// them.
fn wall_frame_and_openings(model: &IfcModel) -> BTreeMap<usize, WallOpenings> {
    let mut out: BTreeMap<usize, WallOpenings> = BTreeMap::new();
    for (index, entity) in model.entities.iter().enumerate() {
        let IfcEntity::BuildingElement {
            ifc_type,
            rotation_radians,
            ..
        } = entity
        else {
            continue;
        };
        if matches!(ifc_type.as_str(), "IFCWALL" | "IFCWALLSTANDARDCASE") {
            out.insert(
                index,
                WallOpenings {
                    frame: WallFrame {
                        turn: rotation_radians.unwrap_or(0.0),
                    },
                    boxes: Vec::new(),
                },
            );
        }
    }
    for (index, entity) in model.entities.iter().enumerate() {
        let IfcEntity::BuildingElement {
            host_element_index: Some(host),
            location_feet: Some(location),
            rotation_radians,
            extrusion: Some(body),
            solid_shape: None,
            ..
        } = entity
        else {
            continue;
        };
        // Only a body of a plain rectangle is the opening the writer emits
        // from it; a profile or a solid of another shape explains nothing.
        if body.profile_override.is_some() {
            continue;
        }
        let Some(hosted) = out.get_mut(host) else {
            continue;
        };
        let turn = rotation_radians.unwrap_or(0.0) - hosted.frame.turn;
        if (2.0 * turn).sin().abs() > OPENING_TURN_TOLERANCE {
            continue;
        }
        let cut = model.opening_cuts.get(&index);
        let [x_dim, y_dim] = cut.map_or([body.width_feet, body.depth_feet], |cut| {
            [cut.x_dim_feet, cut.y_dim_feet]
        });
        let [cx, cy] = cut.map_or([0.0, 0.0], |cut| cut.centre_feet);
        // `z_range_feet` is where the opening starts and how high it is,
        // as the writer extrudes it: its top is `start + height`, so the start
        // is added once, below.
        let [start, height] = cut
            .and_then(|cut| cut.z_range_feet)
            .unwrap_or([0.0, body.height_feet]);
        let (sin, cos) = rotation_radians.unwrap_or(0.0).sin_cos();
        let mut x = [f64::INFINITY, f64::NEG_INFINITY];
        let mut y = [f64::INFINITY, f64::NEG_INFINITY];
        for side_x in [-1.0, 1.0] {
            for side_y in [-1.0, 1.0] {
                let (px, py) = (cx + side_x * x_dim / 2.0, cy + side_y * y_dim / 2.0);
                let corner = [
                    location[0] + px * cos - py * sin,
                    location[1] + px * sin + py * cos,
                ];
                x = [x[0].min(corner[0]), x[1].max(corner[0])];
                y = [y[0].min(corner[1]), y[1].max(corner[1])];
            }
        }
        let world = [
            x[0],
            y[0],
            location[2] + start,
            x[1],
            y[1],
            location[2] + start + height,
        ];
        if world.iter().all(|value| value.is_finite()) {
            hosted.boxes.push(world);
        }
    }
    out
}

/// Give each wall of `found` (an index into the entities and what the mesh
/// Revit saved for it gives) the quantities Revit's export writes from its
/// solid, in the base quantity set it holds, where that holds none of them:
/// `NetVolume`, and, of a wall with no more than four vertical faces
/// or of one whose mesh is a vertical prism less the openings cut in it and
/// whose base is not attached, `NetSideArea` and, where its two sides
/// agree on it, `GrossSideArea` and, the wall type's `Width` times it,
/// `GrossVolume`
/// (Revit's IFC exporter's `PropertyUtil.CreateWallBaseQuantities`, CLEANROOM.md source 5).
/// A wall with no base quantity set, as a wall Revit's export writes
/// none for may be, gets none.
///
/// The quantities of a wall's box hold only where the box is
/// the solid. A wall whose held `NetVolume` is not the mesh's, by more
/// than [`WALL_BOX_MESH_TOLERANCE`] of it, is a box that is not Revit's solid (a
/// wall joined at an angle, whose footprint is a pentagon, or a box longer than
/// the wall): the quantities taken from the box are dropped, and the mesh gives
/// those it can. The footprint stays where the mesh's vertical span is not the
/// box's height, which says the wall is cut in height and not in plan. A
/// `Height` is the box's, and is written for a wall that gets its areas, where
/// the mesh's vertical span confirms it and is dropped where the span does not
/// (a wall whose lower part a door as wide as the wall cuts away).
fn add_wall_quantities(model: &mut IfcModel, found: &[(usize, WallMeshQuantities)]) {
    if found.is_empty() {
        return;
    }
    let mut held: BTreeMap<usize, usize> = BTreeMap::new();
    for (position, entity) in model.entities.iter().enumerate() {
        if let IfcEntity::ElementPropertySet { element, set } = entity {
            if set.name == "Qto_WallBaseQuantities" {
                held.entry(*element).or_insert(position);
            }
        }
    }
    for (index, quantities) in found {
        let Some(&position) = held.get(index) else {
            continue;
        };
        // The height of the wall's body, where it is the extrusion of its box,
        // or where its top is cut along a sloped roof: that solid keeps
        // the box's extrusion, whose height is the record's, and Revit's export
        // writes that height as the wall's `Height` (25 of 25 on the R2025
        // models, which matched it before the cut).
        let body_height = match model.entities.get(*index) {
            Some(IfcEntity::BuildingElement {
                extrusion: Some(body),
                solid_shape: None,
                ..
            }) => Some(body.height_feet),
            Some(IfcEntity::BuildingElement {
                extrusion: Some(body),
                solid_shape: Some(_),
                property_set: Some(own),
                ..
            }) if has_text_property(
                own,
                "BodyHeightSource",
                WALL_TOP_SLOPED_BODY_HEIGHT_SOURCE,
            ) =>
            {
                Some(body.height_feet)
            }
            _ => None,
        };
        let Some(IfcEntity::ElementPropertySet { set, .. }) = model.entities.get_mut(position)
        else {
            continue;
        };
        let height_agrees = body_height
            .map(|height| (quantities.height_feet - height).abs() <= WALL_HEIGHT_AGREE_FEET);
        let held_volume = set
            .properties
            .iter()
            .find_map(|property| match &property.value {
                PropertyValue::VolumeCubicFeet(volume) if property.name == "NetVolume" => {
                    Some(*volume)
                }
                _ => None,
            });
        if held_volume.is_some_and(|held| {
            (held - quantities.net_volume).abs() > WALL_BOX_MESH_TOLERANCE * quantities.net_volume
        }) {
            // The footprint is the plan's: a box whose height the mesh does not
            // have (a wall cut away from below or above) may still have Revit's.
            let keep_footprint = height_agrees == Some(false);
            set.properties
                .retain(|property| match property.name.as_str() {
                    "NetVolume" | "NetSideArea" | "GrossSideArea" | "GrossVolume" => false,
                    "GrossFootprintArea" => keep_footprint,
                    _ => true,
                });
        }
        if let Some(height) = body_height {
            if height_agrees == Some(false) {
                set.properties.retain(|property| property.name != "Height");
            } else if quantities.sides.is_some() && !has_property(set, "Height") {
                set.properties.push(Property {
                    name: "Height".into(),
                    value: PropertyValue::PositiveLengthFeet(height),
                });
            }
        }
        let width = set
            .properties
            .iter()
            .find_map(|property| match &property.value {
                PropertyValue::PositiveLengthFeet(width) if property.name == "Width" => {
                    Some(*width)
                }
                _ => None,
            });
        let mut add = |name: &str, value: PropertyValue| {
            if !has_property(set, name) {
                set.properties.push(Property {
                    name: name.into(),
                    value,
                });
            }
        };
        add(
            "NetVolume",
            PropertyValue::VolumeCubicFeet(quantities.net_volume),
        );
        let Some(sides) = quantities.sides else {
            continue;
        };
        add("NetSideArea", PropertyValue::AreaSquareFeet(sides.net));
        if let Some(gross) = sides.gross {
            add("GrossSideArea", PropertyValue::AreaSquareFeet(gross));
            if let Some(width) = width {
                add("GrossVolume", PropertyValue::VolumeCubicFeet(width * gross));
            }
        }
    }
}

/// Give each curtain panel of `widths` (an index into the entities and the
/// longer horizontal side, feet, of the box its body was drawn from a mesh as)
/// the `Width` of its `Qto_PlateBaseQuantities`, where that holds none.
/// Revit's export writes it on the panels it draws as a box and on no
/// other, and `add_mesh_volumes` has made the set by now for each of these.
fn add_plate_widths(model: &mut IfcModel, widths: &[(usize, f64)]) {
    if widths.is_empty() {
        return;
    }
    let mut held: BTreeMap<usize, usize> = BTreeMap::new();
    for (position, entity) in model.entities.iter().enumerate() {
        if let IfcEntity::ElementPropertySet { element, set } = entity {
            if set.name == "Qto_PlateBaseQuantities" {
                held.entry(*element).or_insert(position);
            }
        }
    }
    for &(index, width) in widths {
        let Some(&position) = held.get(&index) else {
            continue;
        };
        if let Some(IfcEntity::ElementPropertySet { set, .. }) = model.entities.get_mut(position) {
            if !has_property(set, "Width") {
                set.properties.push(Property {
                    name: "Width".into(),
                    value: PropertyValue::LengthFeet(width),
                });
            }
        }
    }
}

/// An element's saved graphics as a body placed at `origin`, turned by
/// `rotation` about Z: tidied, back faces dropped, each face in its Revit
/// material (materials are added to `materials`). Returns the
/// shape and how many back-face triangles were dropped; `None` when the
/// graphics hold no triangle.
pub(crate) fn element_shape(
    materials: &mut Vec<MaterialInfo>,
    element: &crate::native_saved_scene::SavedElement,
    origin: [f64; 3],
    rotation: f64,
) -> Option<(SolidShape, usize)> {
    let primitives = &element.meshes.primitives;
    // each face group's material, as the model's material.
    let group_materials: Vec<Option<usize>> = (0..primitives.len())
        .map(|i| {
            element
                .render_materials
                .get(&i)
                .map(|m| material_index_for(materials, m))
        })
        .collect();
    let (sin, cos) = (-rotation).sin_cos();
    let mut vertices = Vec::new();
    let mut triangles = Vec::new();
    let mut triangle_groups = Vec::new();
    for (group, primitive) in primitives.iter().enumerate() {
        let base = vertices.len() as u32;
        vertices.extend(primitive.vertices.iter().map(|p| {
            let (dx, dy) = (p[0] - origin[0], p[1] - origin[1]);
            [dx * cos - dy * sin, dx * sin + dy * cos, p[2] - origin[2]]
        }));
        triangles.extend(
            primitive
                .triangles
                .iter()
                .filter(|t| t.iter().all(|&v| (v as usize) < primitive.vertices.len()))
                .map(|t| BrepTriangle(base + t[0], base + t[1], base + t[2])),
        );
        triangle_groups.resize(triangles.len(), group_materials[group]);
    }
    if triangles.is_empty() {
        return None;
    }
    let face_of = face_material_lookup(&vertices, &triangles, &triangle_groups);
    let (vertices, triangles) = tidy_mesh(vertices, triangles);
    let before = triangles.len();
    let triangles = drop_back_faces(&vertices, triangles);
    let dropped = before - triangles.len();
    let face_materials = face_materials_of(&face_of, &vertices, &triangles);
    Some((
        SolidShape::TriangulatedFaceSet {
            vertices_feet: vertices,
            triangles,
            face_materials,
        },
        dropped,
    ))
}

/// host family-instance poses on releases before 2023 (empty
/// otherwise, or when the family documents do not read).
fn pre_2023_instance_poses(
    rf: &mut crate::RevitFile,
) -> BTreeMap<i64, crate::family_documents::InstancePose> {
    let release = rf.basic_file_info().map(|b| b.version).unwrap_or(0);
    if release >= 2023 {
        return BTreeMap::new();
    }
    crate::family_documents::read_memoised(rf)
        .map(|d| d.instances.clone())
        .unwrap_or_default()
}

/// Instances of one family type draw one mesh: two saved meshes with the same
/// triangles whose vertices all lie within 2 mm, or 0.3% of the shape's size up
/// to 25 mm, of each other (the angle noise of two placements) take the first one's vertices, so that the shape
/// definitions are equal and every writer shares them.
/// What [`attach_display_bodies`] did.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize)]
#[non_exhaustive]
pub struct DisplayBodyReport {
    /// Elements asked for: hosts of a door, window or opening, and walls,
    /// floors and roofs, not already drawn from their saved graphics.
    pub hosts: usize,
    /// Of those, elements now drawn from their saved graphics, openings cut.
    pub drawn: usize,
    /// Hosts keeping their own body: no saved graphics, or not all decoded.
    pub partial: usize,
    pub missing: usize,
    /// Triangles written in all.
    pub triangles: usize,
}

/// The elements a door, window or opening cuts, by index into
/// `model.entities`: the hosts of filling elements and of unfilled openings.
pub fn opening_hosts(model: &IfcModel) -> BTreeSet<usize> {
    model
        .entities
        .iter()
        .filter_map(|entity| match entity {
            IfcEntity::BuildingElement {
                host_element_index: Some(host),
                ..
            }
            | IfcEntity::VoidOpening { host, .. }
            | IfcEntity::WallProfileOpening { host, .. }
            | IfcEntity::HostedCutOpening { host, .. } => Some(*host),
            _ => None,
        })
        .collect()
}

/// give each element a door, window or opening cuts, and each
/// wall, floor and roof, a display body ([`IfcModel::display_bodies`]) from its
/// saved graphics, which Revit draws with the openings cut, where they
/// decode whole. The mesh formats
/// draw it; the IFC4 writer keeps the element's body and its voids.
pub fn attach_display_bodies(
    rf: &mut crate::RevitFile,
    model: &mut IfcModel,
    control: &crate::control::WalkerControl,
) -> anyhow::Result<DisplayBodyReport> {
    let mut wanted = opening_hosts(model);
    // Every wall, floor and roof too: openings with no filling element (a
    // shaft, a wall opening, an edited profile, a joined wall's cut) are cut
    // in its saved graphics (Revit's export of Technical school voids 8
    // walls and 2 floors with 22 such openings, rac_basic 10 walls with 19).
    wanted.extend(
        model
            .entities
            .iter()
            .enumerate()
            .filter_map(|(i, e)| match e {
                IfcEntity::BuildingElement { ifc_type, .. }
                    if matches!(
                        ifc_type.as_str(),
                        "IFCWALL" | "IFCWALLSTANDARDCASE" | "IFCSLAB" | "IFCROOF"
                    ) =>
                {
                    Some(i)
                }
                _ => None,
            }),
    );
    attach_display_bodies_for(rf, model, &wanted, control, saved_scene_batch())
}

/// [`attach_display_bodies`] for the elements `wanted` names, `batch_size`
/// elements' saved graphics at a time.
pub fn attach_display_bodies_for(
    rf: &mut crate::RevitFile,
    model: &mut IfcModel,
    wanted: &BTreeSet<usize>,
    control: &crate::control::WalkerControl,
    batch_size: usize,
) -> anyhow::Result<DisplayBodyReport> {
    let mut report = DisplayBodyReport::default();
    let mut by_id: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
    for &index in wanted {
        let Some(IfcEntity::BuildingElement {
            type_guid: Some(tag),
            location_feet: Some(_),
            property_set,
            ..
        }) = model.entities.get(index)
        else {
            continue;
        };
        // Already drawn from its saved graphics.
        let saved = property_set.as_ref().is_some_and(|set| {
            set.properties.iter().any(|p| {
                p.name == "BodySource"
                    && matches!(&p.value, PropertyValue::Text(t) if t == SAVED_MESH_BODY_SOURCE)
            })
        });
        if saved {
            continue;
        }
        let Ok(id) = tag.parse::<u64>() else {
            continue;
        };
        report.hosts += 1;
        by_id.entry(id).or_default().push(index);
    }
    let ids: Vec<u64> = by_id.keys().copied().collect();
    let mut seen = BTreeSet::new();
    // The material pass reads every element whatever the batch selects:
    // run it once, not once per batch.
    let materials = if ids.is_empty() {
        Default::default()
    } else {
        material_resolver(rf)?
    };
    for batch in ids.chunks(batch_size.max(1)) {
        let options = crate::native_document::Options {
            selected_ids: batch.iter().copied().collect::<BTreeSet<_>>(),
            max_graph_values: 1_000_000,
            max_graph_objects: 1_000_000,
            ..Default::default()
        };
        let scene = crate::native_saved_scene::extract_framed_with_materials(
            rf,
            &options,
            3,
            control,
            false,
            Some(&*materials),
        )?;
        crate::control::stage("saved graphics: openings");
        for element in scene.elements {
            // A Tag carried by several entities (a slab's pieces) is ambiguous.
            let Some([index]) = by_id.get(&element.identity.element_id).map(Vec::as_slice) else {
                continue;
            };
            if element.meshes.primitives.is_empty() {
                continue;
            }
            seen.insert(*index);
            if is_partial(&element) {
                report.partial += 1;
                continue;
            }
            let Some(IfcEntity::BuildingElement {
                location_feet: Some(origin),
                rotation_radians,
                ..
            }) = model.entities.get(*index)
            else {
                continue;
            };
            let (origin, rotation) = (*origin, *rotation_radians);
            let Some((shape, triangles, _)) =
                local_body(&element, &[], origin, rotation, &mut model.materials)
            else {
                continue;
            };
            report.drawn += 1;
            report.triangles += triangles;
            model.display_bodies.insert(*index, shape);
        }
    }
    report.missing = report.hosts - seen.len().min(report.hosts);
    Ok(report)
}

/// Whether part of an element's saved graphics did not decode (a
/// face or branch), so its mesh is only part of its body. Branches left out
/// for their view semantics leave it whole.
pub(crate) fn is_partial(element: &crate::native_saved_scene::SavedElement) -> bool {
    element.meshes.diagnostics.iter().any(|diagnostic| {
        !diagnostic.ends_with(crate::native_graphics_traversal::NESTED_DETAIL_FILTER_DIAGNOSTIC)
            && !diagnostic.ends_with(crate::native_graphics_traversal::VIEW_COMPONENT_DIAGNOSTIC)
    })
}

/// An element's saved mesh in its own frame (`origin`, turned `rotation`
/// about Z), tidied, one side of each face, with each face's material:
/// the shape, its triangle count and the back faces dropped.
/// `None` for a mesh with no triangle.
fn local_body(
    element: &crate::native_saved_scene::SavedElement,
    rails: &[&crate::native_saved_scene::SavedElement],
    origin: [f64; 3],
    rotation: Option<f64>,
    materials: &mut Vec<MaterialInfo>,
) -> Option<(SolidShape, usize, usize)> {
    // The element's face groups, then each continuous rail's,
    // each with its own group's material.
    let parts: Vec<&crate::native_saved_scene::SavedElement> = std::iter::once(element)
        .chain(rails.iter().copied())
        .collect();
    let primitives: Vec<&Primitive> = parts
        .iter()
        .flat_map(|e| e.meshes.primitives.iter())
        .collect();
    let group_materials: Vec<Option<usize>> = parts
        .iter()
        .flat_map(|e| (0..e.meshes.primitives.len()).map(move |i| e.render_materials.get(&i)))
        .map(|m| m.map(|m| material_index_for(materials, m)))
        .collect();
    let (sin, cos) = (-rotation.unwrap_or(0.0)).sin_cos();
    let mut vertices = Vec::new();
    let mut triangles = Vec::new();
    let mut triangle_groups = Vec::new();
    for (group, primitive) in primitives.iter().enumerate() {
        let base = vertices.len() as u32;
        vertices.extend(primitive.vertices.iter().map(|p| {
            let (dx, dy) = (p[0] - origin[0], p[1] - origin[1]);
            [dx * cos - dy * sin, dx * sin + dy * cos, p[2] - origin[2]]
        }));
        triangles.extend(
            primitive
                .triangles
                .iter()
                .filter(|t| t.iter().all(|&v| (v as usize) < primitive.vertices.len()))
                .map(|t| BrepTriangle(base + t[0], base + t[1], base + t[2])),
        );
        triangle_groups.resize(triangles.len(), group_materials[group]);
    }
    if triangles.is_empty() {
        return None;
    }
    let face_of = face_material_lookup(&vertices, &triangles, &triangle_groups);
    let (vertices, triangles) = tidy_mesh(vertices, triangles);
    let before = triangles.len();
    let triangles = drop_back_faces(&vertices, triangles);
    let dropped = before - triangles.len();
    let count = triangles.len();
    let face_materials = face_materials_of(&face_of, &vertices, &triangles);
    Some((
        SolidShape::TriangulatedFaceSet {
            vertices_feet: vertices,
            triangles,
            face_materials,
        },
        count,
        dropped,
    ))
}

fn unify_instances(model: &mut IfcModel) -> (usize, usize, usize) {
    use std::hash::{Hash, Hasher};
    const FLOOR: f64 = 0.002 / 0.3048;
    const CEILING: f64 = 0.025 / 0.3048;
    let mut reps: BTreeMap<(String, usize, u64), Vec<usize>> = BTreeMap::new();
    let mut unified = 0;
    let (mut misses, mut closest) = (0_usize, f64::INFINITY);
    for index in 0..model.entities.len() {
        // an instance of a symbol mesh shares it exactly.
        if model.instancing.instances.contains_key(&index) {
            continue;
        }
        let IfcEntity::BuildingElement {
            ifc_type,
            solid_shape:
                Some(SolidShape::TriangulatedFaceSet {
                    vertices_feet,
                    triangles,
                    ..
                }),
            property_set: Some(set),
            ..
        } = &model.entities[index]
        else {
            continue;
        };
        let saved = set.properties.iter().any(|p| {
            p.name == "BodySource"
                && matches!(&p.value, PropertyValue::Text(t) if t == SAVED_MESH_BODY_SOURCE)
        });
        if !saved {
            continue;
        }
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        triangles
            .iter()
            .for_each(|t| (t.0, t.1, t.2).hash(&mut hasher));
        let key = (ifc_type.clone(), vertices_feet.len(), hasher.finish());
        let found = reps.get(&key).and_then(|candidates| {
            candidates.iter().copied().find(|&r| {
                let IfcEntity::BuildingElement {
                    solid_shape:
                        Some(SolidShape::TriangulatedFaceSet {
                            vertices_feet: rv, ..
                        }),
                    ..
                } = &model.entities[r]
                else {
                    return false;
                };
                let extent = (0..3)
                    .map(|k| {
                        let (lo, hi) = rv.iter().fold((f64::MAX, f64::MIN), |(lo, hi), v| {
                            (lo.min(v[k]), hi.max(v[k]))
                        });
                        hi - lo
                    })
                    .fold(0.0_f64, f64::max);
                let tolerance = (extent * 0.003).clamp(FLOOR, CEILING);
                rv.iter()
                    .zip(vertices_feet)
                    .all(|(a, b)| (0..3).all(|k| (a[k] - b[k]).abs() <= tolerance))
            })
        });
        match found {
            Some(rep) => {
                let IfcEntity::BuildingElement {
                    solid_shape:
                        Some(SolidShape::TriangulatedFaceSet {
                            vertices_feet: rv, ..
                        }),
                    ..
                } = &model.entities[rep]
                else {
                    continue;
                };
                let rv = rv.clone();
                if let IfcEntity::BuildingElement {
                    solid_shape: Some(SolidShape::TriangulatedFaceSet { vertices_feet, .. }),
                    ..
                } = &mut model.entities[index]
                {
                    if *vertices_feet != rv {
                        *vertices_feet = rv;
                        unified += 1;
                    }
                }
            }
            None => {
                if let Some(first) = reps.get(&key).and_then(|c| c.first()) {
                    misses += 1;
                    if let IfcEntity::BuildingElement {
                        solid_shape:
                            Some(SolidShape::TriangulatedFaceSet {
                                vertices_feet: rv, ..
                            }),
                        ..
                    } = &model.entities[*first]
                    {
                        let gap = rv
                            .iter()
                            .zip(vertices_feet)
                            .flat_map(|(a, b)| (0..3).map(move |k| (a[k] - b[k]).abs()))
                            .fold(0.0_f64, f64::max);
                        closest = closest.min(gap);
                    }
                }
                reps.entry(key).or_default().push(index)
            }
        }
    }
    let closest_mm = if closest.is_finite() {
        (closest * 304.8).round() as usize
    } else {
        0
    };
    (unified, misses, closest_mm)
}

/// A triangle's corners on [`tidy_mesh`]'s 1e-5 ft grid, in a fixed order, so
/// a triangle is found again after tidying, welding and back-face dropping.
type FaceKey = [[i64; 3]; 3];
fn face_key(vertices: &[[f64; 3]], t: &BrepTriangle) -> FaceKey {
    let q = |v: u32| vertices[v as usize].map(|c| (c * 100_000.0).round() as i64);
    let mut k = [q(t.0), q(t.1), q(t.2)];
    k.sort_unstable();
    k
}

/// each triangle's face-group material, by its corners.
pub(crate) fn face_material_lookup(
    vertices: &[[f64; 3]],
    triangles: &[BrepTriangle],
    groups: &[Option<usize>],
) -> BTreeMap<FaceKey, usize> {
    let mut out = BTreeMap::new();
    for (t, g) in triangles.iter().zip(groups) {
        if let Some(m) = g {
            out.entry(face_key(vertices, t)).or_insert(*m);
        }
    }
    out
}

/// One material per triangle (`usize::MAX` where the face group had none),
/// or none when no face group had one.
pub(crate) fn face_materials_of(
    lookup: &BTreeMap<FaceKey, usize>,
    vertices: &[[f64; 3]],
    triangles: &[BrepTriangle],
) -> Vec<usize> {
    if lookup.is_empty() {
        return Vec::new();
    }
    triangles
        .iter()
        .map(|t| {
            lookup
                .get(&face_key(vertices, t))
                .copied()
                .unwrap_or(super::body_geometry::NO_FACE_MATERIAL)
        })
        .collect()
}

/// Revit packs a colour as `0x00BBGGRR`.
fn packed_colour(found: &crate::native_saved_materials::RenderMaterial) -> u32 {
    // the material's graphics (Shaded) colour, as Revit's Shaded
    // view and its IFC export draw it, not the render appearance's.
    let channel = |c: f64| (c.clamp(0.0, 1.0) * 255.0).round() as u32;
    channel(found.shading_color[0])
        | (channel(found.shading_color[1]) << 8)
        | (channel(found.shading_color[2]) << 16)
}

/// The material's graphics transparency (`m_transparency`), when not opaque.
fn shading_transparency(found: &crate::native_saved_materials::RenderMaterial) -> Option<f64> {
    let opacity = found.shading_color[3];
    (opacity < 1.0).then(|| (1.0 - opacity).clamp(0.0, 1.0))
}

/// The index in `materials` of the material `found` names, added when no
/// entry has its name and colour.
pub(crate) fn material_index_for(
    materials: &mut Vec<MaterialInfo>,
    found: &crate::native_saved_materials::RenderMaterial,
) -> usize {
    let packed = packed_colour(found);
    if let Some(i) = materials
        .iter()
        .position(|m| m.name == found.name && m.color_packed == Some(packed))
    {
        return i;
    }
    materials.push(MaterialInfo {
        name: found.name.clone(),
        color_packed: Some(packed),
        transparency: shading_transparency(found),
    });
    materials.len() - 1
}

/// Give every building element that has no material (no single material,
/// layer set or profile set) the material of the face group with the most
/// triangles in its saved graphics, and return how many took one.
/// Bodies are not touched.
pub fn attach_materials(rf: &mut crate::RevitFile, model: &mut IfcModel) -> anyhow::Result<usize> {
    attach_materials_in_batches(rf, model, saved_scene_batch())
}

/// [`attach_materials`] in batches of at most `batch_size` elements, as
/// [`attach_scoped_in_batches`]; the same result for any batch size.
pub fn attach_materials_in_batches(
    rf: &mut crate::RevitFile,
    model: &mut IfcModel,
    batch_size: usize,
) -> anyhow::Result<usize> {
    let mut needs: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
    // Walls, floors and roofs hold a layer set, not a material, and their
    // layer materials carry no colour: they take the colour of their saved
    // graphics onto the dominant layer's material.
    let mut uncoloured_sets: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
    let has_colour = |m: Option<usize>| {
        m.and_then(|m| model.materials.get(m))
            .is_some_and(|m| m.color_packed.is_some())
    };
    for (index, entity) in model.entities.iter().enumerate() {
        let IfcEntity::BuildingElement {
            type_guid: Some(tag),
            material_index,
            material_layer_set_index,
            material_profile_set_index,
            ..
        } = entity
        else {
            continue;
        };
        let Ok(id) = tag.parse::<u64>() else {
            continue;
        };
        if material_index.is_none()
            && material_layer_set_index.is_none()
            && material_profile_set_index.is_none()
        {
            needs.entry(id).or_default().push(index);
        } else if material_index.is_none() {
            let coloured = material_layer_set_index
                .and_then(|i| model.material_layer_sets.get(i))
                .is_some_and(|set| set.layers.iter().any(|l| has_colour(l.material_index)))
                || material_profile_set_index
                    .and_then(|i| model.material_profile_sets.get(i))
                    .is_some_and(|set| {
                        set.profiles
                            .iter()
                            .any(|p| has_colour(Some(p.material_index)))
                    });
            if !coloured {
                uncoloured_sets.entry(id).or_default().push(index);
            }
        }
    }
    let wanted: BTreeSet<u64> = needs
        .keys()
        .chain(uncoloured_sets.keys())
        .copied()
        .collect();
    if wanted.is_empty() {
        return Ok(0);
    }
    // in batches, as `attach_scoped_batched`; the batches go in id
    // order, so materials are added in the same order as from one scene.
    let ids: Vec<u64> = wanted.into_iter().collect();
    let mut set = 0;
    // The material pass reads every element whatever the batch selects:
    // run it once, not once per batch.
    let materials = material_resolver(rf)?;
    for batch in ids.chunks(batch_size.max(1)) {
        let options = crate::native_document::Options {
            selected_ids: batch.iter().copied().collect(),
            max_graph_values: 1_000_000,
            max_graph_objects: 1_000_000,
            ..Default::default()
        };
        let scene = crate::native_saved_scene::extract_framed_with_materials(
            rf,
            &options,
            3,
            &crate::control::WalkerControl::default(),
            false,
            Some(&*materials),
        )?;
        crate::control::stage("saved materials: apply");
        // By value, as in `attach_scoped_with_control`: freed as it is read.
        for element in scene.elements {
            let id = element.identity.element_id;
            let empty = Vec::new();
            let indices = needs.get(&id).unwrap_or(&empty);
            let sets = uncoloured_sets.get(&id).unwrap_or(&empty);
            if indices.is_empty() && sets.is_empty() {
                continue;
            }
            let dominant = element
                .meshes
                .primitives
                .iter()
                .enumerate()
                .filter_map(|(i, p)| {
                    element
                        .render_materials
                        .get(&i)
                        .map(|m| (p.triangles.len(), m))
                })
                .max_by_key(|(n, _)| *n);
            let Some((_, found)) = dominant else {
                continue;
            };
            if !indices.is_empty() {
                let chosen = material_index_for(&mut model.materials, found);
                for &index in indices {
                    if let IfcEntity::BuildingElement { material_index, .. } =
                        &mut model.entities[index]
                    {
                        *material_index = Some(chosen);
                        set += 1;
                    }
                }
            }
            for &index in sets {
                // The thickest layer's material (or the first profile's) takes
                // the colour; every output reads it from there.
                let target = match &model.entities[index] {
                    IfcEntity::BuildingElement {
                        material_layer_set_index,
                        material_profile_set_index,
                        ..
                    } => material_layer_set_index
                        .and_then(|i| model.material_layer_sets.get(i))
                        .and_then(|set| {
                            set.layers
                                .iter()
                                .filter(|l| l.material_index.is_some())
                                .max_by(|a, b| a.thickness_feet.total_cmp(&b.thickness_feet))
                                .and_then(|l| l.material_index)
                        })
                        .or_else(|| {
                            material_profile_set_index
                                .and_then(|i| model.material_profile_sets.get(i))
                                .and_then(|set| set.profiles.first().map(|p| p.material_index))
                        }),
                    _ => None,
                };
                if let Some(m) = target.and_then(|t| model.materials.get_mut(t))
                    && m.color_packed.is_none()
                {
                    m.color_packed = Some(packed_colour(found));
                    m.transparency = shading_transparency(found);
                    set += 1;
                }
            }
        }
    }
    Ok(set)
}

/// Revit's saved graphics hold each face of a solid twice, once per side
/// (on rac_basic 2024 every zero-volume item had a reversed twin
/// for every triangle). Keep one triangle per twin pair, the one facing out
/// of the body: a ray from its centroid along its normal leaves the mesh when
/// it crosses the other faces an even number of times.
pub(crate) fn drop_back_faces(
    vertices: &[[f64; 3]],
    triangles: Vec<BrepTriangle>,
) -> Vec<BrepTriangle> {
    type Key = [[i64; 3]; 3];
    let weld = |v: u32| -> [i64; 3] {
        let p = vertices[v as usize];
        std::array::from_fn(|k| (p[k] * 10_000.0).round() as i64)
    };
    // Canonical rotation of the corner list, and whether it is the original
    // winding.
    let canon = |t: &BrepTriangle| -> (Key, bool) {
        let c = [weld(t.0), weld(t.1), weld(t.2)];
        let r = [c[0], c[2], c[1]];
        let rot = |c: [[i64; 3]; 3]| (0..3).map(|i| [c[i], c[(i + 1) % 3], c[(i + 2) % 3]]).min();
        let (a, b) = (rot(c).unwrap(), rot(r).unwrap());
        if a <= b { (a, true) } else { (b, false) }
    };
    let mut groups: BTreeMap<Key, [Option<usize>; 2]> = BTreeMap::new();
    for (i, t) in triangles.iter().enumerate() {
        let (k, fwd) = canon(t);
        let slot = &mut groups.entry(k).or_default()[usize::from(!fwd)];
        slot.get_or_insert(i);
    }
    let unique: Vec<usize> = groups.values().filter_map(|g| g[0].or(g[1])).collect();
    let corners = |i: usize| {
        let t = &triangles[i];
        [t.0, t.1, t.2].map(|v| vertices[v as usize])
    };
    let sub = |a: [f64; 3], b: [f64; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let cross = |a: [f64; 3], b: [f64; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let twins = groups
        .values()
        .filter(|g| g[0].is_some() && g[1].is_some())
        .count();
    if twins == 0 {
        return triangles;
    }
    let mut centre = [0.0; 3];
    for &i in &unique {
        let [a, b, c] = corners(i);
        for k in 0..3 {
            centre[k] += (a[k] + b[k] + c[k]) / (3.0 * unique.len() as f64);
        }
    }
    // ponytail: parity ray test is O(twins x faces); past 40M pairs fall
    // back to "faces away from the body's centroid".
    let ray = twins.saturating_mul(unique.len()) <= 40_000_000;
    let outward = |i: usize| -> bool {
        let [a, b, c] = corners(i);
        let n = cross(sub(b, a), sub(c, a));
        let len = dot(n, n).sqrt();
        if len < 1e-18 {
            return true;
        }
        let n = [n[0] / len, n[1] / len, n[2] / len];
        let o: [f64; 3] = std::array::from_fn(|k| (a[k] + b[k] + c[k]) / 3.0 + n[k] * 1e-6);
        if !ray {
            return dot(n, sub(o, centre)) >= 0.0;
        }
        let mut hits = 0;
        for &j in &unique {
            let [p, q, r] = corners(j);
            let (e1, e2) = (sub(q, p), sub(r, p));
            let h = cross(n, e2);
            let det = dot(e1, h);
            if det.abs() < 1e-15 {
                continue;
            }
            let s = sub(o, p);
            let u = dot(s, h) / det;
            if !(0.0..=1.0).contains(&u) {
                continue;
            }
            let qv = cross(s, e1);
            let v = dot(n, qv) / det;
            if v < 0.0 || u + v > 1.0 {
                continue;
            }
            if dot(e2, qv) / det > 1e-9 {
                hits += 1;
            }
        }
        hits % 2 == 0
    };
    let mut keep: Vec<usize> = groups
        .values()
        .filter_map(|g| match g {
            [Some(f), Some(b)] => Some(if outward(*f) { *f } else { *b }),
            [a, b] => a.or(*b),
        })
        .collect();
    keep.sort_unstable();
    keep.into_iter().map(|i| triangles[i]).collect()
}

#[cfg(test)]
mod back_face_tests {
    use super::*;

    #[test]
    fn double_sided_cube_keeps_one_outward_side() {
        let v: Vec<[f64; 3]> = (0..8)
            .map(|i| [(i & 1) as f64, ((i >> 1) & 1) as f64, ((i >> 2) & 1) as f64])
            .collect();
        // outward-wound cube
        let quads = [
            [0, 2, 3, 1],
            [4, 5, 7, 6],
            [0, 1, 5, 4],
            [2, 6, 7, 3],
            [0, 4, 6, 2],
            [1, 3, 7, 5],
        ];
        let mut tris = Vec::new();
        for q in quads {
            tris.push(BrepTriangle(q[0], q[1], q[2]));
            tris.push(BrepTriangle(q[0], q[2], q[3]));
        }
        let vol = |ts: &[BrepTriangle]| -> f64 {
            ts.iter()
                .map(|t| {
                    let [a, b, c] = [t.0, t.1, t.2].map(|i| v[i as usize]);
                    (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                        + a[2] * (b[0] * c[1] - b[1] * c[0]))
                        / 6.0
                })
                .sum()
        };
        // Whichever winding the cube was written in, the set that survives
        // must be a closed body of volume 1 (inward and outward both do
        // that), so the twin pair must collapse to 12 faces, and the
        // outward pick must be positive-volume for an outward cube.
        let outward = vol(&tris).abs();
        assert!((outward - 1.0).abs() < 1e-9);
        let positive = if vol(&tris) > 0.0 {
            tris.clone()
        } else {
            tris.iter().map(|t| BrepTriangle(t.0, t.2, t.1)).collect()
        };
        let mut doubled = positive.clone();
        doubled.extend(positive.iter().map(|t| BrepTriangle(t.0, t.2, t.1)));
        let kept = drop_back_faces(&v, doubled);
        assert_eq!(kept.len(), 12);
        assert!((vol(&kept) - 1.0).abs() < 1e-9, "kept the inward side");
    }
}

/// Snap vertices to 1e-5 ft and merge the ones that meet, and drop the
/// triangles that collapse (Revit's saved faces carry their own
/// copy of every corner, and two instances of one type differ by float noise,
/// which kept `share_shapes` from giving them one map). Output is the size
/// of the shape, and instances of one type compare equal.
pub(crate) fn tidy_mesh(
    vertices: Vec<[f64; 3]>,
    triangles: Vec<BrepTriangle>,
) -> (Vec<[f64; 3]>, Vec<BrepTriangle>) {
    let snap = |v: f64| (v * 100_000.0).round() / 100_000.0;
    let mut index: BTreeMap<[i64; 3], u32> = BTreeMap::new();
    let mut out: Vec<[f64; 3]> = Vec::new();
    let remap: Vec<u32> = vertices
        .iter()
        .map(|p| {
            let q = [snap(p[0]), snap(p[1]), snap(p[2])];
            let key = q.map(|c| (c * 100_000.0).round() as i64);
            *index.entry(key).or_insert_with(|| {
                out.push(q);
                (out.len() - 1) as u32
            })
        })
        .collect();
    let triangles = triangles
        .into_iter()
        .map(|t| {
            BrepTriangle(
                remap[t.0 as usize],
                remap[t.1 as usize],
                remap[t.2 as usize],
            )
        })
        .filter(|t| t.0 != t.1 && t.1 != t.2 && t.0 != t.2)
        .collect();
    (out, triangles)
}

#[cfg(test)]
mod probe_detail {
    /// `RVT_PROBE_FILE=<rvt> cargo test --lib probe_detail -- --ignored --nocapture`
    /// Triangles of every element's saved graphics at each detail level.
    #[test]
    #[ignore]
    fn probe_detail() {
        let Ok(path) = std::env::var("RVT_PROBE_FILE") else {
            return;
        };
        let mut rf = crate::RevitFile::open(&path).unwrap();
        let options = crate::native_document::Options {
            max_graph_values: 1_000_000,
            max_graph_objects: 1_000_000,
            ..Default::default()
        };
        for detail in 1..=3 {
            let scene = crate::native_saved_scene::extract_at_detail_with_control(
                &mut rf,
                &options,
                detail,
                &crate::control::WalkerControl::default(),
            )
            .unwrap();
            let mut per: Vec<usize> = scene
                .elements
                .iter()
                .map(|e| e.meshes.primitives.iter().map(|p| p.triangles.len()).sum())
                .collect();
            per.sort_unstable_by(|a, b| b.cmp(a));
            let total: usize = per.iter().sum();
            let top: usize = per.iter().take(10).sum();
            println!(
                "detail {detail}: elements {} triangles {total} top10 {top} largest {:?}",
                per.len(),
                &per[..per.len().min(5)]
            );
        }
    }
}

#[cfg(test)]
mod probe_trees {
    use super::*;

    /// `RVT_PROBE_FILE=<rvt> cargo test --lib probe_trees -- --ignored --nocapture`
    /// Why RPC trees and plants stay boxes: their saved graphics and family document.
    #[test]
    #[ignore]
    fn probe_trees() {
        let Ok(path) = std::env::var("RVT_PROBE_FILE") else {
            return;
        };
        let mut rf = crate::RevitFile::open(&path).unwrap();
        let exported =
            crate::convert::export(&mut rf, &crate::convert::ConvertOptions::default()).unwrap();
        let model = exported.model;
        let docs = crate::family_documents::read(&mut rf).unwrap();
        let mut picked: BTreeMap<String, (u64, String)> = BTreeMap::new();
        for e in &model.entities {
            if let IfcEntity::BuildingElement {
                name,
                type_guid: Some(tag),
                ifc_type,
                ..
            } = e
            {
                let n = format!("{name:?}");
                if n.contains("RPC")
                    || n.contains("Tree")
                    || n.contains("Plant")
                    || ifc_type.contains("GEOGRAPHIC")
                {
                    if let Ok(id) = tag.parse::<u64>() {
                        let key = n.split(':').take(2).collect::<Vec<_>>().join(":");
                        picked.entry(key).or_insert((id, ifc_type.clone()));
                    }
                }
            }
        }
        let options = crate::native_document::Options {
            selected_ids: picked.values().map(|(i, _)| *i).collect(),
            max_graph_values: 1_000_000,
            max_graph_objects: 1_000_000,
            ..Default::default()
        };
        let scene = crate::native_saved_scene::extract_at_detail_with_control(
            &mut rf,
            &options,
            3,
            &crate::control::WalkerControl::default(),
        )
        .unwrap();
        for (name, (id, ty)) in &picked {
            let solids = docs.solids_for(*id as i64);
            let doc = solids
                .map(|(p, d)| {
                    format!(
                        "solids {} undecoded {:?} flipped {}{}",
                        d.solids.len(),
                        d.undecoded,
                        p.flipped_x,
                        p.flipped_y
                    )
                })
                .unwrap_or("no family document".into());
            let saved = scene
                .elements
                .iter()
                .find(|e| e.identity.element_id == *id)
                .map(|e| {
                    let t: usize = e.meshes.primitives.iter().map(|p| p.triangles.len()).sum();
                    format!(
                        "saved prims {} tris {t} diag {:?}",
                        e.meshes.primitives.len(),
                        e.meshes.diagnostics.iter().take(2).collect::<Vec<_>>()
                    )
                })
                .unwrap_or("no saved element".into());
            println!("{name} [{ty} #{id}] | {doc} | {saved}");
        }
    }
}

#[cfg(test)]
mod probe_imposter {
    /// `RVT_PROBE_FILE=<rvt> cargo test --lib probe_imposter -- --ignored --nocapture`
    /// The graphics objects of an RPC tree (947273), a person (954641) and a car-like RPC.
    #[test]
    #[ignore]
    fn probe_imposter() {
        let Ok(path) = std::env::var("RVT_PROBE_FILE") else {
            return;
        };
        let mut rf = crate::RevitFile::open(&path).unwrap();
        let options = crate::native_document::Options {
            selected_ids: [947273_u64, 954641, 950367].into_iter().collect(),
            channels: [103_u64].into_iter().collect(),
            max_graph_values: 1_000_000,
            max_graph_objects: 1_000_000,
            ..Default::default()
        };
        let mut symbols = std::collections::BTreeSet::new();
        crate::native_document::extract_graphs(&mut rf, &options, |r| {
            if let Some(g) = &r.graph {
                for o in &g.objects {
                    if o.class_name == "InstanceInfo" {
                        let id = crate::native_metadata::identifier(&o.fields["m_symbolId"]).ok();
                        println!("element {} symbol {:?}", r.identity.element_id, id);
                        if let Some(id) = id.and_then(|i| u64::try_from(i).ok()) {
                            symbols.insert(id);
                        }
                    }
                }
            }
            Ok(())
        })
        .unwrap();
        let options = crate::native_document::Options {
            selected_ids: symbols,
            channels: [103_u64].into_iter().collect(),
            max_graph_values: 1_000_000,
            max_graph_objects: 1_000_000,
            ..Default::default()
        };
        crate::native_document::extract_graphs(&mut rf, &options, |r| {
            println!("== symbol {} channel {}", r.identity.element_id, r.channel);
            if let Some(g) = &r.graph {
                println!(
                    "objects {}: {:?}",
                    g.objects.len(),
                    g.objects
                        .iter()
                        .take(40)
                        .map(|o| o.class_name.as_str())
                        .collect::<Vec<_>>()
                );
                for (i, o) in g.objects.iter().enumerate() {
                    if o.class_name.contains("Imposter")
                        || o.class_name.contains("Facet")
                        || o.class_name.contains("PolyMesh")
                        || o.class_name == "GInstance"
                    {
                        let s = o.fields.to_string();
                        println!("[{i}] {} {}", o.class_name, &s[..s.len().min(1500)]);
                    }
                }
            }
            Ok(())
        })
        .unwrap();
    }
}

/// Whether `set` has a property called `name`.
fn has_property(set: &PropertySet, name: &str) -> bool {
    set.properties.iter().any(|property| property.name == name)
}

/// Whether `set` has the text property `name` with the value `text`.
fn has_text_property(set: &PropertySet, name: &str, text: &str) -> bool {
    set.properties.iter().any(|property| {
        property.name == name && matches!(&property.value, PropertyValue::Text(t) if t == text)
    })
}

/// Whether a wall's body is a plain extrusion or one with the stepped top,
/// which a saved mesh includes. A base cut at a floor is neither.
fn plain_or_stepped(solid_shape: &Option<SolidShape>, property_set: &Option<PropertySet>) -> bool {
    match (solid_shape, property_set) {
        (None, Some(set)) => !has_property(set, WALL_BASE_SOURCE_PROPERTY),
        (Some(_), Some(set)) => {
            has_text_property(set, "BodyHeightSource", WALL_TOP_STEPPED_BODY_HEIGHT_SOURCE)
                && !has_property(set, WALL_BASE_SOURCE_PROPERTY)
        }
        _ => false,
    }
}

/// The walls that embed a curtain wall (`WallEmbedWallGStep`), ElementId to
/// entity index: drawn as plain or stepped extrusions, alone in their Tag
/// (no other entity of any kind has it), not in `taken`, and with nothing
/// hosted in them but the opening their own curtain wall cuts (an
/// `IfcEntity::WallCutOpening` whose Tag is the curtain wall's: Revit's export
/// holds that void and the raised strip both). Nothing where the model has no
/// curtain wall, so a file without one reads no wall record here, and nothing
/// on a release outside [`EMBED_HOST_REVIT_VERSIONS`]. An `Err`
/// names why the walls' native records were not read.
fn embed_hosts(
    rf: &mut crate::RevitFile,
    model: &IfcModel,
    taken: &BTreeMap<u64, Vec<usize>>,
) -> Result<BTreeMap<u64, usize>, String> {
    if !rf
        .basic_file_info()
        .is_ok_and(|info| EMBED_HOST_REVIT_VERSIONS.contains(&info.version))
    {
        return Ok(BTreeMap::new());
    }
    let element_id = |entity: &IfcEntity, wanted: &str| match entity {
        IfcEntity::BuildingElement {
            ifc_type,
            type_guid: Some(tag),
            ..
        } if ifc_type == wanted => tag.parse::<u64>().ok(),
        _ => None,
    };
    let curtain: BTreeSet<u64> = model
        .entities
        .iter()
        .filter_map(|entity| element_id(entity, "IFCCURTAINWALL"))
        .collect();
    if curtain.is_empty() {
        return Ok(BTreeMap::new());
    }
    // Alone in the Tag is decided over every entity that has it, before any
    // piece is dropped for what it is hosting or how its base was cut.
    let mut by_tag: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
    for (index, entity) in model.entities.iter().enumerate() {
        if let IfcEntity::BuildingElement {
            type_guid: Some(tag),
            ..
        } = entity
        {
            if let Ok(id) = tag.parse::<u64>() {
                by_tag.entry(id).or_default().push(index);
            }
        }
    }
    let mut walls: BTreeMap<u64, usize> = BTreeMap::new();
    for (id, indices) in &by_tag {
        let [index] = indices.as_slice() else {
            continue;
        };
        let IfcEntity::BuildingElement {
            ifc_type,
            location_feet: Some(_),
            extrusion: Some(_),
            solid_shape,
            property_set,
            ..
        } = &model.entities[*index]
        else {
            continue;
        };
        if ifc_type == "IFCWALL"
            && !taken.contains_key(id)
            && plain_or_stepped(solid_shape, property_set)
        {
            walls.insert(*id, *index);
        }
    }
    let ids: BTreeSet<u64> = walls.keys().copied().collect();
    let steps =
        crate::native_wall_cuts::wall_cut_steps(rf, &ids).map_err(|error| format!("{error:#}"))?;
    // What is hosted in a wall: a mesh already holds the holes Revit cuts in
    // it, so an opening that voids it again would cut twice.
    let mut foreign: BTreeSet<usize> = BTreeSet::new();
    let mut cut_by: BTreeMap<usize, BTreeSet<u64>> = BTreeMap::new();
    for entity in &model.entities {
        match entity {
            IfcEntity::BuildingElement {
                host_element_index: Some(host),
                ..
            }
            | IfcEntity::VoidOpening { host, .. }
            | IfcEntity::WallProfileOpening { host, .. }
            | IfcEntity::HostedCutOpening { host, .. } => {
                foreign.insert(*host);
            }
            IfcEntity::WallCutOpening { host, tag, .. } => match tag.parse::<u64>() {
                Ok(cutter) => {
                    cut_by.entry(*host).or_default().insert(cutter);
                }
                Err(_) => {
                    foreign.insert(*host);
                }
            },
            _ => {}
        }
    }
    Ok(walls
        .into_iter()
        .filter_map(|(id, index)| {
            let cutters: BTreeSet<u64> = steps
                .get(&id)
                .into_iter()
                .flatten()
                .filter(|step| step.kind == WallCutKind::Embed && curtain.contains(&step.cutter))
                .map(|step| step.cutter)
                .collect();
            let own_only = cut_by
                .get(&index)
                .is_none_or(|openings| openings.is_subset(&cutters));
            (!cutters.is_empty() && own_only && !foreign.contains(&index)).then_some((id, index))
        })
        .collect())
}

/// Draw the wall at `index`, one that embeds a curtain wall, from its saved
/// `meshes` where they start [`EMBED_RAISE_MIN_FEET`] above its base and end
/// as far below its top: Revit has cut its end stubs away and raised its base
///. A wall whose mesh starts at its base keeps its body, as
/// does one whose mesh did not decode whole.
fn raise_embed_host(
    model: &mut IfcModel,
    index: usize,
    meshes: &GraphicsMeshes,
    report: &mut SavedMeshReport,
) {
    if meshes.primitives.is_empty() {
        return;
    }
    if meshes_partial(meshes) {
        report.embed_partial += 1;
        return;
    }
    let IfcEntity::BuildingElement {
        location_feet: Some(origin),
        rotation_radians,
        extrusion: Some(body),
        solid_shape,
        property_set,
        ..
    } = &mut model.entities[index]
    else {
        return;
    };
    let lowest = meshes
        .primitives
        .iter()
        .flat_map(|primitive| primitive.vertices.iter())
        .map(|vertex| vertex[2])
        .fold(f64::INFINITY, f64::min);
    let (base, top) = (origin[2], origin[2] + body.height_feet);
    if !(lowest - base >= EMBED_RAISE_MIN_FEET && top - lowest >= EMBED_RAISE_MIN_FEET) {
        return;
    }
    let (vertices, triangles) = local_mesh(&meshes.primitives, *origin, *rotation_radians);
    if triangles.is_empty() {
        return;
    }
    report.embed_raised += 1;
    *solid_shape = Some(SolidShape::TriangulatedFaceSet {
        vertices_feet: vertices,
        triangles,
        face_materials: Vec::new(),
    });
    if let Some(set) = property_set {
        let mut named = false;
        for property in &mut set.properties {
            if property.name == "BodySource" {
                property.value = PropertyValue::Text(SAVED_MESH_BODY_SOURCE.into());
                named = true;
            }
        }
        if !named {
            set.properties.push(Property {
                name: "BodySource".into(),
                value: PropertyValue::Text(SAVED_MESH_BODY_SOURCE.into()),
            });
        }
    }
}
