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
def _(mo, views):
    # A floor plan and a ceiling plan can share a name, so each option is the view's own ElementId.
    view = mo.ui.dropdown(
        options={f"{v['name']} ({v['id']})": str(v["id"]) for v in views},
        value=f"{views[0]['name']} ({views[0]['id']})" if views else None,
        label="Plan view",
    )
    view
    return (view,)


@app.cell
def _(mo, sheets):
    sheet = mo.ui.dropdown(
        options={f"{s['number']} {s['name']}": s["number"] for s in sheets},
        value=f"{sheets[0]['number']} {sheets[0]['name']}" if sheets else None,
        label="Sheet",
    )
    sheet if sheets else mo.md("This model has no sheets.")
    return (sheet,)


@app.cell
def _(mo, sheet_tool, view, work):
    plan_svg = ""
    if view.value is None:
        _out = mo.md("This model has no plan views.")
    else:
        plan_path = work / "plan.svg"
        sheet_tool("--view", view.value, "--format", "svg", "-o", str(plan_path))
        plan_svg = plan_path.read_text(encoding="utf-8")
        _out = mo.vstack([mo.md(f"### Plan view {view.selected_key}"), mo.Html(plan_svg)])
    _out
    return (plan_svg,)


@app.cell
def _(mo, sheet, sheet_tool, work):
    sheet_svg = ""
    if sheet.value is None:
        _out = mo.md("No sheet to draw.")
    else:
        sheet_path = work / "sheet.svg"
        sheet_tool("--sheet", sheet.value, "--format", "svg", "-o", str(sheet_path))
        sheet_svg = sheet_path.read_text(encoding="utf-8")
        _out = mo.vstack([mo.md(f"### Sheet {sheet.selected_key}"), mo.Html(sheet_svg)])
    _out
    return (sheet_svg,)


@app.cell
def _(MODEL, plan_svg, sheet_svg, sheets, views):
    # Results: named values that CI checks. A drawing is non-empty when it holds more shapes than the
    # background and border rectangles every plan SVG has (a lone rectangular footprint is a <rect> too).
    def shapes(svg):
        return sum(svg.count(t) for t in ("<path", "<line", "<polyline", "<polygon", "<rect"))

    result = {
        "model": MODEL,
        "plan_views": len(views),
        "sheets": len(sheets),
        "plan_svg_bytes": len(plan_svg),
        "plan_svg_shapes": shapes(plan_svg),
        "sheet_svg_bytes": len(sheet_svg),
        "sheet_svg_shapes": shapes(sheet_svg),
    }
    if views:
        assert "<svg" in plan_svg and result["plan_svg_shapes"] > 2, "the plan SVG holds no model geometry"
    if sheets:
        assert "<svg" in sheet_svg and result["sheet_svg_shapes"] > 2, "the sheet SVG holds no drawing"
    print(result)
    return (result,)


if __name__ == "__main__":
    app.run()
