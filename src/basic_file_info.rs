//! Parse the `BasicFileInfo` OLE stream — UTF-16LE metadata every Revit file carries.
//!
//! Content is a loose structure of UTF-16LE strings interleaved with binary
//! markers. The fields we reliably extract across 2016-2026:
//!
//! - Version (4-digit year) — preceded by `\x04\x00` in the raw bytes
//! - Build string — either `(Build: YYYYMMDD_HHMM(x64))` or `YYYYMMDD_HHMM(x64)`
//! - Original local file path (embedded by the creator's filesystem)
//! - File GUID — standard UUID notation, sometimes repeated
//! - Locale — e.g. `ENU` appears as `E·N·U` in UTF-16LE
//!
//! This is the regex-driven "easy layer" of the format.
//!
//! # The `Key: value` text block
//!
//! After the binary header, every release 2016-2026 appends a CRLF-separated
//! block of `Key: value` lines — the same text Revit shows nowhere in its UI
//! but writes on every save. On the 11-release `phi-ag/rvt` family corpus and
//! both magnetar project files it carries, in order:
//!
//! ```text
//! Worksharing: Not enabled
//! Username:
//! Central Model Path:
//! Format: 2024                      (2019+; 2016-2018 write "Revit Build: Autodesk Revit 2016 (Build: …)")
//! Build: 20230509_0315(x64)
//! Last Save Path: B:\…\2024_Core_Interior.rvt
//! Open Workset Default: 3
//! Project Spark File: 0
//! Central Model Identity: 00000000-0000-0000-0000-000000000000
//! Locale when saved: ENU
//! All Local Changes Saved To Central: 0
//! Central model's version number corresponding to the last reload latest: 66
//! Central model's episode GUID corresponding to the last reload latest: 2a619b2e-…
//! Unique Document GUID: 2a619b2e-…
//! Unique Document Increments: 66
//! Model Identity: 00000000-0000-0000-0000-000000000000
//! IsSingleUserCloudModel: False      (2019+)
//! Author: Autodesk Revit             (2019+)
//! ClientAppName: RevitApplication    (2021+)
//! ```
//!
//! Two quirks make a plain UTF-16LE decode miss it:
//!
//! 1. The binary header before the block contains 1-byte fields, so the
//!    block starts at an **odd** byte offset on some files (2026, both
//!    project files) and an even one on others. [`parse_properties`] decodes
//!    both alignments and keeps the one that yields the known keys.
//! 2. `IsSingleUserCloudModel` is written as a *narrow* C string packed into
//!    the wide text (`"False\0"` reads back as `慆獬e`), and a 5-byte
//!    `"True\0"` would flip the alignment of everything after it. Values that
//!    unpack to `True` / `False` are restored; lines after a flip are
//!    recovered from the other alignment.
//!
//! [`BasicFileInfo::properties`] keeps every line verbatim; typed accessors
//! such as [`BasicFileInfo::worksharing`] and [`BasicFileInfo::username`]
//! read from it and return `None` for an absent or empty value.

use crate::le_bytes;
use crate::{Error, Result};
use encoding_rs::UTF_16LE;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BasicFileInfo {
    /// 4-digit Revit release year (e.g. 2024).
    pub version: u32,
    /// Build tag — either `YYYYMMDD_HHMM(x64)` or a free-form string like
    /// `"Development Build"`.
    pub build: Option<String>,
    /// Original file path recorded at save time on the creator's system.
    pub original_path: Option<String>,
    /// File GUID (UUIDv4) if recoverable.
    pub guid: Option<String>,
    /// Locale code if present (e.g. `ENU`, `FRA`).
    pub locale: Option<String>,
    /// Every `Key: value` line of the trailing text block, in file order,
    /// values verbatim (see the module docs). Empty when the stream carries
    /// no such block.
    #[serde(default)]
    pub properties: Vec<FileProperty>,
    /// Raw UTF-16LE decoded text for debugging.
    pub raw_text: String,
}

/// One `Key: value` line from the `BasicFileInfo` text block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileProperty {
    pub key: String,
    pub value: String,
}

