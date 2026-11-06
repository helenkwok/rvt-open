#![no_main]

//! Fuzz target for `rvt::partitions::find_chunks`.
//!
//! `find_chunks` scans a raw `Partitions/NN` byte slice for gzip magic
//! triplets `1F 8B 08` and returns `(offset, length)` ranges for each
//! chunk boundary it detects. Bomb-rejection limits sit in front of this
//! function, and its scan range once had an off-by-one, so we fuzz it with arbitrary input to harden the
//! chunk-discovery loop against adversarial or malformed streams.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = rvt::partitions::find_chunks(data);
});
