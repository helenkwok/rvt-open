//! The run report (`report.json`): one JSON object per converted file,
//! saying what was written, how long it took, how many elements of each
//! category the export holds, and the export's own diagnostics.
//!
//! The diagnostics are [`super::ExportDiagnostics`] as `rvt-ifc
//! --diagnostics` writes them (what was read, what was skipped and why,
//! warnings, streams a resource limit kept from being read), embedded
//! unchanged under `diagnostics`; the report adds only what they do not
//! hold. `docs/data-outputs.md` describes every field.

use super::ExportDiagnostics;
use super::ifc_options::IfcOptionsReport;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The report's schema version; additive fields keep it.
pub const RUN_REPORT_SCHEMA_VERSION: u32 = 1;

/// One file written by the run.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct OutputFile {
    /// `ifc`, `elements-jsonl`, `glb`, `obj`, `mtl`, ...
    pub format: String,
    pub path: String,
    pub bytes: u64,
}

/// The outcome of one file's conversion.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// Everything asked for was written.
    Ok,
    /// Written, but a resource limit kept part of the input from being read
    /// (the diagnostics list the streams).
    Partial,
    /// Nothing, or not everything, was written; `error` says why.
    Failed,
    /// Not converted: its output folder already existed.
    Skipped,
}

/// `report.json`.
#[derive(Debug, Clone, Serialize)]
pub struct RunReport {
    pub schema_version: u32,
    /// The command that wrote it (`rvt-ifc`, `rvt-batch`).
    pub tool: String,
    pub tool_version: String,
    /// The input path as given.
    pub input: String,
    pub status: RunStatus,
    pub error: Option<String>,
    /// Wall-clock seconds the conversion took.
    pub seconds: f64,
    pub outputs: Vec<OutputFile>,
    /// Exported elements by Revit category, or by IFC class where an element
    /// has no category.
    pub elements_by_category: BTreeMap<String, usize>,
    /// What the IFC options (`--ifc-profile`, `--ifc-categories`) left out.
    pub ifc_options: Option<IfcOptionsReport>,
    /// The conversion's notes (what each pass drew, read or left out), as
    /// the command prints them on stderr.
    pub notes: Vec<String>,
    /// The export diagnostics, as `rvt-ifc --diagnostics` writes them.
    pub diagnostics: Option<ExportDiagnostics>,
}

impl RunReport {
    /// An empty report for `input`, written by `tool`.
    pub fn new(tool: &str, input: &str) -> RunReport {
        RunReport {
            schema_version: RUN_REPORT_SCHEMA_VERSION,
            tool: tool.into(),
            tool_version: env!("CARGO_PKG_VERSION").into(),
            input: input.into(),
            status: RunStatus::Ok,
            error: None,
            seconds: 0.0,
            outputs: Vec::new(),
            elements_by_category: BTreeMap::new(),
            ifc_options: None,
            notes: Vec::new(),
            diagnostics: None,
        }
    }

    /// Record `path` (already written) as an output of `format`.
    pub fn output(&mut self, format: &str, path: &std::path::Path) {
        let bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        self.outputs.push(OutputFile {
            format: format.into(),
            path: path.display().to_string(),
            bytes,
        });
    }

    /// The report as pretty JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".into())
    }
}

/// The `format` of every output manifest.
pub const OUTPUT_MANIFEST_FORMAT: &str = "rvt-open-output";
/// The output manifest's schema version; additive fields keep it.
pub const OUTPUT_MANIFEST_SCHEMA_VERSION: u32 = 1;
/// The manifest's file name in an output folder.
pub const OUTPUT_MANIFEST_FILE: &str = "manifest.json";

/// One file of an output folder.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManifestOutput {
    /// `ifc`, `elements-jsonl`, `elements-xlsx`, `glb`, ...
    pub format: String,
    /// Its name in the folder.
    pub file: String,
    pub bytes: u64,
    /// SHA-256 of the bytes, lowercase hex.
    pub sha256: String,
}

impl ManifestOutput {
    /// The entry of `bytes`, written as `file`, an output of `format`.
    pub fn of(format: &str, file: &str, bytes: &[u8]) -> Self {
        use sha2::Digest;
        let digest = sha2::Sha256::digest(bytes);
        ManifestOutput {
            format: format.into(),
            file: file.into(),
            bytes: bytes.len() as u64,
            sha256: digest.iter().map(|b| format!("{b:02x}")).collect(),
        }
    }
}

/// The decode limits a conversion ran under; `null` is unlimited.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct DecodeLimits {
    pub max_file_bytes: Option<u64>,
    pub max_stream_bytes: Option<u64>,
    pub max_inflate_output_bytes: Option<u64>,
    pub max_inflate_total_bytes: Option<u64>,
    pub max_inflate_ratio: Option<u64>,
    pub max_scan_bytes: Option<u64>,
    pub max_candidates: Option<u64>,
    pub max_trial_offsets: Option<u64>,
    pub max_record_decode_bytes: Option<u64>,
    pub max_container_records: Option<u64>,
}

