#!/usr/bin/env bash
# Fetch the CC0 demo models (release asset fixtures-cc0-1) and verify them.
#
# Usage:
#   tools/ci/fetch-cc0-models.sh TARGET_DIR
#
# Downloads rvt-open-cc0-models-1.tar.gz from the fixtures-cc0-1 release of
# $GITHUB_REPOSITORY (default helenkwok/rvt-open), checks its sha256, unpacks it
# into TARGET_DIR/rvt-open-cc0-models-1/ and checks every file against the
# archive's own SHA256SUMS. The archive's SOURCE.md names where each model comes
# from and its licence. A folder already there with its SHA256SUMS passing is not
# downloaded again. Exits 1 on a mismatch and 2 when the download fails.
set -euo pipefail

TARGET="${1:?usage: fetch-cc0-models.sh TARGET_DIR}"
REPO="${GITHUB_REPOSITORY:-helenkwok/rvt-open}"
TAG=fixtures-cc0-1
NAME=rvt-open-cc0-models-1
SHA=1ceb480db04ab360c31e20585f6c8cbaa50d6e0c61a4e25f4ca4c81979cd8660

mkdir -p "$TARGET"
if [ -f "$TARGET/$NAME/SHA256SUMS" ] && (cd "$TARGET/$NAME" && sha256sum --quiet -c SHA256SUMS); then
  echo "$TARGET/$NAME: present and verified"
  exit 0
fi
tgz="$TARGET/$NAME.tar.gz"
if ! curl -fsSL --retry 3 --retry-all-errors --retry-delay 5 --max-time 600 -o "$tgz" \
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
