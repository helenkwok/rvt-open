#!/usr/bin/env bash
# Fetch the public Revit samples public CI reads, check each against its pin, and lay them out for the tests.
# Usage: tools/ci/fetch-public-samples.sh DIR   (prints the RVT_* variables to add to the environment)
# Sources: Autodesk's own sample downloads (research/autodesk-sample-projects.tsv) and VTT's BIM4LCA models
# (research/bim4lca-models.tsv, CC BY-SA 4.0). No credentials. A file that cannot be fetched is left out, and the
# tests that read it skip; a file that no longer matches its pin fails.
set -euo pipefail
d=${1:?usage: fetch-public-samples.sh DIR}; root=$(cd "$(dirname "$0")/../.." && pwd); mkdir -p "$d"; d=$(cd "$d" && pwd)
"$root/tools/fetch-autodesk-samples.sh" "$d/flat" >&2
# By year (RVT_AUTODESK_SAMPLES_DIR=<dir>/<year>/<file>, RVT_REVIT_<year>_DIR) and Revit 2024 under its install names.
for f in "$d"/flat/*.rvt; do b=$(basename "$f"); y=${b%%-*}; n=${b#*-}; mkdir -p "$d/by-year/$y"; ln -sf "$f" "$d/by-year/$y/$n"; done
# Both spellings Autodesk has used (racbasicsampleproject.rvt, rac_basic_sample_project.rvt).
for f in "$d"/by-year/*/r[as][ct]*.rvt "$d"/by-year/*/rme*.rvt; do [ -e "$f" ] || continue; n=$(basename "$f")
  a=$(sed -E -e 's/^(rac|rst|rme)(basic|advanced)sampleproject\.rvt$/\1_\2_sample_project.rvt/' -e t -e 's/^(rac|rst|rme)_(basic|advanced)_sample_project\.rvt$/\1\2sampleproject.rvt/' <<<"$n")
  [ "$a" != "$n" ] && [ ! -e "$(dirname "$f")/$a" ] && ln -s "$n" "$(dirname "$f")/$a"; done
mkdir -p "$d/2024"
for f in "$d"/by-year/2024/*.rvt; do n=$(basename "$f")
  case $n in snowdon_towers_sample_*) s=${n#snowdon_towers_sample_}; s=${s%.rvt}
    [ "$s" = hvac ] && s=HVAC || s="$(tr '[:lower:]' '[:upper:]' <<<"${s:0:1}")${s:1}"; n="Snowdon Towers Sample $s.rvt";; esac
  ln -sf "$f" "$d/2024/$n"; done
set +e; python3 "$root/tools/ci/fetch-bim4lca.py" "$root/research/bim4lca-models.tsv" "$d/bim4lca" >&2; st=$?; set -e
[ $st != 0 ] && [ $st != 2 ] && { echo "BIM4LCA models no longer match their pins" >&2; exit 1; }
echo "RVT_AUTODESK_SAMPLES_DIR=$d/by-year"
echo "RVT_REVIT2024_SAMPLES_DIR=$d/2024"
for y in "$d"/by-year/*; do echo "RVT_REVIT_$(basename "$y")_DIR=$y"; done
[ $st = 0 ] && echo "RVT_NORDIC_LCA_DIR=$d/bim4lca" || echo "BIM4LCA host unreachable: those tests skip" >&2
