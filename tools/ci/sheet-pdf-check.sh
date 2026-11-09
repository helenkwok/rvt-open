#!/bin/bash
# 2D PDF check with independent tools: `rvt-sheet --format pdf` of a sheet and of a plan view, then
#   qpdf --check     the PDF's syntax, cross-reference table and content stream;
#   pdftoppm         renders the page and the render is not blank;
#   rvt-dwg pdf-check  the PDF's segments, mapped back to millimetres, lie on the DXF drawing
#                    `rvt-sheet` writes for the same sheet or view, and cover it.
# Usage: sheet-pdf-check.sh BIN_DIR SAMPLE_FILE SHEET VIEW
set -euo pipefail
BIN=$1 RVT=$2 SHEET=$3 VIEW=$4
W=${RUNNER_TEMP:-/tmp}/sheet-pdf; rm -rf "$W"; mkdir -p "$W"
check() {
  local name=$1; shift
  local layout=(); [[ $1 == --sheet ]] && layout=(--layout Layout1 --tolerance 0.1)
  [[ $1 == --view ]] && layout=(--tolerance 5)
  "$BIN/rvt-sheet" "$RVT" "$@" --format pdf -o "$W/$name.pdf"
  "$BIN/rvt-sheet" "$RVT" "$@" --format dxf -o "$W/$name.dxf"
  qpdf --check "$W/$name.pdf"
  pdftoppm -r 30 -singlefile "$W/$name.pdf" "$W/$name.render"
  python3 - "$W/$name.render.ppm" <<'P'
import sys
d = open(sys.argv[1], "rb").read()
head = d.split(b"\n", 3)
w, h = map(int, head[1].split()); px = head[3]
ink = sum(1 for i in range(0, len(px) - 2, 3) if px[i] < 240 or px[i + 1] < 240 or px[i + 2] < 240)
share = ink / (w * h)
print(f"render {w}x{h}, {share:.1%} drawn")
assert share > 0.005, "the page renders blank"
P
  "$BIN/rvt-dwg" pdf-check "$W/$name.pdf" "$W/$name.dxf" "${layout[@]}"
  echo "SHEET-PDF-CHECK ok $name $(stat -c %s "$W/$name.pdf") bytes"
}
check sheet --sheet "$SHEET"
check view --view "$VIEW"
