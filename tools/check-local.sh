#!/usr/bin/env bash
# Developer-friendly local quality gate for rvt-open.
#
# Default (no flags): fmt check, clippy -D warnings, rustdoc -D warnings,
# a build of every target, and the targets tools/ci/test-targets.txt runs
# (real-file, CLI, contract and fuzz-regression hardening). Unit tests are
# banned and never run. Does not require network.
#
# Optional expensive / environment-dependent checks are opt-in via flags.
# Prefer this script for day-to-day local verification; use tools/quality.sh
# when you also want the pre-push supply-chain / bench compile path.
set -euo pipefail

usage() {
    cat <<'USAGE'
Usage: tools/check-local.sh [options]

Required gates (always run):
  cargo fmt --all -- --check
  cargo clippy --workspace --all-targets --all-features -- -D warnings
  RUSTFLAGS="-D warnings -D unnameable_types" cargo check --lib --bins
  RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --lib
  cargo build --workspace --lib --bins --all-features
  cargo test --profile ci --doc     (the API docs' examples, on corpus/tier1)
  tools/ci/verify-real-files.sh --profile ci  (real-file, CLI, contract targets only)
  cargo run --profile ci --example basics_*  (the user examples, on corpus/tier1)
  tools/ci/audit-mcp-network.sh     (the MCP server's tree has no HTTP crate)
  cargo test --profile ci -p rvt-open-mcp  (the MCP server's end-to-end test)

Optional gates (opt-in; fail clearly when prerequisites are missing):
  --viewer         viewer/ pnpm typecheck + build (no network install)
  --corpus         re-run the real-file checks against the corpora
                   (RVT_SAMPLES_DIR / RVT_PROJECT_CORPUS_DIR, --profile ci)
  --ifcopenshell   verify the ifcopenshell Python module imports
  --deny           run cargo deny check (requires cargo-deny)
  --audit          run cargo audit (requires cargo-audit)
  --all-optional   enable every optional gate above

  -h, --help       show this help

Corpus directories (for --corpus):
  RVT_SAMPLES_DIR          default: $PWD/_corpus/examples/Autodesk
  RVT_PROJECT_CORPUS_DIR   default: $PWD/_project_corpus/Revit

Does not download corpora or install tools. Fetch corpora with
tools/fetch-corpus.sh or the AGENTS.md clone recipe when needed.
USAGE
}

run() {
    printf '\n==> %s\n' "$*"
    "$@"
}

require_cmd() {
    local name="$1"
    local hint="$2"
    if ! command -v "$name" >/dev/null 2>&1; then
        echo "error: '$name' is required for this check but is not installed." >&2
        echo "       $hint" >&2
        exit 1
    fi
}

run_viewer=0
run_corpus=0
run_ifcopenshell=0
run_deny=0
run_audit=0

while [[ $# -gt 0 ]]; do
    case "$1" in
        --viewer)
            run_viewer=1
            ;;
        --corpus)
            run_corpus=1
            ;;
        --ifcopenshell)
            run_ifcopenshell=1
            ;;
        --deny)
            run_deny=1
            ;;
        --audit)
            run_audit=1
            ;;
        --all-optional)
            run_viewer=1
            run_corpus=1
            run_ifcopenshell=1
            run_deny=1
            run_audit=1
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "unknown option: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
    shift
done

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

# --- required gates ---------------------------------------------------------

run cargo fmt --all -- --check
# the box check first, so a box regression fails before clippy and the tests: per public sample
# and category, elements drawn as a 12-triangle box or still on the bounding-box fallback, against
# tools/ci/box-baseline.tsv (target zero; lower it as shape work lands). Skips when no sample directory is set.
run cargo run --profile ci --example box_check -- tools/ci/box-baseline.tsv
run cargo clippy --workspace --all-targets --all-features -- -D warnings
run env RUSTFLAGS="-D warnings -D unnameable_types" cargo check --lib --bins
run env RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --lib
run cargo clippy -p rvt-wasm --all-targets -- -D warnings
# with a built wasm pkg, a native rvt-elements and the samples named, every release 2016-2027 converts
# in wasm without a trap and with the native element counts (the cloud gate sets RVT_WASM_PKG/_NATIVE/_SAMPLES).
if [[ -n "${RVT_WASM_PKG:-}" ]]; then
    run tools/ci/wasm-samples.sh "$RVT_WASM_PKG" "${RVT_WASM_NATIVE:?}" "${RVT_WASM_SAMPLES:?}"
