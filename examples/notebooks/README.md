# Example notebooks

Three [marimo](https://marimo.io) notebooks (reactive, stored as `.py`) on a public sample, using only the public API and the command-line tools:

- [`01_open_a_model.py`](01_open_a_model.py): file information, element counts by category, a table of one category's elements, and the parameters of the element you pick (Python bindings).
- [`02_parameters_across_a_model.py`](02_parameters_across_a_model.py): runs `rvt-schedule --format sqlite`, reads the element and parameter tables with `sqlite3`, lists the parameters of a set with how many elements hold each, and charts elements per level.
- [`03_a_view_and_a_sheet.py`](03_a_view_and_a_sheet.py): lists the plan views and sheets and draws the ones you pick as SVG with `rvt-sheet` (`--view-id` for the plan view, so views that share a name stay apart).

## Set up

The Python bindings are not on PyPI yet: build the wheel, and the two tools, from this checkout.

```bash
python3 -m venv .venv && .venv/bin/pip install --require-hashes -r tools/ci/requirements-notebooks.txt
PATH=.venv/bin:$PATH maturin build --release -o dist && .venv/bin/pip install --no-deps dist/rvt_open-*.whl
cargo build --release --features sqlite --bin rvt-schedule --bin rvt-sheet
tools/fetch-autodesk-samples.sh _samples/flat 2024 2024   # Autodesk's sample projects, each checked against its pinned hash
export RVT_MODEL=_samples/flat/2024-racbasicsampleproject.rvt RVT_BIN_DIR=target/release
.venv/bin/marimo edit examples/notebooks/01_open_a_model.py       # interactive
.venv/bin/python examples/notebooks/01_open_a_model.py            # as a script; fails on any error
```

`RVT_MODEL` is a Revit file (the sample above is the supported starting point) and `RVT_BIN_DIR` the folder with `rvt-schedule` and `rvt-sheet`. Notebooks 01 and 02 need a file with decoded elements (a project; a family or template with none shows empty tables and fails the results cell). Notebook 03 draws a plan view and a sheet when the model has them and says so when it has none. Each notebook ends with a cell that names its results and asserts them, so a run that reads nothing fails (notebook 03 asserts a drawing only for the plan views and sheets the model has).

CI (the `Example notebooks` job) runs every notebook this way on the Revit 2024 basic sample project, then exports each to static HTML (`marimo export html`) and uploads the pages as the `example-notebooks-html` artifact.

## In the browser, with no install

Notebook 01 also runs in a browser: the Python bindings are built as a Pyodide wheel (Python in WebAssembly), and `marimo export html-wasm` turns the notebook into a static site that installs the wheel, fetches a Revit file and runs the notebook's cells in the page. One thread, so the first read of a large file takes several times as long as natively (the 19.7 MB basic sample's first read took about 8 s under Pyodide against 1.8 s natively).

```bash
uv venv --python 3.14 .venv-pyodide && uv pip install --python .venv-pyodide/bin/python --require-hashes -r tools/ci/pyodide/requirements.txt
export PATH=$PWD/.venv-pyodide/bin:$PATH
rustup target add wasm32-unknown-emscripten
tools/ci/pyodide/build-wheel.sh dist-pyodide                      # pins emsdk 5.0.3 and the Pyodide cross-build environment
tools/ci/pyodide/make-site.sh dist-pyodide/rvt_open-*.whl site http://localhost:8000/model.rvt
cp _samples/flat/2024-racbasicsampleproject.rvt site/model.rvt    # or any Revit file; a hosted file's URL works in place of localhost
python3 -m http.server --directory site                           # then open http://localhost:8000
```

- **The model comes from a URL** you give `make-site.sh`, fetched by the page, so that host must allow cross-origin reads (CORS) unless it serves the page too. Autodesk's sample download host does not send `Access-Control-Allow-Origin`, so its files cannot be fetched by a page from another host; raw GitHub file URLs do. For a quick try, serve the file from the same host as the site, as above.
- **The page needs the network**: marimo's runtime and Pyodide load from a CDN (jsDelivr) when the page opens, unless you self-host them. Nothing you open leaves the page: the file is read in your browser.
- Notebooks 02 and 03 call the `rvt-schedule` and `rvt-sheet` programs, which do not run in a browser, so only notebook 01 is exported.
- CI (the `Pyodide wheel and notebook site` job) builds the wheel with pinned tools, runs notebook 01's calls under Node and Pyodide and requires the numbers the native wheel gives, then opens the exported site in Chromium, Firefox and WebKit and checks the counts the page shows. It uploads the wheel (`pyodide-wheel`), the site (`notebook-01-site`, which fetches its model from `http://127.0.0.1:8765/model.rvt`, the address the test serves it on) and the screenshots. Nothing is published.
