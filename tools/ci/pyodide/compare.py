"""Compare an `OPS {...}` line (stdin) with expected-basic-sample.json: every expected key must match."""
import json
import sys

here = __file__.rsplit("/", 1)[0] if "/" in __file__ else "."
expected = json.load(open(here + "/expected-basic-sample.json"))
line = [l for l in sys.stdin.read().splitlines() if l.startswith("OPS ")][-1]
got = json.loads(line[4:])
bad = {k: (v, got.get(k)) for k, v in expected.items() if got.get(k) != v}
print("expected", expected)
print("got     ", {k: got.get(k) for k in expected}, "| seconds:", got.get("seconds_counts"), got.get("seconds_all"))
sys.exit(1 if bad else 0)
