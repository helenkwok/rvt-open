//! no fixed size cap on legitimate data, a ratio guard against
//! decompression bombs (Revit itself reads multi-GB models).
//!
//! 1. A partition-shaped stream of three gzip members, 300, 400 and 400 MiB
//!    inflated (1.1 GiB in all) from compressible data of a corpus-like
//!    ratio, is read whole: past the old 256 MiB per-member cap and the old
//!    1 GiB per-stream cap, which skipped those members without a word.
//!    It is read whole both held ([`InflatedStream`]) and member by member
//!    ([`for_each_member`]).
//! 2. A 200 MiB member of zeros (~220:1 from this encoder) is refused as a decompression
//!    bomb with an error that says so. The old 256 MiB cap let it through.
//!
//! Synthetic by nature: no real file can be both this large and shareable.

use flate2::{Compression, write::GzEncoder};
use rvt::compression::{
    INFLATE_RATIO, InflateLimits, InflatedStream, REVIT_PAGE_CHECKSUM_BYTES,
    REVIT_PAGE_PAYLOAD_BYTES, for_each_member, inflate_member_at_with_limits,
};
use std::io::Write;

const MIB: usize = 1024 * 1024;

/// One gzip member of `len` bytes from `fill`, written in 1 MiB pieces so
/// the inflated data is never held here.
fn member(len: usize, mut fill: impl FnMut(&mut [u8])) -> Vec<u8> {
    let mut gz = GzEncoder::new(Vec::new(), Compression::fast());
    let mut block = vec![0u8; MIB];
    let mut left = len;
    while left > 0 {
        let n = left.min(MIB);
        fill(&mut block[..n]);
        gz.write_all(&block[..n]).expect("compress");
        left -= n;
    }
    gz.finish().expect("compress")
}

/// `payload` as Revit stores a `Partitions/*` stream: a checksum trailer
/// after every full page, the last short page as is. `Partitions/*` names
/// are stripped of those trailers before inflating, so unpaged bytes under
fn paged(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(
        payload.len() + payload.len() / REVIT_PAGE_PAYLOAD_BYTES * REVIT_PAGE_CHECKSUM_BYTES,
    );
    let mut pages = payload.chunks_exact(REVIT_PAGE_PAYLOAD_BYTES);
    for page in &mut pages {
        out.extend_from_slice(page);
        out.extend(std::iter::repeat_n(0xA5u8, REVIT_PAGE_CHECKSUM_BYTES));
    }
    out.extend_from_slice(pages.remainder());
    out
}

/// Corpus-like data: 16 pseudo-random bytes then 48 repeating ones, about
/// 4:1 under DEFLATE (the corpus's streams run near 5:1).
fn corpus_like(seed: u64) -> impl FnMut(&mut [u8]) {
    let mut x = seed | 1;
    let mut pos = 0usize;
    move |buf: &mut [u8]| {
        for b in buf {
            *b = if pos % 64 < 16 {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                x as u8
            } else {
                (pos % 64) as u8
            };
            pos += 1;
        }
    }
}

#[test]
fn a_stream_past_every_old_cap_is_read_whole() {
    let sizes = [300 * MIB, 400 * MIB, 400 * MIB];
    let mut stored = Vec::new();
    for (i, len) in sizes.iter().enumerate() {
        stored.extend(member(*len, corpus_like(i as u64 + 7)));
    }
    let total: usize = sizes.iter().sum();
    let ratio = total as f64 / stored.len() as f64;
    eprintln!(
        "stored {} MiB, inflated {} MiB, ratio {ratio:.1}:1",
        stored.len() / MIB,
        total / MIB
    );
    assert!(ratio < 50.0, "the data is corpus-like, not a bomb");

    let stored = paged(&stored);
    let held = InflatedStream::from_stored("Partitions/1", &stored);
    assert_eq!(held.limit_hit(), None);
    assert_eq!(held.chunk_count(), 3, "every member is read");
    assert_eq!(held.bytes().len(), total, "the stream is read whole");
    // The last member's last bytes are the generator's, not zeros.
    let mut tail = vec![0u8; MIB];
    let mut gen_last = corpus_like(9);
    let mut left = sizes[2];
    while left > 0 {
        let n = left.min(MIB);
        gen_last(&mut tail[..n]);
        left -= n;
    }
    let last_len = sizes[2] % MIB;
    let last_len = if last_len == 0 { MIB } else { last_len };
    assert_eq!(&held.bytes()[total - last_len..], &tail[..last_len]);
    drop(held);

    let mut seen = 0usize;
    let mut largest_buffer = 0usize;
    let (index, hit, skipped) = for_each_member(
        "Partitions/1",
        &stored,
        InflateLimits::default(),
        |span, bytes| {
            assert_eq!(span.inflated_start, seen);
            assert_eq!(span.inflated_len, bytes.len());
            seen += bytes.len();
            largest_buffer = largest_buffer.max(bytes.len());
            true
        },
    );
    assert_eq!(hit, None);
    assert_eq!(skipped, None, "no member is skipped");
    assert_eq!(index.len(), 3);
    assert_eq!(seen, total, "member by member, the stream is read whole");
    // Bounded: the buffer handed out is one member, never the stream.
    assert_eq!(
        largest_buffer,
        400 * MIB,
        "the largest buffer is the largest member"
    );
}

