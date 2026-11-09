#!/usr/bin/env bash
# install-cargo-tools.sh TOOL@VERSION[,TOOL@VERSION...]: build each tool from
# crates.io with `cargo install --locked`, for when the prebuilt download a
# workflow uses first failed (its release host down). A tool already on PATH is
# skipped. wasm-bindgen's CLI crate is wasm-bindgen-cli.
set -euo pipefail
IFS=, read -r -a specs <<< "$1"
for spec in "${specs[@]}"; do
  tool=${spec%@*}
  version=${spec#*@}
  if command -v "$tool" > /dev/null; then
    continue
  fi
  crate=$tool
  [ "$tool" = wasm-bindgen ] && crate=wasm-bindgen-cli
  echo "::warning::$tool was not installed from its prebuilt download; building $crate $version from crates.io"
  tools/ci/retry.sh 3 cargo install --locked "$crate" --version "=$version"
done
