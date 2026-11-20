#!/usr/bin/env bash
# Build the Python bindings' wheel for Pyodide (wasm32-unknown-emscripten): the browser has no install step, this wheel is
# what the exported notebook site installs.
# Usage: tools/ci/pyodide/build-wheel.sh OUT_DIR [EMSDK_DIR]
# Needs, on PATH: the Rust toolchain with the wasm32-unknown-emscripten target, and a Python 3.14 environment with
# tools/ci/pyodide/requirements.txt installed (pyodide-build, maturin).
# Pins: emsdk 5.0.3 (the emscripten the Pyodide 314 cross-build environment is built with; the script stops on any other),
# by the commit its tag points at.
set -euo pipefail
out=${1:?out dir}; emsdk=${2:-$HOME/emsdk}
EMSDK_REF=a620cf1d71c62dfdfbb0c01fe0a371e2af2dda6c # emscripten-core/emsdk, tag 5.0.3
EMSCRIPTEN=5.0.3
root=$(cd "$(dirname "$0")/../../.." && pwd); mkdir -p "$out"; out=$(cd "$out" && pwd)

# The cross-build environment is fetched by its release asset and checked against its hash (Pyodide 314.0.8's
# xbuildenv, built for the 314 runtime and the emscripten pinned above), not picked from remote metadata.
XBUILDENV_URL=https://github.com/pyodide/pyodide/releases/download/314.0.8/xbuildenv-314.0.8.tar.bz2
XBUILDENV_SHA256=4dc1904011077281827f607c59aa0547e08ad26f307a4c7ab7382d6c95ed3cd9
xb=$(mktemp -d)
curl -sSfL --retry 3 -o "$xb/xbuildenv.tar.bz2" "$XBUILDENV_URL"
echo "$XBUILDENV_SHA256  $xb/xbuildenv.tar.bz2" | sha256sum -c -
pyodide xbuildenv install --url "file://$xb/xbuildenv.tar.bz2"
rm -rf "$xb"
got=$(pyodide config get emscripten_version)
[ "$got" = "$EMSCRIPTEN" ] || { echo "::error::the cross-build environment wants emscripten $got, this script pins $EMSCRIPTEN"; exit 1; }

if [ ! -d "$emsdk/.git" ]; then git clone -q https://github.com/emscripten-core/emsdk.git "$emsdk"; fi
git -C "$emsdk" fetch -q origin "$EMSDK_REF" 2>/dev/null || true
git -C "$emsdk" checkout -q "$EMSDK_REF"
(cd "$emsdk" && ./emsdk install "$EMSCRIPTEN" && ./emsdk activate "$EMSCRIPTEN")
# shellcheck disable=SC1091
. "$emsdk/emsdk_env.sh"

cd "$root"
start=$(date +%s)
# Build dependencies (maturin) come from the hash-pinned environment, not from an isolated one fetched afresh.
pyodide build --no-isolation --skip-dependency-check -o "$out"
echo "wheel build seconds: $(( $(date +%s) - start ))"
ls -l "$out"
