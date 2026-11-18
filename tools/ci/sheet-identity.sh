#!/usr/bin/env bash
# Draws the same sheet of each model twice with rvt-sheet, from the model the viewer's open keeps for its first drawing
# (--viewer-model) and from a fresh export, and fails unless the two SVGs are the same bytes.
#
#   tools/ci/sheet-identity.sh <rvt-sheet> <model.rvt:SHEET>...
set -euo pipefail
sheet=${1:?usage: sheet-identity.sh <rvt-sheet> <model.rvt:SHEET>...}; shift
out=sheet-identity-out; mkdir -p "$out"
status=0
for spec in "$@"; do
  model=${spec%:*}; name=${spec##*:}; stem=$(basename "$model" .rvt)-$name
  [[ -s $model ]] || { echo "FAIL: $model missing"; exit 1; }
  "$sheet" "$model" --sheet "$name" --format svg -o "$out/$stem-export.svg"
  "$sheet" "$model" --sheet "$name" --format svg --viewer-model -o "$out/$stem-kept.svg"
  # Each way twice: a way that differs from itself is not deterministic, whatever the other says.
  "$sheet" "$model" --sheet "$name" --format svg -o "$out/$stem-export2.svg"
  "$sheet" "$model" --sheet "$name" --format svg --viewer-model -o "$out/$stem-kept2.svg"
  cmp -s "$out/$stem-export.svg" "$out/$stem-export2.svg" && echo "export twice: same" || echo "export twice: DIFFERENT"
  cmp -s "$out/$stem-kept.svg" "$out/$stem-kept2.svg" && echo "kept twice: same" || echo "kept twice: DIFFERENT"
  if cmp -s "$out/$stem-export.svg" "$out/$stem-kept.svg"; then
    echo "identical: $stem ($(stat -c %s "$out/$stem-kept.svg") bytes)"
  else
    echo "DIFFERENT: $stem"; cmp "$out/$stem-export.svg" "$out/$stem-kept.svg" | head -1 || true
    status=1
  fi
done
exit $status
