//! Minimal STEP / ISO-10303-21 serializer for `IfcModel` → valid IFC4.
//!
//! This is the serialization half of the IFC export. Given an `IfcModel` in
//! memory, produce a .ifc text file that a spec-compliant reader
//! (IfcOpenShell, IFC viewers, buildingSMART validator) accepts.
//!
//! The output is intentionally minimal but **structurally valid**: a
//! well-formed IFC4 schema header, the required framework entities
//! (`IfcPerson` / `IfcOrganization` / `IfcApplication` /
//! `IfcOwnerHistory` / `IfcSIUnit` / `IfcUnitAssignment` /
//! `IfcGeometricRepresentationContext`), and an `IfcProject` populated
//! from the model's metadata. As the walker grows, `BuildingElement`
//! and family entities will land too; those extensions plug in here
//! without touching the header-level plumbing.
//!
//! Design principle: string-based emission, no external IFC library
//! dependency, fully `#![deny(unsafe_code)]`-clean.

use std::collections::{BTreeMap, HashMap};

use super::IfcModel;
use super::entities::{Extrusion, OpeningCut, PropertySet, SolidShape};

/// Options controlling STEP serialization.
#[derive(Debug, Clone, Default)]
pub struct StepOptions {
    /// If `Some`, use this Unix timestamp (in seconds) for both the
    /// `FILE_NAME` header and the `IfcOwnerHistory` creation time
    /// instead of `SystemTime::now()`. Setting this makes the
    /// output deterministic — identical `(IfcModel, StepOptions)`
    /// pairs produce byte-identical STEP strings, which makes
    /// STEP-text diffs tractable and regression tests reliable.
    pub timestamp: Option<i64>,
}

/// Serialize an `IfcModel` into an IFC4 STEP text stream. The output
/// includes the ISO-10303-21 envelope and a minimal but spec-valid
/// data section centred on `IfcProject`. Uses current wall-clock
/// timestamp. For deterministic output (e.g. tests), use
/// [`write_step_with_options`] with `StepOptions::timestamp = Some(_)`.
///
/// # Examples
///
/// ```
/// use rvt::RevitFile;
/// use rvt::ifc::{Exporter, RvtDocExporter, write_step};
///
/// let mut rf = RevitFile::open("corpus/tier1/architectural-2024/architectural-2024.rvt")?;
/// let ifc = write_step(&RvtDocExporter.export(&mut rf)?);
/// assert!(ifc.starts_with("ISO-10303-21;"));
/// # Ok::<(), rvt::Error>(())
/// ```
pub fn write_step(model: &IfcModel) -> String {
    write_step_with_options(model, &StepOptions::default())
}

/// The IFC4 STEP text of a graph: the [`IfcModel`] view derived from it
/// ([`super::IfcxGraph::to_model`]), written.
pub fn write_step_graph(graph: &super::IfcxGraph) -> String {
    write_step(&graph.to_model())
}

/// [`write_step_graph`] with options.
pub fn write_step_graph_with_options(graph: &super::IfcxGraph, options: &StepOptions) -> String {
    write_step_with_options(&graph.to_model(), options)
}

/// [`write_step_graph_with_options`], written to `out` as it is built
/// ([`write_step_to`]). Returns how many bytes were written.
///
/// # Errors
///
/// The first error `out` returns.
pub fn write_step_graph_to<W: std::io::Write>(
    graph: &super::IfcxGraph,
    options: &StepOptions,
    out: W,
) -> std::io::Result<u64> {
    write_step_to(&graph.to_model(), options, out)
}

/// Deterministic-option variant of [`write_step`].
///
/// When `options.timestamp = Some(t)`, the emitted STEP is a pure
/// function of `(model, t)` — no wall-clock access. Use this in
/// tests and regression fixtures.
pub fn write_step_with_options(model: &IfcModel, options: &StepOptions) -> String {
    write_step_string(model, options, false)
}

/// [`write_step_with_options`], also associating a material with every
/// `IfcSpace` that has one, as Revit's own export does (`rvt-ifc
/// --revit-set-placement`). IFC4's `IfcRelAssociatesMaterial.AllowedElements`
/// takes no `IfcSpace`, so this output breaks that rule, as Revit's does;
/// [`write_step_with_options`] leaves those associations out.
pub fn write_step_with_space_materials(model: &IfcModel, options: &StepOptions) -> String {
    write_step_string(model, options, true)
}

fn write_step_string(model: &IfcModel, options: &StepOptions, space_materials: bool) -> String {
    let now = options.timestamp.unwrap_or_else(unix_seconds);
    let mut w = StepWriter::new(now);
    w.space_materials = space_materials;
    w.emit_header(model);
    w.emit_data(model);
    w.finish()
}

/// [`write_step_with_options`], writing the text to `out` as it is built
/// instead of returning it, so a large model's text is never held whole in
/// memory. The bytes are the same. Returns how many were written.
/// Pass `&mut writer` to keep the writer.
///
/// # Errors
///
/// The first error `out` returns. What was written before it stays written.
pub fn write_step_to<W: std::io::Write>(
    model: &IfcModel,
    options: &StepOptions,
    out: W,
) -> std::io::Result<u64> {
    write_step_to_sink(model, options, false, out)
}

/// [`write_step_to`], also associating a material with every `IfcSpace` that
/// has one, as [`write_step_with_space_materials`] does.
///
/// # Errors
///
/// The first error `out` returns. What was written before it stays written.
pub fn write_step_to_with_space_materials<W: std::io::Write>(
    model: &IfcModel,
    options: &StepOptions,
    out: W,
) -> std::io::Result<u64> {
    write_step_to_sink(model, options, true, out)
}

fn write_step_to_sink<W: std::io::Write>(
    model: &IfcModel,
    options: &StepOptions,
    space_materials: bool,
    mut out: W,
) -> std::io::Result<u64> {
    let now = options.timestamp.unwrap_or_else(unix_seconds);
    let mut w = StepWriter::new(now);
    w.space_materials = space_materials;
    w.sink = Some(Sink {
        writer: &mut out,
        written: 0,
        error: None,
    });
    w.emit_header(model);
    w.emit_data(model);
    w.finish_to_sink()
}

/// The GlobalId [`write_step`] gives each building element, by index into
/// `model.entities`; `None` for an entity that is not a building element.
///
/// Fixed from what the element is rather than where it lands in the file:
/// Revit's own where the file yields it; otherwise one derived
/// under the model's GlobalId namespace from the document's identity, the element's ElementId (its `Tag`) and which
/// of the entities sharing that Tag it is, since a slab's further pieces
/// or a curtain wall's parts share their element's. An element with no
/// Tag falls back to its entity type and name. A caller that lists a model's
/// elements reads their GlobalIds here, so that they are the ones the written
/// IFC carries.
pub fn element_global_ids(model: &IfcModel) -> Vec<Option<String>> {
    building_element_global_ids(model)
}

/// The `Name` and `LongName` [`write_step`] gives a building element
///, by [`written_names`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct WrittenNames {
    /// The element's `Name`.
    pub name: String,
    /// True when `name` is the exporter's own `<Revit class>-<ElementId>`,
    /// kept for a space because neither a room number nor a room name was
    /// read: a placeholder made where no source named the element. It is
    /// only ever true for an `IfcSpace`; the other elements' names are not
    /// told apart from their own.
    pub name_is_placeholder: bool,
    /// An `IfcSpace`'s `LongName`, its room name; `None` for any other
    /// element and for a space whose name was not read.
    pub long_name: Option<String>,
}

/// The `Name` and `LongName` [`write_step`] writes for a building element of
/// `ifc_type` with the model name `name`, ElementId `type_guid` and record
/// property set `property_set`.
///
/// An `IfcSpace` is named by its room number and described by its room name,
/// as Revit's own exporter writes it: `Name` the number, `LongName`
/// the name. A room with a name and no number is named `<name>:<ElementId>`,
/// the name Revit's exporter falls back to; one with neither keeps
/// the model's own name, which is a placeholder (`name_is_placeholder`); one
/// with no name has no `LongName`. Any other element keeps its name.
pub fn written_names(
    ifc_type: &str,
    name: &str,
    type_guid: Option<&str>,
    property_set: Option<&PropertySet>,
) -> WrittenNames {
    use super::entities::PropertyValue;
    use super::export_content::{ROOM_NAME_PROPERTY, ROOM_NUMBER_PROPERTY};
    if !ifc_type.eq_ignore_ascii_case("IFCSPACE") {
        return WrittenNames {
            name: name.to_string(),
            name_is_placeholder: false,
            long_name: None,
        };
    }
    // A room's own text property, when recovered and not blank.
    let room_text = |key: &str| {
        property_set.and_then(|set| {
            set.properties
                .iter()
                .find(|p| p.name == key)
                .and_then(|p| match &p.value {
                    PropertyValue::Text(text) if !text.trim().is_empty() => Some(text.clone()),
                    _ => None,
                })
        })
    };
    let room_name = property_set.and_then(|set| {
        set.properties.iter().find_map(|p| match &p.value {
            PropertyValue::Text(text)
                if p.name == ROOM_NAME_PROPERTY && !text.trim().is_empty() =>
            {
                Some(text.as_str())
            }
            _ => None,
        })
    });
    let unnumbered = room_name
        .zip(type_guid)
        .map(|(room_name, tag)| format!("{room_name}:{tag}"));
    let named = room_text(ROOM_NUMBER_PROPERTY).or(unnumbered);
    // The model's own name for an element it read no name of is
    // `<class>-<ElementId>`, or `<class>-unnamed` without an ElementId.
    let made = name.rsplit_once('-').is_some_and(|(class, id)| {
        !class.is_empty()
            && class.bytes().all(|byte| byte.is_ascii_alphanumeric())
            && match type_guid {
                Some(tag) => id == tag,
                None => id == "unnamed",
            }
    });
    let name_is_placeholder = named.is_none() && made;
    WrittenNames {
        name: named.unwrap_or_else(|| name.to_string()),
        name_is_placeholder,
        long_name: room_text(ROOM_NAME_PROPERTY),
    }
}

/// Attribute layout of the tail an IFC4 building-element instance
/// carries after `IfcProduct`'s seven common attributes
/// (`GlobalId`, `OwnerHistory`, `Name`, `Description`, `ObjectType`,
/// `ObjectPlacement`, `Representation`).
///
/// IFC4 requires an instance to list **every** attribute its entity
/// type declares; an unknown optional occupies its slot as `$`.
/// Omitting the trailing ones entirely — which is what the writer did
/// before — yields a record that most readers tolerate but that
/// no spec-conformant consumer can index, so `IfcColumn.PredefinedType`
/// raised "index 8 is out of range for variant of size 8".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ElementTail {
    /// `Tag` only — the entity declares no `PredefinedType`
    /// (`IfcFurnishingElement`, `IfcFlowController`). Arity 8.
    Tag,
    /// `Tag, PredefinedType` — the shape almost every
    /// `IfcBuildingElement` subtype takes. Arity 9.
    TagPredefined,
    /// `Tag, OverallHeight, OverallWidth, PredefinedType,
    /// OperationType|PartitioningType, UserDefined…Type` —
    /// `IfcDoor` / `IfcWindow`. Arity 13.
    Opening,
    /// `LongName, CompositionType, PredefinedType,
    /// ElevationWithFlooring` — `IfcSpace`, which is an
    /// `IfcSpatialStructureElement` and so declares no `Tag` at all.
    /// Arity 11.
    Space,
    /// `Tag, SteelGrade, NominalDiameter, CrossSectionArea,
    /// BarLength, PredefinedType, BarSurface` —
    /// `IfcReinforcingBar`. Arity 14.
    ReinforcingBar,
    /// `Tag, NumberOfRisers, NumberOfTreads, RiserHeight,
    /// TreadLength, PredefinedType` — `IfcStairFlight`. Arity 13.
    StairFlight,
    /// `Tag, AssemblyPlace, PredefinedType` — `IfcElementAssembly`.
    /// Arity 10.
    ElementAssembly,
}

/// Which tail an emitted STEP entity name takes.
///
/// Every value here is the IFC4 EXPRESS declaration, cross-checked
/// against `ifcopenshell.ifcopenshell_wrapper.schema_by_name("IFC4")
/// .declaration_by_name(<name>).attribute_count()`; the same table is
/// re-derived from the schema by `tools/ci/ifc_schema_arity.py` on
/// every emitted file, so a wrong row cannot survive CI.
///
/// Unlisted names fall through to [`ElementTail::TagPredefined`],
/// which is the shape of every remaining `IfcBuildingElement`
/// subtype in IFC4.
fn element_tail_for(ifc_upper: &str) -> ElementTail {
    match ifc_upper {
        "IFCFURNISHINGELEMENT" | "IFCFLOWCONTROLLER" | "IFCFLOWTERMINAL" => ElementTail::Tag,
        "IFCDOOR" | "IFCWINDOW" => ElementTail::Opening,
        "IFCSPACE" => ElementTail::Space,
        "IFCREINFORCINGBAR" => ElementTail::ReinforcingBar,
        "IFCSTAIRFLIGHT" => ElementTail::StairFlight,
        "IFCELEMENTASSEMBLY" => ElementTail::ElementAssembly,
        _ => ElementTail::TagPredefined,
    }
}

/// IFC4's `IfcFlowDirectionEnum`, the values a port's `FlowDirection` is
/// written with.
const PORT_FLOW_DIRECTIONS: [&str; 4] = ["SOURCE", "SINK", "SOURCEANDSINK", "NOTDEFINED"];
/// IFC4's `IfcDistributionPortTypeEnum` but `USERDEFINED`, which needs an
/// `ObjectType` a port is written without: the values a port's
/// `PredefinedType` is written with.
const PORT_TYPES: [&str; 5] = ["CABLE", "CABLECARRIER", "DUCT", "PIPE", "NOTDEFINED"];

/// `raw` as a token of `allowed`, an IFC4 enumeration, or `None`.
fn step_enum_of(raw: &str, allowed: &[&str]) -> Option<String> {
    step_enum_token(raw).filter(|token| allowed.contains(&token.as_str()))
}

/// Accept a `PredefinedType` token only when it is a well-formed STEP
/// enumeration name: uppercase ASCII letters, digits and underscores,
/// starting with a letter.
///
/// An enumeration reference is written bare between dots, so unlike a
/// string it has no escape form — a caller-supplied value with a
/// quote, dot or newline in it would corrupt the record. Rejecting
/// writes `$` instead, which is always legal for an optional
/// attribute. (`tests/fuzz_regressions.rs` drives hostile names
/// through this path.)
fn step_enum_token(raw: &str) -> Option<String> {
    let token = raw.trim().to_ascii_uppercase();
    let mut chars = token.chars();
    match chars.next() {
        Some(c) if c.is_ascii_uppercase() => {}
        _ => return None,
    }
    if chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') {
        Some(token)
    } else {
        None
    }
}

/// the IFC occurrence class (and predefined type) a type of
/// `builtin_category` is written for when no placed type of its category
/// says (an unplaced type): the class the category's instances take by
/// default (a door type an `IfcDoorType`, a generic model a proxy type).
/// Not measured against Revit, which writes no unplaced type. `None` for a
/// category that is no building element (views, annotation, materials).
fn category_occurrence(builtin_category: i64) -> Option<(&'static str, Option<&'static str>)> {
    Some(match builtin_category {
        -2_000_011 => ("IFCWALL", None),
        -2_000_014 => ("IFCWINDOW", None),
        -2_000_023 => ("IFCDOOR", None),
        -2_000_032 => ("IFCSLAB", Some("FLOOR")),
        -2_000_035 => ("IFCROOF", None),
        -2_000_038 => ("IFCCOVERING", Some("CEILING")),
        -2_000_080 | -2_001_100 | -2_001_000 => ("IFCFURNITURE", None),
        -2_000_100 | -2_001_330 => ("IFCCOLUMN", None),
        -2_000_120 => ("IFCSTAIR", None),
        -2_000_919 => ("IFCSTAIRFLIGHT", None),
        -2_000_920 => ("IFCSLAB", Some("LANDING")),
        -2_000_126 => ("IFCRAILING", None),
        -2_000_170 => ("IFCPLATE", None),
        -2_000_171 => ("IFCMEMBER", None),
        -2_001_320 => ("IFCBEAM", None),
        -2_001_120 => ("IFCLIGHTFIXTURE", None),
        -2_001_160 => ("IFCSANITARYTERMINAL", None),
        -2_001_300 => ("IFCFOOTING", None),
        -2_008_000 | -2_008_020 => ("IFCDUCTSEGMENT", None),
        -2_008_010 => ("IFCDUCTFITTING", None),
        -2_008_013 => ("IFCAIRTERMINAL", None),
        -2_008_044 => ("IFCPIPESEGMENT", None),
        -2_008_049 => ("IFCPIPEFITTING", None),
        -2_008_130 => ("IFCCABLECARRIERSEGMENT", None),
        -2_008_126 => ("IFCCABLECARRIERFITTING", None),
        -2_008_132 => ("IFCCABLECARRIERSEGMENT", Some("CONDUITSEGMENT")),
        -2_008_128 => ("IFCCABLECARRIERFITTING", None),
        -2_008_099 => ("IFCFIRESUPPRESSIONTERMINAL", Some("SPRINKLER")),
        // Every other model category Revit's export writes as a proxy.
        -2_000_151 | -2_000_181 | -2_000_182 | -2_000_123 | -2_001_040 | -2_001_060
        | -2_001_140 | -2_001_180 | -2_001_263 | -2_001_350 | -2_001_354 | -2_001_360
        | -2_001_370 | -2_001_392 | -2_003_400 | -2_008_083 | -2_008_087 | -2_009_030
        // Pipe and duct accessories, balusters, food service equipment,
        | -2_008_055 | -2_008_016 | -2_000_127 | -2_001_043 | -2_001_036 | -2_000_948
        | -2_009_060 | -2_001_260 => {
            ("IFCBUILDINGELEMENTPROXY", None)
        }
        _ => return None,
    })
}

/// The IFC4 type entity an occurrence of `ifc_upper` is typed by, and the
/// attributes that entity declares after `ElementType`, with `predefined`
/// the occurrence's `PredefinedType`. An occurrence and its type
/// share one enumeration. `None` for entities rvt-open does not type.
fn type_entity_for(ifc_upper: &str, predefined: Option<&str>) -> Option<(String, String)> {
    let pt = format!(
        ".{}.",
        predefined
            .and_then(step_enum_token)
            .unwrap_or_else(|| "NOTDEFINED".into())
    );
    // Revit types a plumbing fixture it writes as `IfcFlowTerminal`
    // with `IfcDistributionElementType` (17 of 17 on Snowdon Towers Plumbing,
    // 11 on rme_advanced), which has no `PredefinedType`.
    if ifc_upper == "IFCFLOWTERMINAL" {
        return Some(("IFCDISTRIBUTIONELEMENTTYPE".into(), String::new()));
    }
    let tail = match ifc_upper {
        "IFCDOOR" | "IFCWINDOW" => format!("{pt},.NOTDEFINED.,$,$"),
        "IFCFURNITURE" => format!(".NOTDEFINED.,{pt}"),
        // a family instance written as a space (its "Export to
        // IFC As") is typed by its family type, which holds its maps, as in
        // Revit's export; `LongName` is not written. Rooms, areas and MEP
        // spaces have no type here (their own instance-as-type).
        "IFCSPACE" => format!("{pt},$"),
        "IFCWALL"
        | "IFCSLAB"
        | "IFCBEAM"
        | "IFCCOLUMN"
        | "IFCMEMBER"
        | "IFCPLATE"
        | "IFCRAILING"
        | "IFCCOVERING"
        | "IFCCURTAINWALL"
        | "IFCFOOTING"
        | "IFCRAMP"
        | "IFCROOF"
        | "IFCSTAIR"
        | "IFCSTAIRFLIGHT"
        | "IFCSHADINGDEVICE"
        | "IFCBUILDINGELEMENTPROXY"
        | "IFCAIRTERMINAL"
        | "IFCALARM"
        | "IFCCABLECARRIERFITTING"
        | "IFCCABLECARRIERSEGMENT"
        | "IFCFIRESUPPRESSIONTERMINAL"
        | "IFCVALVE"
        | "IFCWASTETERMINAL"
        | "IFCDUCTFITTING"
        | "IFCDUCTSEGMENT"
        | "IFCELECTRICAPPLIANCE"
        | "IFCLIGHTFIXTURE"
        | "IFCPIPEFITTING"
        | "IFCPIPESEGMENT"
        | "IFCSANITARYTERMINAL"
        | "IFCTRANSPORTELEMENT"
        | "IFCGEOGRAPHICELEMENT" => pt,
        _ => return None,
    };
    Some((format!("{ifc_upper}TYPE"), tail))
}

/// One IFC type object the writer makes, and the building elements it
/// types, by [`type_objects`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct TypeObject {
    /// The type object's IFC entity, such as `IFCWALLTYPE`.
    pub entity: String,
    /// Its `Name`.
    pub name: String,
    /// Its `Tag`: the ElementId of the type, or of the room an `IfcSpaceType`
    /// is made for.
    pub tag: String,
    /// The property sets it holds, in the order the writer writes them.
    pub property_sets: Vec<PropertySet>,
    /// The elements it types, as indices into `IfcModel::entities`.
    pub elements: Vec<usize>,
}

/// The type objects [`write_step`] makes for a model.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct TypeObjects {
    /// One per type, entity and predefined type of the elements named
    /// `Family:Type` whose type is read.
    pub groups: Vec<TypeObject>,
    /// One `IfcSpaceType` per room whose name was read.
    pub rooms: Vec<TypeObject>,
}

impl TypeObjects {
    /// Every type object, the groups and then the rooms'.
    pub fn all(&self) -> impl Iterator<Item = &TypeObject> {
        self.groups.iter().chain(self.rooms.iter())
    }
}

/// The type objects [`write_step`] makes for `model`: which elements are
/// typed by which, and the sets each holds. The writer writes exactly these
/// (the plan its STEP and IFCX writers share), so a reader of the model that wants an element's
/// type object asks here rather than grouping the elements again.
pub fn type_objects(model: &IfcModel) -> TypeObjects {
    let gids = building_element_global_ids(model);
    let mut out = TypeObjects::default();
    for plan in type_object_plan(model, &gids) {
        let object = TypeObject {
            entity: plan.entity,
            name: plan.name,
            tag: plan.tag,
            property_sets: plan.sets,
            elements: plan.elements,
        };
        if plan.space {
            out.rooms.push(object);
        } else {
            out.groups.push(object);
        }
    }
    out
}

/// The values of the `Pset_` sets the writer gives the type objects from
/// [`IfcModel::type_entity_sets`], for the diagnostics' parameter
/// value count: the sets of each type object the writer makes, which are the
/// groups of elements named `Family:Type` by type, entity, predefined type
/// and GlobalId (see the grouping in the writer, which this follows).
pub(super) fn type_object_pset_values(model: &IfcModel) -> usize {
    let mut groups: std::collections::BTreeSet<(u32, String, String, Option<String>)> =
        Default::default();
    let roof_slabs = super::roof_slab_wholes(model);
    for (entity_idx, entity) in model.entities.iter().enumerate() {
        let super::entities::IfcEntity::BuildingElement {
            type_guid,
            property_set,
            ..
        } = entity
        else {
            continue;
        };
        let Some((ifc_type, predefined_type)) = typed_as(model, &roof_slabs, entity_idx) else {
            continue;
        };
        let element = type_guid.as_deref().and_then(|tag| tag.parse::<u32>().ok());
        let Some(type_id) = element.and_then(|id| model.element_type_ids.get(&id)) else {
            continue;
        };
        let has_text = |key: &str| {
            property_set.as_ref().is_some_and(|set| {
                set.properties.iter().any(|property| {
                    matches!(property.value, super::entities::PropertyValue::Text(_))
                        && property.name == key
                })
            })
        };
        if !has_text(super::export_content::FAMILY_NAME_PROPERTY)
            || !has_text(super::export_content::TYPE_NAME_PROPERTY)
        {
            continue;
        }
        let Some((entity_name, tail)) =
            type_entity_for(&ifc_type.to_ascii_uppercase(), predefined_type.as_deref())
        else {
            continue;
        };
        let global_id = element
            .and_then(|id| model.global_ids.element_types.get(&id))
            .cloned();
        groups.insert((*type_id, entity_name, tail, global_id));
    }
    groups
        .iter()
        .filter_map(|(type_id, entity, _, _)| model.type_entity_sets.get(type_id)?.get(entity))
        .flatten()
        .filter(|set| set.name.starts_with("Pset_"))
        .map(|set| set.properties.len())
        .sum()
}

/// A port's element (index into the model's entities), its point (model
/// feet) and the direction its Z axis points along.
type PortPoint = (usize, [f64; 3], Option<[f64; 3]>);

/// Revit's IFC exporter's name for a type entity the writer emits (its `IFCEntityType`
/// enum), as its GUID keys spell it.
fn revit_ifc_type_entity_name(step_name: &str) -> Option<&'static str> {
    Some(match step_name {
        "IFCDOORTYPE" => "IfcDoorType",
        "IFCWINDOWTYPE" => "IfcWindowType",
        "IFCFURNITURETYPE" => "IfcFurnitureType",
        "IFCSPACETYPE" => "IfcSpaceType",
        "IFCWALLTYPE" => "IfcWallType",
        "IFCSLABTYPE" => "IfcSlabType",
        "IFCBEAMTYPE" => "IfcBeamType",
        "IFCCOLUMNTYPE" => "IfcColumnType",
        "IFCMEMBERTYPE" => "IfcMemberType",
        "IFCPLATETYPE" => "IfcPlateType",
        "IFCRAILINGTYPE" => "IfcRailingType",
        "IFCCOVERINGTYPE" => "IfcCoveringType",
        "IFCCURTAINWALLTYPE" => "IfcCurtainWallType",
        "IFCFOOTINGTYPE" => "IfcFootingType",
        "IFCRAMPTYPE" => "IfcRampType",
        "IFCROOFTYPE" => "IfcRoofType",
        "IFCSTAIRTYPE" => "IfcStairType",
        "IFCSTAIRFLIGHTTYPE" => "IfcStairFlightType",
        "IFCSHADINGDEVICETYPE" => "IfcShadingDeviceType",
        "IFCGEOGRAPHICELEMENTTYPE" => "IfcGeographicElementType",
        "IFCBUILDINGELEMENTPROXYTYPE" => "IfcBuildingElementProxyType",
        "IFCAIRTERMINALTYPE" => "IfcAirTerminalType",
        "IFCALARMTYPE" => "IfcAlarmType",
        "IFCCABLECARRIERFITTINGTYPE" => "IfcCableCarrierFittingType",
        "IFCDISTRIBUTIONELEMENTTYPE" => "IfcDistributionElementType",
        "IFCCABLECARRIERSEGMENTTYPE" => "IfcCableCarrierSegmentType",
        "IFCFIRESUPPRESSIONTERMINALTYPE" => "IfcFireSuppressionTerminalType",
        "IFCVALVETYPE" => "IfcValveType",
        "IFCWASTETERMINALTYPE" => "IfcWasteTerminalType",
        "IFCDUCTFITTINGTYPE" => "IfcDuctFittingType",
        "IFCDUCTSEGMENTTYPE" => "IfcDuctSegmentType",
        "IFCELECTRICAPPLIANCETYPE" => "IfcElectricApplianceType",
        "IFCLIGHTFIXTURETYPE" => "IfcLightFixtureType",
        "IFCPIPEFITTINGTYPE" => "IfcPipeFittingType",
        "IFCPIPESEGMENTTYPE" => "IfcPipeSegmentType",
        "IFCSANITARYTERMINALTYPE" => "IfcSanitaryTerminalType",
        "IFCTRANSPORTELEMENTTYPE" => "IfcTransportElementType",
        _ => return None,
    })
}

/// Render the attribute tail for one building element.
///
/// `tag_quoted` and `name_quoted` are already STEP-quoted (`'…'` or
/// `$`). `predefined` is the bare enum token (`"COLUMN"`), written as
/// `.COLUMN.`; `None` writes `$` — the writer never invents an enum
/// value it did not decode.
fn element_attribute_tail(
    ifc_upper: &str,
    tag_quoted: &str,
    predefined: Option<&str>,
    long_name_quoted: &str,
    overall_height_width_feet: Option<(f64, f64)>,
) -> String {
    let pt = match predefined.and_then(step_enum_token) {
        Some(token) => format!(".{token}."),
        None => "$".to_string(),
    };
    match element_tail_for(ifc_upper) {
        ElementTail::Tag => tag_quoted.to_string(),
        ElementTail::TagPredefined => format!("{tag_quoted},{pt}"),
        // OverallHeight / OverallWidth are the type's, where read,
        // and otherwise `$`.
        ElementTail::Opening => match overall_height_width_feet {
            Some((height, width)) => format!(
                "{tag_quoted},{:.6},{:.6},{pt},$,$",
                height * 0.3048,
                width * 0.3048
            ),
            None => format!("{tag_quoted},$,$,{pt},$,$"),
        },
        // IfcSpace has no Tag attribute — slot 8 is LongName. Before
        // the writer put the type GUID there, which typed as a
        // label but meant the wrong thing; the element's own Name
        // carries the identity instead. CompositionType `.ELEMENT.`
        // matches what Revit's exporter writes for a room.
        //
        // `long_name_quoted` is the room's name from its `RoomName`
        // property, the slot Revit's own exporter puts it in, or `$`.
        // The element Name is its room number.
        ElementTail::Space => format!("{long_name_quoted},.ELEMENT.,{pt},$"),
        ElementTail::ReinforcingBar => format!("{tag_quoted},$,$,$,$,{pt},$"),
        ElementTail::StairFlight => format!("{tag_quoted},$,$,$,$,{pt}"),
        ElementTail::ElementAssembly => format!("{tag_quoted},.NOTDEFINED.,{pt}"),
    }
}

