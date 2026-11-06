//! Truncated-gzip decompression for Revit streams.
//!
//! Revit writes standard gzip file headers (magic `1F 8B 08` + 10-byte
//! minimum header) but omits the trailing 8-byte CRC32 + ISIZE that
//! RFC 1952 requires. `flate2::read::GzDecoder` validates those trailing
//! bytes and refuses truncated streams, so we skip the gzip header manually
//! and pump the raw DEFLATE body through `flate2::read::DeflateDecoder`.

use crate::le_bytes;
use crate::{Error, Result};
use flate2::read::DeflateDecoder;
use std::io::Read;
use std::ops::Range;

pub const GZIP_MAGIC: [u8; 3] = [0x1F, 0x8B, 0x08];

/// Default decompression-ratio guard: a gzip member may inflate to at most
/// this many times the compressed bytes it has consumed.
///
/// There is no fixed cap on how large a legitimate stream may inflate:
/// Revit reads multi-GB models, and so does this reader. What a
/// decompression bomb has that real data does not is its ratio. DEFLATE
/// corpus member that reaches [`RATIO_GUARD_FLOOR_BYTES`] exceeds 5.7:1
/// (101 models); small members
/// reach 217.7:1 but never the floor. A 200 MiB zero-fill member from
/// this crate's encoder is ~220:1, so the guard must sit between: 100:1 is
/// 17x the largest real value and refuses that member.
pub const DEFAULT_MAX_INFLATE_RATIO: u64 = 100;

/// Output a member may reach before the ratio guard applies, so a small
/// member of zeros (legitimately extreme ratio, negligible size) passes.
pub const RATIO_GUARD_FLOOR_BYTES: usize = 32 * 1024 * 1024;

/// Limit code: a gzip member would inflate past [`InflateLimits::max_output_bytes`].
pub const INFLATE_MAX_MEMBER_BYTES: &str = "inflate-max-member-bytes";
/// Limit code: a stream's members past [`InflateLimits::max_total_output_bytes`].
pub const INFLATE_MAX_STREAM_BYTES: &str = "inflate-max-stream-bytes";
/// Limit code: a gzip member inflates past [`InflateLimits::max_ratio`].
pub const INFLATE_RATIO: &str = "inflate-ratio";
/// Limit code: not enough memory to go on inflating ([`crate::memory`]).
pub const INFLATE_MEMORY: &str = "inflate-memory";

/// Diagnostic code: stored bytes that hold a gzip member which did not
/// inflate (corrupt or truncated DEFLATE) and were skipped.
pub const SKIPPED_MEMBER: &str = "skipped-member";

/// Stored regions of a stream whose gzip member did not inflate.
///
/// A gzip magic that fails to inflate is either a false hit inside another
/// member's compressed bytes, or a real member that is corrupt or truncated.
/// Members follow one another, so a failed magic *inside* the bytes the
/// previous good member consumed is a false hit and says nothing; a failed
/// magic at or past the end of the previous good member means the bytes from
/// that end to the next good member (or the stream end) were not decoded.
/// Each such run is one skipped region, `(start, end)` in the prepared stream.
#[derive(Debug, Default)]
struct SkipTracker {
    prev_end: usize,
    gap_open: bool,
    regions: Vec<(usize, usize)>,
}

impl SkipTracker {
    fn inflated(&mut self, offset: usize, consumed: usize) {
        if self.gap_open {
            self.regions.push((self.prev_end, offset));
            self.gap_open = false;
        }
        self.prev_end = self.prev_end.max(offset + consumed);
    }

    /// A member over an opt-in size cap: reported as a limit hit, and its
    /// length is unknown, so nothing after it is judged skipped.
    fn capped(&mut self, offset: usize) {
        self.inflated(offset, usize::MAX - offset);
    }

    fn failed(&mut self, offset: usize) {
        if offset >= self.prev_end {
            self.gap_open = true;
        }
    }

    fn finish(mut self, len: usize) -> Vec<(usize, usize)> {
        if self.gap_open {
            self.regions.push((self.prev_end, len));
        }
        self.regions
    }
}

/// The `skipped-member` diagnostic for `regions` of `stream_name`, or `None`
/// when nothing was skipped. The same text whichever reader walked the
/// stream, so a set of events holds it once.
pub fn skipped_member_event(stream_name: &str, regions: &[(usize, usize)]) -> Option<String> {
    if regions.is_empty() {
        return None;
    }
    let bytes: usize = regions.iter().map(|(a, b)| b - a).sum();
    let at: Vec<String> = regions.iter().map(|(a, b)| format!("{a}..{b}")).collect();
    Some(format!(
        "{SKIPPED_MEMBER}: {stream_name}: {} region(s), {bytes} stored bytes, did not inflate \
         and were skipped (at {})",
        regions.len(),
        at.join(", ")
    ))
}

/// Default maximum inflated bytes of one multi-member stream, all its members
/// together: what `InflateLimits::max_stream_total_bytes` gives at the
/// default per-member cap.
pub const DEFAULT_MAX_INFLATED_STREAM_BYTES: usize = 1024 * 1024 * 1024;

/// Upper bound on the capacity reserved before a DEFLATE member is
/// decoded. The reservation must not scale with the input that follows
/// the member: [`inflate_all_chunks_with_limits`] decodes every member
/// of a stream with the whole remainder as `data`, keeps each result
/// alive, and a 33 MB Revit 2024 project holds dozens of members, so a
/// `4 x remaining` hint reserved gigabytes. Native allocators back
/// untouched capacity lazily; wasm32 linear memory commits every grown
/// page and trapped at the 4 GiB ceiling in ~140 ms (viewer demo
/// core-interior-2024). Amortised `Vec` growth covers larger outputs.
const INITIAL_INFLATE_CAPACITY_BYTES: usize = 1024 * 1024;

/// Caps for bounded decompression. Passed explicitly to
/// [`inflate_at_with_limits`] or pulled from
/// [`InflateLimits::default`] by the back-compat [`inflate_at`]
/// wrapper.
///
/// # Why this exists
///
/// DEFLATE is an adversary-choice format: a small number of input
/// bytes can legitimately expand by 1000× or more (e.g. a 1 KB
/// block of zeros compresses to ~20 bytes; uncompressed is 1 KB;
/// a 1 MB block of zeros compresses to ~1 KB). The previous
/// `inflate_at` allocated `body.len() * 4` without a hard upper
/// bound and called `read_to_end` with no size limit, so a
/// hostile `.rvt` could trivially DoS any process that opened it.
///
/// size fields default to unlimited and are opt-in for untrusted input;
/// the protection on by default is the ratio guard ([`Self::max_ratio`])
/// and the available-memory check ([`crate::memory`]), both of which fail
/// with an error rather than returning less than the stream holds.
#[derive(Debug, Clone, Copy)]
pub struct InflateLimits {
    /// Maximum decompressed output bytes per gzip member. Default unlimited.
    pub max_output_bytes: usize,
    /// Maximum decompressed bytes of one multi-member stream
    /// ([`InflatedStream`]). Default unlimited.
    pub max_total_output_bytes: usize,
    /// Decompression-ratio guard: a member past
    /// [`RATIO_GUARD_FLOOR_BYTES`] of output may not exceed this many
    /// times the compressed bytes it has consumed. Default
    /// [`DEFAULT_MAX_INFLATE_RATIO`]; `u64::MAX` turns it off.
    pub max_ratio: u64,
}

impl Default for InflateLimits {
    fn default() -> Self {
        Self {
            max_output_bytes: usize::MAX,
            max_total_output_bytes: usize::MAX,
            max_ratio: DEFAULT_MAX_INFLATE_RATIO,
        }
    }
}

/// Stored bytes in a full page of a large gzip-bearing stream.
///
/// Evidence: our own probes on public models. A stream
/// of 65_249 or more bytes carries 353 extra bytes at the end of every full
/// 65_249-byte page, counted from stream offset 0; with them removed the gzip
/// members inflate and their CRC32 and ISIZE trailers verify, and without
/// binary cyclic code (see `revit_page_trailer`), which the writer computes
/// with `add_revit_page_checksums`.
pub const REVIT_STORED_PAGE_BYTES: usize = 65_249;

/// Payload bytes in a full page (65_249 - 353, see [`REVIT_STORED_PAGE_BYTES`]).
pub const REVIT_PAGE_PAYLOAD_BYTES: usize = 64_896;

/// Extra bytes cut from the end of each complete stored page before inflate.
pub const REVIT_PAGE_CHECKSUM_BYTES: usize = REVIT_STORED_PAGE_BYTES - REVIT_PAGE_PAYLOAD_BYTES;

/// Normalize a CFB stream path for gate comparisons.
fn clean_stream_path(path: &str) -> &str {
    let clean = path
        .trim_start_matches('/')
        .trim_start_matches("Root Entry/");
    clean.trim_start_matches('/')
}

