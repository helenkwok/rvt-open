//! WASM bindings — JS-callable wrappers around the viewer API,
//! listed in `docs/viewer-api.md` ("The JavaScript bindings").
//!
//! Its own crate, so the core library builds as a plain `rlib`.
//! Build with, from the repository root:
//!
//! ```bash
//! wasm-pack build rvt-wasm --target web --out-dir ../viewer/pkg --out-name rvt
//! ```
//!
//! Every binding is JSON round-trippable via
//! `serde-wasm-bindgen`: caller passes a JS object in, gets a
//! JS object back. No custom FFI conversions needed. Matches the
//! surface documented in `docs/viewer-api.md`.
//!
//! Network APIs (`fetch`, `XMLHttpRequest`, `WebSocket`) are
//! deliberately excluded per the client-side-only posture
//! (`docs/viewer-privacy-posture.md`). CI grep-checks the
//! compiled `.wasm` for those imports and fails the build if any
//! appear.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use wasm_bindgen::JsCast;

use wasm_bindgen::prelude::*;

use rvt::RevitFile;
use rvt::control::{ProgressEvent, Stage, WalkerControl};
use rvt::ifc::{
    ExportDiagnostics, ExportNote, ExportOptions, ExportQualityMode, ExportRun, IfcModel,
    RvtDocExporter, SavedMeshChoice,
    gltf::model_to_glb,
    record_unread_streams, run_export_with_limits_and_control,
    scene_graph::{
        ElementInfoPanel, SceneNode, Schedule, build_scene_graph, build_schedule,
        distinct_ifc_types, element_info_panel,
    },
    sheet::{SheetOptions, render_plan_svg},
    step_writer::write_step,
};
use rvt::reader::{OpenLimits, ProjectPhase};
use rvt::research::native_document::{NATIVE_RELEASES, Options};
use rvt::research::native_element_parameters;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Deserialize)]
struct WasmWalkerLimits {
    max_scan_bytes: Option<usize>,
    max_candidates: Option<usize>,
    max_trial_offsets: Option<usize>,
    max_record_decode_bytes: Option<usize>,
    max_container_records: Option<usize>,
}

fn err_str<E: std::fmt::Display>(e: E) -> JsValue {
    JsValue::from_str(&e.to_string())
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console, js_name = error)]
    fn console_error(s: &str);
}

/// Turn a Rust panic into a thrown JS `Error` carrying the panic message and
/// source location (also logged to `console.error`), instead of a bare
/// installed the hook, so a 2027 panic surfaced with no message. It is now
/// installed once per wasm instance by the `start` function below, so every
/// binding is covered. After a panic the instance state is undefined: reload it.
fn install_panic_hook() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            let msg = format!("rvt-open wasm panic: {info}");
            console_error(&msg);
            // Throwing from an import unwinds the JS stack through the wasm frames.
            wasm_bindgen::throw_val(js_sys::Error::new(&msg).into());
        }));
    });
}

/// wasm32 has a 4 GiB linear-memory ceiling, and a failed allocation aborts with a bare
/// `RuntimeError: unreachable` that no panic hook sees (the 2027 DACH and Snowdon
/// architectural samples reach it). This allocator wrapper throws a readable JS `Error` instead
/// when the allocator returns null. The message is a constant: formatting one would allocate
/// again, in the very state that just failed.
/// The 64-bit build (wasm64) gets the same wrapper; browsers cap a 64-bit memory at
/// 16 GiB (JS API limits), so its message names that ceiling.
#[cfg(target_family = "wasm")]
mod oom {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

    struct ReadableOom;

    #[cfg(target_arch = "wasm32")]
    const MESSAGE: &str = "rvt-open wasm out of memory: this file needs more than the 4 GiB the browser build can address. Convert it with the native rvt-ifc";
    #[cfg(not(target_arch = "wasm32"))]
    const MESSAGE: &str = "rvt-open wasm out of memory: this file needs more than the 16 GiB the 64-bit browser build can address. Convert it with the native rvt-ifc";

    /// Bytes allocated and not yet freed, and the most there were since the
    /// last [`reset_peak`] (where a conversion's peak is).
    pub static LIVE: AtomicUsize = AtomicUsize::new(0);
    pub static PEAK: AtomicUsize = AtomicUsize::new(0);

    fn grew(n: usize) {
        let live = LIVE.fetch_add(n, Relaxed) + n;
        PEAK.fetch_max(live, Relaxed);
    }

    pub fn reset_peak() {
        PEAK.store(LIVE.load(Relaxed), Relaxed);
    }

    fn throw_oom() -> ! {
        wasm_bindgen::throw_str(MESSAGE)
    }

    // SAFETY: every call forwards to `System` unchanged; a null result diverges by throwing.
    unsafe impl GlobalAlloc for ReadableOom {
        unsafe fn alloc(&self, l: Layout) -> *mut u8 {
            let p = unsafe { System.alloc(l) };
            if p.is_null() {
                throw_oom()
            }
            grew(l.size());
            p
        }
        unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
            LIVE.fetch_sub(l.size(), Relaxed);
            unsafe { System.dealloc(p, l) }
        }
        unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
            let p = unsafe { System.alloc_zeroed(l) };
            if p.is_null() {
                throw_oom()
            }
            grew(l.size());
            p
        }
        unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
            let q = unsafe { System.realloc(p, l, n) };
            if q.is_null() {
                throw_oom()
            }
            LIVE.fetch_sub(l.size(), Relaxed);
            grew(n);
            q
        }
    }

    #[global_allocator]
    static ALLOC: ReadableOom = ReadableOom;
}

#[wasm_bindgen(start)]
fn start() {
    install_panic_hook();
}

/// `[live, peak, wasm]`: bytes the converter holds now, the most it held
/// since the last `reset_peak()` (or since load), and the size of the wasm
/// linear memory, which never shrinks. Zeros outside wasm.
#[wasm_bindgen]
pub fn memory_stats() -> Vec<f64> {
    #[cfg(target_family = "wasm")]
    {
        use std::sync::atomic::Ordering::Relaxed;
        #[cfg(target_arch = "wasm32")]
        let wasm = (core::arch::wasm32::memory_size(0) as f64) * 65536.0;
        // `core::arch::wasm64` is unstable: ask the JS side for the memory's size.
        #[cfg(not(target_arch = "wasm32"))]
        let wasm = {
            let memory: js_sys::WebAssembly::Memory = wasm_bindgen::memory().unchecked_into();
            memory
                .buffer()
                .unchecked_into::<js_sys::ArrayBuffer>()
                .byte_length() as f64
        };
        vec![
            oom::LIVE.load(Relaxed) as f64,
            oom::PEAK.load(Relaxed) as f64,
            wasm,
        ]
    }
    #[cfg(not(target_family = "wasm"))]
    vec![0.0; 3]
}

/// Start a new peak for `memory_stats()` at the bytes held now.
#[wasm_bindgen]
pub fn reset_peak() {
    #[cfg(target_family = "wasm")]
    oom::reset_peak();
}

/// Open an RVT / RFA byte slice and return the raw `IfcModel` as
/// a JS object. The viewer passes this around and then calls the
/// other bindings to derive scene graph / glTF / schedule / etc.
#[wasm_bindgen(js_name = openRvtBytes)]
pub fn open_rvt_bytes(bytes: &[u8]) -> Result<JsValue, JsValue> {
    install_panic_hook();
    let mut rf = RevitFile::open_bytes(bytes.to_vec()).map_err(err_str)?;
    let model = RvtDocExporter
        .export_with_limits(&mut rf, rvt::walker::WalkerLimits::default())
        .map_err(err_str)?;
    serde_wasm_bindgen::to_value(&model).map_err(err_str)
}

/// Open an RVT / RFA byte slice with explicit walker scan limits.
#[wasm_bindgen(js_name = openRvtBytesWithLimits)]
pub fn open_rvt_bytes_with_limits(bytes: &[u8], limits: JsValue) -> Result<JsValue, JsValue> {
    install_panic_hook();
    let mut rf = RevitFile::open_bytes(bytes.to_vec()).map_err(err_str)?;
    let model = RvtDocExporter
        .export_with_limits(&mut rf, walker_limits_from_js(limits)?)
        .map_err(err_str)?;
    serde_wasm_bindgen::to_value(&model).map_err(err_str)
}

/// Open an RVT / RFA byte slice and return `{ model, diagnostics }`.
///
/// The diagnostics payload matches `rvt-ifc --diagnostics` and is
/// intended for viewer bug reports and export-readiness messaging.
#[wasm_bindgen(js_name = openRvtBytesWithDiagnostics)]
pub fn open_rvt_bytes_with_diagnostics(bytes: &[u8]) -> Result<JsValue, JsValue> {
    install_panic_hook();
    let mut rf = RevitFile::open_bytes(bytes.to_vec()).map_err(err_str)?;
    let result = RvtDocExporter
        .export_with_diagnostics_and_limits(&mut rf, rvt::walker::WalkerLimits::default())
        .map_err(err_str)?;
    serde_wasm_bindgen::to_value(&result).map_err(err_str)
}

/// Open an RVT / RFA byte slice and return `{ model, diagnostics }`
/// with explicit walker scan limits.
#[wasm_bindgen(js_name = openRvtBytesWithDiagnosticsAndLimits)]
pub fn open_rvt_bytes_with_diagnostics_and_limits(
    bytes: &[u8],
    limits: JsValue,
) -> Result<JsValue, JsValue> {
    install_panic_hook();
    let mut rf = RevitFile::open_bytes(bytes.to_vec()).map_err(err_str)?;
    let result = RvtDocExporter
        .export_with_diagnostics_and_limits(&mut rf, walker_limits_from_js(limits)?)
        .map_err(err_str)?;
    serde_wasm_bindgen::to_value(&result).map_err(err_str)
}