struct StepWriter<'a> {
    out: String,
    /// Where [`write_step_to`] sends the text: `out` is handed on whenever it
    /// passes [`SPILL_BYTES`].
    sink: Option<Sink<'a>>,
    next_id: usize,
    /// Unix timestamp (seconds) used for FILE_NAME + IfcOwnerHistory.
    /// Injected at construction so the output is a pure function of
    /// `(model, timestamp)` when a fixed timestamp is supplied.
    timestamp: i64,
    /// each model material's sRGB colour (`0x00BBGGRR`), for
    /// face-coloured triangle meshes (`IfcIndexedColourMap`).
    material_colours: Vec<Option<u32>>,
    /// Whether an `IfcSpace` keeps its material association, which IFC4's
    /// `IfcRelAssociatesMaterial.AllowedElements` does not allow and Revit's
    /// export writes: [`write_step_with_space_materials`].
    space_materials: bool,
}

/// A writer [`write_step_to`] streams to, the bytes it took and its first
/// error.
struct Sink<'a> {
    writer: &'a mut dyn std::io::Write,
    written: u64,
    error: Option<std::io::Error>,
}

/// How much text [`StepWriter`] holds before handing it to its sink.
const SPILL_BYTES: usize = 1 << 20;

/// The `IfcShapeRepresentation.RepresentationType` IFC4's
/// `IfcShapeRepresentationTypes` accepts for items of every kind in `kinds`
/// (the tokens `emit_solid_shape` returns), or `None` where there are none or
/// no one type holds them all:
/// - `AdvancedSweptSolid` takes every `IfcSweptAreaSolid`, extruded and
///   revolved ones too;
/// - `SolidModel` takes those and `IfcFacetedBrep`;
/// - `SurfaceOrSolidModel` takes those and `IfcTriangulatedFaceSet`;
/// - `CSG` takes `IfcBooleanResult` alone, and no other type takes one; a mix
///   with a boolean result is written as the union of its items, a
///   triangulated one as a faceted brep.
fn shared_representation_type(
    kinds: &std::collections::BTreeSet<&'static str>,
) -> Option<&'static str> {
    let within = |allowed: &[&str]| kinds.iter().all(|kind| allowed.contains(kind));
    if kinds.len() <= 1 {
        kinds.iter().next().copied()
    } else if within(&["SweptSolid", "AdvancedSweptSolid"]) {
        Some("AdvancedSweptSolid")
    } else if within(&["SweptSolid", "AdvancedSweptSolid", "Brep"]) {
        Some("SolidModel")
    } else if within(&["SweptSolid", "AdvancedSweptSolid", "Brep", "Tessellation"]) {
        Some("SurfaceOrSolidModel")
    } else {
        None
    }
}

/// Whether the default export associates a material with an element of IFC
/// type `ifc_type`. An `IfcSpace` is none of the entities IFC4's
/// `IfcRelAssociatesMaterial.AllowedElements` takes (elements, element types,
/// door and window styles, structural members and ports), so it has none,
/// though Revit's own export associates one, which
/// [`write_step_with_space_materials`] also does. The consumers that list an
/// element's material (the elements CSV, the viewer's panels) follow this, as
/// they follow `placements`.
pub(super) fn associates_material(ifc_type: &str) -> bool {
    !ifc_type.eq_ignore_ascii_case("IFCSPACE")
}

/// The token `emit_solid_shape` returns for `shape`, worked out before it is
/// written, so that a compound whose items have no shared type is written
/// with its items as union operands.
fn solid_kind(shape: &SolidShape) -> &'static str {
    match shape {
        SolidShape::ExtrudedArea(_)
        | SolidShape::RevolvedArea { .. }
        | SolidShape::PlacedExtrusion { .. } => "SweptSolid",
        SolidShape::SweptPath { .. } => "AdvancedSweptSolid",
        SolidShape::FacetedBrep { .. } => "Brep",
        SolidShape::TriangulatedFaceSet { .. } => "Tessellation",
        SolidShape::BooleanResult { .. } => "CSG",
        SolidShape::Compound(items) => match items.as_slice() {
            [] => "Tessellation",
            [one] => solid_kind(one),
            _ => "CSG",
        },
    }
}

