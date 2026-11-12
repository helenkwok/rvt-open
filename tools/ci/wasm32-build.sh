#!/usr/bin/env bash
# Build the 32-bit browser module (rvt_open, 4 GiB) into a folder: one half of tools/ci/wasm-package.sh,
# which CI also runs in a job of its own beside tools/ci/wasm64-build.sh.
#
#   tools/ci/wasm32-build.sh <out dir>
#
# Needs the wasm32 target on the default toolchain and wasm-bindgen-cli at the version Cargo.lock pins.
set -euo pipefail
out=${1:?usage: wasm32-build.sh <out dir>}
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
cargo build -p rvt-wasm --release --locked --target wasm32-unknown-unknown
wasm-bindgen --target web --out-dir "$out" --out-name rvt_open \
    "${CARGO_TARGET_DIR:-target}/wasm32-unknown-unknown/release/rvt_wasm.wasm"
