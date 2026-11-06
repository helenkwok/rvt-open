#!/usr/bin/env python3
"""summarize.py results.jsonl... : one line per failing file, then FORMATS-CHECK files= bad=. Exit 1 on any failure.
A file fails when an export fails or a format's own official validator reports an error (IFC: IfcOpenShell; glTF: Khronos
validator, and every primitive needs NORMAL; USD: UsdValidation, and every mesh needs normals; .frag: ThatOpen's reader must
load it with unique GUIDs; IFCX: buildingSMART ifcx-cli compose with validation, and the schema regenerated from ifcx.tsp)."""
import sys, json
R = [json.loads(l) for f in sys.argv[1:] for l in open(f) if l.strip()]
def problems(r):
    p = []
    if r.get("rc_ifc") != 0 or r.get("rc_gltf") != 0: return ["export failed"]
    v = r.get("ifc") or {}
    if v.get("errors") != 0: p.append(f"ifc errors={v.get('errors')} {v.get('kinds')}")
    v = r.get("gltf") or {}
    if v.get("errors") != 0 or v.get("noNormal") != 0: p.append(f"gltf errors={v.get('errors')} noNormal={v.get('noNormal')} {v.get('kinds')}")
    for k in ("usda", "usdc"):
        v = r.get(k) or {}
        if not v.get("opened") or v.get("errors") != 0 or v.get("noNormals") != 0: p.append(f"{k} {v}")
    v = r.get("frag") or {}
    if not v.get("loaded") or v.get("error") or v.get("duplicateGuids") != 0 or v.get("geoWithoutGuid") != 0: p.append(f"frag {v.get('error') or v}")
    v = r.get("ifcx") or {}
    if not str(v.get("unparsed", "")).startswith("ifcx rc=0"): p.append(f"ifcx compose {v}")
    v = r.get("ifcxschema") or {}
    if v.get("valid") is not True: p.append(f"ifcx schema {v}")
    return p
bad = 0
for r in R:
    p = problems(r)
    if p: bad += 1; print("FORMAT-FAIL", r.get("file") or r.get("id"), "; ".join(p)[:600])
print(f"FORMATS-CHECK files={len(R)} bad={bad}")
sys.exit(1 if bad or not R else 0)