/// Whether production inflate wrappers strip checksum-page trailers.
///
/// `Partitions/*`, the listed `Global/*` streams and,
/// `Formats/Latest`. `Formats/Latest` was left
/// out at first because stripping it lowered an opportunistic class-name
/// count; that count was drift garbage. With the strip, every file of one
/// Revit release inflates to the byte-identical schema, family or project,
/// and without it no two do.
///
/// `ProjectInformation`, `PartAtom`, `BasicFileInfo`, and preview streams are
/// never gated — they are not routed through the paged gzip reader.
///
/// Raw [`inflate_at`] / [`inflate_all_chunks`] never strip; only
/// [`prepare_stream_for_inflate`] / [`inflate_stream_at`] /
/// [`inflate_stream_auto`] / [`inflate_all_chunks_for_stream`] consult this gate.
pub fn is_checksum_paged_stream(path: &str) -> bool {
    let clean = clean_stream_path(path);
    if let Some(rest) = clean
        .strip_prefix("Partitions/")
        .or_else(|| clean.strip_prefix("partitions/"))
    {
        return !rest.is_empty() && !rest.contains('/');
    }
    matches!(
        clean,
        "Formats/Latest"
            | "Global/ContentDocuments"
            | "Global/DocumentIncrementTable"
            | "Global/ElemTable"
            | "Global/History"
            | "Global/Latest"
            | "Global/PartitionTable"
    )
}

/// Broader candidate set (includes `Formats/Latest`) for probes and research
/// — **not** the production strip gate.
///
/// Prefer [`is_checksum_paged_stream`] at decode call sites.
pub fn is_revit_paged_loader_candidate(path: &str) -> bool {
    let clean = clean_stream_path(path);
    if clean.eq_ignore_ascii_case("Formats/Latest") {
        return true;
    }
    is_checksum_paged_stream(path)
}

/// Remove each complete stored page's checksum trailer.
///
/// The last short page is retained in full: it carries no extra bytes
/// (its gzip trailer verifies unstripped). Call this on **stored** CFB bytes for
/// [`is_checksum_paged_stream`] paths **before** [`inflate_at`] /
/// [`inflate_all_chunks`]. Do **not** run this on writer identity copies —
/// [`crate::reader::RevitFile::read_stream`] returns stored bytes unchanged.
pub fn strip_revit_page_checksums(data: &[u8]) -> Vec<u8> {
    let full_pages = data.len() / REVIT_STORED_PAGE_BYTES;
    if full_pages == 0 {
        return data.to_vec();
    }
    let remainder = data.len() - full_pages * REVIT_STORED_PAGE_BYTES;
    let mut out = Vec::with_capacity(full_pages * REVIT_PAGE_PAYLOAD_BYTES + remainder);
    for page in 0..full_pages {
        let start = page * REVIT_STORED_PAGE_BYTES;
        out.extend_from_slice(&data[start..start + REVIT_PAGE_PAYLOAD_BYTES]);
    }
    out.extend_from_slice(&data[full_pages * REVIT_STORED_PAGE_BYTES..]);
    out
}

/// Primitive polynomials (with the x^d term) used by the page code, by degree.
const PAGE_CODE_POLYS: [(usize, u32); 7] = [
    (3, 0b1011),
    (4, 0b1_0011),
    (5, 0b10_0101),
    (6, 0b100_0011),
    (7, 0b1000_0011),
    (9, 0b10_0001_0001),
    (11, 0b1000_0000_0101),
];

/// The parity layout of a stored page of `n` bytes: `(u, d, rows)`.
///
/// **every** page of a checksum-paged stream carries parity, the
/// short last page (and a stream shorter than one page) included. The page is
/// read as bits, LSB first, in stream order; bit `j` belongs to codeword
/// `j % u` at row `j / u`. Each of the `u` codewords has `rows = 8n / u` rows:
/// the first `rows - d` are data and the last `d` are a CRC (zero init, MSB
/// first, the primitive polynomial of degree `d` in `PAGE_CODE_POLYS`) of them.
/// The `8n % u` bits after the last row are zero. `(u, d)` is chosen from `n`
/// alone: the smallest `d * u` over `65 <= u <= 255` with `8n % u <= 7` and `d`
/// the smallest listed degree with `2^d - 1 >= rows`. For a full page
/// (`n = 65_249`) that is `u = 255, d = 11`, the code of
/// `revit_page_trailer`. The layout came as a hint from an MIT-licensed
/// writer; it is accepted here because it reproduces Revit's own stored pages
/// byte for byte (`tests/page_trailers.rs`).
pub fn revit_page_code(n: usize) -> Option<(usize, usize, usize)> {
    let bits = 8 * n;
    let mut best: Option<(usize, usize, usize)> = None;
    for u in 65..=255.min(bits / 2) {
        if bits % u > 7 {
            continue;
        }
        let rows = bits / u;
        let Some(&(d, _)) = PAGE_CODE_POLYS.iter().find(|(d, _)| (1usize << d) > rows) else {
            continue;
        };
        if best.is_none_or(|(bu, bd, _)| d * u < bd * bu) {
            best = Some((u, d, rows));
        }
    }
    best
}

type CodewordBits = [u64; 4];

fn page_bit(page: &[u8], i: usize) -> bool {
    page[i / 8] >> (i % 8) & 1 == 1
}

fn page_row(page: &[u8], row: usize, u: usize) -> CodewordBits {
    let mut out = [0u64; 4];
    for j in 0..u {
        if page_bit(page, row * u + j) {
            out[j / 64] |= 1 << (j % 64);
        }
    }
    out
}

/// The `d` parity rows (`[0]` = first stored row) of a page under its code.
fn page_parity_rows(page: &[u8], u: usize, d: usize, rows: usize) -> Vec<CodewordBits> {
    let poly = PAGE_CODE_POLYS
        .iter()
        .find(|(x, _)| *x == d)
        .map_or(0, |p| p.1);
    let mut st = vec![[0u64; 4]; d];
    for t in 0..rows - d {
        let row = page_row(page, t, u);
        let fb: CodewordBits = std::array::from_fn(|k| st[d - 1][k] ^ row[k]);
        st.copy_within(0..d - 1, 1);
        st[0] = [0; 4];
        for (k, reg) in st.iter_mut().enumerate() {
            if poly >> k & 1 == 1 {
                for (r, f) in reg.iter_mut().zip(fb) {
                    *r ^= f;
                }
            }
        }
    }
    (0..d).map(|tp| st[d - 1 - tp]).collect()
}

/// Number of codewords of a stored page whose parity does not match, or
/// `None` when no code fits `page.len()`. Pad bits that are set count as one
/// more bad codeword.
pub fn revit_page_bad_codewords(page: &[u8]) -> Option<usize> {
    let (u, d, rows) = revit_page_code(page.len())?;
    let want = page_parity_rows(page, u, d, rows);
    let mut bad = [0u64; 4];
    for (tp, w) in want.iter().enumerate() {
        let got = page_row(page, rows - d + tp, u);
        for k in 0..4 {
            bad[k] |= got[k] ^ w[k];
        }
    }
    let pad = (rows * u..8 * page.len()).any(|i| page_bit(page, i));
    Some(bad.iter().map(|w| w.count_ones() as usize).sum::<usize>() + usize::from(pad))
}
/// [`strip_revit_page_checksums`] on bytes the caller owns: the payloads are
/// moved down over the trailers in the same buffer, so a stream the caller has
/// just read is not held twice while it is stripped.
pub(crate) fn strip_revit_page_checksums_in_place(mut data: Vec<u8>) -> Vec<u8> {
    let full_pages = data.len() / REVIT_STORED_PAGE_BYTES;
    if full_pages == 0 {
        return data;
    }
    let remainder_start = full_pages * REVIT_STORED_PAGE_BYTES;
    let remainder = data.len() - remainder_start;
    for page in 1..full_pages {
        let from = page * REVIT_STORED_PAGE_BYTES;
        data.copy_within(
            from..from + REVIT_PAGE_PAYLOAD_BYTES,
            page * REVIT_PAGE_PAYLOAD_BYTES,
        );
    }
    let stripped = full_pages * REVIT_PAGE_PAYLOAD_BYTES;
    data.copy_within(remainder_start.., stripped);
    data.truncate(stripped + remainder);
    data
}

/// [`prepare_stream_for_inflate`] on stored bytes the caller owns, which are
/// stripped in place rather than copied.
pub(crate) fn prepare_stream_owned(stream_name: &str, stored: Vec<u8>) -> Vec<u8> {
    if is_checksum_paged_stream(stream_name) {
        strip_revit_page_checksums_in_place(stored)
    } else {
        stored
    }
}

