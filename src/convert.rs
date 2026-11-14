//! The conversion pipeline behind `rvt-ifc`, `rvt-elements` and `rvt-info
//! --json`, as library calls.
//!
//! The CLIs and the WebAssembly package (`rvt-wasm`, the browser converter
//! page) call the same functions in the same order, so the browser's output
//! is byte-identical to the command line's for the same input and options.
//! Nothing here writes or modifies a Revit file: it reads one and returns
//! IFC4, IFCX, Fragments, USD, glTF, CSV and JSON as bytes or text.

use crate::RevitFile;
use crate::control::WalkerControl;
use crate::ifc::{
    DiagnosticRvtDocExporter, ExportConvention, ExportDiagnostics, ExportQualityMode, GraphOptions,
    IfcModel, IfcxGraph, PlaceholderExporter, RvtDocExporter,
};
use crate::walker::{
    DecodedElement, InstanceField, PRODUCTION_ELEMENT_MIN_SCORE, WalkerLimits,
    iter_elements_with_control,
};
use serde::Serialize;
use std::collections::BTreeMap;

/// What `rvt-ifc` takes on its command line, minus the output paths.
#[derive(Debug, Clone)]
pub struct ConvertOptions {
    /// `--placeholder`: empty project body.
    pub placeholder: bool,
    /// `--mode`.
    pub quality: ExportQualityMode,
    /// `--convention`.
    pub convention: ExportConvention,
    /// `--diagnostic-proxies`.
    pub diagnostic_proxies: bool,
    /// `--family-geometry`: draw box bodies the saved graphics leave from
    /// the family's embedded document.
    pub family_geometry: bool,
    /// `--family-first`: try the family document before the saved graphics.
    /// Off: saved graphics first, the family document for what they leave
    /// as boxes (a document draws the family's saved type, so
    /// its sizes are right for about 5% of instances).
    pub family_first: bool,
    /// `--saved-meshes`: draw every box body from Revit's saved graphics.
    pub saved_meshes: bool,
    /// `--no-saved-meshes`: keep doors and windows as their record's box.
    pub no_saved_meshes: bool,
    /// Trees, people and cars still drawn as their record's box get a shaped
    /// stand-in (`--no-proxy-shapes` turns it off).
    pub proxy_shapes: bool,
    /// `--saved-materials`: give elements with no material the one of their
    /// saved graphics.
    pub saved_materials: bool,
    /// `--per-element-meshes`.
    pub per_element_meshes: bool,
    /// Revit's own property sets (one per parameter group, built-in
    /// parameters named) and each element's `Revit` set (ElementId,
    /// category); on by default, `--no-revit-property-sets` turns it off.
    pub revit_property_sets: bool,
    /// `--epsg`.
    pub epsg: Option<u32>,
    /// `--guid-namespace` / `RVT_OPEN_GUID_NAMESPACE`: what generated
    /// GlobalIds are derived under; `None` is
    /// [`crate::ifc::DEFAULT_GUID_NAMESPACE`].
    pub guid_namespace: Option<String>,
    /// `--max-walker-*`.
    pub walker_limits: WalkerLimits,
    /// `--element-provenance`: list, in the diagnostics, where each decoded
    /// element was read from.
    pub element_provenance: bool,
    /// `--export-decisions`: list, in the diagnostics, each fail-closed
    /// decision per element.
    pub export_decisions: bool,
    /// `--family-extrusions`: draw a family instance from its family's own
    /// extrusions where they reproduce its saved mesh.
    pub family_extrusions: bool,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        Self {
            placeholder: false,
            quality: ExportQualityMode::Scaffold,
            convention: ExportConvention::EXPORT_DEFAULT,
            diagnostic_proxies: false,
            family_geometry: false,
            family_first: false,
            saved_meshes: false,
            no_saved_meshes: false,
            saved_materials: false,
            proxy_shapes: false,
            per_element_meshes: false,
            revit_property_sets: true,
            epsg: None,
            guid_namespace: None,
            walker_limits: WalkerLimits::default(),
            element_provenance: false,
            export_decisions: false,
            family_extrusions: false,
        }
    }
}

impl ConvertOptions {
    /// What `rvt-ifc` does with no flags: every body pass (saved graphics,
    /// family documents, saved materials, proxy shapes) and Revit's property
    /// sets on.
    pub fn rvt_ifc_defaults() -> Self {
        Self {
            family_geometry: true,
            saved_meshes: true,
            saved_materials: true,
            proxy_shapes: true,
            revit_property_sets: true,
            ..Self::default()
        }
    }
}

