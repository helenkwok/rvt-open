#!/usr/bin/env bash
# Score rvt-open's drawings against the authoring tool's own DWG exports of the same public samples.
#
#   tools/dwg-oracle/score.sh <exports dir> <samples dir> <out dir> [manifest.tsv] [layer-map.tsv]
#
# <exports dir> holds the reference DWG/DXF exports and is only read. The manifest (default:
# <exports dir>/manifest.tsv, else tools/dwg-oracle/manifest.example.tsv as a template) has one row per
# export, tab separated, '#' comments allowed:
#
#   export            path of the export, relative to <exports dir>
#   sample            the .rvt it was exported from, relative to <samples dir>
#   kind              plan  - a floor plan view: compared with `rvt-sheet --view` (model space)
#                     sheet - a sheet: compared with `rvt-sheet --sheet <number>` (the export's file name
#                             up to ' - '), on its paper layout with model space seen through each
#                             viewport (viewport centres and scales, title block, text)
#                     view  - any other view: the reference inventory is recorded
#   layout            sheet layout to compare (paper space; the export's is 'Layout1'), or '-' for model space
#   tolerance         match distance in the export's units (e.g. 5 for 5 mm), or '-' for 5
#   view              optional: the plan view the export shows, passed to `rvt-sheet --view`. Left out
#                     (or '-'), it is taken from the export's file name ('... Floor Plan - <view>.dwg' or
#                     'FloorPlan - <view>.dwg'); '*' draws the whole model instead
#
# The layer map (default: <exports dir>/layer-map.tsv when it exists) pairs export layers with rvt-open's.
# Plan candidates are drawn with their labels (room tags, grid heads, text notes, dimensions), so text and
# dimensions are scored as well as linework.
#
# Each candidate is written in every format of FORMATS (default "dxf dwg") and each is scored, one
# summary row per format; with "pdf" in FORMATS a PDF is written too and checked against the DXF
# drawing (`rvt-dwg pdf-check`: every segment of the PDF lies on the DXF's linework).
#
# Writes under <out dir>/<export name>/: reference/ (tables, SVG of every layout, report.json),
# candidate.<format> and <format>/score.json, score.tsv when there is a candidate, and
# <out dir>/summary.tsv. Binaries: rvt-dwg and rvt-sheet on PATH, or RVT_BIN=<folder holding them>.
set -euo pipefail
exports=${1:?usage: score.sh <exports dir> <samples dir> <out dir> [manifest.tsv] [layer-map.tsv]}
samples=${2:?samples dir}
out=${3:?out dir}
manifest=${4:-$exports/manifest.tsv}
layer_map=${5:-$exports/layer-map.tsv}
[[ -z ${5:-} || -f $layer_map ]] || { echo "no layer map at $layer_map" >&2; exit 1; }
[[ -f $manifest ]] || { echo "no manifest at $manifest: copy tools/dwg-oracle/manifest.example.tsv there and fill it in" >&2; exit 1; }
bin() { if [[ -n ${RVT_BIN:-} ]]; then echo "$RVT_BIN/$1"; else command -v "$1"; fi; }
DWG=$(bin rvt-dwg) SHEET=$(bin rvt-sheet)
case $(realpath -m "$out")/ in "$(realpath -m "$exports")"/*) echo "the out dir must not be inside the exports dir (it is read-only)" >&2; exit 1;; esac
mkdir -p "$out"
read -r -a formats <<< "${FORMATS:-dxf dwg}"
printf 'export\tkind\tformat\tlinework_precision\tlinework_recall\tlinework_f1\ttext_f1\tdimensions_f1\tviewports\tviewports_within_10mm\tnote\n' > "$out/summary.tsv"
# The plan view an export shows, from its file name: '<sheet>-Floor Plan - <view>' or 'FloorPlan - <view>'.
view_of() { local n; n=$(basename "${1%.*}"); case $n in *"Floor Plan - "*) echo "${n##*Floor Plan - }";; "FloorPlan - "*) echo "${n#FloorPlan - }";; *) echo '*';; esac; }
while IFS=$'\t' read -r export sample kind layout tol view; do
  [[ -z ${export// } || $export == \#* ]] && continue
  name=$(basename "${export%.*}") dir="$out/$name"
  mkdir -p "$dir/reference"
  ref="$exports/$export"
  [[ -f $ref ]] || { printf '%s\t%s\t\t\t\t\t\t\t\t\tmissing export\n' "$export" "$kind" >> "$out/summary.tsv"; continue; }
  "$DWG" "$ref" --all-layouts --format csv,svg,report -o "$dir/reference" > "$dir/reference.log" 2>&1 || true
  tol=${tol:--}; [[ $tol == - ]] && tol=5
  args=(--tolerance "$tol"); [[ ${layout:--} != - ]] && args+=(--layout "$layout")
  case $kind in
    plan)
      [[ ${view:--} == - ]] && view=$(view_of "$export")
      sheet=() note="whole model"
      [[ $view != '*' ]] && sheet+=(--view "$view") note="plan view $view";;
    sheet)
      number=$(basename "${export%.*}"); number=${number%% - *}
      sheet=(--sheet "$number") note="sheet $number";;
    *)
      printf '%s\t%s\t\t\t\t\t\t\t\t\treference inventory only: rvt-open does not write %ss yet\n' "$export" "$kind" "$kind" >> "$out/summary.tsv"
      continue;;
  esac
  [[ -f $layer_map ]] && args+=(--layer-map "$layer_map")
  for fmt in "${formats[@]}"; do
    [[ $fmt == pdf ]] && continue
    cand="$dir/candidate.$fmt"
    "$SHEET" "$samples/$sample" "${sheet[@]}" --format "$fmt" -o "$cand" > "$dir/candidate.$fmt.log" 2>&1 \
      || { printf '%s\t%s\t%s\t\t\t\t\t\t\t\tcandidate failed (%s)\n' "$export" "$kind" "$fmt" "$note" >> "$out/summary.tsv"; continue; }
    mkdir -p "$dir/$fmt"
    "$DWG" compare "$ref" "$cand" "${args[@]}" -o "$dir/$fmt" > "$dir/$fmt/compare.log" 2>&1 \
      || { printf '%s\t%s\t%s\t\t\t\t\t\t\t\tcompare failed (%s)\n' "$export" "$kind" "$fmt" "$note" >> "$out/summary.tsv"; continue; }
    python3 - "$dir/$fmt/score.json" "$export" "$kind" "$fmt" "$note" >> "$out/summary.tsv" <<'PY'
import json, sys
s = json.load(open(sys.argv[1])); l = s["linework"]
# Reference viewports with a candidate viewport within 10 mm of their centre at their scale.
near = [v for sh in s.get("sheets", []) for v in sh["viewports"]
        if v["centre_distance"] <= 10 and abs(v["scale_candidate"] - v["scale_reference"]) <= 1e-3 * v["scale_reference"]]
counts = ";".join(f"{sh['viewports_candidate']}/{sh['viewports_reference']}" for sh in s.get("sheets", []))
centre = f"{len(near)}" + (f" ({max(v['centre_distance'] for v in near):.1f})" if near else "")
print("\t".join([sys.argv[2], sys.argv[3], sys.argv[4], f"{l['precision']:.4f}", f"{l['recall']:.4f}", f"{l['f1']:.4f}", f"{s['text']['f1']:.4f}", f"{s['dimensions']['f1']:.4f}", counts, centre, sys.argv[5]]))
PY
  done
  if [[ " ${formats[*]} " == *" pdf "* ]]; then
    pdfargs=(); [[ ${layout:--} != - ]] && pdfargs+=(--layout Layout1)
    if "$SHEET" "$samples/$sample" "${sheet[@]}" --format pdf -o "$dir/candidate.pdf" > "$dir/candidate.pdf.log" 2>&1 \
      && "$SHEET" "$samples/$sample" "${sheet[@]}" --format dxf -o "$dir/pdf-source.dxf" > /dev/null 2>&1 \
      && "$DWG" pdf-check "$dir/candidate.pdf" "$dir/pdf-source.dxf" "${pdfargs[@]}" > "$dir/pdf-check.txt" 2>&1; then
      printf '%s\t%s\tpdf\t\t\t\t\t\t\t\t%s\n' "$export" "$kind" "$(tail -1 "$dir/pdf-check.txt")" >> "$out/summary.tsv"
    else
      printf '%s\t%s\tpdf\t\t\t\t\t\t\t\tpdf check failed (%s)\n' "$export" "$kind" "$(tail -1 "$dir/pdf-check.txt" 2>/dev/null)" >> "$out/summary.tsv"
    fi
  fi
done < "$manifest"
# Aligned columns with awk (POSIX), not `column`, which minimal systems lack.
awk -F'\t' 'NR == FNR { for (i = 1; i <= NF; i++) if (length($i) > w[i]) w[i] = length($i); next }
  { line = ""; for (i = 1; i <= NF; i++) line = line sprintf(i < NF ? "%-" w[i] "s  " : "%s", $i); print line }' \
  "$out/summary.tsv" "$out/summary.tsv"