/// Return checksum-clean bytes ready for inflate when `stream_name` is gated.
///
/// Does **not** mutate writer identity copies: [`crate::reader::RevitFile::read_stream`]
/// still returns stored CFB bytes unchanged. Decode-path only.
pub fn prepare_stream_for_inflate<'a>(
    stream_name: &str,
    stored: &'a [u8],
) -> std::borrow::Cow<'a, [u8]> {
    if is_checksum_paged_stream(stream_name) {
        std::borrow::Cow::Owned(strip_revit_page_checksums(stored))
    } else {
        std::borrow::Cow::Borrowed(stored)
    }
}

/// Inflate a named stored stream, stripping page checksums when
/// [`is_checksum_paged_stream`] says so.
///
/// Prefer this over bare [`inflate_at`] whenever the CFB stream path is known.
/// Raw [`inflate_at`] remains a path-agnostic codec and **never** strips — so
/// Formats/Latest, tiny streams, and already-clean buffers stay untouched.
pub fn inflate_stream_at(stream_name: &str, stored: &[u8], offset: usize) -> Result<Vec<u8>> {
    inflate_stream_at_with_limits(stream_name, stored, offset, InflateLimits::default())
}

/// [`inflate_stream_at`] under `limits`.
pub fn inflate_stream_at_with_limits(
    stream_name: &str,
    stored: &[u8],
    offset: usize,
    limits: InflateLimits,
) -> Result<Vec<u8>> {
    let prepared = prepare_stream_for_inflate(stream_name, stored);
    inflate_at_with_limits(prepared.as_ref(), offset, limits)
}

/// Auto-offset inflate for a named stored stream (page-stripped when gated).
pub fn inflate_stream_auto(stream_name: &str, stored: &[u8]) -> Result<(usize, Vec<u8>)> {
    inflate_stream_auto_with_limits(stream_name, stored, InflateLimits::default())
}

/// [`inflate_stream_auto`] under `limits`.
pub fn inflate_stream_auto_with_limits(
    stream_name: &str,
    stored: &[u8],
    limits: InflateLimits,
) -> Result<(usize, Vec<u8>)> {
    let prepared = prepare_stream_for_inflate(stream_name, stored);
    inflate_at_auto_with_limits(prepared.as_ref(), limits)
}

/// Inflate every gzip member in a named stored stream (page-stripped when gated).
///
/// Primary production entry for multi-member `Partitions/*` recovery.
pub fn inflate_all_chunks_for_stream(stream_name: &str, stored: &[u8]) -> Vec<Vec<u8>> {
    let prepared = prepare_stream_for_inflate(stream_name, stored);
    inflate_all_chunks(prepared.as_ref())
}

/// Read all of a decompressing `reader` into `out` under the default ratio
/// guard ([`DEFAULT_MAX_INFLATE_RATIO`]) and the memory check
/// ([`crate::memory`]), for decoders this module does not drive itself.
/// `remaining(reader)` is how much of the `input_len`
/// compressed bytes the reader has not consumed yet.
pub fn read_guarded<R: Read>(
    reader: &mut R,
    out: &mut Vec<u8>,
    remaining: impl Fn(&R) -> usize,
    input_len: usize,
) -> Result<()> {
    let mut buf = [0u8; 8192];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            return Ok(());
        }
        let consumed = input_len.saturating_sub(remaining(reader)).max(1) as u64;
        let produced = out.len() + n;
        if produced > RATIO_GUARD_FLOOR_BYTES
            && produced as u64 > DEFAULT_MAX_INFLATE_RATIO.saturating_mul(consumed)
        {
            return Err(Error::DecompressLimitExceeded(format!(
                "a gzip member inflates {produced} bytes from {consumed} \
                 (over {DEFAULT_MAX_INFLATE_RATIO}:1): refused as a decompression bomb"
            )));
        }
        if out.len() + n > out.capacity() {
            crate::memory::ensure_available(
                out.capacity().max(INITIAL_INFLATE_CAPACITY_BYTES) as u64,
                "inflating a native stream",
            )?;
        }
        out.extend_from_slice(&buf[..n]);
    }
}

/// One gzip member of a stored stream: where its header starts
/// in the prepared stream ([`prepare_stream_for_inflate`]), the bytes its
/// header and DEFLATE stream take there, and where its bytes start in the
/// stream's inflated concatenation and how many there are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemberSpan {
    pub offset: usize,
    pub stored_len: usize,
    pub inflated_start: usize,
    pub inflated_len: usize,
}