/// The exported model before the quality check, with the messages
/// `rvt-ifc` prints on stderr (without its `rvt-ifc: ` prefix).
#[derive(Debug)]
pub struct Exported {
    pub model: IfcModel,
    pub diagnostics: ExportDiagnostics,
    pub notes: Vec<String>,
    /// each element's measured quantities ([`refine`] fills it).
    pub quantities: Vec<crate::ifc::quantities::ElementQuantities>,
    /// The decisions the exporter's `skipped` counts were made from, for the
    /// log of decisions ([`ConvertOptions::export_decisions`]).
    pub decisions: Vec<crate::ifc::export_decisions::ExportDecision>,
    /// The provenance table the log of decisions reads where the diagnostics
    /// do not carry one.
    pub log_provenance: Option<crate::ifc::element_provenance::ElementProvenanceTable>,
}

/// Step 1 of `rvt-ifc`: [`export_base`], then [`refine`].
pub fn export(rf: &mut RevitFile, opts: &ConvertOptions) -> anyhow::Result<Exported> {
    export_with_control(rf, opts, &WalkerControl::default())
}

/// [`export`], reporting the walk's and the saved graphics' progress to
/// `control`; a cancelled token stops it with [`crate::Error::Cancelled`].
pub fn export_with_control(
    rf: &mut RevitFile,
    opts: &ConvertOptions,
    control: &WalkerControl,
) -> anyhow::Result<Exported> {
    let mut exported = export_base_with_control(rf, opts, control)?;
    control.check()?;
    refine_with_control(rf, &mut exported, opts, control);
    control.check()?;
    Ok(exported)
}

/// Step 1a: the exporter alone. With default options its model is the one
/// `rvt-schedule` and `rvt-gltf` export.
pub fn export_base(rf: &mut RevitFile, opts: &ConvertOptions) -> anyhow::Result<Exported> {
    export_base_with_control(rf, opts, &WalkerControl::default())
}

/// [`export_base`] with the walk's progress reported to `control`.
pub fn export_base_with_control(
    rf: &mut RevitFile,
    opts: &ConvertOptions,
    control: &WalkerControl,
) -> anyhow::Result<Exported> {
    // The readers note the copy they take each value from only when the
    // table that lists them is asked for.
    rf.record_value_sources(opts.element_provenance && !opts.placeholder);
    // The frames of a Revit 2008 to 2023 file that stop are listed only
    // for the log of decisions; their counts are kept either way.
    rf.record_frame_decisions(opts.export_decisions);
    let result = if opts.placeholder {
        PlaceholderExporter.export_with_diagnostics(rf)?
    } else if opts.diagnostic_proxies {
        DiagnosticRvtDocExporter.export_with_diagnostics_and_convention(
            rf,
            opts.walker_limits,
            opts.convention,
        )?
    } else {
        RvtDocExporter.export_with_diagnostics_convention_and_control(
            rf,
            opts.quality,
            opts.walker_limits,
            opts.convention,
            control,
        )?
    };
    let mut diagnostics = result.diagnostics;
    let mut notes = Vec::new();
    // The rooms of an earlier phase, written because the last phase
    // holds none.
    if let Some(fallback) = &diagnostics.room_phase_fallback {
        notes.push(format!(
            "the project's last phase holds no room, so the {} room(s) of {:?}, the latest phase that holds any, are written; every other element is of the last phase. Naming the export phase (rvt-ifc --phase) is exact: naming the last writes no room",
            fallback.rooms, fallback.phase
        ));
    }
    // Built from the walk the exporter just made, which the handle
    // memoised, before the later passes touch the handle again.
    let mut log_provenance = None;
    if !opts.placeholder {
        if opts.element_provenance {
            control.check()?;
            diagnostics.decoded.element_provenance = Some(crate::ifc::element_provenance::build(
                rf,
                opts.walker_limits,
            ));
        } else if opts.export_decisions {
            control.check()?;
            log_provenance = Some(crate::ifc::element_provenance::build(
                rf,
                opts.walker_limits,
            ));
        }
    }
    Ok(Exported {
        model: result.model,
        diagnostics,
        notes,
        quantities: Vec::new(),
        decisions: result.decisions,
        log_provenance,
    })
}

/// Building elements with no material of any kind.
pub fn count_without_material(model: &IfcModel) -> usize {
    model
        .entities
        .iter()
        .filter(|e| {
            matches!(
                e,
                crate::ifc::entities::IfcEntity::BuildingElement {
                    material_index: None,
                    material_layer_set_index: None,
                    material_profile_set_index: None,
                    ..
                }
            )
        })
        .count()
}

