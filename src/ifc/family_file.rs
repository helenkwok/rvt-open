//! Standalone family files (`.rfa`): the family's own forms as elements.
//!
//! A project's elements are records with a level; a family file has none. Its
//! geometry is its solid forms (`ExtrusionElem`, `SweepElem`,
//! `RevolutionElem`, `BlendElem`, `SweptBlendElem`, `FreeFormElement`) and the
//! instances of the families nested in it, all in the family's own
//! coordinates. Each visible, non-void form becomes one element at the family
//! origin, in one storey ("Ref. Level", as Revit's family editor names its
//! level), and is drawn from, in order:
//!
//! 1. its saved graphics (channel 103, Revit's own tessellation of the form
//!    after any void has cut it), each face in its material;
//! 2. its decoded solid ([`crate::family_documents`], extrusions);
//! 3. the box of its stored bounding box, reported as missing.
//!
//! Nested instances are exported by the project exporter as family instances;
//! on releases whose saved graphics the project pass does not read (before
//! 2023) this pass draws them from their saved graphics too, and an instance
//! no pass drew (every one before 2015) from its family's decoded document,
//! placed by the instance's transform. Releases 2015 to
//! 2022 are read here only with a self-check: the mesh must lie inside the
//! element's stored bounding box.

use super::IfcModel;
use super::entities::{BrepTriangle, IfcEntity, Property, PropertySet, PropertyValue, SolidShape};
use crate::RevitFile;
use crate::family_documents::{self, HOST_DOCUMENT};
use crate::schema_objects::{Fields, Value, for_each_object_where};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// `BodySource` of a form drawn from its saved graphics.
pub const FORM_SAVED_MESH_BODY_SOURCE: &str = "family_form_saved_mesh";
/// `BodySource` of a form drawn from its decoded solid.
pub const FORM_SOLID_BODY_SOURCE: &str = "family_form_decoded_solid";
/// `BodySource` of a sweep, revolution or blend rebuilt from its sketches.
pub const FORM_SOLVED_BODY_SOURCE: &str = "family_form_solved_solid";
/// `BodySource` of a form drawn as its stored bounding box.
pub const FORM_BOX_BODY_SOURCE: &str = "family_form_bounding_box";
/// `BodySource` of the family template's sample host wall.
pub const SAMPLE_WALL_BODY_SOURCE: &str = "family_sample_host_wall";
/// `BodySource` of a nested instance drawn from its saved graphics here.
pub const NESTED_SAVED_MESH_BODY_SOURCE: &str = "family_nested_saved_mesh";

/// `BodySource` of a nested instance drawn from its family's decoded
/// document.
pub const NESTED_DOCUMENT_BODY_SOURCE: &str = "family_nested_document";
/// `BodySource` of such an instance stretched to its own stored box: its
/// family's document draws that family's saved type, and the instance's type
/// can be another size (a door's panel of 4 ft in a 3 ft door).
pub const NESTED_DOCUMENT_SCALED_BODY_SOURCE: &str = "family_nested_document_scaled";
/// A stretch beyond these factors is another shape, not another size.
const NESTED_SCALE_LIMITS: (f64, f64) = (0.2, 5.0);
/// A body within this fraction of its stored box fits it.
const NESTED_FACET_ALLOWANCE: f64 = 0.05;
/// A solved factor above 1 by no more than this is rounding, not growth.
const NESTED_ROUNDOFF: f64 = 1e-6;
/// How well a turn must tell a body's own axes apart to shrink it along them
/// (the determinant of `|l|^T |l|` over the axes solved: 1 on the axes, 0.41
/// at 25 degrees, 0.25 at 60, 0.1 at about 36 and 54, 0 at 45).
const NESTED_TURN_DETERMINED: f64 = 0.1;

/// Why a form whose voids removed all of it is not drawn.
const REMOVED_BY_VOID: &str = "removed_by_void";

/// The storey a family's elements are placed in.
pub const FAMILY_STOREY: &str = "Ref. Level";

/// `IS_VISIBLE_PARAM`.
const VISIBLE_PARAMETER: i64 = -1_006_205;

/// The solid-form classes, with the kind name the report uses.
const FORM_CLASSES: [(&str, &str); 6] = [
    ("ExtrusionElem", "extrusion"),
    ("SweepElem", "sweep"),
    ("RevolutionElem", "revolution"),
    ("BlendElem", "blend"),
    ("SweptBlendElem", "swept_blend"),
    ("FreeFormElement", "free_form"),
];

/// What a family file is and what of its geometry was drawn. Serialized into
/// the export diagnostics' `input.family`.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct FamilySummary {
    /// One line for a reader: "Revit family, release 2027, category
    /// Furniture, types A, B".
    pub summary: String,
    pub release: Option<u32>,
    /// The family's category (`PartAtom`'s Revit grouping).
    pub category: Option<String>,
    /// Its types (`PartAtom` `A:part` titles).
    pub types: Vec<String>,
    /// The type the file was saved with, which its forms draw.
    pub default_type: Option<String>,
    /// Solid forms in the family's own document, voids and hidden ones
    /// included, by kind.
    pub forms_found: BTreeMap<String, usize>,
    /// Visible solid forms drawn, by `BodySource`.
    pub forms_drawn: BTreeMap<String, usize>,
    /// The same by `kind:BodySource`.
    #[serde(default)]
    pub forms_drawn_by_kind: BTreeMap<String, usize>,
    /// Visible solid forms not drawn from file data (drawn as their box, or
    /// not at all), by `kind:reason`.
    pub forms_missing: BTreeMap<String, usize>,
    /// Forms whose `Visible` is off.
    pub forms_hidden: usize,
    /// Void forms (they cut; their result is in the cut forms' graphics).
    pub voids: usize,
    /// Forms of groups no instance places (a group type's own copy of its
    /// members): not drawn, as Revit draws none.
    #[serde(default)]
    pub forms_unplaced: usize,
    /// Forms of deleted elements (their last copies in an older partition,
    /// the element table listing none of them): not drawn.
    #[serde(default)]
    pub forms_deleted: usize,
    /// Solids a decoded void was cut from.
    #[serde(default)]
    pub void_cuts: usize,
    /// Sample host walls drawn (with the family's openings cut in them).
    #[serde(default)]
    pub sample_walls: usize,
    /// Openings of those walls that could not be cut (the wall is drawn
    /// with the others only).
    #[serde(default)]
    pub sample_wall_openings_uncut: usize,
    /// Solids a void meets whose cut gave up: drawn uncut.
    #[serde(default)]
    pub void_cuts_failed: usize,
    /// Instances of nested families, and how many this pass drew.
    pub nested_instances: usize,
    pub nested_drawn: usize,
    /// Of `nested_drawn`, instances whose family's voids are not cut.
    #[serde(default)]
    pub nested_uncut: usize,
    /// Undecoded forms of nested families drawn as their stored boxes.
    #[serde(default)]
    pub nested_form_boxes: usize,
    /// Of `nested_drawn`, instances stretched to their own stored box.
    #[serde(default)]
    pub nested_scaled: usize,
    /// Of `nested_drawn`, instances whose stored box differs from their
    /// body by a stretch beyond the limits: drawn as their document draws them.
    #[serde(default)]
    pub nested_size_mismatch: usize,
    /// Of `nested_drawn`, turned instances whose stored box is smaller than
    /// their turned body but whose turn does not tell which own axes to
    /// shrink: drawn as their document draws them.
    #[serde(default)]
    pub nested_size_undetermined: usize,
    /// Of `nested_drawn`, forms of their families' documents not in the
    /// mesh, by why (an instance counted once per reason).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub nested_partial: BTreeMap<String, usize>,
    /// Nested instances not drawn from their family's document, by why.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub nested_missing: BTreeMap<String, usize>,
    /// Why sweeps, revolutions and blends were not rebuilt from their
    /// sketches, by `kind:reason` (forms whose saved graphics drew them too).
    #[serde(default)]
    pub solve_failures: BTreeMap<String, usize>,
    /// Why the saved graphics could not be read at all, when they could not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved_graphics_error: Option<String>,
}

