#!/usr/bin/env bash
# change-scope.sh: prints `skip=true` when the Rust jobs of CI cannot be
# affected by this run, else `skip=false`, for $GITHUB_OUTPUT.
#
# - A pull request whose every changed file is prose that nothing reads
#   (changelog fragments, Markdown pages under docs/ and Markdown files at the
#   root, none named by any code, test, tool or workflow) runs no Rust job.
# - A push to main whose tree is the tree of a pull request head that already
#   passed CI runs no Rust job again: that tree has been tested.
# Any doubt, or an answer that does not come, runs everything.
# Needs GH_TOKEN, GH_REPO, EVENT, and PR (pull requests) or SHA (pushes).
set -uo pipefail
answer() { echo "skip=$1"; exit 0; }
case "${EVENT:-}" in
  pull_request)
    files=$(gh api "repos/$GH_REPO/pulls/$PR/files" --paginate --jq '.[] | .filename, (.previous_filename // empty)') || answer false
    [ -n "$files" ] || answer false
    while IFS= read -r f; do
      case "$f" in
        changelog.d/*.md | docs/*.md | docs/*/*.md | [A-Za-z]*.md) ;;
        *) answer false ;;
      esac
      case "$f" in */*) ;; *) [ "$f" = README.md ] && answer false ;; esac
      # A page a test, tool or workflow reads is not only prose.
      if git grep -q -F "$(basename "$f")" -- src tests tools examples .github rvt-py rvt-wasm rvt-open-mcp rvt-dwg viewer/tests viewer/src 2>/dev/null; then
        answer false
      fi
    done <<< "$files"
    answer true
    ;;
  push)
    tree=$(gh api "repos/$GH_REPO/commits/$SHA" --jq .commit.tree.sha) || answer false
    runs=$(gh run list --repo "$GH_REPO" --workflow CI --event pull_request --status success --limit 30 --json databaseId,headSha --jq '.[] | "\(.databaseId) \(.headSha)"') || answer false
    while read -r id s; do
      [ -n "$s" ] || continue
      t=$(gh api "repos/$GH_REPO/commits/$s" --jq .commit.tree.sha 2>/dev/null) || continue
      [ "$t" = "$tree" ] || continue
      # Only a run that ran the full test jobs counts (a draft's run does not).
      full=$(gh api "repos/$GH_REPO/actions/runs/$id/jobs" --paginate --jq '[.jobs[] | select(.name | startswith("tests ")) | .conclusion] | length > 0 and all(. == "success")' 2>/dev/null)
      [ "$full" = true ] && answer true
    done <<< "$runs"
    answer false
    ;;
esac
answer false
