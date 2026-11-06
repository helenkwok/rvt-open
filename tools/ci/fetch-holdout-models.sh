#!/usr/bin/env bash
# Fetch the held-out models (research/holdout-models.tsv) for a Measure
# run and verify them.
#
# Usage:
#   tools/ci/fetch-holdout-models.sh PINS TARGET_DIR
#
# Downloads each file PINS names into TARGET_DIR/holdout/ and checks its sha256
# and byte count. A url `notice:<key>/<path>` is the "Fetch base (<key>):" URL in
# NOTICE's Test data section followed by <path>. Exits 1 when a file does not match its pin and 2 when one
# could not be downloaded (NOT MEASURED).
set -euo pipefail

PINS="${1:?usage: fetch-holdout-models.sh PINS TARGET_DIR}"
TARGET="${2:?usage: fetch-holdout-models.sh PINS TARGET_DIR}"
sha256() { if command -v sha256sum > /dev/null; then sha256sum "$1"; else shasum -a 256 "$1"; fi | cut -d' ' -f1; }
NOTICE_FILE="$(cd "$(dirname "$0")/../.." && pwd)/NOTICE"
mkdir -p "$TARGET/holdout"
while IFS=$'\t' read -r name url sha bytes; do
  if [[ "$url" == notice:* ]]; then
    key="${url#notice:}"; key="${key%%/*}"
    base=$(sed -n "s/^ *Fetch base ($key): *//p" "$NOTICE_FILE" | head -1)
    if [ -z "$base" ]; then
      echo "$name: no Fetch base ($key) in $NOTICE_FILE"
      exit 1
    fi
    url="$base${url#notice:"$key"/}"
  fi
  file="$TARGET/holdout/$name"
  if ! curl -fsSL --retry 3 --retry-all-errors --retry-delay 5 --max-time 900 -o "$file" "$url"; then
    echo "could not download $name"
    rm -f "$file"
    exit 2
  fi
  got_sha=$(sha256 "$file")
  got_bytes=$(wc -c < "$file" | tr -d ' ')
  if [ "$got_sha" != "$sha" ] || [ "$got_bytes" != "$bytes" ]; then
    echo "$name: got $got_bytes bytes sha256 $got_sha, pinned $bytes bytes $sha"
    rm -f "$file"
    exit 1
  fi
  echo "$name: matches its pin"
done < <(grep -v '^#' "$PINS")
