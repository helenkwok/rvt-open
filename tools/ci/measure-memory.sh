#!/usr/bin/env bash
# Peak memory and time of rvt-ifc and rvt-gltf on one model, before and after a change,
# for the Memory workflow (.github/workflows/memory.yml).
#
# Usage:
#   tools/ci/measure-memory.sh BASE_BIN HEAD_BIN OUT_DIR FILE [heaptrack]
#
# Runs these variants on FILE: the base build; the head build as it is; the head build with
# RVT_SAVED_SCENE_BATCH=1024 and =4096 (saved graphics extracted that many elements at a time);
# and the head build with MALLOC_ARENA_MAX=2 (glibc keeps two heaps, so freed memory is
# reused across threads instead of held per thread). Each variant
# writes IFC, IFCX and .frag (rvt-ifc, --ifc4-timestamp 1970-01-01T00:00:00Z) and a .glb (rvt-gltf) under GNU
# time. OUT_DIR gets:
#   summary.md         one row per variant: peak resident memory (MiB), seconds, and whether
#                      every output's sha256 equals the base build's
#   stages-*.tsv       each head variant's peak per stage (RVT_STAGE_MEMORY), rvt-ifc and rvt-gltf
#   heaptrack-*.txt    with "heaptrack": the head rvt-ifc's largest allocators at its peak
#                      (slow, and heaptrack_print needs several GB on the largest models)
# Exit 1 when an export fails or an output differs from the base build's.
set -uo pipefail
BASE="$1"; HEAD="$2"; OUT="$3"; FILE="$4"; HT="${5:-}"
mkdir -p "$OUT"
name=$(basename "$FILE" .rvt | tr ' ' '_')
tmp=$(mktemp -d)
status=0

run() { # variant bin_dir env...
  local v="$1" bin="$2"; shift 2
  mkdir -p "$tmp/$v"
  echo "$name $v: $(date -u +%T) start"
  env "$@" RVT_STAGE_MEMORY=1 /usr/bin/time -f '%M %e' -o "$tmp/$v/ifc.time" \
    "$bin/rvt-ifc" "$FILE" -o "$tmp/$v/model.ifc" --ifcx "$tmp/$v/model.ifcx" --frag "$tmp/$v/model.frag" \
    --ifc4-timestamp 1970-01-01T00:00:00Z 2> "$tmp/$v/ifc.err" > /dev/null || { echo "::error::$name $v: rvt-ifc failed"; tail -5 "$tmp/$v/ifc.err"; status=1; }
  env "$@" RVT_STAGE_MEMORY=1 /usr/bin/time -f '%M %e' -o "$tmp/$v/gltf.time" \
    "$bin/rvt-gltf" "$FILE" -o "$tmp/$v/model.glb" 2> "$tmp/$v/gltf.err" > /dev/null || { echo "::error::$name $v: rvt-gltf failed"; tail -5 "$tmp/$v/gltf.err"; status=1; }
  (cd "$tmp/$v" && sha256sum model.ifc model.ifcx model.frag model.glb > sha256.txt 2> /dev/null)
  cp "$tmp/$v/sha256.txt" "$OUT/sha256-$v.txt"
  grep -h '^stage-memory' "$tmp/$v/ifc.err" | cut -f2- > "$OUT/stages-ifc-$v.tsv"
  grep -h '^stage-memory' "$tmp/$v/gltf.err" | cut -f2- > "$OUT/stages-gltf-$v.tsv"
  rm -f "$tmp/$v"/model.*
  echo "$name $v: $(date -u +%T) done (rvt-ifc $(cat "$tmp/$v/ifc.time" 2> /dev/null), rvt-gltf $(cat "$tmp/$v/gltf.time" 2> /dev/null); KiB s)"
}

variants="base head head-batch1024 head-batch4096 head-arena2"
run base "$BASE"
run head "$HEAD"
run head-batch1024 "$HEAD" RVT_SAVED_SCENE_BATCH=1024
run head-batch4096 "$HEAD" RVT_SAVED_SCENE_BATCH=4096
run head-arena2 "$HEAD" MALLOC_ARENA_MAX=2

{
  echo "### $name ($(du -m "$FILE" | cut -f1) MB)"
  echo
  echo "| build | rvt-ifc peak (MiB) | rvt-ifc time (s) | rvt-gltf peak (MiB) | rvt-gltf time (s) | outputs equal to base |"
  echo "|---|---:|---:|---:|---:|---|"
  for v in $variants; do
    read -r ik is < "$tmp/$v/ifc.time" 2> /dev/null || { ik=0; is=-; }
    read -r gk gs < "$tmp/$v/gltf.time" 2> /dev/null || { gk=0; gs=-; }
    if cmp -s "$OUT/sha256-base.txt" "$OUT/sha256-$v.txt" && [ "$(wc -l < "$OUT/sha256-$v.txt")" = 4 ]; then same=yes
    else same="**no**: $(diff "$OUT/sha256-base.txt" "$OUT/sha256-$v.txt" | sed -nE 's/^> [0-9a-f]+ +//p' | tr '\n' ' ')"; status=1; fi
    echo "| $v | $((ik / 1024)) | $is | $((gk / 1024)) | $gs | $same |"
  done
  for tool in ifc gltf; do
    for v in ${variants#base }; do
      f="$OUT/stages-$tool-$v.tsv"; [ -s "$f" ] || continue
      echo
      echo "rvt-$tool, $v, by stage (peak resident memory during the stage, resident at its end):"
      echo
      echo "| stage | seconds | resident at end (MiB) | peak (MiB) |"
      echo "|---|---:|---:|---:|"
      awk -F'\t' '{ printf "| %s | %s | %s | %s |\n", $1, $2, $3, $4 }' "$f"
    done
  done
  echo
} > "$OUT/summary.md"

if [ "$HT" = heaptrack ]; then
  heaptrack -o "$tmp/ht" "$HEAD/rvt-ifc" "$FILE" -o "$tmp/ht.ifc" --ifcx "$tmp/ht.ifcx" --frag "$tmp/ht.frag" \
    --ifc4-timestamp 1970-01-01T00:00:00Z > /dev/null 2> "$OUT/heaptrack-ifc.err" || echo "::warning::$name: heaptrack run failed"
  ht=$(ls "$tmp"/ht.*zst "$tmp"/ht.*gz 2> /dev/null | head -1)
  if [ -n "$ht" ]; then
    heaptrack_print -f "$ht" --print-allocators false --print-temporary false --print-leaks false \
      --peak-limit 25 --sub-peak-limit 8 > "$OUT/heaptrack-ifc.txt" 2>&1 \
      || heaptrack_print -f "$ht" > "$OUT/heaptrack-ifc.txt" 2>&1
    cp "$ht" "$OUT/"
    { echo "rvt-ifc (head) heaptrack, largest allocators at the peak: heaptrack-ifc.txt in the artifact."; echo
      echo '```'; grep -m1 -i 'peak heap memory' "$OUT/heaptrack-ifc.txt"; sed -n '/PEAK MEMORY CONSUMERS/,+60p' "$OUT/heaptrack-ifc.txt" | cut -c1-200; echo '```'; echo; } >> "$OUT/summary.md"
  fi
fi
cat "$OUT/summary.md"
rm -rf "$tmp"
exit $status
