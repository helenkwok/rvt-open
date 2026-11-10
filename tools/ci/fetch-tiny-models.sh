#!/usr/bin/env bash
# Fetch the tiny reference models (release assets fixtures-tiny-1 and fixtures-tiny-2, MIT) and verify them.
#
# Usage:
#   tools/ci/fetch-tiny-models.sh TARGET_DIR
#
# For each set, downloads rvt-open-tiny-models-N.tar.gz from the fixtures-tiny-N
# release of $GITHUB_REPOSITORY (default helenkwok/rvt-open), checks its sha256,
# unpacks it into TARGET_DIR/rvt-open-tiny-models-N/ and checks every file
# against the archive's own SHA256SUMS. A set whose folder is already there with
# its SHA256SUMS passing is not downloaded again. Exits 1 on a mismatch and 2
# when a download fails.
set -euo pipefail

TARGET="${1:?usage: fetch-tiny-models.sh TARGET_DIR}"
REPO="${GITHUB_REPOSITORY:-helenkwok/rvt-open}"
SETS=(
  "fixtures-tiny-1 rvt-open-tiny-models-1 7f22fcc0a1d4753e18245577755c1072830e87df0b8953f7e5f254b2e46de0ab"
  "fixtures-tiny-2 rvt-open-tiny-models-2 9b312cb67db72b2b91c5d4bbaf93db206dfcd2f8f3c45bfa33ca1539bb037956"
)

mkdir -p "$TARGET"
for set in "${SETS[@]}"; do
  read -r TAG NAME SHA <<< "$set"
  if [ -f "$TARGET/$NAME/SHA256SUMS" ] && (cd "$TARGET/$NAME" && sha256sum --quiet -c SHA256SUMS); then
    echo "$TARGET/$NAME: present and verified"
    continue
  fi
  tgz="$TARGET/$NAME.tar.gz"
  if ! curl -fsSL --retry 3 --retry-all-errors --retry-delay 5 --max-time 1800 -o "$tgz" \
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
done
