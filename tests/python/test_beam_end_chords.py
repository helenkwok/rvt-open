"""The beam-end scorer's parallel chords are a sample: their longest is a lower bound of the longest chord.

Needs numpy and IfcOpenShell (the scorer imports it), as tools/ci/requirements-ifc.txt installs them.
"""

import importlib.util
import math
import sys
from pathlib import Path

import numpy

TOOLS_RE = Path(__file__).resolve().parents[2] / "tools" / "re"
sys.path.insert(0, str(TOOLS_RE))
_spec = importlib.util.spec_from_file_location("beam_ends_vs_ifc", TOOLS_RE / "beam_ends_vs_ifc.py")
scorer = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(scorer)


def diamond_prism():
    """A column whose plan is the square (+-1, 0), (0, +-1), 1 high, as an array of triangles."""
    plan = [(1.0, 0.0), (0.0, 1.0), (-1.0, 0.0), (0.0, -1.0)]
    low = [numpy.array([x, y, 0.0]) for x, y in plan]
    high = [numpy.array([x, y, 1.0]) for x, y in plan]
    triangles = [(low[0], low[2], low[1]), (low[0], low[3], low[2]), (high[0], high[1], high[2]), (high[0], high[2], high[3])]
    for i in range(4):
        j = (i + 1) % 4
        triangles += [(low[i], low[j], high[j]), (low[i], high[j], high[i])]
    return numpy.array(triangles)


def test_the_longest_sampled_chord_can_miss_the_longest_chord():
    triangles = diamond_prism()
    axis = numpy.array([1.0, 0.0, 0.0])
    across = numpy.array([0.0, 1.0, 0.0])
    # Through y = 0 the chord is the diagonal, 2 long.
    low, high = scorer.chord(triangles, numpy.array([-5.0, 0.0, 0.5]), axis)
    assert math.isclose(high - low, 2.0, rel_tol=1e-9)
    # Lines through y = sqrt(2)/100, sqrt(2)/50 apart across the half-diagonal, never pass through y = 0.
    origin = numpy.array([-5.0, math.sqrt(2) / 100, 0.5])
    sampled = scorer.longest_parallel_chord(triangles, origin, axis, across, math.sqrt(2))
    assert 1.95 < sampled < 2.0 - 1e-3