struct Form {
    id: i64,
    /// Its class tag and the number of the partition it was read from.
    tag: u16,
    part: u32,
    kind: &'static str,
    cutting: bool,
    visible: bool,
    material: Option<i64>,
}

fn id_of(f: &Fields, name: &str) -> Option<i64> {
    match f.get(name)? {
        Value::Id(i) | Value::Int(i) => Some(*i),
        _ => None,
    }
}

fn vec3(v: &Value) -> Option<[f64; 3]> {
    let Value::List(l) = v else { return None };
    let f = |v: &Value| match v {
        Value::Float(f) => Some(*f),
        Value::Int(i) => Some(*i as f64),
        _ => None,
    };
    match l.as_slice() {
        [x, y, z] => Some([f(x)?, f(y)?, f(z)?]),
        _ => None,
    }
}

/// `PartAtom`'s category: the Revit grouping term (the one that is not an
/// OmniClass number).
fn category(atom: &crate::part_atom::PartAtom) -> Option<String> {
    atom.categories
        .iter()
        .map(|c| c.term.trim())
        .find(|t| !t.is_empty() && !t.chars().all(|c| c.is_ascii_digit() || c == '.'))
        .map(str::to_string)
}

/// The IFC class of a family category: the category map's entry for the
/// category name without spaces (`Generic Models` -> `GenericModel`).
fn ifc_type_for(category: Option<&str>) -> (&'static str, Option<&'static str>) {
    let Some(c) = category else {
        return ("IFCBUILDINGELEMENTPROXY", None);
    };
    let key: String = c.split_whitespace().collect();
    let found = super::category_map::lookup(&key)
        .or_else(|| key.strip_suffix('s').and_then(super::category_map::lookup));
    match found {
        Some(m) => (m.ifc_type, m.predefined_type),
        None => ("IFCBUILDINGELEMENTPROXY", None),
    }
}

/// The family summary alone (report fields, no geometry).
pub fn summary(rf: &mut RevitFile) -> FamilySummary {
    let release = rf.basic_file_info().ok().map(|b| b.version);
    let atom = rf.part_atom().ok();
    let category = atom.as_ref().and_then(category);
    let types: Vec<String> = atom
        .as_ref()
        .and_then(|a| a.family_types().ok())
        .map(|t| t.into_iter().map(|t| t.name).collect())
        .unwrap_or_default();
    let default_type = atom
        .as_ref()
        .and_then(|a| a.title.clone())
        .filter(|t| types.is_empty() || types.contains(t));
    let mut s = FamilySummary {
        release,
        category,
        types,
        default_type,
        ..Default::default()
    };
    s.summary = summary_line(&s);
    s
}

fn summary_line(s: &FamilySummary) -> String {
    let mut line = "Revit family".to_string();
    if let Some(r) = s.release {
        line += &format!(", release {r}");
    }
    if let Some(c) = &s.category {
        line += &format!(", category {c}");
    }
    if !s.types.is_empty() {
        line += &format!(
            ", type{} {}",
            if s.types.len() == 1 { "" } else { "s" },
            s.types.join(", ")
        );
    }
    if let Some(d) = &s.default_type {
        line += &format!(" (drawn: {d})");
    }
    line
}

fn bbox_of(vertices: &[[f64; 3]]) -> Option<[[f64; 3]; 2]> {
    let mut b = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    for v in vertices {
        for k in 0..3 {
            b[0][k] = b[0][k].min(v[k]);
            b[1][k] = b[1][k].max(v[k]);
        }
    }
    b[0][0].is_finite().then_some(b)
}

/// Whether `inner` lies inside `outer` (within 1e-3 ft and 0.1% of its size).
fn inside(inner: [[f64; 3]; 2], outer: [[f64; 3]; 2]) -> bool {
    (0..3).all(|k| {
        let tol = 1e-3_f64.max(1e-3 * (outer[1][k] - outer[0][k]).abs());
        inner[0][k] >= outer[0][k] - tol && inner[1][k] <= outer[1][k] + tol
    })
}

fn box_mesh(b: [[f64; 3]; 2]) -> SolidShape {
    let [lo, hi] = b;
    let vertices: Vec<[f64; 3]> = (0..8)
        .map(|i| {
            [
                if i & 1 == 0 { lo[0] } else { hi[0] },
                if i & 2 == 0 { lo[1] } else { hi[1] },
                if i & 4 == 0 { lo[2] } else { hi[2] },
            ]
        })
        .collect();
    // Outward-wound quads of the cube, two triangles each.
    const QUADS: [[u32; 4]; 6] = [
        [0, 2, 3, 1],
        [4, 5, 7, 6],
        [0, 1, 5, 4],
        [2, 6, 7, 3],
        [0, 4, 6, 2],
        [1, 3, 7, 5],
    ];
    let triangles = QUADS
        .iter()
        .flat_map(|q| {
            [
                BrepTriangle(q[0], q[1], q[2]),
                BrepTriangle(q[0], q[2], q[3]),
            ]
        })
        .collect();
    SolidShape::TriangulatedFaceSet {
        vertices_feet: vertices,
        triangles,
        face_materials: Vec::new(),
        uvs: Vec::new(),
    }
}

