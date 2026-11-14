//! The two tools that write a file: `export_ifc` and `export_gltf`.
//!
//! Each writes what `rvt-ifc` and `rvt-gltf` write from the same model, to the
//! path the caller names, and returns a summary of what went in and what was
//! left out, with the SHA-256 of the file. The file itself is never returned.

use std::io::Write;

use rmcp::model::Resource;
use rvt::control::WalkerControl;
use rvt::ifc::gltf::{GltfOptions, model_to_glb_with};
use rvt::ifc::{ExportDiagnostics, ExportNote, IfcModel};
use rvt::reader::UnreadStream;

use crate::args::{ExportGltfArgs, ExportIfcArgs, phase_of};
use crate::config::Failure;
use crate::output::{self, Written};
use crate::schema::{ExportOut, OutputOut};
use crate::store::{Entry, Store, note_texts, run_decode};
use crate::views::{bounded_diagnostics, unread_out};

/// An export's summary, and the link to its file.
pub struct Exported {
    pub out: ExportOut,
    pub link: Resource,
}

/// The model an export is written from, and what its decode noted.
struct Source<'a> {
    model: &'a IfcModel,
    diagnostics: &'a ExportDiagnostics,
    unread: &'a [UnreadStream],
    notes: &'a [String],
}

/// The time `SOURCE_DATE_EPOCH` sets for the IFC header, or `None` when it is
/// unset or empty (reproducible-builds.org), as `rvt-ifc` reads it.
fn source_date_epoch() -> Result<Option<i64>, String> {
    match std::env::var("SOURCE_DATE_EPOCH") {
        Ok(value) if value.is_empty() => Ok(None),
        Ok(value) => value
            .parse()
            .map(Some)
            .map_err(|_| format!("SOURCE_DATE_EPOCH must be Unix seconds, not {value:?}")),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(value)) => Err(format!(
            "SOURCE_DATE_EPOCH must be Unix seconds, not {value:?}"
        )),
    }
}

/// The refusal `rvt-ifc` gives with exit status 3: a resource limit kept
/// streams from being read, and the caller did not accept an incomplete file.
/// A reader that stopped at its work budget refuses nothing: the file
/// is written and the result's notes name it.
fn refuse_partial(source: &Source<'_>, allow_partial: bool) -> Result<(), Failure> {
    if !source.diagnostics.is_partial() || allow_partial {
        return Ok(());
    }
    let mut message = String::from(
        "No file written: a resource limit kept part of the model from being read, so the export would be missing what these hold:",
    );
    for stream in source.unread {
        message.push_str(&format!("\n  {stream}"));
    }
    message.push_str(
        "\nStart the server with --large-model (or --max-stream-bytes and --max-inflate-bytes) to read them, or pass allow_partial=true to write the incomplete file.",
    );
    Err(Failure::Message(message))
}

