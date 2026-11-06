#!/usr/bin/env bash
# Uploads release archives to an online scanning service and writes a Markdown section with the result
# and the report link for each one. Used by .github/workflows/release.yml.
#
#   tools/ci/online-scan.sh virustotal|metadefender|hybrid-analysis <section.md> <archive>...
#
# The service's key comes from the environment (VT_API_KEY, METADEFENDER_API_KEY or
# HYBRID_ANALYSIS_API_KEY) and reaches curl only through a mode-600 header file: never argv, never the log.
# Uploads made this way are public on each service; the archives are public release assets anyway.
# A failed upload is recorded in the section and warned about. This script never fails the release.
# Hybrid Analysis results are printed to the job log only; its scans.md section is a one-line note.
set -uo pipefail
svc=$1 out=$2; shift 2
hdr=$(mktemp); chmod 600 "$hdr"; trap 'rm -f "$hdr"' EXIT
api() { curl -sS --fail --retry 2 --retry-delay 30 -A rvt-open-release -H @"$hdr" "$@"; }
row() { echo "| \`$1\` | $2 |" >> "$out"; }
failed() { echo "::warning::$svc: $2 failed for $1"; row "$1" "$2 failed"; }
section() { printf '## %s\n\n%s\n\n| File | Result |\n|---|---|\n' "$1" "$2" > "$out"; }
size() { stat -c %s "$1"; }

virustotal() {
  printf 'x-apikey: %s\n' "$VT_API_KEY" > "$hdr"
  local base=https://www.virustotal.com/api/v3 f name sum url
  section VirusTotal "Each archive was uploaded through the VirusTotal API. A report can take a few minutes to complete after the release is published."
  for f; do
    name=${f##*/}; sum=$(sha256sum "$f" | cut -d' ' -f1); url=$base/files
    # Free API: 4 requests a minute and 500 a day; direct upload up to 32 MB, larger files through an upload URL.
    if [ "$(size "$f")" -gt 33554432 ]; then
      url=$(api "$base/files/upload_url" | jq -er .data) || { failed "$name" upload; sleep 16; continue; }
      sleep 16
    fi
    if api -F "file=@$f" "$url" | jq -e .data.id > /dev/null; then
      row "$name" "https://www.virustotal.com/gui/file/$sum"
    else failed "$name" upload; fi
    sleep 16
  done
}

metadefender() {
  printf 'apikey: %s\n' "$METADEFENDER_API_KEY" > "$hdr"
  local base=https://api.metadefender.com/v4 f name id r limit_mb mb i
  section "MetaDefender Cloud" "Each archive was scanned by MetaDefender Cloud's multi-engine scan. The result is the verdict and the number of engines that flagged the file out of those that scanned it."
  # The key's own limits (upload size in MB, daily Prevention API calls) decide what can be sent.
  limit_mb=$(api "$base/apikey/" | jq -r '.max_archive_file_size // .max_upload_file_size // empty') || limit_mb=
  echo "MetaDefender key limits: archive upload ${limit_mb:-unknown} MB"
  for f; do
    name=${f##*/}; mb=$(( ($(size "$f") + 1048575) / 1048576 ))
    if [ -n "$limit_mb" ] && [ "$mb" -gt "$limit_mb" ]; then row "$name" "not scanned: $mb MB is over the key's $limit_mb MB limit"; continue; fi
    id=$(api -H "filename: $name" -F "file=@$f" "$base/file" | jq -er .data_id) || { failed "$name" upload; continue; }
    r=
    for i in $(seq 1 20); do
      sleep 20
      r=$(api "$base/file/$id") || continue
      [ "$(jq -r '.scan_results.progress_percentage // 0' <<< "$r")" = 100 ] && break
    done
    row "$name" "$(jq -r '"\(.scan_results.scan_all_result_a // "scan still running") (\(.scan_results.total_detected_avs // "?") of \(.scan_results.total_avs // "?") engines)"' <<< "${r:-null}"), https://metadefender.opswat.com/results/file/$id/regular/overview"
  done
}

hybrid_analysis() {
  printf 'api-key: %s\n' "$HYBRID_ANALYSIS_API_KEY" > "$hdr"
  local base=https://hybrid-analysis.com/api/v2 f name sum id r i env quick sandbox
  # Results go to the job log only; scans.md gets a one-line note.
  printf '## Hybrid Analysis\n\nHybrid Analysis (CrowdStrike Falcon Sandbox): scanned in the release workflow; results in the workflow log.\n' > "$out"
  out=/dev/stdout
  echo "Hybrid Analysis key level: $(api "$base/key/current" | jq -r '.auth_level_name // "unknown"')"
  echo "Hybrid Analysis submission quota: $(api "$base/key/submission-quota" | jq -c . 2>/dev/null)"
  printf 'Hybrid Analysis: quick scan of each archive, and a Falcon Sandbox submission of the Windows, Linux x86_64\nand macOS arm64 archives in a matching environment when the key quota allows it.\n\n| File | Result |\n|---|---|\n'
  for f; do
    name=${f##*/}; sum=$(sha256sum "$f" | cut -d' ' -f1); quick="quick scan failed"; sandbox=
    if id=$(api -F scan_type=all -F "submit_name=$name" -F "file=@$f" "$base/quick-scan/file" | jq -er .id); then
      r=
      for i in $(seq 1 15); do
        r=$(api "$base/quick-scan/$id") || { sleep 20; continue; }
        [ "$(jq -r .finished <<< "$r")" = true ] && break
        sleep 20
      done
      quick="quick scan: $(jq -r '[.scanners_v2 // {} | to_entries[] | select(.value != null) | "\(.value.name // .key) \(.value.status // "?")"] | join(", ") | if . == "" then "no scanner results yet" else . end' <<< "${r:-null}")"
    else echo "::warning::hybrid-analysis: quick scan failed for $name"; fi
    case $name in
      *windows*.zip) env=160 ;;               # Windows 10 64 bit
      *x86_64-unknown-linux-gnu*) env=330 ;;  # Linux (Ubuntu 24.04, 64 bit)
      *aarch64-apple-darwin*) env=430 ;;      # macOS Tahoe (ARM64)
      *) env= ;;
    esac
    if [ -n "$env" ]; then
      if api -F "environment_id=$env" -F "file=@$f" "$base/submit/file" | jq -e .job_id > /dev/null; then sandbox="; sandbox submitted (environment $env)"
      else echo "::warning::hybrid-analysis: sandbox submission failed for $name"; sandbox="; sandbox submission failed"; fi
    fi
    row "$name" "$quick$sandbox, https://hybrid-analysis.com/sample/$sum"
    sleep 5
  done
}

case $svc in
  virustotal) virustotal "$@" ;;
  metadefender) metadefender "$@" ;;
  hybrid-analysis) hybrid_analysis "$@" ;;
  *) echo "unknown service: $svc" >&2; exit 2 ;;
esac
exit 0