fn set_body_source(entity: &mut IfcEntity, source: &str) {
    if let IfcEntity::BuildingElement {
        property_set: Some(set),
        ..
    } = entity
    {
        for p in &mut set.properties {
            if p.name == "BodySource" {
                p.value = PropertyValue::Text(source.into());
            }
            super::export_content::mark_space_family_body(p);
        }
    }
}

/// Add the family file's forms to `model`, draw its nested instances where
/// the project passes do not, and place every element in one storey.
/// `saved_graphics` false (`--no-saved-meshes`) skips step 1.
pub fn attach(
    rf: &mut RevitFile,
    model: &mut IfcModel,
    saved_graphics: bool,
) -> anyhow::Result<FamilySummary> {
    let mut out = summary(rf);
    let release = out.release.unwrap_or(0);

    // The host document's forms, bounding boxes and nested instances.
    let layouts = rf.schema_layouts()?;
    let names: BTreeMap<u16, String> = layouts
        .classes
        .iter()
        .map(|(t, c)| (*t, c.name.clone()))
        .collect();
    let mut first_section: BTreeMap<String, usize> = BTreeMap::new();
    for name in rf.partition_stream_names() {
        if let Ok(inflated) = rf.inflated_partition(&name) {
            if let Some((at, _)) = inflated.sections().first() {
                first_section.insert(name, *at);
            }
        }
    }
    let mut forms: Vec<Form> = Vec::new();
    let mut boxes: BTreeMap<i64, [[f64; 3]; 2]> = BTreeMap::new();
    let mut nested: BTreeSet<i64> = BTreeSet::new();
    // Material element id -> its material's name (the model's materials
    // are listed by name).
    let mut material_names: BTreeMap<i64, String> = BTreeMap::new();
    // The newest partition each object was read from: older copies of a
    // record a file keeps in earlier partitions are not current.
    let mut newest: BTreeMap<(u16, i64), u32> = BTreeMap::new();
    let mut unplaced_forms: BTreeSet<i64> = BTreeSet::new();
    // Deleted elements' last copies an older partition keeps.
    let deleted = family_documents::deleted_elements(rf);
    let mut deleted_forms: BTreeSet<i64> = BTreeSet::new();
    for_each_object_where(
        rf,
        |_, _| true,
        |x| {
            if first_section
                .get(&x.stream)
                .is_some_and(|at| x.frame.span.start >= *at)
            {
                return; // inside an embedded (nested) family document
            }
            let Some(class) = names.get(&x.frame.header.tag) else {
                return;
            };
            let Some(root) = x.objects.first() else {
                return;
            };
            let id = x.frame.header.id as i64;
            if FORM_CLASSES.iter().any(|(c, _)| c == class) && deleted.holds(id, &x.stream) {
                deleted_forms.insert(id);
                return;
            }
            let part = crate::partition_room_parameters::partition_number(&x.stream);
            match newest.get(&(x.frame.header.tag, id)) {
                Some(p) if *p > part => return,
                _ => {
                    newest.insert((x.frame.header.tag, id), part);
                }
            }
            if class == "GElement" {
                if let Some(Value::List(b)) = root.get("m_bBox") {
                    if let [lo, hi] = b.as_slice() {
                        if let (Some(lo), Some(hi)) = (vec3(lo), vec3(hi)) {
                            boxes.insert(id, [lo, hi]);
                        }
                    }
                }
                return;
            }
            // A group type's own copy of its members (no instance places
            // the group): Revit draws none of them.
            let unplaced = id_of(root, "m_unplacedOwnerId").is_some_and(|o| o > 0);
            // Each accepted copy (the newest so far) decides.
            if class == "FamilyInstance" {
                if unplaced {
                    nested.remove(&id);
                } else {
                    nested.insert(id);
                }
                return;
            }
            if class == "MaterialElem" {
                let name = x
                    .objects
                    .iter()
                    .filter(|o| names.get(&o.tag).map(String::as_str) == Some("Material"))
                    .find_map(|o| match o.get("m_name") {
                        Some(Value::Text(t)) if !t.trim().is_empty() => Some(t.clone()),
                        _ => None,
                    });
                if let Some(name) = name {
                    material_names.insert(id, name);
                }
                return;
            }
            let Some((_, kind)) = FORM_CLASSES.iter().find(|(c, _)| c == class) else {
                return;
            };
            let mut visible = true;
            for o in &x.objects {
                if names.get(&o.tag).map(String::as_str) != Some("ParamValueSetInt") {
                    continue;
                }
                if let Some(Value::List(l)) = o.get("m_paramSet") {
                    for e in l {
                        if let Value::Object(pv) = e {
                            if id_of(pv, "m_paramId") == Some(VISIBLE_PARAMETER) {
                                visible = id_of(pv, "m_value") != Some(0);
                            }
                        }
                    }
                }
            }
            if unplaced {
                unplaced_forms.insert(id);
                return;
            }
            unplaced_forms.remove(&id);
            forms.push(Form {
                id,
                tag: x.frame.header.tag,
                part,
                kind,
                cutting: matches!(root.get("m_cutting"), Some(Value::Bool(true))),
                visible,
                material: id_of(root, "m_materialId").filter(|m| *m > 0),
            });
        },
    )?;
    forms
        .retain(|f| newest.get(&(f.tag, f.id)) == Some(&f.part) && !unplaced_forms.contains(&f.id));
    out.forms_unplaced = unplaced_forms.len();
    // Decoded solids of the family's own document, and which forms belong
    // to a nested family's document instead (files without keyed sections).
    let docs = family_documents::read_with_host(rf, true).ok();
    if let Some(d) = &docs {
        forms.retain(|f| {
            !d.nested_objects.contains(&f.id) && !d.host_samples.contains(&(HOST_DOCUMENT, f.id))
        });
        nested.retain(|i| !d.nested_objects.contains(i));
        // A nested family's forms are its document's, not deleted ones.
        deleted_forms.retain(|i| !d.nested_objects.contains(i));
    }
    out.forms_deleted = deleted_forms.len();
    forms.sort_by_key(|f| f.id);
    for f in &forms {
        *out.forms_found.entry(f.kind.to_string()).or_default() += 1;
    }
    out.voids = forms.iter().filter(|f| f.cutting).count();
    out.forms_hidden = forms.iter().filter(|f| !f.cutting && !f.visible).count();
    out.nested_instances = nested.len();

    // Nested instances still drawn as their record box, by Tag.
    let mut nested_entities: BTreeMap<i64, usize> = BTreeMap::new();
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
            if nested.contains(&id) {
                nested_entities.insert(id, index);
            }
        }
    }

    // Saved graphics of the visible forms and of those nested instances.
    let drawn_forms: Vec<&Form> = forms.iter().filter(|f| !f.cutting && f.visible).collect();
    let mut wanted: BTreeSet<u64> = drawn_forms.iter().map(|f| f.id as u64).collect();
    // The project pass reads 2023 and later itself.
    if release < 2023 {
        wanted.extend(nested_entities.keys().map(|id| *id as u64));
    }
    let mut saved: BTreeMap<i64, crate::native_saved_scene::SavedElement> = BTreeMap::new();
    // Elements whose saved graphics were read but not taken, and why.
    let mut rejected: BTreeMap<i64, &'static str> = BTreeMap::new();
    if saved_graphics && (2015..=2027).contains(&release) && !wanted.is_empty() {
        let ids: Vec<u64> = wanted.into_iter().collect();
        for batch in ids.chunks(256) {
            let options = crate::native_document::Options {
                selected_ids: batch.iter().copied().collect(),
                max_graph_values: 1_000_000,
                max_graph_objects: 1_000_000,
                releases: 2015..=2027,
                ..Default::default()
            };
            match crate::native_saved_scene::extract_at_detail(rf, &options, 3) {
                Ok(scene) => {
                    for e in scene.elements {
                        let id = e.identity.element_id as i64;
                        let mesh_box = bbox_of(
                            &e.meshes
                                .primitives
                                .iter()
                                .flat_map(|p| p.vertices.iter().copied())
                                .collect::<Vec<_>>(),
                        );
                        // Before 2023 the record framing is checked by the
                        // element's own stored bounding box.
                        let checked = release >= 2023
                            || match (mesh_box, boxes.get(&id)) {
                                (Some(m), Some(b)) => inside(m, *b),
                                _ => false,
                            };
                        if mesh_box.is_none() {
                            rejected.insert(id, "saved_graphics_empty");
                        } else if super::saved_meshes::is_partial(&e) {
                            rejected.insert(id, "saved_graphics_partial");
                        } else if !checked {
                            rejected.insert(id, "saved_graphics_outside_bbox");
                        } else {
                            saved.insert(id, e);
                        }
                    }
                }
                Err(e) => out.saved_graphics_error = Some(format!("{e:#}")),
            }
        }
    }

    if let Some(d) = &docs {
        out.solve_failures = d.stats.solve_failures.clone();
        out.void_cuts = d.stats.void_cuts;
        out.sample_wall_openings_uncut = d.stats.wall_openings_uncut;
        out.void_cuts_failed = d.stats.void_cuts_failed;
    }
    let solids: BTreeMap<i64, Vec<&family_documents::FamilySolid>> = {
        let mut m: BTreeMap<i64, Vec<&family_documents::FamilySolid>> = BTreeMap::new();
        if let Some(d) = docs.as_ref().and_then(|d| d.docs.get(&HOST_DOCUMENT)) {
            for s in &d.solids {
                m.entry(s.form_id).or_default().push(s);
            }
        }
        m
    };
    let mut resolver = None;

    let (ifc_type, predefined) = ifc_type_for(out.category.as_deref());
    let label = out
        .default_type
        .clone()
        .or_else(|| out.category.clone())
        .unwrap_or_else(|| "Family".into());
    for form in drawn_forms {
        let mut material_index = None;
        let (shape, source) = if let Some(e) = saved.get(&form.id) {
            match super::saved_meshes::element_shape(&mut model.materials, e, [0.0; 3], 0.0) {
                Some((shape, _)) => (Some(shape), FORM_SAVED_MESH_BODY_SOURCE),
                None => (None, ""),
            }
        } else {
            (None, "")
        };
        let (shape, source) = match (shape, solids.get(&form.id)) {
            (Some(s), _) => (Some(s), source),
            // Its voids removed the whole form: nothing to draw.
            (None, Some(list)) if list.iter().all(|s| s.triangles.is_empty()) => {
                (None, REMOVED_BY_VOID)
            }
            (None, Some(list)) => {
                let mut vertices = Vec::new();
                let mut triangles = Vec::new();
                for s in list {
                    let base = vertices.len() as u32;
                    vertices.extend(s.vertices.iter().copied());
                    triangles.extend(
                        s.triangles
                            .iter()
                            .map(|t| BrepTriangle(base + t[0], base + t[1], base + t[2])),
                    );
                }
                let (vertices, triangles) = super::saved_meshes::tidy_mesh(vertices, triangles);
                (
                    Some(SolidShape::TriangulatedFaceSet {
                        vertices_feet: vertices,
                        triangles,
                        face_materials: Vec::new(),
                        uvs: Vec::new(),
                    }),
                    if form.kind == "extrusion" {
                        FORM_SOLID_BODY_SOURCE
                    } else {
                        FORM_SOLVED_BODY_SOURCE
                    },
                )
            }
            (None, None) => match boxes.get(&form.id) {
                Some(b) => (Some(box_mesh(*b)), FORM_BOX_BODY_SOURCE),
                None => (None, ""),
            },
        };
        if source != FORM_SAVED_MESH_BODY_SOURCE
            && source != FORM_SOLID_BODY_SOURCE
            && source != FORM_SOLVED_BODY_SOURCE
        {
            let reason = if source == REMOVED_BY_VOID {
                REMOVED_BY_VOID
            } else if !saved_graphics {
                "saved_graphics_off"
            } else if release < 2015 {
                // No saved graphics are read before 2015: why the form did
                // not decode.
                docs.as_ref()
                    .and_then(|d| d.docs.get(&HOST_DOCUMENT))
                    .and_then(|d| d.form_failures.get(&form.id).copied())
                    .map_or("not_decoded", |why| match why {
                        "geometry" => "geometry_not_decoded",
                        "fields" => "fields_not_read",
                        "bbox_mismatch" => "solve_misses_stored_box",
                        w => w,
                    })
            } else if out.saved_graphics_error.is_some() {
                "saved_graphics_error"
            } else {
                rejected
                    .get(&form.id)
                    .copied()
                    .unwrap_or("no_saved_graphics")
            };
            *out.forms_missing
                .entry(format!("{}:{reason}", form.kind))
                .or_default() += 1;
        }
        let Some(shape) = shape else {
            continue;
        };
        *out.forms_drawn.entry(source.to_string()).or_default() += 1;
        *out.forms_drawn_by_kind
            .entry(format!("{}:{source}", form.kind))
            .or_default() += 1;
        if let Some(m) = form.material {
            if resolver.is_none() {
                resolver =
                    crate::native_saved_scene::material_resolver(rf, &Default::default()).ok();
            }
            if let Some(found) = resolver
                .as_ref()
                .and_then(|r| r.resolve(form.id as u64, None, 0, 0, Some(m)))
            {
                material_index = Some(super::saved_meshes::material_index_for(
                    &mut model.materials,
                    &found,
                ));
            }
            // Releases the resolver does not read (before 2015): the
            // model's material of the same name, read from the file's
            // material records.
            if material_index.is_none() {
                material_index = material_names
                    .get(&m)
                    .and_then(|n| model.materials.iter().position(|mi| &mi.name == n));
            }
        }
        model.entities.push(IfcEntity::BuildingElement {
            ifc_type: ifc_type.into(),
            name: format!("{label}:{} {}", form.kind, form.id),
            type_guid: Some(form.id.to_string()),
            predefined_type: predefined.map(str::to_string),
            storey_index: None,
            material_index,
            property_set: Some(PropertySet {
                name: "RvtFamilyGeometry".into(),
                properties: vec![
                    Property {
                        name: "BodySource".into(),
                        value: PropertyValue::Text(source.into()),
                    },
                    Property {
                        name: "FormKind".into(),
                        value: PropertyValue::Text(form.kind.into()),
                    },
                ],
            }),
            location_feet: Some([0.0; 3]),
            rotation_radians: None,
            extrusion: None,
            host_element_index: None,
            material_layer_set_index: None,
            material_profile_set_index: None,
            solid_shape: Some(shape),
            representation_map_index: None,
        });
    }

    // The template's sample host wall, as the family editor shows it.
    for wall in docs
        .as_ref()
        .and_then(|d| d.docs.get(&HOST_DOCUMENT))
        .map(|d| d.sample_walls.as_slice())
        .unwrap_or_default()
    {
        let (vertices, triangles) = super::saved_meshes::tidy_mesh(
            wall.vertices.clone(),
            wall.triangles
                .iter()
                .map(|t| BrepTriangle(t[0], t[1], t[2]))
                .collect(),
        );
        model.entities.push(IfcEntity::BuildingElement {
            ifc_type: "IFCWALL".into(),
            name: format!("{label}:sample host wall {}", wall.form_id),
            type_guid: Some(wall.form_id.to_string()),
            predefined_type: None,
            storey_index: None,
            material_index: None,
            property_set: Some(PropertySet {
                name: "RvtFamilyGeometry".into(),
                properties: vec![Property {
                    name: "BodySource".into(),
                    value: PropertyValue::Text(SAMPLE_WALL_BODY_SOURCE.into()),
                }],
            }),
            location_feet: Some([0.0; 3]),
            rotation_radians: None,
            extrusion: None,
            host_element_index: None,
            material_layer_set_index: None,
            material_profile_set_index: None,
            solid_shape: Some(SolidShape::TriangulatedFaceSet {
                vertices_feet: vertices,
                triangles,
                face_materials: Vec::new(),
                uvs: Vec::new(),
            }),
            representation_map_index: None,
        });
        out.sample_walls += 1;
    }

    // Nested instances before 2023, from their saved graphics.
    if release < 2023 {
        for (id, index) in &nested_entities {
            let Some(e) = saved.get(id) else { continue };
            let IfcEntity::BuildingElement {
                location_feet: Some(origin),
                rotation_radians,
                ..
            } = &model.entities[*index]
            else {
                continue;
            };
            let (origin, rotation) = (*origin, rotation_radians.unwrap_or(0.0));
            let Some((shape, _)) =
                super::saved_meshes::element_shape(&mut model.materials, e, origin, rotation)
            else {
                continue;
            };
            if let IfcEntity::BuildingElement { solid_shape, .. } = &mut model.entities[*index] {
                *solid_shape = Some(shape);
            }
            set_body_source(&mut model.entities[*index], NESTED_SAVED_MESH_BODY_SOURCE);
            out.nested_drawn += 1;
        }
    }

    // Nested instances no other pass drew, from their family's decoded
    // document placed by the instance's transform (its matrix's columns
    // are the instance's axes, as for a project's instances).
    let placed: BTreeSet<i64> = model
        .entities
        .iter()
        .filter_map(|e| match e {
            IfcEntity::BuildingElement {
                type_guid: Some(t), ..
            } => t.parse().ok(),
            _ => None,
        })
        .collect();
    if let Some(d) = &docs {
        for (id, pose) in &d.instances {
            if !nested.contains(id) || placed.contains(id) {
                continue;
            }
            let Some((guid, _)) = d.document_of_symbol(pose.symbol) else {
                *out.nested_missing
                    .entry("no_family_document".into())
                    .or_default() += 1;
                continue;
            };
            let mut body = NestedBody::default();
            body.collect(d, guid, pose_affine(pose), 0);
            if body.triangles.is_empty() {
                for w in body.missing.keys() {
                    *out.nested_missing.entry(w.clone()).or_default() += 1;
                }
                if body.missing.is_empty() {
                    *out.nested_missing
                        .entry("no_solid_form".into())
                        .or_default() += 1;
                }
                continue;
            }
            // The instance's own stored box is the size of its type; the
            // document draws its family's saved type, which can be another.
            // Revit stores the box of the instance's turned own box (on a ring
            // of chairs measured, those turned by 25 and 60 degrees store the
            // box of their family's box turned, 10% to 19% larger than their
            // turned body), so the body's own box is turned the same way
            // before the two are compared.
            let mut source = NESTED_DOCUMENT_BODY_SOURCE;
            if let Some(stored) = boxes.get(id) {
                match stretch_nested_to_box(&mut body.vertices, *stored, &pose_affine(pose)) {
                    NestedStretch::Fits => {}
                    NestedStretch::Stretched => {
                        source = NESTED_DOCUMENT_SCALED_BODY_SOURCE;
                        out.nested_scaled += 1;
                    }
                    NestedStretch::BeyondLimits => out.nested_size_mismatch += 1,
                    NestedStretch::Undetermined => out.nested_size_undetermined += 1,
                }
            }
            let material_index = body
                .material
                .and_then(|(_, m)| material_names.get(&m))
                .and_then(|name| model.materials.iter().position(|mi| &mi.name == name));
            model.entities.push(IfcEntity::BuildingElement {
                ifc_type: ifc_type.into(),
                name: format!("{label}:nested {id}"),
                type_guid: Some(id.to_string()),
                predefined_type: predefined.map(str::to_string),
                storey_index: None,
                material_index,
                property_set: Some(PropertySet {
                    name: "RvtFamilyGeometry".into(),
                    properties: vec![Property {
                        name: "BodySource".into(),
                        value: PropertyValue::Text(source.into()),
                    }],
                }),
                location_feet: Some([0.0; 3]),
                rotation_radians: None,
                extrusion: None,
                host_element_index: None,
                material_layer_set_index: None,
                material_profile_set_index: None,
                solid_shape: Some(SolidShape::TriangulatedFaceSet {
                    vertices_feet: body.vertices,
                    triangles: body.triangles,
                    face_materials: Vec::new(),
                    uvs: Vec::new(),
                }),
                representation_map_index: None,
            });
            out.nested_drawn += 1;
            out.nested_uncut += usize::from(body.uncut);
            out.nested_form_boxes += body.boxes;
            for w in body.missing.into_keys() {
                *out.nested_partial.entry(w).or_default() += 1;
            }
        }
    }

    // One storey at the family origin.
    if model.building_storeys.is_empty() {
        model.building_storeys.push(super::Storey {
            name: FAMILY_STOREY.into(),
            elevation_feet: 0.0,
        });
    }
    for entity in &mut model.entities {
        if let IfcEntity::BuildingElement { storey_index, .. } = entity {
            storey_index.get_or_insert(0);
        }
    }
    if model.building_name.is_none() {
        model.building_name = out.default_type.clone().or_else(|| out.category.clone());
    }
    out.summary = summary_line(&out);
    Ok(out)
}