#[test]
fn a_decompression_bomb_is_refused_with_an_error() {
    let bomb = member(200 * MIB, |buf| buf.fill(0));
    eprintln!("bomb: {} KiB inflating to 200 MiB", bomb.len() / 1024);

    let err = inflate_member_at_with_limits(&bomb, 0, InflateLimits::default())
        .expect_err("a ~220:1 member is refused");
    assert!(
        matches!(err, rvt::Error::DecompressLimitExceeded(_))
            && err.to_string().contains("decompression bomb"),
        "{err}"
    );

    let held = InflatedStream::from_stored("Partitions/1", &paged(&bomb));
    let (code, message) = held.limit_hit().expect("the stream records the refusal");
    assert_eq!(code, INFLATE_RATIO, "{message}");
    assert!(held.bytes().is_empty(), "nothing of the bomb is kept");
}

/// A capped container that has just read a few GB of models: usage is
/// 3 MiB under the 12 GiB limit, but 7 GiB of it is page cache the kernel
/// reclaims before failing an allocation. Counting only limit - usage (the
/// gate's spurious "needs 21 MiB but only 3 MiB is available") refuses real
/// models under parallel tests and drops their data.
#[test]
fn page_cache_counts_as_available_memory() {
    const GIB: u64 = 1 << 30;
    let meminfo = format!(
        "MemTotal: {} kB\nMemAvailable: {} kB\n",
        64 * GIB / 1024,
        20 * GIB / 1024
    );
    let max = (12 * GIB).to_string();
    let current = (12 * GIB - 3 * MIB as u64).to_string();
    let stat = format!(
        "anon 4000000000\nactive_file {}\ninactive_file {}\nslab 1000\n",
        GIB,
        6 * GIB
    );
    let avail =
        rvt::memory::available_from(Some(&meminfo), Some(&max), Some(&current), Some(&stat))
            .expect("known");
    assert_eq!(
        avail,
        7 * GIB + 3 * MIB as u64,
        "headroom + reclaimable file pages"
    );

    // cgroup v1 names, same figures.
    let v1 = stat
        .replace("inactive_file", "total_inactive_file")
        .replace("\nactive_file", "\ntotal_active_file");
    assert_eq!(
        rvt::memory::available_from(Some(&meminfo), Some(&max), Some(&current), Some(&v1)),
        Some(avail)
    );
    // No limit ("max", or v1's huge sentinel): the host figure.
    assert_eq!(
        rvt::memory::available_from(Some(&meminfo), Some("max\n"), Some(&current), Some(&stat)),
        Some(20 * GIB)
    );
    assert_eq!(
        rvt::memory::available_from(
            Some(&meminfo),
            Some("9223372036854771712"),
            Some(&current),
            Some(&stat)
        ),
        Some(20 * GIB)
    );
    // No cgroup files at all.
    assert_eq!(
        rvt::memory::available_from(Some(&meminfo), None, None, None),
        Some(20 * GIB)
    );
    // The host is the tighter of the two.
    let tight = format!("MemAvailable: {} kB\n", GIB / 1024);
    assert_eq!(
        rvt::memory::available_from(Some(&tight), Some(&max), Some(&current), Some(&stat)),
        Some(GIB)
    );
}