/// Open an RVT / RFA byte slice with an IFC export quality mode and
/// return `{ model, diagnostics }`.
///
/// `mode` is one of `scaffold`, `typed-no-geometry`, `geometry`, or
/// `strict`. Stronger modes fail closed when the decoded
/// coverage cannot satisfy the requested bar — the viewer uses this to
/// surface honest confidence rather than over-claiming geometry.
///
/// Doors and windows are drawn from the meshes Revit saved for them, as
/// `rvt-ifc` does by default, on Revit 2023 to 2027 files.
#[wasm_bindgen(js_name = openRvtBytesWithDiagnosticsMode)]
pub fn open_rvt_bytes_with_diagnostics_mode(bytes: &[u8], mode: &str) -> Result<JsValue, JsValue> {
    install_panic_hook();
    export_with_limits(
        bytes,
        mode,
        OpenLimits::default(),
        rvt::walker::WalkerLimits::default(),
        true,
        None,
        None,
    )
}

/// [`open_rvt_bytes_with_diagnostics_mode`] under the resource limits of
/// `limits`, a JS object
/// `{ largeModel?, maxStreamBytes?, maxInflateBytes?, savedMeshes?, phase? }` or
/// `null` for the defaults. A field left out keeps its default; the first
/// three are `rvt-ifc`'s flags of the same names, resolved by
/// [`rvt::cli::open_limits`] (`largeModel` is `--large-model`, and an
/// explicit byte limit wins over it):
///
/// - `largeModel: true` reads streams up to 2 GiB and lets a stream inflate
///   up to the 4 GiB a wasm32 address space holds (`rvt-ifc` gives 8 GiB on
///   a 64-bit host), and lifts the walker's scan cap the same way.
/// - `maxStreamBytes` caps one stream as stored; a larger stream is not read.
/// - `maxInflateBytes` caps one gzip member's inflated size; a stream's
///   members together may inflate to this or 1 GiB, whichever is larger.
///   A value past what a 32-bit `usize` holds saturates at 4 GiB minus 1.
/// - `savedMeshes` (default `true`) draws the doors and windows of Revit 2023
///   to 2027 files from the meshes Revit saved for them, as `rvt-ifc` does
///   without `--no-saved-meshes`, and the bounding boxes left after them from
///   their families' own documents, as its default `--family-geometry` does;
///   `false` keeps their bounding boxes and skips both passes.
/// - `phase` (a phase's name, matched exactly first, or its ElementId as a
///   number) exports that project phase, as `rvt-ifc --phase` does: an element
///   that does not exist in it, and a room whose own phase is not it, are left
///   out. Left out, the export is the project's last phase. A phase the file
///   lacks throws, with the file's phases in the message. The result's
///   `phases` lists them (`{ id, name? }`, in order, the last being the
///   default).
///
/// Byte limits are whole numbers of bytes. A stream a limit keeps from being
/// read is listed in `diagnostics` (the `stream_not_read_resource_limit`
/// skipped item and a `Not read whole:` warning per stream), and the
/// model is missing what it holds. Unknown fields and fractional, negative
/// or non-finite byte limits throw.
///
/// `progress`, when given, is a JS function `(stage, done, total)` called as
/// the decoder works: `stage` is a [`Stage`] name as `rvt-gltf --progress`
/// prints it (`schema-parse`, `candidate-scan`, `element-decode`,
/// `partition-scan`, `saved-graphics`, `saved-meshes`), `done` a count and
/// `total` the count it is out of, or `null` where the stage has no
/// denominator yet. It is called when a stage begins, when it ends,
/// and each time a stage with a total passes another whole percent, so a
/// stage costs at most about a hundred calls. An exception it throws is
/// ignored.
#[wasm_bindgen(js_name = openRvtBytesWithDiagnosticsModeAndOpenLimits)]
pub fn open_rvt_bytes_with_diagnostics_mode_and_open_limits(
    bytes: &[u8],
    mode: &str,
    limits: JsValue,
    progress: Option<js_sys::Function>,
) -> Result<JsValue, JsValue> {
    install_panic_hook();
    let request = limits_from_js(limits)?;
    export_with_limits(
        bytes,
        mode,
        request.open_limits,
        request.walker_limits,
        request.saved_meshes,
        request.phase,
        progress,
    )
}

/// The JS function a decode reports its progress to, and the last report it
/// forwarded. A `js_sys::Function` is not `Send + Sync`, which
/// [`WalkerControl::with_progress`] requires, so it sits here, where one
/// single-threaded wasm instance reads it, and the closure handed to the
/// control captures nothing.
struct ProgressSink {
    callback: js_sys::Function,
    last: Option<(Stage, Option<u64>)>,
}

thread_local! {
    static PROGRESS: RefCell<Option<ProgressSink>> = const { RefCell::new(None) };
}

/// The control of one decode: reports to `progress` when a function is given
/// and does nothing otherwise.
fn progress_control(progress: Option<js_sys::Function>) -> WalkerControl {
    match progress {
        Some(callback) => {
            PROGRESS.with(|slot| {
                *slot.borrow_mut() = Some(ProgressSink {
                    callback,
                    last: None,
                });
            });
            WalkerControl::new().with_progress(forward_progress)
        }
        None => WalkerControl::default(),
    }
}

fn clear_progress() {
    PROGRESS.with(|slot| {
        slot.borrow_mut().take();
    });
}

/// Hands `event` to the JS function unless it is a report within the same
/// whole percent of the same stage as the last one handed over. The decoder
/// reports each megabyte scanned and every 256th candidate, which the page
/// has no use for; a stage with no total reports once, when it begins.
fn forward_progress(event: ProgressEvent) {
    PROGRESS.with(|slot| {
        let Ok(mut slot) = slot.try_borrow_mut() else {
            return;
        };
        let Some(sink) = slot.as_mut() else {
            return;
        };
        let percent = event
            .total
            .filter(|total| *total > 0)
            .map(|total| event.done.saturating_mul(100) / total);
        let finished = event.total.is_some_and(|total| event.done >= total);
        let same_stage = sink.last.is_some_and(|(stage, _)| stage == event.stage);
        let same_percent = sink.last.is_some_and(|(_, last)| last == percent);
        if same_stage && same_percent && !finished {
            return;
        }
        sink.last = Some((event.stage, percent));
        let total = event
            .total
            .map_or(JsValue::NULL, |total| JsValue::from_f64(total as f64));
        let _ = sink.callback.call3(
            &JsValue::NULL,
            &JsValue::from_str(event.stage.as_str()),
            &JsValue::from_f64(event.done as f64),
            &total,
        );
    });
}

fn export_with_limits(
    bytes: &[u8],
    mode: &str,
    open_limits: OpenLimits,
    walker_limits: rvt::walker::WalkerLimits,
    saved_meshes: bool,
    phase: Option<String>,
    progress: Option<js_sys::Function>,
) -> Result<JsValue, JsValue> {
    let control = progress_control(progress);
    let result = export_with_control(
        bytes,
        mode,
        open_limits,
        walker_limits,
        saved_meshes,
        phase,
        &control,
    );
    clear_progress();
    result
}

/// What the viewer gets back: [`rvt::ifc::ExportResult`]'s two fields, and the
/// project's phases in order (the last is the export's unless the request
/// named another), empty where the export applies no phase.
#[derive(serde::Serialize)]
struct ExportJson {
    model: IfcModel,
    diagnostics: ExportDiagnostics,
    phases: Vec<ProjectPhase>,
}

fn export_with_control(
    bytes: &[u8],
    mode: &str,
    open_limits: OpenLimits,
    walker_limits: rvt::walker::WalkerLimits,
    saved_meshes: bool,
    phase: Option<String>,
    control: &WalkerControl,
) -> Result<JsValue, JsValue> {
    let quality_mode = ExportQualityMode::parse(mode).map_err(err_str)?;
    let mut rf = RevitFile::open_bytes_with_limits(bytes.to_vec(), open_limits).map_err(err_str)?;
    // The build `rvt-ifc` runs ([`run_export_with_limits_and_control`]), with doors and windows drawn
    // from their saved meshes (Revit 2023 to 2027 only) unless `saved_meshes`
    // is false.
    let options = ExportOptions::default()
        .with_mode(quality_mode)
        .with_saved_meshes(if saved_meshes {
            SavedMeshChoice::Default
        } else {
            SavedMeshChoice::Off
        })
        .with_phase(phase);
    let run = run_export_with_limits_and_control(&mut rf, &options, walker_limits, control)
        .map_err(err_str)?;
    let ExportRun {
        model,
        mut diagnostics,
        unread,
        notes,
        ..
    } = run;
    // Saved graphics only improve bodies: when they cannot be read the model
    // keeps its bounding boxes and the diagnostics carry a warning saying so.
    let mut warned = false;
    for note in &notes {
        if let ExportNote::SavedMeshesSkipped { error, .. } = note {
            diagnostics.warnings.push(format!(
                "Saved graphics not read, door and window bodies unchanged: {error}"
            ));
            warned = true;
        }
    }
    if warned {
        // Recounts the warnings, and puts the unread-stream ones last.
        record_unread_streams(&mut diagnostics, &unread);
    }
    let phases = rf.project_phases();
    serde_wasm_bindgen::to_value(&ExportJson {
        model,
        diagnostics,
        phases,
    })
    .map_err(err_str)
}

#[derive(Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LimitsOptions {
    large_model: Option<bool>,
    max_stream_bytes: Option<f64>,
    max_inflate_bytes: Option<f64>,
    saved_meshes: Option<bool>,
    phase: Option<PhaseOption>,
}

/// A phase as the page names it: its name, or its ElementId as a number.
#[derive(serde::Deserialize)]
#[serde(untagged)]
enum PhaseOption {
    Name(String),
    Id(f64),
}

impl PhaseOption {
    /// What `RevitFile::set_export_phase` takes: a name is matched first and an
    /// ElementId only where no phase has that name.
    fn into_request(self) -> Result<String, JsValue> {
        match self {
            Self::Name(name) => Ok(name),
            Self::Id(id) if id.is_finite() && id.fract() == 0.0 => Ok(format!("{id:.0}")),
            Self::Id(id) => Err(JsValue::from_str(&format!(
                "phase must be a name or a whole-number ElementId, got {id}"
            ))),
        }
    }
}