/// Keys observed on every release of the reference corpora. Used to pick
/// the byte alignment the text block was written at; unknown keys are still
/// kept in [`BasicFileInfo::properties`].
const KNOWN_KEYS: &[&str] = &[
    "Worksharing",
    "Username",
    "Central Model Path",
    "Format",
    "Build",
    "Revit Build",
    "Last Save Path",
    "Open Workset Default",
    "Project Spark File",
    "Central Model Identity",
    "Locale when saved",
    "All Local Changes Saved To Central",
    "Central model's version number corresponding to the last reload latest",
    "Central model's episode GUID corresponding to the last reload latest",
    "Unique Document GUID",
    "Unique Document Increments",
    "Model Identity",
    "IsSingleUserCloudModel",
    "Author",
    "ClientAppName",
];

impl BasicFileInfo {
    /// Parse the raw `BasicFileInfo` stream bytes.
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        let (cow, _, had_errors) = UTF_16LE.decode(data);
        if had_errors {
            // Not fatal — some single-byte markers aren't valid UTF-16 pairs.
            // We still extract from whatever decoded cleanly.
        }
        let raw = cow.into_owned();
        let properties = parse_properties(data);
        let prop = |key: &str| lookup(&properties, key);

        let build = extract_build(&raw)
            .or_else(|| prop("Build").map(str::to_string))
            .or_else(|| prop("Revit Build").map(str::to_string));
        let version = year_before_build(&raw)
            .or_else(|| prop("Format").and_then(parse_release_year))
            .or_else(|| extract_version(&raw))
            .or_else(|| build.as_deref().and_then(release_from_build))
            .ok_or_else(|| Error::BasicFileInfo("no 4-digit Revit version found".into()))?;
        let original_path =
            extract_path(&raw).or_else(|| prop("Last Save Path").map(str::to_string));
        let guid = prop("Unique Document GUID")
            .filter(|g| is_guid(g))
            .map(str::to_string)
            .or_else(|| extract_guid(&raw));
        let locale = prop("Locale when saved")
            .map(str::to_string)
            .or_else(|| extract_locale(&raw));

        Ok(Self {
            version,
            build,
            original_path,
            guid,
            locale,
            properties,
            raw_text: raw,
        })
    }

    /// Value of the text-block line `key` (exact, case-sensitive match),
    /// or `None` when the line is absent or its value is empty.
    pub fn property(&self, key: &str) -> Option<&str> {
        lookup(&self.properties, key)
    }

    /// The `Worksharing` line verbatim: `Not enabled` on a file without
    /// worksharing; other values (`Central`, `Local`, …) name the role the
    /// saved copy had in a workshared project.
    pub fn worksharing(&self) -> Option<&str> {
        self.property("Worksharing")
    }

    /// `Some(false)` when `Worksharing` reads `Not enabled`, `Some(true)`
    /// for any other recorded value, `None` when the line is missing.
    pub fn is_workshared(&self) -> Option<bool> {
        self.worksharing()
            .map(|w| !w.eq_ignore_ascii_case("Not enabled"))
    }

    /// Windows / Autodesk account name recorded by the last save
    /// (empty on non-workshared files, which yields `None`).
    pub fn username(&self) -> Option<&str> {
        self.property("Username")
    }

    /// Path of the central model a workshared file syncs with — a UNC or
    /// drive path, an `RSN://` Revit Server path, or a cloud path.
    pub fn central_model_path(&self) -> Option<&str> {
        self.property("Central Model Path")
    }

    /// Full path the file was last saved to.
    pub fn last_save_path(&self) -> Option<&str> {
        self.property("Last Save Path")
    }

    /// `Unique Document GUID`: the document's identity, stable across saves.
    pub fn document_guid(&self) -> Option<&str> {
        self.property("Unique Document GUID")
    }

    /// `Unique Document Increments`: Revit's per-document counter. It grows
    /// as the document is saved (59 → 70 across the eleven yearly re-saves
    /// of the reference sample family); the exact increment rule is not
    /// documented.
    pub fn document_increments(&self) -> Option<u32> {
        self.property("Unique Document Increments")
            .and_then(|v| v.trim().parse().ok())
    }

    /// `IsSingleUserCloudModel` (Revit 2019+).
    pub fn is_single_user_cloud_model(&self) -> Option<bool> {
        self.property("IsSingleUserCloudModel").and_then(parse_bool)
    }

    /// `All Local Changes Saved To Central` (`0` / `1` on disk).
    pub fn all_local_changes_saved_to_central(&self) -> Option<bool> {
        self.property("All Local Changes Saved To Central")
            .and_then(parse_bool)
    }

    /// `Open Workset Default`: the worksets-to-open choice stored with the file.
    pub fn open_workset_default(&self) -> Option<u32> {
        self.property("Open Workset Default")
            .and_then(|v| v.trim().parse().ok())
    }

    /// `Author` (Revit 2019+; `Autodesk Revit` on every observed file).
    pub fn author(&self) -> Option<&str> {
        self.property("Author")
    }

    /// `ClientAppName` (Revit 2021+; `RevitApplication` on every observed file).
    pub fn client_app_name(&self) -> Option<&str> {
        self.property("ClientAppName")
    }
}

