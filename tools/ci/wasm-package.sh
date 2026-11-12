#!/usr/bin/env bash
# Build the browser package: the 32-bit build (rvt_open, 4 GiB), the 64-bit build (rvt_open64, memory64, up to
# 16 GiB) and the loader that picks between them (rvt_open_auto.js), with LICENSE and NOTICE, in one folder.
# CI and the release both build it with this script, so a release ships what CI tested.
#
#   tools/ci/wasm-package.sh <out dir> [--assemble]
#
# With --assemble the two builds are already in <out dir> (CI runs tools/ci/wasm32-build.sh and
# tools/ci/wasm64-build.sh in jobs of their own), and only the rest is done and checked.
#
# Needs the wasm32 target on the default toolchain, wasm-bindgen-cli at the version Cargo.lock pins, node, and
# the nightly that tools/ci/wasm64-build.sh names (it adds rust-src itself).
set -euo pipefail
out=${1:?usage: wasm-package.sh <out dir>}
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
if [[ ${2:-} != --assemble ]]; then
  # The two builds (tools/ci/wasm32-build.sh, tools/ci/wasm64-build.sh) run side by side: each spends
  # much of its time in a single-threaded step (the rvt-open crate at one codegen unit, then the LTO
  # link). The 64-bit one gets a target folder of its own, since cargo builds one at a time in a
  # folder; its output is printed when it ends.
  target=${CARGO_TARGET_DIR:-target}
  log64=$(mktemp)
  CARGO_TARGET_DIR="$target/wasm64" tools/ci/wasm64-build.sh "$out" > "$log64" 2>&1 &
  pid64=$!
  trap 'kill "$pid64" 2> /dev/null' EXIT
  tools/ci/wasm32-build.sh "$out"
  rc64=0; wait "$pid64" || rc64=$?
  trap - EXIT
  cat "$log64"; rm -f "$log64"
  [[ $rc64 == 0 ]] || { echo "FAIL: the wasm64 build exited $rc64" >&2; exit 1; }
fi
cp LICENSE NOTICE "$out/"
for f in rvt_open.js rvt_open_bg.wasm rvt_open64.js rvt_open64_bg.wasm rvt_open_auto.js LICENSE NOTICE; do
    [[ -s $out/$f ]] || { echo "FAIL: $out/$f missing" >&2; exit 1; }
done
echo "wasm package ok: $out"