/// How many building elements (and how many walls, slabs, roofs and
/// coverings among them) draw in a material that has a colour: the element's
/// own, else one of its layers' or profiles'.
fn coloured_note(model: &IfcModel) -> String {
    let has_colour = |m: Option<usize>| {
        m.and_then(|m| model.materials.get(m))
            .is_some_and(|m| m.color_packed.is_some())
    };
    let (mut all, mut all_coloured, mut system, mut system_coloured) = (0, 0, 0, 0);
    for entity in &model.entities {
        let crate::ifc::entities::IfcEntity::BuildingElement {
            ifc_type,
            material_index,
            material_layer_set_index,
            material_profile_set_index,
            ..
        } = entity
        else {
            continue;
        };
        let coloured = has_colour(*material_index)
            || material_layer_set_index
                .and_then(|i| model.material_layer_sets.get(i))
                .is_some_and(|set| set.layers.iter().any(|l| has_colour(l.material_index)))
            || material_profile_set_index
                .and_then(|i| model.material_profile_sets.get(i))
                .is_some_and(|set| {
                    set.profiles
                        .iter()
                        .any(|p| has_colour(Some(p.material_index)))
                });
        all += 1;
        all_coloured += usize::from(coloured);
        if matches!(
            ifc_type.as_str(),
            "IFCWALL" | "IFCWALLSTANDARDCASE" | "IFCSLAB" | "IFCROOF" | "IFCCOVERING"
        ) {
            system += 1;
            system_coloured += usize::from(coloured);
        }
    }
    format!(
        "{all_coloured} of {all} building elements draw in a coloured material ({system_coloured} of {system} walls, slabs, roofs and coverings)"
    )
}

/// The family-document pass of [`refine`], with its stderr note.
fn family_pass(
    rf: &mut RevitFile,
    model: &mut IfcModel,
    diagnostics: &mut ExportDiagnostics,
    notes: &mut Vec<String>,
) {
    crate::control::stage("family documents");
    match crate::ifc::family_bodies::attach(rf, model) {
        Ok(report) => {
            crate::ifc::recount_exported(diagnostics, model);
            notes.push(format!(
                "{} of {} bounding-box bodies drawn from their family documents ({} triangles; {} not instances of a loaded family; {} wrong size for the instance; {} stretched along their one ruled axis; {} mirrored; incomplete documents {})",
                report.replaced,
                report.candidates,
                report.triangles,
                report.no_family_document,
                report.size_rejected,
                report.scaled_ruled_along_axis,
                report.flipped_skipped,
                serde_json::to_string(&report.incomplete).unwrap_or_default()
            ));
        }
        Err(e) => notes.push(format!("family documents not read, bodies unchanged: {e}")),
    }
}

/// Step 1b: saved graphics (`--saved-meshes`) and the CRS override
/// (`--epsg`), with their stderr notes.
pub fn refine(rf: &mut RevitFile, exported: &mut Exported, opts: &ConvertOptions) {
    refine_with_control(rf, exported, opts, &WalkerControl::default());
}

