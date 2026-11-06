#!/usr/bin/env bash
# Score rvt-open's drawings against the authoring tool's own DWG exports of the same public samples.
#
#   tools/dwg-oracle/score.sh <exports dir> <samples dir> <out dir> [manifest.tsv]
#
# <exports dir> holds the reference DWG/DXF exports and is only read. The manifest (default:
# <exports dir>/manifest.tsv, else tools/dwg-oracle/manifest.example.tsv as a template) has one row per
# export, tab separated, '#' comments allowed:
#
#   export            path of the export, relative to <exports dir>
#   sample            the .rvt it was exported from, relative to <samples dir>
#   kind              plan  - a floor plan view: compared with `rvt-sheet --format dxf` (model space)
#                     sheet - a sheet: the reference inventory is recorded; compared when rvt-open writes sheets
#                     view  - any other view: the reference inventory is recorded
#   layout            sheet layout to compare (paper space), or '-' for model space
#   tolerance         match distance in the export's units (e.g. 5 for 5 mm), or '-' for 5
#
# Writes under <out dir>/<export name>/: reference/ (tables, SVG of every layout, report.json),
# candidate.dxf and score.json/score.tsv when there is a candidate, and <out dir>/summary.tsv.
# Binaries: rvt-dwg and rvt-sheet on PATH, or RVT_BIN=<folder holding them>.
set -euo pipefail
exports=${1:?usage: score.sh <exports dir> <samples dir> <out dir> [manifest.tsv]}
samples=${2:?samples dir}
out=${3:?out dir}
manifest=${4:-$exports/manifest.tsv}
[[ -f $manifest ]] || { echo "no manifest at $manifest: copy tools/dwg-oracle/manifest.example.tsv there and fill it in" >&2; exit 1; }
bin() { if [[ -n ${RVT_BIN:-} ]]; then echo "$RVT_BIN/$1"; else command -v "$1"; fi; }
DWG=$(bin rvt-dwg) SHEET=$(bin rvt-sheet)
case $(realpath -m "$out")/ in "$(realpath -m "$exports")"/*) echo "the out dir must not be inside the exports dir (it is read-only)" >&2; exit 1;; esac
mkdir -p "$out"
printf 'export\tkind\tlinework_precision\tlinework_recall\tlinework_f1\ttext_f1\tdimensions_f1\tnote\n' > "$out/summary.tsv"
while IFS=$'\t' read -r export sample kind layout tol; do
  [[ -z ${export// } || $export == \#* ]] && continue
  name=$(basename "${export%.*}") dir="$out/$name"
  mkdir -p "$dir/reference"
  ref="$exports/$export"
  [[ -f $ref ]] || { printf '%s\t%s\t\t\t\t\t\tmissing export\n' "$export" "$kind" >> "$out/summary.tsv"; continue; }
  "$DWG" "$ref" --all-layouts --format csv,svg,report -o "$dir/reference" > "$dir/reference.log" 2>&1 || true
  tol=${tol:--}; [[ $tol == - ]] && tol=5
  args=(--tolerance "$tol"); [[ ${layout:--} != - ]] && args+=(--layout "$layout")
  if [[ $kind == plan ]]; then
    "$SHEET" "$samples/$sample" --format dxf --no-labels -o "$dir/candidate.dxf" > "$dir/candidate.log" 2>&1 \
      || { printf '%s\t%s\t\t\t\t\t\tcandidate failed\n' "$export" "$kind" >> "$out/summary.tsv"; continue; }
    [[ -f $exports/layer-map.tsv ]] && args+=(--layer-map "$exports/layer-map.tsv")
    "$DWG" compare "$ref" "$dir/candidate.dxf" "${args[@]}" -o "$dir" > "$dir/compare.log" 2>&1
    python3 - "$dir/score.json" "$export" "$kind" >> "$out/summary.tsv" <<'PY'
import json, sys
s = json.load(open(sys.argv[1])); l = s["linework"]
print("\t".join([sys.argv[2], sys.argv[3], f"{l['precision']:.4f}", f"{l['recall']:.4f}", f"{l['f1']:.4f}", f"{s['text']['f1']:.4f}", f"{s['dimensions']['f1']:.4f}", "synthetic plan baseline"]))
PY
  else
    printf '%s\t%s\t\t\t\t\t\treference inventory only: rvt-open does not write %ss yet\n' "$export" "$kind" "$kind" >> "$out/summary.tsv"
  fi
done < "$manifest"
column -t -s $'\t' "$out/summary.tsv"
