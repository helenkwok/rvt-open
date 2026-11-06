#!/usr/bin/env bash
# Fetch the tiny reference models (release asset fixtures-tiny-1, MIT) and verify them.
#
# Usage:
#   tools/ci/fetch-tiny-models.sh TARGET_DIR
#
# Downloads rvt-open-tiny-models-1.tar.gz from the fixtures-tiny-1 release of
# $GITHUB_REPOSITORY (default helenkwok/rvt-open), checks its sha256, unpacks it
# into TARGET_DIR/rvt-open-tiny-models-1/ and checks every file against the
# archive's own SHA256SUMS. With the folder already there and its SHA256SUMS
# passing, nothing is downloaded. Exits 1 on a mismatch and 2 when the download fails.
set -euo pipefail

TARGET="${1:?usage: fetch-tiny-models.sh TARGET_DIR}"
TAG=fixtures-tiny-1
NAME=rvt-open-tiny-models-1
SHA=67f06a30110313174c16c6ea3a4e21c29d34a42a30afd51877d849199e843d33
REPO="${GITHUB_REPOSITORY:-helenkwok/rvt-open}"

mkdir -p "$TARGET"
if [ -f "$TARGET/$NAME/SHA256SUMS" ] && (cd "$TARGET/$NAME" && sha256sum --quiet -c SHA256SUMS); then
  echo "$TARGET/$NAME: present and verified"
  exit 0
fi
tgz="$TARGET/$NAME.tar.gz"
if ! curl -fsSL --retry 3 --retry-all-errors --retry-delay 5 --max-time 900 -o "$tgz" \
    "https://github.com/$REPO/releases/download/$TAG/$NAME.tar.gz"; then
  echo "could not download $TAG/$NAME.tar.gz from $REPO"
  exit 2
fi
if ! echo "$SHA  $tgz" | sha256sum -c --quiet -; then
  echo "$tgz: sha256 does not match the pin"
  exit 1
fi
rm -rf "${TARGET:?}/$NAME"
tar -xzf "$tgz" -C "$TARGET" --no-same-owner
rm -f "$tgz"
(cd "$TARGET/$NAME" && sha256sum --quiet -c SHA256SUMS) || { echo "$TARGET/$NAME: SHA256SUMS check failed"; exit 1; }
echo "$TARGET/$NAME: $(find "$TARGET/$NAME" -name '*.rvt' | wc -l) models verified"