/// [`refine`] with the saved graphics' progress reported to `control`.
pub fn refine_with_control(
    rf: &mut RevitFile,
    exported: &mut Exported,
    opts: &ConvertOptions,
    control: &WalkerControl,
) {
    let Exported {
        model,
        diagnostics,
        notes,
        quantities,
        decisions,
        log_provenance,
    } = exported;
    if let Some(ns) = &opts.guid_namespace {
        model.global_ids.guid_namespace = Some(ns.clone());
        diagnostics.guid_namespace = ns.clone();
    }
    if opts.revit_property_sets && !opts.placeholder {
        revit_properties(rf, model, notes);
    }
    let without_before = count_without_material(model);
    // a family file's own forms become its elements, in one
    // storey; the passes below then treat them like any other.
    let family = if !opts.placeholder && rf.is_family_document() {
        crate::control::stage("family file");
        match crate::ifc::family_file::attach(rf, model, !opts.no_saved_meshes) {
            Ok(summary) => {
                notes.push(format!(
                    "{}: {} solid forms {} drawn {} missing {} hidden {} voids {}; nested instances {} ({} drawn here)",
                    summary.summary,
                    summary.forms_found.values().sum::<usize>(),
                    serde_json::to_string(&summary.forms_found).unwrap_or_default(),
                    serde_json::to_string(&summary.forms_drawn).unwrap_or_default(),
                    serde_json::to_string(&summary.forms_missing).unwrap_or_default(),
                    summary.forms_hidden,
                    summary.voids,
                    summary.nested_instances,
                    summary.nested_drawn
                ));
                Some(summary)
            }
            Err(e) => {
                notes.push(format!("family file forms not read: {e}"));
                None
            }
        }
    } else {
        None
    };
    // a family file whose forms drew no body: a standalone revolution
    // rebuilt from its saved records as an analytic solid, in the
    // scaffold mode alone (it has no measured IFC product category, so the
    // modes that need mapped types gain no proxy here).
    if !opts.placeholder
        && rf.is_family_document()
        && opts.quality == ExportQualityMode::Scaffold
        && !model.entities.iter().any(|entity| {
            matches!(
                entity,
                crate::ifc::entities::IfcEntity::BuildingElement {
                    solid_shape: Some(_),
                    ..
                }
            )
        })
    {
        match crate::ifc::family_revolutions::attach(rf, model, control) {
            Ok(0) => {}
            Ok(added) => {
                crate::ifc::recount_exported(diagnostics, model);
                notes.push(format!(
                    "{added} standalone family revolution bodies decoded from native records"
                ));
            }
            Err(e) => notes.push(format!("family revolution bodies unchanged: {e:#}")),
        }
    }
    // Bodies for an instance that has only its record box: the saved
    // graphics and the family document, in the order asked, then the box.
    if opts.family_geometry && opts.family_first {
        family_pass(rf, model, diagnostics, notes);
    }
    // toposurfaces and Site-category instances the walk does not
    // yield, before the body passes draw them.
    if opts.family_geometry {
        match crate::ifc::symbol_meshes::add_site_content(rf, model) {
            Ok(report) => {
                crate::ifc::recount_exported(diagnostics, model);
                if report.terrains + report.site_instances > 0 {
                    notes.push(format!(
                        "site content added: {} toposurfaces ({} triangles), {} site-category instances",
                        report.terrains, report.terrain_triangles, report.site_instances
                    ));
                }
            }
            Err(e) => notes.push(format!("site content not read: {e}")),
        }
    }
    if let Some(scope) = saved_mesh_scope(rf, opts) {
        // The saved graphics only improve bodies: when they cannot be read,
        // the export goes ahead with the bodies it has, and says so.
        crate::control::stage("saved graphics");
        match crate::ifc::saved_meshes::attach_scoped_with_control(rf, model, scope, control) {
            Ok(report) => {
                crate::ifc::recount_exported(diagnostics, model);
                if report.candidates > 0 {
                    let what = if opts.saved_meshes {
                        "bounding-box bodies"
                    } else {
                        "door and window bounding-box bodies"
                    };
                    notes.push(format!(
                        "{} of {} {what} drawn from their saved graphics ({} triangles); {} kept their box, their saved graphics not decoding whole; {} back-face triangles dropped, {} instances took an earlier instance's vertices ({} same-triangle instances did not, closest {} mm apart)",
                        report.replaced,
                        report.candidates,
                        report.triangles,
                        report.partial,
                        report.back_faces_dropped,
                        report.unified_instances,
                        report.unify_misses,
                        report.unify_closest_miss_mm
                    ));
                }
            }
            Err(e) => {
                notes.push(format!("saved graphics not read, bodies unchanged: {e}"));
                // the boxes the pass was to look at keep their body, and say why.
                if crate::ifc::saved_meshes::note_not_offered(model, "pass_failed") > 0 {
                    crate::ifc::recount_exported(diagnostics, model);
                }
            }
        }
    } else if !opts.placeholder {
        // no pass was offered, so each box says why it is one: the caller
        // turned the pass off, or the release's saved graphics are not read.
        let why = if opts.no_saved_meshes {
            "saved_meshes_off"
        } else if rf.basic_file_info().is_ok() {
            "release_outside_2014_to_2027"
        } else {
            "release_not_read"
        };
        if crate::ifc::saved_meshes::note_not_offered(model, why) > 0 {
            crate::ifc::recount_exported(diagnostics, model);
        }
    }
    // walls, floors and roofs a door, window or opening cuts are
    // drawn in the mesh formats with their openings cut, as Revit saved them.
    if saved_mesh_scope(rf, opts).is_some() {
        crate::control::stage("saved graphics: openings");
        match crate::ifc::saved_meshes::attach_display_bodies(rf, model, control) {
            Ok(r) if r.hosts > 0 => notes.push(format!(
                "{} of {} walls, floors, roofs and other hosts of openings drawn with their openings cut from their saved graphics ({} triangles); {} not decoding whole, {} without saved graphics",
                r.drawn, r.hosts, r.triangles, r.partial, r.missing
            )),
            Ok(_) => {}
            Err(e) => notes.push(format!("openings not cut, saved graphics not read: {e}")),
        }
    }
    if opts.family_geometry && !opts.family_first {
        family_pass(rf, model, diagnostics, notes);
    }
    // RPC content (trees, people, cars) from its symbol's own
    // saved mesh, on every release, before any stand-in shape.
    if opts.family_geometry {
        match crate::ifc::symbol_meshes::attach(rf, model) {
            Ok(report) => {
                crate::ifc::recount_exported(diagnostics, model);
                if report.replaced > 0 {
                    notes.push(format!(
                        "{} of {} bounding-box instance bodies drawn from their family symbol's saved mesh ({} triangles, {} mesh symbols, {} mirrored skipped)",
                        report.replaced, report.candidates, report.triangles, report.mesh_symbols, report.flipped_skipped
                    ));
                }
            }
            Err(e) => notes.push(format!(
                "family symbol meshes not read, bodies unchanged: {e}"
            )),
        }
    }
    if opts.proxy_shapes {
        crate::control::stage("proxy shapes");
        let report = crate::ifc::proxy_shapes::attach(model);
        let total: usize = report.replaced.values().sum();
        crate::ifc::recount_exported(diagnostics, model);
        notes.push(format!(
            "{total} elements drawn as approximate proxy shapes {} ({} elements remain bounding boxes)",
            serde_json::to_string(&report.replaced).unwrap_or_default(),
            report.still_boxes
        ));
    }
    if opts.saved_materials {
        crate::control::stage("saved materials");
        match crate::ifc::saved_meshes::attach_materials(rf, model) {
            Ok(set) => notes.push(format!(
                "{set} elements took the material of their saved graphics"
            )),
            Err(e) => notes.push(format!("saved materials not read: {e}")),
        }
    }
    if opts.saved_materials || opts.family_geometry {
        notes.push(format!(
            "elements without a material {} -> {}",
            without_before,
            count_without_material(model)
        ));
        notes.push(coloured_note(model));
    }
    if let Some(summary) = family {
        crate::ifc::refresh_family_diagnostics(diagnostics, model, summary);
    }
    if opts.family_extrusions && !opts.placeholder {
        match crate::ifc::family_extrusions::attach_with_control(rf, model, control) {
            Ok(report) => {
                crate::ifc::recount_exported(diagnostics, model);
                notes.push(format!(
                    "family extrusions: {} of {} instances drawn from {} of {} families' extrusions",
                    report.replaced, report.candidates, report.families_rebuilt, report.families
                ));
            }
            Err(e) => notes.push(format!("family extrusions not read: {e:#}")),
        }
    }
    // The model is final, so the bodies the later passes replaced name
    // their own sources.
    if let Some(table) = diagnostics.decoded.element_provenance.as_mut() {
        crate::ifc::element_provenance::attach_value_sources(table, rf, model);
    }
    // The decisions the `skipped` counts were made from, and those behind
    // the final model's box bodies and outlines.
    if opts.export_decisions && !opts.placeholder {
        let mut all = std::mem::take(decisions);
        all.extend(crate::ifc::model_decisions(model));
        let table = diagnostics
            .decoded
            .element_provenance
            .as_ref()
            .or(log_provenance.as_ref());
        diagnostics.export_decisions = Some(
            crate::ifc::export_decisions::ExportDecisionLog::build(all, table),
        );
    }
    *log_provenance = None;
    // A footprint roof with no slope-defining edge is its one slab.
    crate::ifc::split_single_slab_roofs(model);
    // volumes, areas and material layers into the base
    // quantities, on the bodies as finally drawn.
    if !opts.placeholder {
        crate::control::stage("quantities");
        let (measured, report) = crate::ifc::quantities::attach(model);
        *quantities = measured;
        crate::ifc::recount_exported(diagnostics, model);
        notes.push(format!(
            "quantities {}",
            serde_json::to_string(&report).unwrap_or_default()
        ));
    }
    // the CRS the user names wins; say when it overrides the
    // file's, or when the file leaves the choice open.
    if let Some(g) = model.georeference.as_mut() {
        let warning = match opts.epsg {
            Some(code) => g.override_epsg(code),
            None => g.unresolved_crs_warning(),
        };
        if let Some(w) = warning {
            notes.push(format!("warning: {w}"));
        }
    } else if opts.epsg.is_some() {
        notes.push("warning: --epsg ignored: the file's georeference was not decoded".into());
    }
}