fn limit(v: u64) -> Option<u64> {
    (v != u64::MAX && v != usize::MAX as u64).then_some(v)
}

impl DecodeLimits {
    /// The limits of a file opened with `open` and walked with `walker`.
    pub fn new(open: &crate::reader::OpenLimits, walker: &crate::walker::WalkerLimits) -> Self {
        DecodeLimits {
            max_file_bytes: limit(open.max_file_bytes),
            max_stream_bytes: limit(open.max_stream_bytes),
            max_inflate_output_bytes: limit(open.inflate_limits.max_output_bytes as u64),
            max_inflate_total_bytes: limit(open.inflate_limits.max_total_output_bytes as u64),
            max_inflate_ratio: limit(open.inflate_limits.max_ratio),
            max_scan_bytes: limit(walker.max_scan_bytes as u64),
            max_candidates: limit(walker.max_candidates as u64),
            max_trial_offsets: limit(walker.max_trial_offsets as u64),
            max_record_decode_bytes: limit(walker.max_per_record_decode_bytes as u64),
            max_container_records: limit(walker.max_container_records as u64),
        }
    }
}

/// `manifest.json`, the envelope of an output folder: what wrote it, under
/// which decode limits, and every file it holds with its size and SHA-256.
/// It is written last, so a folder that has one is complete.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OutputManifest {
    /// Always [`OUTPUT_MANIFEST_FORMAT`].
    pub format: String,
    pub schema_version: u32,
    pub tool: String,
    pub tool_version: String,
    pub input: String,
    pub status: RunStatus,
    pub limits: DecodeLimits,
    pub outputs: Vec<ManifestOutput>,
    /// The run report's file name in the folder, where there is one.
    pub report: Option<String>,
}

impl OutputManifest {
    /// An empty manifest for `input`, written by `tool`.
    pub fn new(tool: &str, input: &str, status: RunStatus, limits: DecodeLimits) -> Self {
        OutputManifest {
            format: OUTPUT_MANIFEST_FORMAT.into(),
            schema_version: OUTPUT_MANIFEST_SCHEMA_VERSION,
            tool: tool.into(),
            tool_version: env!("CARGO_PKG_VERSION").into(),
            input: input.into(),
            status,
            limits,
            outputs: Vec::new(),
            report: None,
        }
    }

    /// Record `bytes`, written as `file`, as an output of `format`.
    pub fn add(&mut self, format: &str, file: &str, bytes: &[u8]) {
        self.outputs.push(ManifestOutput::of(format, file, bytes));
    }

    /// The manifest as pretty JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".into())
    }

    /// Read a manifest, refusing one that is not an rvt-open output manifest
    /// or whose schema is newer than this build reads.
    pub fn read(text: &str) -> Result<Self, String> {
        let value: serde_json::Value =
            serde_json::from_str(text).map_err(|e| format!("manifest: {e}"))?;
        if value.get("format").and_then(|f| f.as_str()) != Some(OUTPUT_MANIFEST_FORMAT) {
            return Err(format!("not an {OUTPUT_MANIFEST_FORMAT} manifest"));
        }
        match value.get("schema_version").and_then(|v| v.as_u64()) {
            Some(v) if v <= u64::from(OUTPUT_MANIFEST_SCHEMA_VERSION) => {}
            Some(v) => {
                return Err(format!(
                    "manifest schema {v} is newer than this build reads ({OUTPUT_MANIFEST_SCHEMA_VERSION})"
                ));
            }
            None => return Err("manifest has no schema_version".into()),
        }
        serde_json::from_value(value).map_err(|e| format!("manifest: {e}"))
    }
}

#[cfg(test)]
mod manifest_tests {
    use super::*;

    #[test]
    fn manifest_round_trips_and_refuses_what_it_cannot_read() {
        let limits = DecodeLimits::new(&Default::default(), &Default::default());
        let mut m = OutputManifest::new("rvt-batch", "in/a.rvt", RunStatus::Ok, limits);
        m.add("ifc", "a.ifc", b"ISO-10303-21;");
        assert_eq!(m.outputs[0].bytes, 13);
        assert_eq!(m.outputs[0].sha256.len(), 64);
        let json = m.to_json();
        assert_eq!(OutputManifest::read(&json).unwrap(), m);
        let newer = json.replace("\"schema_version\": 1", "\"schema_version\": 2");
        assert!(OutputManifest::read(&newer).unwrap_err().contains("newer"));
        assert!(OutputManifest::read("{\"format\": \"other\"}").is_err());
    }
}