fn lookup<'a>(properties: &'a [FileProperty], key: &str) -> Option<&'a str> {
    properties
        .iter()
        .find(|p| p.key == key)
        .map(|p| p.value.as_str())
        .filter(|v| !v.is_empty())
}

/// Recover the `Key: value` text block (see the module docs).
///
/// Decodes the stream at both byte alignments. The alignment that yields
/// more of the keys observed on the reference corpora supplies the lines in
/// file order; known keys only the other alignment decodes (the tail after a
/// narrow `True\0` flip) are appended.
pub fn parse_properties(data: &[u8]) -> Vec<FileProperty> {
    let decode = |align: usize| -> Vec<FileProperty> {
        let Some(body) = data.get(align..) else {
            return Vec::new();
        };
        let units = le_bytes::u16_units(body);
        let text: String = char::decode_utf16(units)
            .map(|unit| unit.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect();
        text.split(['\r', '\n', '\u{0A0D}'])
            .filter_map(parse_line)
            .collect()
    };
    let score = |props: &[FileProperty]| {
        props
            .iter()
            .filter(|p| KNOWN_KEYS.contains(&p.key.as_str()))
            .count()
    };
    let even = decode(0);
    let odd = decode(1);
    let (mut primary, secondary) = if score(&odd) > score(&even) {
        (odd, even)
    } else {
        (even, odd)
    };
    if score(&primary) == 0 {
        return Vec::new();
    }
    for prop in secondary {
        if KNOWN_KEYS.contains(&prop.key.as_str()) && !primary.iter().any(|p| p.key == prop.key) {
            primary.push(prop);
        }
    }
    primary
}

fn parse_line(line: &str) -> Option<FileProperty> {
    let line = line.trim_matches(is_padding);
    let colon = line.find(':')?;
    let key = line[..colon].trim();
    if !is_label(key) {
        return None;
    }
    let rest = &line[colon + 1..];
    let value = if rest.is_empty() {
        ""
    } else {
        // `Key: value` — a colon glued to a non-space is a drive letter or a
        // URL scheme inside some other string, not a label.
        rest.strip_prefix(' ')?
    };
    Some(FileProperty {
        key: key.to_string(),
        value: clean_value(value),
    })
}

fn is_padding(c: char) -> bool {
    c == '\0' || c == char::REPLACEMENT_CHARACTER || c.is_whitespace()
}

fn is_label(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 96
        && key.starts_with(|c: char| c.is_ascii_alphabetic())
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '\'' | '-' | '_'))
}

fn clean_value(value: &str) -> String {
    let value = value.trim_matches(is_padding);
    unpack_narrow_bool(value).unwrap_or_else(|| value.to_string())
}