/// What [`stretch_nested_to_box`] did.
#[derive(Debug, PartialEq)]
pub enum NestedStretch {
    /// The body fits its stored box (or is smaller, or within the facet
    /// allowance): left as it is.
    Fits,
    /// The body was shrunk onto its stored box.
    Stretched,
    /// The factors are beyond the limits or would enlarge an axis: left as
    /// it is.
    BeyondLimits,
    /// The stored box is smaller than the turned body's, but its turn does
    /// not tell which of the body's own axes to shrink (near 45 degrees, a
    /// box's width and depth add up the same either way).
    Undetermined,
}

/// Fit a nested instance's body (`vertices`, placed by `map`) to its stored
/// box. `stored` is axis-aligned in the host's frame, and is the box of the
/// instance's own box turned by `map`, so the body's own box (in its family's
/// frame) is turned likewise and compared with it. When `stored` lies inside
/// that turned box and differs from it by more than the box tolerance and
/// `NESTED_FACET_ALLOWANCE` (5%), the body is shrunk along its own axes by the
/// factors whose turned box is `stored` (each within the limits; an axis
/// either box is flat along keeps its size), and centred on `stored`. A
/// mirrored map keeps its reflection: only the size changes.
pub fn stretch_nested_to_box(
    vertices: &mut [[f64; 3]],
    stored: [[f64; 3]; 2],
    map: &Affine,
) -> NestedStretch {
    let (l, t) = map;
    let Some(inv) = inverse3(l) else {
        return NestedStretch::Fits;
    };
    let own: Vec<[f64; 3]> = vertices
        .iter()
        .map(|v| std::array::from_fn(|k| (0..3).map(|j| inv[k][j] * (v[j] - t[j])).sum()))
        .collect();
    let Some([olo, ohi]) = bbox_of(&own) else {
        return NestedStretch::Fits;
    };
    let a: [[f64; 3]; 3] = std::array::from_fn(|k| std::array::from_fn(|j| l[k][j].abs()));
    let ow: [f64; 3] = std::array::from_fn(|j| ohi[j] - olo[j]);
    let oc: [f64; 3] = std::array::from_fn(|j| (olo[j] + ohi[j]) / 2.0);
    let pc: [f64; 3] = std::array::from_fn(|k| t[k] + (0..3).map(|j| l[k][j] * oc[j]).sum::<f64>());
    let pw: [f64; 3] = std::array::from_fn(|k| (0..3).map(|j| a[k][j] * ow[j]).sum());
    let (lo, hi): ([f64; 3], [f64; 3]) = (
        std::array::from_fn(|k| pc[k] - pw[k] / 2.0),
        std::array::from_fn(|k| pc[k] + pw[k] / 2.0),
    );
    let sw: [f64; 3] = std::array::from_fn(|k| stored[1][k] - stored[0][k]);
    let size = sw.iter().copied().fold(0.0, f64::max);
    let tol = family_documents::BBOX_TOLERANCE.max(family_documents::SOLVED_BBOX_FRACTION * size);
    if (0..3).all(|k| (lo[k] - stored[0][k]).abs() <= tol && (hi[k] - stored[1][k]).abs() <= tol) {
        return NestedStretch::Fits;
    }
    // Revit's stored box holds all the instance draws, so only a body larger
    // than it is another size; one smaller can be lines and reference planes
    // the box also holds.
    if (0..3).any(|k| stored[0][k] < lo[k] - tol || stored[1][k] > hi[k] + tol) {
        return NestedStretch::Fits;
    }
    // Own widths x with |l| x = stored widths: an own axis the body is flat
    // along keeps its width, and a host axis the stored box is flat along
    // is not matched.
    let rows: Vec<usize> = (0..3).filter(|&k| sw[k] > tol).collect();
    let free: Vec<usize> = (0..3)
        .filter(|&j| ow[j] > tol && rows.iter().any(|&k| a[k][j] > 1e-6))
        .collect();
    let rhs: Vec<f64> = rows
        .iter()
        .map(|&k| {
            sw[k]
                - (0..3)
                    .filter(|j| !free.contains(j))
                    .map(|j| a[k][j] * ow[j])
                    .sum::<f64>()
        })
        .collect();
    let Some(x) = least_squares(
        &rows
            .iter()
            .map(|&k| free.iter().map(|&j| a[k][j]).collect())
            .collect::<Vec<Vec<f64>>>(),
        &rhs,
    ) else {
        return NestedStretch::Undetermined;
    };
    let mut factor = [1.0; 3];
    for (i, &j) in free.iter().enumerate() {
        factor[j] = x[i] / ow[j];
    }
    // The factors must give the stored box back.
    let fw: [f64; 3] = std::array::from_fn(|k| (0..3).map(|j| a[k][j] * ow[j] * factor[j]).sum());
    if rows.iter().any(|&k| (fw[k] - sw[k]).abs() > tol) {
        return NestedStretch::Undetermined;
    }
    // Within a few percent is the display's facets (a round form's stored
    // box is that of its faceted display), not another size.
    let near = NESTED_FACET_ALLOWANCE * size;
    if factor
        .iter()
        .all(|f| (f - 1.0).abs() <= NESTED_FACET_ALLOWANCE)
        && (0..3).all(|k| (lo[k] - stored[0][k]).abs() <= near.max(tol))
    {
        return NestedStretch::Fits;
    }
    // Only a shrink: a body larger than its box along one own axis and
    // smaller along another is another shape (the facet allowance decides
    // only whether to leave the whole body as it is, above).
    if !factor
        .iter()
        .all(|f| (NESTED_SCALE_LIMITS.0..=1.0 + NESTED_ROUNDOFF).contains(f))
    {
        return NestedStretch::BeyondLimits;
    }
    let sc: [f64; 3] = std::array::from_fn(|k| (stored[0][k] + stored[1][k]) / 2.0);
    for (v, o) in vertices.iter_mut().zip(&own) {
        let p: [f64; 3] = std::array::from_fn(|j| (o[j] - oc[j]) * factor[j]);
        *v = std::array::from_fn(|k| sc[k] + (0..3).map(|j| l[k][j] * p[j]).sum::<f64>());
    }
    NestedStretch::Stretched
}

