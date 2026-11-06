#!/usr/bin/env bash
# build the 64-bit browser package (wasm64-unknown-unknown, memory64: up to 16 GiB in a browser)
# next to the default 32-bit one (wasm-pack, 4 GiB), and put the loader that picks between them beside it.
#
#   tools/ci/wasm64-build.sh <out dir>      # writes rvt_open64.js, rvt_open64_bg.wasm, ... and rvt_open_auto.js
#
# wasm64 is a tier-3 Rust target: no prebuilt std, so this needs a nightly with rust-src (-Z build-std).
# RUSTUP_TOOLCHAIN, not `cargo +nightly`: the +toolchain shorthand needs rustup under CARGO_HOME, which CI moves.
# wasm-bindgen-cli must be on PATH at the version Cargo.lock pins for wasm-bindgen (>= 0.2.119 for wasm64).
# Copy rvt_open.js / rvt_open_bg.wasm from the wasm-pack build into the same dir for the loader's fallback.
set -euo pipefail
out=${1:?usage: wasm64-build.sh <out dir>}
nightly=${RVT_WASM64_NIGHTLY:-nightly-2026-10-30}
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
# `rustup component add` under RUSTUP_TOOLCHAIN installs the toolchain too; `rustup toolchain install` failed in
# the CI image, where it tries to write proxies into a CARGO_HOME that holds no rustup.
RUSTUP_TOOLCHAIN="$nightly" rustup component add rust-src
want=$(awk '$0 == "name = \"wasm-bindgen\"" { getline; gsub(/[^0-9.]/, ""); print }' Cargo.lock)
have=$(wasm-bindgen --version | awk '{ print $2 }')
[[ $have == "$want" ]] || { echo "FAIL: wasm-bindgen-cli $have, Cargo.lock pins wasm-bindgen $want" >&2; exit 1; }
RUSTUP_TOOLCHAIN="$nightly" cargo build -p rvt-wasm --release --locked --target wasm64-unknown-unknown -Z build-std=std,panic_abort
wasm-bindgen --target web --out-dir "$out" --out-name rvt_open64 \
    "${CARGO_TARGET_DIR:-target}/wasm64-unknown-unknown/release/rvt_wasm.wasm"
cp rvt-wasm/js/rvt_open_auto.js "$out/"
command -v node >/dev/null || { echo "FAIL: node is needed to patch and check the package" >&2; exit 1; }
# wasm-bindgen 0.2.130 bug on wasm64: an export that returns through a stack slot takes that slot's pointer as an
# i64 (the .d.ts says `a: bigint`), but the generated JS passes the Number it got from
# __wbindgen_add_to_stack_pointer, so every such call throws "Cannot convert <n> to a BigInt". Wrap the pointer in
# BigInt() at exactly those call sites (also a call whose only argument is the slot, e.g. memory_stats), and fail if
# any `wasm.<fn>(retptr` is left unwrapped or the shape differs (a fixed wasm-bindgen leaves no bigint params).
node -e 'const fs = require("fs"); const [js, dts] = process.argv.slice(1); const d = fs.readFileSync(dts, "utf8");
  const names = [...d.matchAll(/export const (\w+): \(a: bigint/g)].map((m) => m[1]);
  if (/[b-z]: bigint/.test(d)) { console.error("FAIL: a bigint parameter other than the return slot"); process.exit(1); }
  let s = fs.readFileSync(js, "utf8"), n = 0;
  for (const f of names) s = s.replace(new RegExp(`\\bwasm\\.${f}\\(retptr([,)])`, "g"), (_, end) => (n++, `wasm.${f}(BigInt(retptr)${end}`));
  if (n !== names.length) { console.error(`FAIL: ${names.length} i64 return-slot exports, ${n} call sites patched`); process.exit(1); }
  const left = s.match(/\bwasm\.\w+\(retptr\b/g);
  if (left) { console.error(`FAIL: ${left.length} return-slot call sites left unwrapped: ${[...new Set(left)].join(" ")}`); process.exit(1); }
  fs.writeFileSync(js, s); console.log(`return-slot pointer wrapped in BigInt at ${n} call sites`)' \
  "$out/rvt_open64.js" "$out/rvt_open64_bg.wasm.d.ts"
# The module must really declare a 64-bit memory: memory section (id 5), first limits flag 0x04 or 0x05.
node -e 'const b = require("fs").readFileSync(process.argv[1]); let i = 8;
  const leb = () => { let r = 0, s = 0, x; do { x = b[i++]; r += (x & 0x7f) * 2 ** s; s += 7; } while (x & 0x80); return r; };
  while (i < b.length) { const id = b[i++], n = leb(), end = i + n;
    if (id === 5) { leb(); const f = b[i]; if (f !== 4 && f !== 5) { console.error("FAIL: memory limits flag", f, "is not 64-bit"); process.exit(1); }
      console.log("wasm64 module ok:", b.length, "bytes, memory flag", f); process.exit(0); }
    i = end; }
  console.error("FAIL: no memory section"); process.exit(1)' "$out/rvt_open64_bg.wasm"
