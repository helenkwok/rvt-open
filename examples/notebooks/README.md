# Example notebooks

Three [marimo](https://marimo.io) notebooks (reactive, stored as `.py`) on a public sample, using only the public API and the command-line tools:

- [`01_open_a_model.py`](01_open_a_model.py): file information, element counts by category, a table of one category's elements, and the parameters of the element you pick (Python bindings).
- [`02_parameters_across_a_model.py`](02_parameters_across_a_model.py): runs `rvt-schedule --format sqlite`, reads the element and parameter tables with `sqlite3`, lists the parameters of a set with how many elements hold each, and charts elements per level.
- [`03_a_view_and_a_sheet.py`](03_a_view_and_a_sheet.py): lists the plan views and sheets and draws the ones you pick as SVG with `rvt-sheet`.

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

`RVT_MODEL` is any `.rvt`, `.rfa`, `.rte` or `.rft` file, and `RVT_BIN_DIR` the folder with `rvt-schedule` and `rvt-sheet`. Each notebook ends with a cell that names its results and asserts them, so a run that reads nothing fails.

CI (the `Example notebooks` job) runs every notebook this way on the Revit 2024 basic sample project, then exports each to static HTML (`marimo export html`) and uploads the pages as the `example-notebooks-html` artifact.
