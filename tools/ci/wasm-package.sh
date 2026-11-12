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
# The two builds run side by side: each spends most of its time in a single-threaded step (the
# rvt-open crate at one codegen unit, then the LTO link). The 64-bit one gets a target folder of its
# own, since cargo builds one at a time in a folder; its output is printed when it ends.
target=${CARGO_TARGET_DIR:-target}
log64=$(mktemp)
CARGO_TARGET_DIR="$target/wasm64" tools/ci/wasm64-build.sh "$out" > "$log64" 2>&1 &
pid64=$!
trap 'kill "$pid64" 2> /dev/null' EXIT
cargo build -p rvt-wasm --release --locked --target wasm32-unknown-unknown
wasm-bindgen --target web --out-dir "$out" --out-name rvt_open \
    "$target/wasm32-unknown-unknown/release/rvt_wasm.wasm"
rc64=0; wait "$pid64" || rc64=$?
trap - EXIT
cat "$log64"; rm -f "$log64"
[[ $rc64 == 0 ]] || { echo "FAIL: the wasm64 build exited $rc64" >&2; exit 1; }
cp LICENSE NOTICE "$out/"
for f in rvt_open.js rvt_open_bg.wasm rvt_open64.js rvt_open64_bg.wasm rvt_open_auto.js LICENSE NOTICE; do
    [[ -s $out/$f ]] || { echo "FAIL: $out/$f missing" >&2; exit 1; }
done
echo "wasm package ok: $out"