impl StepWriter<'_> {
    /// The `Position` and depth, metres, of an opening's extruded solid: the
    /// shared `solid_position` and the body's `depth_m`, or, for a cut with
    /// its own height, a placement that far above the element's and
    /// that height.
    /// A port's `IfcLocalPlacement` at `point_feet` (model feet), its Z axis
    /// along `direction` (model axes) where given, in the frame of its
    /// element's placement `parent`, which sits at `element_location` turned
    /// by `rotation` about Z.
    fn port_placement(
        &mut self,
        parent: usize,
        element_location: Option<[f64; 3]>,
        rotation: Option<f64>,
        point_feet: [f64; 3],
        direction: Option<[f64; 3]>,
    ) -> usize {
        let [ox, oy, oz] = element_location.unwrap_or([0.0; 3]);
        let (dx, dy, dz) = (point_feet[0] - ox, point_feet[1] - oy, point_feet[2] - oz);
        let angle = rotation.unwrap_or(0.0);
        let (c, s) = (angle.cos(), angle.sin());
        let local = [c * dx + s * dy, -s * dx + c * dy, dz];
        let point = self.id();
        self.emit_entity(
            point,
            format!(
                "IFCCARTESIANPOINT(({:.6},{:.6},{:.6}))",
                local[0] * 0.3048,
                local[1] * 0.3048,
                local[2] * 0.3048
            ),
        );
        // The Z axis along `direction`, and a reference direction across
        // it: up for a port that does not point up or down, else +X. Nothing
        // read gives the port's X axis, and IFC's default (+X) leaves none
        // for a Z axis along -X.
        let axes = direction.map(|[x, y, z]| {
            let local = [c * x + s * y, -s * x + c * y, z];
            let z_id = self.id();
            self.emit_entity(
                z_id,
                format!(
                    "IFCDIRECTION(({:.6},{:.6},{:.6}))",
                    local[0], local[1], local[2]
                ),
            );
            let reference = if local[2].abs() < 0.9 {
                "IFCDIRECTION((0.,0.,1.))"
            } else {
                "IFCDIRECTION((1.,0.,0.))"
            };
            let x_id = self.id();
            self.emit_entity(x_id, reference);
            (z_id, x_id)
        });
        let axis = self.id();
        self.emit_entity(
            axis,
            match axes {
                Some((z_id, x_id)) => format!("IFCAXIS2PLACEMENT3D(#{point},#{z_id},#{x_id})"),
                None => format!("IFCAXIS2PLACEMENT3D(#{point},$,$)"),
            },
        );
        let placement = self.id();
        self.emit_entity(placement, format!("IFCLOCALPLACEMENT(#{parent},#{axis})"));
        placement
    }

    fn opening_solid_position(
        &mut self,
        solid_position: usize,
        cut: Option<&OpeningCut>,
        depth_m: f64,
    ) -> (usize, f64) {
        let Some([z, height]) = cut.and_then(|cut| cut.z_range_feet) else {
            return (solid_position, depth_m);
        };
        let point = self.id();
        self.emit_entity(
            point,
            format!("IFCCARTESIANPOINT((0.,0.,{:.6}))", z * 0.3048),
        );
        let position = self.id();
        self.emit_entity(position, format!("IFCAXIS2PLACEMENT3D(#{point},$,$)"));
        (position, height * 0.3048)
    }

    fn new(timestamp: i64) -> Self {
        Self {
            out: String::new(),
            sink: None,
            next_id: 1,
            timestamp,
            material_colours: Vec::new(),
            space_materials: false,
        }
    }

    /// `IfcProjectedCRS` + `IfcMapConversion` for a model whose
    /// CRS the file names: the internal origin's shared coordinates and the
    /// internal X axis in map coordinates, in the CRS's unit (CS-MAP `FOOT`
    /// is the US survey foot; Revit's shared-coordinate feet are read as it).
    fn map_conversion(&mut self, g: &crate::georeference::Georeference, context: usize) {
        let Some(crs) = &g.crs else {
            return;
        };
        let dims = self.id();
        self.emit_entity(dims, "IFCDIMENSIONALEXPONENTS(1,0,0,0,0,0,0)");
        let metre = self.id();
        self.emit_entity(metre, "IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.)");
        let unit = if crs.in_feet() {
            let measure = self.id();
            self.emit_entity(
                measure,
                format!("IFCMEASUREWITHUNIT(IFCLENGTHMEASURE(0.3048006096012192),#{metre})"),
            );
            let foot = self.id();
            self.emit_entity(
                foot,
                format!("IFCCONVERSIONBASEDUNIT(#{dims},.LENGTHUNIT.,'US SURVEY FOOT',#{measure})"),
            );
            foot
        } else {
            metre
        };
        let name = crs
            .epsg
            .map_or_else(|| crs.name.clone(), |e| format!("EPSG:{e}"));
        let crs_id = self.id();
        self.emit_entity(
            crs_id,
            format!(
                "IFCPROJECTEDCRS('{}',{},$,$,$,$,#{unit})",
                escape(&name),
                quoted_or_dollar(&escape(crs.description.as_deref().unwrap_or(&crs.name)))
            ),
        );
        let (sin, cos) = g.true_north.to_radians().sin_cos();
        let [e, n, h] = g.origin_shared;
        let id = self.id();
        self.emit_entity(
            id,
            format!(
                "IFCMAPCONVERSION(#{context},#{crs_id},{},{},{},{},{},$)",
                format_real(crs.map_value(e)),
                format_real(crs.map_value(n)),
                format_real(crs.map_value(h)),
                format_real(cos),
                format_real(-sin)
            ),
        );
    }

    fn id(&mut self) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn emit_line<S: AsRef<str>>(&mut self, line: S) {
        self.out.push_str(line.as_ref());
        self.out.push('\n');
        if self.out.len() >= SPILL_BYTES {
            self.spill();
        }
    }

    fn emit_entity<S: AsRef<str>>(&mut self, id: usize, body: S) {
        self.out.push_str(&format!("#{id}={};\n", body.as_ref()));
        if self.out.len() >= SPILL_BYTES {
            self.spill();
        }
    }

    /// Hand the text held so far to the sink, if there is one; after the
    /// sink's first error the text is dropped unwritten.
    fn spill(&mut self) {
        if let Some(sink) = &mut self.sink {
            if sink.error.is_none() {
                match sink.writer.write_all(self.out.as_bytes()) {
                    Ok(()) => sink.written += self.out.len() as u64,
                    Err(error) => sink.error = Some(error),
                }
            }
            self.out.clear();
        }
    }

    /// Emit `pset` as one `IfcPropertySingleValue` per property, the
    /// `IfcPropertySet` holding them (GlobalId `set_gid`) and the
    /// `IfcRelDefinesByProperties` (GlobalId `rel_gid`) relating it to
    /// `object`.
    /// Emit a property set and its properties, with no relationship: the
    /// set id, for a type object's `HasPropertySets`.
    fn emit_bare_property_set(
        &mut self,
        owner_hist: usize,
        pset: &PropertySet,
        set_gid: &str,
    ) -> usize {
        if pset.name.starts_with("Qto_") {
            return self.emit_element_quantity(owner_hist, pset, set_gid);
        }
        let mut prop_ids: Vec<usize> = Vec::with_capacity(pset.properties.len());
        for prop in &pset.properties {
            let p_id = self.id();
            let name_esc = escape(&prop.name);
            let value_step = prop.value.to_step();
            let entity = match prop.value {
                super::entities::PropertyValue::List(_) => "IFCPROPERTYLISTVALUE",
                super::entities::PropertyValue::Enumerated(_) => "IFCPROPERTYENUMERATEDVALUE",
                _ => "IFCPROPERTYSINGLEVALUE",
            };
            self.emit_entity(p_id, format!("{entity}('{name_esc}',$,{value_step},$)"));
            prop_ids.push(p_id);
        }
        let refs = prop_ids
            .iter()
            .map(|id| format!("#{id}"))
            .collect::<Vec<_>>()
            .join(",");
        let set_id = self.id();
        let set_name = escape(&pset.name);
        self.emit_entity(
            set_id,
            format!("IFCPROPERTYSET('{set_gid}',#{owner_hist},'{set_name}',$,({refs}))"),
        );
        set_id
    }

    /// A `Qto_` set as IFC4's `IfcElementQuantity`: each value that is
    /// a quantity ([`super::entities::PropertyValue::quantity`]) as its simple
    /// quantity, any other left out. [`Self::emit_property_set`] writes no set
    /// that has none.
    fn emit_element_quantity(
        &mut self,
        owner_hist: usize,
        pset: &PropertySet,
        set_gid: &str,
    ) -> usize {
        let mut quantity_ids: Vec<usize> = Vec::with_capacity(pset.properties.len());
        for prop in &pset.properties {
            // a layer's quantities as an IfcPhysicalComplexQuantity.
            if let super::entities::PropertyValue::PhysicalComplex {
                discrimination,
                quantities,
            } = &prop.value
            {
                let parts: Vec<String> = quantities
                    .iter()
                    .filter_map(|part| {
                        let (entity, value) = part.value.quantity()?;
                        let q_id = self.id();
                        let name_esc = escape(&part.name);
                        self.emit_entity(q_id, format!("{entity}('{name_esc}',$,$,{value},$)"));
                        Some(format!("#{q_id}"))
                    })
                    .collect();
                if parts.is_empty() {
                    continue;
                }
                let q_id = self.id();
                self.emit_entity(
                    q_id,
                    format!(
                        "IFCPHYSICALCOMPLEXQUANTITY('{}',$,({}),'{}',$,$)",
                        escape(&prop.name),
                        parts.join(","),
                        escape(discrimination)
                    ),
                );
                quantity_ids.push(q_id);
                continue;
            }
            let Some((entity, value)) = prop.value.quantity() else {
                continue;
            };
            let q_id = self.id();
            let name_esc = escape(&prop.name);
            self.emit_entity(q_id, format!("{entity}('{name_esc}',$,$,{value},$)"));
            quantity_ids.push(q_id);
        }
        let refs = quantity_ids
            .iter()
            .map(|id| format!("#{id}"))
            .collect::<Vec<_>>()
            .join(",");
        let set_id = self.id();
        let set_name = escape(&pset.name);
        self.emit_entity(
            set_id,
            format!("IFCELEMENTQUANTITY('{set_gid}',#{owner_hist},'{set_name}',$,$,({refs}))"),
        );
        set_id
    }

    fn emit_property_set(
        &mut self,
        owner_hist: usize,
        object: usize,
        pset: &PropertySet,
        set_gid: &str,
        rel_gid: &str,
    ) {
        // An IfcElementQuantity holds a quantity at least.
        if pset.name.starts_with("Qto_")
            && pset.properties.iter().all(|prop| {
                prop.value.quantity().is_none()
                    && !matches!(
                        prop.value,
                        super::entities::PropertyValue::PhysicalComplex { .. }
                    )
            })
        {
            return;
        }
        let set_id = self.emit_bare_property_set(owner_hist, pset, set_gid);
        let rel_id = self.id();
        self.emit_entity(
            rel_id,
            format!(
                "IFCRELDEFINESBYPROPERTIES('{rel_gid}',#{owner_hist},$,$,(#{object}),#{set_id})"
            ),
        );
    }

    /// Emit one `IfcProfileDef` subclass from an [`Extrusion`],
    /// dispatching on [`Extrusion::profile_override`]. Returns the
    /// entity ID of the emitted profile so the caller can wire it
    /// into an `IFCEXTRUDEDAREASOLID`.
    ///
    /// - `ex.profile_override = None` emits the default
    ///   `IFCRECTANGLEPROFILEDEF` from `width_feet` × `depth_feet`.
    /// - `Circle { radius }` emits `IFCCIRCLEPROFILEDEF`.
    /// - `IShape { … }` emits `IFCIShapeProfileDef` with a full
    ///   OverallWidth / OverallDepth / WebThickness /
    ///   FlangeThickness attribute set (fillet radius = $).
    /// - `TShape { … }` emits `IFCTShapeProfileDef`.
    /// - `LShape { … }` emits `IFCLShapeProfileDef` (EdgeRadius,
    ///   LegSlope = $).
    /// - `UShape { … }` emits `IFCUShapeProfileDef`.
    /// - `RectangleHollow { … }` emits
    ///   `IFCRectangleHollowProfileDef`.
    /// - `CircleHollow { … }` emits `IFCCircleHollowProfileDef`.
    /// - `ArbitraryClosed { points }` emits `IFCPOLYLINE` +
    ///   `IFCArbitraryClosedProfileDef`; if the polyline isn't
    ///   already closed (last == first), the writer appends the
    ///   first point at the tail.
    /// - `ArbitraryWithVoids { points, voids }` emits one
    ///   `IFCPOLYLINE` for the outer ring and one per hole, then
    ///   `IFCArbitraryProfileDefWithVoids`.
    ///
    /// All length values are converted from feet to metres at emit
    /// time (factor 0.3048).
    fn emit_profile_def(&mut self, ex: &Extrusion, profile_placement: usize) -> usize {
        use super::entities::ProfileDef;

        let profile_id = self.id();
        match &ex.profile_override {
            None | Some(ProfileDef::Rectangle { .. }) => {
                let (w_ft, d_ft) = match ex.profile_override {
                    Some(ProfileDef::Rectangle {
                        width_feet,
                        depth_feet,
                    }) => (width_feet, depth_feet),
                    _ => (ex.width_feet, ex.depth_feet),
                };
                let x_dim = w_ft * 0.3048;
                let y_dim = d_ft * 0.3048;
                self.emit_entity(
                    profile_id,
                    format!(
                        "IFCRECTANGLEPROFILEDEF(.AREA.,$,#{profile_placement},{x_dim:.6},{y_dim:.6})"
                    ),
                );
            }
            Some(ProfileDef::Circle { radius_feet }) => {
                let r = radius_feet * 0.3048;
                self.emit_entity(
                    profile_id,
                    format!("IFCCIRCLEPROFILEDEF(.AREA.,$,#{profile_placement},{r:.6})"),
                );
            }
            Some(ProfileDef::IShape {
                overall_width_feet,
                overall_depth_feet,
                web_thickness_feet,
                flange_thickness_feet,
            }) => {
                let w = overall_width_feet * 0.3048;
                let d = overall_depth_feet * 0.3048;
                let tw = web_thickness_feet * 0.3048;
                let tf = flange_thickness_feet * 0.3048;
                // IFCIShapeProfileDef(ProfileType, ProfileName,
                // Position, OverallWidth, OverallDepth,
                // WebThickness, FlangeThickness, FilletRadius?,
                // FlangeEdgeRadius?, FlangeSlope?) — ten attributes
                // in IFC4; the last two are IFC4 additions over
                // IFC2X3 and must still occupy their slots.
                self.emit_entity(
                    profile_id,
                    format!(
                        "IFCISHAPEPROFILEDEF(.AREA.,$,#{profile_placement},{w:.6},{d:.6},{tw:.6},{tf:.6},$,$,$)"
                    ),
                );
            }
            Some(ProfileDef::TShape {
                overall_depth_feet,
                flange_width_feet,
                web_thickness_feet,
                flange_thickness_feet,
            }) => {
                let d = overall_depth_feet * 0.3048;
                let fw = flange_width_feet * 0.3048;
                let tw = web_thickness_feet * 0.3048;
                let tf = flange_thickness_feet * 0.3048;
                // IFCTShapeProfileDef(ProfileType, ProfileName,
                // Position, Depth, FlangeWidth, WebThickness,
                // FlangeThickness, FilletRadius?, FlangeEdgeRadius?,
                // WebEdgeRadius?, WebSlope?, FlangeSlope?).
                self.emit_entity(
                    profile_id,
                    format!(
                        "IFCTSHAPEPROFILEDEF(.AREA.,$,#{profile_placement},{d:.6},{fw:.6},{tw:.6},{tf:.6},$,$,$,$,$)"
                    ),
                );
            }
            Some(ProfileDef::LShape {
                overall_depth_feet,
                overall_width_feet,
                thickness_feet,
            }) => {
                let d = overall_depth_feet * 0.3048;
                let w = overall_width_feet * 0.3048;
                let t = thickness_feet * 0.3048;
                // IFCLShapeProfileDef(ProfileType, ProfileName,
                // Position, Depth, Width, Thickness, FilletRadius?,
                // EdgeRadius?, LegSlope?).
                self.emit_entity(
                    profile_id,
                    format!(
                        "IFCLSHAPEPROFILEDEF(.AREA.,$,#{profile_placement},{d:.6},{w:.6},{t:.6},$,$,$)"
                    ),
                );
            }
            Some(ProfileDef::UShape {
                overall_depth_feet,
                flange_width_feet,
                web_thickness_feet,
                flange_thickness_feet,
            }) => {
                let d = overall_depth_feet * 0.3048;
                let fw = flange_width_feet * 0.3048;
                let tw = web_thickness_feet * 0.3048;
                let tf = flange_thickness_feet * 0.3048;
                // IFCUShapeProfileDef(ProfileType, ProfileName,
                // Position, Depth, FlangeWidth, WebThickness,
                // FlangeThickness, FilletRadius?,
                // EdgeRadius?, FlangeSlope?).
                self.emit_entity(
                    profile_id,
                    format!(
                        "IFCUSHAPEPROFILEDEF(.AREA.,$,#{profile_placement},{d:.6},{fw:.6},{tw:.6},{tf:.6},$,$,$)"
                    ),
                );
            }
            Some(ProfileDef::RectangleHollow {
                overall_width_feet,
                overall_depth_feet,
                wall_thickness_feet,
            }) => {
                let w = overall_width_feet * 0.3048;
                let d = overall_depth_feet * 0.3048;
                let t = wall_thickness_feet * 0.3048;
                // IFCRectangleHollowProfileDef(ProfileType, ProfileName,
                // Position, XDim, YDim, WallThickness,
                // InnerFilletRadius?, OuterFilletRadius?).
                self.emit_entity(
                    profile_id,
                    format!(
                        "IFCRECTANGLEHOLLOWPROFILEDEF(.AREA.,$,#{profile_placement},{w:.6},{d:.6},{t:.6},$,$)"
                    ),
                );
            }
            Some(ProfileDef::CircleHollow {
                radius_feet,
                wall_thickness_feet,
            }) => {
                let r = radius_feet * 0.3048;
                let t = wall_thickness_feet * 0.3048;
                // IFCCircleHollowProfileDef(ProfileType, ProfileName,
                // Position, Radius, WallThickness).
                self.emit_entity(
                    profile_id,
                    format!(
                        "IFCCIRCLEHOLLOWPROFILEDEF(.AREA.,$,#{profile_placement},{r:.6},{t:.6})"
                    ),
                );
            }
            Some(ProfileDef::ArbitraryClosed { points }) => {
                // Build the polyline by emitting one IFCCARTESIANPOINT
                // per vertex, then an IFCPOLYLINE that references them
                // as its Points list. Auto-close by appending the first
                // point if the last doesn't already equal it.
                let mut pts: Vec<(f64, f64)> = points
                    .iter()
                    .map(|(x, y)| (*x * 0.3048, *y * 0.3048))
                    .collect();
                if let (Some(first), Some(last)) = (pts.first().copied(), pts.last().copied()) {
                    if (first.0 - last.0).abs().max((first.1 - last.1).abs()) > 1e-9_f64 {
                        pts.push(first);
                    }
                }
                let mut point_ids: Vec<usize> = Vec::with_capacity(pts.len());
                for (x, y) in &pts {
                    let pt_id = self.id();
                    self.emit_entity(pt_id, format!("IFCCARTESIANPOINT(({x:.6},{y:.6}))"));
                    point_ids.push(pt_id);
                }
                let polyline_id = self.id();
                let refs = point_ids
                    .iter()
                    .map(|id| format!("#{id}"))
                    .collect::<Vec<_>>()
                    .join(",");
                self.emit_entity(polyline_id, format!("IFCPOLYLINE(({refs}))"));
                // IFCArbitraryClosedProfileDef(ProfileType, ProfileName,
                // OuterCurve).
                self.emit_entity(
                    profile_id,
                    format!("IFCARBITRARYCLOSEDPROFILEDEF(.AREA.,$,#{polyline_id})"),
                );
            }
            Some(ProfileDef::IndexedPolyCurve { points, segments }) => {
                let curve_id = self.emit_indexed_poly_curve(points, segments);
                self.emit_entity(
                    profile_id,
                    format!("IFCARBITRARYCLOSEDPROFILEDEF(.AREA.,$,#{curve_id})"),
                );
            }
            Some(ProfileDef::IndexedPolyCurveWithVoids { outer, voids }) => {
                let outer_id = self.emit_indexed_poly_curve(&outer.points, &outer.segments);
                let inner_refs = voids
                    .iter()
                    .map(|void| {
                        format!(
                            "#{}",
                            self.emit_indexed_poly_curve(&void.points, &void.segments)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                self.emit_entity(
                    profile_id,
                    format!("IFCARBITRARYPROFILEDEFWITHVOIDS(.AREA.,$,#{outer_id},({inner_refs}))"),
                );
            }
            Some(ProfileDef::ArbitraryWithVoids { points, voids }) => {
                let outer_id = self.emit_closed_polyline(points);
                let inner_ids: Vec<usize> = voids
                    .iter()
                    .map(|hole| self.emit_closed_polyline(hole))
                    .collect();
                let inner_refs = inner_ids
                    .iter()
                    .map(|id| format!("#{id}"))
                    .collect::<Vec<_>>()
                    .join(",");
                // IFCArbitraryProfileDefWithVoids(ProfileType,
                // ProfileName, OuterCurve, InnerCurves) — four
                // attributes in IFC4, the fourth a set of curves.
                self.emit_entity(
                    profile_id,
                    format!("IFCARBITRARYPROFILEDEFWITHVOIDS(.AREA.,$,#{outer_id},({inner_refs}))"),
                );
            }
        }
        profile_id
    }

    /// Emit an `IFCINDEXEDPOLYCURVE` over `points` (feet in, metres out)
    /// with `segments`, and its `IFCCARTESIANPOINTLIST2D`. Returns the
    /// curve's entity id.
    fn emit_indexed_poly_curve(
        &mut self,
        points: &[(f64, f64)],
        segments: &[super::entities::CurveSegment],
    ) -> usize {
        use super::entities::CurveSegment;
        let coords = points
            .iter()
            .map(|(x, y)| format!("({:.6},{:.6})", x * 0.3048, y * 0.3048))
            .collect::<Vec<_>>()
            .join(",");
        let list_id = self.id();
        self.emit_entity(list_id, format!("IFCCARTESIANPOINTLIST2D(({coords}))"));
        // IFC indices count from 1.
        let index = |i: usize| (i + 1).to_string();
        let segments = segments
            .iter()
            .map(|segment| match segment {
                CurveSegment::Line(ends) => {
                    format!("IFCLINEINDEX(({}))", ends.map(index).join(","))
                }
                CurveSegment::Arc(points) => {
                    format!("IFCARCINDEX(({}))", points.map(index).join(","))
                }
            })
            .collect::<Vec<_>>()
            .join(",");
        let curve_id = self.id();
        self.emit_entity(
            curve_id,
            format!("IFCINDEXEDPOLYCURVE(#{list_id},({segments}),.F.)"),
        );
        curve_id
    }

    /// Emit one `IFCCARTESIANPOINT` per vertex plus the
    /// `IFCPOLYLINE` that lists them, closing the ring by repeating
    /// the first point when the caller did not. Returns the polyline
    /// entity id. Coordinates are feet in, metres out.
    fn emit_closed_polyline(&mut self, points: &[(f64, f64)]) -> usize {
        let mut metres: Vec<(f64, f64)> = points
            .iter()
            .map(|(x, y)| (*x * 0.3048, *y * 0.3048))
            .collect();
        if let (Some(first), Some(last)) = (metres.first().copied(), metres.last().copied()) {
            if (first.0 - last.0).abs().max((first.1 - last.1).abs()) > 1e-9_f64 {
                metres.push(first);
            }
        }
        let mut point_ids: Vec<usize> = Vec::with_capacity(metres.len());
        for (x, y) in &metres {
            let pt_id = self.id();
            self.emit_entity(pt_id, format!("IFCCARTESIANPOINT(({x:.6},{y:.6}))"));
            point_ids.push(pt_id);
        }
        let polyline_id = self.id();
        let refs = point_ids
            .iter()
            .map(|id| format!("#{id}"))
            .collect::<Vec<_>>()
            .join(",");
        self.emit_entity(polyline_id, format!("IFCPOLYLINE(({refs}))"));
        polyline_id
    }

    /// Emit a richer solid body and
    /// return `(solid_entity_id, representation_type_token)`. The
    /// second value is the `IfcShapeRepresentation.RepresentationType`
    /// token that the caller should use: `"SweptSolid"` for
    /// extruded / revolved, `"AdvancedSweptSolid"` for a profile swept
    /// along a directrix, `"CSG"` for boolean results, `"Brep"` for
    /// faceted breps and `"Tessellation"` for triangulated face sets.
    /// IFC4's `IfcShapeRepresentationTypes` accepts only those item
    /// classes under each token.
    ///
    /// The caller is responsible for wrapping the returned entity
    /// ID in an `IfcShapeRepresentation` + `IfcProductDefinitionShape`
    /// chain.
    ///
    /// `solid_position` is the `IFCAXIS2PLACEMENT3D` that goes into
    /// the swept solid's `Position` slot, and `z_axis` the
    /// `IFCDIRECTION` used as the extrusion direction.
    ///
    /// `Position` is the profile's frame **inside** the object's own
    /// coordinate system, not a second copy of the object placement:
    /// a conforming consumer composes `ObjectPlacement × Position`, so
    /// passing the element's own axis here applies the element
    /// translation twice. Callers that author their profiles in
    /// the element-local frame — every rvt-open path does — pass the
    /// project-level identity placement.
    /// The representation items of `shape`: each of a
    /// [`SolidShape::Compound`]'s solids, else the one solid
    /// [`Self::emit_solid_shape`] writes, with the representation type.
    ///
    /// Where no one type takes all the items of a compound
    /// ([`shared_representation_type`]: a boolean result beside another kind)
    /// they are written as their union, whose operands are solids
    /// (`emit_solid_shape`'s `operand`), under `CSG`.
    fn emit_solid_items(
        &mut self,
        shape: &SolidShape,
        solid_position: usize,
        z_axis: usize,
    ) -> (Vec<usize>, &'static str) {
        match shape {
            SolidShape::Compound(items) if !items.is_empty() => {
                let kinds: std::collections::BTreeSet<&'static str> =
                    items.iter().map(solid_kind).collect();
                let shared = shared_representation_type(&kinds);
                // `CSG` takes boolean results alone, and no other type takes
                // one: without a shared type the items are written as their
                // union, so as operands.
                let operand = shared.is_none();
                let ids: Vec<usize> = items
                    .iter()
                    .map(|item| {
                        self.emit_solid_shape(item, solid_position, z_axis, operand)
                            .0
                    })
                    .collect();
                if let Some(kind) = shared {
                    return (ids, kind);
                }
                match ids.split_first() {
                    Some((first, rest)) => (vec![self.emit_union(*first, rest)], "CSG"),
                    None => (Vec::new(), "CSG"),
                }
            }
            _ => {
                let (id, kind) = self.emit_solid_shape(shape, solid_position, z_axis, false);
                (vec![id], kind)
            }
        }
    }

    /// an `IfcIndexedColourMap` giving each triangle of the face
    /// set `set_id` its material's colour (a face without one takes the
    /// most common), when its faces are not all of one material.
    fn emit_face_colours(&mut self, set_id: usize, face_materials: &[usize], kept: &[usize]) {
        let colour = |m: usize| self.material_colours.get(m).copied().flatten();
        let faces: Vec<Option<u32>> = kept
            .iter()
            .map(|&k| face_materials.get(k).and_then(|&m| colour(m)))
            .collect();
        let mut palette: Vec<u32> = Vec::new();
        let mut counts: Vec<usize> = Vec::new();
        for c in faces.iter().flatten() {
            match palette.iter().position(|p| p == c) {
                Some(i) => counts[i] += 1,
                None => {
                    palette.push(*c);
                    counts.push(1);
                }
            }
        }
        if palette.len() < 2 {
            return;
        }
        let common = (0..palette.len()).max_by_key(|&i| counts[i]).unwrap_or(0);
        let rgb = palette
            .iter()
            .map(|p| {
                let c = |s: u32| f64::from((p >> s) & 0xff) / 255.0;
                format!("({:.6},{:.6},{:.6})", c(0), c(8), c(16))
            })
            .collect::<Vec<_>>()
            .join(",");
        let index = faces
            .iter()
            .map(|f| {
                let i = f
                    .and_then(|c| palette.iter().position(|p| *p == c))
                    .unwrap_or(common);
                (i + 1).to_string()
            })
            .collect::<Vec<_>>()
            .join(",");
        let list = self.id();
        self.emit_entity(list, format!("IFCCOLOURRGBLIST(({rgb}))"));
        let map = self.id();
        self.emit_entity(
            map,
            format!("IFCINDEXEDCOLOURMAP(#{set_id},$,#{list},({index}))"),
        );
    }

    /// The union of solids `first` and `rest`, as nested `IfcBooleanResult`s.
    fn emit_union(&mut self, first: usize, rest: &[usize]) -> usize {
        let mut solid_id = first;
        for other in rest {
            let union = self.id();
            self.emit_entity(
                union,
                format!("IFCBOOLEANRESULT(.UNION.,#{solid_id},#{other})"),
            );
            solid_id = union;
        }
        solid_id
    }

    /// A wall's `Axis` representation in context `axis_ctx`: an
    /// `IfcIndexedPolyCurve` from `ends[0]` to `ends[1]` (element frame,
    /// feet, at z 0), as Revit's export writes it. Returns the
    /// representation's id.
    fn emit_wall_axis(&mut self, axis_ctx: usize, ends: [[f64; 2]; 2]) -> usize {
        let [a, b] = ends;
        let list = self.id();
        self.emit_entity(
            list,
            format!(
                "IFCCARTESIANPOINTLIST3D((({},{},0.),({},{},0.)))",
                format_real(a[0] * 0.3048),
                format_real(a[1] * 0.3048),
                format_real(b[0] * 0.3048),
                format_real(b[1] * 0.3048),
            ),
        );
        let curve = self.id();
        self.emit_entity(curve, format!("IFCINDEXEDPOLYCURVE(#{list},$,.F.)"));
        let rep = self.id();
        self.emit_entity(
            rep,
            format!("IFCSHAPEREPRESENTATION(#{axis_ctx},'Axis','Curve3D',(#{curve}))"),
        );
        rep
    }

    /// One solid of a body. `operand` says it is an operand of an
    /// `IfcBooleanResult` (a union of a compound's items, or an operand of a
    /// [`SolidShape::BooleanResult`]): IFC4 takes a tessellated operand only
    /// where it is closed (`IfcBooleanResult.FirstOperandClosed` and
    /// `SecondOperandClosed`), and the face set records `Closed` as `$`, so a
    /// triangulated operand is written as the faceted brep of the same
    /// triangles, a solid with no such rule.
    fn emit_solid_shape(
        &mut self,
        shape: &SolidShape,
        solid_position: usize,
        z_axis: usize,
        operand: bool,
    ) -> (usize, &'static str) {
        match shape {
            SolidShape::ExtrudedArea(ex) => {
                // Same chain as the inline extrusion path — call
                // emit_profile_def then wrap in IFCEXTRUDEDAREASOLID.
                let profile_origin = self.id();
                self.emit_entity(profile_origin, "IFCCARTESIANPOINT((0.,0.))");
                let profile_x_axis = self.id();
                self.emit_entity(profile_x_axis, "IFCDIRECTION((1.,0.))");
                let profile_placement = self.id();
                self.emit_entity(
                    profile_placement,
                    format!("IFCAXIS2PLACEMENT2D(#{profile_origin},#{profile_x_axis})"),
                );
                let profile_id = self.emit_profile_def(ex, profile_placement);
                let depth = ex.height_feet * 0.3048;
                let solid_id = self.id();
                self.emit_entity(
                    solid_id,
                    format!(
                        "IFCEXTRUDEDAREASOLID(#{profile_id},#{solid_position},#{z_axis},{depth:.6})"
                    ),
                );
                (solid_id, "SweptSolid")
            }
            SolidShape::RevolvedArea {
                profile,
                axis_origin_feet,
                axis_direction,
                angle_radians,
            } => {
                // Axis-of-revolution: IfcAxis1Placement (Location,
                // Direction). Both location and direction are
                // element-local.
                let axis_origin_id = self.id();
                let [ox, oy, oz] = *axis_origin_feet;
                let (ox, oy, oz) = (ox * 0.3048, oy * 0.3048, oz * 0.3048);
                self.emit_entity(
                    axis_origin_id,
                    format!("IFCCARTESIANPOINT(({ox:.6},{oy:.6},{oz:.6}))"),
                );
                let axis_dir_id = self.id();
                let [dx, dy, dz] = *axis_direction;
                // Normalise to unit vector per IFC4.
                let mag = (dx * dx + dy * dy + dz * dz).sqrt().max(1e-12);
                let (dx, dy, dz) = (dx / mag, dy / mag, dz / mag);
                self.emit_entity(
                    axis_dir_id,
                    format!("IFCDIRECTION(({dx:.6},{dy:.6},{dz:.6}))"),
                );
                let axis1_id = self.id();
                self.emit_entity(
                    axis1_id,
                    format!("IFCAXIS1PLACEMENT(#{axis_origin_id},#{axis_dir_id})"),
                );
                // Profile (same 2D placement boilerplate as
                // extruded-area). Reuse emit_profile_def by wrapping
                // our ProfileDef in a temporary Extrusion — the
                // emit_profile_def signature takes an Extrusion but
                // only reads profile_override / width_feet / depth_feet.
                let profile_origin = self.id();
                self.emit_entity(profile_origin, "IFCCARTESIANPOINT((0.,0.))");
                let profile_x_axis = self.id();
                self.emit_entity(profile_x_axis, "IFCDIRECTION((1.,0.))");
                let profile_placement = self.id();
                self.emit_entity(
                    profile_placement,
                    format!("IFCAXIS2PLACEMENT2D(#{profile_origin},#{profile_x_axis})"),
                );
                let wrap_ex = Extrusion {
                    width_feet: 0.0,
                    depth_feet: 0.0,
                    height_feet: 0.0,
                    profile_override: Some(profile.clone()),
                };
                let profile_id = self.emit_profile_def(&wrap_ex, profile_placement);
                let solid_id = self.id();
                // IFCREVOLVEDAREASOLID(SweptArea, Position, Axis, Angle).
                // SI units = radians, writer converts caller's angle
                // (already in radians) at identity.
                self.emit_entity(
                    solid_id,
                    format!(
                        "IFCREVOLVEDAREASOLID(#{profile_id},#{solid_position},#{axis1_id},{angle_radians:.6})"
                    ),
                );
                (solid_id, "SweptSolid")
            }
            SolidShape::Compound(items) => {
                // Nested in another solid, the items are one: their union.
                let union = items.len() > 1;
                let parts: Vec<(usize, &'static str)> = items
                    .iter()
                    .map(|item| {
                        self.emit_solid_shape(item, solid_position, z_axis, operand || union)
                    })
                    .collect();
                let Some(((first, kind), rest)) = parts.split_first() else {
                    return self.emit_solid_shape(
                        &SolidShape::TriangulatedFaceSet {
                            vertices_feet: Vec::new(),
                            triangles: Vec::new(),
                            face_materials: Vec::new(),
                        },
                        solid_position,
                        z_axis,
                        operand,
                    );
                };
                if rest.is_empty() {
                    return (*first, *kind);
                }
                let rest: Vec<usize> = rest.iter().map(|(id, _)| *id).collect();
                (self.emit_union(*first, &rest), "CSG")
            }
            SolidShape::BooleanResult {
                op,
                operand_a,
                operand_b,
            } => {
                // Recursively emit operands, then wrap in
                // IFCBOOLEANRESULT(op, first, second). Nested
                // booleans compose naturally.
                let (a_id, _rep_a) = self.emit_solid_shape(operand_a, solid_position, z_axis, true);
                let (b_id, _rep_b) = self.emit_solid_shape(operand_b, solid_position, z_axis, true);
                let solid_id = self.id();
                self.emit_entity(
                    solid_id,
                    format!("IFCBOOLEANRESULT({},#{a_id},#{b_id})", op.as_step_keyword()),
                );
                (solid_id, "CSG")
            }
            SolidShape::FacetedBrep {
                vertices_feet,
                triangles,
            } => {
                // One IfcCartesianPoint per vertex.
                let mut vert_ids: Vec<usize> = Vec::with_capacity(vertices_feet.len());
                for [x, y, z] in vertices_feet {
                    let (xm, ym, zm) = (x * 0.3048, y * 0.3048, z * 0.3048);
                    let pid = self.id();
                    self.emit_entity(pid, format!("IFCCARTESIANPOINT(({xm:.6},{ym:.6},{zm:.6}))"));
                    vert_ids.push(pid);
                }
                // One IfcPolyLoop + IfcFaceBound + IfcFace per triangle.
                // Triangles with out-of-range indices are skipped so
                // downstream parsers don't choke on a dangling ref;
                // debug builds panic instead so the caller learns
                // about bad mesh input during testing.
                let mut face_ids: Vec<usize> = Vec::new();
                for tri in triangles {
                    let idx_a = tri.0 as usize;
                    let idx_b = tri.1 as usize;
                    let idx_c = tri.2 as usize;
                    if idx_a >= vert_ids.len() || idx_b >= vert_ids.len() || idx_c >= vert_ids.len()
                    {
                        debug_assert!(
                            false,
                            "FacetedBrep triangle ({},{},{}) refers to vertex out of range (len={})",
                            tri.0,
                            tri.1,
                            tri.2,
                            vert_ids.len(),
                        );
                        continue;
                    }
                    let (va, vb, vc) = (vert_ids[idx_a], vert_ids[idx_b], vert_ids[idx_c]);
                    let loop_id = self.id();
                    self.emit_entity(loop_id, format!("IFCPOLYLOOP((#{va},#{vb},#{vc}))"));
                    let bound_id = self.id();
                    self.emit_entity(bound_id, format!("IFCFACEBOUND(#{loop_id},.T.)"));
                    let face_id = self.id();
                    self.emit_entity(face_id, format!("IFCFACE((#{bound_id}))"));
                    face_ids.push(face_id);
                }
                // IfcClosedShell wrapping all faces, then IfcFacetedBrep.
                let shell_id = self.id();
                let face_refs = face_ids
                    .iter()
                    .map(|id| format!("#{id}"))
                    .collect::<Vec<_>>()
                    .join(",");
                self.emit_entity(shell_id, format!("IFCCLOSEDSHELL(({face_refs}))"));
                let brep_id = self.id();
                self.emit_entity(
                    brep_id,
                    format!(
                        "IFCFACETEDBREP(#{brep_id_placeholder})",
                        brep_id_placeholder = shell_id
                    ),
                );
                (brep_id, "Brep")
            }
            SolidShape::TriangulatedFaceSet {
                vertices_feet,
                triangles,
                ..
            } if operand => {
                let count = vertices_feet.len();
                let brep = SolidShape::FacetedBrep {
                    vertices_feet: vertices_feet.clone(),
                    triangles: triangles
                        .iter()
                        .filter(|t| [t.0, t.1, t.2].iter().all(|&i| (i as usize) < count))
                        .cloned()
                        .collect(),
                };
                self.emit_solid_shape(&brep, solid_position, z_axis, false)
            }
            SolidShape::TriangulatedFaceSet {
                vertices_feet,
                triangles,
                face_materials,
            } => {
                let points = vertices_feet
                    .iter()
                    .map(|[x, y, z]| {
                        format!("({:.6},{:.6},{:.6})", x * 0.3048, y * 0.3048, z * 0.3048)
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                let list_id = self.id();
                self.emit_entity(list_id, format!("IFCCARTESIANPOINTLIST3D(({points}))"));
                // `CoordIndex` is 1-based; triangles pointing outside the
                // list are left out rather than written dangling.
                let count = vertices_feet.len();
                let kept: Vec<usize> = (0..triangles.len())
                    .filter(|&k| {
                        let t = &triangles[k];
                        [t.0, t.1, t.2].iter().all(|&i| (i as usize) < count)
                    })
                    .collect();
                let indices = kept
                    .iter()
                    .map(|&k| {
                        let t = &triangles[k];
                        format!("({},{},{})", t.0 + 1, t.1 + 1, t.2 + 1)
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                let set_id = self.id();
                self.emit_entity(
                    set_id,
                    format!("IFCTRIANGULATEDFACESET(#{list_id},$,$,({indices}),$)"),
                );
                self.emit_face_colours(set_id, face_materials, &kept);
                (set_id, "Tessellation")
            }
            SolidShape::SweptPath {
                profile,
                directrix_points_feet,
                fixed_reference,
            } => {
                // IFC-17: IfcFixedReferenceSweptAreaSolid. Profile
                // sweeps along the directrix polyline, orthogonal
                // at every sample, with `fixed_reference` providing
                // the "up" direction.

                // Directrix — one IfcCartesianPoint per vertex, then
                // an IfcPolyline referencing them.
                let mut dir_pt_ids: Vec<usize> = Vec::with_capacity(directrix_points_feet.len());
                for [x, y, z] in directrix_points_feet {
                    let (xm, ym, zm) = (x * 0.3048, y * 0.3048, z * 0.3048);
                    let pt = self.id();
                    self.emit_entity(pt, format!("IFCCARTESIANPOINT(({xm:.6},{ym:.6},{zm:.6}))"));
                    dir_pt_ids.push(pt);
                }
                let directrix = self.id();
                let pt_refs = dir_pt_ids
                    .iter()
                    .map(|id| format!("#{id}"))
                    .collect::<Vec<_>>()
                    .join(",");
                self.emit_entity(directrix, format!("IFCPOLYLINE(({pt_refs}))"));

                // Profile — same 2D placement pattern as
                // emit_profile_def. Wrap ProfileDef in a temporary
                // Extrusion to reuse the existing helper (its
                // dimensions aren't used since we override via
                // profile_override).
                let profile_origin = self.id();
                self.emit_entity(profile_origin, "IFCCARTESIANPOINT((0.,0.))");
                let profile_x_axis = self.id();
                self.emit_entity(profile_x_axis, "IFCDIRECTION((1.,0.))");
                let profile_placement = self.id();
                self.emit_entity(
                    profile_placement,
                    format!("IFCAXIS2PLACEMENT2D(#{profile_origin},#{profile_x_axis})"),
                );
                let wrap_ex = Extrusion {
                    width_feet: 0.0,
                    depth_feet: 0.0,
                    height_feet: 0.0,
                    profile_override: Some(profile.clone()),
                };
                let profile_id = self.emit_profile_def(&wrap_ex, profile_placement);

                // Fixed reference — normalise to unit length.
                let [fx, fy, fz] = *fixed_reference;
                let mag = (fx * fx + fy * fy + fz * fz).sqrt().max(1e-12);
                let (fx, fy, fz) = (fx / mag, fy / mag, fz / mag);
                let fixed_ref_id = self.id();
                self.emit_entity(
                    fixed_ref_id,
                    format!("IFCDIRECTION(({fx:.6},{fy:.6},{fz:.6}))"),
                );

                // IFCFIXEDREFERENCESWEPTAREASOLID(SweptArea, Position,
                //   Directrix, StartParam, EndParam, FixedReference).
                // StartParam / EndParam = $ means use the full
                // directrix from start to end — the IFC4 default.
                let solid_id = self.id();
                self.emit_entity(
                    solid_id,
                    format!(
                        "IFCFIXEDREFERENCESWEPTAREASOLID(#{profile_id},#{solid_position},#{directrix},$,$,#{fixed_ref_id})"
                    ),
                );
                // IFC4 reserves `SweptSolid` for extruded and revolved
                // solids; a fixed-reference sweep is `AdvancedSweptSolid`
                // (IfcShapeRepresentationTypes).
                (solid_id, "AdvancedSweptSolid")
            }
            SolidShape::PlacedExtrusion {
                profile,
                origin_feet,
                axis,
                ref_direction,
                depth_feet,
            } => {
                // IfcExtrudedAreaSolid with its own Position, so the
                // profile's plane is stated rather than implied.
                let profile_origin = self.id();
                self.emit_entity(profile_origin, "IFCCARTESIANPOINT((0.,0.))");
                let profile_x_axis = self.id();
                self.emit_entity(profile_x_axis, "IFCDIRECTION((1.,0.))");
                let profile_placement = self.id();
                self.emit_entity(
                    profile_placement,
                    format!("IFCAXIS2PLACEMENT2D(#{profile_origin},#{profile_x_axis})"),
                );
                let wrap_ex = Extrusion {
                    width_feet: 0.0,
                    depth_feet: 0.0,
                    height_feet: 0.0,
                    profile_override: Some(profile.clone()),
                };
                let profile_id = self.emit_profile_def(&wrap_ex, profile_placement);
                let unit = |v: [f64; 3]| {
                    let mag = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-12);
                    (v[0] / mag, v[1] / mag, v[2] / mag)
                };
                let [ox, oy, oz] = *origin_feet;
                let origin_id = self.id();
                self.emit_entity(
                    origin_id,
                    format!(
                        "IFCCARTESIANPOINT(({:.6},{:.6},{:.6}))",
                        ox * 0.3048,
                        oy * 0.3048,
                        oz * 0.3048
                    ),
                );
                let (ax, ay, az) = unit(*axis);
                let axis_id = self.id();
                self.emit_entity(axis_id, format!("IFCDIRECTION(({ax:.9},{ay:.9},{az:.9}))"));
                let (rx, ry, rz) = unit(*ref_direction);
                let ref_id = self.id();
                self.emit_entity(ref_id, format!("IFCDIRECTION(({rx:.9},{ry:.9},{rz:.9}))"));
                let position = self.id();
                self.emit_entity(
                    position,
                    format!("IFCAXIS2PLACEMENT3D(#{origin_id},#{axis_id},#{ref_id})"),
                );
                let depth = depth_feet * 0.3048;
                let solid_id = self.id();
                self.emit_entity(
                    solid_id,
                    format!("IFCEXTRUDEDAREASOLID(#{profile_id},#{position},#{z_axis},{depth:.6})"),
                );
                (solid_id, "SweptSolid")
            }
        }
    }

    /// The IfcUnitAssignment: the SI units every value is written in.
    /// The writer converts Revit's feet, square and cubic feet and
    /// pounds on the way out and writes angles in radians, so these are
    /// the units a reader must apply. `model.units`, Revit's display units,
    /// describe the source model and stay out of it; the export
    /// diagnostics report them. A model with a thermal transmittance in any
    /// element's property set also gets kelvin and W/(m²·K), the derived unit
    /// its values are in, as Revit's export declares it; one with a
    /// pressure, power, voltage, current or volumetric flow rate, in an
    /// element's or a type object's set, gets pascal, watt, volt, ampere or
    /// m³/s.
    fn emit_unit_assignment(&mut self, model: &IfcModel) -> Vec<usize> {
        let mut units: Vec<usize> = [
            ("LENGTHUNIT", "$", "METRE"),
            ("AREAUNIT", "$", "SQUARE_METRE"),
            ("VOLUMEUNIT", "$", "CUBIC_METRE"),
            ("PLANEANGLEUNIT", "$", "RADIAN"),
            ("MASSUNIT", ".KILO.", "GRAM"),
            ("TIMEUNIT", "$", "SECOND"),
        ]
        .into_iter()
        .map(|(unit_type, prefix, name)| {
            let id = self.id();
            self.emit_entity(id, format!("IFCSIUNIT(*,.{unit_type}.,{prefix},.{name}.)"));
            id
        })
        .collect();
        // Whether any value written in a property set, on an element or a
        // type object, is of the kind `kind` tests.
        fn any_value(
            value: &super::entities::PropertyValue,
            kind: fn(&super::entities::PropertyValue) -> bool,
        ) -> bool {
            match value {
                super::entities::PropertyValue::List(items) => {
                    items.iter().any(|item| any_value(item, kind))
                }
                value => kind(value),
            }
        }
        let uses = |kind: fn(&super::entities::PropertyValue) -> bool| {
            model
                .entities
                .iter()
                .filter_map(|entity| match entity {
                    super::entities::IfcEntity::ElementPropertySet { set, .. } => Some(set),
                    super::entities::IfcEntity::BuildingElement { property_set, .. } => {
                        property_set.as_ref()
                    }
                    _ => None,
                })
                .chain(model.type_property_sets.values().flatten())
                .any(|set| {
                    set.properties
                        .iter()
                        .any(|property| any_value(&property.value, kind))
                })
        };
        use super::entities::PropertyValue as V;
        let thermal = uses(|v| matches!(v, V::ThermalTransmittance(_)));
        if thermal {
            let (kilogram, second) = (units[4], units[5]);
            let kelvin = self.id();
            self.emit_entity(
                kelvin,
                "IFCSIUNIT(*,.THERMODYNAMICTEMPERATUREUNIT.,$,.KELVIN.)",
            );
            let elements: Vec<usize> = [(kilogram, 1), (kelvin, -1), (second, -3)]
                .into_iter()
                .map(|(unit, exponent)| {
                    let id = self.id();
                    self.emit_entity(id, format!("IFCDERIVEDUNITELEMENT(#{unit},{exponent})"));
                    id
                })
                .collect();
            let derived = self.id();
            self.emit_entity(
                derived,
                format!(
                    "IFCDERIVEDUNIT(({}),.THERMALTRANSMITTANCEUNIT.,$)",
                    elements
                        .iter()
                        .map(|id| format!("#{id}"))
                        .collect::<Vec<_>>()
                        .join(",")
                ),
            );
            units.extend([kelvin, derived]);
        }
        // Revit's air flow, pressure, power, voltage and current parameters,
        // in the SI units Revit's export declares for them.
        for (kind, unit_type, name) in [
            (
                (|v| matches!(v, V::Pressure(_))) as fn(&V) -> bool,
                "PRESSUREUNIT",
                "PASCAL",
            ),
            (|v| matches!(v, V::Power(_)), "POWERUNIT", "WATT"),
            (
                |v| matches!(v, V::ElectricVoltage(_)),
                "ELECTRICVOLTAGEUNIT",
                "VOLT",
            ),
            (
                |v| matches!(v, V::ElectricCurrent(_)),
                "ELECTRICCURRENTUNIT",
                "AMPERE",
            ),
        ] {
            if uses(kind) {
                let id = self.id();
                self.emit_entity(id, format!("IFCSIUNIT(*,.{unit_type}.,$,.{name}.)"));
                units.push(id);
            }
        }
        if uses(|v| matches!(v, V::VolumetricFlowRate(_))) {
            let (metre, second) = (units[0], units[5]);
            let elements: Vec<usize> = [(metre, 3), (second, -1)]
                .into_iter()
                .map(|(unit, exponent)| {
                    let id = self.id();
                    self.emit_entity(id, format!("IFCDERIVEDUNITELEMENT(#{unit},{exponent})"));
                    id
                })
                .collect();
            let derived = self.id();
            self.emit_entity(
                derived,
                format!(
                    "IFCDERIVEDUNIT((#{},#{}),.VOLUMETRICFLOWRATEUNIT.,$)",
                    elements[0], elements[1]
                ),
            );
            units.push(derived);
        }
        units
    }

    fn emit_header(&mut self, model: &IfcModel) {
        let project = escape(model.project_name.as_deref().unwrap_or("Untitled"));
        let desc = escape(
            model
                .description
                .as_deref()
                .unwrap_or("Produced by rvt-open — clean-room Apache-2 Revit reader."),
        );
        self.emit_line("ISO-10303-21;");
        self.emit_line("HEADER;");
        self.emit_line("FILE_DESCRIPTION(('ViewDefinition [CoordinationView]'),'2;1');");
        self.emit_line(format!(
            "FILE_NAME('{project}.ifc','{}',('rvt-open'),('rvt-open contributors'),'rvt-open {version}','rvt-open STEP writer','');",
            iso_timestamp_from(self.timestamp),
            version = env!("CARGO_PKG_VERSION"),
        ));
        self.emit_line("FILE_SCHEMA(('IFC4'));");
        self.emit_line("ENDSEC;");
        self.emit_line(format!("/* {desc} */"));
    }

    fn emit_data(&mut self, model: &IfcModel) {
        self.material_colours = model.materials.iter().map(|m| m.color_packed).collect();
        self.emit_line("DATA;");
        let seed = global_id_seed(model);
        let ns = guid_namespace(model);
        let gid = |key: &[&str]| stable_guid(ns, seed, key);

        // Required framework entities (buildingSMART minimum viable).
        let person = self.id();
        self.emit_entity(person, "IFCPERSON($,$,'rvt-open',$,$,$,$,$)");
        let org = self.id();
        self.emit_entity(
            org,
            "IFCORGANIZATION($,'rvt-open','Clean-room Apache-2 Revit reader',$,$)",
        );
        let person_and_org = self.id();
        self.emit_entity(
            person_and_org,
            format!("IFCPERSONANDORGANIZATION(#{person},#{org},$)"),
        );
        let application = self.id();
        self.emit_entity(
            application,
            format!(
                "IFCAPPLICATION(#{org},'{}','rvt-open','rvt_open')",
                env!("CARGO_PKG_VERSION")
            ),
        );
        let owner_hist = self.id();
        self.emit_entity(
            owner_hist,
            format!(
                // As Revit's own exports (2015, 2024, 2027): NOCHANGE with no last-modified fields, which
                // meets IFC4's CorrectChangeAction rule (ADDED needs a LastModifiedDate).
                "IFCOWNERHISTORY(#{person_and_org},#{application},$,.NOCHANGE.,$,$,$,{})",
                self.timestamp
            ),
        );

        // Unit assignment (IFC-40).
        let unit_entity_ids = self.emit_unit_assignment(model);
        let unit_assignment = self.id();
        let unit_refs = unit_entity_ids
            .iter()
            .map(|id| format!("#{id}"))
            .collect::<Vec<_>>()
            .join(",");
        self.emit_entity(unit_assignment, format!("IFCUNITASSIGNMENT(({unit_refs}))"));

        // Representation context — needs IfcAxis2Placement3D +
        // IfcDirection + IfcCartesianPoint (origin, X, Z axes).
        let origin = self.id();
        self.emit_entity(origin, "IFCCARTESIANPOINT((0.,0.,0.))");
        let z_axis = self.id();
        self.emit_entity(z_axis, "IFCDIRECTION((0.,0.,1.))");
        let x_axis = self.id();
        self.emit_entity(x_axis, "IFCDIRECTION((1.,0.,0.))");
        let axis_placement = self.id();
        self.emit_entity(
            axis_placement,
            format!("IFCAXIS2PLACEMENT3D(#{origin},#{z_axis},#{x_axis})"),
        );
        // true north is the shared coordinates' north in the
        // model's internal frame, at Revit's Angle to True North.
        let true_north = model.georeference.as_ref().map(|g| {
            let (sin, cos) = g.true_north.to_radians().sin_cos();
            let id = self.id();
            self.emit_entity(
                id,
                format!("IFCDIRECTION(({},{}))", format_real(-sin), format_real(cos)),
            );
            format!("#{id}")
        });
        let geom_ctx = self.id();
        self.emit_entity(
            geom_ctx,
            format!(
                "IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-5,#{axis_placement},{})",
                true_north.as_deref().unwrap_or("$")
            ),
        );
        if let Some(g) = &model.georeference {
            self.map_conversion(g, geom_ctx);
        }
        // the contexts of the 2D maps, as Revit's export writes
        // them: a family's plan symbol in the `Plan` context's `Annotation`
        // sub-context (`PLAN_VIEW`), an extrusion base in the model's
        // `FootPrint` (`MODEL_VIEW`) and a beam's line in its `Axis`
        // (`GRAPH_VIEW`). Emitted only when a map uses them.
        let used_plan_contexts: std::collections::BTreeSet<&str> = model
            .representation_maps
            .iter()
            .flat_map(|m| m.plan.iter().map(|p| p.context.as_str()))
            .collect();
        let mut plan_ctx_ids: HashMap<String, usize> = HashMap::new();
        let mut extra_contexts = String::new();
        if used_plan_contexts.contains("Annotation") {
            let plan_ctx = self.id();
            self.emit_entity(
                plan_ctx,
                format!("IFCGEOMETRICREPRESENTATIONCONTEXT($,'Plan',3,1.E-5,#{axis_placement},$)"),
            );
            extra_contexts.push_str(&format!(",#{plan_ctx}"));
            let sub = self.id();
            self.emit_entity(
                sub,
                format!("IFCGEOMETRICREPRESENTATIONSUBCONTEXT('Annotation','Plan',*,*,*,*,#{plan_ctx},$,.PLAN_VIEW.,$)"),
            );
            plan_ctx_ids.insert("Annotation".into(), sub);
        }
        for (name, view) in [("FootPrint", "MODEL_VIEW"), ("Axis", "GRAPH_VIEW")] {
            if used_plan_contexts.contains(name)
                || (name == "Axis" && !model.wall_location_lines.is_empty())
            {
                let sub = self.id();
                self.emit_entity(
                    sub,
                    format!("IFCGEOMETRICREPRESENTATIONSUBCONTEXT('{name}','Model',*,*,*,*,#{geom_ctx},$,.{view}.,$)"),
                );
                plan_ctx_ids.insert(name.into(), sub);
            }
        }

        // Root project.
        let project_name = escape(model.project_name.as_deref().unwrap_or("Untitled"));
        let project_desc = escape(
            model
                .description
                .as_deref()
                .unwrap_or("Exported by rvt-open"),
        );
        let project_id = self.id();
        self.emit_entity(
            project_id,
            format!(
                "IFCPROJECT('{}',#{owner_hist},'{}',{},$,$,$,(#{geom_ctx}{extra_contexts}),#{unit_assignment})",
                project_level_global_id(model, "project"),
                project_name,
                quoted_or_dollar(&project_desc),
            ),
        );

        // Spatial containment hierarchy — required by IFC4 for any
        // project with building content. We emit a minimal but valid
        // IfcSite → IfcBuilding → IfcBuildingStorey chain with
        // identity placements so downstream viewers (IFC viewers,
        // IfcOpenShell-based tools, buildingSMART validator) render
        // the file directly without needing to synthesise a host
        // structure. Names default to "Default {Site,Building,Level
        // 1}"; once the walker surfaces site/level instances they'll
        // flow in here.
        //
        // Every IfcSpatialStructureElement needs its own
        // IfcLocalPlacement — we share the `axis_placement` across
        // the three (they're all identity), then chain the
        // placements via `PlacementRelTo` so the coordinate frames
        // compose correctly.
        let site_placement = self.id();
        self.emit_entity(
            site_placement,
            format!("IFCLOCALPLACEMENT($,#{axis_placement})"),
        );
        let site_id = self.id();
        let site_label = escape(model.site_name.as_deref().unwrap_or("Default Site"));
        let building_label = escape(model.building_name.as_deref().unwrap_or("Default Building"));
        self.emit_entity(
            site_id,
            format!(
                "IFCSITE('{}',#{owner_hist},'{site_label}',$,$,#{site_placement},$,'{site_label}',.ELEMENT.,{},$,$)",
                project_level_global_id(model, "site"),
                site_reference(model.georeference.as_ref()),
            ),
        );

        // under `revit` a model with no storey has no `IfcBuilding`, and the
        // site is the spatial root that holds the elements and the spaces.
        let site_holds = model
            .convention
            .site_holds_elements(model.building_storeys.is_empty());
        let (building_placement, building_id) = if site_holds {
            (site_placement, site_id)
        } else {
            let building_placement = self.id();
            self.emit_entity(
                building_placement,
                format!("IFCLOCALPLACEMENT(#{site_placement},#{axis_placement})"),
            );
            let building_id = self.id();
            self.emit_entity(
                building_id,
                format!(
                    "IFCBUILDING('{}',#{owner_hist},'{building_label}',$,$,#{building_placement},$,'{building_label}',.ELEMENT.,$,$,$)",
                    project_level_global_id(model, "building"),
                ),
            );
            (building_placement, building_id)
        };
        let root_key = if site_holds { "site" } else { "building" };

        // Emit one IfcBuildingStorey per Revit Level. Every storey gets
        // its own IfcLocalPlacement; their IDs are later bundled into a
        // single IfcRelAggregates bound to the building. A model with no
        // decoded Levels has no storey: IFC4 does not require one, and an
        // invented `Level 1` at elevation 0 is not the model's.
        // Elements without a storey are contained in the building.
        let mut storey_ids: Vec<usize> = Vec::new();
        let mut storey_placements: Vec<usize> = Vec::new();
        let storeys = model.building_storeys.clone();
        let mut storey_gids: Vec<String> = Vec::with_capacity(storeys.len());
        let mut storey_name_counts: HashMap<&str, usize> = HashMap::new();
        for (storey_index, storey) in storeys.iter().enumerate() {
            let occurrence = storey_name_counts.entry(storey.name.as_str()).or_default();
            // The Level's own GlobalId where the file yields it.
            let storey_gid = model
                .global_ids
                .storeys
                .get(&storey_index)
                .cloned()
                .unwrap_or_else(|| gid(&["storey", &storey.name, &occurrence.to_string()]));
            *occurrence += 1;
            let placement_id = self.id();
            self.emit_entity(
                placement_id,
                format!("IFCLOCALPLACEMENT(#{building_placement},#{axis_placement})"),
            );
            let id = self.id();
            // Convert feet → metres at emit boundary; IFC4 elevation
            // attribute is in the project's length unit (metres here).
            let elevation_m = storey.elevation_feet * 0.3048;
            let name_escaped = escape(&storey.name);
            self.emit_entity(
                id,
                format!(
                    // `Elevation` is an IfcLengthMeasure (REAL), so it
                    // needs a decimal point — `0` is an INTEGER literal
                    // and ISO-10303-21 does not admit it here.
                    "IFCBUILDINGSTOREY('{storey_gid}',#{owner_hist},'{name_escaped}',$,$,#{placement_id},$,'{name_escaped}',.ELEMENT.,{elevation_m:.6})",
                ),
            );
            storey_gids.push(storey_gid);
            storey_ids.push(id);
            storey_placements.push(placement_id);
        }

        // Aggregation relationships — IfcRelAggregates is how the
        // spatial hierarchy binds in IFC4. Each level of the chain
        // gets one relationship pointing from parent to child.
        let rel_proj_site = self.id();
        self.emit_entity(
            rel_proj_site,
            format!(
                "IFCRELAGGREGATES('{}',#{owner_hist},$,$,#{project_id},(#{site_id}))",
                gid(&["aggregates", "project"]),
            ),
        );
        if !site_holds {
            let rel_site_building = self.id();
            self.emit_entity(
                rel_site_building,
                format!(
                    "IFCRELAGGREGATES('{}',#{owner_hist},$,$,#{site_id},(#{building_id}))",
                    gid(&["aggregates", "site"]),
                ),
            );
        }
        // IfcRelAggregates needs at least one related object, so a
        // building with no storey has no aggregation below it.
        if !storey_ids.is_empty() {
            let rel_building_storey = self.id();
            let storey_refs = storey_ids
                .iter()
                .map(|id| format!("#{id}"))
                .collect::<Vec<_>>()
                .join(",");
            self.emit_entity(
                rel_building_storey,
                format!(
                    "IFCRELAGGREGATES('{}',#{owner_hist},$,$,#{building_id},({storey_refs}))",
                    gid(&["aggregates", "building"]),
                ),
            );
        }

        // the sets Revit's export gives every storey and the building.
        // A model with no decoded Level has neither.
        for (index, (storey_gid, &storey_id)) in storey_gids.iter().zip(&storey_ids).enumerate() {
            for set in &PropertySet::storey_sets_for(model.storey_properties.get(&index)) {
                let key = [storey_gid.as_str(), set.name.as_str()].join("\u{1f}");
                self.emit_property_set(
                    owner_hist,
                    storey_id,
                    set,
                    &gid(&["property-set", &key]),
                    &gid(&["defines-by-properties", &key]),
                );
            }
        }
        if !storeys.is_empty() {
            let mut set = PropertySet::building_set(storeys.len());
            // The storey count only where Revit's two rules for it
            // agree.
            if !every_storey_holds_an_element(model) {
                set.properties
                    .retain(|property| property.name != "NumberOfStoreys");
            }
            let key = ["building", set.name.as_str()].join("\u{1f}");
            self.emit_property_set(
                owner_hist,
                building_id,
                &set,
                &gid(&["property-set", &key]),
                &gid(&["defines-by-properties", &key]),
            );
        }

        // Classifications — one IfcClassification per source
        // (OmniClass, Uniformat, …), with one IfcClassificationReference
        // per coded item. Each classification gets its own
        // IfcRelAssociatesClassification tying its references back to
        // the project, which is how IFC4 consumers (IFC viewers,
        // IfcOpenShell's classification viewer) discover code refs.
        //
        // RvtDocExporter populates `model.classifications` from
        // PartAtom's `<category term="...">` blocks. Previously those
        // codes were collected but never emitted; this wires them
        // through the STEP writer so downstream consumers can see
        // them directly.
        let mut classification_counts: HashMap<(&str, &str), usize> = HashMap::new();
        for classification in &model.classifications {
            let source_name = match &classification.source {
                super::entities::ClassificationSource::OmniClass => "OmniClass",
                super::entities::ClassificationSource::Uniformat => "Uniformat",
                super::entities::ClassificationSource::Other(s) => s.as_str(),
            };
            let source_name_escaped = escape(source_name);
            let edition = classification
                .edition
                .as_deref()
                .map(escape)
                .map(|e| format!("'{e}'"))
                .unwrap_or_else(|| "$".into());

            let classification_id = self.id();
            self.emit_entity(
                classification_id,
                format!("IFCCLASSIFICATION($,{edition},$,'{source_name_escaped}',$,$,$)"),
            );

            // One IfcClassificationReference per item; collect their
            // ids so we can bundle them into the IfcRelAssociatesClassification.
            let mut ref_ids: Vec<usize> = Vec::with_capacity(classification.items.len());
            let mut ref_gids: Vec<String> = Vec::with_capacity(classification.items.len());
            for item in &classification.items {
                let code_escaped = escape(&item.code);
                let name_str = item
                    .name
                    .as_deref()
                    .map(escape)
                    .map(|n| format!("'{n}'"))
                    .unwrap_or_else(|| "$".into());
                let ref_id = self.id();
                self.emit_entity(
                    ref_id,
                    format!(
                        // (Location, Identification, Name,
                        //  ReferencedSource, Description, Sort) — six
                        // attributes in IFC4.
                        "IFCCLASSIFICATIONREFERENCE($,'{code_escaped}',{name_str},#{classification_id},$,$)"
                    ),
                );
                ref_ids.push(ref_id);
                let occurrence = classification_counts
                    .entry((source_name, item.code.as_str()))
                    .or_default();
                ref_gids.push(gid(&[
                    "classification",
                    source_name,
                    &item.code,
                    &occurrence.to_string(),
                ]));
                *occurrence += 1;
            }

            if !ref_ids.is_empty() {
                let refs_list = ref_ids
                    .iter()
                    .map(|id| format!("#{id}"))
                    .collect::<Vec<_>>()
                    .join(",");
                // IfcRelAssociatesClassification binds a set of objects
                // to one classification reference. IFC4's schema
                // requires the RelatingClassification to be a single
                // IfcClassificationReferenceSelect; we pick the last
                // reference as the relating one and treat the rest as
                // project associations. If the project only has one
                // reference this is exact; when there are multiple,
                // each gets its own association relationship.
                for (ref_id, ref_gid) in ref_ids.iter().zip(&ref_gids) {
                    let rel_id = self.id();
                    self.emit_entity(
                        rel_id,
                        format!(
                            "IFCRELASSOCIATESCLASSIFICATION('{ref_gid}',#{owner_hist},$,$,(#{project_id}),#{ref_id})",
                        ),
                    );
                }
                // Silence the warning about an unused local when the
                // outer `for` loop only iterates once.
                let _ = refs_list;
            }
        }

        // BuildingElement emission — one `IFC<TYPE>` instance per decoded
        // Revit element (Wall, Floor, Roof, Ceiling, Door, Window, Column,
        // Beam…). Each element gets its own `IFCLOCALPLACEMENT` relative
        // to whichever storey contains it (index from
        // `BuildingElement.storey_index`; falls back to storey[0] when
        // unset). At the end, elements are grouped by storey and one
        // `IFCRELCONTAINEDINSPATIALSTRUCTURE` is emitted per non-empty
        // storey — which is how IFC4 tools (IFC viewers, IfcOpenShell)
        // show "Floor 2 contains Wall-7, Wall-8" in the project
        // browser.
        //
        // Geometry (`IfcShapeRepresentation`) is intentionally omitted
        // here — tasks IFC-15 through IFC-22 produce proper
        // representations once Phase-5 geometry lands. For now every
        // element carries its placement + name + GUID, which validates
        // against the IFC4 schema as a "geometry-free" element.
        // Emit IfcMaterials upfront so BuildingElements can reference
        // them. Each material gets:
        //   - IFCMATERIAL with just a name (IFC4 minimum)
        //   - If the material has a color: IFCCOLOURRGB +
        //     IFCSURFACESTYLERENDERING + IFCSURFACESTYLE +
        //     IFCSTYLEDITEM to surface the color to IFC4 viewers
        // The color emission is gated because a color-less material
        // is valid IFC4 — we don't want to emit empty rendering
        // records when there's nothing to render.
        let mut material_ids: Vec<usize> = Vec::with_capacity(model.materials.len());
        for mat in &model.materials {
            let mat_id = self.id();
            let name_escaped = escape(&mat.name);
            self.emit_entity(mat_id, format!("IFCMATERIAL('{name_escaped}',$,$)"));
            if let Some(packed) = mat.color_packed {
                let r = (packed & 0xFF) as f64 / 255.0;
                let g = ((packed >> 8) & 0xFF) as f64 / 255.0;
                let b = ((packed >> 16) & 0xFF) as f64 / 255.0;
                let colour_id = self.id();
                self.emit_entity(colour_id, format!("IFCCOLOURRGB($,{r:.6},{g:.6},{b:.6})"));
                let rendering_id = self.id();
                let transparency = mat.transparency.unwrap_or(0.0);
                self.emit_entity(
                    rendering_id,
                    format!(
                        "IFCSURFACESTYLERENDERING(#{colour_id},{transparency:.6},$,$,$,$,$,$,.FLAT.)"
                    ),
                );
                let style_id = self.id();
                self.emit_entity(
                    style_id,
                    format!("IFCSURFACESTYLE('{name_escaped}',.BOTH.,(#{rendering_id}))"),
                );
                let presentation_id = self.id();
                self.emit_entity(
                    presentation_id,
                    format!("IFCPRESENTATIONSTYLEASSIGNMENT((#{style_id}))"),
                );
                let styled_item_id = self.id();
                self.emit_entity(
                    styled_item_id,
                    format!("IFCSTYLEDITEM($,(#{presentation_id}),'{name_escaped}')"),
                );
            }
            material_ids.push(mat_id);
        }

        // IFC-28: Emit IfcMaterialLayer + IfcMaterialLayerSet for
        // every compound assembly declared on the model. Layers
        // reference the `material_ids` vector above by index, so
        // layer_set emission MUST follow single-material emission.
        // Track layer-set ids for later IfcRelAssociatesMaterial
        // pairing.
        let mut layer_set_ids: Vec<usize> = Vec::with_capacity(model.material_layer_sets.len());
        for lset in &model.material_layer_sets {
            let mut layer_ids: Vec<usize> = Vec::with_capacity(lset.layers.len());
            for layer in &lset.layers {
                // A layer with no material writes `$` (IFC4 makes
                // IfcMaterialLayer.Material optional). An out-of-range
                // index gets the first material as a defensive fallback;
                // losing the layer would be worse.
                let material_slot = match layer.material_index {
                    None => "$".to_string(),
                    Some(index) => {
                        let mat_id = material_ids
                            .get(index)
                            .copied()
                            .or_else(|| material_ids.first().copied())
                            .unwrap_or(0);
                        if mat_id == 0 {
                            // No materials at all — skip this layer. IFC4
                            // allows IfcMaterialLayerSet with zero layers,
                            // so the set will still emit below, just empty.
                            continue;
                        }
                        format!("#{mat_id}")
                    }
                };
                let layer_id = self.id();
                // IfcMaterialLayer(Material, LayerThickness, IsVentilated=$,
                //                  Category=$, Priority=$, Name=$, Description=$)
                // Thickness in metres (feet × 0.3048).
                let thickness_m = layer.thickness_feet * 0.3048;
                let name_slot = match &layer.name {
                    Some(n) => format!("'{}'", escape(n)),
                    None => "$".into(),
                };
                self.emit_entity(
                    layer_id,
                    format!(
                        "IFCMATERIALLAYER({material_slot},{thickness_m:.6},$,$,$,{name_slot},$)"
                    ),
                );
                layer_ids.push(layer_id);
            }
            let set_id = self.id();
            let set_name = escape(&lset.name);
            let desc_slot = match &lset.description {
                Some(d) => format!("'{}'", escape(d)),
                None => "$".into(),
            };
            let layer_refs = if layer_ids.is_empty() {
                "()".to_string()
            } else {
                format!(
                    "({})",
                    layer_ids
                        .iter()
                        .map(|id| format!("#{id}"))
                        .collect::<Vec<_>>()
                        .join(",")
                )
            };
            // IfcMaterialLayerSet(MaterialLayers, LayerSetName, Description)
            self.emit_entity(
                set_id,
                format!("IFCMATERIALLAYERSET({layer_refs},'{set_name}',{desc_slot})"),
            );
            layer_set_ids.push(set_id);
        }

        // IFC-30: Emit IfcMaterialProfileSet for structural framing
        // (columns / beams) with named cross-sections. Each profile
        // carries a material index + profile name; at IfcMaterialProfile
        // emission time the profile name is also echoed as the
        // associated IfcProfileDef name — downstream tools that care
        // about precise cross-section geometry still need the profile
        // def itself (tracked separately in IFC-24).
        let mut profile_set_ids: Vec<usize> = Vec::with_capacity(model.material_profile_sets.len());
        for pset in &model.material_profile_sets {
            let mut profile_ids: Vec<usize> = Vec::with_capacity(pset.profiles.len());
            for profile in &pset.profiles {
                let mat_id = material_ids
                    .get(profile.material_index)
                    .copied()
                    .or_else(|| material_ids.first().copied())
                    .unwrap_or(0);
                if mat_id == 0 {
                    continue;
                }
                // The profile itself where the model carries it;
                // otherwise a 1 x 1 m IfcRectangleProfileDef stand-in, as
                // IfcMaterialProfile requires a profile def reference.
                let profile_def_id = match &profile.profile {
                    Some(def) => {
                        let origin = self.id();
                        self.emit_entity(origin, "IFCCARTESIANPOINT((0.,0.))");
                        let x_axis = self.id();
                        self.emit_entity(x_axis, "IFCDIRECTION((1.,0.))");
                        let placement = self.id();
                        self.emit_entity(
                            placement,
                            format!("IFCAXIS2PLACEMENT2D(#{origin},#{x_axis})"),
                        );
                        let wrap = Extrusion {
                            width_feet: 0.0,
                            depth_feet: 0.0,
                            height_feet: 0.0,
                            profile_override: Some(def.clone()),
                        };
                        self.emit_profile_def(&wrap, placement)
                    }
                    None => {
                        let profile_def_id = self.id();
                        let profile_def_name = escape(&profile.profile_name);
                        self.emit_entity(
                            profile_def_id,
                            format!("IFCRECTANGLEPROFILEDEF(.AREA.,'{profile_def_name}',$,1.,1.)"),
                        );
                        profile_def_id
                    }
                };
                let profile_id = self.id();
                let profile_name = escape(&profile.profile_name);
                let desc_slot = match &profile.description {
                    Some(d) => format!("'{}'", escape(d)),
                    None => "$".into(),
                };
                // IfcMaterialProfile(Name, Description, Material, Profile,
                //                    Priority=$, Category=$)
                self.emit_entity(
                    profile_id,
                    format!(
                        "IFCMATERIALPROFILE('{profile_name}',{desc_slot},#{mat_id},#{profile_def_id},$,$)"
                    ),
                );
                profile_ids.push(profile_id);
            }
            let set_id = self.id();
            let set_name = escape(&pset.name);
            let desc_slot = match &pset.description {
                Some(d) => format!("'{}'", escape(d)),
                None => "$".into(),
            };
            let profile_refs = if profile_ids.is_empty() {
                "()".to_string()
            } else {
                format!(
                    "({})",
                    profile_ids
                        .iter()
                        .map(|id| format!("#{id}"))
                        .collect::<Vec<_>>()
                        .join(",")
                )
            };
            // IfcMaterialProfileSet(Name, Description, MaterialProfiles, CompositeProfile)
            self.emit_entity(
                set_id,
                format!("IFCMATERIALPROFILESET('{set_name}',{desc_slot},{profile_refs},$)"),
            );
            profile_set_ids.push(set_id);
        }

        // Family instances' material constituent sets, as Revit's
        // export writes them: unnamed, one constituent per material named
        // after it, in the category "Materials".
        let mut constituent_set_ids: Vec<Option<usize>> =
            Vec::with_capacity(model.material_constituent_sets.len());
        for cset in &model.material_constituent_sets {
            let mut constituent_ids = Vec::with_capacity(cset.material_indices.len());
            for (position, &material_index) in cset.material_indices.iter().enumerate() {
                let (Some(&mat_id), Some(material)) = (
                    material_ids.get(material_index),
                    model.materials.get(material_index),
                ) else {
                    continue;
                };
                let constituent_id = self.id();
                // IfcMaterialConstituent(Name, Description, Material,
                //   Fraction, Category)
                self.emit_entity(
                    constituent_id,
                    format!(
                        "IFCMATERIALCONSTITUENT('{}',$,#{mat_id},$,'Materials')",
                        escape(cset.names.get(position).unwrap_or(&material.name))
                    ),
                );
                constituent_ids.push(constituent_id);
            }
            if constituent_ids.is_empty() {
                constituent_set_ids.push(None);
                continue;
            }
            let set_id = self.id();
            // IfcMaterialConstituentSet(Name, Description, MaterialConstituents)
            self.emit_entity(
                set_id,
                format!(
                    "IFCMATERIALCONSTITUENTSET($,$,({}))",
                    constituent_ids
                        .iter()
                        .map(|id| format!("#{id}"))
                        .collect::<Vec<_>>()
                        .join(",")
                ),
            );
            constituent_set_ids.push(Some(set_id));
        }
        let constituent_set_of: HashMap<usize, usize> = model
            .material_constituent_sets
            .iter()
            .enumerate()
            .flat_map(|(set, cset)| cset.elements.iter().map(move |&entity| (entity, set)))
            .collect();
        let mut element_constituent_pairs: Vec<(usize, usize)> = Vec::new();

        // IFC-21: emit IfcRepresentationMap entities up-front. Each
        // map carries its shape representation (emitted once) and an
        // IFCAXIS2PLACEMENT3D mapping origin; instances reference
        // the map via IFCMAPPEDITEM in their own
        // IfcShapeRepresentation body. The emitted-map ID is stored
        // by index so the per-element loop can look it up from
        // `representation_map_index`.
        let mut representation_map_ids: Vec<usize> =
            Vec::with_capacity(model.representation_maps.len());
        let mut plan_map_ids: Vec<Vec<(String, usize, usize)>> =
            Vec::with_capacity(model.representation_maps.len());
        for rmap in &model.representation_maps {
            // Mapping origin: per IFC4, usually identity (0,0,0)
            // with +X east / +Z up. Non-identity origins are rare —
            // they shift the shared shape relative to the mapped-item
            // transform.
            let [mx, my, mz] = rmap.origin_feet;
            let (mx, my, mz) = (mx * 0.3048, my * 0.3048, mz * 0.3048);
            let map_origin_pt = self.id();
            self.emit_entity(
                map_origin_pt,
                format!("IFCCARTESIANPOINT(({mx:.6},{my:.6},{mz:.6}))"),
            );
            // Reuse the project-level #z_axis / project-level #x_axis
            // directions — identity orientation is the common case.
            let map_placement = self.id();
            self.emit_entity(
                map_placement,
                format!("IFCAXIS2PLACEMENT3D(#{map_origin_pt},#{z_axis},#{x_axis})"),
            );
            // Emit the shared shape body. The emission helper returns the
            // solids' entity ids and the one RepresentationType IFC4 takes
            // for them (SweptSolid, AdvancedSweptSolid, SolidModel,
            // SurfaceOrSolidModel, CSG, Brep or Tessellation), which is the
            // map's own IfcShapeRepresentation's.
            //
            // Passing `map_placement` as the solid `Position` here is
            // not the double-translation: a mapped item resolves
            // as `MappingTarget × MappingOrigin⁻¹ × geometry`, so a
            // Position equal to the MappingOrigin cancels against the
            // inverse rather than compounding. `origin_feet` is
            // `[0,0,0]` on every map rvt-open builds today, which makes
            // the two placements the same identity frame anyway.
            let (solid_ids, rep_type) = self.emit_solid_items(&rmap.shape, map_placement, z_axis);
            let items = solid_ids
                .iter()
                .map(|id| format!("#{id}"))
                .collect::<Vec<_>>()
                .join(",");
            let body_rep_id = self.id();
            self.emit_entity(
                body_rep_id,
                format!("IFCSHAPEREPRESENTATION(#{geom_ctx},'Body','{rep_type}',({items}))"),
            );
            // IFCREPRESENTATIONMAP(MappingOrigin, MappedRepresentation).
            let rep_map_id = self.id();
            self.emit_entity(
                rep_map_id,
                format!("IFCREPRESENTATIONMAP(#{map_placement},#{body_rep_id})"),
            );
            representation_map_ids.push(rep_map_id);
            // the type's 2D maps beside its Body map.
            let mut plan_maps = Vec::new();
            for shape in &rmap.plan {
                let Some(&ctx) = plan_ctx_ids.get(shape.context.as_str()) else {
                    continue;
                };
                let mut curve_ids = Vec::new();
                for curve in &shape.curves {
                    if curve.points_feet.len() < 2 {
                        continue;
                    }
                    let coords: Vec<String> = curve
                        .points_feet
                        .iter()
                        .map(|p| {
                            format!(
                                "({},{},{})",
                                format_real(p[0] * 0.3048),
                                format_real(p[1] * 0.3048),
                                format_real(p[2] * 0.3048)
                            )
                        })
                        .collect();
                    let list = self.id();
                    self.emit_entity(
                        list,
                        // IFC4 (ADD2 TC1) IfcCartesianPointList3D has one attribute,
                        // CoordList; TagList arrived in IFC4.3.
                        format!("IFCCARTESIANPOINTLIST3D(({}))", coords.join(",")),
                    );
                    let segments = if curve.arc && curve.points_feet.len() == 3 {
                        "(IFCARCINDEX((1,2,3)))".to_string()
                    } else {
                        "$".to_string()
                    };
                    let id = self.id();
                    self.emit_entity(id, format!("IFCINDEXEDPOLYCURVE(#{list},{segments},.F.)"));
                    curve_ids.push(format!("#{id}"));
                }
                if curve_ids.is_empty() {
                    continue;
                }
                let (item, rep_type) = match shape.context.as_str() {
                    "Axis" if curve_ids.len() == 1 => (curve_ids[0].clone(), "Curve3D"),
                    "Axis" | "FootPrint" => {
                        let set = self.id();
                        self.emit_entity(
                            set,
                            format!("IFCGEOMETRICCURVESET(({}))", curve_ids.join(",")),
                        );
                        (format!("#{set}"), "GeometricCurveSet")
                    }
                    _ => {
                        let set = self.id();
                        self.emit_entity(
                            set,
                            format!("IFCGEOMETRICSET(({}))", curve_ids.join(",")),
                        );
                        (format!("#{set}"), "GeometricSet")
                    }
                };
                let rep = self.id();
                self.emit_entity(
                    rep,
                    format!(
                        "IFCSHAPEREPRESENTATION(#{ctx},'{}','{rep_type}',({item}))",
                        shape.identifier
                    ),
                );
                let map = self.id();
                self.emit_entity(
                    map,
                    format!("IFCREPRESENTATIONMAP(#{map_placement},#{rep})"),
                );
                plan_maps.push((shape.identifier.clone(), ctx, map));
            }
            plan_map_ids.push(plan_maps);
        }

        let mut per_storey_elements: Vec<Vec<usize>> = vec![Vec::new(); storeys.len()];
        // elements no storey join reached. They are contained in
        // the `IfcBuilding` rather than in whichever storey happens to
        // be first — "somewhere in this building" is the honest claim,
        // and putting them on a named storey would state a containment
        // nothing measured.
        let mut unplaced_elements: Vec<usize> = Vec::new();
        // A space is a spatial element, not a contained one: it is an
        // aggregated part of its storey (or of the building when no storey
        // join reached it), never in `IfcRelContainedInSpatialStructure`
        // (buildingSMART validation rules SPS002 and SPS007).
        let mut per_storey_spaces: Vec<Vec<usize>> = vec![Vec::new(); storeys.len()];
        let mut unplaced_spaces: Vec<usize> = Vec::new();
        // The elements a space contains, by the space's entity index;
        // they are left out of their storey's containment.
        let mut per_space_elements: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        // Track (element_id, material_index) pairs so we can emit
        // IfcRelAssociatesMaterial per material after the element
        // loop completes.
        let mut element_material_pairs: Vec<(usize, usize)> = Vec::new();
        // Track (element_id, &PropertySet) so property-set emission
        // runs after all element IDs are assigned.
        let mut element_property_sets: Vec<(usize, &PropertySet)> = Vec::new();
        // Map vec index in model.entities → emitted IFC element id.
        // None when that entity wasn't a BuildingElement. Consulted
        // when resolving `host_element_index` for openings + for
        // IfcRelVoidsElement / IfcRelFillsElement emission.
        let mut entity_index_to_el_id: Vec<Option<usize>> = vec![None; model.entities.len()];
        // Each building element's IfcLocalPlacement, for the openings placed
        // in their host's frame.
        let mut entity_index_to_placement: Vec<Option<usize>> = vec![None; model.entities.len()];
        // Void/fill tracking: for each element with a host, note
        // (host_el_id, opening_el_id, element_el_id) so the rels can
        // be emitted after the element loop. Openings themselves are
        // also BuildingElements (IfcOpeningElement) but they're never
        // contained in a storey — IFC4 treats them as "virtual"
        // elements that only live through IfcRelVoidsElement.
        let mut void_fill_triples: Vec<(usize, usize, Option<usize>, String)> = Vec::new();
        //: each wall's `Qto_WallBaseQuantities.Width`, the
        // `Depth` of the openings its doors and windows cut, as Revit's export
        // gives every one of them.
        let wall_widths: HashMap<usize, f64> = model
            .entities
            .iter()
            .filter_map(|entity| match entity {
                super::entities::IfcEntity::ElementPropertySet { element, set }
                    if set.name == "Qto_WallBaseQuantities" =>
                {
                    set.properties
                        .iter()
                        .find_map(|property| match &property.value {
                            super::entities::PropertyValue::PositiveLengthFeet(width)
                                if property.name == "Width" =>
                            {
                                Some((*element, *width))
                            }
                            _ => None,
                        })
                }
                _ => None,
            })
            .collect();
        // parts of an aggregate whose whole is a building element
        // reach the spatial structure through that whole, so they are left
        // out of the storey containment below.
        let is_building_element = |index: usize| {
            matches!(
                model.entities.get(index),
                Some(super::entities::IfcEntity::BuildingElement { .. })
            )
        };
        let aggregate_parts = aggregate_parts_of_elements(model);
        // Where each element goes in the spatial structure: the one decision
        // the server's answers read as well (`placements`).
        let element_placements = placements(model);
        // each building element's GlobalId, fixed from what the
        // element is rather than where it lands in the file. Revit's own
        // where the file yields it; otherwise the element's
        // ElementId (its `Tag`) and which of the entities sharing that Tag
        // it is — a slab's further pieces or a curtain wall's parts
        // share their element's. An element with no Tag falls back to its
        // entity type and name. Everything the element owns (its opening,
        // property sets, material association) derives from this one.
        let element_gids = building_element_global_ids(model);
        // the further pieces of an element of separate loops that are
        // written as items of its body: where the element and the piece are
        // both plain extrusions, or neither has geometry. Any other piece is
        // written as an element of its own.
        let (piece_bodies, folded_pieces) = fold_body_pieces(model);
        let mut el_id_to_gid: HashMap<usize, &str> = HashMap::new();
        // mapped-item targets already written, by their text.
        let mut target_ops: HashMap<String, usize> = Default::default();
        let mut identity_op: Option<usize> = None;
        for (entity_idx, entity) in model.entities.iter().enumerate() {
            if folded_pieces.contains(&entity_idx) {
                continue;
            }
            if let super::entities::IfcEntity::BuildingElement {
                ifc_type,
                name,
                type_guid,
                predefined_type,
                storey_index,
                material_index,
                property_set,
                location_feet,
                rotation_radians,
                extrusion,
                host_element_index,
                material_layer_set_index,
                material_profile_set_index,
                solid_shape,
                representation_map_index,
            } = entity
            {
                // An out-of-range index is a caller bug; treat it as
                // unbound rather than silently dropping the element or
                // pinning it to an unrelated storey.
                let idx = storey_index.filter(|index| *index < storeys.len());
                let placement_parent = match idx {
                    Some(index) => storey_placements[index],
                    None => building_placement,
                };
                // Decide whether to emit a per-element axis placement
                // (with real origin + rotation) or share the project-
                // level identity placement. Sharing keeps byte counts
                // down for elements where no location has been decoded;
                // unique placements matter once geometry attaches to
                // the element and tools read element.position.
                let element_axis = if let Some([x_ft, y_ft, z_ft]) = location_feet {
                    let x_m = x_ft * 0.3048;
                    let y_m = y_ft * 0.3048;
                    let z_m = z_ft * 0.3048;
                    let point_id = self.id();
                    self.emit_entity(
                        point_id,
                        format!("IFCCARTESIANPOINT(({x_m:.6},{y_m:.6},{z_m:.6}))"),
                    );
                    // Rotation is applied about Z only — all Revit
                    // element placements we've seen so far are upright
                    // with yaw-only rotation. Full 3D rotation needs
                    // the BasePoint / ProjectPosition transform chain
                    // (already decoded, not yet threaded here).
                    let x_axis_id = if let Some(angle) = rotation_radians {
                        let cx = angle.cos();
                        let cy = angle.sin();
                        let dir = self.id();
                        self.emit_entity(dir, format!("IFCDIRECTION(({cx:.6},{cy:.6},0.))"));
                        Some(dir)
                    } else {
                        None
                    };
                    let axis_id = self.id();
                    // If we have a yaw rotation, reference our own X-
                    // axis IfcDirection; otherwise reuse the shared
                    // +X axis_placement points at (via `x_axis`).
                    let axis_body = match x_axis_id {
                        Some(d) => format!("IFCAXIS2PLACEMENT3D(#{point_id},#{z_axis},#{d})"),
                        None => format!("IFCAXIS2PLACEMENT3D(#{point_id},#{z_axis},#{x_axis})"),
                    };
                    self.emit_entity(axis_id, axis_body);
                    axis_id
                } else {
                    axis_placement
                };
                let placement_id = self.id();
                self.emit_entity(
                    placement_id,
                    format!("IFCLOCALPLACEMENT(#{placement_parent},#{element_axis})"),
                );
                // The element translation is carried exactly once, by
                // the `IfcLocalPlacement` above. Every swept solid this
                // element emits is authored in the element-local frame
                // (profiles are centred on their own origin), so its
                // `Position` is the project-level identity placement —
                // reusing `element_axis` there made a conforming
                // consumer composing `ObjectPlacement × Position` apply
                // the translation twice. Revit's own exporter
                // splits it the same way: on
                // `2024_Core_Interior_slim.ifc` the `IfcWall`
                // `SweptSolid` bodies sit at `Position = (0,0,0)` with
                // the wall's `IfcLocalPlacement` carrying the location,
                // and the `IfcSlab` / `IfcSpace` bodies do the mirror
                // image — an identity placement with the frame in
                // `Position`. Never both.
                let solid_position = axis_placement;

                // Emit the extrusion chain when geometry is present.
                // Chain: IfcProfileDef subclass → IfcExtrudedAreaSolid
                // → IfcShapeRepresentation → IfcProductDefinitionShape.
                // Profile placement uses a single fresh 2D axis per
                // element (profile-local XY frame centred on origin).
                // Precedence (highest wins):
                //   1. representation_map_index (IFC-21 mapped item)
                //   2. solid_shape              (IFC-18/19/20 solid)
                //   3. extrusion                (IFC-16 extruded area)
                //   4. none                     (Representation = $)
                let shape_ref = if let Some(rm_idx) = representation_map_index {
                    representation_map_ids.get(*rm_idx).map(|rep_map_id| {
                        // Mapped item: the instance's own
                        // transformation operator (identity for now —
                        // the element's own IfcLocalPlacement carries
                        // the instance position, and the mapped item
                        // rides on top of that).
                        // one identity operator for every
                        // mapped item that needs no transform.
                        let tx_op = match identity_op {
                            Some(op) => op,
                            None => {
                                let origin = self.id();
                                self.emit_entity(origin, "IFCCARTESIANPOINT((0.,0.,0.))");
                                // IFC4 uses IfcCartesianTransformationOperator3D
                                // for the mapped-target transform. Identity =
                                // ($,$,ORIGIN,$,$) per spec.
                                let op = self.id();
                                self.emit_entity(
                                    op,
                                    format!("IFCCARTESIANTRANSFORMATIONOPERATOR3D($,$,#{origin},$,$)"),
                                );
                                identity_op = Some(op);
                                op
                            }
                        };
                        // an instance of a symbol mesh maps it
                        // through its own transform (a rotation: the axes
                        // are its columns). The 2D maps keep the identity.
                        let plan_op = tx_op;
                        let tx_op = match model.instancing.targets.get(&entity_idx) {
                            Some(t) => {
                                let text = |t: &super::instancing::Frame| {
                                    let dir = |k: usize| {
                                        format!("({:.15},{:.15},{:.15})", t[0][k], t[1][k], t[2][k])
                                            .replace("-0.000000000000000", "0.000000000000000")
                                    };
                                    let origin = format!(
                                        "({:.12},{:.12},{:.12})",
                                        t[0][3] * 0.3048,
                                        t[1][3] * 0.3048,
                                        t[2][3] * 0.3048
                                    )
                                    .replace("-0.000000000000", "0.000000000000");
                                    (dir(0), dir(1), dir(2), origin)
                                };
                                let (d1, d2, d3, origin) = text(t);
                                let key = format!("{d1}{d2}{origin}{d3}");
                                // A target that writes as the identity: the plain operator.
                                if text(&super::instancing::IDENTITY) == (d1.clone(), d2.clone(), d3.clone(), origin.clone()) {
                                    tx_op
                                } else if let Some(&op) = target_ops.get(&key) {
                                    op
                                } else {
                                    let mut emit = |text: String| {
                                        let id = self.id();
                                        self.emit_entity(id, text);
                                        id
                                    };
                                    let a1 = emit(format!("IFCDIRECTION({d1})"));
                                    let a2 = emit(format!("IFCDIRECTION({d2})"));
                                    let a3 = emit(format!("IFCDIRECTION({d3})"));
                                    let o = emit(format!("IFCCARTESIANPOINT({origin})"));
                                    let op = emit(format!(
                                        "IFCCARTESIANTRANSFORMATIONOPERATOR3D(#{a1},#{a2},#{o},$,#{a3})"
                                    ));
                                    target_ops.insert(key, op);
                                    op
                                }
                            }
                            None => tx_op,
                        };
                        let mapped_item = self.id();
                        self.emit_entity(
                            mapped_item,
                            format!("IFCMAPPEDITEM(#{rep_map_id},#{tx_op})"),
                        );
                        // The instance's IfcShapeRepresentation wraps
                        // the mapped item with representation-type
                        // 'MappedRepresentation' (IFC4 convention for
                        // type-instance shared geometry).
                        let rep_id = self.id();
                        self.emit_entity(
                            rep_id,
                            format!(
                                "IFCSHAPEREPRESENTATION(#{geom_ctx},'Body','MappedRepresentation',(#{mapped_item}))"
                            ),
                        );
                        // and the type's 2D maps, each its own
                        // mapped representation.
                        let mut reps = vec![format!("#{rep_id}")];
                        for (ident, ctx, plan_map) in
                            plan_map_ids.get(*rm_idx).map(Vec::as_slice).unwrap_or_default()
                        {
                            let item = self.id();
                            self.emit_entity(item, format!("IFCMAPPEDITEM(#{plan_map},#{plan_op})"));
                            let rep = self.id();
                            self.emit_entity(
                                rep,
                                format!("IFCSHAPEREPRESENTATION(#{ctx},'{ident}','MappedRepresentation',(#{item}))"),
                            );
                            reps.push(format!("#{rep}"));
                        }
                        let prod_shape_id = self.id();
                        self.emit_entity(
                            prod_shape_id,
                            format!("IFCPRODUCTDEFINITIONSHAPE($,$,({}))", reps.join(",")),
                        );
                        prod_shape_id
                    })
                } else if let Some(shape) = solid_shape {
                    // A wall drawn from a mesh or another solid gets its
                    // `Axis` too, as Revit's export writes one for every
                    // wall: on its location line, end to end of the body.
                    let mut reps = Vec::new();
                    if let (Some(line), Some(&axis_ctx), Some(location)) = (
                        model.wall_location_lines.get(&entity_idx),
                        plan_ctx_ids.get("Axis"),
                        location_feet,
                    ) {
                        let points = solid_plan_points(shape);
                        if let Some(ends) =
                            wall_axis_local(*line, *location, *rotation_radians, &points)
                        {
                            reps.push(format!("#{}", self.emit_wall_axis(axis_ctx, ends)));
                        }
                    }
                    let (solid_ids, rep_type) =
                        self.emit_solid_items(shape, solid_position, z_axis);
                    let items = solid_ids
                        .iter()
                        .map(|id| format!("#{id}"))
                        .collect::<Vec<_>>()
                        .join(",");
                    let rep_id = self.id();
                    self.emit_entity(
                        rep_id,
                        format!(
                            "IFCSHAPEREPRESENTATION(#{geom_ctx},'Body','{rep_type}',({items}))"
                        ),
                    );
                    reps.push(format!("#{rep_id}"));
                    let prod_shape_id = self.id();
                    self.emit_entity(
                        prod_shape_id,
                        format!("IFCPRODUCTDEFINITIONSHAPE($,$,({}))", reps.join(",")),
                    );
                    Some(prod_shape_id)
                } else if let Some(ex) = extrusion {
                    let depth = ex.height_feet * 0.3048;
                    // IfcProfileDef has a 2D placement; we emit a
                    // fresh 2D origin + direction + 2D axis per
                    // extrusion. Sharing a single 2D placement across
                    // all extrusions would be possible but muddies
                    // byte-by-byte diff tooling, so pay the ~3-entity
                    // cost for clarity.
                    // An opening standing for a door or window takes
                    // the same cut to its host's depth as a filled one.
                    let own_cut = (ifc_type == "IFCOPENINGELEMENT")
                        .then(|| model.opening_cuts.get(&entity_idx))
                        .flatten();
                    let [cx, cy] = own_cut.map_or([0.0, 0.0], |c| c.centre_feet);
                    let profile_origin = self.id();
                    self.emit_entity(
                        profile_origin,
                        format!("IFCCARTESIANPOINT(({:.6},{:.6}))", cx * 0.3048, cy * 0.3048),
                    );
                    let profile_x_axis = self.id();
                    self.emit_entity(profile_x_axis, "IFCDIRECTION((1.,0.))");
                    let profile_placement = self.id();
                    self.emit_entity(
                        profile_placement,
                        format!("IFCAXIS2PLACEMENT2D(#{profile_origin},#{profile_x_axis})"),
                    );
                    let profile_id = match own_cut {
                        Some(cut) => {
                            let profile_id = self.id();
                            self.emit_entity(
                                profile_id,
                                format!(
                                    "IFCRECTANGLEPROFILEDEF(.AREA.,$,#{profile_placement},{:.6},{:.6})",
                                    cut.x_dim_feet * 0.3048,
                                    cut.y_dim_feet * 0.3048,
                                ),
                            );
                            profile_id
                        }
                        None => self.emit_profile_def(ex, profile_placement),
                    };
                    // Solid-local placement: the identity axis, so the
                    // extrusion sits at the element origin and the
                    // element's own IfcLocalPlacement moves it into
                    // the world exactly once.
                    let (position, depth) =
                        self.opening_solid_position(solid_position, own_cut, depth);
                    let solid_id = self.id();
                    self.emit_entity(
                        solid_id,
                        format!(
                            "IFCEXTRUDEDAREASOLID(#{profile_id},#{position},#{z_axis},{depth:.6})"
                        ),
                    );
                    // each further piece written in this element's
                    // body, a solid of its own in the same frame.
                    let mut items = vec![format!("#{solid_id}")];
                    for piece in piece_bodies.get(&entity_idx).into_iter().flatten() {
                        let origin = self.id();
                        self.emit_entity(origin, "IFCCARTESIANPOINT((0.000000,0.000000))");
                        let x_axis = self.id();
                        self.emit_entity(x_axis, "IFCDIRECTION((1.,0.))");
                        let placement = self.id();
                        self.emit_entity(
                            placement,
                            format!("IFCAXIS2PLACEMENT2D(#{origin},#{x_axis})"),
                        );
                        let piece_profile = self.emit_profile_def(piece, placement);
                        let piece_solid = self.id();
                        self.emit_entity(
                            piece_solid,
                            format!(
                                "IFCEXTRUDEDAREASOLID(#{piece_profile},#{solid_position},#{z_axis},{:.6})",
                                piece.height_feet * 0.3048
                            ),
                        );
                        items.push(format!("#{piece_solid}"));
                    }
                    // A wall's `Axis` before its body, as Revit's export
                    // writes it: its location line where the body covers it.
                    let mut reps = Vec::new();
                    if let (Some(line), Some(&axis_ctx), Some(location)) = (
                        model.wall_location_lines.get(&entity_idx),
                        plan_ctx_ids.get("Axis"),
                        location_feet,
                    ) {
                        if let Some(ends) = extrusion_plan_points(ex).and_then(|points| {
                            wall_axis_local(*line, *location, *rotation_radians, &points)
                        }) {
                            reps.push(format!("#{}", self.emit_wall_axis(axis_ctx, ends)));
                        }
                    }
                    // The representation groups the solid inside the
                    // project's 3D geometric context (#geom_ctx) with
                    // the IFC4-standard identifier + type for a
                    // swept-solid body.
                    let rep_id = self.id();
                    self.emit_entity(
                        rep_id,
                        format!(
                            "IFCSHAPEREPRESENTATION(#{geom_ctx},'Body','SweptSolid',({}))",
                            items.join(",")
                        ),
                    );
                    reps.push(format!("#{rep_id}"));
                    let prod_shape_id = self.id();
                    self.emit_entity(
                        prod_shape_id,
                        format!("IFCPRODUCTDEFINITIONSHAPE($,$,({}))", reps.join(",")),
                    );
                    Some(prod_shape_id)
                } else {
                    None
                };

                let el_id = self.id();
                // The Name and LongName of an element, a space's number and
                // room name (`written_names`).
                let names =
                    written_names(ifc_type, name, type_guid.as_deref(), property_set.as_ref());
                let name_quoted = quoted_or_dollar(&escape(&names.name));
                let long_name_quoted = names
                    .long_name
                    .as_deref()
                    .map_or_else(|| "$".into(), |text| quoted_or_dollar(&escape(text)));
                // `ObjectType` is `Family:Type` when the reader named the
                // element's family and type, as in Revit's own export.
                let property_text = |key: &str| {
                    property_set.as_ref().and_then(|set| {
                        set.properties.iter().find_map(|p| match &p.value {
                            super::entities::PropertyValue::Text(text) if p.name == key => {
                                Some(text.clone())
                            }
                            _ => None,
                        })
                    })
                };
                let object_type_quoted = match (
                    property_text(super::export_content::FAMILY_NAME_PROPERTY),
                    property_text(super::export_content::TYPE_NAME_PROPERTY),
                ) {
                    (Some(family), Some(type_name)) => {
                        quoted_or_dollar(&escape(&format!("{family}:{type_name}")))
                    }
                    _ => "$".into(),
                };
                let tag_quoted = type_guid
                    .as_deref()
                    .map(escape)
                    .map(|t| format!("'{t}'"))
                    .unwrap_or_else(|| "$".into());
                let rep_slot = match shape_ref {
                    Some(id) => format!("#{id}"),
                    None => "$".into(),
                };
                // Every IFC4 instance carries every attribute its
                // entity type declares — an unknown optional is `$`,
                // never an omitted slot. The shared head is
                // IfcProduct's:
                //   (GlobalId, OwnerHist, Name, Desc, ObjectType,
                //    ObjectPlacement, Representation)
                // and `element_attribute_tail` supplies the rest for
                // the type, so a reader can index PredefinedType
                // instead of hitting "index out of range".
                let ifc_upper = ifc_type.to_ascii_uppercase();
                // A window's or door's OverallHeight and
                // OverallWidth are its opening's, which Revit's export writes
                // there; `IfcModel::filler_sizes` has what they are worked out
                // to.
                let filler_size = model.filler_sizes.get(&entity_idx);
                let overall = filler_size.map(|size| (size.height_feet, size.width_feet));
                // IFC4 CorrectPredefinedType: USERDEFINED needs an
                // `ObjectType`; without a family and type name it is
                // NOTDEFINED (an untyped entourage).
                let predefined = match predefined_type.as_deref() {
                    Some("USERDEFINED") if object_type_quoted == "$" => Some("NOTDEFINED"),
                    other => other,
                };
                let tail = element_attribute_tail(
                    &ifc_upper,
                    &tag_quoted,
                    predefined,
                    &long_name_quoted,
                    overall,
                );
                let global_id: &str = element_gids[entity_idx]
                    .as_deref()
                    .expect("every building element has a GlobalId");
                el_id_to_gid.insert(el_id, global_id);
                let line = format!(
                    "{ifc_upper}('{global_id}',#{owner_hist},{name_quoted},$,{object_type_quoted},#{placement_id},{rep_slot},{tail})",
                );
                self.emit_entity(el_id, line);
                // An opening standing for a door or window that draws
                // no geometry voids its host and fills nothing. Like every
                // opening it lives through that relation, not in a storey.
                // Under a convention that contains family spaces, a space
                // with no spatial kind (a family instance written as one) is
                // contained like an element.
                let is_space = ifc_upper == "IFCSPACE"
                    && !(model.convention.contains_family_spaces()
                        && !property_set.as_ref().is_some_and(|set| {
                            set.properties
                                .iter()
                                .any(|p| p.name == super::export_content::SPATIAL_KIND_PROPERTY)
                        }));
                let hosted_opening = matches!(
                    element_placements[entity_idx],
                    Some(Placement::Opening { .. })
                );
                match element_placements[entity_idx] {
                    Some(Placement::Opening { host }) => {
                        void_fill_triples.push((host, el_id, None, global_id.to_string()));
                    }
                    Some(Placement::InSpace(space)) => {
                        per_space_elements.entry(space).or_default().push(el_id);
                    }
                    Some(Placement::OnStorey(index)) if is_space => {
                        per_storey_spaces[index].push(el_id);
                    }
                    Some(Placement::OnStorey(index)) => per_storey_elements[index].push(el_id),
                    Some(Placement::InBuilding) if is_space => unplaced_spaces.push(el_id),
                    Some(Placement::InBuilding) => unplaced_elements.push(el_id),
                    // A part reaches the structure through its whole.
                    Some(Placement::PartOf(_) | Placement::Folded) | None => {}
                }
                entity_index_to_el_id[entity_idx] = Some(el_id);
                entity_index_to_placement[entity_idx] = Some(placement_id);
                // IFC-30 / IFC-28: precedence order for material
                // association is profile_set > layer_set > single
                // material. Try each in order; `profile_or_layer_applied`
                // carries the "already emitted" flag through so the
                // single-material path below doesn't double-associate.
                //
                // An IfcSpace is none of the entities IFC4's
                // IfcRelAssociatesMaterial.AllowedElements takes (elements,
                // element types, door and window styles, structural members
                // and ports), so a space is associated with no material,
                // though Revit's own export associates one, which
                // `write_step_with_space_materials` also does.
                let holds_material = associates_material(ifc_type) || self.space_materials;
                let material_index = material_index.as_ref().filter(|_| holds_material);
                let material_profile_set_index = material_profile_set_index
                    .as_ref()
                    .filter(|_| holds_material);
                let material_layer_set_index =
                    material_layer_set_index.as_ref().filter(|_| holds_material);
                let profile_set_applied = if let Some(ps_idx) = material_profile_set_index {
                    if let Some(&ps_id) = profile_set_ids.get(*ps_idx) {
                        let usage_id = self.id();
                        // IfcMaterialProfileSetUsage(ForProfileSet,
                        //   CardinalPoint=5, ReferenceExtent=$). In IFC4's
                        // IfcCardinalPointReference, 5 is the mid-depth
                        // centre: the profile is centred on the member's
                        // axis, as every profile the exporter writes is.
                        self.emit_entity(
                            usage_id,
                            format!("IFCMATERIALPROFILESETUSAGE(#{ps_id},5,$)"),
                        );
                        let rel_id = self.id();
                        self.emit_entity(
                            rel_id,
                            format!(
                                "IFCRELASSOCIATESMATERIAL('{}',#{owner_hist},$,$,(#{el_id}),#{usage_id})",
                                gid(&["material-association", global_id]),
                            ),
                        );
                        true
                    } else {
                        false
                    }
                } else {
                    false
                };
                // IFC-28: material_layer_set_index falls through when
                // profile-set didn't apply. Emits IfcMaterialLayerSetUsage
                // + IfcRelAssociatesMaterial.
                let layer_set_applied = !profile_set_applied
                    && if let Some(ls_idx) = material_layer_set_index {
                        if let Some(&ls_id) = layer_set_ids.get(*ls_idx) {
                            let usage_id = self.id();
                            // IfcMaterialLayerSetUsage(ForLayerSet, LayerSetDirection,
                            //   DirectionSense, OffsetFromReferenceLine, ReferenceExtent).
                            // The element's own usage when the model states
                            // one; otherwise.AXIS2. (the extrusion's
                            // Y axis, a wall's thickness), .POSITIVE. from 0,
                            // the IFC4 defaults most exporters emit.
                            // ReferenceExtent stays `$` — undecoded, but its
                            // slot is still written.
                            let usage = model.material_layer_usages.get(&entity_idx);
                            let direction = match usage.map(|u| u.direction) {
                                Some(super::entities::LayerSetDirection::Axis1) => ".AXIS1.",
                                Some(super::entities::LayerSetDirection::Axis3) => ".AXIS3.",
                                _ => ".AXIS2.",
                            };
                            let sense = if usage.is_none_or(|u| u.positive) {
                                ".POSITIVE."
                            } else {
                                ".NEGATIVE."
                            };
                            let offset = usage.map_or_else(
                                || "0.".to_string(),
                                |u| format!("{:.6}", u.offset_feet * 0.3048),
                            );
                            self.emit_entity(
                                usage_id,
                                format!(
                                    "IFCMATERIALLAYERSETUSAGE(#{ls_id},{direction},{sense},{offset},$)"
                                ),
                            );
                            let rel_id = self.id();
                            self.emit_entity(
                            rel_id,
                            format!(
                                "IFCRELASSOCIATESMATERIAL('{}',#{owner_hist},$,$,(#{el_id}),#{usage_id})",
                                gid(&["material-association", global_id]),
                            ),
                        );
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                if !profile_set_applied && !layer_set_applied {
                    if let Some(m_idx) = material_index {
                        if *m_idx < material_ids.len() {
                            element_material_pairs.push((el_id, *m_idx));
                        }
                    } else if let Some(&set) = constituent_set_of
                        .get(&entity_idx)
                        .filter(|_| holds_material)
                    {
                        element_constituent_pairs.push((el_id, set));
                    }
                }
                if let Some(pset) = property_set {
                    if !pset.properties.is_empty() {
                        element_property_sets.push((el_id, pset));
                    }
                }
                // Void / fill chain: when the element has both an
                // extrusion (describes its volume) and a host_element_
                // index (points at its parent wall/floor), emit an
                // IfcOpeningElement matching the shape + the two
                // binding relationships.
                if let (false, Some(h_idx), Some(ex)) =
                    (hosted_opening, host_element_index, extrusion)
                {
                    {
                        // Emit a second extrusion chain — same shape
                        // as the element, placed on the element's
                        // placement so the void sits where the door
                        // sits. We reuse the element's placement for
                        // simplicity; IFC4 validators accept this. The
                        // solid's own `Position` stays identity for the
                        // same reason it does on the filling element
                        // — the opening's IfcLocalPlacement
                        // below already carries `element_axis`.
                        // cut to the host wall's thickness where it is
                        // known, else the element's own body.
                        let cut = model.opening_cuts.get(&entity_idx);
                        let x_dim = cut.map_or(ex.width_feet, |c| c.x_dim_feet) * 0.3048;
                        let y_dim = cut.map_or(ex.depth_feet, |c| c.y_dim_feet) * 0.3048;
                        let [cx, cy] = cut.map_or([0.0, 0.0], |c| c.centre_feet);
                        let depth_m = ex.height_feet * 0.3048;
                        let o_profile_origin = self.id();
                        self.emit_entity(
                            o_profile_origin,
                            format!("IFCCARTESIANPOINT(({:.6},{:.6}))", cx * 0.3048, cy * 0.3048),
                        );
                        let o_profile_x = self.id();
                        self.emit_entity(o_profile_x, "IFCDIRECTION((1.,0.))");
                        let o_profile_place = self.id();
                        self.emit_entity(
                            o_profile_place,
                            format!("IFCAXIS2PLACEMENT2D(#{o_profile_origin},#{o_profile_x})"),
                        );
                        let o_profile = self.id();
                        self.emit_entity(
                            o_profile,
                            format!(
                                "IFCRECTANGLEPROFILEDEF(.AREA.,$,#{o_profile_place},{x_dim:.6},{y_dim:.6})"
                            ),
                        );
                        let (position, depth_m) =
                            self.opening_solid_position(solid_position, cut, depth_m);
                        let o_solid = self.id();
                        self.emit_entity(
                            o_solid,
                            format!(
                                "IFCEXTRUDEDAREASOLID(#{o_profile},#{position},#{z_axis},{depth_m:.6})"
                            ),
                        );
                        let o_rep = self.id();
                        self.emit_entity(
                            o_rep,
                            format!(
                                "IFCSHAPEREPRESENTATION(#{geom_ctx},'Body','SweptSolid',(#{o_solid}))"
                            ),
                        );
                        let o_prod_shape = self.id();
                        self.emit_entity(
                            o_prod_shape,
                            format!("IFCPRODUCTDEFINITIONSHAPE($,$,(#{o_rep}))"),
                        );
                        // IfcOpeningElement uses its own placement
                        // (relative to the host wall's) — we reuse the
                        // door/window placement.
                        let o_placement = self.id();
                        self.emit_entity(
                            o_placement,
                            format!("IFCLOCALPLACEMENT(#{placement_parent},#{element_axis})"),
                        );
                        let opening_id = self.id();
                        // an opening exists only because the element
                        // filling it does, so its identity is that element's.
                        let opening_gid = gid(&["opening", global_id]);
                        // IfcOpeningElement: (GlobalId, OwnerHist, Name,
                        //   Desc, ObjectType, Placement, Rep, Tag,
                        //   PredefinedType). IFC4 adds PredefinedType.
                        //
                        // `Tag` repeats the filling element's own tag.
                        // That is what Revit's exporter does — on
                        // `2024_Core_Interior_slim.ifc` all 138
                        // door/window openings carry the Tag of the
                        // element that fills them — and it invents
                        // nothing: the opening exists only because
                        // that element does.
                        self.emit_entity(
                            opening_id,
                            format!(
                                "IFCOPENINGELEMENT('{opening_gid}',#{owner_hist},'Opening for {name_esc}',$,$,#{o_placement},#{o_prod_shape},{tag_quoted},.OPENING.)",
                                name_esc = escape(name),
                            ),
                        );
                        // Only where its symbol cuts an outline,
                        // which Revit's export draws the opening
                        // as an extrusion of; with no cut loop it writes no
                        // quantities on the opening.
                        let extruded = filler_size.is_some_and(|size| size.cut_loop);
                        if let (true, Some(&width)) = (extruded, wall_widths.get(h_idx)) {
                            let quantities = PropertySet {
                                name: "Qto_OpeningElementBaseQuantities".into(),
                                properties: vec![super::entities::Property {
                                    name: "Depth".into(),
                                    value: super::entities::PropertyValue::PositiveLengthFeet(
                                        width,
                                    ),
                                }],
                            };
                            let set_key = [opening_gid.as_str(), quantities.name.as_str(), "0"]
                                .join("\u{1f}");
                            self.emit_property_set(
                                owner_hist,
                                opening_id,
                                &quantities,
                                &gid(&["property-set", &set_key]),
                                &gid(&["defines-by-properties", &set_key]),
                            );
                        }
                        void_fill_triples.push((*h_idx, opening_id, Some(el_id), opening_gid));
                    }
                }
            }
        }

        // Each element's type name, for the openings of its own
        // geometry below.
        let host_references = super::host_references(model);

        // An opening nothing fills, a hole in a floor's sketch.
        // As in Revit's export it is placed in its host's frame, its outline
        // extruded up through the host, and it voids the host without being
        // contained in a storey.: so is the opening a face-hosted door
        // or window cuts in its host, numbered among the host's openings.
        // Either is the host's own geometry, so, as Revit's export
        // writes it, its Pset_OpeningElementCommon.Reference is the host's.
        for entity in &model.entities {
            let (host, tag, name, outline_feet, depth_feet, number) = match entity {
                super::entities::IfcEntity::VoidOpening {
                    host,
                    tag,
                    name,
                    outline_feet,
                    depth_feet,
                } => (host, tag, name, outline_feet, depth_feet, None),
                super::entities::IfcEntity::HostedCutOpening {
                    host,
                    number,
                    tag,
                    name,
                    outline_feet,
                    depth_feet,
                } => (host, tag, name, outline_feet, depth_feet, Some(*number)),
                _ => continue,
            };
            let (Some(Some(host_el_id)), Some(Some(host_placement))) = (
                entity_index_to_el_id.get(*host),
                entity_index_to_placement.get(*host),
            ) else {
                continue;
            };
            let profile_origin = self.id();
            self.emit_entity(profile_origin, "IFCCARTESIANPOINT((0.,0.))");
            let profile_x_axis = self.id();
            self.emit_entity(profile_x_axis, "IFCDIRECTION((1.,0.))");
            let profile_placement = self.id();
            self.emit_entity(
                profile_placement,
                format!("IFCAXIS2PLACEMENT2D(#{profile_origin},#{profile_x_axis})"),
            );
            let body = Extrusion {
                width_feet: 0.0,
                depth_feet: 0.0,
                height_feet: *depth_feet,
                profile_override: Some(super::entities::ProfileDef::ArbitraryClosed {
                    points: outline_feet.clone(),
                }),
            };
            let profile_id = self.emit_profile_def(&body, profile_placement);
            let solid_id = self.id();
            self.emit_entity(
                solid_id,
                format!(
                    "IFCEXTRUDEDAREASOLID(#{profile_id},#{axis_placement},#{z_axis},{:.6})",
                    depth_feet * 0.3048
                ),
            );
            let rep_id = self.id();
            self.emit_entity(
                rep_id,
                format!("IFCSHAPEREPRESENTATION(#{geom_ctx},'Body','SweptSolid',(#{solid_id}))"),
            );
            let prod_shape_id = self.id();
            self.emit_entity(
                prod_shape_id,
                format!("IFCPRODUCTDEFINITIONSHAPE($,$,(#{rep_id}))"),
            );
            // Its own identity axis, so the solid's Position is never also the
            // placement's.
            let opening_axis = self.id();
            self.emit_entity(opening_axis, format!("IFCAXIS2PLACEMENT3D(#{origin},$,$)"));
            let placement_id = self.id();
            self.emit_entity(
                placement_id,
                format!("IFCLOCALPLACEMENT(#{host_placement},#{opening_axis})"),
            );
            let host_gid = el_id_to_gid.get(host_el_id).copied().unwrap_or_default();
            // Revit's own GlobalId, Revit's IFC exporter `OpeningUtil`'s hash of
            // `IfcOpeningElement:<number>:<host GlobalId>` (CLEANROOM.md
            // source 5): all four of the BIM4LCA office roof's.
            let opening_gid = match number {
                Some(number) => crate::revit_global_ids::hashed_global_id(&format!(
                    "IfcOpeningElement:{number}:{host_gid}"
                )),
                None => gid(&["void-opening", host_gid, tag]),
            };
            let opening_id = self.id();
            self.emit_entity(
                opening_id,
                format!(
                    "IFCOPENINGELEMENT('{opening_gid}',#{owner_hist},{},$,$,#{placement_id},#{prod_shape_id},'{}',.OPENING.)",
                    quoted_or_dollar(&escape(name)),
                    escape(tag),
                ),
            );
            if let Some(reference) = host_references.get(host) {
                let set = PropertySet {
                    name: "Pset_OpeningElementCommon".into(),
                    properties: vec![super::entities::Property {
                        name: "Reference".into(),
                        value: super::entities::PropertyValue::Identifier(reference.clone()),
                    }],
                };
                self.emit_property_set(
                    owner_hist,
                    opening_id,
                    &set,
                    &gid(&[
                        "property-set",
                        opening_gid.as_str(),
                        "Pset_OpeningElementCommon",
                    ]),
                    &gid(&[
                        "defines-by-properties",
                        opening_gid.as_str(),
                        "Pset_OpeningElementCommon",
                    ]),
                );
            }
            void_fill_triples.push((*host, opening_id, None, opening_gid));
        }

        // The opening a wall's edited elevation profile cuts. As in
        // Revit's export it is placed in the wall's frame, its outline in the
        // wall's vertical plane extruded through the wall's thickness.
        for entity in &model.entities {
            let super::entities::IfcEntity::WallProfileOpening {
                host,
                tag,
                name,
                outline_feet,
                axis_turn_radians,
                plane_offset_feet,
                thickness_feet,
            } = entity
            else {
                continue;
            };
            let (Some(Some(host_el_id)), Some(Some(host_placement))) = (
                entity_index_to_el_id.get(*host),
                entity_index_to_placement.get(*host),
            ) else {
                continue;
            };
            let profile_origin = self.id();
            self.emit_entity(profile_origin, "IFCCARTESIANPOINT((0.,0.))");
            let profile_x_axis = self.id();
            self.emit_entity(profile_x_axis, "IFCDIRECTION((1.,0.))");
            let profile_placement = self.id();
            self.emit_entity(
                profile_placement,
                format!("IFCAXIS2PLACEMENT2D(#{profile_origin},#{profile_x_axis})"),
            );
            let body = Extrusion {
                width_feet: 0.0,
                depth_feet: 0.0,
                height_feet: *thickness_feet,
                profile_override: Some(super::entities::ProfileDef::ArbitraryClosed {
                    points: outline_feet.clone(),
                }),
            };
            let profile_id = self.emit_profile_def(&body, profile_placement);
            // The profile's X runs along the wall and its Y up; it is extruded
            // from the plane's far face back through the wall.
            let (sin, cos) = axis_turn_radians.sin_cos();
            let start = (plane_offset_feet + thickness_feet / 2.0) * 0.3048;
            let solid_origin = self.id();
            self.emit_entity(
                solid_origin,
                format!(
                    "IFCCARTESIANPOINT(({:.6},{:.6},0.))",
                    -sin * start,
                    cos * start
                ),
            );
            let solid_axis = self.id();
            self.emit_entity(
                solid_axis,
                format!("IFCDIRECTION(({sin:.9},{:.9},0.))", -cos),
            );
            let solid_x = self.id();
            self.emit_entity(solid_x, format!("IFCDIRECTION(({cos:.9},{sin:.9},0.))"));
            let solid_position = self.id();
            self.emit_entity(
                solid_position,
                format!("IFCAXIS2PLACEMENT3D(#{solid_origin},#{solid_axis},#{solid_x})"),
            );
            let solid_id = self.id();
            self.emit_entity(
                solid_id,
                format!(
                    "IFCEXTRUDEDAREASOLID(#{profile_id},#{solid_position},#{z_axis},{:.6})",
                    thickness_feet * 0.3048
                ),
            );
            let rep_id = self.id();
            self.emit_entity(
                rep_id,
                format!("IFCSHAPEREPRESENTATION(#{geom_ctx},'Body','SweptSolid',(#{solid_id}))"),
            );
            let prod_shape_id = self.id();
            self.emit_entity(
                prod_shape_id,
                format!("IFCPRODUCTDEFINITIONSHAPE($,$,(#{rep_id}))"),
            );
            let opening_axis = self.id();
            self.emit_entity(opening_axis, format!("IFCAXIS2PLACEMENT3D(#{origin},$,$)"));
            let placement_id = self.id();
            self.emit_entity(
                placement_id,
                format!("IFCLOCALPLACEMENT(#{host_placement},#{opening_axis})"),
            );
            let host_gid = el_id_to_gid.get(host_el_id).copied().unwrap_or_default();
            let opening_gid = gid(&["wall-profile-opening", host_gid, tag]);
            let opening_id = self.id();
            self.emit_entity(
                opening_id,
                format!(
                    "IFCOPENINGELEMENT('{opening_gid}',#{owner_hist},{},$,$,#{placement_id},#{prod_shape_id},'{}',.OPENING.)",
                    quoted_or_dollar(&escape(name)),
                    escape(tag),
                ),
            );
            void_fill_triples.push((*host, opening_id, None, opening_gid));
        }

        // An opening an element cuts in a wall besides the one a door
        // or window fills: a rectangular wall opening's, a cabinet's, a door's
        // in a second wall or a curtain wall's. Placed in the wall's frame as
        // the opening of its elevation profile is, tagged with the cutter's
        // ElementId and named for the wall. Its Reference is the cutter's
        // type name, and its GlobalId Revit's where the cutter is an element
        // of the export: Revit's IFC exporter's `OpeningUtil` hashes its number in the
        // wall's openings with the wall's and the cutter's GlobalIds
        // (CLEANROOM.md source 5).
        for entity in &model.entities {
            let super::entities::IfcEntity::WallCutOpening {
                host,
                number,
                tag,
                name,
                cutter_global_id,
                reference_from,
                outline_feet,
                axis_turn_radians,
                plane_offset_feet,
                thickness_feet,
                recess,
                quantities: with_quantities,
            } = entity
            else {
                continue;
            };
            let (Some(Some(host_el_id)), Some(Some(host_placement))) = (
                entity_index_to_el_id.get(*host),
                entity_index_to_placement.get(*host),
            ) else {
                continue;
            };
            let profile_origin = self.id();
            self.emit_entity(profile_origin, "IFCCARTESIANPOINT((0.,0.))");
            let profile_x_axis = self.id();
            self.emit_entity(profile_x_axis, "IFCDIRECTION((1.,0.))");
            let profile_placement = self.id();
            self.emit_entity(
                profile_placement,
                format!("IFCAXIS2PLACEMENT2D(#{profile_origin},#{profile_x_axis})"),
            );
            let body = Extrusion {
                width_feet: 0.0,
                depth_feet: 0.0,
                height_feet: *thickness_feet,
                profile_override: Some(super::entities::ProfileDef::ArbitraryClosed {
                    points: outline_feet.clone(),
                }),
            };
            let profile_id = self.emit_profile_def(&body, profile_placement);
            let (sin, cos) = axis_turn_radians.sin_cos();
            let start = (plane_offset_feet + thickness_feet / 2.0) * 0.3048;
            let solid_origin = self.id();
            self.emit_entity(
                solid_origin,
                format!(
                    "IFCCARTESIANPOINT(({:.6},{:.6},0.))",
                    -sin * start,
                    cos * start
                ),
            );
            let solid_axis = self.id();
            self.emit_entity(
                solid_axis,
                format!("IFCDIRECTION(({sin:.9},{:.9},0.))", -cos),
            );
            let solid_x = self.id();
            self.emit_entity(solid_x, format!("IFCDIRECTION(({cos:.9},{sin:.9},0.))"));
            let solid_position = self.id();
            self.emit_entity(
                solid_position,
                format!("IFCAXIS2PLACEMENT3D(#{solid_origin},#{solid_axis},#{solid_x})"),
            );
            let solid_id = self.id();
            self.emit_entity(
                solid_id,
                format!(
                    "IFCEXTRUDEDAREASOLID(#{profile_id},#{solid_position},#{z_axis},{:.6})",
                    thickness_feet * 0.3048
                ),
            );
            let rep_id = self.id();
            self.emit_entity(
                rep_id,
                format!("IFCSHAPEREPRESENTATION(#{geom_ctx},'Body','SweptSolid',(#{solid_id}))"),
            );
            let prod_shape_id = self.id();
            self.emit_entity(
                prod_shape_id,
                format!("IFCPRODUCTDEFINITIONSHAPE($,$,(#{rep_id}))"),
            );
            let opening_axis = self.id();
            self.emit_entity(opening_axis, format!("IFCAXIS2PLACEMENT3D(#{origin},$,$)"));
            let placement_id = self.id();
            self.emit_entity(
                placement_id,
                format!("IFCLOCALPLACEMENT(#{host_placement},#{opening_axis})"),
            );
            let host_gid = el_id_to_gid.get(host_el_id).copied().unwrap_or_default();
            let opening_gid = match cutter_global_id {
                Some(cutter) if !host_gid.is_empty() => crate::revit_global_ids::hashed_global_id(
                    &format!("Opening:{number}Solid:1{host_gid}{cutter}"),
                ),
                _ => gid(&["wall-cut-opening", host_gid, tag]),
            };
            let opening_id = self.id();
            self.emit_entity(
                opening_id,
                format!(
                    "IFCOPENINGELEMENT('{opening_gid}',#{owner_hist},{},$,$,#{placement_id},#{prod_shape_id},'{}',{})",
                    quoted_or_dollar(&escape(name)),
                    escape(tag),
                    if *recess { ".RECESS." } else { ".OPENING." },
                ),
            );
            if let Some(reference) = host_references.get(reference_from) {
                for set_name in [
                    "Pset_OpeningElementCommon",
                    "Pset_EnvironmentalImpactIndicators",
                ] {
                    let set = PropertySet {
                        name: set_name.into(),
                        properties: vec![super::entities::Property {
                            name: "Reference".into(),
                            value: super::entities::PropertyValue::Identifier(reference.clone()),
                        }],
                    };
                    self.emit_property_set(
                        owner_hist,
                        opening_id,
                        &set,
                        &gid(&["property-set", opening_gid.as_str(), set_name]),
                        &gid(&["defines-by-properties", opening_gid.as_str(), set_name]),
                    );
                }
            }
            // Revit's `Depth` is the extrusion's depth, and its `Width`
            // and `Height` are the rectangle's extents along the wall and up,
            // which are the right way round on a rectangular opening (not on
            // a door's). With base quantities off the wall has none
            // and the opening none either.
            if *with_quantities && wall_widths.contains_key(host) {
                let extent = |axis: fn(&(f64, f64)) -> f64| {
                    let low = outline_feet.iter().map(axis).fold(f64::INFINITY, f64::min);
                    let high = outline_feet
                        .iter()
                        .map(axis)
                        .fold(f64::NEG_INFINITY, f64::max);
                    high - low
                };
                let length = |name: &str, feet: f64| super::entities::Property {
                    name: name.into(),
                    value: super::entities::PropertyValue::PositiveLengthFeet(feet),
                };
                let quantities = PropertySet {
                    name: "Qto_OpeningElementBaseQuantities".into(),
                    properties: vec![
                        length("Width", extent(|point| point.0)),
                        length("Height", extent(|point| point.1)),
                        length("Depth", *thickness_feet),
                    ],
                };
                self.emit_property_set(
                    owner_hist,
                    opening_id,
                    &quantities,
                    &gid(&[
                        "property-set",
                        opening_gid.as_str(),
                        "Qto_OpeningElementBaseQuantities",
                    ]),
                    &gid(&[
                        "defines-by-properties",
                        opening_gid.as_str(),
                        "Qto_OpeningElementBaseQuantities",
                    ]),
                );
            }
            void_fill_triples.push((*host, opening_id, None, opening_gid));
        }

        // IfcRelVoidsElement + IfcRelFillsElement — for each
        // (host, opening, element) triple we collected during
        // element emission, emit:
        //   - IFCRELVOIDSELEMENT(host_wall, opening)
        //   - IFCRELFILLSELEMENT(opening, door/window)
        // Together these tell IFC4 viewers "subtract this opening's
        // volume from the host wall, and fill the hole with this
        // door/window."
        //
        // The host is resolved here, after the element loop, rather
        // than inside it: `entity_index_to_el_id` is only complete
        // once every BuildingElement has been emitted, so resolving
        // inline silently dropped the chain whenever a host happened
        // to sit after its opening in `model.entities`.
        for (host_entity_idx, opening_id, el_id, opening_gid) in &void_fill_triples {
            let Some(host_el_id) = entity_index_to_el_id
                .get(*host_entity_idx)
                .and_then(|slot| *slot)
            else {
                continue;
            };
            let voids_rel = self.id();
            self.emit_entity(
                voids_rel,
                format!(
                    "IFCRELVOIDSELEMENT('{}',#{owner_hist},$,$,#{host_el_id},#{opening_id})",
                    gid(&["voids", opening_gid]),
                ),
            );
            let Some(el_id) = el_id else {
                continue;
            };
            let fills_rel = self.id();
            self.emit_entity(
                fills_rel,
                format!(
                    "IFCRELFILLSELEMENT('{}',#{owner_hist},$,$,#{opening_id},#{el_id})",
                    gid(&["fills", opening_gid]),
                ),
            );
        }

        // one IfcRelAggregates per aggregate, whole to parts. Resolved
        // after the element loop for the same reason as the void chain.
        for entity in &model.entities {
            let super::entities::IfcEntity::Aggregate { whole, parts } = entity else {
                continue;
            };
            if !is_building_element(*whole) {
                continue;
            }
            let Some(whole_el_id) = entity_index_to_el_id.get(*whole).and_then(|slot| *slot) else {
                continue;
            };
            let part_refs: Vec<String> = parts
                .iter()
                .filter(|part| aggregate_parts.contains(part))
                .filter_map(|part| entity_index_to_el_id.get(*part).and_then(|slot| *slot))
                .map(|id| format!("#{id}"))
                .collect();
            if part_refs.is_empty() {
                continue;
            }
            let rel_id = self.id();
            self.emit_entity(
                rel_id,
                format!(
                    "IFCRELAGGREGATES('{}',#{owner_hist},$,$,#{whole_el_id},({}))",
                    gid(&["aggregates", el_id_to_gid[&whole_el_id]]),
                    part_refs.join(","),
                ),
            );
        }

        // The joins of ducts and pipes. A connector is an
        // `IfcDistributionPort` named after its element and index, with
        // `SOURCEANDSINK` where its connector's domain gives it no direction
        // and `NOTDEFINED` where the direction is the connector's, which is
        // not read, nested in its element
        // by one `IfcRelNests` per element, as Revit's IFC4 export nests them
        //: IFC4 keeps `IfcRelConnectsPortToElement` for a dynamic
        // connection and restricts it to distribution elements. A join is an
        // `IfcRelConnectsPorts` between two ports; a port is shared by every
        // join it is in.
        let mut ports: HashMap<(u32, u32), usize> = HashMap::new();
        let mut nested: BTreeMap<u32, (usize, Vec<usize>)> = BTreeMap::new();
        // each connector's flow direction, `.NOTDEFINED.` where none
        // is read, and port type, `$` where none is read.
        let port_attributes: HashMap<(u32, u32), String> = model
            .entities
            .iter()
            .filter_map(|entity| match entity {
                super::entities::IfcEntity::Port {
                    id,
                    index,
                    predefined_type,
                    flow_direction,
                    ..
                } => Some((
                    (*id, *index),
                    format!(
                        ".{}.,{}",
                        flow_direction
                            .as_deref()
                            .and_then(|flow| step_enum_of(flow, &PORT_FLOW_DIRECTIONS))
                            .unwrap_or_else(|| "NOTDEFINED".into()),
                        predefined_type
                            .as_deref()
                            .and_then(|kind| step_enum_of(kind, &PORT_TYPES))
                            .map_or("$".to_string(), |kind| format!(".{kind}."))
                    ),
                )),
                _ => None,
            })
            .collect();
        let port_attribute = |id: u32, index: u32| {
            port_attributes
                .get(&(id, index))
                .map_or(".NOTDEFINED.,$", String::as_str)
                .to_string()
        };
        // Revit names a port by its flow, `InPort_` for a sink,
        // `OutPort_` for a source and `Port_` otherwise (Revit's IFC exporter
        // `ConnectorExporter`).
        let port_prefixes: HashMap<(u32, u32), &'static str> = model
            .entities
            .iter()
            .filter_map(|entity| match entity {
                super::entities::IfcEntity::Port {
                    id,
                    index,
                    flow_direction: Some(flow),
                    ..
                } => Some((
                    (*id, *index),
                    match flow.to_ascii_uppercase().as_str() {
                        "SINK" => "In",
                        "SOURCE" => "Out",
                        _ => "",
                    },
                )),
                _ => None,
            })
            .collect();
        let port_name = |id: u32, index: u32| {
            format!(
                "{}Port_{id}_{index}",
                port_prefixes.get(&(id, index)).copied().unwrap_or("")
            )
        };
        // where each connector is, by its element, when it is read.
        let mut port_points: HashMap<(u32, u32), PortPoint> = model
            .entities
            .iter()
            .filter_map(|entity| match entity {
                super::entities::IfcEntity::Port {
                    element,
                    id,
                    index,
                    location_feet: Some(point),
                    direction,
                    flow_direction,
                    ..
                } => {
                    // Revit's export points a port into its pipe where its
                    // flow is SINK, and out of it otherwise.
                    let sink = flow_direction
                        .as_deref()
                        .is_some_and(|flow| flow.eq_ignore_ascii_case("SINK"));
                    let direction = direction.map(|d| if sink { [-d[0], -d[1], -d[2]] } else { d });
                    Some(((*id, *index), (*element, *point, direction)))
                }
                _ => None,
            })
            .collect();
        // a connector joined to a placed connector of a duct or pipe is
        // at the same point, with the same axis, as Revit's export writes
        // every port joined to a segment's (RE1: 118 of 118 on Plumbing, 59
        // of 59 on Mechanical, at the point).
        for entity in &model.entities {
            let super::entities::IfcEntity::PortConnection {
                a,
                a_id,
                a_index,
                b,
                b_id,
                b_index,
            } = entity
            else {
                continue;
            };
            for (placed, element, open) in [
                ((*a_id, *a_index), *b, (*b_id, *b_index)),
                ((*b_id, *b_index), *a, (*a_id, *a_index)),
            ] {
                if port_points.contains_key(&open) {
                    continue;
                }
                if let Some(&(_, point, direction)) = port_points.get(&placed) {
                    port_points.insert(open, (element, point, direction));
                }
            }
        }
        let element_frames: HashMap<usize, (Option<[f64; 3]>, Option<f64>)> = model
            .entities
            .iter()
            .enumerate()
            .filter_map(|(index, entity)| match entity {
                super::entities::IfcEntity::BuildingElement {
                    location_feet,
                    rotation_radians,
                    ..
                } => Some((index, (*location_feet, *rotation_radians))),
                _ => None,
            })
            .collect();
        for entity in &model.entities {
            let super::entities::IfcEntity::PortConnection {
                a,
                a_id,
                a_index,
                b,
                b_id,
                b_index,
            } = entity
            else {
                continue;
            };
            let (Some(a_el), Some(b_el)) = (
                entity_index_to_el_id.get(*a).and_then(|slot| *slot),
                entity_index_to_el_id.get(*b).and_then(|slot| *slot),
            ) else {
                continue;
            };
            let ends = [(a_el, *a_id, *a_index), (b_el, *b_id, *b_index)];
            let mut port_ids = [0usize; 2];
            for (slot, &(element, id, index)) in ends.iter().enumerate() {
                if let Some(&port) = ports.get(&(id, index)) {
                    port_ids[slot] = port;
                    continue;
                }
                let placement = port_points
                    .get(&(id, index))
                    .and_then(|(owner, point, direction)| {
                        let parent = entity_index_to_placement.get(*owner).copied().flatten()?;
                        let (location, rotation) =
                            element_frames.get(owner).copied().unwrap_or((None, None));
                        Some(self.port_placement(parent, location, rotation, *point, *direction))
                    })
                    .map_or("$".to_string(), |placement| format!("#{placement}"));
                let (id_text, index_text) = (id.to_string(), index.to_string());
                let port = self.id();
                self.emit_entity(
                    port,
                    format!(
                        "IFCDISTRIBUTIONPORT('{}',#{owner_hist},'{}','Flow',$,{placement},$,{},$)",
                        gid(&["port", &id_text, &index_text]),
                        port_name(id, index),
                        port_attribute(id, index),
                    ),
                );
                nested
                    .entry(id)
                    .or_insert((element, Vec::new()))
                    .1
                    .push(port);
                ports.insert((id, index), port);
                port_ids[slot] = port;
            }
            let rel = self.id();
            self.emit_entity(
                rel,
                format!(
                    "IFCRELCONNECTSPORTS('{}',#{owner_hist},$,$,#{},#{},$)",
                    gid(&[
                        "ports",
                        &a_id.to_string(),
                        &a_index.to_string(),
                        &b_id.to_string(),
                        &b_index.to_string(),
                    ]),
                    port_ids[0],
                    port_ids[1],
                ),
            );
        }

        // A duct's or pipe's connector no join has given a port.
        for entity in &model.entities {
            let super::entities::IfcEntity::Port {
                element, id, index, ..
            } = entity
            else {
                continue;
            };
            if ports.contains_key(&(*id, *index)) {
                continue;
            }
            let Some(element_id) = entity_index_to_el_id.get(*element).and_then(|slot| *slot)
            else {
                continue;
            };
            let placement = port_points
                .get(&(*id, *index))
                .and_then(|(owner, point, direction)| {
                    let parent = entity_index_to_placement.get(*owner).copied().flatten()?;
                    let (location, rotation) =
                        element_frames.get(owner).copied().unwrap_or((None, None));
                    Some(self.port_placement(parent, location, rotation, *point, *direction))
                })
                .map_or("$".to_string(), |placement| format!("#{placement}"));
            let (id_text, index_text) = (id.to_string(), index.to_string());
            let port = self.id();
            self.emit_entity(
                port,
                format!(
                    "IFCDISTRIBUTIONPORT('{}',#{owner_hist},'{}','Flow',$,{placement},$,{},$)",
                    gid(&["port", &id_text, &index_text]),
                    port_name(*id, *index),
                    port_attribute(*id, *index),
                ),
            );
            nested
                .entry(*id)
                .or_insert((element_id, Vec::new()))
                .1
                .push(port);
            ports.insert((*id, *index), port);
        }
        for (id, (element, element_ports)) in nested {
            let refs: Vec<String> = element_ports
                .iter()
                .map(|port| format!("#{port}"))
                .collect();
            let rel = self.id();
            self.emit_entity(
                rel,
                format!(
                    "IFCRELNESTS('{}',#{owner_hist},'NestedPorts','Flow',#{element},({}))",
                    gid(&["nested_ports", &id.to_string()]),
                    refs.join(","),
                ),
            );
        }

        // Each MEP system, an `IfcSystem` grouping its members
        // and serving the building.
        for entity in &model.entities {
            let super::entities::IfcEntity::System { id, name, members } = entity else {
                continue;
            };
            let member_refs: Vec<String> = members
                .iter()
                .filter(|member| is_building_element(**member))
                .filter_map(|member| entity_index_to_el_id.get(*member).and_then(|slot| *slot))
                .map(|el| format!("#{el}"))
                .collect();
            if member_refs.is_empty() {
                continue;
            }
            let id_text = id.to_string();
            let name = name
                .as_deref()
                .map(|n| format!("'{}'", escape(n)))
                .unwrap_or_else(|| "$".into());
            let system = self.id();
            self.emit_entity(
                system,
                format!(
                    "IFCSYSTEM('{}',#{owner_hist},{name},$,$)",
                    gid(&["system", &id_text]),
                ),
            );
            let group = self.id();
            self.emit_entity(
                group,
                format!(
                    "IFCRELASSIGNSTOGROUP('{}',#{owner_hist},$,$,({}),$,#{system})",
                    gid(&["system_members", &id_text]),
                    member_refs.join(","),
                ),
            );
            let serves = self.id();
            self.emit_entity(
                serves,
                format!(
                    "IFCRELSERVICESBUILDINGS('{}',#{owner_hist},$,$,#{system},(#{building_id}))",
                    gid(&["system_building", &id_text]),
                ),
            );
        }

        // each HVAC zone an `IfcZone` of its
        // spaces.
        for entity in &model.entities {
            let super::entities::IfcEntity::Zone { id, name, members } = entity else {
                continue;
            };
            let member_refs: Vec<String> = members
                .iter()
                .filter(|member| is_building_element(**member))
                .filter_map(|member| entity_index_to_el_id.get(*member).and_then(|slot| *slot))
                .map(|el| format!("#{el}"))
                .collect();
            if member_refs.is_empty() {
                continue;
            }
            let id_text = id.to_string();
            let name = name
                .as_deref()
                .map(|n| format!("'{}'", escape(n)))
                .unwrap_or_else(|| "$".into());
            let zone = self.id();
            self.emit_entity(
                zone,
                format!(
                    "IFCZONE('{}',#{owner_hist},{name},$,'HVAC Zone',$)",
                    gid(&["zone", &id_text]),
                ),
            );
            let rel = self.id();
            self.emit_entity(
                rel,
                format!(
                    "IFCRELASSIGNSTOGROUP('{}',#{owner_hist},$,$,({}),$,#{zone})",
                    gid(&["zone_members", &id_text]),
                    member_refs.join(","),
                ),
            );
        }

        // each design option an `IfcGroup` of
        // its members, grouped into an `IfcGroup` of its set.
        let mut set_groups: BTreeMap<u32, (String, Vec<String>)> = BTreeMap::new();
        for entity in &model.entities {
            let super::entities::IfcEntity::DesignOption {
                set_id,
                set_name,
                option_id,
                option_name,
                primary,
                members,
            } = entity
            else {
                continue;
            };
            let member_refs: Vec<String> = members
                .iter()
                .filter(|member| is_building_element(**member))
                .filter_map(|member| entity_index_to_el_id.get(*member).and_then(|slot| *slot))
                .map(|el| format!("#{el}"))
                .collect();
            let id_text = option_id.to_string();
            let name = option_name
                .as_deref()
                .map(|n| format!("'{}'", escape(n)))
                .unwrap_or_else(|| "$".into());
            let group = self.id();
            self.emit_entity(
                group,
                format!(
                    "IFCGROUP('{}',#{owner_hist},{name},$,'Design Option')",
                    gid(&["design_option", &id_text]),
                ),
            );
            if !member_refs.is_empty() {
                let rel = self.id();
                self.emit_entity(
                    rel,
                    format!(
                        "IFCRELASSIGNSTOGROUP('{}',#{owner_hist},$,$,({}),$,#{group})",
                        gid(&["design_option_members", &id_text]),
                        member_refs.join(","),
                    ),
                );
            }
            let mut props = Vec::new();
            for (prop, value) in [
                (
                    "Design Option Set",
                    format!("IFCLABEL('{}')", escape(set_name)),
                ),
                ("Design Option Set Id", format!("IFCIDENTIFIER('{set_id}')")),
                ("Design Option Id", format!("IFCIDENTIFIER('{option_id}')")),
                (
                    "Is Primary",
                    format!("IFCBOOLEAN(.{}.)", if *primary { "T" } else { "F" }),
                ),
            ] {
                let p = self.id();
                self.emit_entity(p, format!("IFCPROPERTYSINGLEVALUE('{prop}',$,{value},$)"));
                props.push(format!("#{p}"));
            }
            let pset = self.id();
            self.emit_entity(
                pset,
                format!(
                    "IFCPROPERTYSET('{}',#{owner_hist},'Design Option',$,({}))",
                    gid(&["design_option_pset", &id_text]),
                    props.join(","),
                ),
            );
            let defines = self.id();
            self.emit_entity(
                defines,
                format!(
                    "IFCRELDEFINESBYPROPERTIES('{}',#{owner_hist},$,$,(#{group}),#{pset})",
                    gid(&["design_option_defines", &id_text]),
                ),
            );
            set_groups
                .entry(*set_id)
                .or_insert_with(|| (set_name.clone(), Vec::new()))
                .1
                .push(format!("#{group}"));
        }
        for (set_id, (set_name, options)) in set_groups {
            let id_text = set_id.to_string();
            let group = self.id();
            self.emit_entity(
                group,
                format!(
                    "IFCGROUP('{}',#{owner_hist},'{}',$,'Design Option Set')",
                    gid(&["design_option_set", &id_text]),
                    escape(&set_name),
                ),
            );
            let rel = self.id();
            self.emit_entity(
                rel,
                format!(
                    "IFCRELASSIGNSTOGROUP('{}',#{owner_hist},$,$,({}),$,#{group})",
                    gid(&["design_option_set_members", &id_text]),
                    options.join(","),
                ),
            );
        }

        // Further sets of an element, such as Pset_StairCommon.
        for entity in &model.entities {
            let super::entities::IfcEntity::ElementPropertySet { element, set } = entity else {
                continue;
            };
            if !is_building_element(*element) || set.properties.is_empty() {
                continue;
            }
            if let Some(el_id) = entity_index_to_el_id.get(*element).and_then(|slot| *slot) {
                element_property_sets.push((el_id, set));
            }
        }

        // IfcPropertySet emission — one set per element that ships
        // properties. Each property becomes an
        // IfcPropertySingleValue, the set wraps them, then an
        // IfcRelDefinesByProperties links the set to its element.
        let mut pset_counts: HashMap<(usize, &str), usize> = HashMap::new();
        for (el_id, pset) in &element_property_sets {
            let occurrence = pset_counts.entry((*el_id, pset.name.as_str())).or_default();
            let set_key = [
                el_id_to_gid[el_id],
                pset.name.as_str(),
                &occurrence.to_string(),
            ]
            .join("\u{1f}");
            *occurrence += 1;
            self.emit_property_set(
                owner_hist,
                *el_id,
                pset,
                &gid(&["property-set", &set_key]),
                &gid(&["defines-by-properties", &set_key]),
            );
        }

        // Bucket element_id lists by material_index so each material
        // gets one IfcRelAssociatesMaterial (rather than N, where N
        // is the number of elements using it).
        let mut by_material: Vec<Vec<usize>> = vec![Vec::new(); material_ids.len()];
        // One association per constituent set, keyed by its
        // materials' names so its GlobalId does not depend on file order.
        let mut by_constituent_set: Vec<Vec<usize>> =
            vec![Vec::new(); model.material_constituent_sets.len()];
        for (el_id, set) in &element_constituent_pairs {
            by_constituent_set[*set].push(*el_id);
        }
        for (set, elements) in by_constituent_set.iter().enumerate() {
            let Some(Some(set_id)) = constituent_set_ids.get(set) else {
                continue;
            };
            if elements.is_empty() {
                continue;
            }
            let names: Vec<&str> = model.material_constituent_sets[set]
                .material_indices
                .iter()
                .filter_map(|&index| model.materials.get(index).map(|m| m.name.as_str()))
                .collect();
            let mut key = vec!["material-constituents"];
            key.extend(names);
            let rel_id = self.id();
            self.emit_entity(
                rel_id,
                format!(
                    "IFCRELASSOCIATESMATERIAL('{}',#{owner_hist},$,$,({}),#{set_id})",
                    gid(&key),
                    elements
                        .iter()
                        .map(|id| format!("#{id}"))
                        .collect::<Vec<_>>()
                        .join(","),
                ),
            );
        }

        for (el_id, m_idx) in &element_material_pairs {
            by_material[*m_idx].push(*el_id);
        }
        let mut material_name_counts: HashMap<&str, usize> = HashMap::new();
        for (m_idx, elements) in by_material.iter().enumerate() {
            let material_name = model.materials[m_idx].name.as_str();
            let occurrence = material_name_counts.entry(material_name).or_default();
            let material_key = occurrence.to_string();
            *occurrence += 1;
            if elements.is_empty() {
                continue;
            }
            let rel_id = self.id();
            let refs_list = elements
                .iter()
                .map(|id| format!("#{id}"))
                .collect::<Vec<_>>()
                .join(",");
            self.emit_entity(
                rel_id,
                format!(
                    "IFCRELASSOCIATESMATERIAL('{}',#{owner_hist},$,$,({refs_list}),#{})",
                    gid(&["material", material_name, &material_key]),
                    material_ids[m_idx],
                ),
            );
        }

        // the elements drawing each representation map, so a
        // type holds the maps only its own occurrences draw (IFC4
        // `IfcTypeProduct.RepresentationMaps`, as Revit's export gives a
        // family type its maps).
        let mut map_users: Vec<Vec<usize>> = vec![Vec::new(); model.representation_maps.len()];
        for (i, entity) in model.entities.iter().enumerate() {
            if let super::entities::IfcEntity::BuildingElement {
                representation_map_index: Some(m),
                ..
            } = entity
            {
                if let Some(users) = map_users.get_mut(*m) {
                    users.push(i);
                }
            }
        }

        // The type objects, each with the relation typing
        // its occurrences.
        for t in type_object_plan(model, &element_gids) {
            let own: std::collections::BTreeSet<usize> = t.elements.iter().copied().collect();
            let maps: Vec<String> = map_users
                .iter()
                .enumerate()
                .filter(|(_, users)| !users.is_empty() && users.iter().all(|u| own.contains(u)))
                .flat_map(|(m, _)| {
                    representation_map_ids.get(m).into_iter().copied().chain(
                        plan_map_ids
                            .get(m)
                            .into_iter()
                            .flatten()
                            .map(|(_, _, id)| *id),
                    )
                })
                .map(|id| format!("#{id}"))
                .collect();
            let maps = if maps.is_empty() {
                "$".to_string()
            } else {
                format!("({})", maps.join(","))
            };
            let type_gid = &t.global_id;
            let type_el_id = if t.space {
                let type_el_id = self.id();
                self.emit_entity(
                    type_el_id,
                    format!(
                        "IFCSPACETYPE('{type_gid}',#{owner_hist},'{}',$,$,$,{maps},'{}',$,.NOTDEFINED.,$)",
                        escape(&t.name),
                        escape(&t.tag),
                    ),
                );
                type_el_id
            } else {
                let set_ids: Vec<String> = t
                    .sets
                    .iter()
                    .map(|set| {
                        let set_gid = gid(&["type-pset", type_gid, &set.name]);
                        format!(
                            "#{}",
                            self.emit_bare_property_set(owner_hist, set, &set_gid)
                        )
                    })
                    .collect();
                let type_el_id = self.id();
                self.emit_entity(
                    type_el_id,
                    format!(
                        "{}('{type_gid}',#{owner_hist},{},$,$,({}),{maps},'{}',{}{})",
                        t.entity,
                        quoted_or_dollar(&escape(&t.name)),
                        set_ids.join(","),
                        t.tag,
                        // IFC4 CorrectPredefinedType: a USERDEFINED type names
                        // itself in `ElementType`; Revit 27.1 writes the family
                        // name there (Nordic terrain models).
                        if t.entity == "IFCGEOGRAPHICELEMENTTYPE"
                            && t.tail.contains(".USERDEFINED.")
                        {
                            quoted_or_dollar(&escape(t.name.split(':').next().unwrap_or("")))
                        } else {
                            "$".into()
                        },
                        // `IfcDistributionElementType` declares nothing after
                        // `ElementType`.
                        if t.tail.is_empty() {
                            String::new()
                        } else {
                            format!(",{}", t.tail)
                        },
                    ),
                );
                type_el_id
            };
            // An unplaced type types nothing: IFC4's
            // `RelatedObjects` needs at least one.
            if t.elements.is_empty() {
                continue;
            }
            let rel_id = self.id();
            let elements = t
                .elements
                .iter()
                .filter_map(|i| entity_index_to_el_id[*i])
                .map(|id| format!("#{id}"))
                .collect::<Vec<_>>()
                .join(",");
            self.emit_entity(
                rel_id,
                format!(
                    "IFCRELDEFINESBYTYPE('{}',#{owner_hist},$,$,({elements}),#{type_el_id})",
                    gid(&["defines-by-type", type_gid]),
                ),
            );
        }

        for (idx, element_ids) in per_storey_elements
            .iter()
            .enumerate()
            .map(|(idx, ids)| (Some(idx), ids))
            .chain(std::iter::once((None, &unplaced_elements)))
        {
            if element_ids.is_empty() {
                continue;
            }
            let (target_storey, container_gid) = match idx {
                Some(index) => (storey_ids[index], gid(&["contains", &storey_gids[index]])),
                None => (building_id, gid(&["contains", root_key])),
            };
            let rel_id = self.id();
            let refs_list = element_ids
                .iter()
                .map(|id| format!("#{id}"))
                .collect::<Vec<_>>()
                .join(",");
            self.emit_entity(
                rel_id,
                format!(
                    "IFCRELCONTAINEDINSPATIALSTRUCTURE('{container_gid}',#{owner_hist},$,$,({refs_list}),#{target_storey})",
                ),
            );
        }
        for (idx, space_ids) in per_storey_spaces
            .iter()
            .enumerate()
            .map(|(idx, ids)| (Some(idx), ids))
            .chain(std::iter::once((None, &unplaced_spaces)))
        {
            if space_ids.is_empty() {
                continue;
            }
            let (parent, rel_gid) = match idx {
                Some(index) => (storey_ids[index], gid(&["aggregates", &storey_gids[index]])),
                None => (building_id, gid(&["aggregates", root_key, "spaces"])),
            };
            let rel_id = self.id();
            let refs_list = space_ids
                .iter()
                .map(|id| format!("#{id}"))
                .collect::<Vec<_>>()
                .join(",");
            self.emit_entity(
                rel_id,
                format!("IFCRELAGGREGATES('{rel_gid}',#{owner_hist},$,$,#{parent},({refs_list}))"),
            );
        }
        // Each space's furniture, fixtures and equipment.
        for (space, element_ids) in &per_space_elements {
            let Some(space_id) = entity_index_to_el_id.get(*space).and_then(|slot| *slot) else {
                continue;
            };
            let container_gid = gid(&["contains", el_id_to_gid[&space_id]]);
            let refs_list = element_ids
                .iter()
                .map(|id| format!("#{id}"))
                .collect::<Vec<_>>()
                .join(",");
            let rel_id = self.id();
            self.emit_entity(
                rel_id,
                format!(
                    "IFCRELCONTAINEDINSPATIALSTRUCTURE('{container_gid}',#{owner_hist},$,$,({refs_list}),#{space_id})",
                ),
            );
        }
        self.emit_line("ENDSEC;");
    }

    fn finish(self) -> String {
        let mut out = self.out;
        out.push_str("END-ISO-10303-21;\n");
        out
    }

    fn finish_to_sink(mut self) -> std::io::Result<u64> {
        self.out.push_str("END-ISO-10303-21;\n");
        self.spill();
        match self.sink {
            Some(Sink {
                error: Some(error), ..
            }) => Err(error),
            Some(Sink { written, .. }) => Ok(written),
            None => Ok(0),
        }
    }
}

/// `RefLatitude,RefLongitude,RefElevation` of `IfcSite`, as
/// IFC compound plane angles (degrees, minutes, seconds, millionths) and
/// metres; `$,$,$` without a georeference.
fn site_reference(g: Option<&crate::georeference::Georeference>) -> String {
    let Some(g) = g else {
        return "$,$,$".into();
    };
    format!(
        "{},{},{}",
        compound_angle(g.latitude),
        compound_angle(g.longitude),
        format_real(g.elevation)
    )
}

/// A STEP real with every digit of the `f64` (round-trip), `1.5E-17` style.
/// The `Axis` of a wall whose body has the plan points `points` (its own
/// frame, feet: [`extrusion_plan_points`], [`solid_plan_points`]), placed at
/// `location` and turned by `rotation` about Z: its location line `line`
/// (plan model feet, start and end) from where the body starts along it to
/// where it ends, in the element's own frame, feet, in the line's direction.
/// Revit's export writes a wall's `Axis` so: on its IFC4 Reference View
/// exports of the tiny reference models, 2018 to 2027, every wall's axis lies
/// on its location line and runs from end to end of its body along it (past
/// the line's end where the body runs through a join, short of it where it
/// stops at one). `None` without points, or where the line has no length.
fn wall_axis_local(
    line: [[f64; 2]; 2],
    location: [f64; 3],
    rotation: Option<f64>,
    points: &[(f64, f64)],
) -> Option<[[f64; 2]; 2]> {
    let [[sx, sy], [ex, ey]] = line;
    let length = (ex - sx).hypot(ey - sy);
    if points.is_empty() || !length.is_finite() || length <= 1e-9 {
        return None;
    }
    let (ux, uy) = ((ex - sx) / length, (ey - sy) / length);
    let (sin, cos) = rotation.unwrap_or(0.0).sin_cos();
    let along = points.iter().map(|&(x, y)| {
        let (wx, wy) = (
            location[0] + x * cos - y * sin,
            location[1] + x * sin + y * cos,
        );
        (wx - sx) * ux + (wy - sy) * uy
    });
    let (low, high) = along.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), t| {
        (lo.min(t), hi.max(t))
    });
    if !(low.is_finite() && high.is_finite() && high - low > 1e-9) {
        return None;
    }
    let local = |t: f64| {
        let (dx, dy) = (sx + ux * t - location[0], sy + uy * t - location[1]);
        [dx * cos + dy * sin, -dx * sin + dy * cos]
    };
    Some([local(low), local(high)])
}

/// The plan corners of an extrusion's profile in the element's frame, feet:
/// its rectangle, or its polygon. `None` for any other profile.
fn extrusion_plan_points(body: &Extrusion) -> Option<Vec<(f64, f64)>> {
    use super::entities::ProfileDef;
    let corners = |w: f64, d: f64| {
        vec![
            (-w / 2.0, -d / 2.0),
            (w / 2.0, -d / 2.0),
            (w / 2.0, d / 2.0),
            (-w / 2.0, d / 2.0),
        ]
    };
    match &body.profile_override {
        None => Some(corners(body.width_feet, body.depth_feet)),
        Some(ProfileDef::Rectangle {
            width_feet,
            depth_feet,
        }) => Some(corners(*width_feet, *depth_feet)),
        Some(
            ProfileDef::ArbitraryClosed { points } | ProfileDef::ArbitraryWithVoids { points, .. },
        ) => Some(points.clone()),
        Some(_) => None,
    }
}

/// The plan points of a solid in the element's frame, feet, for how far a
/// wall's body reaches along its location line ([`wall_axis_local`]): an
/// extrusion's profile corners (a placed one's at both ends), a mesh's
/// vertices, a compound's parts' and a boolean's first operand's. Empty for a
/// sweep or a revolution.
fn solid_plan_points(shape: &SolidShape) -> Vec<(f64, f64)> {
    match shape {
        SolidShape::ExtrudedArea(body) => extrusion_plan_points(body).unwrap_or_default(),
        SolidShape::FacetedBrep { vertices_feet, .. }
        | SolidShape::TriangulatedFaceSet { vertices_feet, .. } => {
            vertices_feet.iter().map(|v| (v[0], v[1])).collect()
        }
        SolidShape::BooleanResult { operand_a, .. } => solid_plan_points(operand_a),
        SolidShape::Compound(parts) => parts.iter().flat_map(solid_plan_points).collect(),
        SolidShape::PlacedExtrusion {
            profile,
            origin_feet: o,
            axis: z,
            ref_direction: r,
            depth_feet,
        } => {
            let body = Extrusion {
                profile_override: Some(profile.clone()),
                ..Extrusion::rectangle(0.0, 0.0, 0.0)
            };
            let norm = |v: [f64; 3]| {
                let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                (n > 1e-12).then(|| v.map(|c| c / n))
            };
            let (Some(z), Some(points)) = (norm(*z), extrusion_plan_points(&body)) else {
                return Vec::new();
            };
            let dot = r[0] * z[0] + r[1] * z[1] + r[2] * z[2];
            let Some(x) = norm([r[0] - dot * z[0], r[1] - dot * z[1], r[2] - dot * z[2]]) else {
                return Vec::new();
            };
            let y = [
                z[1] * x[2] - z[2] * x[1],
                z[2] * x[0] - z[0] * x[2],
                z[0] * x[1] - z[1] * x[0],
            ];
            points
                .iter()
                .flat_map(|&(u, v)| {
                    let p = [0, 1].map(|k| o[k] + u * x[k] + v * y[k]);
                    [
                        (p[0], p[1]),
                        (p[0] + depth_feet * z[0], p[1] + depth_feet * z[1]),
                    ]
                })
                .collect()
        }
        SolidShape::SweptPath { .. } | SolidShape::RevolvedArea { .. } => Vec::new(),
    }
}

fn format_real(x: f64) -> String {
    let t = format!("{x:?}");
    let (m, e) = t
        .split_once('e')
        .map_or((t.as_str(), None), |(m, e)| (m, Some(e)));
    let m = if m.contains('.') {
        m.to_string()
    } else {
        format!("{m}.")
    };
    match e {
        Some(e) => format!("{m}E{e}"),
        None => m,
    }
}

/// Decimal degrees as an `IfcCompoundPlaneAngleMeasure`: every part carries
/// the angle's sign.
fn compound_angle(degrees: f64) -> String {
    let sign = if degrees < 0.0 { -1 } else { 1 };
    let micro = (degrees.abs() * 3_600_000_000.0).round() as i64;
    let (d, rest) = (micro / 3_600_000_000, micro % 3_600_000_000);
    let (m, rest) = (rest / 60_000_000, rest % 60_000_000);
    let (sec, millionths) = (rest / 1_000_000, rest % 1_000_000);
    format!(
        "({},{},{},{})",
        sign * d,
        sign * m,
        sign * sec,
        sign * millionths
    )
}

/// STEP-style string escape per ISO-10303-21:
///
/// - Literal apostrophe → `''` (doubled).
/// - Literal backslash → `\\`.
/// - ASCII printable (0x20–0x7E) → pass through.
/// - ASCII control (0x00–0x1F, 0x7F) → `\X\<HH>` (2-hex-digit byte).
/// - Non-ASCII code points:
///   - BMP (≤ U+FFFF): `\X2\<HHHH>\X0\`
///   - Supplementary plane (> U+FFFF): `\X4\<HHHHHHHH>\X0\`
///
/// Previous implementation replaced non-ASCII with underscore, which
/// silently mangled accented project names, CJK text, and any Unicode
/// symbols in Revit metadata. Real RVT files routinely have
/// non-ASCII in title, path, and taxonomy strings; this escape
/// preserves them round-trip per the STEP spec.
///
/// Two consecutive non-ASCII chars produce separate `\X2\<HHHH>\X0\`
/// sequences rather than a concatenated run. The spec allows either
/// form; separate sequences keep the encoder stateless and the
/// output diff-friendly.
pub(crate) fn escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '\'' => out.push_str("''"),
            '\\' => out.push_str("\\\\"),
            c if c.is_ascii() && !c.is_control() => out.push(c),
            c if c.is_ascii() => {
                // ASCII control byte.
                out.push_str(&format!("\\X\\{:02X}", c as u32));
            }
            c if (c as u32) <= 0xFFFF => {
                // BMP non-ASCII.
                out.push_str(&format!("\\X2\\{:04X}\\X0\\", c as u32));
            }
            c => {
                // Supplementary plane (emoji, rare scripts).
                out.push_str(&format!("\\X4\\{:08X}\\X0\\", c as u32));
            }
        }
    }
    out
}

fn quoted_or_dollar(s: &str) -> String {
    if s.is_empty() {
        "$".into()
    } else {
        format!("'{s}'")
    }
}

pub(crate) fn iso_timestamp_from(secs: i64) -> String {
    // Format a Unix epoch as ISO 8601. Avoids chrono to stay
    // dep-lean. Pure function — deterministic given its input.
    let (y, m, d, hh, mm, ss) = epoch_to_ymdhms(secs);
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}")
}

/// Wall-clock Unix seconds for the header stamps. `std::time::SystemTime`
/// panics on `wasm32-unknown-unknown` ("time not implemented on this
/// platform"), which made the browser viewer's Export IFC fail on every
/// click; the wasm build reads the JavaScript host clock instead.
fn unix_seconds() -> i64 {
    #[cfg(all(target_family = "wasm", target_os = "unknown"))]
    {
        (js_sys::Date::now() / 1000.0) as i64
    }
    #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }
}

/// Gregorian breakdown without chrono. Good from 1970 through 2400.
fn epoch_to_ymdhms(secs: i64) -> (i32, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let remainder = secs.rem_euclid(86_400) as u32;
    let hh = remainder / 3600;
    let mm = (remainder % 3600) / 60;
    let ss = remainder % 60;

    // Days since 1970-01-01 → Gregorian date. Algorithm from Howard
    // Hinnant's date.h (public domain).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let y = (y + if m <= 2 { 1 } else { 0 }) as i32;
    (y, m, d, hh, mm, ss)
}

/// One IFC type object the STEP writer emits, with the
/// occurrences it types. Shared by the STEP and IFCX writers so both name a
/// type, and link it, alike.
pub(super) struct TypeObjectPlan {
    /// The occurrences' `ifc_type`, upper case.
    pub occurrence: String,
    /// The STEP entity name (`IFCWALLTYPE`).
    pub entity: String,
    pub name: String,
    /// The attributes the entity declares after `ElementType`.
    pub tail: String,
    pub global_id: String,
    /// The type's `Tag`.
    pub tag: String,
    /// The type's `PredefinedType` token.
    pub predefined: String,
    pub sets: Vec<PropertySet>,
    /// The rule that inferred the family half of `name`, if one did.
    pub name_rule: Option<String>,
    /// Indices into `IfcModel::entities`.
    pub elements: Vec<usize>,
    pub space: bool,
}

/// The occurrence entity and predefined type the type object of element
/// `index` is chosen by: its own, but a roof's slab's are its roof's, and a
/// roof written as its slabs is not typed. Revit's export types the slabs
/// with the `IfcRoofType` (all 20 tiny reference roofs).
fn typed_as<'a>(
    model: &'a IfcModel,
    roof_slabs: &HashMap<usize, usize>,
    index: usize,
) -> Option<(&'a String, &'a Option<String>)> {
    if roof_slabs.values().any(|&whole| whole == index) {
        return None;
    }
    let source = roof_slabs.get(&index).copied().unwrap_or(index);
    match model.entities.get(source)? {
        super::entities::IfcEntity::BuildingElement {
            ifc_type,
            predefined_type,
            ..
        } => Some((ifc_type, predefined_type)),
        _ => None,
    }
}

/// Every type object of the model, in the order the STEP writer emits them:
/// the `Family:Type` ones, then one per room.
pub(super) fn type_object_plan(
    model: &IfcModel,
    element_gids: &[Option<String>],
) -> Vec<TypeObjectPlan> {
    let gid = |key: &[&str]| generated_global_id(model, key);
    // Every element named `Family:Type` whose type is read is typed
    // by one IFC type object per type, as in Revit's own export: named
    // `Family:Type`, its `Tag` the type's ElementId and its GlobalId Revit's.
    struct TypeGroup {
        type_id: u32,
        entity: String,
        occurrence: String,
        name: String,
        tail: String,
        predefined: String,
        global_id: Option<String>,
        first_element: u32,
        elements: Vec<usize>,
        family: String,
        type_name: String,
        category: Option<String>,
        /// elements draw, one type object per map as Revit's export writes
        /// a type object per distinct geometry of a type.
        map: Option<usize>,
        /// the group's elements draw their own saved meshes.
        own_mesh: bool,
    }
    // the Category an element's `Other` set names, which its type
    // object's own `Other` set repeats, as Revit writes it.
    let mut categories: HashMap<usize, String> = HashMap::new();
    for entity in &model.entities {
        let super::entities::IfcEntity::ElementPropertySet { element, set } = entity else {
            continue;
        };
        if set.name != super::export_content::REVIT_OTHER_SET {
            continue;
        }
        if let Some(super::entities::PropertyValue::Label(category)) = set
            .properties
            .iter()
            .find(|p| p.name == "Category")
            .map(|p| &p.value)
        {
            categories.insert(*element, category.clone());
        }
    }
    let mut type_groups: Vec<TypeGroup> = Vec::new();
    let roof_slabs = super::roof_slab_wholes(model);
    for (entity_idx, entity) in model.entities.iter().enumerate() {
        let super::entities::IfcEntity::BuildingElement {
            type_guid,
            property_set,
            representation_map_index,
            ..
        } = entity
        else {
            continue;
        };
        let Some((ifc_type, predefined_type)) = typed_as(model, &roof_slabs, entity_idx) else {
            continue;
        };
        // planting and entourage keep one type object per
        // Revit type whatever meshes its instances draw; the type holds every
        // map its instances use.
        let map = match model.convention {
            super::convention::ExportConvention::Revit if ifc_type != "IFCGEOGRAPHICELEMENT" => {
                *representation_map_index
            }
            _ => None,
        };
        // a body drawn from the saved graphics is the instance's
        // own mesh; instances of one type whose meshes differ by placement
        // noise keep their own maps but stay one type object, as Revit's
        // export writes one per Revit type (the Honey-Locust canopies: 3).
        let own_mesh = property_set.as_ref().is_some_and(|set| {
            set.properties.iter().any(|p| {
                p.name == "BodySource"
                    && matches!(&p.value, super::entities::PropertyValue::Text(t)
                        if t == super::saved_meshes::SAVED_MESH_BODY_SOURCE
                            || t == super::symbol_meshes::SYMBOL_MESH_BODY_SOURCE)
            })
        });
        let Some(type_id) = type_guid
            .as_deref()
            .and_then(|tag| tag.parse::<u32>().ok())
            .and_then(|id| model.element_type_ids.get(&id))
        else {
            continue;
        };
        let property_text = |key: &str| {
            property_set.as_ref().and_then(|set| {
                set.properties.iter().find_map(|p| match &p.value {
                    super::entities::PropertyValue::Text(text) if p.name == key => {
                        Some(text.clone())
                    }
                    _ => None,
                })
            })
        };
        let (Some(family), Some(type_name)) = (
            property_text(super::export_content::FAMILY_NAME_PROPERTY),
            property_text(super::export_content::TYPE_NAME_PROPERTY),
        ) else {
            continue;
        };
        let Some((entity_name, tail)) =
            type_entity_for(&ifc_type.to_ascii_uppercase(), predefined_type.as_deref())
        else {
            continue;
        };
        // Revit writes a type object per original symbol, with that
        // symbol's GlobalId and the instance's symbol's Tag.
        let global_id = type_guid
            .as_deref()
            .and_then(|tag| tag.parse::<u32>().ok())
            .and_then(|id| model.global_ids.element_types.get(&id))
            .cloned();
        let element_id = type_guid
            .as_deref()
            .and_then(|tag| tag.parse::<u32>().ok())
            .unwrap_or(u32::MAX);
        // A curtain wall type is a wall type: a basic wall held in a curtain
        // grid keeps its instance's Curtain Panels, its type is Walls.
        let category = categories.get(&entity_idx).cloned().map(|c| {
            if entity_name.eq_ignore_ascii_case("IfcCurtainWallType") {
                "Walls".to_string()
            } else {
                c
            }
        });
        // Revit writes a type object per entity and predefined type.
        match type_groups.iter_mut().find(|group| {
            group.type_id == *type_id
                && group.entity == entity_name
                && group.tail == tail
                && group.global_id == global_id
                && (group.map == map || (own_mesh && group.own_mesh))
        }) {
            Some(group) => {
                group.elements.push(entity_idx);
                group.first_element = group.first_element.min(element_id);
            }
            None => type_groups.push(TypeGroup {
                type_id: *type_id,
                occurrence: ifc_type.to_ascii_uppercase(),
                // Revit names a family instance's `IfcSpaceType`
                // after its type alone.
                name: if entity_name == "IFCSPACETYPE" {
                    type_name.clone()
                } else {
                    format!("{family}:{type_name}")
                },
                entity: entity_name,
                category,
                family,
                type_name,
                tail,
                predefined: predefined_type
                    .as_deref()
                    .and_then(step_enum_token)
                    .unwrap_or_else(|| "NOTDEFINED".into()),
                global_id,
                first_element: element_id,
                elements: vec![entity_idx],
                map,
                own_mesh,
            }),
        }
    }
    // of a type's groups split by map, the one with the most
    // elements (then the lowest ElementId) is the type's own; each other
    // is a type object of an instance's own geometry, with a GlobalId of
    // its own.
    // (type, entity, attributes, GlobalId) -> (elements, lowest ElementId, group).
    type SplitKey = (u32, String, String, Option<String>);
    type SplitRank = (usize, std::cmp::Reverse<u32>, usize);
    let mut main_split: BTreeMap<SplitKey, SplitRank> = Default::default();
    for (index, group) in type_groups.iter().enumerate() {
        let key = (
            group.type_id,
            group.entity.clone(),
            group.tail.clone(),
            group.global_id.clone(),
        );
        let rank = (
            group.elements.len(),
            std::cmp::Reverse(group.first_element),
            index,
        );
        let held = main_split.entry(key).or_insert(rank);
        if (rank.0, rank.1) > (held.0, held.1) {
            *held = rank;
        }
    }
    let split_maps: BTreeMap<usize, Option<usize>> = type_groups
        .iter()
        .enumerate()
        .filter(|(index, group)| {
            main_split
                .get(&(
                    group.type_id,
                    group.entity.clone(),
                    group.tail.clone(),
                    group.global_id.clone(),
                ))
                .is_some_and(|main| main.2 != *index)
        })
        .map(|(index, group)| (index, group.map))
        .collect();
    // Of the type objects a type is written as, the one holding its
    // lowest ElementId takes the type's own GlobalId; Revit's exporter keys
    // the others by entity and predefined type.
    let mut plain_owner: BTreeMap<u32, (u32, usize)> = Default::default();
    for (index, group) in type_groups.iter().enumerate() {
        if group.global_id.is_some() || split_maps.contains_key(&index) {
            continue;
        }
        let owner = plain_owner
            .entry(group.type_id)
            .or_insert((group.first_element, index));
        if group.first_element < owner.0 {
            *owner = (group.first_element, index);
        }
    }
    // each type's text parameters (Type Mark, Description, Fire
    // Rating), which Revit shows in a type's Identity Data.
    let parameters_of: HashMap<u32, &BTreeMap<String, String>> = model
        .file_types
        .iter()
        .map(|t| (t.id, &t.parameters))
        .collect();
    let type_parameters = |id: u32| parameters_of.get(&id).into_iter().flat_map(|p| p.iter());
    let mut out = Vec::new();
    let mut used_gids: std::collections::BTreeSet<String> = Default::default();
    for (index, group) in type_groups.into_iter().enumerate() {
        let TypeGroup {
            type_id,
            entity: entity_name,
            occurrence,
            name,
            tail,
            predefined,
            global_id,
            elements,
            family,
            type_name,
            category,
            ..
        } = group;
        let tag = type_id.to_string();
        let name_rule = model.family_name_rules.get(&type_id).cloned();
        let revit = match &global_id {
            Some(global_id) => Some(global_id.clone()),
            None => model.global_ids.types.get(&type_id).and_then(|own| {
                if plain_owner.get(&type_id).map(|(_, owner)| *owner) == Some(index) {
                    return Some(own.clone());
                }
                let entity = revit_ifc_type_entity_name(&entity_name)?;
                let suffix = if model.global_ids.revit_version.unwrap_or(0) >= 2025 {
                    " InAssembly: False"
                } else {
                    ""
                };
                Some(crate::revit_global_ids::hashed_global_id(&format!(
                    "{own}Sub-element: Entity: {entity}:{predefined}{suffix}"
                )))
            }),
        };
        let type_gid = match (split_maps.get(&index), revit) {
            // A type object of one map of a type: keyed by the
            // map and, where several share it, which one this is.
            (Some(map), _) => {
                let map = map.map_or_else(|| "none".to_string(), |m| m.to_string());
                let mut n = 0usize;
                loop {
                    let candidate = gid(&["type", &entity_name, &tag, "map", &map, &n.to_string()]);
                    if used_gids.insert(candidate.clone()) {
                        break candidate;
                    }
                    n += 1;
                }
            }
            (None, Some(global_id)) if used_gids.insert(global_id.clone()) => global_id,
            // two type objects of one type and entity (a plumbing
            // fixture type whose instances split between typed groups, as
            // `IfcDistributionElementType`, which has no predefined type to tell
            // them apart) must not share the fallback GlobalId either.
            _ => {
                let fallback = gid(&["type", &entity_name, &tag]);
                if used_gids.insert(fallback.clone()) {
                    fallback
                } else {
                    let keyed = gid(&["type", &entity_name, &tag, &index.to_string()]);
                    used_gids.insert(keyed.clone());
                    keyed
                }
            }
        };
        // Revit's type parameters Category, Family Name and Type
        // Name, in the sets Revit's export gives a type object.
        let text = |name: &str, value: String| super::entities::Property {
            name: name.into(),
            value: super::entities::PropertyValue::Text(value),
        };
        let mut other = Vec::new();
        if let Some(category) = category {
            other.push(super::entities::Property {
                name: "Category".into(),
                value: super::entities::PropertyValue::Label(category),
            });
        }
        other.push(text("Family Name", family));
        let mut sets = vec![
            PropertySet {
                name: super::export_content::REVIT_OTHER_SET.into(),
                properties: other,
            },
            PropertySet {
                name: "Identity Data".into(),
                properties: std::iter::once(text("Type Name", type_name))
                    .chain(type_parameters(type_id).map(|(k, v)| text(k, v.clone())))
                    .collect(),
            },
        ];
        // Revit's own sets of the type's parameters, and the
        // standard sets of this type entity, each into the set of its name,
        // the properties it does not hold yet.
        let by_entity = model
            .type_entity_sets
            .get(&type_id)
            .and_then(|sets| sets.get(entity_name.as_str()))
            .into_iter()
            .flatten();
        for extra in model
            .type_property_sets
            .get(&type_id)
            .into_iter()
            .flatten()
            .chain(by_entity)
        {
            match sets.iter_mut().find(|set| set.name == extra.name) {
                Some(set) => {
                    for property in &extra.properties {
                        if !set.properties.iter().any(|p| p.name == property.name) {
                            set.properties.push(property.clone());
                        }
                    }
                }
                None => sets.push(extra.clone()),
            }
        }
        out.push(TypeObjectPlan {
            occurrence,
            entity: entity_name,
            name,
            tail,
            global_id: type_gid,
            tag,
            predefined,
            sets,
            name_rule,
            elements,
            space: false,
        });
    }

    // Under the `open` convention, every type the file defines that no
    // element names, so a family or type the project holds survives the
    // export (Revit's own export writes only the types its instances use:
    // `ExportConvention::writes_unused_types`).
    // A type is written as the entity this file's placed types of its
    // category are written as, else by its category's default, else as a
    // proxy type; a type of a category with neither (annotation, views)
    // and a type with no name read are left out.
    let typed: std::collections::BTreeSet<String> = out.iter().map(|t| t.tag.clone()).collect();
    let category_of: HashMap<u32, i64> = model
        .file_types
        .iter()
        .map(|t| (t.id, t.builtin_category))
        .collect();
    let mut placed_entity: BTreeMap<i64, (String, String, String)> = Default::default();
    for t in &out {
        if let Some(category) = t
            .tag
            .parse::<u32>()
            .ok()
            .and_then(|id| category_of.get(&id))
        {
            placed_entity
                .entry(*category)
                .or_insert_with(|| (t.occurrence.clone(), t.entity.clone(), t.tail.clone()));
        }
    }
    let class_of: HashMap<u32, &str> = model
        .file_types
        .iter()
        .map(|t| (t.id, t.class_name.as_str()))
        .collect();
    // (family, the rule that inferred it) of each schema class's placed types.
    type ClassFamily = Option<(String, Option<String>)>;
    let mut class_family: HashMap<String, ClassFamily> = HashMap::new();
    for t in &out {
        let (Some(class), Some(family)) = (
            t.tag.parse::<u32>().ok().and_then(|id| class_of.get(&id)),
            t.sets
                .iter()
                .flat_map(|s| &s.properties)
                .find_map(|p| match &p.value {
                    super::entities::PropertyValue::Text(f) if p.name == "Family Name" => Some(f),
                    _ => None,
                }),
        ) else {
            continue;
        };
        let seen = Some((family.clone(), t.name_rule.clone()));
        class_family
            .entry((*class).to_string())
            .and_modify(|held| {
                if *held != seen {
                    *held = None;
                }
            })
            .or_insert(seen);
    }
    let unused_types = if model.convention.writes_unused_types() {
        model.file_types.as_slice()
    } else {
        &[]
    };
    for ft in unused_types {
        let tag = ft.id.to_string();
        if typed.contains(&tag) {
            continue;
        }
        let Some(type_name) = ft.name.clone().filter(|n| !n.trim().is_empty()) else {
            continue;
        };
        let Some((occurrence, entity_name, tail)) = placed_entity
            .get(&ft.builtin_category)
            .cloned()
            .or_else(|| {
                let (occurrence, predefined) = category_occurrence(ft.builtin_category)?;
                // A class the writer does not type yet is a proxy type.
                match type_entity_for(occurrence, predefined) {
                    Some((entity, tail)) => Some((occurrence.to_string(), entity, tail)),
                    None => {
                        let (entity, tail) = type_entity_for("IFCBUILDINGELEMENTPROXY", None)?;
                        Some(("IFCBUILDINGELEMENTPROXY".to_string(), entity, tail))
                    }
                }
            })
        else {
            continue;
        };
        // A system type's family is not in its record: take the one every
        // placed type of its class in this file is written with.
        let borrowed = (!ft.family_symbol && ft.family_name.is_none())
            .then(|| class_family.get(&ft.class_name).cloned().flatten())
            .flatten();
        let name_rule = borrowed.as_ref().and_then(|(_, rule)| rule.clone());
        let family = ft
            .family_name
            .clone()
            .or_else(|| borrowed.map(|(family, _)| family));
        let name = match &family {
            Some(family) => format!("{family}:{type_name}"),
            None => type_name.clone(),
        };
        let type_gid = model
            .global_ids
            .types
            .get(&ft.id)
            .filter(|g| used_gids.insert((*g).clone()))
            .cloned()
            .unwrap_or_else(|| gid(&["type", &entity_name, &tag]));
        let text = |name: &str, value: String| super::entities::Property {
            name: name.into(),
            value: super::entities::PropertyValue::Text(value),
        };
        let mut other = Vec::new();
        if let Some(category) =
            crate::partition_element_records::builtin_category_name(ft.builtin_category)
        {
            other.push(super::entities::Property {
                name: "Category".into(),
                value: super::entities::PropertyValue::Label(category.into()),
            });
        }
        if let Some(family) = family {
            other.push(text("Family Name", family));
        }
        let mut sets = Vec::new();
        if !other.is_empty() {
            sets.push(PropertySet {
                name: super::export_content::REVIT_OTHER_SET.into(),
                properties: other,
            });
        }
        sets.push(PropertySet {
            name: "Identity Data".into(),
            properties: std::iter::once(text("Type Name", type_name))
                .chain(ft.parameters.iter().map(|(k, v)| text(k, v.clone())))
                .collect(),
        });
        out.push(TypeObjectPlan {
            occurrence,
            entity: entity_name,
            name,
            tail,
            global_id: type_gid,
            tag,
            predefined: "NOTDEFINED".into(),
            sets,
            name_rule,
            elements: Vec::new(),
            space: false,
        });
    }

    // Each room is typed by an `IfcSpaceType` of its own, as Revit's
    // export types all 116 of Core Interior's: named `<name> <number>:
    // <ElementId>`, its Tag the room's ElementId, and its GlobalId the room's
    // as a sub-element (`InstanceAsType`).
    for (entity_idx, entity) in model.entities.iter().enumerate() {
        let super::entities::IfcEntity::BuildingElement {
            ifc_type,
            type_guid: Some(tag),
            property_set,
            ..
        } = entity
        else {
            continue;
        };
        if !ifc_type.eq_ignore_ascii_case("IFCSPACE") {
            continue;
        }
        let room_text = |key: &str| {
            property_set.as_ref().and_then(|set| {
                set.properties.iter().find_map(|p| match &p.value {
                    super::entities::PropertyValue::Text(text)
                        if p.name == key && !text.trim().is_empty() =>
                    {
                        Some(text.clone())
                    }
                    _ => None,
                })
            })
        };
        let (Some(number), Some(room_name), Some(space_gid)) = (
            room_text(super::export_content::ROOM_NUMBER_PROPERTY),
            room_text(super::export_content::ROOM_NAME_PROPERTY),
            element_gids.get(entity_idx).and_then(|g| g.as_deref()),
        ) else {
            continue;
        };
        let type_gid =
            crate::revit_global_ids::sub_element_global_id(space_gid, super::INSTANCE_AS_TYPE)
                .filter(|global_id| used_gids.insert(global_id.clone()))
                .unwrap_or_else(|| gid(&["space-type", tag]));
        out.push(TypeObjectPlan {
            occurrence: "IFCSPACE".into(),
            entity: "IFCSPACETYPE".into(),
            name: format!("{room_name} {number}:{tag}"),
            tail: String::new(),
            global_id: type_gid,
            tag: tag.clone(),
            predefined: "NOTDEFINED".into(),
            sets: Vec::new(),
            name_rule: None,
            elements: vec![entity_idx],
            space: true,
        });
    }
    out
}

/// The GlobalId of an entity Revit gives none: SHA-256 over a fixed
/// namespace, the document's identity and `key` (the entity's role and the
/// identities it derives from), as an RFC 9562 version-8 UUID in IFC's
/// 22-character form. It depends on nothing about the entity's position in
/// the file, so it survives changes elsewhere in the model, and the
/// namespace keeps it apart from GlobalIds other tools derive.
/// Each building element's GlobalId, by index into `model.entities`:
/// Revit's own where the file yields it, otherwise derived from the
/// element's Tag and which of the entities sharing it this is. Shared by the
/// STEP and IFCX writers so both name an element alike.
pub(super) fn building_element_global_ids(model: &IfcModel) -> Vec<Option<String>> {
    let seed = global_id_seed(model);
    let ns = guid_namespace(model);
    let gid = |key: &[&str]| stable_guid(ns, seed, key);
    let mut element_gids: Vec<Option<String>> = vec![None; model.entities.len()];
    let mut element_counts: HashMap<(&str, &str, &str), usize> = HashMap::new();
    for (entity_idx, entity) in model.entities.iter().enumerate() {
        let super::entities::IfcEntity::BuildingElement {
            ifc_type,
            name,
            type_guid,
            ..
        } = entity
        else {
            continue;
        };
        let key = match type_guid.as_deref() {
            Some(tag) => ("element", tag, ""),
            None => ("untagged-element", ifc_type.as_str(), name.as_str()),
        };
        let occurrence = element_counts.entry(key).or_default();
        let element_gid = model
            .global_ids
            .elements
            .get(&entity_idx)
            .cloned()
            .unwrap_or_else(|| gid(&[key.0, key.1, key.2, &occurrence.to_string()]));
        *occurrence += 1;
        element_gids[entity_idx] = Some(element_gid);
    }
    element_gids
}

/// The GlobalId the STEP writer gives a generated entity: `key` under the
/// document's identity ([`global_id_seed`]).
pub(super) fn generated_global_id(model: &IfcModel, key: &[&str]) -> String {
    stable_guid(guid_namespace(model), global_id_seed(model), key)
}

/// What generated GlobalIds are derived from: the document's lasting
/// identity, which a re-save keeps, else its `Unique Document
/// GUID`, which a re-save changes.
pub(super) fn global_id_seed(model: &IfcModel) -> &str {
    let ids = &model.global_ids;
    ids.history
        .as_deref()
        .or(ids.document.as_deref())
        .unwrap_or("")
}

/// The GlobalId of `kind` (`project`, `site`, `building`): Revit's own
/// where the document's lasting identity is known, else
/// generated.
pub(super) fn project_level_global_id(model: &IfcModel, kind: &str) -> String {
    // The derivation from the document identity (checked on all
    // 27 models against Revit's export), else the GlobalId the file stores
    // for the entity, else a generated one.
    let stored = match kind {
        "project" => &model.global_ids.project,
        "site" => &model.global_ids.site,
        "building" => &model.global_ids.building,
        _ => &None,
    };
    model
        .global_ids
        .history
        .as_deref()
        .and_then(|h| crate::revit_global_ids::project_level_global_id(h, kind))
        .or_else(|| stored.clone())
        .unwrap_or_else(|| generated_global_id(model, &[kind]))
}

/// The namespace generated GlobalIds are derived under.
pub(super) fn guid_namespace(model: &IfcModel) -> &str {
    model
        .global_ids
        .guid_namespace
        .as_deref()
        .unwrap_or(super::DEFAULT_GUID_NAMESPACE)
}

fn stable_guid(namespace: &str, document: &str, key: &[&str]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(namespace.as_bytes());
    hasher.update([0]);
    hasher.update(document.as_bytes());
    for part in key {
        hasher.update([0x1f]);
        hasher.update(part.as_bytes());
    }
    let digest = hasher.finalize();
    let mut value = [0u8; 16];
    value.copy_from_slice(&digest[..16]);
    value[6] = (value[6] & 0x0f) | 0x80;
    value[8] = (value[8] & 0x3f) | 0x80;
    crate::revit_global_ids::compress_ifc_guid(value)
}

/// The elements a space contains, by their entity index: each maps to its
/// space's, and is left out of its storey's containment.
fn elements_in_spaces(model: &IfcModel) -> HashMap<usize, usize> {
    model
        .entities
        .iter()
        .filter_map(|entity| match entity {
            super::entities::IfcEntity::SpaceContainment { space, elements } => {
                Some(elements.iter().map(move |element| (*element, *space)))
            }
            _ => None,
        })
        .flatten()
        .collect()
}

/// Where [`write_step`] puts a building element in the spatial structure,
/// by [`placements`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Placement {
    /// Not written as an element: a further piece of an element of separate
    /// loops, written as an item of the body of the element that owns it
    Folded,
    /// The opening of a door or window that draws no geometry. It voids its
    /// host (an index into `IfcModel::entities`) and is contained nowhere.
    #[non_exhaustive]
    Opening {
        /// The host, an index into `IfcModel::entities`.
        host: usize,
    },
    /// Contained in a space (an index into `IfcModel::entities`).
    InSpace(usize),
    /// A part of an aggregate: it reaches the spatial structure
    /// through its whole (an index into `IfcModel::entities`).
    PartOf(usize),
    /// Contained in a storey (an index into `IfcModel::building_storeys`), or,
    /// for a space, aggregated under it.
    OnStorey(usize),
    /// Contained in the building, or, for a space, aggregated under it: no
    /// storey reached it.
    InBuilding,
}

