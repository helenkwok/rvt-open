//! Behaviour shared by the shipped command-line tools.

/// Make a CLI exit quietly when its standard output is closed early —
/// `rvt-schema model.rvt | head` — instead of panicking with
/// `failed printing to stdout: Broken pipe` and a backtrace hint.
///
/// Rust ignores `SIGPIPE`, so `println!` into a closed pipe panics. This
/// installs a panic hook that turns exactly that panic into exit status 141
/// (what a shell reports for a process killed by `SIGPIPE`) and leaves every
/// other panic to the default report. Call it first thing in `main`.
pub fn exit_quietly_on_broken_pipe() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info.payload();
        let message = payload
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| payload.downcast_ref::<&str>().copied())
            .unwrap_or_default();
        if is_closed_stdout_pipe(message) {
            std::process::exit(141);
        }
        default_hook(info);
    }));
}

/// `println!`'s panic message for a write into a closed pipe: `EPIPE`
/// (`os error 32`) on Unix; on Windows `ERROR_BROKEN_PIPE` (`os error 109`,
/// "The pipe has been ended" — what the CI runner reports) or
/// `ERROR_NO_DATA` (`os error 232`, "The pipe is being closed"). Other stdout
/// failures (a full disk behind a redirect) are not swallowed.
fn is_closed_stdout_pipe(message: &str) -> bool {
    message.starts_with("failed printing to stdout")
        && ["os error 32)", "os error 109)", "os error 232)"]
            .iter()
            .any(|code| message.contains(code))
}

/// The `--large-model` preset's per-stream cap: the whole file's cap
/// ([`crate::reader::DEFAULT_MAX_FILE_BYTES`]). A stream is never larger
/// than its file, which is already read into memory, so this adds no
/// allocation an attacker could not force with the file itself.
pub const LARGE_MODEL_MAX_STREAM_BYTES: u64 = crate::reader::DEFAULT_MAX_FILE_BYTES;

/// The `--large-model` preset's inflate cap, per gzip member and per stream
/// in total (`crate::compression::InflateLimits::max_stream_total_bytes`):
/// 8 GiB, or the address space on a 32-bit target. Project models with a
/// 1 GiB stored partition inflate past 1 GiB.
pub const LARGE_MODEL_MAX_INFLATE_BYTES: usize = if usize::BITS >= 64 {
    (8u64 << 30) as usize
} else {
    usize::MAX
};

/// The [`crate::reader::OpenLimits`] a CLI opens its input with: the
/// defaults, the `--large-model` preset over them, and an explicit
/// `--max-stream-bytes` / `--max-inflate-bytes` over both.
pub fn open_limits(
    max_stream_bytes: Option<u64>,
    max_inflate_bytes: Option<usize>,
    large_model: bool,
) -> crate::reader::OpenLimits {
    let mut limits = crate::reader::OpenLimits::default();
    if large_model {
        limits.max_stream_bytes = LARGE_MODEL_MAX_STREAM_BYTES;
        limits.inflate_limits.max_output_bytes = LARGE_MODEL_MAX_INFLATE_BYTES;
    }
    if let Some(bytes) = max_stream_bytes {
        limits.max_stream_bytes = bytes;
    }
    if let Some(bytes) = max_inflate_bytes {
        limits.inflate_limits.max_output_bytes = bytes;
    }
    limits
}

/// The lines a CLI prints when a resource limit kept streams from being read
/// whole, one per stream, then how to go on.
pub fn unread_stream_report(unread: &[crate::reader::UnreadStream]) -> Vec<String> {
    let mut lines: Vec<String> = unread.iter().map(|stream| format!("  {stream}")).collect();
    lines.push(
        "  Re-run with --large-model (or a larger --max-stream-bytes / --max-inflate-bytes) to read them, or --allow-partial to write the incomplete output anyway."
            .to_string(),
    );
    lines
}

/// With `RVT_STAGE_MEMORY` set, print one line to stderr as a conversion
/// leaves each phase ([`crate::control::stage`]): the phase, its seconds,
/// the resident memory at its end and the peak resident memory reached
/// during it (MiB). Linux only (`/proc/self`); elsewhere, and without the
/// variable, it does nothing. The peak is reset at each phase through
/// `/proc/self/clear_refs`, so each line holds that phase's own peak.
/// Call [`crate::control::stage`] with `"done"` at the end to close the last phase.
pub fn log_stage_memory() {
    if std::env::var_os("RVT_STAGE_MEMORY").is_none() || status_kib("VmRSS").is_none() {
        return;
    }
    let state = std::cell::RefCell::new(("open".to_string(), std::time::Instant::now()));
    reset_peak();
    crate::control::set_stage_hook(Some(Box::new(move |label: &str| {
        let mut s = state.borrow_mut();
        let rss = status_kib("VmRSS").unwrap_or(0) / 1024;
        let peak = status_kib("VmHWM").unwrap_or(0) / 1024;
        eprintln!(
            "stage-memory\t{}\t{:.1}\t{rss}\t{peak}",
            s.0,
            s.1.elapsed().as_secs_f64()
        );
        reset_peak();
        *s = (label.to_string(), std::time::Instant::now());
    })));
}