/// The inverse of `l`, when it is not singular.
fn inverse3(l: &[[f64; 3]; 3]) -> Option<[[f64; 3]; 3]> {
    let d = det(l);
    if d.abs() < 1e-9 {
        return None;
    }
    let c = |r: usize, s: usize| {
        let (r0, r1) = ((r + 1) % 3, (r + 2) % 3);
        let (s0, s1) = ((s + 1) % 3, (s + 2) % 3);
        l[r0][s0] * l[r1][s1] - l[r0][s1] * l[r1][s0]
    };
    Some(std::array::from_fn(|i| {
        std::array::from_fn(|j| c(j, i) / d)
    }))
}

/// The `x` that best solves `m x = b` (rows of `m`), when it is well
/// determined: the normal matrix's determinant, for columns of up to unit
/// length, at least [`NESTED_TURN_DETERMINED`]. An empty `m` gives an empty
/// `x`.
fn least_squares(m: &[Vec<f64>], b: &[f64]) -> Option<Vec<f64>> {
    let n = m.first().map_or(0, Vec::len);
    if n == 0 {
        return Some(Vec::new());
    }
    let mut ata = [[0.0; 3]; 3];
    let mut atb = [0.0; 3];
    for (row, bk) in m.iter().zip(b) {
        for i in 0..n {
            atb[i] += row[i] * bk;
            for j in 0..n {
                ata[i][j] += row[i] * row[j];
            }
        }
    }
    for (i, r) in ata.iter_mut().enumerate().skip(n) {
        r[i] = 1.0;
    }
    if det(&ata) < NESTED_TURN_DETERMINED {
        return None;
    }
    let inv = inverse3(&ata)?;
    Some(
        (0..n)
            .map(|i| (0..n).map(|j| inv[i][j] * atb[j]).sum())
            .collect(),
    )
}