/// Each element's `Revit` set and Revit's own property sets.
/// Like the saved graphics, the parameters only add to the export: when
/// they cannot be read, it goes ahead without them, and says so.
fn revit_properties(rf: &mut RevitFile, model: &mut IfcModel, notes: &mut Vec<String>) {
    crate::control::stage("Revit properties");
    let identity = crate::ifc::revit_identity::attach(rf, model);
    notes.push(format!(
        "Revit ElementId on {} elements ({} with their category) and {} types",
        identity.elements, identity.with_category, identity.types
    ));
    match crate::ifc::revit_property_sets::attach(rf, model) {
        Ok(report) => notes.push(format!(
            "Revit property sets on {} elements and {} types, and type sets on {} elements of types with no type object ({} properties, {} named parameters left out)",
            report.elements,
            report.types,
            report.untyped_elements,
            report.properties,
            report.left_out
        )),
        Err(e) => notes.push(format!("Revit property sets not written: {e:#}")),
    }
}

/// Which bodies to draw from the saved graphics: every boxed one with
/// `--saved-meshes`, none with `--no-saved-meshes` or `--placeholder`, and
/// otherwise doors and windows on the releases
/// whose saved graphics are read, Revit 2023 to 2027.
fn saved_mesh_scope(
    rf: &mut RevitFile,
    opts: &ConvertOptions,
) -> Option<crate::ifc::saved_meshes::SavedMeshScope> {
    use crate::ifc::saved_meshes::SavedMeshScope;
    if opts.placeholder || opts.no_saved_meshes {
        return None;
    }
    // Releases before 2023 have no readable saved graphics: they keep their
    // boxes (or family documents) without a warning per run.
    let release = rf.basic_file_info().ok()?.version;
    (2014..=2027)
        .contains(&release)
        .then_some(if opts.saved_meshes {
            SavedMeshScope::All
        } else {
            SavedMeshScope::DoorsAndWindows
        })
}

