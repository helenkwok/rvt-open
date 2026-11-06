#!/usr/bin/env bash
# Pre-merge fast check: fmt, clippy, the build, the box check and the integration-test modules the change can
# affect. Target: under 5 minutes on a warm target dir. The full suite (tools/check-local.sh plus the IFC rules and the
# format validators) runs after the merge; a red full run reverts the merge.
#
# Usage: tools/ci/fast-check.sh [scope-file]
#   scope-file: the output of tools/ci/affected-tests.py (module names, NONE or FULL). Without one, it is computed
#   against origin/main-next or main-next. FULL runs the smoke set: the cli and hardening targets of test-targets.txt.
# Prints FAST-SCOPE: and step timings; exits non-zero on the first failure.
set -euo pipefail
cd "$(dirname "$0")/../.."
if [ -n "${1:-}" ]; then scope=$(cat "$1"); else
  base=$(git rev-parse -q --verify origin/main-next || git rev-parse -q --verify main-next || echo HEAD~1)
  scope=$(python3 tools/ci/affected-tests.py . HEAD "$base" || echo FULL); fi
if [ "$scope" = FULL ]; then scope=$(awk '$1=="cli"||$1=="hardening"{print $2}' tools/ci/test-targets.txt); fi
mods=$(printf '%s\n' "$scope" | grep -v '^$' | grep -vx NONE | paste -sd'|' - || true)
echo "FAST-SCOPE: ${mods:-none}"
t=$(date +%s); step() { echo "==> $* ($(( $(date +%s) - t ))s)"; }
step fmt; cargo fmt --all -- --check
step clippy; cargo clippy --workspace --all-targets --all-features -- -D warnings
step build; cargo build --profile ci --workspace --lib --bins --all-features
# The box check's wall time is its slowest sample (Snowdon Towers, ~4 min), so it runs beside the tests, not before them.
cargo build --profile ci --example box_check -q
step "box-check + tests (side by side)"; bl=$(mktemp)
cargo run --profile ci -q --example box_check -- tools/ci/box-baseline.tsv > "$bl" 2>&1 & bp=$!
trc=0; if [ -n "$mods" ]; then
  cargo nextest run --no-tests=pass --cargo-profile ci --test integration -E "test(/^($mods)::/)" || trc=$?; fi
step "tests done rc=$trc"; brc=0; wait $bp || brc=$?
echo "==> box-check output (rc=$brc)"; cat "$bl"; rm -f "$bl"
step done; [ $trc = 0 ] && [ $brc = 0 ]
