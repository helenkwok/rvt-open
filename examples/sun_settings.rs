//! Prints each file's sun settings and view display options (`rvt::sun_settings`) as one JSON
//! line: `{ file, release, ms, sun }`, `sun` null where none was read, or `{ file, error }`.
//!
//! ```sh
//! cargo run --example sun_settings -- a.rvt b.rvt > sun.jsonl
//! ```
fn main() {
    for path in std::env::args().skip(1) {
        let t = std::time::Instant::now();
        let line = match rvt::RevitFile::open(&path) {
            Ok(mut rf) => {
                let release = rf.basic_file_info().map(|b| b.version).unwrap_or(0);
                let sun = rvt::sun_settings::read(&mut rf);
                serde_json::json!({
                    "file": path,
                    "release": release,
                    "ms": t.elapsed().as_millis() as u64,
                    "sun": sun,
                })
            }
            Err(e) => serde_json::json!({ "file": path, "error": e.to_string() }),
        };
        println!("{line}");
    }
}