/// A linear map (rows) and a translation: `p -> l * p + t`.
pub type Affine = ([[f64; 3]; 3], [f64; 3]);

/// An instance's placement as a map from its family's coordinates. A
/// flipped instance mirrors its family across the family's own axis before
/// the transform (the matrix keeps no reflection: on a ring of chairs
/// measured, the six flipped in Y have the unflipped ones' matrices and face
/// them across the table). The matrix's columns are the instance's axes.
fn pose_affine(pose: &family_documents::InstancePose) -> Affine {
    let mirror = [
        if pose.flipped_x { -1.0 } else { 1.0 },
        if pose.flipped_y { -1.0 } else { 1.0 },
        1.0,
    ];
    let l = std::array::from_fn(|k| std::array::from_fn(|j| pose.matrix[k][j] * mirror[j]));
    (l, pose.origin)
}

/// `a` after `b`.
fn compose(a: &Affine, b: &Affine) -> Affine {
    let l = std::array::from_fn(|i| {
        std::array::from_fn(|j| (0..3).map(|k| a.0[i][k] * b.0[k][j]).sum())
    });
    let t = std::array::from_fn(|i| a.1[i] + (0..3).map(|k| a.0[i][k] * b.1[k]).sum::<f64>());
    (l, t)
}

fn det(l: &[[f64; 3]; 3]) -> f64 {
    l[0][0] * (l[1][1] * l[2][2] - l[1][2] * l[2][1])
        - l[0][1] * (l[1][0] * l[2][2] - l[1][2] * l[2][0])
        + l[0][2] * (l[1][0] * l[2][1] - l[1][1] * l[2][0])
}

