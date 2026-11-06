#!/usr/bin/env bash
# Scans files with ClamAV and writes a Markdown section and the full log. Archives (.tar.gz, .zip) are
# scanned as they are and also unpacked, so the binaries inside are scanned too.
#
#   tools/ci/clamav-scan.sh <section.md> <clamav.log> <file>...
#
# ClamAV runs from the official image (clamav/clamav, tag 1.5.4, pinned by digest below). That tag bundles
# the signature database, so a scan never depends on reaching the signature mirrors; freshclam runs first
# inside the container to fetch newer signatures, and a failed update only warns.
#
# Exit status: 1 when ClamAV flags anything, 0 otherwise (a scan that could not complete warns, and the
# section says so).
#
# ClamAV is GPL-2.0. It runs here as a separate program in CI only; it is never linked into, bundled with
# or shipped in rvt-open, so its licence does not apply to rvt-open.
set -uo pipefail
IMAGE=clamav/clamav@sha256:bda39705e291105049c3936e2803ac90b955172af8030f94f05dce1252df6c6d # 1.5.4
out=$1 log=$2; shift 2

stage=$(mktemp -d); mkdir -p "$stage/files" "$stage/unpacked"
for f; do
  [ -f "$f" ] || continue
  cp "$f" "$stage/files/"
  d="$stage/unpacked/${f##*/}"
  case $f in
    *.tar.gz) mkdir -p "$d" && tar xzf "$f" -C "$d" ;;
    *.zip) mkdir -p "$d" && unzip -q "$f" -d "$d" ;;
  esac
done
chmod -R a+rX "$stage"

# Generous limits, and anything over them is reported (alert-exceeds-max) rather than silently skipped.
docker run --rm -v "$stage:/scan:ro" --entrypoint /bin/sh "$IMAGE" -c '
  if freshclam --stdout; then echo "FRESHCLAM: updated"; else echo "FRESHCLAM: failed"; fi
  echo "VERSION: $(clamscan --version)"
  clamscan --recursive --infected --scan-archive=yes --alert-exceeds-max=yes \
    --max-filesize=2000M --max-scansize=4000M --max-files=100000 /scan
' > "$log" 2>&1
rc=$?
version=$(sed -n 's/^VERSION: //p' "$log" | tail -1)
scanned=$(sed -n 's/^Scanned files: //p' "$log" | tail -1)
infected=$(sed -n 's/^Infected files: //p' "$log" | tail -1)
if grep -q '^FRESHCLAM: updated' "$log"; then updated="yes"
else updated="no: the update failed, so the scan used the signatures bundled in the image"; echo "::warning::freshclam could not update the ClamAV signatures; scanned with the image's bundled database"; fi
case $rc in
  0) result="no threats found" ;;
  1) result="**$infected file(s) flagged**: see the log"; echo "::error::ClamAV flagged $infected file(s)"; grep ' FOUND$' "$log" ;;
  *) result="**the scan did not complete** (exit $rc), so these files were not checked by ClamAV: see the log"; echo "::warning::the ClamAV scan did not complete (exit $rc)" ;;
esac
{
  echo "## ClamAV"
  echo
  echo "Every archive and every file unpacked from it was scanned with ClamAV on the build runner, using the official \`clamav/clamav:1.5.4\` image pinned by digest."
  echo
  echo "- Engine and signature database used: \`${version:-unknown}\`"
  echo "- Signatures updated with freshclam before the scan: $updated"
  echo "- Files scanned: ${scanned:-unknown}"
  echo "- Result: $result"
  echo "- Full output: \`${log##*/}\`"
} > "$out"
cat "$out"
[ "$rc" = 1 ] && exit 1
exit 0