/// Where [`write_step`] puts each entity of `model` in the spatial structure,
/// by index into `model.entities`; `None` for an entity that is not a
/// building element. The writer writes with exactly this, so a reader that
/// wants an element's storey or whether it is written at all asks here.
pub fn placements(model: &IfcModel) -> Vec<Option<Placement>> {
    let (_, folded) = fold_body_pieces(model);
    let in_spaces = elements_in_spaces(model);
    let wholes = aggregate_wholes(model);
    model
        .entities
        .iter()
        .enumerate()
        .map(|(index, entity)| {
            let super::entities::IfcEntity::BuildingElement {
                ifc_type,
                storey_index,
                host_element_index,
                ..
            } = entity
            else {
                return None;
            };
            if folded.contains(&index) {
                return Some(Placement::Folded);
            }
            // An out-of-range index is a caller bug; treat it as unbound
            // rather than silently dropping the element or pinning it to an
            // unrelated storey.
            let storey = storey_index.filter(|storey| *storey < model.building_storeys.len());
            let on_storey = || storey.map_or(Placement::InBuilding, Placement::OnStorey);
            let is_space = ifc_type.eq_ignore_ascii_case("IFCSPACE");
            // An opening standing for a door or window that draws no
            // geometry voids its host and fills nothing. Like every opening
            // it lives through that relation, not in a storey.
            if let (true, Some(host)) = (
                ifc_type.eq_ignore_ascii_case("IFCOPENINGELEMENT"),
                host_element_index,
            ) {
                Some(Placement::Opening { host: *host })
            } else if let Some(&space) = in_spaces.get(&index) {
                Some(Placement::InSpace(space))
            } else if is_space {
                Some(on_storey())
            } else if let Some(&whole) = wholes.get(&index) {
                Some(Placement::PartOf(whole))
            } else {
                Some(on_storey())
            }
        })
        .collect()
}

