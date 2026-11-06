//! a real CFB file whose `Partitions/NN`
//! streams cross the old limits (256 MiB stored per stream, 1 GiB inflated)
//! opens and reads whole through `RevitFile`, with no resource-limit or
//! skip event. Run: `cargo test --release --test large_partition_file -- --ignored --nocapture`
//! Synthetic by nature: no shareable real file is this large.

use flate2::{Compression, write::GzEncoder};
use rvt::compression::{REVIT_PAGE_CHECKSUM_BYTES, REVIT_PAGE_PAYLOAD_BYTES};
use std::io::Write;

const MIB: usize = 1024 * 1024;

fn member(len: usize, random: bool, seed: u64) -> Vec<u8> {
    let mut gz = GzEncoder::new(Vec::new(), Compression::fast());
    let mut x = seed | 1;
    let mut block = vec![0u8; MIB];
    let mut left = len;
    while left > 0 {
        let n = left.min(MIB);
        for (i, b) in block[..n].iter_mut().enumerate() {
            *b = if random || i % 64 < 16 {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                x as u8
            } else {
                (i % 64) as u8
            };
        }
        gz.write_all(&block[..n]).unwrap();
        left -= n;
    }
    gz.finish().unwrap()
}

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

fn peak_rss_mib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmHWM:"))
                .map(str::to_string)
        })
        .and_then(|l| {
            l.split_whitespace()
                .nth(1)
                .and_then(|v| v.parse::<u64>().ok())
        })
        .map_or(0, |kb| kb / 1024)
}

#[test]
#[ignore = "writes a ~1.7 GiB file; run on a VM"]
fn partitions_past_the_old_limits_open_and_read_whole() {
    // Partition 67: 3 x 110 MiB random members, ~330 MiB stored (> 256 MiB).
    let mut p67 = Vec::new();
    for i in 0..3 {
        p67.extend(member(110 * MIB, true, i + 3));
    }
    let p67 = paged(&p67);
    assert!(p67.len() > 256 * MIB, "stored {} MiB", p67.len() / MIB);
    // Partition 66: 3 x 400 MiB compressible, 1.2 GiB inflated (> 1 GiB).
    let mut p66 = Vec::new();
    for i in 0..3 {
        p66.extend(member(400 * MIB, false, i + 11));
    }
    let p66 = paged(&p66);

    let dir = std::env::var("RVT_LARGE_PARTITION_DIR")
        .unwrap_or_else(|_| std::env::temp_dir().display().to_string());
    let path = std::path::Path::new(&dir).join("large-partitions.cfb");
    {
        let f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        let mut cf = cfb::CompoundFile::create(f).unwrap();
        cf.create_storage("/Partitions").unwrap();
        for (n, d) in [("/Partitions/67", &p67), ("/Partitions/66", &p66)] {
            let mut s = cf.create_stream(n).unwrap();
            s.write_all(d).unwrap();
        }
        cf.flush().unwrap();
    }
    eprintln!(
        "file {} MiB, p67 stored {} MiB, p66 stored {} MiB",
        std::fs::metadata(&path).unwrap().len() / MIB as u64,
        p67.len() / MIB,
        p66.len() / MIB
    );

    let mut rf = rvt::RevitFile::open(&path).expect("file opens");
    let raw = rf
        .read_stream("Partitions/67")
        .expect("stream over 256 MiB reads");
    assert_eq!(raw.len(), p67.len(), "stored stream read whole");
    drop(raw);
    let inf67 = rf.inflated_partition("Partitions/67").unwrap();
    assert_eq!(inf67.limit_hit(), None);
    assert_eq!(inf67.bytes().len(), 330 * MIB);
    let inf66 = rf.inflated_partition("Partitions/66").unwrap();
    assert_eq!(inf66.limit_hit(), None);
    assert_eq!(inf66.bytes().len(), 1200 * MIB, "past the old 1 GiB total");
    assert!(rf.limit_events().is_empty(), "{:?}", rf.limit_events());
    assert!(rf.skip_events().is_empty(), "{:?}", rf.skip_events());
    eprintln!(
        "PEAK-RSS-MIB {} (file {} MiB, inflated {} MiB)",
        peak_rss_mib(),
        std::fs::metadata(&path).unwrap().len() / MIB as u64,
        1530
    );
    if std::env::var("RVT_LARGE_PARTITION_KEEP").is_err() {
        let _ = std::fs::remove_file(&path);
    }
}