/// The resource limits, saved-mesh switch and export phase of one open.
struct ExportRequest {
    open_limits: OpenLimits,
    walker_limits: rvt::walker::WalkerLimits,
    saved_meshes: bool,
    phase: Option<String>,
}

fn whole_bytes(field: &str, value: Option<f64>) -> Result<Option<u64>, JsValue> {
    match value {
        None => Ok(None),
        Some(bytes) if bytes.is_finite() && bytes >= 0.0 && bytes.fract() == 0.0 => {
            Ok(Some(bytes as u64))
        }
        Some(bytes) => Err(JsValue::from_str(&format!(
            "{field} must be a whole number of bytes, got {bytes}"
        ))),
    }
}

fn limits_from_js(limits: JsValue) -> Result<ExportRequest, JsValue> {
    let options: LimitsOptions = if limits.is_null() || limits.is_undefined() {
        LimitsOptions::default()
    } else {
        serde_wasm_bindgen::from_value(limits).map_err(err_str)?
    };
    let large_model = options.large_model.unwrap_or(false);
    let max_inflate_bytes = whole_bytes("maxInflateBytes", options.max_inflate_bytes)?
        .map(|bytes| usize::try_from(bytes).unwrap_or(usize::MAX));
    let open_limits = rvt::cli::open_limits(
        whole_bytes("maxStreamBytes", options.max_stream_bytes)?,
        max_inflate_bytes,
        large_model,
    );
    let mut walker_limits = rvt::walker::WalkerLimits::default();
    if large_model {
        walker_limits.max_scan_bytes = rvt::cli::LARGE_MODEL_MAX_INFLATE_BYTES;
    }
    Ok(ExportRequest {
        open_limits,
        walker_limits,
        saved_meshes: options.saved_meshes.unwrap_or(true),
        phase: options.phase.map(PhaseOption::into_request).transpose()?,
    })
}

/// Quick summary — reads only the cheap metadata (BasicFileInfo +
/// PartAtom + stream inventory) and returns instantly even for
/// multi-hundred-megabyte RFAs. Used by the viewer for the
/// progressive-loading splash before the full model parse
/// completes. Returns a [`rvt::reader::Summary`] as a JS object.
#[wasm_bindgen(js_name = quickSummary)]
pub fn quick_summary(bytes: &[u8]) -> Result<JsValue, JsValue> {
    install_panic_hook();
    let mut rf = RevitFile::open_bytes(bytes.to_vec()).map_err(err_str)?;
    let summary = rf.summarize_lossy().map_err(err_str)?.value;
    serde_wasm_bindgen::to_value(&summary).map_err(err_str)
}

/// Document identity — release, worksharing, central model, last saved
/// (time and user), document GUID, save counter, and every
/// `BasicFileInfo` `Key: value` line — as a
/// [`rvt::metadata::FileMetadata`] JS object. Reads only the two
/// identity streams straight from `bytes` (no copy of the file), so it
/// returns instantly on any file size. Errors when `bytes` is not a
/// readable Revit file.
#[wasm_bindgen(js_name = fileMetadata)]
pub fn file_metadata(bytes: &[u8]) -> Result<JsValue, JsValue> {
    install_panic_hook();
    let metadata =
        rvt::metadata::read_metadata_from(std::io::Cursor::new(bytes)).map_err(err_str)?;
    serde_wasm_bindgen::to_value(&metadata).map_err(err_str)
}

/// Spreadsheet schedule of an exported model as CSV
/// ([`rvt::ifc::schedule_csv`]): `kind` is `"elements"` (every building
/// element the IFC export writes, on the storey it puts it on) or `"rooms"`;
/// `metric` switches lengths to metres; `excel`
/// prefixes a UTF-8 byte-order mark so Excel on Windows reads non-ASCII
/// names correctly.
#[wasm_bindgen(js_name = scheduleCsv)]
pub fn js_schedule_csv(
    model: JsValue,
    kind: &str,
    metric: bool,
    excel: bool,
) -> Result<String, JsValue> {
    install_panic_hook();
    use rvt::ifc::schedule_csv::{CsvOptions, LengthUnit, elements_csv, rooms_csv};
    let model: IfcModel = serde_wasm_bindgen::from_value(model).map_err(err_str)?;
    let options = CsvOptions {
        unit: if metric {
            LengthUnit::Metres
        } else {
            LengthUnit::Feet
        },
        excel_bom: excel,
    };
    match kind {
        "elements" => Ok(elements_csv(&model, &options)),
        "rooms" => Ok(rooms_csv(&model, &options)),
        other => Err(JsValue::from_str(&format!(
            "unknown schedule kind {other:?}; expected \"elements\" or \"rooms\""
        ))),
    }
}

/// Build the scene-graph tree for a model.
#[wasm_bindgen(js_name = buildSceneGraph)]
pub fn js_build_scene_graph(model: JsValue) -> Result<JsValue, JsValue> {
    install_panic_hook();
    let model: IfcModel = serde_wasm_bindgen::from_value(model).map_err(err_str)?;
    let scene: SceneNode = build_scene_graph(&model);
    serde_wasm_bindgen::to_value(&scene).map_err(err_str)
}

/// Distinct IFC types in the scene — source of truth for the
/// layer-toggle UI.
#[wasm_bindgen(js_name = distinctIfcTypes)]
pub fn js_distinct_ifc_types(scene: JsValue) -> Result<JsValue, JsValue> {
    install_panic_hook();
    let scene: SceneNode = serde_wasm_bindgen::from_value(scene).map_err(err_str)?;
    let types = distinct_ifc_types(&scene);
    serde_wasm_bindgen::to_value(&types).map_err(err_str)
}

/// Populate the element info panel for a click target.
#[wasm_bindgen(js_name = elementInfoPanel)]
pub fn js_element_info_panel(model: JsValue, entity_index: usize) -> Result<JsValue, JsValue> {
    install_panic_hook();
    let model: IfcModel = serde_wasm_bindgen::from_value(model).map_err(err_str)?;
    let panel: Option<ElementInfoPanel> = element_info_panel(&model, entity_index);
    serde_wasm_bindgen::to_value(&panel).map_err(err_str)
}

/// What [`saved_parameters`] returns, as JSON: the file's release, whether
/// the saved parameters of that release are read and how far they are
/// measured, and each requested element that has a record, with its
/// parameters. The keys are the binding's contract.
#[derive(serde::Serialize)]
struct SavedParametersJson {
    release: u32,
    /// Whether the saved parameters of `release` are read at all.
    release_read: bool,
    /// Read, but only experimentally: outside `releases_measured`.
    release_experimental: bool,
    /// The first and last release whose saved parameters are read.
    releases_read: [u32; 2],
    /// The first and last release the reader is measured on; the releases read
    /// before it are read experimentally.
    releases_measured: [u32; 2],
    /// By ElementId. An element with no record is not listed. Each entry is
    /// `serde_json::to_value` of the parameter, as `rvt-elements` prints it.
    elements: BTreeMap<String, Vec<serde_json::Value>>,
    unresolved_family_slots: usize,
    instance_parameters_without_value: usize,
}

/// A whole, non-negative number a JS number holds exactly, as an ElementId.
fn element_id(id: f64) -> Result<u64, JsValue> {
    const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;
    if (0.0..=MAX_SAFE_INTEGER).contains(&id) && id.fract() == 0.0 {
        Ok(id as u64)
    } else {
        Err(JsValue::from_str(&format!(
            "an ElementId must be a whole number, got {id}"
        )))
    }
}

/// The saved parameters of the elements `ids` (ElementIds), as JSON text:
/// `{ release, release_read, release_experimental, releases_read,
/// releases_measured, elements, unresolved_family_slots,
/// instance_parameters_without_value }`.
///
/// It is the read `rvt-elements` and Python's `decoded_elements()` list as an
/// element's `parameters` (`rvt::research::native_element_parameters`),
/// so each entry of `elements[id]` is the entry those print for that element:
/// name (null for a built-in), `parameter_id`, `kind`, `storage_type`,
/// `value` in internal units with `si_value` and `si_unit` where the unit is
/// known, `level` (`instance` or `type`) and the record it was read from.
/// `release_read` is false, and `elements` empty, for a file of a release
/// outside `releases_read`; `release_experimental` is true for one of
/// `releases_read` that `releases_measured` leaves out. An element whose id has
/// no record is left out of `elements`. The two counts are what the read could
/// not list for these elements.
///
/// `bytes` is the whole file and is opened here for this call alone, with
/// `limits` as `openRvtBytesWithDiagnosticsModeAndOpenLimits` takes them
/// (`largeModel`, `maxStreamBytes`, `maxInflateBytes`; `null` for the
/// defaults), and dropped when it returns: nothing of the file stays in this
/// instance. The page calls it in a worker of its own, for one element at a
/// time, and ends that worker when it has answered. An id that is not a whole
/// number, a file that is not a Revit file and a file whose parameters cannot
/// be read throw.
#[wasm_bindgen(js_name = savedParameters)]
pub fn saved_parameters(bytes: Vec<u8>, ids: Vec<f64>, limits: JsValue) -> Result<String, JsValue> {
    install_panic_hook();
    let request = limits_from_js(limits)?;
    let ids = ids
        .into_iter()
        .map(element_id)
        .collect::<Result<BTreeSet<u64>, JsValue>>()?;
    let mut rf = RevitFile::open_bytes_with_limits(bytes, request.open_limits).map_err(err_str)?;
    let release = rf.layout_release().map_err(err_str)?;
    let release_read = NATIVE_RELEASES.contains(&release);
    let measured = Options::default().releases;
    let read = if release_read {
        native_element_parameters::read(&mut rf, &ids)
            .map_err(|error| JsValue::from_str(&format!("{error:#}")))?
    } else {
        native_element_parameters::ElementParameters::default()
    };
    let json = SavedParametersJson {
        release,
        release_read,
        release_experimental: release_read && !measured.contains(&release),
        releases_read: [*NATIVE_RELEASES.start(), *NATIVE_RELEASES.end()],
        releases_measured: [*measured.start(), *measured.end()],
        elements: read
            .elements
            .iter()
            .map(|(id, parameters)| {
                let entries: Vec<serde_json::Value> = parameters
                    .iter()
                    .filter_map(|parameter| serde_json::to_value(parameter).ok())
                    .collect();
                (id.to_string(), entries)
            })
            .collect(),
        unresolved_family_slots: read.unresolved_family_slots,
        instance_parameters_without_value: read.instance_parameters_without_value,
    };
    serde_json::to_string(&json).map_err(err_str)
}