fn status_kib(key: &str) -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status.lines().find_map(|line| {
        let rest = line.strip_prefix(key)?.strip_prefix(':')?;
        rest.trim().trim_end_matches("kB").trim().parse().ok()
    })
}

fn reset_peak() {
    // "5" resets the peak resident set size (Linux 4.0 and later).
    let _ = std::fs::write("/proc/self/clear_refs", "5");
}

/// The IFC options `rvt-ifc` and `rvt-batch` share
/// ([`crate::ifc::ifc_options`]), and the phase every output shows.
#[derive(Debug, Clone, clap::Args)]
pub struct IfcOutputArgs {
    /// How much the IFC file holds: `full` (everything), `coordination`
    /// (geometry, identity and the IFC common `Pset_` sets; Revit's own
    /// parameter sets and the base quantities left out, as Revit's
    /// coordination setups) or `geometry` (geometry and identity only). Both
    /// keep each element's `Revit` set (ElementId and category).
    #[arg(long, value_enum, default_value = "full")]
    pub ifc_profile: IfcProfileArg,
    /// The IFC file's length unit. `m` and `mm` keep areas in m2 and volumes
    /// in m3; `ft` and `in` write them in square and cubic feet or inches.
    #[arg(long, value_enum, default_value = "m")]
    pub ifc_units: IfcUnitsArg,
    /// The IFC file's origin: Revit's internal origin, the project base
    /// point (axes unchanged) or the survey point (axes turned to true
    /// north), as Revit's export coordinate bases. Fails when the file's
    /// point was not read.
    #[arg(long, value_enum, default_value = "internal")]
    pub ifc_origin: IfcOriginArg,
    /// Keep only these elements in the IFC file: Revit category names
    /// (`Walls`, `Doors`) or IFC classes (`IfcWall`), comma separated, case
    /// insensitive. Openings, ports and parts go with their element.
    #[arg(long, value_name = "LIST", value_delimiter = ',')]
    pub ifc_categories: Vec<String>,
    /// Show the model as it is in this phase (Revit's "Phase to export"), by
    /// its name or its ElementId: elements created in it or before and not
    /// demolished in it or before, and rooms whose own phase is it.
    /// Default: the project's last phase. Applies to every output.
    #[arg(long, value_name = "NAME|ID")]
    pub phase: Option<String>,
}

/// `--ifc-profile`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum IfcProfileArg {
    Full,
    Coordination,
    Geometry,
}

/// `--ifc-units`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum IfcUnitsArg {
    M,
    Mm,
    Ft,
    In,
}

/// `--ifc-origin`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
#[value(rename_all = "kebab-case")]
pub enum IfcOriginArg {
    Internal,
    ProjectBase,
    SurveyPoint,
}

impl IfcOutputArgs {
    /// The options [`crate::ifc::ifc_options::apply`] takes.
    pub fn options(&self) -> crate::ifc::ifc_options::IfcOptions {
        use crate::ifc::ifc_options::{IfcLengthUnit, IfcOrigin, IfcProfile};
        crate::ifc::ifc_options::IfcOptions {
            profile: match self.ifc_profile {
                IfcProfileArg::Full => IfcProfile::Full,
                IfcProfileArg::Coordination => IfcProfile::Coordination,
                IfcProfileArg::Geometry => IfcProfile::Geometry,
            },
            units: match self.ifc_units {
                IfcUnitsArg::M => IfcLengthUnit::Metre,
                IfcUnitsArg::Mm => IfcLengthUnit::Millimetre,
                IfcUnitsArg::Ft => IfcLengthUnit::Foot,
                IfcUnitsArg::In => IfcLengthUnit::Inch,
            },
            origin: match self.ifc_origin {
                IfcOriginArg::Internal => IfcOrigin::Internal,
                IfcOriginArg::ProjectBase => IfcOrigin::ProjectBase,
                IfcOriginArg::SurveyPoint => IfcOrigin::SurveyPoint,
            },
            categories: self
                .ifc_categories
                .iter()
                .map(|c| c.trim().to_string())
                .filter(|c| !c.is_empty())
                .collect(),
        }
    }

    /// Set `--phase` on `rf`: a phase's name, or its ElementId. An unknown
    /// phase is an error that lists the file's phases.
    pub fn set_phase(&self, rf: &mut crate::RevitFile) -> anyhow::Result<()> {
        let Some(name) = &self.phase else {
            return Ok(());
        };
        rf.set_export_phase(name.trim())
            .map_err(|e| anyhow::anyhow!("--phase {name:?}: {e}"))
    }
}

/// Write an output file so that a crash never leaves it half written: the
/// bytes go to a temporary file beside `path`, are flushed to disk and then
/// renamed over `path` (the vendored `craft-atomic`). Same arguments and
/// result as [`std::fs::write`]. A `path` that exists and is not a regular
/// file (`/dev/null`, a pipe) is written to directly.
#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
pub fn write_output(
    path: impl AsRef<std::path::Path>,
    bytes: impl AsRef<[u8]>,
) -> std::io::Result<()> {
    let path = path.as_ref();
    match std::fs::metadata(path) {
        Ok(meta) if !meta.is_file() => std::fs::write(path, bytes),
        _ => craft_atomic::atomic_write(path, bytes.as_ref()),
    }
}

