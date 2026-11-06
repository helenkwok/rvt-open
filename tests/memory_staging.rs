//! source (read on demand, never whole in memory) reads and converts exactly
//! as the same bytes in memory, and saved graphics attached in batches give
//! the same model as one scene.

use rvt::RevitFile;
use rvt::reader::{OpenLimits, ReadAt, ReadAtSource};

const FILE: &str = "corpus/tier1/architectural-2024/architectural-2024.rvt";

/// Serves at most 777 bytes per call, so the window fill loops.
struct Chunky(Vec<u8>);

impl ReadAt for Chunky {
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> std::io::Result<usize> {
        let start = (offset as usize).min(self.0.len());
        let n = buf.len().min(777).min(self.0.len() - start);
        buf[..n].copy_from_slice(&self.0[start..start + n]);
        Ok(n)
    }
}

fn streamed(bytes: &[u8]) -> RevitFile {
    let source = ReadAtSource::new(Chunky(bytes.to_vec()), bytes.len() as u64);
    RevitFile::open_reader(Box::new(source), bytes.len() as u64, OpenLimits::default()).unwrap()
}

#[test]
fn streams_read_identically() {
    let bytes = std::fs::read(FILE).unwrap();
    let mut whole = RevitFile::open_bytes(bytes.clone()).unwrap();
    let mut part = streamed(&bytes);
    let names = whole.stream_names();
    assert_eq!(names, part.stream_names());
    for name in &names {
        assert_eq!(
            whole.read_stream(name).ok(),
            part.read_stream(name).ok(),
            "{name}"
        );
    }
}

#[test]
fn export_is_byte_identical() {
    let bytes = std::fs::read(FILE).unwrap();
    let opts = rvt::convert::ConvertOptions::default();
    let step = |rf: &mut RevitFile| {
        let exported = rvt::convert::export(rf, &opts).unwrap();
        let graph = rvt::convert::graph(&exported.model, &opts, true);
        rvt::convert::ifc4_step(&graph, Some(0))
    };
    let mut whole = RevitFile::open_bytes(bytes.clone()).unwrap();
    assert_eq!(step(&mut whole), step(&mut streamed(&bytes)));
}

#[test]
fn not_a_cfb_and_too_large() {
    let tiny = ReadAtSource::new(Chunky(b"nope".to_vec()), 4);
    assert!(matches!(
        RevitFile::open_reader(Box::new(tiny), 4, OpenLimits::default()),
        Err(rvt::Error::NotACfbFile)
    ));
    let limits = OpenLimits {
        max_file_bytes: 1024,
        ..OpenLimits::default()
    };
    let big = ReadAtSource::new(Chunky(vec![0; 2048]), 2048);
    assert!(matches!(
        RevitFile::open_reader(Box::new(big), 2048, limits),
        Err(rvt::Error::TooLarge { .. })
    ));
}

/// Saved meshes and materials attached in three batches give the same
/// model as one scene (runs against `RVT_AUTODESK_SAMPLES_DIR`, skips when
/// absent).
#[test]
fn saved_graphics_in_batches_equal_one_scene() {
    use rvt::ifc::saved_meshes::{self, SavedMeshScope};
    let Some(dir) = std::env::var_os("RVT_AUTODESK_SAMPLES_DIR") else {
        eprintln!("skipping: RVT_AUTODESK_SAMPLES_DIR is not set");
        return;
    };
    let path = std::path::PathBuf::from(dir).join("2024/rac_basic_sample_project.rvt");
    if !path.exists() {
        eprintln!("skipping: rac_basic 2024 absent");
        return;
    }
    let run = |batch: usize| {
        let mut rf = RevitFile::open(&path).unwrap();
        let mut model = rvt::ifc::RvtDocExporter
            .export_with_diagnostics(&mut rf)
            .unwrap()
            .model;
        let control = rvt::control::WalkerControl::default();
        let report = saved_meshes::attach_scoped_in_batches(
            &mut rf,
            &mut model,
            SavedMeshScope::All,
            &control,
            batch,
        )
        .unwrap();
        let set = saved_meshes::attach_materials_in_batches(&mut rf, &mut model, batch).unwrap();
        (report, set, format!("{model:?}"))
    };
    let whole = run(usize::MAX);
    assert!(whole.0.candidates > 3, "{:?}", whole.0);
    // Three batches: each runs every extraction pass again, so more is slow.
    let batched = run(whole.0.candidates.div_ceil(3));
    assert_eq!((&whole.0, whole.1), (&batched.0, batched.1));
    assert!(whole.2 == batched.2, "models differ");
}