/// Undo Revit writing a narrow `"True\0"` / `"False\0"` into the wide text:
/// each UTF-16 unit then carries two ASCII bytes, read up to the NUL.
fn unpack_narrow_bool(value: &str) -> Option<String> {
    if value.is_ascii() {
        return None;
    }
    let mut narrow = Vec::new();
    'units: for unit in value.encode_utf16() {
        for byte in unit.to_le_bytes() {
            if byte == 0 {
                break 'units;
            }
            narrow.push(byte);
        }
    }
    let narrow = String::from_utf8(narrow).ok()?;
    parse_bool(&narrow).map(|b| if b { "True" } else { "False" }.to_string())
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim() {
        v if v.eq_ignore_ascii_case("true") || v == "1" => Some(true),
        v if v.eq_ignore_ascii_case("false") || v == "0" => Some(false),
        _ => None,
    }
}

/// Plausible Revit release years. The lower bound is Revit 2008, the oldest
/// Paul Aubin's Revit Architecture 2010 file); the upper bound leaves room for
/// releases newer than the reference corpus (Revit 2027 shipped in 2026)
/// without accepting arbitrary 4-digit numbers.
const RELEASE_YEARS: std::ops::RangeInclusive<u32> = 2008..=2040;

fn parse_release_year(value: &str) -> Option<u32> {
    value
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|y| RELEASE_YEARS.contains(y))
}

/// The release named in the product line: the year right before
/// `(Build`, as in `Autodesk Revit Architecture 2012 (Build: 20110916_2132)`.
/// Preferred over the first year in the text, which on a workshared file can
/// come from the central model's path (a Revit 2012 file whose
/// central path names 2011 read as 2011; its schema is 2012's).
fn year_before_build(text: &str) -> Option<u32> {
    text.match_indices("(Build").find_map(|(at, _)| {
        let head = text[..at].trim_end();
        let start = head.len().checked_sub(4)?;
        let year = head.get(start..)?;
        let standalone = head[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_ascii_digit());
        (standalone && year.bytes().all(|b| b.is_ascii_digit()))
            .then(|| parse_release_year(year))
            .flatten()
    })
}

fn extract_version(text: &str) -> Option<u32> {
    // Patterns seen:
    //   "Autodesk Revit 2018 (Build: 20170130_1515(x64))"   <- 2016-2018
    //   "2019  20180123_1515(x64)"                          <- 2019+
    //   "Autodesk Revit Architecture 2010 (Build: 20090612_2115)" <- 2010-2015
    // Strategy: the first run of exactly 4 digits in RELEASE_YEARS. A
    // longer run is a build date: "20080321_1900" must not read as 2008
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i - start == 4 {
            if let Some(n) = parse_release_year(&text[start..i]) {
                return Some(n);
            }
        }
    }
    None
}

/// The release of a file that states none, from its build date.
/// Revit 2008 and 2009 write only `Revit Build:
/// 20070324_1700`; every file that states both was built in the year
/// before its release, so the release is the build year plus one.
fn release_from_build(build: &str) -> Option<u32> {
    let date = build
        .get(..8)
        .filter(|d| d.bytes().all(|b| b.is_ascii_digit()))?;
    if build.as_bytes().get(8) != Some(&b'_') {
        return None;
    }
    let year: u32 = date[..4].parse().ok()?;
    Some(year + 1).filter(|y| RELEASE_YEARS.contains(y))
}

fn extract_build(text: &str) -> Option<String> {
    // Pattern 1: "(Build: 20170130_1515(x64))" — most common wrapper
    // on files saved by Revit 2016–2018.
    if let Some(p) = text.find("Build: ") {
        let tail = &text[p + 7..];
        if let Some(end) = tail.find(')') {
            return Some(tail[..end + 1].to_string());
        }
    }
    // Pattern 2: plain "YYYYMMDD_HHMM(x64)" format — Revit 2019+ and
    // later releases embed the build tag without the "Build:" wrapper.
    // The HHMM component varies (1515, 1635, 1200, …); we scan for
    // the full YYYYMMDD_HHMM(x64) shape directly rather than literal
    // substring matches so all build tags survive this path.
    //
    // Wire format: 8 digits, '_', 4 digits, literal "(x64)".
    // Total length: 18 chars.
    let suffix = "(x64)";
    let bytes = text.as_bytes();
    let n = bytes.len();
    let tag_len = 8 + 1 + 4 + suffix.len();
    if n >= tag_len {
        for i in 0..=n - tag_len {
            let window = &bytes[i..i + tag_len];
            let ymd = &window[0..8];
            let us = window[8];
            let hm = &window[9..13];
            let sfx = &window[13..];
            let ok = ymd.iter().all(u8::is_ascii_digit)
                && us == b'_'
                && hm.iter().all(u8::is_ascii_digit)
                && sfx == suffix.as_bytes();
            if ok {
                return Some(String::from_utf8_lossy(window).into_owned());
            }
        }
    }
    // Pattern 3: "Development Build" — Revit dev releases ship
    // without a build tag; callers want SOME string.
    if text.contains("Development Build") {
        return Some("Development Build".to_string());
    }
    None
}

