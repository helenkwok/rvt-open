#!/usr/bin/env bash
# Read models whose partition streams inflate past 256 MiB, for the Measure
# workflow (.github/workflows/measure.yml).
#
# Usage:
#   tools/ci/measure-large-partitions.sh BIN_DIR OUT_DIR FILE...
#
# There is no fixed cap on how large a partition may be, so a model with a
# large partition must read like any other. For each FILE: rvt-dump gives
# every partition's stored and inflated size, as the
# library's own decoders inflate it (OUT_DIR/<file>-partitions.tsv),
# then rvt-ifc and rvt-gltf export it with the default limits, under GNU time
# for peak resident memory and wall time. OUT_DIR/summary.md gets one row per
# file. Exit 1 when any file opens empty: an export that fails, has no
# building elements or none with geometry, or leaves a stream unread. When no
# FILE has a partition over 256 MiB the summary says so and names the largest.
# FILEs that do not exist are skipped.
set -uo pipefail
BIN="$1"; OUT="$2"; shift 2
LIMIT=$((256 << 20))
mkdir -p "$OUT"
tmp=$(mktemp -d)
status=0 over=0 largest=0 largest_name=-
{
  echo "| file | partitions | largest partition | stored (MiB) | inflated (MiB) | over 256 MiB | rvt-ifc exit | building elements | with geometry | storeys | unread streams | ifc peak (MiB) | ifc time (s) | rvt-gltf exit | glb elements | gltf peak (MiB) | gltf time (s) |"
  echo "|---|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|"
} > "$OUT/summary.md"

mib() { awk -v b="$1" 'BEGIN { printf "%.1f", b / 1048576 }'; }

for file in "$@"; do
  [ -f "$file" ] || continue
  name=$(basename "$file" .rvt | tr ' ' '_')
  echo "::group::$name"
  rm -rf "$tmp/dump"
  "$BIN/rvt-dump" "$file" -o "$tmp/dump" > "$tmp/dump.out" || { echo "::error::$name: rvt-dump failed"; status=1; }
  tsv="$OUT/$name-partitions.tsv"
  printf 'partition\tstored_bytes\tinflated_bytes\n' > "$tsv"
  # "  Partitions/68  raw=N bytes  decomp=M bytes ..."; a partition that did not inflate counts 0.
  awk '$1 ~ /^Partitions\// { s = $2; sub(/raw=/, "", s); i = 0
         if ($4 ~ /^decomp=/) { i = $4; sub(/decomp=/, "", i) }
         print $1 "\t" s "\t" i }' "$tmp/dump.out" | sort -t$'\t' -k3,3nr >> "$tsv"
  rm -rf "$tmp/dump"
  count=$(($(wc -l < "$tsv") - 1))
  read -r top stored inflated < <(sed -n 2p "$tsv")
  n_over=$(awk -F'\t' -v l="$LIMIT" 'NR > 1 && $3 > l { n++ } END { print n + 0 }' "$tsv")
  over=$((over + n_over))
  [ "${inflated:-0}" -gt "$largest" ] && { largest=$inflated; largest_name="$name $top"; }
  echo "largest partition: $top stored ${stored:-0} inflated ${inflated:-0}; over 256 MiB: $n_over"

  /usr/bin/time -f '%M %e' -o "$tmp/ifc.time" "$BIN/rvt-ifc" "$file" -o "$tmp/model.ifc" --diagnostics "$tmp/d.json"
  ifc=$?
  read -r ifc_kib ifc_s < "$tmp/ifc.time" 2> /dev/null || { ifc_kib=0; ifc_s=-; }
  read -r elems geom storeys unread < <(python3 - "$tmp/d.json" <<'EOF'
import json, sys
try:
    d = json.load(open(sys.argv[1]))
except Exception:
    print("0 0 0 -"); sys.exit()
e = d["exported"]
unread = sum(i["count"] for i in d["skipped"] if i["reason"] == "stream_not_read_resource_limit")
print(e["building_elements"], e["building_elements_with_geometry"], e["storey_count"], unread)
EOF
  )
  /usr/bin/time -f '%M %e' -o "$tmp/gltf.time" "$BIN/rvt-gltf" "$file" -o "$tmp/model.glb" --verbose > "$tmp/gltf.out"
  gltf=$?
  read -r gltf_kib gltf_s < "$tmp/gltf.time" 2> /dev/null || { gltf_kib=0; gltf_s=-; }
  glb_elems=$(sed -nE 's/.*\(([0-9]+) building elements\).*/\1/p' "$tmp/gltf.out")
  rm -f "$tmp/model.ifc" "$tmp/model.glb" "$tmp/d.json"

  echo "| $name | $count | $top | $(mib "${stored:-0}") | $(mib "${inflated:-0}") | $n_over | $ifc | $elems | $geom | $storeys | $unread | $((ifc_kib / 1024)) | $ifc_s | $gltf | ${glb_elems:-0} | $((gltf_kib / 1024)) | $gltf_s |" >> "$OUT/summary.md"
  if [ "$ifc" != 0 ] || [ "$elems" = 0 ] || [ "$geom" = 0 ] || [ "$unread" != 0 ] || [ "$gltf" != 0 ] || [ "${glb_elems:-0}" = 0 ]; then
    echo "::error::$name opened empty or incomplete (rvt-ifc exit $ifc, $elems elements, $geom with geometry, $unread unread streams; rvt-gltf exit $gltf, ${glb_elems:-0} elements)"
    status=1
  fi
  echo "::endgroup::"
done
if [ "$over" = 0 ]; then
  echo "" >> "$OUT/summary.md"
  echo "No partition over 256 MiB inflated in these files. The largest is $largest_name, $(mib "$largest") MiB." >> "$OUT/summary.md"
fi
rm -rf "$tmp"
exit $status