/// Inflate the members of a stored stream one at a time into one reused
/// buffer and hand each to `visit` with its [`MemberSpan`].
/// Memory is one member, not the stream: the way to read a multi-GB stream
/// whole without holding it. `visit` returns `false` to stop. Returns the
/// member index read so far and the limit that stopped the read, as
/// [`InflatedStream::limit_hit`] reports it.
pub fn for_each_member(
    stream_name: &str,
    stored: &[u8],
    limits: InflateLimits,
    mut visit: impl FnMut(&MemberSpan, &[u8]) -> bool,
) -> (
    Vec<MemberSpan>,
    Option<(&'static str, String)>,
    Option<String>,
) {
    let prepared = prepare_stream_for_inflate(stream_name, stored);
    let data = prepared.as_ref();
    let mut index = Vec::new();
    let mut buf = Vec::new();
    let mut inflated_start = 0usize;
    let mut hit = None;
    let mut skips = SkipTracker::default();
    let mut stopped = false;
    for off in find_gzip_offsets(data) {
        buf.clear();
        match inflate_member_into(data, off, limits, &mut buf) {
            Ok(consumed) => {
                if inflated_start.saturating_add(buf.len()) > limits.max_total_output_bytes {
                    let message = format!(
                        "{stream_name}: members past {} inflated bytes were not read",
                        limits.max_total_output_bytes
                    );
                    let skipped = skipped_member_event(stream_name, &skips.regions);
                    return (
                        index,
                        hit.or(Some((INFLATE_MAX_STREAM_BYTES, message))),
                        skipped,
                    );
                }
                skips.inflated(off, consumed);
                let span = MemberSpan {
                    offset: off,
                    stored_len: consumed,
                    inflated_start,
                    inflated_len: buf.len(),
                };
                inflated_start += buf.len();
                index.push(span);
                if !visit(&span, &buf) {
                    stopped = true;
                    break;
                }
            }
            Err(Stop::Limit(code, message)) => {
                let hit_now = (code, format!("{stream_name}: {message}"));
                if code != INFLATE_MAX_MEMBER_BYTES {
                    let skipped = skipped_member_event(stream_name, &skips.regions);
                    return (index, hit.or(Some(hit_now)), skipped);
                }
                hit.get_or_insert(hit_now);
                skips.capped(off);
            }
            Err(Stop::Corrupt(_)) => skips.failed(off),
        }
    }
    // A visitor that stops early has not read the rest: nothing past it is
    // judged skipped.
    let regions = if stopped {
        skips.regions
    } else {
        skips.finish(data.len())
    };
    (index, hit, skipped_member_event(stream_name, &regions))
}

/// Every gzip member of one stored stream, inflated and concatenated.
///
/// Built once per stream by [`crate::reader::RevitFile::inflated_partition`]
/// and shared by every partition consumer. Holding the concatenation
/// (rather than the member vector) is what the scanners actually want:
/// each of them re-joined the members itself, and the byte-at-a-time
/// `chunks.into_iter().flatten().collect()` those sites used cost more
/// than the inflate on a project file.
#[derive(Debug, Default)]
pub struct InflatedStream {
    concat: Vec<u8>,
    chunk_ends: Vec<usize>,
    /// Each member's offset in the prepared stream and the bytes its gzip
    /// header and DEFLATE stream take there.
    members: Vec<(usize, usize)>,
    /// Where each keyed section (embedded content document) starts in the
    /// inflated bytes, with its GUID from the 220-byte header before its
    /// first member.
    sections: Vec<(usize, [u8; 16])>,
    /// The limit that stopped the inflate, as (code, message).
    limit_hit: Option<(&'static str, String)>,
    /// The `skipped-member` diagnostic, when a member did not inflate.
    skipped: Option<String>,
    /// Each member's CRC-32, computed the first time a reader checks it:
    /// every native pass checks the same members.
    crcs: Vec<std::sync::OnceLock<u32>>,
}

/// UTF-16LE "Data generated" of the 220-byte section header's signature text.
const SECTION_HEADER_TEXT: &[u8] = b"D\0a\0t\0a\0 \0g\0e\0n\0e\0r\0a\0t\0e\0d\0";

/// The GUID in the 220-byte header that precedes a member at `off`, if one
/// does (header bytes 178..194; T: 163 of 163 on rac_basic 2024).
fn section_guid_before(data: &[u8], off: usize) -> Option<[u8; 16]> {
    let window = data.get(off.checked_sub(220)?..off)?;
    memchr::memmem::find(window, SECTION_HEADER_TEXT)?;
    window.get(178..194)?.try_into().ok()
}

impl InflatedStream {
    /// Inflate every member of `stored` and concatenate them in order, under
    /// the default [`InflateLimits`] (no size cap; ratio and memory guards).
    pub fn from_stored(stream_name: &str, stored: &[u8]) -> Self {
        Self::from_stored_with_limits(stream_name, stored, InflateLimits::default())
    }

    /// [`Self::from_stored`] under explicit limits. Members are inflated
    /// straight into one buffer, so the stream is held once, not once per
    /// member and again concatenated.
    pub fn from_stored_with_limits(
        stream_name: &str,
        stored: &[u8],
        limits: InflateLimits,
    ) -> Self {
        let prepared = prepare_stream_for_inflate(stream_name, stored);
        let data = prepared.as_ref();
        let mut out = Self::default();
        let mut skips = SkipTracker::default();
        let mut stopped = false;
        for off in find_gzip_offsets(data) {
            let start = out.concat.len();
            match inflate_member_into(data, off, limits, &mut out.concat) {
                Ok(consumed) => {
                    skips.inflated(off, consumed);
                    if out.concat.len() > limits.max_total_output_bytes {
                        stopped = true;
                        out.concat.truncate(start);
                        out.limit_hit = Some((
                            INFLATE_MAX_STREAM_BYTES,
                            format!(
                                "{stream_name}: members past {} inflated bytes were not read",
                                limits.max_total_output_bytes
                            ),
                        ));
                        break;
                    }
                    out.chunk_ends.push(out.concat.len());
                    out.members.push((off, consumed));
                    if let Some(guid) = section_guid_before(data, off) {
                        out.sections.push((start, guid));
                    }
                }
                Err(Stop::Limit(code, message)) => {
                    out.limit_hit
                        .get_or_insert((code, format!("{stream_name}: {message}")));
                    // A member over the opt-in size cap is skipped; a bomb or
                    // a lack of memory stops the stream.
                    if code != INFLATE_MAX_MEMBER_BYTES {
                        stopped = true;
                        break;
                    }
                    skips.capped(off);
                }
                // A false magic inside compressed data, or a corrupt or
                // truncated member: [`SkipTracker`] tells them apart.
                Err(Stop::Corrupt(_)) => skips.failed(off),
            }
        }
        // A limit that stopped the stream is reported as such; only the
        // regions before it are judged skipped.
        let regions = if stopped {
            skips.regions
        } else {
            skips.finish(data.len())
        };
        out.skipped = skipped_member_event(stream_name, &regions);
        out.crcs = (0..out.members.len())
            .map(|_| std::sync::OnceLock::new())
            .collect();
        out.concat.shrink_to_fit();
        out
    }

    /// The `skipped-member` diagnostic when a gzip member of this stream did
    /// not inflate and its bytes were skipped, else `None`.
    pub fn skipped_member(&self) -> Option<&str> {
        self.skipped.as_deref()
    }

    /// The limit that stopped part of this stream from being inflated, as
    /// `(code, message)`: [`INFLATE_RATIO`] or
    /// [`INFLATE_MEMORY`] by default; [`INFLATE_MAX_MEMBER_BYTES`] or
    /// [`INFLATE_MAX_STREAM_BYTES`] only under opt-in size caps.
    pub fn limit_hit(&self) -> Option<(&'static str, &str)> {
        self.limit_hit.as_ref().map(|(c, m)| (*c, m.as_str()))
    }

    /// The inflated member whose gzip header starts at `offset` of the
    /// prepared stream ([`prepare_stream_for_inflate`]), with the bytes its
    /// header and DEFLATE stream take there.
    pub fn member_at(&self, offset: usize) -> Option<(&[u8], usize)> {
        let (i, member) = self.member_index(offset)?;
        Some((member, self.members[i].1))
    }

    /// The index of the member whose gzip header starts at `offset`, and its
    /// inflated bytes.
    pub(crate) fn member_index(&self, offset: usize) -> Option<(usize, &[u8])> {
        let i = self
            .members
            .binary_search_by_key(&offset, |&(start, _)| start)
            .ok()?;
        let start = i.checked_sub(1).map_or(0, |prev| self.chunk_ends[prev]);
        Some((i, &self.concat[start..self.chunk_ends[i]]))
    }

    /// Where member `index` starts in the concatenated inflated bytes
    /// ([`Self::bytes`]).
    pub(crate) fn member_start(&self, index: usize) -> Option<usize> {
        if index >= self.chunk_ends.len() {
            return None;
        }
        Some(index.checked_sub(1).map_or(0, |prev| self.chunk_ends[prev]))
    }

    /// The CRC-32 of the member [`Self::member_at`] gives for `offset`,
    /// computed once for the stream's lifetime.
    pub(crate) fn member_crc_at(&self, offset: usize) -> Option<u32> {
        let (i, member) = self.member_index(offset)?;
        Some(*self.crcs.get(i)?.get_or_init(|| {
            let mut crc = flate2::Crc::new();
            crc.update(member);
            crc.sum()
        }))
    }

    /// The keyed sections of this stream: the inflated offset each starts at
    /// and its content document's GUID. Empty before Revit 2020.
    pub fn sections(&self) -> &[(usize, [u8; 16])] {
        &self.sections
    }

    /// The inflated members, concatenated in stored order.
    pub fn bytes(&self) -> &[u8] {
        &self.concat
    }

    /// The bytes member `index` takes in the prepared stream: its gzip header
    /// and its DEFLATE stream.
    pub(crate) fn member_consumed(&self, index: usize) -> Option<usize> {
        self.members.get(index).map(|&(_, consumed)| consumed)
    }

    /// The members `first` to `last` (indices into [`Self::chunk_ends`]) as
    /// one slice of [`Self::bytes`], borrowed rather than copied.
    pub(crate) fn members_span(&self, first: usize, last: usize) -> Option<&[u8]> {
        let start = first
            .checked_sub(1)
            .map_or(Some(0), |prev| self.chunk_ends.get(prev).copied())?;
        self.concat.get(start..*self.chunk_ends.get(last)?)
    }

    /// End offset of each member within [`Self::bytes`].
    pub fn chunk_ends(&self) -> &[usize] {
        &self.chunk_ends
    }

    /// Number of inflated members.
    pub fn chunk_count(&self) -> usize {
        self.chunk_ends.len()
    }

    /// Iterate the members as slices of [`Self::bytes`].
    pub fn chunks(&self) -> impl Iterator<Item = &[u8]> {
        let mut start = 0usize;
        self.chunk_ends.iter().map(move |end| {
            let slice = &self.concat[start..*end];
            start = *end;
            slice
        })
    }
}

/// Whether production inflate strips checksum-page trailers for a stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ChecksumTailStripping {
    /// No longer produced for `Formats/Latest`; kept for readers of
    /// older diagnostics.
    Disabled,
    Enabled,
}

/// Coarse integrity verdict for `Formats/Latest` after production inflate.
///
/// Multipage streams stay [`FormatsIntegrityStatus::Uncertain`] while strip
/// remains disabled — do **not** treat inflate success as completeness proof
/// (residual).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum FormatsIntegrityStatus {
    /// Stored length is under one full checksum page and inflate succeeded.
    Ok,
    /// At least one full page boundary is present; completeness unverified.
    Uncertain,
    /// Stream missing or production inflate failed.
    Incomplete,
}

/// Stable diagnostic code for Formats multipage integrity.
pub const RVT_FORMATS_MULTIPAGE_UNVERIFIED: &str = "RVT_FORMATS_MULTIPAGE_UNVERIFIED";

/// Machine-readable `Formats/Latest` integrity sidecar for inspect / export /
/// viewer File Status.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct FormatsLatestIntegrity {
    pub stream: String,
    pub stored_bytes: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inflated_bytes: Option<usize>,
    pub page_boundary_detected: bool,
    pub checksum_tail_stripping: ChecksumTailStripping,
    pub integrity_status: FormatsIntegrityStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostic_code: Option<String>,
    /// `true` when the inflated stream is larger than
    /// [`crate::formats::SCHEMA_SCAN_LIMIT`], so schema parsing stops before
    /// the end of the stream. A "we stopped looking" signal — it does not
    /// claim the unscanned tail is parseable schema.
    #[serde(default)]
    pub schema_scan_truncated: bool,
    /// Bytes schema parsing would scan — `min(inflated_bytes,
    /// SCHEMA_SCAN_LIMIT)`, or `None` when inflate failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_scanned_bytes: Option<usize>,
}