fn extract_path(text: &str) -> Option<String> {
    // Windows paths like C:\Users\...\...rfa
    // UTF-16LE-decoded text has these as plain chars.
    let needle_start = text.find(['C', 'D']);
    let start = needle_start?;
    let tail = &text[start..];
    // Must be "C:\" or "D:\" pattern
    if !tail.starts_with("C:\\") && !tail.starts_with("D:\\") {
        // fallback: first occurrence of ":\\" anywhere
        let colon_backslash = text.find(":\\")?;
        // `colon_backslash` points to the ':' byte. Back up one byte
        // to include the drive letter, but land on a char boundary —
        // the saturating_sub can point into the middle of a multi-byte
        // UTF-8 character when text before ":\\" is non-ASCII (caught
        // by libFuzzer fuzz_basic_file_info on input
        // containing UTF-8 BOMs + Arabic bytes before a ":\\" literal).
        let s = colon_backslash.saturating_sub(1);
        let tail = text.get(s..)?;
        return take_until_rfa(tail);
    }
    take_until_rfa(tail)
}

fn take_until_rfa(tail: &str) -> Option<String> {
    for ext in &[".rvt", ".rfa", ".rte", ".rft"] {
        if let Some(end) = tail.find(ext) {
            return Some(tail[..end + ext.len()].to_string());
        }
    }
    None
}

fn extract_guid(text: &str) -> Option<String> {
    // UUIDv4: 8-4-4-4-12 hex chars
    let bytes = text.as_bytes();
    for i in 0..bytes.len().saturating_sub(36) {
        let slice = &text.get(i..i + 36)?;
        if is_guid(slice) {
            return Some(slice.to_string());
        }
    }
    None
}

fn is_guid(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    let dash_positions = [8, 13, 18, 23];
    for (i, b) in bytes.iter().enumerate() {
        if dash_positions.contains(&i) {
            if *b != b'-' {
                return false;
            }
        } else if !b.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

fn extract_locale(text: &str) -> Option<String> {
    // Locale codes appear as 3-letter ASCII blocks like ENU, FRA, DEU, ESP, RUS, JPN, CHS
    for code in &[
        "ENU", "FRA", "DEU", "ESP", "ITA", "RUS", "JPN", "CHS", "CHT", "KOR", "PLK", "PTB", "CSY",
    ] {
        if text.contains(*code) {
            return Some(code.to_string());
        }
    }
    None
}

#[cfg(test)]
mod release_year_tests {
    use super::{extract_version, release_from_build, year_before_build};

    #[test]
    fn a_build_date_is_not_a_release_year() {
        assert_eq!(
            extract_version("Autodesk Revit Architecture 2010 (Build: 20090612_2115)"),
            Some(2010)
        );
        assert_eq!(extract_version("2019  20180123_1515(x64)"), Some(2019));
        assert_eq!(
            year_before_build(
                "C:\\Data_2011_Worksharing\\x.rvt Autodesk Revit Architecture 2012 (Build: 20110916_2132(x64))"
            ),
            Some(2012)
        );
        assert_eq!(year_before_build("Revit Build: 20070324_1700"), None);
        assert_eq!(extract_version("Revit Build: 20080321_1900"), None);
        assert_eq!(release_from_build("20070324_1700"), Some(2008));
        assert_eq!(release_from_build("2007032_1700"), None);
    }
}