/// Step 2 of `rvt-ifc`, after the quality check: shared shapes and the 2D
/// plan maps. Returns the stderr notes.
pub fn share_and_map(
    rf: &mut RevitFile,
    model: &mut IfcModel,
    opts: &ConvertOptions,
) -> Vec<String> {
    let mut notes = Vec::new();
    if !opts.per_element_meshes {
        // `open` every exactly equal definition.
        crate::control::stage("shared shapes");
        crate::ifc::body_geometry::share_shapes_by_convention(model);
        // the 2D maps (plan symbols, extrusion bases) Revit's
        // export gives a type beside its Body map.
        crate::control::stage("plan maps");
        match crate::ifc::plan_maps::attach(rf, model) {
            Ok(report) => notes.push(format!(
                "plan maps {}",
                serde_json::to_string(&report).unwrap_or_default()
            )),
            Err(err) => notes.push(format!("warning: plan maps not read: {err}")),
        }
    }
    notes
}

/// Step 3: the IFCX-shaped graph every format is written from. `meshes` is
/// true when any of IFCX, `.frag` or USD is wanted (as `rvt-ifc` decides).
pub fn graph(model: &IfcModel, opts: &ConvertOptions, meshes: bool) -> IfcxGraph {
    IfcxGraph::from_model(
        model,
        GraphOptions {
            meshes,
            shared_shapes: !opts.per_element_meshes,
        },
    )
}

/// IFC4 STEP text from the graph. `timestamp`: Unix seconds for the header,
/// `None` for the current time.
pub fn ifc4_step(graph: &IfcxGraph, timestamp: Option<i64>) -> String {
    crate::ifc::step_writer::write_step_graph_with_options(
        graph,
        &crate::ifc::step_writer::StepOptions { timestamp },
    )
}

/// [`ifc4_step`], written to `out` as it is built, so the text is never held
/// whole. Returns how many bytes were written.
///
/// # Errors
///
/// The first error `out` returns.
pub fn ifc4_step_to<W: std::io::Write>(
    graph: &IfcxGraph,
    timestamp: Option<i64>,
    out: W,
) -> std::io::Result<u64> {
    crate::ifc::step_writer::write_step_graph_to(
        graph,
        &crate::ifc::step_writer::StepOptions { timestamp },
        out,
    )
}

/// What [`gltf_model`] draws, as `rvt-gltf`'s `--no-*` switches choose.
#[derive(Debug, Clone)]
pub struct GltfModelOptions {
    /// Draw box bodies from Revit's saved graphics (`--no-saved-meshes` off).
    pub saved_meshes: bool,
    /// Family documents and symbol meshes (`--no-family-geometry` off).
    pub family_geometry: bool,
    /// Saved materials (`--no-saved-materials` off).
    pub saved_materials: bool,
    /// Shaped stand-ins for planting and entourage (`--no-proxy-shapes` off).
    pub proxy_shapes: bool,
    /// Walker limits.
    pub walker_limits: WalkerLimits,
}

