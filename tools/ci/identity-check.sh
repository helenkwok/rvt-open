#!/usr/bin/env bash
# Fails if the tree carries names it must not: the private deny-list in the
# IDENTITY_DENYLIST environment variable (a PCRE, case-insensitive; CI sets it
# from the repository secret of the same name), tracker ids, writer wording,
# the Python name `rvt` (another project's PyPI package), a crates.io package
# called `rvt`, or an npm name someone has published.
# With --commits BASE..HEAD it checks commit messages instead: the deny-list,
# tracker ids, issue refs (#123) and AI attribution lines.
# The Rust [lib] name `rvt` is fine. Prints each hit as file:line.
#
# An empty IDENTITY_DENYLIST fails, except on a pull request from a fork
# (IDENTITY_FORK_PR=true), where secrets are not available: the deny-list
# part is then skipped with a notice and the other checks still run.
set -u
cd "$(git rev-parse --show-toplevel)"
fail=0

deny=${IDENTITY_DENYLIST:-}
deny=${deny//$'\n'/}
deny=${deny//$'\r'/}
if [ -z "$deny" ]; then
  if [ "${IDENTITY_FORK_PR:-}" = true ]; then
    echo "::notice::identity check: IDENTITY_DENYLIST is not available on fork pull requests; skipping the deny-list"
  else
    echo "identity check: IDENTITY_DENYLIST is empty; set the repository secret (failing closed)" >&2
    fail=1
  fi
fi

# Generic checks, safe to publish.
ids='\b(RE|VW1|L5B|SEC|COR|DEC|WRT|M3)-[0-9]+\b|// Tracker:'
spike='\bspike[_ ]?[0-9]{3}\b'
writer='write-back|WRITE_ENABLED|rvt-write|wasm-no-write|round-trip.*(rvt|revit)'
pyname='`rvt` (module|package|wheel)'

# --commits A..B: check commit messages in the range instead of the tree.
if [ "${1:-}" = "--commits" ]; then
  grep() { command "${PCRE_GREP:-grep}" "$@"; }
  echo x | grep -qP x || { echo "identity check: grep -P is unavailable (set PCRE_GREP)" >&2; exit 1; }
  range=${2:?usage: identity-check.sh --commits BASE..HEAD}
  bad="#[0-9]{2,5}([^0-9]|\$)|Co-Author[e]d-By:|[C]laude-Session|Generated with|\\bspike\\b|\\bADR[ -]?[0-9]+|(^|[^A-Za-z0-9_./-])(import ([A-Za-z0-9_.]+, *)*rvt|from rvt)([ .,;]|\$)"
  for c in $(git rev-list "$range"); do
    msg=$(git log -1 --format=%B "$c")
    short=$(git rev-parse --short "$c")
    printf '%s\n' "$msg" | grep -nP "$ids" | sed "s/^/commit $short:/" | grep . && fail=1
    printf '%s\n' "$msg" | grep -niP "$bad" | sed "s/^/commit $short:/" | grep . && fail=1
    if [ -n "$deny" ]; then
      printf '%s\n' "$msg" | grep -qiP "$deny" && { echo "commit $short: message matches the deny-list"; fail=1; }
    fi
  done
  [ $fail = 0 ] && echo "identity check (commits $range): clean"
  exit $fail
fi

hit() { [ -n "$1" ] && { printf '%s\n' "$1"; fail=1; }; }
x=(':!LICENSE' ':!NOTICE' ':!tools/ci/identity-check.sh')

if [ -n "$deny" ]; then
  hit "$(git grep -nIiP "$deny" -- "${x[@]}")"
fi
hit "$(git grep -nIP "$ids" -- "${x[@]}")"
hit "$(git grep -nIiP "$spike" -- "${x[@]}")"
hit "$(git grep -nIiP "$writer" -- "${x[@]}")"
hit "$(git grep -nIP "$pyname" -- "${x[@]}")"
hit "$(git grep -nIE '^[[:space:]]*(name[[:space:]]*=[[:space:]]*"rvt"|module-name[[:space:]]*=[[:space:]]*"rvt\.)' -- 'pyproject.toml' '**/pyproject.toml')"
hit "$(git grep -nIE '(^|[^A-Za-z0-9_./-])(import ([A-Za-z0-9_.]+, *)*rvt|from rvt)([ .,;]|$)' -- '*.py' '*.pyi' '*.md' '*.ipynb')"
# crates.io package names: `name = "rvt"` directly under [package].
for f in $(git ls-files 'Cargo.toml' '**/Cargo.toml'); do
  hit "$(awk -v f="$f" '/^\[/{s=$0} s=="[package]" && /^name *= *"rvt"/{print f":"NR":"$0}' "$f")"
done
# npm: a package name already published by someone else.
for f in $(git ls-files 'package.json' '**/package.json' | grep -v node_modules); do
  n=$(sed -n 's/^  "name": *"\([^"]*\)".*/\1/p' "$f" | head -1)
  [ -z "$n" ] && continue
  code=$(curl -s -o /dev/null -w '%{http_code}' --max-time 20 "https://registry.npmjs.org/${n/\//%2f}")
  case $code in
    404) ;;
    200) hit "$f:$(grep -n '"name"' "$f" | head -1 | cut -d: -f1): npm name \"$n\" is already published" ;;
    *) echo "warning: npm registry check for $n returned $code" ;;
  esac
done
[ $fail = 0 ] && echo "identity check: clean"
exit $fail