/// The storey of the entity at `index` as [`write_step`] puts it, an index
/// into `IfcModel::building_storeys`, by the `placements` of its model: the
/// storey it is contained in, that of the space that contains it, that of the
/// whole it is a part of, or that of the host an opening cuts. `None` where it
/// is in the building only, or is not written.
pub fn storey_of(placements: &[Option<Placement>], index: usize) -> Option<usize> {
    let mut at = index;
    // A chain of spaces, wholes and hosts is short; a cycle ends the walk.
    for _ in 0..16 {
        match (*placements.get(at)?)? {
            Placement::OnStorey(storey) => return Some(storey),
            Placement::InBuilding | Placement::Folded => return None,
            Placement::InSpace(next)
            | Placement::PartOf(next)
            | Placement::Opening { host: next } => {
                at = next;
            }
        }
    }
    None
}

/// The whole of every part of an aggregate whose whole is a building element:
/// the part reaches the spatial structure through that whole. A part
/// of several wholes has the first.
fn aggregate_wholes(model: &IfcModel) -> BTreeMap<usize, usize> {
    let is_building_element = |index: usize| {
        matches!(
            model.entities.get(index),
            Some(super::entities::IfcEntity::BuildingElement { .. })
        )
    };
    let mut wholes = BTreeMap::new();
    for entity in &model.entities {
        let super::entities::IfcEntity::Aggregate { whole, parts } = entity else {
            continue;
        };
        if !is_building_element(*whole) {
            continue;
        }
        for part in parts {
            if *part != *whole && is_building_element(*part) {
                wholes.entry(*part).or_insert(*whole);
            }
        }
    }
    wholes
}