/// The model `rvt-gltf` writes, before its shared shapes: the export, then
/// the saved graphics, family documents, symbol meshes, saved materials,
/// stand-ins and Revit identity, in that order. Returns the notes a pass
/// left (a pass that cannot read leaves the bodies it has and says so).
///
/// # Errors
///
/// The export's errors; a cancelled `control` gives [`crate::Error::Cancelled`].
pub fn gltf_model(
    rf: &mut RevitFile,
    opts: &GltfModelOptions,
    control: &WalkerControl,
) -> crate::Result<(IfcModel, Vec<String>)> {
    let mut notes = Vec::new();
    let mut model = RvtDocExporter.export_with_mode_limits_and_control(
        rf,
        ExportQualityMode::Scaffold,
        opts.walker_limits,
        control,
    )?;
    // a family file's own forms, as `rvt-ifc` draws them.
    if rf.is_family_document() {
        if let Err(e) = crate::ifc::family_file::attach(rf, &mut model, opts.saved_meshes) {
            notes.push(format!("family file forms not read: {e}"));
        }
    }
    let scope = if opts.saved_meshes {
        crate::ifc::saved_meshes::default_scope(rf)
            .map(|_| crate::ifc::saved_meshes::SavedMeshScope::All)
    } else {
        None
    };
    if opts.family_geometry {
        if let Err(e) = crate::ifc::symbol_meshes::add_site_content(rf, &mut model) {
            notes.push(format!("site content not read: {e}"));
        }
    }
    if let Some(scope) = scope {
        crate::control::stage("saved graphics");
        if let Err(e) =
            crate::ifc::saved_meshes::attach_scoped_with_control(rf, &mut model, scope, control)
        {
            notes.push(format!("saved graphics not read, bodies unchanged: {e}"));
        }
        match crate::ifc::saved_meshes::attach_display_bodies(rf, &mut model, control) {
            Ok(r) => notes.push(format!(
                "openings {}",
                serde_json::to_string(&r).unwrap_or_default()
            )),
            Err(e) => notes.push(format!("saved graphics not read, openings not cut: {e}")),
        }
    }
    if opts.family_geometry {
        crate::control::stage("family documents");
        if let Err(e) = crate::ifc::family_bodies::attach(rf, &mut model) {
            notes.push(format!("family documents not read, bodies unchanged: {e}"));
        }
        if let Err(e) = crate::ifc::symbol_meshes::attach(rf, &mut model) {
            notes.push(format!(
                "family symbol meshes not read, bodies unchanged: {e}"
            ));
        }
    }
    if opts.saved_materials {
        crate::control::stage("saved materials");
        if let Err(e) = crate::ifc::saved_meshes::attach_materials(rf, &mut model) {
            notes.push(format!("saved materials not read: {e}"));
        }
    }
    if opts.proxy_shapes {
        crate::ifc::proxy_shapes::attach(&mut model);
    }
    crate::ifc::split_single_slab_roofs(&mut model);
    crate::control::stage("Revit identity");
    crate::ifc::revit_identity::attach(rf, &mut model);
    Ok((model, notes))
}

/// `rvt-ifc --diagnostics` sidecar bytes.
pub fn diagnostics_json(diagnostics: &ExportDiagnostics) -> anyhow::Result<Vec<u8>> {
    Ok(serde_json::to_vec_pretty(diagnostics)?)
}

// ---- rvt-info --json (one file) -------------------------------------------

#[derive(Serialize)]
struct Report<'a> {
    #[serde(flatten)]
    summary: &'a crate::reader::Summary,
    /// Worksharing, save history and document identity
    /// (`rvt::metadata::FileMetadata`).
    document: Option<&'a crate::metadata::FileMetadata>,
    /// A family's types and type parameters from its `PartAtom` type table
    /// (`PartAtom::family_types`); absent when it has none.
    #[serde(skip_serializing_if = "Option::is_none")]
    family_types: Option<Vec<crate::part_atom::FamilyType>>,
}

/// `rvt-info <file> --json` without the trailing newline: release, build,
/// basic file info and document identity.
pub fn info_json(rf: &mut RevitFile, redact: bool) -> anyhow::Result<String> {
    // Lossy summarise — tolerates partial parses (missing PartAtom,
    // unreadable class inventory) and reports whatever identity was found.
    let mut summary = rf.summarize_lossy()?.value;
    let mut document = rf.metadata().ok();
    if redact {
        if let Some(p) = &summary.original_path {
            summary.original_path = Some(crate::redact::redact_sensitive(p));
        }
        if let Some(doc) = document.as_mut() {
            doc.redact();
        }
    }
    let family_types = summary
        .partatom
        .as_ref()
        .and_then(|atom| atom.family_types().ok())
        .filter(|types| !types.is_empty());
    Ok(serde_json::to_string_pretty(&Report {
        summary: &summary,
        document: document.as_ref(),
        family_types,
    })?)
}

// ---- rvt-elements -----------------------------------------------------------

#[derive(Serialize)]
struct ByteRange {
    start: usize,
    end: usize,
}

