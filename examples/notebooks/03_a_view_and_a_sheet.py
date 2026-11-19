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
    # A plan view and a sheet as SVG

    `rvt-sheet` draws a plan view, or a whole sheet with its viewports and title block, from the
    model's own data. This notebook lists the views and sheets (`--list-views`, `--list-sheets`),
    lets you pick one of each and shows the SVG.

    Set `RVT_MODEL` to a Revit file and `RVT_BIN_DIR` to the folder holding the `rvt-sheet` binary.
    """)
    return


@app.cell
def _():
    import json
    import os
    import subprocess
    import tempfile
    from pathlib import Path

    MODEL = os.environ.get("RVT_MODEL", "_samples/2024/racbasicsampleproject.rvt")
    BIN = os.environ.get("RVT_BIN_DIR", "target/ci") + "/rvt-sheet"
    work = Path(tempfile.mkdtemp(prefix="rvt-notebook-"))

    def sheet_tool(*args):
        return subprocess.run([BIN, MODEL, *args], check=True, capture_output=True, text=True).stdout

    views = json.loads(sheet_tool("--list-views"))
    sheets = json.loads(sheet_tool("--list-sheets"))
    return MODEL, sheet_tool, sheets, views, work


@app.cell
def _(mo, sheets, views):
    view = mo.ui.dropdown(options=[v["name"] for v in views], value=views[0]["name"], label="Plan view")
    sheet = mo.ui.dropdown(
        options={f"{s['number']} {s['name']}": s["number"] for s in sheets},
        value=f"{sheets[0]['number']} {sheets[0]['name']}", label="Sheet")
    mo.hstack([view, sheet], justify="start")
    return sheet, view


@app.cell
def _(mo, sheet_tool, view, work):
    plan_path = work / "plan.svg"
    sheet_tool("--view", view.value, "--format", "svg", "-o", str(plan_path))
    plan_svg = plan_path.read_text(encoding="utf-8")
    mo.vstack([mo.md(f"### Plan view: {view.value}"), mo.Html(plan_svg)])
    return (plan_svg,)


@app.cell
def _(mo, sheet, sheet_tool, work):
    sheet_path = work / "sheet.svg"
    sheet_tool("--sheet", sheet.value, "--format", "svg", "-o", str(sheet_path))
    sheet_svg = sheet_path.read_text(encoding="utf-8")
    mo.vstack([mo.md(f"### Sheet {sheet.value}"), mo.Html(sheet_svg)])
    return (sheet_svg,)


@app.cell
def _(MODEL, plan_svg, sheet_svg, sheets, views):
    # Results: named values that CI checks.
    result = {
        "model": MODEL,
        "plan_views": len(views),
        "sheets": len(sheets),
        "plan_svg_bytes": len(plan_svg),
        "sheet_svg_bytes": len(sheet_svg),
    }
    shapes = ("<path", "<line", "<polyline", "<polygon")
    assert "<svg" in plan_svg and any(t in plan_svg for t in shapes), "the plan SVG is empty"
    assert "<svg" in sheet_svg and any(t in sheet_svg for t in shapes), "the sheet SVG is empty"
    print(result)
    return (result,)


if __name__ == "__main__":
    app.run()
