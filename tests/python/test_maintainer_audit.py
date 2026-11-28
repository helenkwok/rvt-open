"""The maintainer audit keeps a stale branch only where an open thread names it whole."""

import importlib.util
from pathlib import Path

import pytest

_spec = importlib.util.spec_from_file_location(
    "audit", Path(__file__).resolve().parents[2] / "tools" / "maintainer" / "audit.py"
)
audit = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(audit)


@pytest.mark.parametrize(
    "text, named",
    [
        ("run `research/probe` on your model", True),
        ("https://github.com/helenkwok/rvt-open/tree/research/probe", True),
        ("compare/main...research/probe", True),
        ("the build from research/probe.", True),
        ("(research/probe)", True),
        ("see research/probe-v2", False),
        ("https://github.com/helenkwok/rvt-open/tree/research/probe-v2", False),
        ("research/probe/sub", False),
        ("research/probe.v2", False),
        ("my-research/probe", False),
    ],
)
def test_names_branch_whole(text, named):
    assert audit.names_branch(text, "research/probe") is named
