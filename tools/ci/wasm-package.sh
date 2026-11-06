#!/usr/bin/env bash
# Build the browser package: the 32-bit build (rvt_open, 4 GiB), the 64-bit build (rvt_open64, memory64, up to
# 16 GiB) and the loader that picks between them (rvt_open_auto.js), with LICENSE and NOTICE, in one folder.
# CI and the release both build it with this script, so a release ships what CI tested.
#
#   tools/ci/wasm-package.sh <out dir>
#
# Needs the wasm32 target on the default toolchain, wasm-bindgen-cli at the version Cargo.lock pins, node, and
# the nightly that tools/ci/wasm64-build.sh names (it adds rust-src itself).
set -euo pipefail
out=${1:?usage: wasm-package.sh <out dir>}
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
cargo build -p rvt-wasm --release --locked --target wasm32-unknown-unknown
wasm-bindgen --target web --out-dir "$out" --out-name rvt_open \
    "${CARGO_TARGET_DIR:-target}/wasm32-unknown-unknown/release/rvt_wasm.wasm"
tools/ci/wasm64-build.sh "$out"
cp LICENSE NOTICE "$out/"
for f in rvt_open.js rvt_open_bg.wasm rvt_open64.js rvt_open64_bg.wasm rvt_open_auto.js LICENSE NOTICE; do
    [[ -s $out/$f ]] || { echo "FAIL: $out/$f missing" >&2; exit 1; }
done
echo "wasm package ok: $out"