/// The parts of every aggregate whose whole is a building element:
/// they reach the spatial structure through that whole.
fn aggregate_parts_of_elements(model: &IfcModel) -> std::collections::BTreeSet<usize> {
    aggregate_wholes(model).into_keys().collect()
}

/// the further pieces of an element written as items of its body,
/// by their owner, and every piece folded so: where the element and the
/// piece are both plain extrusions, or neither has geometry.
type PieceBodies<'a> = BTreeMap<usize, Vec<&'a Extrusion>>;

fn fold_body_pieces(model: &IfcModel) -> (PieceBodies<'_>, std::collections::BTreeSet<usize>) {
    let plain_body = |index: usize| match model.entities.get(index) {
        Some(super::entities::IfcEntity::BuildingElement {
            extrusion,
            solid_shape: None,
            representation_map_index: None,
            ..
        }) => Some(extrusion.as_ref()),
        _ => None,
    };
    let mut piece_bodies: BTreeMap<usize, Vec<&Extrusion>> = BTreeMap::new();
    let mut folded = std::collections::BTreeSet::new();
    for (owner, pieces) in &model.body_pieces {
        let Some(owner_body) = plain_body(*owner) else {
            continue;
        };
        for piece in pieces {
            match (owner_body, plain_body(*piece)) {
                (Some(_), Some(Some(body))) => {
                    piece_bodies.entry(*owner).or_default().push(body);
                    folded.insert(*piece);
                }
                (None, Some(None)) => {
                    folded.insert(*piece);
                }
                _ => {}
            }
        }
    }
    (piece_bodies, folded)
}

