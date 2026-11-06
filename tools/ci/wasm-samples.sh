#!/usr/bin/env bash
# the wasm package converts a public sample of every release 2016-2027 without a trap, and its
# element counts equal the native CLI's.
#
#   tools/ci/wasm-samples.sh <wasm pkg dir> <native rvt-elements> <samples dir>
#
# <samples dir> holds <year>/rac_basic_sample_project.rvt (the Autodesk samples; the cloud gate's
# rvt-samples/autodesk-samples-all). Needs node. Exits non-zero on a trap, a thrown error, or a count mismatch.
# A panic must arrive as an Error with a message ("rvt-open wasm panic: ..."), never a bare "unreachable".
set -euo pipefail
pkg=$1 native=$2 dir=$3
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
files=()
for y in 2016 2017 2018 2019 2020 2021 2022 2023 2024 2025 2026 2027; do
    f="$dir/$y/rac_basic_sample_project.rvt"
    [[ -f $f ]] || { echo "FAIL: missing sample $f" >&2; exit 1; }
    files+=("$f")
done
out=$(mktemp)
trap 'rm -f "$out"' EXIT
node --max-old-space-size=8192 "$here/wasm-convert.mjs" "$pkg" "${files[@]}" | tee "$out"
fail=0
for f in "${files[@]}"; do
    rec=$(grep -F "\"file\":\"$f\"" "$out" || true)
    [[ -n $rec ]] || { echo "FAIL: no result for $f" >&2; fail=1; continue; }
    nat=$("$native" "$f" --counts)
    if ! node -e '
        const [rec, nat] = process.argv.slice(1).map(JSON.parse);
        if (!rec.ok) { console.error("FAIL", rec.file, rec.error); process.exit(1); }
        const a = JSON.stringify(rec.counts), b = JSON.stringify(nat);
        if (a !== b) { console.error("FAIL counts differ", rec.file, a, b); process.exit(1); }
        console.log("ok", rec.file, nat.total, "elements");
    ' "$rec" "$nat"; then fail=1; fi
done
exit $fail