/// Build a flat schedule table of the BuildingElements the IFC export
/// writes, each on the storey it puts it on.
#[wasm_bindgen(js_name = buildSchedule)]
pub fn js_build_schedule(model: JsValue) -> Result<JsValue, JsValue> {
    install_panic_hook();
    let model: IfcModel = serde_wasm_bindgen::from_value(model).map_err(err_str)?;
    let schedule: Schedule = build_schedule(&model);
    serde_wasm_bindgen::to_value(&schedule).map_err(err_str)
}

/// Render `model` as a glTF 2.0 binary. Returns a `Uint8Array`
/// the frontend feeds into Three.js's `GLTFLoader`.
#[wasm_bindgen(js_name = modelToGlb)]
pub fn js_model_to_glb(model: JsValue) -> Result<Vec<u8>, JsValue> {
    install_panic_hook();
    let model: IfcModel = serde_wasm_bindgen::from_value(model).map_err(err_str)?;
    Ok(model_to_glb(&model))
}

/// Render `model` as a one-page 3D PDF: the glTF export's meshes as U3D in a
/// PDF 3D annotation, with a flat poster for viewers that show no 3D.
/// Returns the PDF bytes; callers wrap them in a Blob for download.
#[wasm_bindgen(js_name = modelToPdf3d)]
pub fn js_model_to_pdf3d(model: JsValue) -> Result<Vec<u8>, JsValue> {
    install_panic_hook();
    let model: IfcModel = serde_wasm_bindgen::from_value(model).map_err(err_str)?;
    Ok(rvt::ifc::pdf3d::model_to_pdf3d(&model))
}

/// Render `model` as an IFC4 STEP document. Returns the ISO-10303-21
/// text; callers wrap it in a Blob for download.
#[wasm_bindgen(js_name = modelToIfcStep)]
pub fn js_model_to_ifc_step(model: JsValue) -> Result<String, JsValue> {
    install_panic_hook();
    let model: IfcModel = serde_wasm_bindgen::from_value(model).map_err(err_str)?;
    Ok(write_step(&model))
}

/// Render `model` as a 2D SVG plan view (sheet). Options control
/// dimensions + labels + background; pass `null` for defaults.
#[wasm_bindgen(js_name = renderPlanSvg)]
pub fn js_render_plan_svg(model: JsValue, options: JsValue) -> Result<String, JsValue> {
    install_panic_hook();
    let model: IfcModel = serde_wasm_bindgen::from_value(model).map_err(err_str)?;
    let options: SheetOptions = if options.is_null() || options.is_undefined() {
        SheetOptions::default()
    } else {
        serde_wasm_bindgen::from_value(serde_js_options_bridge(options)?).map_err(err_str)?
    };
    Ok(render_plan_svg(&model, &options))
}

// Internal shim: serde-wasm-bindgen sometimes needs a
// round-trip through JSON to normalise JS objects with mixed
// numeric types (Three.js often passes Float32 where we want
// f32; this forces explicit coercion).
fn serde_js_options_bridge(input: JsValue) -> Result<JsValue, JsValue> {
    Ok(input)
}

fn walker_limits_from_js(input: JsValue) -> Result<rvt::walker::WalkerLimits, JsValue> {
    if input.is_null() || input.is_undefined() {
        return Ok(rvt::walker::WalkerLimits::default());
    }
    let parsed: WasmWalkerLimits = serde_wasm_bindgen::from_value(input).map_err(err_str)?;
    let defaults = rvt::walker::WalkerLimits::default();
    Ok(rvt::walker::WalkerLimits {
        max_scan_bytes: parsed.max_scan_bytes.unwrap_or(defaults.max_scan_bytes),
        max_candidates: parsed.max_candidates.unwrap_or(defaults.max_candidates),
        max_trial_offsets: parsed
            .max_trial_offsets
            .unwrap_or(defaults.max_trial_offsets),
        max_per_record_decode_bytes: parsed
            .max_record_decode_bytes
            .unwrap_or(defaults.max_per_record_decode_bytes),
        max_container_records: parsed
            .max_container_records
            .unwrap_or(defaults.max_container_records),
    })
}

// ---- Read-and-convert API (the browser converter page) ----------
//
// Every function below opens the bytes, converts and returns bytes or text,
// through the same `rvt::convert` calls as the native CLIs, so the output is
// byte-identical to `rvt-ifc` / `rvt-schedule` / `rvt-gltf` /
// `rvt-elements` / `rvt-info --json` for the same input and options. This
// crate depends on the core with `default-features = false`.

/// Options shared by the convert functions, named after the `rvt-ifc`
/// flags. Every field is optional.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ConvertOpts {
    /// `--convention`: `"revit"` (default) or `"open"`.
    convention: Option<String>,
    /// `--mode`: `"scaffold"` (default), `"typed-no-geometry"`, `"geometry"`, `"strict"`.
    mode: Option<String>,
    placeholder: bool,
    diagnostic_proxies: bool,
    /// `--saved-meshes`: draw every box body from the graphics Revit saved.
    /// Default true here (the browser converter): boxes are what a viewer shows
    /// otherwise.
    saved_meshes: Option<bool>,
    /// `--family-geometry`: draw box bodies the saved graphics leave from
    /// the family's embedded document. Default true here.
    family_geometry: Option<bool>,
    /// Try the family document before the saved graphics. Default false:
    /// the saved graphics are exact for the instance's type, a family
    /// document draws the family's saved type.
    family_first: bool,
    /// `--saved-materials`: elements with no material take the one of their
    /// saved graphics. Default true here.
    saved_materials: Option<bool>,
    /// Revit's own property sets (built-in parameters named) and each
    /// element's `Revit` set (ElementId, category). Default true.
    revit_property_sets: Option<bool>,
    per_element_meshes: bool,
    epsg: Option<u32>,
    /// Namespace of generated GlobalIds (`--guid-namespace`).
    guid_namespace: Option<String>,
    /// `--ifc4-timestamp` as Unix seconds; default the current time.
    ifc4_timestamp: Option<i64>,
    /// `--max-walker-*`.
    limits: Option<WasmWalkerLimits>,
    /// rvt-schedule `--schedule`: `"elements"` (default) or `"rooms"`.
    schedule: Option<String>,
    /// rvt-schedule `--metric`.
    metric: bool,
    /// rvt-schedule `--excel`.
    excel: bool,
    /// rvt-elements `--min-confidence`.
    min_confidence: f32,
    /// rvt-elements `--no-typed`.
    no_typed: bool,
    /// rvt-elements `--counts`.
    counts: bool,
    /// rvt-info `--redact`.
    redact: bool,
    /// Refuse, before converting, a file the browser build cannot hold
    /// (default true). `false` tries anyway.
    size_check: Option<bool>,
    /// Room and space volumes in the visible 3D scene of the glb and `.frag`
    /// (default false, as Revit's 3D views).
    rooms: bool,
    /// Feature edges as glb lines (default true, as Revit's
    /// Shaded with Show Edges).
    edges: Option<bool>,
}

impl ConvertOpts {
    fn display(&self) -> rvt::ifc::gltf::DisplayOptions {
        rvt::ifc::gltf::DisplayOptions {
            rooms: self.rooms,
            edges: self.edges.unwrap_or(true),
        }
    }
}

struct Parsed {
    convert: rvt::convert::ConvertOptions,
    raw: ConvertOpts,
}

fn parse_opts(options: JsValue) -> Result<Parsed, JsValue> {
    let raw: ConvertOpts = if options.is_null() || options.is_undefined() {
        ConvertOpts::default()
    } else {
        serde_wasm_bindgen::from_value(options).map_err(err_str)?
    };
    let quality = match raw.mode.as_deref().unwrap_or("scaffold") {
        "scaffold" => ExportQualityMode::Scaffold,
        "typed-no-geometry" => ExportQualityMode::TypedNoGeometry,
        "geometry" => ExportQualityMode::Geometry,
        "strict" => ExportQualityMode::Strict,
        other => return Err(JsValue::from_str(&format!("unknown mode {other:?}"))),
    };
    let convention = match raw.convention.as_deref().unwrap_or("revit") {
        "revit" => rvt::ifc::ExportConvention::Revit,
        "open" => rvt::ifc::ExportConvention::Open,
        other => return Err(JsValue::from_str(&format!("unknown convention {other:?}"))),
    };
    if let Some(ns) = &raw.guid_namespace {
        rvt::ifc::validate_guid_namespace(ns).map_err(|e| JsValue::from_str(&e))?;
    }
    if raw.placeholder && raw.diagnostic_proxies {
        return Err(JsValue::from_str(
            "placeholder and diagnostic_proxies conflict",
        ));
    }
    let defaults = rvt::walker::WalkerLimits::default();
    let l = raw.limits.as_ref();
    let walker_limits = rvt::walker::WalkerLimits {
        max_scan_bytes: l
            .and_then(|l| l.max_scan_bytes)
            .unwrap_or(defaults.max_scan_bytes),
        max_candidates: l
            .and_then(|l| l.max_candidates)
            .unwrap_or(defaults.max_candidates),
        max_trial_offsets: l
            .and_then(|l| l.max_trial_offsets)
            .unwrap_or(defaults.max_trial_offsets),
        max_per_record_decode_bytes: l
            .and_then(|l| l.max_record_decode_bytes)
            .unwrap_or(defaults.max_per_record_decode_bytes),
        max_container_records: l
            .and_then(|l| l.max_container_records)
            .unwrap_or(defaults.max_container_records),
    };
    Ok(Parsed {
        convert: rvt::convert::ConvertOptions {
            placeholder: raw.placeholder,
            quality,
            convention,
            diagnostic_proxies: raw.diagnostic_proxies,
            family_geometry: raw.family_geometry.unwrap_or(true),
            family_first: raw.family_first,
            saved_meshes: raw.saved_meshes.unwrap_or(true),
            no_saved_meshes: false,
            saved_materials: raw.saved_materials.unwrap_or(true),
            proxy_shapes: true,
            per_element_meshes: raw.per_element_meshes,
            revit_property_sets: raw.revit_property_sets.unwrap_or(true),
            epsg: raw.epsg,
            guid_namespace: raw.guid_namespace.clone(),
            walker_limits,
            ..rvt::convert::ConvertOptions::default()
        },
        raw,
    })
}