impl FormatsLatestIntegrity {
    /// Short user-facing summary for CLI / File Status rows.
    pub fn summary_line(&self) -> String {
        let mut line = match self.integrity_status {
            FormatsIntegrityStatus::Ok => format!(
                "Formats/Latest · {} stored · inflate ok · single-page",
                self.stored_bytes
            ),
            FormatsIntegrityStatus::Uncertain => format!(
                "Formats/Latest · {} stored · multipage integrity uncertain (strip disabled)",
                self.stored_bytes
            ),
            FormatsIntegrityStatus::Incomplete => format!(
                "Formats/Latest · {} stored · inflate incomplete",
                self.stored_bytes
            ),
        };
        if self.schema_scan_truncated {
            let scanned = self
                .schema_scanned_bytes
                .unwrap_or(crate::formats::SCHEMA_SCAN_LIMIT);
            let total = self.inflated_bytes.unwrap_or(scanned);
            line.push_str(&format!(
                " · schema scan truncated ({scanned} of {total} inflated bytes scanned)"
            ));
        }
        line
    }
}

/// Diagnose `Formats/Latest`: its size, whether it spans checksum pages,
/// and whether it inflates once they are stripped.
pub fn diagnose_formats_latest_integrity(stored: &[u8]) -> FormatsLatestIntegrity {
    let stored_bytes = stored.len();
    let page_boundary_detected = stored_bytes >= REVIT_STORED_PAGE_BYTES;
    let inflate = inflate_stream_at(crate::streams::FORMATS_LATEST, stored, 0);
    let inflated_bytes = inflate.as_ref().ok().map(|b| b.len());
    // The stream ends in a real gzip trailer: after the strip its CRC32 and
    // ISIZE verify on every file measured, so a stream is `Ok` only
    // when they do.
    let (integrity_status, diagnostic_code) = match &inflate {
        Err(_) => (FormatsIntegrityStatus::Incomplete, None),
        Ok(_)
            if gzip_trailer_verifies(&prepare_stream_for_inflate(
                crate::streams::FORMATS_LATEST,
                stored,
            )) =>
        {
            (FormatsIntegrityStatus::Ok, None)
        }
        Ok(_) => (
            FormatsIntegrityStatus::Uncertain,
            Some(RVT_FORMATS_MULTIPAGE_UNVERIFIED.to_string()),
        ),
    };
    let schema_scanned_bytes = inflated_bytes;
    let schema_scan_truncated = false;

    FormatsLatestIntegrity {
        stream: crate::streams::FORMATS_LATEST.to_string(),
        stored_bytes,
        inflated_bytes,
        page_boundary_detected,
        checksum_tail_stripping: ChecksumTailStripping::Enabled,
        integrity_status,
        diagnostic_code,
        schema_scan_truncated,
        schema_scanned_bytes,
    }
}

/// True when `data` is one gzip member whose CRC32 and ISIZE trailer match
/// what it inflates to, within the ratio guard ([`DEFAULT_MAX_INFLATE_RATIO`]).
fn gzip_trailer_verifies(data: &[u8]) -> bool {
    let mut decoder = flate2::read::GzDecoder::new(data);
    let mut buf = [0u8; 8192];
    let mut total = 0usize;
    loop {
        match Read::read(&mut decoder, &mut buf) {
            Ok(0) => return true,
            Ok(n) => {
                total = total.saturating_add(n);
                if total > RATIO_GUARD_FLOOR_BYTES
                    && total as u64 > DEFAULT_MAX_INFLATE_RATIO.saturating_mul(data.len() as u64)
                {
                    return false;
                }
            }
            Err(_) => return false,
        }
    }
}

/// Returns `true` iff `data` starts with the gzip magic at the given offset.
pub fn has_gzip_magic(data: &[u8], offset: usize) -> bool {
    offset
        .checked_add(GZIP_MAGIC.len())
        .and_then(|end| data.get(offset..end))
        == Some(GZIP_MAGIC.as_slice())
}

/// Length of the gzip header starting at `offset`, or `None` if no magic.
///
/// Standard gzip: magic(3) + method(1) + flags(1) + mtime(4) + xfl(1) + os(1) = 10 bytes.
/// Plus optional FEXTRA / FNAME / FCOMMENT / FHCRC fields when flags are set.
pub fn gzip_header_len(data: &[u8], offset: usize) -> Option<usize> {
    if !has_gzip_magic(data, offset) {
        return None;
    }
    let flags = *data.get(offset.checked_add(3)?)?;
    let mut pos = offset.checked_add(10)?;
    if flags & 0x04 != 0 {
        // FEXTRA: 2-byte LE length, then that many bytes
        let xlen_end = pos.checked_add(2)?;
        let xlen = le_bytes::u16_at(data, pos)? as usize;
        pos = xlen_end.checked_add(xlen)?;
    }
    if flags & 0x08 != 0 {
        // FNAME: null-terminated string. Bounds-check `pos` first —
        // the 10-byte base header might not fit if `offset + 10 >
        // data.len()`, in which case slicing `data[pos..]` would
        // panic. Discovered by libFuzzer fuzz_inflate_at_with_limits
        // on input `1f 8b 08 b9` (FNAME+FHCRC flag set,
        // 4-byte buffer).
        pos = data
            .get(pos..)?
            .iter()
            .position(|&b| b == 0)
            .and_then(|i| pos.checked_add(i)?.checked_add(1))?;
    }
    if flags & 0x10 != 0 {
        // FCOMMENT: null-terminated string. Same bounds-check
        // rationale as FNAME above.
        pos = data
            .get(pos..)?
            .iter()
            .position(|&b| b == 0)
            .and_then(|i| pos.checked_add(i)?.checked_add(1))?;
    }
    if flags & 0x02 != 0 {
        // FHCRC: 2-byte header CRC
        pos = pos.checked_add(2)?;
    }
    // The base 10-byte header is set unconditionally at `pos = offset
    // + 10` above, but none of the `data.get(..)` probes on the
    // optional flag fields fire when those flags are clear. Verify
    // the base header actually fits in the buffer before returning —
    // otherwise a caller using the returned length to slice past
    // `data.len()` will panic, as a 9-byte `[0x1f, 0x8b, 0x08, 0x00, ...]`
    // input once did.
    if pos > data.len() {
        return None;
    }
    Some(pos - offset)
}

/// Inflate the DEFLATE stream that follows a gzip header starting at
/// `offset`, with an output-size ceiling enforced.
///
/// Returns the decompressed bytes. Unused tail (any garbage / next
/// chunk / missing CRC+ISIZE) is silently ignored — which is exactly
/// what we need for Revit's truncated-gzip streams.
///
/// The output size is capped by `limits.max_output_bytes`. If a
/// DEFLATE block would expand beyond that cap, the function returns
/// [`Error::DecompressLimitExceeded`] rather than allocating
/// unbounded memory. The initial allocation is also clamped — the
/// historic `body.len() * 4` hint is bounded to at most
/// `max_output_bytes` so an attacker can't force a multi-GB
/// allocation just by handing us a large compressed body.
///
/// ```
/// # use rvt::compression::{self, InflateLimits};
/// let empty_truncated_gzip = [
///     0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff,
///     0x03, 0x00,
/// ];
/// let decomp = compression::inflate_at_with_limits(
///     &empty_truncated_gzip, 0, InflateLimits::default()
/// ).unwrap();
/// assert_eq!(decomp, b"");
/// ```
pub fn inflate_at_with_limits(
    data: &[u8],
    offset: usize,
    limits: InflateLimits,
) -> Result<Vec<u8>> {
    inflate_member_at_with_limits(data, offset, limits).map(|(out, _)| out)
}

/// [`inflate_at_with_limits`], with the number of bytes of `data` the gzip
/// header and its DEFLATE stream take from `offset`.
pub fn inflate_member_at_with_limits(
    data: &[u8],
    offset: usize,
    limits: InflateLimits,
) -> Result<(Vec<u8>, usize)> {
    let mut out = Vec::new();
    let consumed = inflate_member_into(data, offset, limits, &mut out).map_err(Stop::into_error)?;
    // Results are retained per member by the chunk collectors; trim the
    // amortised-growth slack so retained buffers cost what they hold.
    out.shrink_to_fit();
    Ok((out, consumed))
}