/// The mesh of one nested instance: its family's decoded solids and those
/// of the families that family places in turn.
#[derive(Default)]
struct NestedBody {
    vertices: Vec<[f64; 3]>,
    triangles: Vec<BrepTriangle>,
    /// The largest solid (by volume) that names a material, and that material.
    material: Option<(f64, i64)>,
    /// A void was left uncut.
    uncut: bool,
    /// Undecoded forms drawn as their boxes.
    boxes: usize,
    /// Forms of the documents drawn that are not in the mesh, by why.
    missing: BTreeMap<String, usize>,
}

/// Deepest nesting followed.
const MAX_NESTING: usize = 8;

impl NestedBody {
    fn collect(
        &mut self,
        d: &family_documents::FamilyDocuments,
        guid: family_documents::Guid,
        map: Affine,
        depth: usize,
    ) {
        let Some(doc) = d.docs.get(&guid) else {
            *self.missing.entry("no_family_document".into()).or_default() += 1;
            return;
        };
        let reversed = det(&map.0) < 0.0;
        for solid in &doc.solids {
            let base = self.vertices.len() as u32;
            self.vertices
                .extend(solid.vertices.iter().map(|p| -> [f64; 3] {
                    std::array::from_fn(|k| {
                        map.1[k] + (0..3).map(|j| map.0[k][j] * p[j]).sum::<f64>()
                    })
                }));
            self.triangles.extend(solid.triangles.iter().map(|t| {
                if reversed {
                    BrepTriangle(base + t[0], base + t[2], base + t[1])
                } else {
                    BrepTriangle(base + t[0], base + t[1], base + t[2])
                }
            }));
            if let Some(m) = solid.material_id {
                let v = volume(solid);
                if self.material.is_none_or(|(w, _)| v > w) {
                    self.material = Some((v, m));
                }
            }
        }
        // Forms that did not decode, as their stored boxes (the file's own
        // undecoded forms are drawn so too).
        for b in &doc.unsolved_boxes {
            let SolidShape::TriangulatedFaceSet {
                vertices_feet,
                triangles,
                ..
            } = box_mesh(*b)
            else {
                continue;
            };
            let base = self.vertices.len() as u32;
            self.vertices
                .extend(vertices_feet.iter().map(|p| -> [f64; 3] {
                    std::array::from_fn(|k| {
                        map.1[k] + (0..3).map(|j| map.0[k][j] * p[j]).sum::<f64>()
                    })
                }));
            self.triangles.extend(triangles.iter().map(|t| {
                if reversed {
                    BrepTriangle(base + t.0, base + t.2, base + t.1)
                } else {
                    BrepTriangle(base + t.0, base + t.1, base + t.2)
                }
            }));
            self.boxes += 1;
        }
        let placed = d.doc_instances.get(&guid);
        for (why, n) in &doc.undecoded {
            match *why {
                // A void is left uncut, as the file's own forms are.
                "void" => self.uncut = true,
                // Its voids were cut from the solids above.
                "void_cut" => {}
                // Its nested families are placed below when their poses
                // are known; those without one are missing.
                "nested_family" if placed.is_some() => {
                    let short = n.saturating_sub(placed.map_or(0, |p| p.len()));
                    if short > 0 {
                        *self.missing.entry("nested_pose".into()).or_default() += short;
                    }
                }
                w => *self.missing.entry(w.to_string()).or_default() += n,
            }
        }
        if depth >= MAX_NESTING {
            return;
        }
        for pose in placed.into_iter().flat_map(|p| p.values()) {
            match d.document_of_symbol_in(guid, pose.symbol) {
                Some((sub, _)) => {
                    self.collect(d, sub, compose(&map, &pose_affine(pose)), depth + 1)
                }
                None => *self.missing.entry("no_family_document".into()).or_default() += 1,
            }
        }
    }
}