/// The file's bytes as the page holds them, read by slices: the container
/// copies only the sectors it reads into wasm memory, never the whole file
/// (before, `&[u8]` copied it in and `open_bytes` copied it again).
struct JsBytes(js_sys::Uint8Array);

impl rvt::reader::ReadAt for JsBytes {
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> std::io::Result<usize> {
        let len = u64::from(self.0.length());
        if offset >= len {
            return Ok(0);
        }
        let n = buf.len().min((len - offset) as usize);
        let start = offset as u32;
        self.0
            .subarray(start, start + n as u32)
            .copy_to(&mut buf[..n]);
        Ok(n)
    }
}

/// A JS `readAt(offset, length)` returning a `Uint8Array` or `ArrayBuffer`
/// of up to `length` bytes from `offset` (shorter only at the end).
struct JsReadAt(js_sys::Function);

impl rvt::reader::ReadAt for JsReadAt {
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> std::io::Result<usize> {
        let got = self
            .0
            .call2(
                &JsValue::NULL,
                &JsValue::from(offset as f64),
                &JsValue::from(buf.len() as f64),
            )
            .map_err(|e| {
                std::io::Error::other(format!(
                    "readAt({offset}, {}) failed: {}",
                    buf.len(),
                    e.as_string()
                        .or_else(|| js_sys::Error::from(e).message().as_string())
                        .unwrap_or_default()
                ))
            })?;
        let got = js_sys::Uint8Array::new(&got);
        let n = buf.len().min(got.length() as usize);
        got.subarray(0, n as u32).copy_to(&mut buf[..n]);
        Ok(n)
    }
}

fn open_from<R: rvt::reader::ReadAt + 'static>(source: R, size: u64) -> Result<RevitFile, JsValue> {
    install_panic_hook();
    let source = rvt::reader::ReadAtSource::new(source, size);
    #[cfg(target_family = "wasm")]
    let source: Box<dyn rvt::reader::ByteSource> = Box::new(source);
    // A host build (tests, lints) needs a `Send + Sync` source, which a JS
    // handle is not: read it into memory there.
    #[cfg(not(target_family = "wasm"))]
    let source: Box<dyn rvt::reader::ByteSource> = {
        use std::io::Read;
        let mut source = source;
        let mut bytes = Vec::new();
        source.read_to_end(&mut bytes).map_err(err_str)?;
        Box::new(std::io::Cursor::new(bytes))
    };
    RevitFile::open_reader(source, size, OpenLimits::default()).map_err(err_str)
}

fn open(bytes: &js_sys::Uint8Array) -> Result<RevitFile, JsValue> {
    open_from(JsBytes(bytes.clone()), u64::from(bytes.length()))
}

/// `rvt-ifc` up to the graph: export, quality check, shared shapes, plan
/// maps. Returns the graph, the diagnostics and the notes.
fn ifc_pipeline(
    mut rf: RevitFile,
    opts: &rvt::convert::ConvertOptions,
    meshes: bool,
) -> Result<(rvt::ifc::IfcxGraph, ExportDiagnostics, Vec<String>), JsValue> {
    let exported = rvt::convert::export(&mut rf, opts).map_err(err_str)?;
    finish_pipeline(rf, exported, opts, meshes)
}

fn finish_pipeline(
    mut rf: RevitFile,
    exported: rvt::convert::Exported,
    opts: &rvt::convert::ConvertOptions,
    meshes: bool,
) -> Result<(rvt::ifc::IfcxGraph, ExportDiagnostics, Vec<String>), JsValue> {
    let rvt::convert::Exported {
        mut model,
        diagnostics,
        mut notes,
        ..
    } = exported;
    opts.quality.validate(&diagnostics).map_err(err_str)?;
    notes.extend(rvt::convert::share_and_map(&mut rf, &mut model, opts));
    // The graph needs nothing more from the file: free its caches first.
    drop(rf);
    Ok((
        rvt::convert::graph(&model, opts, meshes),
        diagnostics,
        notes,
    ))
}

fn schedule_csv(model: &IfcModel, raw: &ConvertOpts) -> Result<String, JsValue> {
    use rvt::ifc::schedule_csv::{CsvOptions, LengthUnit, elements_csv, rooms_csv};
    let options = CsvOptions {
        unit: if raw.metric {
            LengthUnit::Metres
        } else {
            LengthUnit::Feet
        },
        excel_bom: raw.excel,
    };
    match raw.schedule.as_deref().unwrap_or("elements") {
        "elements" => Ok(elements_csv(model, &options)),
        "rooms" => Ok(rooms_csv(model, &options)),
        other => Err(JsValue::from_str(&format!(
            "unknown schedule {other:?}; expected \"elements\" or \"rooms\""
        ))),
    }
}

/// IFC4 STEP text, as `rvt-ifc <file> -o out.ifc` writes it.
#[wasm_bindgen]
pub fn convert_ifc(bytes: &js_sys::Uint8Array, options: JsValue) -> Result<String, JsValue> {
    let p = parse_opts(options)?;
    let rf = open(bytes)?;
    let (graph, _, _) = ifc_pipeline(rf, &p.convert, false)?;
    Ok(rvt::convert::ifc4_step(&graph, p.raw.ifc4_timestamp))
}

/// IFCX (IFC5 alpha) JSON text, as `rvt-ifc --ifcx` writes it.
#[wasm_bindgen]
pub fn convert_ifcx(bytes: &js_sys::Uint8Array, options: JsValue) -> Result<String, JsValue> {
    let p = parse_opts(options)?;
    let rf = open(bytes)?;
    let (graph, _, _) = ifc_pipeline(rf, &p.convert, true)?;
    let (text, _, _) = rvt::ifc::write_ifcx_graph_layers(&graph).map_err(err_str)?;
    Ok(text)
}

/// ThatOpen Fragments 3.6.0 (`.frag`, zlib-deflated), as `rvt-ifc --frag`
/// writes it: pass the bytes to `@thatopen/fragments`.
#[wasm_bindgen]
pub fn convert_frag(bytes: &js_sys::Uint8Array, options: JsValue) -> Result<Vec<u8>, JsValue> {
    let p = parse_opts(options)?;
    let rf = open(bytes)?;
    let (graph, _, _) = ifc_pipeline(rf, &p.convert, true)?;
    let (frag, _) =
        rvt::ifc::frag_writer::write_frag_graph_with(&graph, &p.raw.display()).map_err(err_str)?;
    Ok(frag)
}

/// OpenUSD text (`.usda`), as `rvt-ifc --usda` writes it.
#[wasm_bindgen]
pub fn convert_usda(bytes: &js_sys::Uint8Array, options: JsValue) -> Result<String, JsValue> {
    let p = parse_opts(options)?;
    let rf = open(bytes)?;
    let (graph, _, _) = ifc_pipeline(rf, &p.convert, true)?;
    rvt::ifc::write_usda_graph(&graph).map_err(err_str)
}

/// Binary OpenUSD (`.usdc`), as `rvt-ifc --usdc` writes it.
#[wasm_bindgen]
pub fn convert_usdc(bytes: &js_sys::Uint8Array, options: JsValue) -> Result<Vec<u8>, JsValue> {
    let p = parse_opts(options)?;
    let rf = open(bytes)?;
    let (graph, _, _) = ifc_pipeline(rf, &p.convert, true)?;
    rvt::ifc::write_usdc_graph(&graph).map_err(err_str)
}

/// glTF binary, as `rvt-gltf [--saved-meshes]` writes it.
#[wasm_bindgen]
pub fn convert_glb(bytes: &js_sys::Uint8Array, options: JsValue) -> Result<Vec<u8>, JsValue> {
    let p = parse_opts(options)?;
    let mut rf = open(bytes)?;
    let mut exported = rvt::convert::export_base(&mut rf, &p.convert).map_err(err_str)?;
    rvt::convert::refine(&mut rf, &mut exported, &p.convert);
    Ok(rvt::ifc::gltf::model_to_glb_with(
        &exported.model,
        &p.raw.display().into(),
    ))
}

/// Element or room schedule as CSV, as `rvt-schedule` writes it
/// (`schedule`, `metric`, `excel` options).
#[wasm_bindgen]
pub fn schedules_csv(bytes: &js_sys::Uint8Array, options: JsValue) -> Result<String, JsValue> {
    let p = parse_opts(options)?;
    let mut rf = open(bytes)?;
    let exported = rvt::convert::export_base(&mut rf, &p.convert).map_err(err_str)?;
    schedule_csv(&exported.model, &p.raw)
}

/// Decoded elements (or `counts`) as JSON, as `rvt-elements` prints it.
#[wasm_bindgen]
pub fn elements_json(bytes: &js_sys::Uint8Array, options: JsValue) -> Result<String, JsValue> {
    let p = parse_opts(options)?;
    let mut rf = open(bytes)?;
    elements_text(&mut rf, &p)
}

fn elements_text(rf: &mut RevitFile, p: &Parsed) -> Result<String, JsValue> {
    let json = rvt::convert::elements_json(
        rf,
        p.raw.min_confidence,
        !p.raw.no_typed,
        p.raw.counts,
        p.convert.walker_limits,
        &WalkerControl::new(),
    )
    .map_err(err_str)?;
    Ok(json + "\n")
}