fi
# the 64-bit (memory64) browser package still builds. Opt-in: needs a nightly with rust-src and
# wasm-bindgen-cli at the locked version, which the stable gate image lacks.
if [[ -n "${RVT_WASM64_OUT:-}" ]]; then
    run tools/ci/wasm64-build.sh "$RVT_WASM64_OUT"
fi
# Examples and benches are type-checked by clippy --all-targets above; linking all
# 203 of them took ~36 of the build's ~49 CPU-minutes (cargo --timings).
run cargo build --workspace --lib --bins --all-features
run cargo test --profile ci --doc
# The optimised ci profile: the 301 real-file tests took 3 h 47 min in the debug
# profile and 6 min in ci.
run tools/ci/verify-real-files.sh --profile ci
run tools/ci/audit-mcp-network.sh
run cargo test --profile ci -p rvt-open-mcp
run cargo run --profile ci --example basics_inspect
run cargo run --profile ci --example basics_elements
basics_out="$(mktemp -d)"
run cargo run --profile ci --example basics_export -- \
    corpus/tier1/architectural-2024/architectural-2024.rvt "$basics_out"
rm -rf "$basics_out"

# --- optional gates ---------------------------------------------------------

if [[ "$run_viewer" -eq 1 ]]; then
    if [[ ! -d viewer/node_modules ]]; then
        echo "error: viewer/node_modules is missing." >&2
        echo "       From viewer/: run 'pnpm install --frozen-lockfile' once (network), then re-run with --viewer." >&2
        exit 1
    fi
    run pnpm --dir viewer run typecheck
    run pnpm --dir viewer run build
    run node viewer/scripts/audit-wasm-imports.mjs
fi

if [[ "$run_corpus" -eq 1 ]]; then
    samples_dir="${RVT_SAMPLES_DIR:-$root/_corpus/examples/Autodesk}"
    project_dir="${RVT_PROJECT_CORPUS_DIR:-$root/_project_corpus/Revit}"
    missing=0
    if [[ ! -d "$samples_dir" ]]; then
        echo "error: sample corpus directory not found: $samples_dir" >&2
        missing=1
    fi
    if [[ ! -d "$project_dir" ]]; then
        echo "error: project corpus directory not found: $project_dir" >&2
        missing=1
    fi
    if [[ "$missing" -eq 1 ]]; then
        echo "       Clone corpora first (see AGENTS.md), or set RVT_SAMPLES_DIR /" >&2
        echo "       RVT_PROJECT_CORPUS_DIR to existing trees. --corpus does not download." >&2
        exit 1
    fi
    # The optimised `ci` profile: the debug profile takes tens of minutes
    # to decode the 33.7 MB project.
    run env \
        RVT_SAMPLES_DIR="$samples_dir" \
        RVT_PROJECT_CORPUS_DIR="$project_dir" \
        tools/ci/verify-real-files.sh --profile ci --no-fail-fast
fi

if [[ "$run_ifcopenshell" -eq 1 ]]; then
    require_cmd python3 "Install Python 3 and the ifcopenshell package."
    printf '\n==> python3 -c "import ifcopenshell"\n'
    if ! python3 -c "import ifcopenshell; print(ifcopenshell.version)" 2>/dev/null; then
        echo "error: Python module 'ifcopenshell' is not importable." >&2
        echo "       Install with: python3 -m pip install --require-hashes -r tools/ci/requirements-ifc.txt" >&2
        exit 1
    fi
    # Same gate CI runs — every instance the exporter writes
    # for the tier-1 projects must carry exactly the attributes its IFC4
    # entity type declares, and mapped PredefinedType values must survive.
    run cargo build --profile ci --bin rvt-ifc
    for f in corpus/tier1/*/*.rvt; do
        run ./target/ci/rvt-ifc "$f" -o "/tmp/rvt-tier1-$(basename "$f" .rvt).ifc"
    done
    run python3 tools/ci/ifc_schema_arity.py /tmp/rvt-tier1-*.ifc
fi

if [[ "$run_deny" -eq 1 ]]; then
    require_cmd cargo-deny "Install with: cargo install cargo-deny --locked"
    run cargo deny check
fi

if [[ "$run_audit" -eq 1 ]]; then
    require_cmd cargo-audit "Install with: cargo install cargo-audit --locked"
    run cargo audit
fi

printf '\ncheck-local: all requested gates passed\n'
