"""rvt_open — Python bindings for rvt-open (Apache-2.0, clean-room Revit reader).

This module re-exports the C extension produced by pyo3/maturin. All
the real work is in the extension; this file exists so the wheel
ships a proper Python package with type stubs (`__init__.pyi`) and
a PEP-561 `py.typed` marker, giving IDEs autocomplete and type
checkers (mypy, pyright) a target they understand.
"""

# Re-export everything from the compiled extension, which maturin
# builds as `rvt_open._rvt` (`module-name` in pyproject.toml).
try:
    from rvt_open._rvt import *  # type: ignore  # noqa: F401,F403
    from rvt_open._rvt import __version__  # type: ignore  # noqa: F401
except ImportError:
    # Flat pyo3 layout — the compiled extension IS this module.
    # Nothing to re-export; native attributes resolve directly.
    pass

from typing import Any, Dict, List, Union

# The return shapes the stubs name, so that `from rvt_open import FileMetadata`
# works at runtime too. `typing` generics, since the wheel supports Python 3.8.
FileMetadata = Dict[str, Any]
TypedSchemaSummary = Dict[str, int]
TypedField = Dict[str, Union[str, int, List[int]]]
TypedADocument = Dict[str, Union[int, List[TypedField]]]
TypedDecodedElement = Dict[str, Any]
TypedElementCounts = Dict[str, Union[int, Dict[str, int]]]