#[derive(Serialize)]
struct ElementOut {
    id: Option<u32>,
    class_name: String,
    byte_range: ByteRange,
    fields: Vec<serde_json::Value>,
    parameters: Vec<serde_json::Value>,
    provenance: serde_json::Value,
    /// The name Revit shows for the element's category, from its partition
    /// record.
    #[serde(skip_serializing_if = "Option::is_none")]
    category: Option<&'static str>,
    /// The category's `BuiltInCategory` enum name.
    #[serde(skip_serializing_if = "Option::is_none")]
    builtin_category: Option<&'static str>,
    /// The category's parent, for a subcategory such as `Runs`.
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_category: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    typed: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct CountsOut {
    total: usize,
    by_class: BTreeMap<String, usize>,
}

/// `rvt-elements` output without the trailing newline: the production
/// decoded elements (or, with `counts`, their class counts) as JSON.
pub fn elements_json(
    rf: &mut RevitFile,
    min_confidence: f32,
    typed: bool,
    counts: bool,
    limits: WalkerLimits,
    control: &WalkerControl,
) -> anyhow::Result<String> {
    let elements: Vec<DecodedElement> =
        iter_elements_with_control(rf, PRODUCTION_ELEMENT_MIN_SCORE, limits, control)?
            .filter(|e| e.meets_confidence(min_confidence))
            .collect();
    if counts {
        let mut by_class = BTreeMap::<String, usize>::new();
        for element in &elements {
            *by_class.entry(element.class.clone()).or_insert(0) += 1;
        }
        return Ok(serde_json::to_string_pretty(&CountsOut {
            total: elements.len(),
            by_class,
        })?);
    }
    // Revit 2023 to 2027 only; other files keep their AProperty* entries.
    let saved = crate::elements::parameters::saved_parameters_lossy(rf, &elements);
    let categories = crate::ifc::revit_identity::element_categories(rf);
    let category_of = |element: &DecodedElement| {
        element.id.and_then(|id| categories.get(&id)).and_then(|c| {
            let (name, parent) = crate::builtin_parameters::category_name_and_parent(*c)?;
            Some((name, crate::builtin_parameters::category(*c)?.0, parent))
        })
    };
    let out: Vec<ElementOut> = elements
        .iter()
        .map(|element| ElementOut {
            category: category_of(element).map(|c| c.0),
            builtin_category: category_of(element).map(|c| c.1),
            parent_category: category_of(element).and_then(|c| c.2),
            id: element.id,
            class_name: element.class.clone(),
            byte_range: ByteRange {
                start: element.byte_range.start,
                end: element.byte_range.end,
            },
            fields: element
                .fields
                .iter()
                .map(|(name, value)| field_json(name, value))
                .collect(),
            parameters: crate::elements::parameters::parameter_entries_from_decoded(element)
                .iter()
                .map(|e| e.to_json_value())
                .chain(saved.value.entries_for(element))
                .collect(),
            provenance: element.provenance.to_json_value(),
            typed: if typed {
                crate::elements::typed_json::mvp_typed_view(element)
            } else {
                None
            },
        })
        .collect();
    Ok(serde_json::to_string_pretty(&out)?)
}

fn field_json(name: &str, value: &InstanceField) -> serde_json::Value {
    match value {
        InstanceField::Pointer { raw } => serde_json::json!({
            "name": name,
            "kind": "pointer",
            "slot_a": raw[0],
            "slot_b": raw[1],
        }),
        InstanceField::ElementId { tag, id } => serde_json::json!({
            "name": name,
            "kind": "element_id",
            "tag": tag,
            "id": id,
        }),
        InstanceField::RefContainer { col_a, col_b } => serde_json::json!({
            "name": name,
            "kind": "ref_container",
            "count": col_a.len(),
            "col_a": col_a,
            "col_b": col_b,
        }),
        InstanceField::Integer {
            value,
            signed,
            size,
        } => serde_json::json!({
            "name": name,
            "kind": "integer",
            "value": value,
            "signed": signed,
            "size": size,
        }),
        InstanceField::Float { value, size } => serde_json::json!({
            "name": name,
            "kind": "float",
            "value": value,
            "size": size,
        }),
        InstanceField::Bool(v) => serde_json::json!({
            "name": name,
            "kind": "bool",
            "value": v,
        }),
        InstanceField::Guid(bytes) => serde_json::json!({
            "name": name,
            "kind": "guid",
            "bytes": bytes.as_slice(),
        }),
        InstanceField::String(s) => serde_json::json!({
            "name": name,
            "kind": "string",
            "value": s,
        }),
        InstanceField::Vector(items) => serde_json::json!({
            "name": name,
            "kind": "vector",
            "len": items.len(),
        }),
        InstanceField::Bytes(b) => serde_json::json!({
            "name": name,
            "kind": "bytes",
            "len": b.len(),
        }),
    }
}