/// Export diagnostics as JSON, as `rvt-ifc --diagnostics` writes them.
#[wasm_bindgen]
pub fn diagnostics_json(bytes: &js_sys::Uint8Array, options: JsValue) -> Result<String, JsValue> {
    let p = parse_opts(options)?;
    let mut rf = open(bytes)?;
    let exported = rvt::convert::export(&mut rf, &p.convert).map_err(err_str)?;
    let json = rvt::convert::diagnostics_json(&exported.diagnostics).map_err(err_str)?;
    String::from_utf8(json).map_err(err_str)
}

/// Release, version, build and basic file info as JSON, as
/// `rvt-info <file> --json` prints it.
#[wasm_bindgen]
pub fn info(bytes: &js_sys::Uint8Array, options: JsValue) -> Result<String, JsValue> {
    let p = parse_opts(options)?;
    let mut rf = open(bytes)?;
    Ok(rvt::convert::info_json(&mut rf, p.raw.redact).map_err(err_str)? + "\n")
}

fn set(obj: &js_sys::Object, key: &str, value: &JsValue) -> Result<(), JsValue> {
    js_sys::Reflect::set(obj, &JsValue::from_str(key), value).map(|_| ())
}

/// Peak wasm memory per stored byte of the file. With the walk
/// memos released and saved graphics in batches of 256, the wasm memory the
/// browser build reached was 2,302-2,303 MB for DACH 2024 and 2027 and Golden
/// Nugget 2024 (139-148 MB stored: 15.5-16.6x), 665 MB for rac_basic (34x,
/// small files carry a fixed cost). 15, under every large-file ratio, refuses
/// only files that cannot fit (over about 270 MB); a file just under that can
/// still run out, with the readable out-of-memory error.
const PEAK_PER_STORED_BYTE: u64 = 15;
/// What the browser build can address: wasm32's 4 GiB.
#[cfg(not(target_arch = "wasm64"))]
const WASM_MEMORY_LIMIT: u64 = 4 << 30;
/// What the 64-bit build can address: browsers cap a 64-bit memory at 16 GiB (JS API limits).
#[cfg(target_arch = "wasm64")]
const WASM_MEMORY_LIMIT: u64 = 16 << 30;

/// The peak memory a conversion of `rf` is estimated to need, from its
/// streams' stored sizes.
fn estimated_peak(rf: &RevitFile) -> u64 {
    let stored: u64 = rf
        .stream_names()
        .iter()
        .filter_map(|n| rf.stream_size(n))
        .sum();
    stored.saturating_mul(PEAK_PER_STORED_BYTE)
}

/// Throw a `RangeError`-like typed error (`name` "RvtOpenTooLarge") before any
/// work when the estimate exceeds [`WASM_MEMORY_LIMIT`], instead of failing minutes later
/// with the out-of-memory error.
fn check_fits(rf: &RevitFile) -> Result<(), JsValue> {
    let need = estimated_peak(rf);
    if need <= WASM_MEMORY_LIMIT {
        return Ok(());
    }
    let e = js_sys::Error::new(&format!(
        "rvt-open wasm: this file needs about {} MB, more than the {} MB the browser build can address. Convert it with the native rvt-ifc, or pass size_check: false to try anyway",
        need >> 20,
        WASM_MEMORY_LIMIT >> 20
    ));
    e.set_name("RvtOpenTooLarge");
    Err(e.into())
}

/// The estimate [`convert_all`] checks before converting: `{ estimated_bytes,
/// limit_bytes, fits }`, from the file's stream sizes (reads only the
/// container's directory).
#[wasm_bindgen]
pub fn memory_estimate(bytes: &js_sys::Uint8Array) -> Result<JsValue, JsValue> {
    let rf = open(bytes)?;
    let need = estimated_peak(&rf);
    let v = serde_json::json!({
        "estimated_bytes": need,
        "limit_bytes": WASM_MEMORY_LIMIT,
        "fits": need <= WASM_MEMORY_LIMIT,
    });
    Ok(v.serialize(&serde_wasm_bindgen::Serializer::json_compatible())?)
}

/// Steps [`convert_all`] reports through its `progress` callback.
const STEPS: u32 = 13;

/// Forwards `rvt::control::stage` labels to a JS progress callback as
/// `progress(step, of, label)` with the current step; cleared on drop.
struct StageHook;

impl StageHook {
    fn install(f: js_sys::Function, n: std::rc::Rc<std::cell::Cell<u32>>) -> Self {
        rvt::control::set_stage_hook(Some(Box::new(move |label: &str| {
            // A failing progress callback must not stop the conversion here;
            // the next tick reports its error.
            let _ = f.call3(
                &JsValue::NULL,
                &JsValue::from(n.get()),
                &JsValue::from(STEPS),
                &JsValue::from_str(label),
            );
        })));
        StageHook
    }
}

impl Drop for StageHook {
    fn drop(&mut self) {
        rvt::control::set_stage_hook(None);
    }
}

/// Open once and return every output: `{ ifc, ifcx, ifcx_diagnostics,
/// ifcx_ledger, frag, frag_stats, usda, usdc, glb, schedule_csv,
/// elements_json, diagnostics_json, info_json, materials_takeoff_csv,
/// manifest_json, manifest }`.
/// Text outputs are strings, binary ones `Uint8Array`s; `manifest` (and the
/// same as JSON text, `manifest_json`) lists each output's size plus the
/// export counts and warnings.
///
/// `progress`, when given, is called as `progress(step, of, label)` before
/// each step (`step` from 1 to `of`); a worker can `postMessage` from it.
#[wasm_bindgen]
pub fn convert_all(
    bytes: &js_sys::Uint8Array,
    options: JsValue,
    progress: Option<js_sys::Function>,
) -> Result<JsValue, JsValue> {
    convert_all_object(|| open(bytes), options, progress)
}

/// [`convert_all`] reading the file by slices through `readAt(offset,
/// length)`, which returns a `Uint8Array` (or `ArrayBuffer`) of up to
/// `length` bytes from `offset`; `size` is the file's length. The file is
/// never held whole, neither in wasm memory nor, if `readAt` reads a
/// `File`, in JS. Reads are synchronous: in a worker, `readAt` can be
/// `(o, n) => new FileReaderSync().readAsArrayBuffer(file.slice(o, o + n))`.
/// The output is the same as [`convert_all`]'s for the same bytes.
#[wasm_bindgen]
pub fn convert_all_reader(
    size: f64,
    read_at: js_sys::Function,
    options: JsValue,
    progress: Option<js_sys::Function>,
) -> Result<JsValue, JsValue> {
    if !(size.is_finite() && size >= 0.0) {
        return Err(JsValue::from_str("size must be a byte count"));
    }
    convert_all_object(
        || open_from(JsReadAt(read_at), size as u64),
        options,
        progress,
    )
}

/// The element table in `format`: `"jsonl"`, `"xlsx"`, `"parquet"` or
/// `"sqlite"` (`"sqlite"` in the 32-bit build only), as `rvt-schedule --format <format>`
/// writes it.
#[wasm_bindgen]
pub fn element_table(
    bytes: &js_sys::Uint8Array,
    options: JsValue,
    format: &str,
) -> Result<Vec<u8>, JsValue> {
    let format = table_format(format)?;
    let p = parse_opts(options)?;
    let mut rf = open(bytes)?;
    let exported = rvt::convert::export(&mut rf, &p.convert).map_err(err_str)?;
    drop(rf);
    let rows = rvt::ifc::element_table::element_rows(&exported.model, &exported.quantities);
    drop(exported);
    rvt::ifc::table_formats::write_table(&rows, format).map_err(err_str)
}

/// The element table format named `name`, if this build writes it.
fn table_format(name: &str) -> Result<rvt::ifc::table_formats::TableFormat, JsValue> {
    let format = rvt::ifc::table_formats::TableFormat::parse(name).ok_or_else(|| {
        JsValue::from_str(&format!(
            "unknown table format {name:?}; expected \"jsonl\", \"xlsx\", \"parquet\" or \"sqlite\""
        ))
    })?;
    if !format.available() {
        return Err(JsValue::from_str(&format!(
            "this build has no {name} writer (it is in the 32-bit package only)"
        )));
    }
    Ok(format)
}

/// Write each output straight to a file handle the page passes, one output
/// at a time, so no two outputs are in memory together: `handles` maps an
/// output's name to a `FileSystemSyncAccessHandle` (the browser's private
/// disk, OPFS, opened in a worker with `createSyncAccessHandle()`). Only the
/// outputs named are made. Names: those of [`convert_all`] (`ifc`, `ifcx`,
/// `ifcx_diagnostics`, `ifcx_ledger`, `frag`, `frag_stats`, `usda`, `usdc`,
/// `glb`, `schedule_csv`, `elements_json`, `diagnostics_json`, `info_json`,
/// `materials_takeoff_csv`), the element table `elements_jsonl`,
/// `elements_xlsx`, `elements_parquet` and `elements_sqlite` (the last in
/// the 32-bit build only), and `manifest`. Each handle is truncated, written from offset 0 and
/// flushed; the page closes it.
///
/// Returns the output manifest (also written to `handles.manifest` when
/// given): format `rvt-open-output`, its schema version, the decode limits
/// and, for each output, its size and SHA-256. `progress` is as for
/// [`convert_all`].
#[wasm_bindgen]
pub fn convert_to_handles(
    bytes: &js_sys::Uint8Array,
    options: JsValue,
    handles: js_sys::Object,
    progress: Option<js_sys::Function>,
) -> Result<JsValue, JsValue> {
    convert_handles(|| open(bytes), options, handles, progress)
}

/// [`convert_to_handles`] reading the file by slices through `readAt`, as
/// [`convert_all_reader`] does.
#[wasm_bindgen]
pub fn convert_to_handles_reader(
    size: f64,
    read_at: js_sys::Function,
    options: JsValue,
    handles: js_sys::Object,
    progress: Option<js_sys::Function>,
) -> Result<JsValue, JsValue> {
    if !(size.is_finite() && size >= 0.0) {
        return Err(JsValue::from_str("size must be a byte count"));
    }
    convert_handles(
        || open_from(JsReadAt(read_at), size as u64),
        options,
        handles,
        progress,
    )
}

