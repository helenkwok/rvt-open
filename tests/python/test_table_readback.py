"""Reads the element table back with each format's own reader.

`rvt-schedule --format jsonl|xlsx|parquet|sqlite -o DIR/t.elements.<ext>`
writes the four files; RVT_TABLE_DIR names that folder (CI sets it). The
XLSX goes through openpyxl, the Parquet through pyarrow and the SQLite
through sqlite3, and each must hold the JSON Lines records' rows and columns.
"""

import json
import os
import sqlite3
from pathlib import Path

import openpyxl
import pyarrow.parquet as pq
import pytest

DIR = os.environ.get("RVT_TABLE_DIR")
pytestmark = pytest.mark.skipif(not DIR, reason="RVT_TABLE_DIR is not set")


def _jsonl():
    records = [
        json.loads(line)
        for line in (Path(DIR) / "t.elements.jsonl").read_text(encoding="utf-8").splitlines()
        if line.strip()
    ]
    assert records, "the JSON Lines table is empty"
    keys = list(records[0])
    # The flat tables split `quantity_source` into three columns, move
    # `parameters` to rows of their own, and number the elements.
    elements = ["element_row"]
    for k in keys:
        if k == "quantity_source":
            elements += ["area_source", "volume_source", "length_source"]
        elif k != "parameters":
            elements.append(k)
    param_keys = list(next((p for r in records for p in r["parameters"]), {}))
    parameters = ["element_row", "element_id", "global_id", "value_number"] + param_keys
    # JSON objects carry no column order, so compare the names as sets.
    elements, parameters = sorted(elements), sorted(parameters)
    n_params = sum(len(r["parameters"]) for r in records)
    return records, elements, parameters, n_params


def test_xlsx():
    records, elements, parameters, n_params = _jsonl()
    book = openpyxl.load_workbook(Path(DIR) / "t.elements.xlsx", read_only=True)
    rows = list(book["elements"].iter_rows(values_only=True))
    assert sorted(rows[0]) == elements
    assert len(rows) - 1 == len(records)
    got = 0
    for name in book.sheetnames:
        if name == "parameters" or name.startswith("parameters "):
            sheet = list(book[name].iter_rows(values_only=True))
            assert sorted(sheet[0]) == parameters
            got += len(sheet) - 1
    assert got == n_params


def test_parquet():
    records, elements, _, n_params = _jsonl()
    table = pq.read_table(Path(DIR) / "t.elements.parquet")
    assert sorted(table.column_names) == sorted(elements + ["parameters"])
    assert table.num_rows == len(records)
    lists = table.column("parameters").to_pylist()
    assert sum(len(p) for p in lists) == n_params
    assert [len(p) for p in lists] == [len(r["parameters"]) for r in records]


def test_sqlite():
    records, elements, parameters, n_params = _jsonl()
    db = sqlite3.connect(Path(DIR) / "t.elements.sqlite")
    try:
        for table, columns, count in (
            ("elements", elements, len(records)),
            ("parameters", parameters, n_params),
        ):
            cur = db.execute(f"SELECT * FROM {table}")
            assert sorted(d[0] for d in cur.description) == columns
            assert len(cur.fetchall()) == count
    finally:
        db.close()