/// Why [`inflate_member_into`] stopped.
enum Stop {
    /// Not a decodable member.
    Corrupt(String),
    /// A limit (code, message).
    Limit(&'static str, String),
}

impl Stop {
    fn into_error(self) -> Error {
        match self {
            Self::Corrupt(m) => Error::Decompress(m),
            Self::Limit(INFLATE_MEMORY, m) => Error::InsufficientMemory(m),
            Self::Limit(_, m) => Error::DecompressLimitExceeded(m),
        }
    }
}

/// Inflate the gzip member at `offset` of `data`, appending to `out`, and
/// return the bytes its header and DEFLATE stream take. On failure `out` is
/// left as it was. Guards: the opt-in size cap, the ratio guard,
/// and the available-memory check made before `out` grows.
fn inflate_member_into(
    data: &[u8],
    offset: usize,
    limits: InflateLimits,
    out: &mut Vec<u8>,
) -> std::result::Result<usize, Stop> {
    inflate_member_into_reporting(data, offset, limits, out, &mut 0)
}

/// [`inflate_member_into`] that also sets `reached` to how far into `data`
/// (from `offset`) the header and DEFLATE input were read, success or not:
/// the streamed index tells a member cut short by the end of its
/// read-ahead from one that ends or fails on its own.
fn inflate_member_into_reporting(
    data: &[u8],
    offset: usize,
    limits: InflateLimits,
    out: &mut Vec<u8>,
    reached: &mut usize,
) -> std::result::Result<usize, Stop> {
    *reached = 0;
    let header_len =
        gzip_header_len(data, offset).ok_or_else(|| Stop::Corrupt("no gzip header".into()))?;
    let body = offset
        .checked_add(header_len)
        .and_then(|start| data.get(start..))
        .ok_or_else(|| Stop::Corrupt("gzip header extends past input".into()))?;
    let start = out.len();
    if out.capacity() == 0 {
        // `body` is the whole remainder of the stream, so the hint is
        // bounded (see [`INITIAL_INFLATE_CAPACITY_BYTES`]).
        out.reserve(
            body.len()
                .saturating_mul(4)
                .min(INITIAL_INFLATE_CAPACITY_BYTES),
        );
    }
    let mut decoder = DeflateDecoder::new(body);
    let mut buf = [0u8; 8192];
    let result = loop {
        let n = match decoder.read(&mut buf) {
            Ok(0) => break Ok(()),
            Ok(n) => n,
            Err(e) => break Err(Stop::Corrupt(format!("DEFLATE at offset {offset}: {e}"))),
        };
        let produced = out.len() - start + n;
        if produced > limits.max_output_bytes {
            break Err(Stop::Limit(
                INFLATE_MAX_MEMBER_BYTES,
                format!(
                    "DEFLATE at offset {offset} would exceed {} bytes",
                    limits.max_output_bytes
                ),
            ));
        }
        let consumed = decoder.total_in().max(1);
        if produced > RATIO_GUARD_FLOOR_BYTES
            && produced as u64 > limits.max_ratio.saturating_mul(consumed)
        {
            break Err(Stop::Limit(
                INFLATE_RATIO,
                format!(
                    "gzip member at offset {offset} inflates {produced} bytes from {consumed} \
                     (over {}:1): refused as a decompression bomb",
                    limits.max_ratio
                ),
            ));
        }
        if out.len() + n > out.capacity() {
            // The next growth allocates about the current capacity again.
            let needed = out.capacity().max(INITIAL_INFLATE_CAPACITY_BYTES) as u64;
            if let Err(e) = crate::memory::ensure_available(
                needed,
                &format!("inflating the gzip member at offset {offset}"),
            ) {
                break Err(Stop::Limit(INFLATE_MEMORY, e.to_string()));
            }
        }
        out.extend_from_slice(&buf[..n]);
    };
    *reached = header_len + decoder.total_in() as usize;
    match result {
        Ok(()) => Ok(header_len + decoder.total_in() as usize),
        Err(stop) => {
            out.truncate(start);
            Err(stop)
        }
    }
}

/// Inflate the DEFLATE stream that follows a gzip header starting at `offset`.
///
/// Backwards-compatible wrapper around [`inflate_at_with_limits`] with
/// the default [`InflateLimits`] (no size cap; ratio and memory guards). Existing callers
/// continue to work unchanged; the cap only kicks in on adversarial
/// input that would not have produced a useful result anyway.
///
/// ```
/// # use rvt::compression;
/// // Revit omits the trailing CRC+ISIZE, so standard gzip decoders refuse
/// // these streams. Our `inflate_at` returns the decompressed bytes
/// // regardless.
/// let empty_truncated_gzip = [
///     0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff, // 10-byte header
///     0x03, 0x00, // empty deflate block
/// ];
/// let decomp = compression::inflate_at(&empty_truncated_gzip, 0).unwrap();
/// assert_eq!(decomp, b"");
/// ```
pub fn inflate_at(data: &[u8], offset: usize) -> Result<Vec<u8>> {
    inflate_at_with_limits(data, offset, InflateLimits::default())
}

/// Inflate a Revit Global/* stream without requiring the caller to
/// know the custom-prefix length.
///
/// Family files (the phi-ag/rvt corpus through 2026) put an 8-byte
/// custom header in front of the gzip body on most Global/* streams
/// (`Global/History`, `Global/Latest`, `Global/PartitionTable`, etc).
/// Project files (`.rvt`) don't always — on at least some releases
/// (observed on a 2025-saved `.rvt`) `Global/History` starts with the
/// gzip magic at offset 0 and has no custom prefix at all.
///
/// This helper probes for the first gzip magic in `data` via
/// [`find_gzip_offsets`] and inflates from there. If no magic is
/// found, it falls back to the family-file heuristic of offset 8 so
/// callers still see a structured `Err(..)` for diagnostics rather
/// than a silent mismatch.
///
/// Returns the tuple `(prefix_len, decompressed_bytes)`; the custom
/// prefix is `data[..prefix_len]`.
pub fn inflate_at_auto(data: &[u8]) -> Result<(usize, Vec<u8>)> {
    inflate_at_auto_with_limits(data, InflateLimits::default())
}

/// [`inflate_at_auto`] under `limits`.
pub fn inflate_at_auto_with_limits(data: &[u8], limits: InflateLimits) -> Result<(usize, Vec<u8>)> {
    // Heuristic: the first gzip magic in the stream. Family files hit
    // at offset 8; project files sometimes hit at offset 0. We always
    // prefer the observed offset over a hard-coded constant because
    // the hard-coded value panicked on the first project-file Global/
    // History we probed.
    let offset = find_gzip_offsets(data).into_iter().next().unwrap_or(8);
    let out = inflate_at_with_limits(data, offset, limits)?;
    Ok((offset, out))
}

/// Find every gzip magic byte-triple in `data`.
/// Used for streams like `Partitions/NN` that pack multiple GZIP segments.
pub fn find_gzip_offsets(data: &[u8]) -> Vec<usize> {
    let mut hits = Vec::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        if data[i..i + 3] == GZIP_MAGIC {
            hits.push(i);
            i += 3;
        } else {
            i += 1;
        }
    }
    hits
}

/// Inflate every GZIP chunk in `data`, concatenating the outputs,
/// with bounded per-chunk + aggregate output sizes.
///
/// `per_chunk` caps each chunk's inflated size (same semantics as
/// [`inflate_at_with_limits`]). `aggregate` caps the total bytes
/// returned across all chunks — useful when the caller wants to
/// refuse processing of streams that would expand to multi-gigabyte
/// totals even if each individual chunk is small.
///
/// False positives (byte triples that match the gzip magic inside
/// random compressed data) continue to be silently skipped via
/// `filter_map` — their error is "looks like gzip but isn't." A
/// chunk that IS gzip but would exceed `per_chunk` is also skipped.
/// A fully-materialised result that would exceed `aggregate` causes
/// iteration to stop early at the last chunk whose inclusion keeps
/// us under budget.
pub fn inflate_all_chunks_with_limits(
    data: &[u8],
    per_chunk: InflateLimits,
    aggregate: usize,
) -> Vec<Vec<u8>> {
    inflate_all_members_with_limits(data, per_chunk, aggregate)
        .0
        .into_iter()
        .map(|(_, _, chunk)| chunk)
        .collect()
}