fn finish(
    entry: &Entry,
    format: &str,
    mime_type: &str,
    written: &Written,
    source: &Source<'_>,
    with_diagnostics: bool,
) -> Exported {
    let diagnostics = source.diagnostics;
    let file_name = written.path.file_name().map_or_else(
        || "export".to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    let link = Resource::new(output::file_uri(&written.path), file_name)
        .with_mime_type(mime_type)
        .with_size(written.bytes)
        .with_description(format!(
            "{format} export of {}; sha256 {}",
            entry.id, written.sha256
        ));
    let mut notes = source.notes.to_vec();
    // `unread_streams` lists the streams; the readers that stopped at a work
    // budget are listed here, since the glb's summary carries no diagnostics.
    for (reader, times) in diagnostics.work_budget_stops().into_iter().flatten() {
        notes.push(format!(
            "Warning: {reader} stopped {times} time(s) at its work budget, so the export leaves out what it would have read."
        ));
    }
    let folded: usize = diagnostics.exported.body_pieces_folded.values().sum();
    if format == "glb" && folded > 0 {
        notes.push(format!(
            "The glb also draws {folded} piece(s) of elements of separate loops that the IFC export folds into the body of another element (exported.body_pieces_folded): building_elements and entity_counts count the elements the IFC writes and leave them out."
        ));
    }
    // Bounded as `get_diagnostics` bounds it, and named in `notes` where a
    // list was held.
    let bounded = with_diagnostics.then(|| bounded_diagnostics(diagnostics, &mut notes));
    let out = ExportOut {
        model_id: entry.id.clone(),
        format: format.to_string(),
        output: OutputOut {
            path: written.path.to_string_lossy().into_owned(),
            bytes: written.bytes,
            sha256: written.sha256.clone(),
        },
        building_elements: diagnostics.exported.building_elements,
        building_elements_with_geometry: diagnostics.exported.building_elements_with_geometry,
        entity_counts: diagnostics.exported.by_ifc_type.clone(),
        partial: diagnostics.is_partial(),
        unread_streams: unread_out(source.unread),
        notes,
        diagnostics: bounded,
    };
    Exported { out, link }
}

pub fn ifc(
    store: &Store,
    args: &ExportIfcArgs,
    control: &WalkerControl,
) -> Result<Exported, Failure> {
    let entry = store.open(&args.model, phase_of(&args.phase), control)?;
    let config = store.config();
    let timestamp = source_date_epoch()?;
    let target = output::plan(
        config,
        &args.out_path,
        "ifc",
        args.overwrite,
        &entry.identity.path,
    )?;
    let switches = args.options.export_options();
    // `rvt-ifc`'s steps after the export: shared shapes and plan maps, which
    // read the file, then the IFC4 text from the graph, so the IFC this
    // server writes is the CLI's. At the phase the model was opened at.
    let opts = rvt::convert::ConvertOptions {
        walker_limits: config.walker_limits,
        guid_namespace: std::env::var("RVT_OPEN_GUID_NAMESPACE")
            .ok()
            .filter(|ns| rvt::ifc::validate_guid_namespace(ns).is_ok()),
        ..rvt::convert::ConvertOptions::rvt_ifc_defaults()
    };
    let options = switches.clone().with_phase(entry.phase_spec());
    let custom = entry.with_file(|file| {
        let mut run = run_decode(file, config, &options, control)?;
        let more = rvt::convert::share_and_map(file, &mut run.model, &opts);
        run.notes
            .extend(more.into_iter().map(|text| ExportNote::Text { text }));
        Ok::<_, Failure>(run)
    })?;
    let custom_notes = note_texts(&custom.notes);
    let source = Source {
        model: &custom.model,
        diagnostics: &custom.diagnostics,
        unread: &custom.unread,
        notes: &custom_notes,
    };
    refuse_partial(&source, args.options.allow_partial)?;
    switches
        .mode
        .validate(source.diagnostics)
        .map_err(|error| Failure::Message(error.to_string()))?;
    control.check().map_err(|_| Failure::Cancelled)?;
    let graph = rvt::convert::graph(source.model, &opts, false);
    let written = output::write(&target, args.overwrite, control, |writer| {
        rvt::convert::ifc4_step_to(&graph, timestamp, writer).map(|_| ())
    })?;
    Ok(finish(
        &entry,
        "ifc",
        "application/x-step",
        &written,
        &source,
        true,
    ))
}

pub fn gltf(
    store: &Store,
    args: &ExportGltfArgs,
    control: &WalkerControl,
) -> Result<Exported, Failure> {
    let entry = store.open(&args.model, phase_of(&args.phase), control)?;
    let config = store.config();
    let target = output::plan(
        config,
        &args.out_path,
        "glb",
        args.overwrite,
        &entry.identity.path,
    )?;
    // `rvt-gltf`'s own model and passes, so the file is the CLI's.
    let built = entry.with_file(|file| {
        let opts = rvt::convert::GltfModelOptions {
            saved_meshes: true,
            family_geometry: true,
            saved_materials: true,
            proxy_shapes: true,
            walker_limits: config.walker_limits,
        };
        let (mut model, mut notes) =
            rvt::convert::gltf_model(file, &opts, control).map_err(|error| match error {
                rvt::Error::Cancelled => Failure::Cancelled,
                other => Failure::Message(format!("the export could not read the file: {other}")),
            })?;
        notes.retain(|note| !note.starts_with("openings "));
        let unread = file.unread_streams().to_vec();
        rvt::ifc::body_geometry::share_shapes_by_convention(&mut model);
        Ok::<_, Failure>((model, notes, unread))
    })?;
    let (model, notes, unread) = built;
    let computed = entry.computed(config, control)?;
    let source = Source {
        model: &model,
        diagnostics: &computed.diagnostics,
        unread: &unread,
        notes: &notes,
    };
    refuse_partial(&source, args.allow_partial)?;
    control.check().map_err(|_| Failure::Cancelled)?;
    let glb = model_to_glb_with(&model, &GltfOptions::default());
    let written = output::write(&target, args.overwrite, control, |writer| {
        writer.write_all(&glb)
    })?;
    Ok(finish(
        &entry,
        "glb",
        "model/gltf-binary",
        &written,
        &source,
        false,
    ))
}
