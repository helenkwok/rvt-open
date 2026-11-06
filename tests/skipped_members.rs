//! a gzip member that does not inflate is never skipped without a
//! word. A corrupt or truncated member gives a `skipped-member:` diagnostic,
//! the same text from [`InflatedStream`] and [`for_each_member`]; a gzip
//! magic inside another member's compressed bytes that fails to inflate is a
//! false hit and gives none.

use flate2::{Compression, write::GzEncoder};
use rvt::compression::{InflateLimits, InflatedStream, SKIPPED_MEMBER, for_each_member};
use std::io::Write;

/// Not a `Partitions/*` name, so no page trailers are stripped.
const NAME: &str = "Test/Stream";

fn member(payload: &[u8], level: Compression) -> Vec<u8> {
    let mut gz = GzEncoder::new(Vec::new(), level);
    gz.write_all(payload).expect("compress");
    gz.finish().expect("compress")
}

fn payload(seed: u8) -> Vec<u8> {
    (0..4096u32).map(|i| (i as u8).wrapping_mul(seed)).collect()
}

/// Both readers' diagnostic and the member count the held reader kept.
fn read(stored: &[u8]) -> (Option<String>, Option<String>, usize) {
    let held = InflatedStream::from_stored(NAME, stored);
    let (_, hit, walked) = for_each_member(NAME, stored, InflateLimits::default(), |_, _| true);
    assert_eq!(hit, None);
    assert_eq!(held.limit_hit(), None);
    (
        held.skipped_member().map(str::to_string),
        walked,
        held.chunk_count(),
    )
}

#[test]
fn clean_members_give_no_diagnostic() {
    let mut stored = member(&payload(3), Compression::default());
    stored.extend(member(&payload(5), Compression::default()));
    let (held, walked, n) = read(&stored);
    assert_eq!((held, walked, n), (None, None, 2));
}

#[test]
fn a_false_magic_inside_a_member_gives_no_diagnostic() {
    // A stored (level 0) DEFLATE block keeps the payload verbatim, so a gzip
    // magic in it sits inside the member's compressed bytes. What follows the
    // fake header is 0xFF: a reserved DEFLATE block type, so it fails.
    let mut fake = vec![0x1f, 0x8b, 0x08, 0, 0, 0, 0, 0, 0, 0xff];
    fake.extend([0xffu8; 64]);
    let mut body = payload(7);
    body.extend(&fake);
    body.extend(payload(9));
    let stored = member(&body, Compression::none());
    let (held, walked, n) = read(&stored);
    assert_eq!((held, walked, n), (None, None, 1));
}

#[test]
fn a_corrupt_member_between_good_ones_is_reported() {
    let first = member(&payload(3), Compression::default());
    let mut bad = member(&payload(5), Compression::default());
    // Past the 10-byte header: a reserved block type.
    for b in &mut bad[10..20] {
        *b = 0xff;
    }
    let third = member(&payload(11), Compression::default());
    let mut stored = first.clone();
    stored.extend(&bad);
    stored.extend(&third);
    let (held, walked, n) = read(&stored);
    assert_eq!(n, 2, "the good members are still read");
    let held = held.expect("the corrupt member is reported");
    assert_eq!(Some(held.clone()), walked, "both readers say the same");
    assert!(held.starts_with(&format!("{SKIPPED_MEMBER}: {NAME}: 1 region(s)")));
    // The region runs from the first member's DEFLATE end (its 8-byte
    // trailer included) to the third member's header.
    let end = first.len() - 8;
    let next = first.len() + bad.len();
    assert!(held.contains(&format!("(at {end}..{next})")), "{held}");
}

#[test]
fn a_truncated_last_member_is_reported_to_the_stream_end() {
    let first = member(&payload(3), Compression::default());
    let second = member(&payload(5), Compression::default());
    let mut stored = first.clone();
    stored.extend(&second[..second.len() / 2]);
    let (held, walked, n) = read(&stored);
    assert_eq!(n, 1);
    let held = held.expect("the truncated member is reported");
    assert_eq!(Some(held.clone()), walked);
    let end = first.len() - 8;
    assert!(
        held.contains(&format!("(at {end}..{})", stored.len())),
        "{held}"
    );
}

#[test]
fn a_stream_where_no_member_inflates_is_reported_whole() {
    let mut bad = member(&payload(5), Compression::default());
    for b in &mut bad[10..20] {
        *b = 0xff;
    }
    let (held, walked, n) = read(&bad);
    assert_eq!(n, 0);
    let held = held.expect("an all-corrupt stream is reported");
    assert_eq!(Some(held.clone()), walked);
    assert!(held.contains(&format!("(at 0..{})", bad.len())), "{held}");
}
