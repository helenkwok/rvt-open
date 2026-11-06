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
//! 2023) this pass draws them from their saved graphics too. Releases 2015 to
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
/// `BodySource` of a nested instance drawn from its saved graphics here.
pub const NESTED_SAVED_MESH_BODY_SOURCE: &str = "family_nested_saved_mesh";

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
    /// Instances of nested families, and how many this pass drew.
    pub nested_instances: usize,
    pub nested_drawn: usize,
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
            if class == "FamilyInstance" {
                nested.insert(id);
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
            forms.push(Form {
                id,
                kind,
                cutting: matches!(root.get("m_cutting"), Some(Value::Bool(true))),
                visible,
                material: id_of(root, "m_materialId").filter(|m| *m > 0),
            });
        },
    )?;
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

    // Decoded solids of the family's own document.
    let docs = family_documents::read_with_host(rf, true).ok();
    if let Some(d) = &docs {
        out.solve_failures = d.stats.solve_failures.clone();
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
            let reason = if !saved_graphics {
                "saved_graphics_off"
            } else if release < 2015 {
                "release_before_2015"
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
