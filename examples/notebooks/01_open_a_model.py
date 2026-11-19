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
    # Open a Revit model

    File information, element counts by category, a filterable element table and the
    parameters of a picked element, all through the public Python API (`rvt_open`).
    Set `RVT_MODEL` to the path of any `.rvt`, `.rfa`, `.rte` or `.rft` file; the default is
    the public Revit 2024 basic sample project that CI fetches, checked against its pinned hash.
    """)
    return


@app.cell
def _():
    import os
    import warnings

    import rvt_open

    MODEL = os.environ.get("RVT_MODEL", "_samples/2024/racbasicsampleproject.rvt")
    model = rvt_open.RevitFile(MODEL)
    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always")
        counts = model.element_counts()
        elements = model.decoded_elements()
    notes = sorted({str(w.message) for w in caught})
    return MODEL, counts, elements, model, notes, rvt_open


@app.cell
def _(MODEL, mo, model, notes, rvt_open):
    meta = model.metadata(redact=True)
    info = [
        {"field": k, "value": meta[k]}
        for k in ("revit_version", "build", "title", "last_saved", "workshared", "locale", "document_guid")
    ]
    mo.vstack(
        [
            mo.md(f"**{MODEL}** read with rvt-open `{rvt_open.__version__}`"),
            mo.ui.table(info, selection=None, label="File information"),
            mo.callout(mo.md("\n".join(f"- {n}" for n in notes)), kind="warn") if notes else mo.md(""),
        ]
    )
    return (meta,)


@app.cell
def _(counts, mo):
    by_class = sorted(counts["by_class"].items(), key=lambda kv: (-kv[1], kv[0]))
    mo.vstack(
        [
            mo.md(f"**{counts['total']} elements** in {len(by_class)} categories"),
            mo.ui.table([{"category": k, "elements": v} for k, v in by_class], selection=None, page_size=10),
        ]
    )
    return (by_class,)


@app.cell
def _(by_class, mo):
    category = mo.ui.dropdown(
        options=[k for k, _ in by_class],
        value="Wall" if any(k == "Wall" for k, _ in by_class) else by_class[0][0],
        label="Category",
    )
    category
    return (category,)


@app.cell
def _(category, elements, mo):
    def _name(e):
        typed = e.get("typed") or {}
        return typed.get("name") or next(
            (p["display_value"] for p in e["parameters"] if p.get("builtin") == "ALL_MODEL_TYPE_NAME"), ""
        )

    rows = [{"id": e["id"], "name": _name(e), "parameters": len(e["parameters"])}
            for e in elements if e["class_name"] == category.value]
    table = mo.ui.table(rows, selection="single", page_size=8, label=f"{category.value}: pick an element")
    table
    return rows, table


@app.cell
def _(elements, mo, rows, table):
    picked = (table.value or rows[:1] or [None])[0]
    if picked is None:
        out = mo.md("No element in this category.")
    else:
        element = next(e for e in elements if e["id"] == picked["id"])
        params = [
            {"name": p["name"], "value": p["display_value"], "group": p["group"], "storage": p["storage_type"], "level": p["level"]}
            for p in element["parameters"]
            if p["level"] == "instance"
        ]
        out = mo.vstack([mo.md(f"### Element {picked['id']}: {picked['name']} ({len(params)} instance parameters)"),
                         mo.ui.table(params, selection=None, page_size=10)])
    out
    return


@app.cell
def _(counts, elements, meta):
    # Results: named values that CI checks (the sample is a Revit 2024 project).
    result = {
        "revit_version": meta["revit_version"],
        "total_elements": counts["total"],
        "categories": len(counts["by_class"]),
        "decoded_elements": len(elements),
        "elements_with_parameters": sum(1 for e in elements if e["parameters"]),
    }
    assert result["total_elements"] == result["decoded_elements"] > 0, result
    assert result["elements_with_parameters"] > 0, result
    print(result)
    return (result,)


if __name__ == "__main__":
    app.run()
