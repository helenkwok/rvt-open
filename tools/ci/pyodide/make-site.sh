#!/usr/bin/env bash
# Export example notebook 01 as a static site that runs in the browser (marimo html-wasm, Pyodide).
# Usage: tools/ci/pyodide/make-site.sh WHEEL OUT_DIR MODEL_URL
#   WHEEL      the Pyodide wheel (tools/ci/pyodide/build-wheel.sh)
#   OUT_DIR    the site is written here (serve it from any static host; nothing is bundled but the wheel)
#   MODEL_URL  where the page fetches the Revit file from: a host that allows cross-origin reads (CORS)
# marimo must be on PATH (tools/ci/pyodide/requirements.txt). The notebook is the one CI runs, unchanged, plus one cell
# first that fetches the model into the browser's file system at the path the notebook reads.
set -euo pipefail
wheel=$(cd "$(dirname "${1:?wheel}")" && pwd)/$(basename "$1"); url=${3:?model url}
mkdir -p "${2:?out dir}"; out=$(cd "$2" && pwd)
here=$(cd "$(dirname "$0")" && pwd); root=$(cd "$here/../../.." && pwd)
nb=$root/examples/notebooks/01_open_a_model.py
path=$(sed -n 's/.*os.environ.get("RVT_MODEL", "\([^"]*\)").*/\1/p' "$nb")
[ -n "$path" ] || { echo "no default model path in $nb" >&2; exit 1; }
w=$(mktemp -d); trap 'rm -rf "$w"' EXIT
cp "$wheel" "$here/rvt_boot.py" "$w/"
{
  echo '# /// script'; echo '# requires-python = ">=3.14"'
  echo "# dependencies = [\"marimo\", \"rvt-open @ ./$(basename "$wheel")\"]"; echo '# ///'
  python3 - "$nb" "$path" "$url" <<'PY'
import sys
nb, path, url = sys.argv[1:4]
s = open(nb).read()
marker = 'app = marimo.App(width="medium")\n'
i = s.index(marker) + len(marker)
cell = '\n\n@app.cell\ndef _():\n    import rvt_boot\n    _ = rvt_boot.stage(%r, %r)\n    return\n' % (path, url)
sys.stdout.write(s[:i] + cell + s[i:])
PY
} > "$w/notebook.py"
(cd "$w" && marimo export html-wasm --no-sandbox notebook.py -o "$out" --mode run -f)
du -sh "$out"
