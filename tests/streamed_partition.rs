//! a partition read member by member gives the same members, the
//! same inflated offsets and sections, the same object frames and gaps, and
//! the same schema objects as the whole-stream read.
//!
//! Runs against `RVT_AUTODESK_SAMPLES_DIR` (Autodesk's sample projects by
//! release, one folder per year): rac_basic 2023 (narrow ids) and 2024 (wide
//! ids). Skips what is absent.

use rvt::RevitFile;
use rvt::schema_objects::{
    IdWidth, STREAM_AHEAD, for_each_object, for_each_object_streamed, frames, frames_streamed,
};
use std::path::PathBuf;

fn sample(release: u32, name: &str) -> Option<PathBuf> {
    let root = PathBuf::from(std::env::var_os("RVT_AUTODESK_SAMPLES_DIR")?);
    let path = root.join(release.to_string()).join(format!("{name}.rvt"));
    if path.exists() {
        Some(path)
    } else {
        eprintln!("skipping: {} absent", path.display());
        None
    }
}

fn check(release: u32) {
    let Some(path) = sample(release, "rac_basic_sample_project") else {
        return;
    };
    let width = IdWidth::of_release(release);
    let mut rf = RevitFile::open(&path).expect("open");
    let names: Vec<String> = rf
        .stream_names()
        .into_iter()
        .filter(|n| n.starts_with("Partitions/"))
        .collect();
    assert!(!names.is_empty());
    for name in &names {
        let whole = rf.inflated_partition(name).expect("inflate");
        let (want, want_gaps) = frames(whole.bytes(), width);
        assert!(want.len() > 100_000, "{name}: {} frames", want.len());
        for ahead in [1 << 20, STREAM_AHEAD] {
            let mut part = rf.streamed_partition(name).expect("index");
            assert_eq!(part.len(), whole.bytes().len(), "{name} length");
            assert_eq!(
                part.index.members.len(),
                whole.chunk_count(),
                "{name} members"
            );
            assert_eq!(part.index.sections, whole.sections(), "{name} sections");
            let mut got = Vec::with_capacity(want.len());
            let gaps = frames_streamed(&mut part, width, ahead, |f, window, base| {
                let mut f = f.clone();
                assert_eq!(
                    &window[f.span.clone()],
                    &whole.bytes()[base + f.span.start..base + f.span.end]
                );
                f.span = base + f.span.start..base + f.span.end;
                f.payload = base + f.payload.start..base + f.payload.end;
                got.push(f);
            })
            .expect("scan");
            assert!(got == want, "{name}: frames differ at look-ahead {ahead}");
            assert_eq!(gaps, want_gaps, "{name}: gaps at look-ahead {ahead}");
        }
    }
    // The schema objects themselves, with stream spans, in the same order
    // (compared by a hash of each whole object, so neither list is held).
    let key = |o: &rvt::schema_objects::SchemaObject| {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        format!("{o:?}").hash(&mut h);
        h.finish()
    };
    let mut whole = Vec::new();
    let whole_gaps = for_each_object(&mut rf, |o| whole.push(key(o))).expect("whole");
    let mut i = 0;
    let streamed_gaps = for_each_object_streamed(&mut rf, |o| {
        assert_eq!(key(o), whole[i], "object {i} differs");
        i += 1;
    })
    .expect("streamed");
    assert_eq!(i, whole.len());
    assert_eq!(streamed_gaps, whole_gaps);
}

#[test]
fn rac_basic_2023_streamed_matches_whole() {
    check(2023);
}

#[test]
fn rac_basic_2024_streamed_matches_whole() {
    check(2024);
}