/// The volume a closed solid encloses.
fn volume(s: &family_documents::FamilySolid) -> f64 {
    s.triangles
        .iter()
        .map(|t| {
            let [a, b, c] = t.map(|i| s.vertices[i as usize]);
            a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                + a[2] * (b[0] * c[1] - b[1] * c[0])
        })
        .sum::<f64>()
        .abs()
        / 6.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn family_category_maps_to_its_ifc_class() {
        assert_eq!(ifc_type_for(Some("Furniture")).0, "IFCFURNITURE");
        assert_eq!(
            ifc_type_for(Some("Generic Models")).0,
            "IFCBUILDINGELEMENTPROXY"
        );
        assert_eq!(ifc_type_for(None).0, "IFCBUILDINGELEMENTPROXY");
    }

    #[test]
    fn summary_line_names_release_category_and_types() {
        let s = FamilySummary {
            release: Some(2027),
            category: Some("Furniture".into()),
            types: vec!["A".into(), "B".into()],
            default_type: Some("A".into()),
            ..Default::default()
        };
        assert_eq!(
            summary_line(&s),
            "Revit family, release 2027, category Furniture, types A, B (drawn: A)"
        );
    }

    /// A flipped instance inside a rotated one: the mirror comes first, and
    /// the map's determinant says to reverse the winding.
    #[test]
    fn nested_placements_compose_with_their_mirrors() {
        let pose = |matrix, origin, flipped_y| family_documents::InstancePose {
            symbol: 1,
            info_symbol: None,
            matrix,
            origin,
            flipped_x: false,
            flipped_y,
        };
        // A quarter turn about z at (10, 0, 0), then a Y flip at (0, 2, 0).
        let outer = pose_affine(&pose(
            [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            [10.0, 0.0, 0.0],
            false,
        ));
        let inner = pose_affine(&pose(
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            [0.0, 2.0, 0.0],
            true,
        ));
        let map = compose(&outer, &inner);
        let p = [1.0, 1.0, 0.0];
        let q: [f64; 3] =
            std::array::from_fn(|k| map.1[k] + (0..3).map(|j| map.0[k][j] * p[j]).sum::<f64>());
        // inner: (1, -1, 0) + (0, 2, 0) = (1, 1, 0); outer: turn to (-1, 1, 0), then + (10, 0, 0).
        assert_eq!(q, [9.0, 1.0, 0.0]);
        assert!(det(&map.0) < 0.0);
    }

    #[test]
    fn form_box_is_closed_and_outward() {
        let shape = box_mesh([[0.0; 3], [1.0, 2.0, 3.0]]);
        let SolidShape::TriangulatedFaceSet {
            vertices_feet,
            triangles,
            ..
        } = shape
        else {
            panic!("mesh");
        };
        // Signed volume of an outward-wound closed mesh is its volume.
        let v: f64 = triangles
            .iter()
            .map(|t| {
                let [a, b, c] = [t.0, t.1, t.2].map(|i| vertices_feet[i as usize]);
                (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                    + a[2] * (b[0] * c[1] - b[1] * c[0]))
                    / 6.0
            })
            .sum();
        assert!((v - 6.0).abs() < 1e-9, "volume {v}");
    }
}
