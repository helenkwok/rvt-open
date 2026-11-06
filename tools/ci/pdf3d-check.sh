#!/bin/bash
# 3D PDF check with independent tools: `rvt-gltf --format pdf3d` on each sample, then
#   qpdf --check     the PDF's syntax, cross-reference table and streams;
#   qpdf --json      one /3D annotation whose /3DD stream is /Subtype /U3D, whose
#                    /3DV has a 12-number /C2W, and whose stream (inflated by qpdf)
#                    starts with the U3D file header block and states its own size;
#   pdftoppm         renders the page (poppler draws the annotation's poster
#                    appearance) and the render is not blank.
# Usage: pdf3d-check.sh BIN_DIR SAMPLE_FILE...
set -euo pipefail
BIN=$1; shift; W=${RUNNER_TEMP:-/tmp}/pdf3d; rm -rf "$W"; mkdir -p "$W"
for p in "$@"; do
  o=$W/$(basename "$p" .rvt).pdf
  "$BIN/rvt-gltf" "$p" --format pdf3d -o "$o"
  qpdf --check "$o"
  qpdf --json "$o" > "$o.json"
  n=$(python3 - "$o.json" <<'P'
import json, sys
objs = json.load(open(sys.argv[1]))["qpdf"][1]
def get(ref):
    o = objs["obj:" + ref]
    return o.get("value") or o["stream"]["dict"]
annots = [v.get("value", {}) for v in objs.values()]
annots = [a for a in annots if isinstance(a, dict) and a.get("/Subtype") == "/3D"]
assert len(annots) == 1, f"{len(annots)} 3D annotations"
a = annots[0]
assert "/N" in a["/AP"], "poster appearance"
s = get(a["/3DD"])
assert s["/Type"] == "/3D" and s["/Subtype"] == "/U3D", s
v = get(a["/3DV"])
assert v["/Type"] == "/3DView" and len(v["/C2W"]) == 12, v
print(a["/3DD"].split()[0])
P
)
  qpdf --show-object="$n" --filtered-stream-data "$o" > "$o.u3d"
  python3 - "$o.u3d" <<'P'
import struct, sys
d = open(sys.argv[1], "rb").read()
assert d[:4] == b"U3D\0", d[:4]
assert struct.unpack_from("<Q", d, 24)[0] == len(d), "U3D file size field"
print(f"U3D stream {len(d)} bytes")
P
  pdftoppm -r 40 -singlefile "$o" "$o.render"
  python3 - "$o.render.ppm" <<'P'
import sys
d = open(sys.argv[1], "rb").read()
head = d.split(b"\n", 3)
w, h = map(int, head[1].split()); px = head[3]
ink = sum(1 for i in range(0, len(px) - 2, 3) if px[i] < 240 or px[i + 1] < 240 or px[i + 2] < 240)
share = ink / (w * h)
print(f"poster render {w}x{h}, {share:.1%} drawn")
assert share > 0.01, "the poster renders blank"
P
  echo "PDF3D-CHECK ok $(basename "$p") $(stat -c %s "$o") bytes"
done