/// An output of [`convert_all_with`].
enum Out {
    Text(String),
    Bytes(Vec<u8>),
}

impl Out {
    fn bytes(&self) -> &[u8] {
        match self {
            Out::Text(t) => t.as_bytes(),
            Out::Bytes(b) => b,
        }
    }
}

/// Where [`convert_all_with`] puts each output it makes.
trait Sink {
    /// Whether to make output `key`.
    fn wants(&self, key: &str) -> bool;
    fn put(&mut self, key: &str, out: Out) -> Result<(), JsValue>;
}

/// The outputs of [`convert_all`], in a JS object.
struct ObjectSink(js_sys::Object);

impl Sink for ObjectSink {
    fn wants(&self, key: &str) -> bool {
        !key.starts_with("elements_") || key == "elements_json"
    }
    fn put(&mut self, key: &str, out: Out) -> Result<(), JsValue> {
        let value: JsValue = match out {
            Out::Text(t) => JsValue::from_str(&t),
            Out::Bytes(b) => js_sys::Uint8Array::from(&b[..]).into(),
        };
        set(&self.0, key, &value)
    }
}

/// The outputs of [`convert_to_handles`], each written to its handle.
struct HandleSink {
    handles: js_sys::Object,
    manifest: rvt::ifc::run_report::OutputManifest,
}

/// Bytes per `write` call: what JS copies at once, not the output's size.
const HANDLE_CHUNK: usize = 8 << 20;

impl HandleSink {
    fn handle(&self, key: &str) -> Option<JsValue> {
        js_sys::Reflect::get(&self.handles, &JsValue::from_str(key))
            .ok()
            .filter(|h| !h.is_undefined() && !h.is_null())
    }

    /// Truncate `handle`, write `bytes` from offset 0 and flush it.
    fn write(handle: &JsValue, key: &str, bytes: &[u8]) -> Result<(), JsValue> {
        let method = |name: &str| -> Result<js_sys::Function, JsValue> {
            js_sys::Reflect::get(handle, &JsValue::from_str(name))?
                .dyn_into::<js_sys::Function>()
                .map_err(|_| {
                    JsValue::from_str(&format!(
                        "handles.{key} is not a FileSystemSyncAccessHandle (no {name}())"
                    ))
                })
        };
        method("truncate")?.call1(handle, &JsValue::from(0))?;
        let write = method("write")?;
        let mut at = 0usize;
        for chunk in bytes.chunks(HANDLE_CHUNK) {
            let opts = js_sys::Object::new();
            set(&opts, "at", &JsValue::from(at as f64))?;
            let n = write
                .call2(handle, &js_sys::Uint8Array::from(chunk).into(), &opts)?
                .as_f64()
                .unwrap_or(chunk.len() as f64);
            if n as usize != chunk.len() {
                return Err(JsValue::from_str(&format!(
                    "handles.{key}: wrote {n} of {} bytes (is the disk full?)",
                    chunk.len()
                )));
            }
            at += chunk.len();
        }
        method("flush")?.call0(handle)?;
        Ok(())
    }
}

impl Sink for HandleSink {
    fn wants(&self, key: &str) -> bool {
        self.handle(key).is_some()
    }
    fn put(&mut self, key: &str, out: Out) -> Result<(), JsValue> {
        let Some(handle) = self.handle(key) else {
            return Ok(());
        };
        Self::write(&handle, key, out.bytes())?;
        self.manifest.add(key, key, out.bytes());
        Ok(())
    }
}

fn convert_all_object(
    open: impl FnOnce() -> Result<RevitFile, JsValue>,
    options: JsValue,
    progress: Option<js_sys::Function>,
) -> Result<JsValue, JsValue> {
    let out = js_sys::Object::new();
    // First key, as before; its text is set at the end.
    set(&out, "manifest_json", &JsValue::from_str(""))?;
    let mut sink = ObjectSink(out);
    let manifest = convert_all_with(open, options, progress, &mut sink)?;
    let out = sink.0;
    set(
        &out,
        "manifest_json",
        &JsValue::from_str(&serde_json::to_string_pretty(&manifest).map_err(err_str)?),
    )?;
    let manifest = manifest.serialize(&serde_wasm_bindgen::Serializer::json_compatible())?;
    set(&out, "manifest", &manifest)?;
    Ok(out.into())
}

fn convert_handles(
    open: impl FnOnce() -> Result<RevitFile, JsValue>,
    options: JsValue,
    handles: js_sys::Object,
    progress: Option<js_sys::Function>,
) -> Result<JsValue, JsValue> {
    let p = parse_opts(options.clone())?;
    let limits =
        rvt::ifc::run_report::DecodeLimits::new(&OpenLimits::default(), &p.convert.walker_limits);
    let mut sink = HandleSink {
        handles,
        manifest: rvt::ifc::run_report::OutputManifest::new(
            "rvt-wasm",
            "",
            rvt::ifc::run_report::RunStatus::Ok,
            limits,
        ),
    };
    for key in [
        "elements_jsonl",
        "elements_xlsx",
        "elements_parquet",
        "elements_sqlite",
    ] {
        if sink.wants(key) {
            table_format(&key["elements_".len()..])?;
        }
    }
    convert_all_with(open, options, progress, &mut sink)?;
    let json = sink.manifest.to_json();
    if let Some(handle) = sink.handle("manifest") {
        HandleSink::write(&handle, "manifest", json.as_bytes())?;
    }
    sink.manifest
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .map_err(Into::into)
}

/// The steps of [`convert_all`], freeing each stage's memory before the next:
/// what the 4 GiB wasm memory has to hold at once is the peak of
/// one stage, not the sum of all of them. Every output goes to the sink as
/// soon as it is written, and only the outputs the sink wants are made; the
/// file handle and its caches go before the graph is built; the model goes
/// once the graph holds what the writers need. Returns [`convert_all`]'s
/// manifest.
fn convert_all_with(
    open: impl FnOnce() -> Result<RevitFile, JsValue>,
    options: JsValue,
    progress: Option<js_sys::Function>,
    sink: &mut dyn Sink,
) -> Result<serde_json::Value, JsValue> {
    // Steps between the ticks report their phases with the same step number
    // (`rvt::control::stage`), so a page sees "walk elements" and so on while
    // the long steps run.
    let n = std::rc::Rc::new(std::cell::Cell::new(0u32));
    let _hook = progress.clone().map(|f| StageHook::install(f, n.clone()));
    let tick = |label: &str| -> Result<(), JsValue> {
        n.set(n.get() + 1);
        if let Some(f) = &progress {
            f.call3(
                &JsValue::NULL,
                &JsValue::from(n.get()),
                &JsValue::from(STEPS),
                &JsValue::from_str(label),
            )?;
        }
        Ok(())
    };
    let p = parse_opts(options)?;
    tick("open")?;
    let mut rf = open()?;
    if p.raw.size_check.unwrap_or(true) {
        check_fits(&rf)?;
    }
    let mut sizes = serde_json::Map::new();
    let mut emit = |sink: &mut dyn Sink, k: &str, v: Out| -> Result<(), JsValue> {
        sizes.insert(k.into(), v.bytes().len().into());
        sink.put(k, v)
    };
    let wants_any = |sink: &dyn Sink, keys: &[&str]| keys.iter().any(|k| sink.wants(k));
    tick("file info")?;
    if sink.wants("info_json") {
        let info = rvt::convert::info_json(&mut rf, p.raw.redact).map_err(err_str)? + "\n";
        emit(sink, "info_json", Out::Text(info))?;
    }
    // The exporter runs once: the schedule reads its model as exported, the
    // glTF and the element table after the saved graphics, the IFC formats
    // after shared shapes.
    tick("export model")?;
    let mut exported = rvt::convert::export_base(&mut rf, &p.convert).map_err(err_str)?;
    tick("schedule CSV")?;
    if sink.wants("schedule_csv") {
        emit(
            sink,
            "schedule_csv",
            Out::Text(schedule_csv(&exported.model, &p.raw)?),
        )?;
    }
    // The decoded elements read the walk the exporter just memoised; done
    // here, the file handle can go before the graph is built.
    tick("elements")?;
    if sink.wants("elements_json") {
        emit(
            sink,
            "elements_json",
            Out::Text(elements_text(&mut rf, &p)?),
        )?;
    }
    rf.release_walk_memos();
    tick("saved graphics, georeference")?;
    rvt::convert::refine(&mut rf, &mut exported, &p.convert);
    let tables: Vec<rvt::ifc::table_formats::TableFormat> =
        rvt::ifc::table_formats::TableFormat::ALL
            .into_iter()
            .filter(|f| sink.wants(&format!("elements_{}", f.name())))
            .collect();
    if !tables.is_empty() {
        let rows = rvt::ifc::element_table::element_rows(&exported.model, &exported.quantities);
        for format in tables {
            let bytes = rvt::ifc::table_formats::write_table(&rows, format).map_err(err_str)?;
            emit(
                sink,
                &format!("elements_{}", format.name()),
                Out::Bytes(bytes),
            )?;
        }
    }
    tick("glTF")?;
    if sink.wants("glb") {
        let glb = rvt::ifc::gltf::model_to_glb_with(&exported.model, &p.raw.display().into());
        emit(sink, "glb", Out::Bytes(glb))?;
    }
    tick("shared shapes, plan maps")?;
    let rvt::convert::Exported {
        mut model,
        diagnostics,
        mut notes,
        quantities,
        ..
    } = exported;
    // the materials takeoff, estimated (box) elements in rows of
    // their own.
    if sink.wants("materials_takeoff_csv") {
        emit(
            sink,
            "materials_takeoff_csv",
            Out::Text(rvt::ifc::quantities::takeoff_csv(&quantities, true)),
        )?;
    }
    drop(quantities);
    p.convert.quality.validate(&diagnostics).map_err(err_str)?;
    let graph_keys = [
        "ifc",
        "ifcx",
        "ifcx_diagnostics",
        "ifcx_ledger",
        "frag",
        "frag_stats",
        "usda",
        "usdc",
    ];
    let mut frag_stats = String::new();
    if wants_any(sink, &graph_keys) {
        notes.extend(rvt::convert::share_and_map(&mut rf, &mut model, &p.convert));
        // Nothing below reads the file: its handle, inflated streams and memos go.
        drop(rf);
        let graph = rvt::convert::graph(&model, &p.convert, true);
        drop(model);
        tick("IFC4")?;
        if sink.wants("ifc") {
            emit(
                sink,
                "ifc",
                Out::Text(rvt::convert::ifc4_step(&graph, p.raw.ifc4_timestamp)),
            )?;
        }
        tick("IFCX")?;
        if wants_any(sink, &["ifcx", "ifcx_diagnostics", "ifcx_ledger"]) {
            let (ifcx, ifcx_diag, ledger) =
                rvt::ifc::write_ifcx_graph_layers(&graph).map_err(err_str)?;
            emit(sink, "ifcx", Out::Text(ifcx))?;
            emit(sink, "ifcx_diagnostics", Out::Text(ifcx_diag))?;
            emit(
                sink,
                "ifcx_ledger",
                Out::Text(serde_json::to_string_pretty(&ledger).map_err(err_str)?),
            )?;
        }
        tick("Fragments")?;
        if wants_any(sink, &["frag", "frag_stats"]) {
            let (frag, stats) =
                rvt::ifc::frag_writer::write_frag_graph_with(&graph, &p.raw.display())
                    .map_err(err_str)?;
            emit(sink, "frag", Out::Bytes(frag))?;
            frag_stats = serde_json::to_string_pretty(&stats).map_err(err_str)?;
            emit(sink, "frag_stats", Out::Text(frag_stats.clone()))?;
        }
        tick("USD")?;
        if sink.wants("usda") {
            emit(
                sink,
                "usda",
                Out::Text(rvt::ifc::write_usda_graph(&graph).map_err(err_str)?),
            )?;
        }
        if sink.wants("usdc") {
            emit(
                sink,
                "usdc",
                Out::Bytes(rvt::ifc::write_usdc_graph(&graph).map_err(err_str)?),
            )?;
        }
        drop(graph);
    } else {
        drop(rf);
        drop(model);
        for label in ["IFC4", "IFCX", "Fragments", "USD"] {
            tick(label)?;
        }
    }
    tick("diagnostics")?;
    if sink.wants("diagnostics_json") {
        emit(
            sink,
            "diagnostics_json",
            Out::Text(
                String::from_utf8(rvt::convert::diagnostics_json(&diagnostics).map_err(err_str)?)
                    .map_err(err_str)?,
            ),
        )?;
    }
    Ok(serde_json::json!({
        "rvt_open_version": rvt::VERSION,
        "formats": {
            "ifc": "IFC4 STEP",
            "ifcx": "IFCX (IFC5 alpha) JSON",
            "frag": "ThatOpen Fragments 3.6.0, zlib-deflated",
            "usda": "OpenUSD text",
            "usdc": "OpenUSD binary (Crate 0.8.0)",
            "glb": "glTF 2.0 binary",
            "materials_takeoff_csv": "materials takeoff CSV",
        },
        "bytes": sizes,
        "exported": diagnostics.exported,
        "frag_stats": serde_json::from_str::<serde_json::Value>(&frag_stats).unwrap_or_default(),
        "notes": notes,
        "warnings": diagnostics.warnings,
    }))
}