/// Whether every storey of `model` contains an element: an element written
/// in its own `IfcRelContainedInSpatialStructure` with that storey, as the
/// writer places it (`placements`: not a space, a hosted opening, a body
/// piece written as an item of its owner, a part of an aggregate or an
/// element a space contains). Revit's exporter up to IFC 26.0 counts the
/// storeys that contain an element as `NumberOfStoreys`, and from 26.1 every
/// storey, so the two agree only here.
pub(crate) fn every_storey_holds_an_element(model: &IfcModel) -> bool {
    let mut holding = vec![false; model.building_storeys.len()];
    for (entity, placement) in model.entities.iter().zip(placements(model)) {
        let super::entities::IfcEntity::BuildingElement { ifc_type, .. } = entity else {
            continue;
        };
        match placement {
            Some(Placement::OnStorey(storey)) if !ifc_type.eq_ignore_ascii_case("IFCSPACE") => {
                holding[storey] = true;
            }
            _ => {}
        }
    }
    holding.iter().all(|held| *held)
}

#[cfg(test)]
mod georeference_step_tests {
    use super::{compound_angle, format_real};

    #[test]
    fn reals_and_compound_angles_are_step_text() {
        assert_eq!(format_real(1370149.5631030737), "1370149.5631030737");
        assert_eq!(format_real(0.0), "0.0");
        assert_eq!(format_real(6.123e-17), "6.123E-17");
        assert_eq!(compound_angle(42.387), "(42,23,13,200000)");
        assert_eq!(compound_angle(-71.242), "(-71,-14,-31,-200000)");
    }
}

