import marimo

__generated_with = "0.25.2"
app = marimo.App(width="medium")


@app.cell
def _():
    import marimo as mo
    return (mo,)


@app.cell
def _(mo):
    mo.md(r"""
    # Parameters across a model

    `rvt-schedule --format sqlite` writes every element and every parameter as two SQLite
    tables (`elements`, `parameters`; see `docs/data-outputs.md`). This notebook runs it on a
    model and reads the result with Python's own `sqlite3`: which parameters the model carries,
    how many elements hold each, and how the elements spread over levels.

    Set `RVT_MODEL` to a Revit file and `RVT_BIN_DIR` to the folder holding the `rvt-schedule`
    binary (built with `--features sqlite`).
    """)
    return


@app.cell
def _():
    import os
    import sqlite3
    import subprocess
    import tempfile
    from pathlib import Path

    MODEL = os.environ.get("RVT_MODEL", "_samples/2024/racbasicsampleproject.rvt")
    BIN_DIR = os.environ.get("RVT_BIN_DIR", "target/ci")
    out = Path(tempfile.mkdtemp(prefix="rvt-notebook-")) / "model.elements.sqlite"
    subprocess.run([f"{BIN_DIR}/rvt-schedule", MODEL, "--format", "sqlite", "-o", str(out)],
                   check=True, capture_output=True)
    db = sqlite3.connect(out)
    return MODEL, db


@app.cell
def _(db, mo):
    n_elements = db.execute("SELECT count(*) FROM elements").fetchone()[0]
    n_params = db.execute("SELECT count(*) FROM parameters").fetchone()[0]
    n_names = db.execute("SELECT count(DISTINCT name) FROM parameters").fetchone()[0]
    mo.md(f"**{n_elements}** elements, **{n_params}** parameter values, **{n_names}** distinct parameter names")
    return n_elements, n_names, n_params


@app.cell
def _(db, mo):
    sets = [r[0] for r in db.execute(
        "SELECT \"set\" FROM parameters GROUP BY \"set\" ORDER BY count(*) DESC")]
    pset = mo.ui.dropdown(options=sets, value=sets[0], label="Parameter set")
    pset
    return (pset,)


@app.cell
def _(db, mo, pset):
    rows = [
        {"parameter": n, "elements": e, "values": v, "numeric": num, "unit": u or ""}
        for n, e, v, num, u in db.execute(
            "SELECT name, count(DISTINCT element_row), count(*), count(value_number), max(unit) "
            "FROM parameters WHERE \"set\" = ? GROUP BY name ORDER BY 2 DESC, 1", (pset.value,))
    ]
    mo.vstack([mo.md(f"### {pset.value}: {len(rows)} parameters"), mo.ui.table(rows, selection=None, page_size=10)])
    return


@app.cell
def _(db, mo):
    # One chart, plain SVG: elements per level, in order of level height.
    levels = db.execute(
        "SELECT coalesce(level, '(none)'), count(*) FROM elements "
        "GROUP BY level ORDER BY min(level_elevation_m), 1").fetchall()
    top = max(c for _, c in levels)
    bar = 22
    svg = [f'<svg xmlns="http://www.w3.org/2000/svg" width="560" height="{bar * len(levels) + 10}" font-family="sans-serif" font-size="12">']
    for i, (name, count) in enumerate(levels):
        y = 5 + i * bar
        w = 340 * count / top
        svg.append(f'<text x="0" y="{y + 14}">{name}</text><rect x="150" y="{y + 2}" width="{w:.0f}" height="16" fill="#3b6ea5"/>'
                   f'<text x="{154 + w:.0f}" y="{y + 14}">{count}</text>')
    svg.append("</svg>")
    mo.vstack([mo.md("### Elements per level"), mo.Html("".join(svg))])
    return (levels,)


@app.cell
def _(MODEL, db, levels, n_elements, n_names, n_params):
    # Results: named values that CI checks.
    result = {
        "model": MODEL,
        "elements": n_elements,
        "parameter_values": n_params,
        "distinct_parameter_names": n_names,
        "levels": len(levels),
        "elements_on_levels": sum(c for _, c in levels),
    }
    assert result["elements"] > 0 and result["parameter_values"] > 0, result
    assert result["elements_on_levels"] == result["elements"], result
    print(result)
    return (result,)


if __name__ == "__main__":
    app.run()