/// [`write_output`] with the bytes written by `fill` as it makes them, through
/// a buffer, instead of held whole first. Returns what `fill` returned and
/// the number of bytes written; `fill`'s error leaves no output behind where
/// [`write_output`] would have replaced the file.
#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
pub fn write_output_with<T, E: std::error::Error + Send + Sync + 'static>(
    path: impl AsRef<std::path::Path>,
    fill: impl FnOnce(&mut dyn std::io::Write) -> Result<T, E>,
) -> std::io::Result<(u64, T)> {
    let path = path.as_ref();
    let mut result = None;
    let write = |file: &mut std::fs::File| -> std::io::Result<()> {
        let mut out = Counted {
            inner: std::io::BufWriter::new(file),
            written: 0,
        };
        let value = fill(&mut out).map_err(std::io::Error::other)?;
        std::io::Write::flush(&mut out)?;
        result = Some((out.written, value));
        Ok(())
    };
    match std::fs::metadata(path) {
        Ok(meta) if !meta.is_file() => write(&mut std::fs::File::create(path)?)?,
        _ => craft_atomic::atomic_write_with(path, write)?,
    }
    Ok(result.expect("written"))
}

/// A writer that counts the bytes it passes on.
#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
struct Counted<W> {
    inner: W,
    written: u64,
}

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
impl<W: std::io::Write> std::io::Write for Counted<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.written += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// The warning lines a CLI prints when readers stopped at a work budget,
/// one per reader with how often, then what that means
/// ([`crate::ifc::ExportDiagnostics::work_budget_stops`]). The output is
/// written all the same; no option raises a work budget.
pub fn work_budget_report(stops: &std::collections::BTreeMap<String, usize>) -> Vec<String> {
    let mut lines: Vec<String> = stops
        .iter()
        .map(|(reader, times)| format!("  {reader} stopped {times} time(s)"))
        .collect();
    lines.push(
        "  The output is written without it. No option raises a work budget: only a file built to make a reader do unbounded work is known to reach one."
            .to_string(),
    );
    lines
}

/// Whether `a` and `b` are one file: the same path once links, `.` and `..`
/// are resolved, or two names of one file (a hard link on Unix). A path that
/// does not exist is no existing file's, so an output not yet written is never
/// the input. A CLI that writes beside its input checks this before it writes,
/// since comparing the paths as typed lets `./model.rvt` and a symlink through.
pub fn same_file(a: &std::path::Path, b: &std::path::Path) -> bool {
    if matches!(
        (std::fs::canonicalize(a), std::fs::canonicalize(b)),
        (Ok(x), Ok(y)) if x == y
    ) {
        return true;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let (Ok(a), Ok(b)) = (std::fs::metadata(a), std::fs::metadata(b)) {
            return a.dev() == b.dev() && a.ino() == b.ino();
        }
    }
    false
}

/// A text cell of a CSV a spreadsheet may open: quoted when it holds a comma, a
/// quote or a line break (RFC 4180), and led by `'` when it starts with `=`,
/// `+`, `-`, `@`, a tab or a carriage return, which a spreadsheet would read as
/// a formula. The text of a crafted file (a title, a user name, a path) reaches
/// such a cell, so a spreadsheet must read it as the text it is. Numbers are
/// written with `to_string`, not through this.
pub fn csv_text(value: &str) -> String {
    let value = if value.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        std::borrow::Cow::Owned(format!("'{value}"))
    } else {
        std::borrow::Cow::Borrowed(value)
    };
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::csv_text;

    #[test]
    fn a_formula_is_led_by_a_quote() {
        for lead in ["=", "+", "-", "@", "\t"] {
            let cell = format!("{lead}1+1");
            assert_eq!(csv_text(&cell), format!("'{cell}"), "led by {lead:?}");
        }
    }

    #[test]
    fn a_leading_carriage_return_is_led_and_then_quoted() {
        assert_eq!(csv_text("\r=1"), "\"'\r=1\"");
    }

    #[test]
    fn a_neutralised_cell_is_still_quoted_for_its_commas_and_quotes() {
        assert_eq!(
            csv_text("=HYPERLINK(\"x\",\"y\")"),
            "\"'=HYPERLINK(\"\"x\"\",\"\"y\"\")\""
        );
    }

    #[test]
    fn text_that_is_not_a_formula_is_left_as_it_is() {
        for plain in ["", "title", "a=b", "x-1", "'=1", "12"] {
            assert_eq!(csv_text(plain), plain);
        }
    }

    #[test]
    fn commas_quotes_and_line_breaks_are_quoted() {
        assert_eq!(csv_text("a,b"), "\"a,b\"");
        assert_eq!(csv_text("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_text("a\nb"), "\"a\nb\"");
    }
}