#[cfg(test)]
mod wall_axis_tests {
    use super::super::entities::{BrepTriangle, Extrusion, ProfileDef, SolidShape};
    use super::{extrusion_plan_points, solid_plan_points, wall_axis_local};

    fn axis_of(
        line: [[f64; 2]; 2],
        location: [f64; 3],
        rotation: Option<f64>,
        body: &Extrusion,
    ) -> Option<[[f64; 2]; 2]> {
        wall_axis_local(
            line,
            location,
            rotation,
            &extrusion_plan_points(body).unwrap_or_default(),
        )
    }

    fn close(a: [[f64; 2]; 2], b: [[f64; 2]; 2]) -> bool {
        a.iter()
            .flatten()
            .zip(b.iter().flatten())
            .all(|(x, y)| (x - y).abs() < 1e-9)
    }

    #[test]
    fn axis_runs_from_end_to_end_of_the_body_along_the_line() {
        // A body 6.1 long centred at (3.05, 0) on the line (0,0)-(6,0): it
        // runs through the join at the line's end.
        let body = Extrusion::rectangle(6.1, 0.2, 3.0);
        let axis = axis_of([[0.0, 0.0], [6.0, 0.0]], [3.05, 0.0, 0.0], None, &body);
        assert!(close(axis.unwrap(), [[-3.05, 0.0], [3.05, 0.0]]));
        // The same wall drawn the other way: the axis follows the line.
        let axis = axis_of([[6.0, 0.0], [0.0, 0.0]], [3.05, 0.0, 0.0], None, &body);
        assert!(close(axis.unwrap(), [[3.05, 0.0], [-3.05, 0.0]]));
        // A mitred end: the outer corner sets how far the axis runs.
        let mitred = Extrusion {
            profile_override: Some(ProfileDef::ArbitraryClosed {
                points: vec![(0.1, -2.0), (0.1, 2.0), (-0.1, 1.8), (-0.1, -2.0)],
            }),
            ..Extrusion::rectangle(0.0, 0.0, 3.0)
        };
        let axis = axis_of([[6.0, 0.0], [6.0, 4.0]], [6.0, 2.1, 0.0], None, &mitred);
        assert!(close(axis.unwrap(), [[0.0, -2.0], [0.0, 2.0]]));
        // A turned placement: local X along the line.
        let turned = axis_of(
            [[0.0, 0.0], [0.0, 4.0]],
            [0.0, 2.0, 0.0],
            Some(std::f64::consts::FRAC_PI_2),
            &Extrusion::rectangle(3.8, 0.2, 3.0),
        );
        assert!(close(turned.unwrap(), [[-1.9, 0.0], [1.9, 0.0]]));
        // No polygon, no axis.
        let round = Extrusion {
            profile_override: Some(ProfileDef::Circle { radius_feet: 1.0 }),
            ..Extrusion::rectangle(0.0, 0.0, 3.0)
        };
        assert!(axis_of([[0.0, 0.0], [1.0, 0.0]], [0.0; 3], None, &round).is_none());
    }

    #[test]
    fn a_wall_drawn_from_a_mesh_or_a_compound_solid_gets_its_axis() {
        // A mesh body from x = -2 to 2.1 in its own frame, placed at (2, 0)
        // on the line (0,0)-(4,0): the axis runs end to end of the mesh.
        let mesh = SolidShape::TriangulatedFaceSet {
            vertices_feet: vec![
                [-2.0, -0.1, 0.0],
                [2.1, -0.1, 0.0],
                [2.1, 0.1, 3.0],
                [-2.0, 0.1, 3.0],
            ],
            triangles: vec![BrepTriangle(0, 1, 2), BrepTriangle(0, 2, 3)],
            face_materials: Vec::new(),
        };
        let line = [[0.0, 0.0], [4.0, 0.0]];
        let axis = wall_axis_local(line, [2.0, 0.0, 0.0], None, &solid_plan_points(&mesh));
        assert!(close(axis.unwrap(), [[-2.0, 0.0], [2.1, 0.0]]));
        // A stepped body of two extrusions: both count.
        let steps = SolidShape::Compound(vec![
            SolidShape::ExtrudedArea(Extrusion::rectangle(2.0, 0.2, 3.0)),
            SolidShape::ExtrudedArea(Extrusion {
                profile_override: Some(ProfileDef::ArbitraryClosed {
                    points: vec![(1.0, -0.1), (2.5, -0.1), (2.5, 0.1), (1.0, 0.1)],
                }),
                ..Extrusion::rectangle(0.0, 0.0, 1.0)
            }),
        ]);
        let axis = wall_axis_local(line, [2.0, 0.0, 0.0], None, &solid_plan_points(&steps));
        assert!(close(axis.unwrap(), [[-1.0, 0.0], [2.5, 0.0]]));
    }
}