/// Inflated members as (offset in `data`, stored length, bytes), plus the
/// limit code that stopped the walk, if any.
type InflatedMembers = (Vec<(usize, usize, Vec<u8>)>, Option<&'static str>);

/// [`inflate_all_chunks_with_limits`], with each chunk's offset in `data`
/// and the bytes its gzip header and DEFLATE stream take there, and what the
/// limits left out.
fn inflate_all_members_with_limits(
    data: &[u8],
    per_chunk: InflateLimits,
    aggregate: usize,
) -> InflatedMembers {
    let mut total: usize = 0;
    let mut results = Vec::new();
    let mut limit_hit = None;
    for off in find_gzip_offsets(data) {
        let mut chunk = Vec::new();
        let consumed = match inflate_member_into(data, off, per_chunk, &mut chunk) {
            Ok(c) => c,
            Err(Stop::Limit(code, _)) => {
                limit_hit.get_or_insert(code);
                if code == INFLATE_MAX_MEMBER_BYTES {
                    continue;
                }
                break;
            }
            Err(Stop::Corrupt(_)) => continue,
        };
        if total.saturating_add(chunk.len()) > aggregate {
            limit_hit.get_or_insert(INFLATE_MAX_STREAM_BYTES);
            break;
        }
        total += chunk.len();
        chunk.shrink_to_fit();
        results.push((off, consumed, chunk));
    }
    (results, limit_hit)
}

/// Inflate every GZIP chunk in `data`, concatenating the outputs.
/// Silently skips chunks that fail to inflate (some trailing "magic-like"
/// byte triples in random compressed data are false positives).
///
/// Backwards-compatible wrapper around [`inflate_all_chunks_with_limits`].
/// Uses the default [`InflateLimits`]: no size cap, ratio and memory guards.
pub fn inflate_all_chunks(data: &[u8]) -> Vec<Vec<u8>> {
    inflate_all_chunks_with_limits(data, InflateLimits::default(), usize::MAX)
}

// ---- Streamed partitions ---------------------------------------
//
// A partition stream read without holding it: one pass over the stored
// bytes records every gzip member (`MemberIndex`), the same members, in the
// same order and with the same acceptance as `InflatedStream`, and any member
// is then re-inflated on demand from its stored bytes. Memory is the index
// (one `MemberSpan` per member) plus whatever the caller keeps.

/// `Read + Seek`: a stored stream opened from the container.
pub trait ReadSeek: Read + std::io::Seek {}
impl<T: Read + std::io::Seek> ReadSeek for T {}

/// Random access to a stored stream's page-stripped bytes
/// ([`prepare_stream_for_inflate`]) without copying the stream: offsets are
/// those of the stripped bytes, mapped to stored offsets page by page.
pub struct StrippedStream {
    inner: Box<dyn ReadSeek>,
    full_pages: u64,
    len: u64,
    paged: bool,
}

/// Its lengths; the source is left out.
impl std::fmt::Debug for StrippedStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StrippedStream")
            .field("len", &self.len)
            .field("paged", &self.paged)
            .finish_non_exhaustive()
    }
}

impl StrippedStream {
    /// `inner` holds the stored bytes of `stream_name`, `stored_len` long.
    pub fn new(stream_name: &str, inner: Box<dyn ReadSeek>, stored_len: u64) -> Self {
        let paged = is_checksum_paged_stream(stream_name);
        let full_pages = if paged {
            stored_len / REVIT_STORED_PAGE_BYTES as u64
        } else {
            0
        };
        let len =
            stored_len - full_pages * (REVIT_STORED_PAGE_BYTES - REVIT_PAGE_PAYLOAD_BYTES) as u64;
        Self {
            inner,
            full_pages,
            len,
            paged,
        }
    }

    /// Length of the stripped bytes.
    pub fn len(&self) -> u64 {
        self.len
    }

    /// Whether the stripped stream is empty.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Fill `buf` with the stripped bytes at `at`; an error past the end.
    pub fn read_at(&mut self, at: u64, buf: &mut [u8]) -> std::io::Result<()> {
        use std::io::{Seek, SeekFrom};
        let (page, payload, stored) = (
            REVIT_PAGE_PAYLOAD_BYTES as u64,
            REVIT_PAGE_PAYLOAD_BYTES,
            REVIT_STORED_PAGE_BYTES as u64,
        );
        let mut done = 0usize;
        while done < buf.len() {
            let s = at + done as u64;
            let (raw, run) = if self.paged && s < self.full_pages * page {
                let in_page = (s % page) as usize;
                ((s / page) * stored + in_page as u64, payload - in_page)
            } else {
                let tail = s - self.full_pages * page;
                (self.full_pages * stored + tail, (self.len - s) as usize)
            };
            let n = run.min(buf.len() - done);
            if n == 0 {
                return Err(std::io::ErrorKind::UnexpectedEof.into());
            }
            self.inner.seek(SeekFrom::Start(raw))?;
            self.inner.read_exact(&mut buf[done..done + n])?;
            done += n;
        }
        Ok(())
    }
}

/// Every member of a stream, as [`InflatedStream`] would inflate them.
#[derive(Debug, Default, Clone)]
pub struct MemberIndex {
    pub members: Vec<MemberSpan>,
    /// Where each keyed section starts in the inflated bytes, and its GUID
    /// ([`InflatedStream::sections`]).
    pub sections: Vec<(usize, [u8; 16])>,
    /// Inflated length of the whole stream.
    pub len: usize,
    /// The limit that stopped or capped the read ([`InflatedStream::limit_hit`]).
    pub limit_hit: Option<(&'static str, String)>,
}

/// Read-ahead for one member attempt; doubled while a member runs to its end.
const MEMBER_READ_AHEAD: usize = 256 << 10;
/// A member attempt that stops this close to the end of its read-ahead may
/// have been cut short by it, and is tried again with more.
const MEMBER_SLACK: usize = 64 << 10;

/// Index the members of `src` in one pass, holding one member at a time.
/// Every gzip magic of the stripped bytes is tried, as
/// [`InflatedStream::from_stored_with_limits`] tries them, so the members,
/// their order and the inflated offsets are the ones it concatenates.
/// The `skipped-member` regions are not indexed.
pub fn index_members(
    src: &mut StrippedStream,
    limits: InflateLimits,
) -> std::io::Result<MemberIndex> {
    let len = src.len() as usize;
    let mut index = MemberIndex::default();
    let (mut scan, mut scan_base) = (Vec::new(), 0usize);
    let (mut attempt, mut out) = (Vec::new(), Vec::new());
    let mut i = 0usize;
    while i + 3 <= len {
        // The next magic at or after `i`, as `find_gzip_offsets` steps.
        if i < scan_base || i + 3 > scan_base + scan.len() {
            scan_base = i;
            scan.resize((len - i).min(1 << 20), 0);
            src.read_at(i as u64, &mut scan)?;
        }
        let Some(hit) = memchr::memmem::find(&scan[i - scan_base..], &GZIP_MAGIC).map(|k| i + k)
        else {
            // A magic may straddle the window's end: re-read from its last 2 bytes.
            i = (scan_base + scan.len()).saturating_sub(2).max(i + 1);
            if scan_base + scan.len() >= len {
                break;
            }
            continue;
        };
        i = hit + 3;
        let mut ahead = MEMBER_READ_AHEAD;
        let result = loop {
            let end = (hit + ahead).min(len);
            attempt.resize(end - hit, 0);
            src.read_at(hit as u64, &mut attempt)?;
            out.clear();
            let mut reached = 0;
            let r = inflate_member_into_reporting(&attempt, 0, limits, &mut out, &mut reached);
            if end == len || reached + MEMBER_SLACK <= attempt.len() {
                break r;
            }
            ahead *= 2;
        };
        match result {
            Ok(consumed) => {
                if index.len.saturating_add(out.len()) > limits.max_total_output_bytes {
                    index.limit_hit = Some((
                        INFLATE_MAX_STREAM_BYTES,
                        format!(
                            "members past {} inflated bytes were not read",
                            limits.max_total_output_bytes
                        ),
                    ));
                    break;
                }
                if hit >= 220 {
                    let mut before = [0u8; 220];
                    src.read_at((hit - 220) as u64, &mut before)?;
                    if let Some(guid) = section_guid_before(&before, 220) {
                        index.sections.push((index.len, guid));
                    }
                }
                index.members.push(MemberSpan {
                    offset: hit,
                    stored_len: consumed,
                    inflated_start: index.len,
                    inflated_len: out.len(),
                });
                index.len += out.len();
            }
            Err(Stop::Limit(code, message)) => {
                index.limit_hit.get_or_insert((code, message));
                if code != INFLATE_MAX_MEMBER_BYTES {
                    break;
                }
            }
            Err(Stop::Corrupt(_)) => {}
        }
    }
    Ok(index)
}

/// A partition stream read member by member: its [`MemberIndex`] and the
/// stripped stored bytes to re-inflate any member from.
pub struct StreamedPartition {
    pub name: String,
    pub index: MemberIndex,
    src: StrippedStream,
    limits: InflateLimits,
    /// The last few members re-inflated, for look-ahead reads.
    cache: Vec<(usize, Vec<u8>)>,
    attempt: Vec<u8>,
}

/// Members kept by [`StreamedPartition`]'s look-ahead cache.
const MEMBER_CACHE: usize = 8;

/// Its name and index; the source and cache are left out.
impl std::fmt::Debug for StreamedPartition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamedPartition")
            .field("name", &self.name)
            .field("members", &self.index.members.len())
            .field("len", &self.index.len)
            .finish_non_exhaustive()
    }
}

impl StreamedPartition {
    /// Index `src` ([`index_members`]).
    pub fn new(
        name: &str,
        mut src: StrippedStream,
        limits: InflateLimits,
    ) -> std::io::Result<Self> {
        let index = index_members(&mut src, limits)?;
        Ok(Self {
            name: name.to_string(),
            index,
            src,
            limits,
            cache: Vec::new(),
            attempt: Vec::new(),
        })
    }

    /// Inflated length.
    pub fn len(&self) -> usize {
        self.index.len
    }