/// the schema, whole (`streamed` false: `for_each_object`) or member by member
/// (`for_each_object_streamed`), and return `objects elements digest` (the
/// digest folds stream offset, id, tag and Adler-32 of every object in order),
/// so the two can be compared and the heap they leave read from JS.
#[wasm_bindgen]
pub fn schema_scan(bytes: &js_sys::Uint8Array, streamed: bool) -> Result<String, JsValue> {
    let mut rf = open(bytes)?;
    let (mut objects, mut elements, mut digest) = (0u64, 0u64, 0xcbf2_9ce4_8422_2325u64);
    let visit = |o: &rvt::schema_objects::SchemaObject| {
        objects += 1;
        elements += u64::from(o.element().is_some());
        for v in [
            o.frame.span.start as u64,
            o.frame.header.id,
            u64::from(o.frame.header.tag),
            u64::from(o.frame.header.adler),
        ] {
            digest = (digest ^ v).wrapping_mul(0x0100_0000_01b3);
        }
    };
    if streamed {
        rvt::schema_objects::for_each_object_streamed(&mut rf, visit).map_err(err_str)?;
    } else {
        rvt::schema_objects::for_each_object(&mut rf, visit).map_err(err_str)?;
    }
    Ok(format!("{objects} {elements} {digest:016x}"))
}

// DWG and DXF for the viewer's 2D pane. The last drawing opened is kept here, so changing the
// layout or looking up an entity does not read the file again.
thread_local! {
    static DRAWING: RefCell<Option<rvt_dwg::Read>> = const { RefCell::new(None) };
}

fn with_drawing<T>(f: impl FnOnce(&rvt_dwg::Read) -> Result<T, JsValue>) -> Result<T, JsValue> {
    DRAWING.with(|d| match d.borrow().as_ref() {
        Some(r) => f(r),
        None => Err(JsValue::from_str("no drawing is open")),
    })
}

/// Open a DWG or DXF and keep it for the other `dwg*` calls. Returns
/// `{ version, units, layouts: [{ name, isModel }], layers: [{ name, colour, on, entities }],
/// notes, counts }`, where `colour` is `#RRGGBB` or null.
#[wasm_bindgen(js_name = dwgOpen)]
pub fn dwg_open(bytes: &[u8]) -> Result<JsValue, JsValue> {
    install_panic_hook();
    let read = rvt_dwg::read_bytes(bytes, rvt_dwg::Limits::DEFAULT).map_err(err_str)?;
    let d = &read.drawing;
    let mut used = std::collections::HashMap::<&str, usize>::new();
    for e in &d.entities {
        *used.entry(e.layer.as_str()).or_default() += 1;
    }
    let summary = serde_json::json!({
        "version": d.version,
        "units": d.units,
        "layouts": d.layouts.iter().map(|l| serde_json::json!({ "name": l.name, "isModel": l.is_model })).collect::<Vec<_>>(),
        "layers": d.layers.iter().map(|l| serde_json::json!({
            "name": l.name,
            "colour": l.colour.to_rgb().map(|[r, g, b]| format!("#{r:02X}{g:02X}{b:02X}")),
            "on": l.on && !l.frozen,
            "entities": used.get(l.name.as_str()).copied().unwrap_or(0),
        })).collect::<Vec<_>>(),
        "notes": read.notes,
        "counts": rvt_dwg::report::Report::new("", bytes, d, Vec::new()).counts,
    });
    DRAWING.with(|cell| *cell.borrow_mut() = Some(read));
    serde_wasm_bindgen::to_value(&summary).map_err(err_str)
}

/// SVG of a layout of the open drawing (`null` for model space): `{ svg, layers, notDrawn,
/// truncated }`.
#[wasm_bindgen(js_name = dwgSvg)]
pub fn dwg_svg(layout: Option<String>) -> Result<JsValue, JsValue> {
    install_panic_hook();
    with_drawing(|r| {
        let s = rvt_dwg::svg::render(&r.drawing, layout.as_deref(), &rvt_dwg::Limits::DEFAULT)
            .map_err(err_str)?;
        let out = serde_json::json!({ "svg": s.svg, "layers": s.layers, "notDrawn": s.not_drawn, "truncated": s.truncated });
        serde_wasm_bindgen::to_value(&out).map_err(err_str)
    })
}

/// The Entities-table row of the entity with `handle`, as an object keyed by column, or null.
#[wasm_bindgen(js_name = dwgEntity)]
pub fn dwg_entity(handle: &str) -> Result<JsValue, JsValue> {
    install_panic_hook();
    with_drawing(|r| {
        let one = rvt_dwg::Drawing2d {
            entities: r
                .drawing
                .entities
                .iter()
                .filter(|e| e.handle == handle)
                .cloned()
                .collect(),
            ..r.drawing.clone()
        };
        let sheets = rvt_dwg::tables::sheets(&one, &rvt_dwg::tables::TableOptions::default());
        let line = rvt_dwg::tables::entities_jsonl(&sheets);
        match line.lines().next() {
            Some(l) => Ok(js_sys::JSON::parse(l)?),
            None => Ok(JsValue::NULL),
        }
    })
}

/// A table of the open drawing as CSV: `entities`, `summary`, `layers` or `blocks`; or
/// `entities.jsonl` for the Entities table as JSON Lines.
#[wasm_bindgen(js_name = dwgTable)]
pub fn dwg_table(name: &str) -> Result<String, JsValue> {
    install_panic_hook();
    with_drawing(|r| {
        let sheets = rvt_dwg::tables::sheets(&r.drawing, &rvt_dwg::tables::TableOptions::default());
        if name == "entities.jsonl" {
            return Ok(rvt_dwg::tables::entities_jsonl(&sheets));
        }
        sheets
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(name))
            .map(rvt_dwg::tables::write_csv)
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "no table {name:?}: entities, summary, layers, blocks or entities.jsonl"
                ))
            })
    })
}

/// The open drawing saved as `dxf` or `dwg` in `version` (2018, 2013, 2010, 2007, 2004, 2000).
#[wasm_bindgen(js_name = dwgSave)]
pub fn dwg_save(format: &str, version: &str) -> Result<Vec<u8>, JsValue> {
    install_panic_hook();
    use rvt_dwg::write::{CadFormat, parse_version, resave};
    let format = match format {
        "dxf" => CadFormat::Dxf,
        "dwg" => CadFormat::Dwg,
        other => {
            return Err(JsValue::from_str(&format!(
                "unknown format {other:?}: dxf or dwg"
            )));
        }
    };
    with_drawing(|r| {
        let v = parse_version(version, format).map_err(err_str)?;
        resave(r, format, v).map(|(b, _)| b).map_err(err_str)
    })
}
