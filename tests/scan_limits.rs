//! a resource limit never cuts a read short without saying so, and
//! the default limits read every stream of the corpus whole.
//!
//! 1. The ArcWall scan reads a partition stream member by member (one gzip
//!    member and a record's length before it held, not the stream) and finds
//!    the same walls at the same offsets as a scan of the whole inflated
//!    stream, on the Revit 2023 files of `RVT_REVIT_EXTRA_DIR` (ArcWall
//!    records are read on 2023 only).
//! 2. An export whose walker scan cap is lower than `Global/Latest` reports
//!    it: a `resource-limit:` warning and a non-zero
//!    `confidence.resource_limit_hits`.
//! 3. With the default limits, the same files export with no limit hit, and
//!    the production walk yields the same elements as an unlimited scan.
//!
//! Each check skips the files that are absent.

use rvt::RevitFile;
use rvt::ifc::RvtDocExporter;
use rvt::research::partition_arc_walls::scan_partition_arc_walls_with_limits;
use rvt::walker::{PRODUCTION_ELEMENT_MIN_SCORE, WalkerLimits, iter_elements_with_limits};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// The Revit 2023 files of `RVT_REVIT_EXTRA_DIR`, if it is set.
fn files_2023() -> Vec<PathBuf> {
    let Some(dir) = std::env::var_os("RVT_REVIT_EXTRA_DIR").map(PathBuf::from) else {
        eprintln!("skipping: RVT_REVIT_EXTRA_DIR is not set");
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir.join("2023")) else {
        eprintln!("skipping: no 2023 folder in RVT_REVIT_EXTRA_DIR");
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "rvt"))
        .collect();
    files.sort();
    files
}

/// Each ArcWall as (partition, offset, ElementId).
fn arc_walls(rf: &mut RevitFile) -> Vec<(String, usize, Option<u32>)> {
    scan_partition_arc_walls_with_limits(rf, 2023, WalkerLimits::default())
        .expect("ArcWall scan")
        .walls
        .into_iter()
        .map(|w| {
            (
                w.partition,
                w.offset,
                w.trailer
                    .and_then(|t| t.element_id)
                    .and_then(|id| id.as_u32()),
            )
        })
        .collect()
}

#[test]
fn member_by_member_arc_walls_are_the_whole_stream_ones() {
    let mut checked = 0;
    for path in files_2023() {
        // Nothing inflated yet: every partition is read member by member.
        let mut rf = RevitFile::open(&path).expect("open");
        let streamed = arc_walls(&mut rf);
        // Every partition inflated whole first: the scan reads the buffers.
        let mut rf = RevitFile::open(&path).expect("open");
        for name in rf.partition_stream_names() {
            let _ = rf.inflated_partition(&name);
        }
        let held = arc_walls(&mut rf);
        assert_eq!(streamed, held, "{}", path.display());
        checked += usize::from(!held.is_empty());
    }
    eprintln!("{checked} file(s) with ArcWalls compared");
}

#[test]
fn an_export_reports_a_scan_cap_it_hits() {
    let Some(path) = files_2023().into_iter().next() else {
        return;
    };
    let mut rf = RevitFile::open(&path).expect("open");
    let limits = WalkerLimits {
        max_scan_bytes: 64 * 1024,
        ..WalkerLimits::default()
    };
    let result = RvtDocExporter
        .export_with_diagnostics_and_limits(&mut rf, limits)
        .expect("export");
    let d = &result.diagnostics;
    assert!(
        d.confidence.resource_limit_hits > 0,
        "{}: a 64 KiB scan cap is reported",
        path.display()
    );
    assert!(
        d.warnings
            .iter()
            .any(|w| w.starts_with("resource-limit: walker-max-scan-bytes: ")),
        "{}: warnings name the cap: {:?}",
        path.display(),
        d.warnings
    );
}

fn default_limits_read_every_stream_whole(k: usize, n: usize) {
    for path in crate::shard(files_2023(), k, n) {
        let mut rf = RevitFile::open(&path).expect("open");
        let result = RvtDocExporter
            .export_with_diagnostics(&mut rf)
            .expect("export");
        assert_eq!(
            result.diagnostics.confidence.resource_limit_hits,
            0,
            "{}: {:?}",
            path.display(),
            rf.limit_events()
        );
        let counts = |limits: WalkerLimits| {
            let mut rf = RevitFile::open(&path).expect("open");
            let mut out = BTreeMap::<String, usize>::new();
            for el in iter_elements_with_limits(&mut rf, PRODUCTION_ELEMENT_MIN_SCORE, limits)
                .expect("walk")
            {
                *out.entry(el.class).or_default() += 1;
            }
            out
        };
        let unlimited = WalkerLimits {
            max_scan_bytes: usize::MAX,
            ..WalkerLimits::default()
        };
        assert_eq!(
            counts(WalkerLimits::default()),
            counts(unlimited),
            "{}: the default walk is the unlimited walk",
            path.display()
        );
    }
}

split!(default_limits_read_every_stream_whole, default_limits_read_every_stream_whole, s0 0, s1 1, s2 2, s3 3, s4 4, s5 5, s6 6, s7 7, s8 8, s9 9);
