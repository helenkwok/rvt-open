"""The calls example notebook 01 makes, as one JSON line: run natively (the `notebooks` job) and under Pyodide
(the `pyodide` job), then both are compared with expected-basic-sample.json.

Usage: python ops.py MODEL.rvt [LABEL]
"""
import hashlib
import json
import sys
import time
import warnings


def run(path, label):
    import rvt_open

    t = time.perf_counter()
    model = rvt_open.RevitFile(path)
    with warnings.catch_warnings(record=True):
        warnings.simplefilter("always")
        counts = model.element_counts()
        t_counts = time.perf_counter() - t
        elements = model.decoded_elements()
    t_all = time.perf_counter() - t
    meta = model.metadata(redact=True)
    # An element's identity as the notebook sees it: id, class, number of parameters (order-independent).
    sig = sorted((e["id"] if e["id"] is not None else -1, e["class_name"], len(e["parameters"])) for e in elements)
    return {
        "label": label,
        "total": counts["total"],
        "categories": len(counts["by_class"]),
        "decoded": len(elements),
        "with_params": sum(1 for e in elements if e["parameters"]),
        "params_total": sum(len(e["parameters"]) for e in elements),
        "counts_sha": hashlib.sha256(json.dumps(counts, sort_keys=True).encode()).hexdigest()[:16],
        "elements_sha": hashlib.sha256(json.dumps(sig).encode()).hexdigest()[:16],
        "revit_version": meta.get("revit_version"),
        "seconds_counts": round(t_counts, 2),
        "seconds_all": round(t_all, 2),
    }


if __name__ == "__main__":
    print("OPS " + json.dumps(run(sys.argv[1], sys.argv[2] if len(sys.argv) > 2 else "native")))