    /// Whether the stream inflates to nothing.
    pub fn is_empty(&self) -> bool {
        self.index.len == 0
    }

    /// Append member `k`'s inflated bytes to `out`.
    pub fn read_member(&mut self, k: usize, out: &mut Vec<u8>) -> Result<()> {
        let span = self.index.members[k];
        self.attempt.resize(span.stored_len, 0);
        self.src.read_at(span.offset as u64, &mut self.attempt)?;
        let before = out.len();
        inflate_member_into(&self.attempt, 0, self.limits, out).map_err(Stop::into_error)?;
        if out.len() - before != span.inflated_len {
            return Err(Error::Decompress(format!(
                "{}: member {k} re-inflated to {} bytes, indexed {}",
                self.name,
                out.len() - before,
                span.inflated_len
            )));
        }
        Ok(())
    }

    /// Visit the inflated bytes of `range` in pieces, re-inflating the members
    /// that hold them through a small cache.
    pub fn for_range(&mut self, range: Range<usize>, mut visit: impl FnMut(&[u8])) -> Result<()> {
        let mut at = range.start;
        while at < range.end {
            let k = self
                .index
                .members
                .partition_point(|m| m.inflated_start + m.inflated_len <= at);
            if k >= self.index.members.len() {
                return Err(Error::Decompress(format!(
                    "{}: offset {at} past the end",
                    self.name
                )));
            }
            let slot = match self.cache.iter().position(|(m, _)| *m == k) {
                Some(slot) => slot,
                None => {
                    let mut bytes = if self.cache.len() >= MEMBER_CACHE {
                        let (_, mut old) = self.cache.remove(0);
                        old.clear();
                        old
                    } else {
                        Vec::new()
                    };
                    self.read_member(k, &mut bytes)?;
                    self.cache.push((k, bytes));
                    self.cache.len() - 1
                }
            };
            let start = self.index.members[k].inflated_start;
            let bytes = &self.cache[slot].1;
            let to = range.end.min(start + bytes.len());
            visit(&bytes[at - start..to - start]);
            at = to;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_magic() {
        assert!(has_gzip_magic(&[0x1F, 0x8B, 0x08, 0, 0, 0], 0));
        assert!(!has_gzip_magic(&[0x1F, 0x8B, 0x07], 0));
        assert!(has_gzip_magic(&[0xFF, 0x1F, 0x8B, 0x08], 1));
    }

    #[test]
    fn minimal_gzip_header_len() {
        // magic(3) + method(1) + flags=0(1) + mtime(4) + xfl(1) + os(1) = 10
        let hdr = [0x1F, 0x8B, 0x08, 0x00, 0, 0, 0, 0, 0, 0x0B];
        assert_eq!(gzip_header_len(&hdr, 0), Some(10));
    }

    #[test]
    fn gzip_header_with_fname() {
        // flags = 0x08 (FNAME), followed by "foo\0" after 10-byte base
        let hdr = [
            0x1F, 0x8B, 0x08, 0x08, 0, 0, 0, 0, 0, 0x0B, b'f', b'o', b'o', 0,
        ];
        assert_eq!(gzip_header_len(&hdr, 0), Some(14));
    }

    #[test]
    fn compressed_bomb_rejected_by_inflate_limits() {
        // Build a minimal compressed bomb: truncated-gzip header plus a
        // DEFLATE body that inflates 1 MB of zeros. Then set the inflate
        // cap below 1 MB and assert the call refuses rather than
        // allocating past the limit.
        use flate2::{Compression, write::DeflateEncoder};
        use std::io::Write;
        let payload = vec![0u8; 1024 * 1024]; // 1 MiB of zeros
        let mut bomb = Vec::new();
        bomb.extend_from_slice(&[0x1f, 0x8b, 0x08, 0x00, 0, 0, 0, 0, 0, 0xff]);
        let mut enc = DeflateEncoder::new(&mut bomb, Compression::default());
        enc.write_all(&payload).unwrap();
        enc.finish().unwrap();

        // Bomb ratio is ~1000:1 — a few KB expands to 1 MB. Cap at
        // 64 KB to force rejection; a legit Formats/Latest stream is
        // nowhere near this small-compared-to-header.
        let tight = InflateLimits {
            max_output_bytes: 64 * 1024,
            ..InflateLimits::default()
        };
        let result = inflate_at_with_limits(&bomb, 0, tight);
        match result {
            Err(Error::DecompressLimitExceeded(msg)) => {
                assert!(msg.contains("65536"), "error should name the cap: {msg}");
            }
            other => panic!("expected DecompressLimitExceeded, got {other:?}"),
        }
    }

    // ---: truncated-gzip encoder validation ----

    #[test]
    fn checksum_paged_stream_paths() {
        // Partitions, Global DB streams and Formats/Latest.
        assert!(is_checksum_paged_stream("Formats/Latest"));
        assert!(is_checksum_paged_stream("/Formats/Latest"));
        assert!(is_revit_paged_loader_candidate("Formats/Latest"));
        assert!(is_checksum_paged_stream("Global/Latest"));
        assert!(is_checksum_paged_stream("Global/ElemTable"));
        assert!(is_checksum_paged_stream("Partitions/67"));
        assert!(is_checksum_paged_stream("Partitions/46"));
        assert!(is_checksum_paged_stream("/Partitions/46"));
        assert!(!is_checksum_paged_stream("BasicFileInfo"));
        assert!(!is_checksum_paged_stream("PartAtom"));
        assert!(!is_checksum_paged_stream("ProjectInformation"));
        assert!(!is_checksum_paged_stream("RevitPreview4.0"));
    }

    #[test]
    fn strip_revit_page_checksums_cuts_full_page_trailers() {
        let mut stored = vec![0xABu8; REVIT_PAGE_PAYLOAD_BYTES];
        stored.extend(vec![0xCDu8; REVIT_PAGE_CHECKSUM_BYTES]);
        stored.extend(vec![0xEFu8; 100]); // short final page
        let clean = strip_revit_page_checksums(&stored);
        assert_eq!(clean.len(), REVIT_PAGE_PAYLOAD_BYTES + 100);
        assert!(
            clean
                .iter()
                .take(REVIT_PAGE_PAYLOAD_BYTES)
                .all(|&b| b == 0xAB)
        );
        assert!(
            clean
                .iter()
                .skip(REVIT_PAGE_PAYLOAD_BYTES)
                .all(|&b| b == 0xEF)
        );
        assert!(!clean.contains(&0xCD));
    }

    #[test]
    fn strip_under_one_page_is_identity() {
        let small = vec![1u8, 2, 3, 4, 5];
        assert_eq!(strip_revit_page_checksums(&small), small);
    }

    #[test]
    fn strip_exact_page_boundary_drops_only_trailer() {
        let mut stored = vec![0x11u8; REVIT_PAGE_PAYLOAD_BYTES];
        stored.extend(vec![0x22u8; REVIT_PAGE_CHECKSUM_BYTES]);
        assert_eq!(stored.len(), REVIT_STORED_PAGE_BYTES);
        let clean = strip_revit_page_checksums(&stored);
        assert_eq!(clean, vec![0x11u8; REVIT_PAGE_PAYLOAD_BYTES]);
    }

    #[test]
    fn strip_empty_is_empty() {
        assert!(strip_revit_page_checksums(&[]).is_empty());
    }

    #[test]
    fn strip_two_full_pages_plus_partial() {
        let mut stored = Vec::new();
        for page in 0..2 {
            stored.extend(vec![page as u8; REVIT_PAGE_PAYLOAD_BYTES]);
            stored.extend(vec![0xFFu8; REVIT_PAGE_CHECKSUM_BYTES]);
        }
        stored.extend(vec![0x33u8; 50]);
        let clean = strip_revit_page_checksums(&stored);
        assert_eq!(clean.len(), REVIT_PAGE_PAYLOAD_BYTES * 2 + 50);
        assert!(clean[..REVIT_PAGE_PAYLOAD_BYTES].iter().all(|&b| b == 0));
        assert!(
            clean[REVIT_PAGE_PAYLOAD_BYTES..REVIT_PAGE_PAYLOAD_BYTES * 2]
                .iter()
                .all(|&b| b == 1)
        );
        assert!(
            clean[REVIT_PAGE_PAYLOAD_BYTES * 2..]
                .iter()
                .all(|&b| b == 0x33)
        );
        assert!(!clean.contains(&0xFF));
    }

    #[test]
    fn formats_integrity_garbage_under_one_page_is_incomplete() {
        let garbage = vec![0u8; 64];
        let diag = diagnose_formats_latest_integrity(&garbage);
        assert!(!diag.page_boundary_detected);
        assert_eq!(diag.integrity_status, FormatsIntegrityStatus::Incomplete);
        assert!(diag.inflated_bytes.is_none());
        assert!(diag.schema_scanned_bytes.is_none());
    }
}
