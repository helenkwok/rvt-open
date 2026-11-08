#!/usr/bin/env python3
"""Check rvt-open's exports of the tiny reference models against their answer keys.

Each model in the fixtures (sets 1 and 2, fetched by tools/ci/fetch-tiny-models.sh) comes with
`<recipe>.answer.json`, read from the Revit API in the release that saved it.
For every release and recipe this runs `rvt-ifc` and `rvt-schedule --format jsonl`,
reads the IFC with IfcOpenShell, and compares both with the answer key.

Every check ends as pass, fail or known-gap. Known gaps are listed in
tools/ci/tiny-models-known-gaps.tsv (release, recipe, check, reason). The run
fails on a fail that is not listed, and on a listed gap that now passes (so the
list only shrinks). A markdown summary goes to $GITHUB_STEP_SUMMARY when set.

Usage: tiny_models_check.py FIXTURES_DIR BIN_DIR WORK_DIR [--known-gaps TSV] [--no-run]
"""
import argparse
import collections
import csv
import glob
import json
import os
import subprocess
import sys

import ifcopenshell
import ifcopenshell.geom
import ifcopenshell.guid
import ifcopenshell.util.placement
import ifcopenshell.util.unit
import numpy as np
from shapely.geometry import Polygon
from shapely.ops import unary_union

TOL_LEN, TOL_AREA, TOL_SLOPE = 0.001, 0.01, 0.1  # m, m2, degrees
# Answer-key lists of elements checked by their bounding box (set 2). Openings and site surfaces are
# only counted: Revit's export draws an opening where it cuts and a site surface without its Tag.
BOX_KEYS = ("floors", "columns", "beams", "stairs", "mep", "instances")
ITEM_KEYS = BOX_KEYS + ("openings_elems", "topo", "areas", "grids")
GEOM = ifcopenshell.geom.settings()
GEOM.set("use-world-coords", True)


def export_guid(unique_id):
    """IFC GlobalId Revit gives an element: the UniqueId's GUID with its last 32 bits XORed with the element id."""
    head, eid = unique_id[:36].replace("-", ""), int(unique_id[37:], 16)
    return ifcopenshell.guid.compress(head[:24] + "%08x" % (int(head[24:], 16) ^ eid))


def sub_element_guid(global_id, index):
    """GlobalId of sub-element `index` of the element with `global_id`: `index` XORed into GUID bytes 10 and 11."""
    h = ifcopenshell.guid.expand(global_id)
    return ifcopenshell.guid.compress(h[:20] + "%04x" % (int(h[20:24], 16) ^ index) + h[24:])


def mesh(prod):
    """World vertices (n, 3) and faces (m, 3) in metres of a product and its aggregated parts."""
    parts = [prod] + [p for r in prod.IsDecomposedBy or [] for p in r.RelatedObjects]
    verts, faces, n = [], [], 0
    for p in parts:
        if not p.Representation:
            continue
        try:
            s = ifcopenshell.geom.create_shape(GEOM, p)
        except Exception:
            continue
        v = np.array(s.geometry.verts).reshape(-1, 3)
        faces.append(np.array(s.geometry.faces).reshape(-1, 3) + n)
        verts.append(v)
        n += len(v)
    return (np.vstack(verts), np.vstack(faces)) if verts else (None, None)


def footprint(v, f):
    tris = [Polygon(v[t][:, :2]) for t in f]
    tris = [t for t in tris if t.is_valid and t.area > 1e-9]
    return unary_union(tris) if tris else None


def roof_slopes(v, f, edges):
    """Per answer edge: area-weighted slope (deg) of the upward faces facing out of that edge, or None."""
    a, b, c = v[f[:, 0]], v[f[:, 1]], v[f[:, 2]]
    n = np.cross(b - a, c - a)
    area = np.linalg.norm(n, axis=1) / 2
    n = n / np.maximum(np.linalg.norm(n, axis=1)[:, None], 1e-12)
    n = np.where(n[:, 2:3] < 0, -n, n)
    sloped = (n[:, 2] > 0.05) & (n[:, 2] < 0.995) & (area > 1e-6)
    plan = n[:, :2] / np.maximum(np.linalg.norm(n[:, :2], axis=1)[:, None], 1e-12)
    centre = np.mean([e["start"]["m"][:2] for e in edges], axis=0)
    out = []
    for e in edges:
        s, t = np.array(e["start"]["m"][:2]), np.array(e["end"]["m"][:2])
        d = (t - s) / np.linalg.norm(t - s)
        outward = np.array([d[1], -d[0]])
        if np.dot(outward, (s + t) / 2 - centre) < 0:
            outward = -outward
        sel = sloped & (plan @ outward > 0.999)
        if not sel.any():
            out.append(None)
            continue
        deg = np.degrees(np.arctan2(np.linalg.norm(n[sel, :2], axis=1), n[sel, 2]))
        out.append(float(np.average(deg, weights=area[sel])))
    return out


def wall_axis(prod):
    """End points (world metres, plan) of the wall's Axis representation, or None."""
    rep = next((r for r in (prod.Representation.Representations if prod.Representation else [])
                if r.RepresentationIdentifier == "Axis"), None)
    if rep is None:
        return None
    m = ifcopenshell.util.placement.get_local_placement(prod.ObjectPlacement)
    scale = ifcopenshell.util.unit.calculate_unit_scale(prod.file)
    item = rep.Items[0]
    if item.is_a("IfcPolyline"):
        pts = [list(p.Coordinates) for p in item.Points]
    elif item.is_a("IfcIndexedPolyCurve"):
        pts = [list(p) for p in item.Points.CoordList]
    elif item.is_a("IfcTrimmedCurve"):
        pts = [list(x[0].Coordinates) for x in (item.Trim1, item.Trim2)]
    else:
        return None
    return [(m @ np.array([p[0], p[1], p[2] if len(p) > 2 else 0.0, 1.0]))[:2] * scale for p in (pts[0], pts[-1])]


def axis_error(ax, ends, verts):
    """How far (m) a wall's Axis end points are from where Revit's export puts them, or None.

    Revit's IFC export draws a wall's Axis on its location line, from where its body starts along
    that line to where it ends: past the line's end where the body runs through a join, short of it
    where the body stops at one. That holds for every wall of its IFC4 Reference View exports of
    the tiny models, 2018 to 2027. The error is the larger of the end points' distance off the line
    and their distance from the body's ends along it."""
    if ax is None or verts is None:
        return None
    d = (ends[1] - ends[0]) / np.linalg.norm(ends[1] - ends[0])
    off = max(abs(float(d[0] * (a - ends[0])[1] - d[1] * (a - ends[0])[0])) for a in ax)
    along = sorted(float(np.dot(a - ends[0], d)) for a in ax)
    body = (verts[:, :2] - ends[0]) @ d
    return max(off, abs(along[0] - float(body.min())), abs(along[1] - float(body.max())))


def items(answer, key):
    """The answer key's list under `key` (a lone object counts as a list of one)."""
    v = answer.get(key) or []
    return [v] if isinstance(v, dict) else v


def near(a, b, tol):
    return a is not None and b is not None and abs(a - b) <= tol


def r4(x):
    return None if x is None else round(float(x), 4)


def param(elem, builtin):
    return next((p.get("value") for p in elem.get("parameters") or [] if p.get("builtin") == builtin), None)


def score(answer, ifc_path, jsonl_path, dump=None):
    """List of (check, ok, expected, got) for one model.

    `dump` (set 2: the model's api-dump.json, elements by id) was read after the steps done in
    Revit's user interface, so it gives the current state where the answer key predates a step."""
    out = []
    dump = dump or {}
    phases = sorted((param(e, "PHASE_SEQUENCE_NUMBER") or 0, e["name"]) for e in dump.values() if e.get("category") == "Phases")
    last_phase = phases[-1][1] if phases else None

    def exported(item):
        """Whether Revit's export holds the element: the last phase, the main model and primary options."""
        if not dump:
            return True
        e = dump.get(item["id"])
        if e is None:  # replaced since the answer was read (a copy moved into a design option)
            return False
        do = e.get("design_option")
        return (e.get("phase_demolished", -1) == -1 and not (do and "<primary>" not in do.get("name", ""))
                and (not item.get("phase") or item["phase"] == last_phase))

    def chk(name, ok, exp, got):
        out.append((name, bool(ok), exp, got))

    f = ifcopenshell.open(ifc_path)
    rows = [json.loads(line) for line in open(jsonl_path) if line.strip()]
    params = {str(r["element_id"]): {p["name"]: p["value"] for p in r.get("parameters") or []} for r in rows}
    by_tag = {}
    for p in f.by_type("IfcProduct"):
        tag = getattr(p, "Tag", None)
        if tag:
            by_tag.setdefault(str(tag), p)
    levels = {lv["id"]: lv for lv in answer["levels"]}
    unit = ifcopenshell.util.unit.calculate_unit_scale(f)

    # Set 2's answer keys (they carry `pending_ui`) list only what each recipe built, so its counts
    # cover the categories listed; set 1's list every element, so its counts cover every category.
    listed = {x["id"]: x["category"] for k in ("elements",) + ITEM_KEYS for x in items(answer, k)}
    listed.update({x["id"]: x["category"] for u in answer.get("ui_done") or [] for x in u["api_read_back"].get("in_place") or []})
    exp_c = collections.Counter(listed.values())
    got_c = collections.Counter(r["category"] for r in rows)
    for cat in sorted(set(exp_c) | (set() if "pending_ui" in answer else set(got_c))):
        chk(f"count:{cat}", exp_c[cat] == got_c[cat], exp_c[cat], got_c[cat])

    storeys = {s.Name: s for s in f.by_type("IfcBuildingStorey")}
    for lv in answer["levels"]:
        s = storeys.get(lv["name"])
        got = None if s is None else (s.Elevation or 0.0) * unit
        chk(f"level:{lv['name']}", near(lv["elevation"]["m"], got, TOL_LEN), r4(lv["elevation"]["m"]), r4(got))

    def gid(label, item):
        if "unique_id" not in item:
            return
        p = by_tag.get(str(item["id"]))
        exp = export_guid(item["unique_id"])
        chk(f"{label}:globalid", p is not None and p.GlobalId == exp, exp, p.GlobalId if p else None)

    walls = answer.get("walls", [])
    for i, w in enumerate(walls, 1):
        if not exported(w):
            continue
        label, wid = f"wall{i}", str(w["id"])
        p, pr = by_tag.get(wid), params.get(wid, {})
        gid(label, w)
        row = next((r for r in rows if str(r["element_id"]) == wid), {})
        chk(f"{label}:length", near(w["length"]["m"], row.get("length_m"), TOL_LEN), r4(w["length"]["m"]), r4(row.get("length_m")))
        ends = [np.array(e["point"]["m"][:2]) for e in w["ends"]]
        ax = wall_axis(p) if p is not None else None
        v = mesh(p)[0] if p is not None else None
        err = axis_error(ax, ends, v)
        if not (p is not None and p.is_a("IfcCurtainWall")):  # Revit's export gives a curtain wall no Axis
            chk(f"{label}:axis", err is not None and err <= TOL_LEN, "Axis on the location line, end to end of the body",
                "no Axis representation" if ax is None else "no body" if err is None else f"error {err:.4f} m")
        base = levels[w["level_id"]]["elevation"]["m"] + w["base_offset"]["m"]
        top = base + w["unconnected_height"]["m"]
        if w["id"] in dump and param(dump[w["id"]], "WALL_TOP_IS_ATTACHED") == 1:
            top = dump[w["id"]]["bbox"]["max"]["m"][2]  # attached: the top follows what it is attached to
        got = None if v is None else [r4(v[:, 2].min()), r4(v[:, 2].max())]
        chk(f"{label}:base-top", got is not None and near(base, got[0], TOL_LEN) and near(top, got[1], TOL_LEN),
            [r4(base), r4(top)], got)
        for end, key in ((0, "Start"), (1, "End")):
            e = w["ends"][end]
            exp = sorted(x for x in e["joined_elements"] if x != w["id"])
            g = pr.get(f"Join{key}WallElementId")
            got = [] if g is None else sorted(g if isinstance(g, list) else [g])
            chk(f"{label}:end{end}:partners", exp == got, exp, got)
            if exp or not e["join_allowed"]:
                exp_t = e["join_type"] if e["join_allowed"] else "Disallowed"
                got_t = pr.get(f"Join{key}Type")
                chk(f"{label}:end{end}:join-type", exp_t == got_t, exp_t, got_t)

    for i, rf in enumerate(answer.get("roofs", []), 1):
        if not exported(rf):
            continue
        label = f"roof{i}"
        p = by_tag.get(str(rf["id"]))
        gid(label, rf)
        # Revit writes the roof as one IfcSlab .ROOF. per slope-defining edge, its sub-elements 2, 3, ...
        slabs = [s for r in (p.IsDecomposedBy if p is not None else []) or [] for s in r.RelatedObjects]
        rf = dict(rf, edges=rf.get("edges") or [])
        # A footprint roof with no slope-defining edge (flat, or sloped by an arrow) is one slab; an extrusion roof none.
        planes = max(1, sum(1 for e in rf["edges"] if e["defines_slope"])) if rf["edges"] else 0
        chk(f"{label}:slabs", len(slabs) == planes and all(s.is_a("IfcSlab") and s.PredefinedType == "ROOF" for s in slabs),
            planes, len(slabs))
        if "unique_id" in rf:
            exp = [sub_element_guid(export_guid(rf["unique_id"]), k + 2) for k in range(planes)]
            got = [s.GlobalId for s in slabs]
            chk(f"{label}:slab-globalids", exp == got, exp, got)
        v, fc = mesh(p) if p is not None else (None, None)
        top = None if v is None else v[:, 2].max()
        exp = dump[rf["id"]]["bbox"]["max"]["m"][2] if rf["id"] in dump else rf["bbox_max"]["m"][2]
        chk(f"{label}:top", near(exp, top, TOL_LEN), r4(exp), r4(top))
        slopes = roof_slopes(v, fc, rf["edges"]) if v is not None else [None] * len(rf["edges"])
        for j, (e, got) in enumerate(zip(rf["edges"], slopes)):
            exp = round(e["slope_degrees"], 2) if e["defines_slope"] else None
            ok = (exp is None and got is None) or near(exp, got, TOL_SLOPE)
            chk(f"{label}:edge{j}:slope", ok, exp, None if got is None else round(got, 2))

    for rm in answer.get("rooms", []):
        if not exported(rm):
            continue
        label = f"room{rm['number']}"
        # IfcSpace has no Tag: find it by its number (IfcSpace.Name).
        p = next((s for s in f.by_type("IfcSpace") if s.Name == rm["number"]), None)
        pr = params.get(str(rm["id"]), {})
        exp = export_guid(rm["unique_id"])
        chk(f"{label}:globalid", p is not None and p.GlobalId == exp, exp, p.GlobalId if p else None)
        chk(f"{label}:name", rm["name"] == pr.get("RoomName", pr.get("Name")), rm["name"], pr.get("RoomName", pr.get("Name")))
        chk(f"{label}:number", rm["number"] == pr.get("RoomNumber", pr.get("Number")), rm["number"],
            pr.get("RoomNumber", pr.get("Number")))
        v, fc = mesh(p) if p is not None else (None, None)
        fp = footprint(v, fc) if v is not None else None
        chk(f"{label}:area", fp is not None and near(rm["area"]["m2"], fp.area, TOL_AREA), round(rm["area"]["m2"], 3),
            None if fp is None else round(fp.area, 3))
        loops = rm["boundary_loops"]
        poly = Polygon([s["start"]["m"][:2] for s in loops[0]]) if loops and len(loops[0]) > 2 else None
        hd = None if fp is None or poly is None else fp.boundary.hausdorff_distance(poly.boundary)
        chk(f"{label}:boundary", len(loops) == 1 and hd is not None and hd <= TOL_LEN, "outline within 1 mm",
            None if hd is None else f"Hausdorff {hd:.4f} m")

    counter = collections.Counter()
    for op in answer.get("openings", []):
        if not exported(op):
            continue
        kind = {"Doors": "door", "Windows": "window"}.get(op["category"], op["category"].lower())
        counter[kind] += 1
        label = kind if counter[kind] == 1 else f"{kind}{counter[kind]}"
        oid = str(op["id"])
        p, pr = by_tag.get(oid), params.get(oid, {})
        gid(label, op)
        host = None
        for rel in (p.FillsVoids if p is not None else []) or []:
            for v in rel.RelatingOpeningElement.VoidsElements:
                host = str(v.RelatingBuildingElement.Tag)
        chk(f"{label}:host", str(op["host_id"]) == host, str(op["host_id"]), host)
        hw = next(w for w in walls if w["id"] == op["host_id"])
        s, t = (np.array(hw["ends"][k]["point"]["m"][:2]) for k in (0, 1))
        d = (t - s) / np.linalg.norm(t - s)
        exp_pos = float(np.dot(np.array(op["location"]["m"][:2]) - s, d))
        v = mesh(p)[0] if p is not None else None
        pos = sill = None
        if v is not None:
            pos = float(np.dot((v[:, :2].min(0) + v[:, :2].max(0)) / 2 - s, d))
            sill = float(v[:, 2].min())
        floor = levels[op["level_id"]]["elevation"]["m"]
        chk(f"{label}:position", near(exp_pos, pos, TOL_LEN), r4(exp_pos), r4(pos))
        chk(f"{label}:sill", near(floor + op["sill_height"]["m"], sill, TOL_LEN), r4(floor + op["sill_height"]["m"]), r4(sill))
        h = pr.get("Height")
        head = None if sill is None or h is None else sill + h
        chk(f"{label}:head", near(floor + op["head_height"]["m"], head, TOL_LEN), r4(floor + op["head_height"]["m"]), r4(head))

    # Elements without a check of their own: their body's world bounding box. A beam's body stops at
    # the columns it frames into, so its extent along a plan axis it runs along is not compared.
    for key in BOX_KEYS:
        for i, it in enumerate(items(answer, key), 1):
            if not exported(it):
                continue
            label = f"{key.rstrip('s')}{i}"
            p = by_tag.get(str(it["id"]))
            v = mesh(p)[0] if p is not None else None
            exp = [r4(x) for x in it["bbox_min"]["m"] + it["bbox_max"]["m"]]
            got = None if v is None else [r4(x) for x in list(v.min(0)) + list(v.max(0))]
            skip = set()
            if key == "beams":
                d = np.array(it["end"]["m"][:2]) - np.array(it["location"]["m"][:2] if "start" not in it else it["start"]["m"][:2])
                skip = {k for k in (0, 1) if abs(d[k]) > 0.99 * np.linalg.norm(d)} | {k + 3 for k in (0, 1) if abs(d[k]) > 0.99 * np.linalg.norm(d)}
            ok = got is not None and all(abs(a - b) <= TOL_LEN for k, (a, b) in enumerate(zip(exp, got)) if k not in skip)
            chk(f"{label}:bbox", ok, exp, got if got is not None else "no IFC element with this Tag" if p is None else "no body")

    rows_by_id = {str(r["element_id"]): r for r in rows}
    for i, ar in enumerate(answer.get("areas") or [], 1):
        got = rows_by_id.get(str(ar["id"]), {}).get("area_m2")
        chk(f"area{i}:area", near(ar["area"]["m2"], got, TOL_AREA), round(ar["area"]["m2"], 3), r4(got))

    # Project position: Revit's export (site placement Shared) puts IfcSite at the E/W, N/S and
    # elevation of the internal origin, turned by the angle to True North.
    pbp = answer.get("project_base_point")
    if pbp:
        site = next(iter(f.by_type("IfcSite")), None)
        m = ifcopenshell.util.placement.get_local_placement(site.ObjectPlacement) if site and site.ObjectPlacement else None
        exp = [r4(pbp[k] * 0.3048) for k in ("BASEPOINT_EASTWEST_PARAM", "BASEPOINT_NORTHSOUTH_PARAM", "BASEPOINT_ELEVATION_PARAM")]
        got = None if m is None else [r4(x * unit) for x in m[:3, 3]]
        chk("site:position", got is not None and all(abs(a - b) <= TOL_LEN for a, b in zip(exp, got)), exp, got)
        exp_a = round((360 - np.degrees(pbp["BASEPOINT_ANGLETON_PARAM"])) % 360, 2)
        got_a = None if m is None else round(float(np.degrees(np.arctan2(m[1, 0], m[0, 0]))) % 360, 2)
        ok = got_a is not None and min(abs(exp_a - got_a), 360 - abs(exp_a - got_a)) <= TOL_SLOPE
        chk("site:rotation", ok, exp_a, got_a)

    used = {o.id() for r in f.by_type("IfcRelDefinesByType") if r.RelatedObjects for o in [r.RelatingType]}
    unused = [t for t in f.by_type("IfcTypeObject") if t.id() not in used]
    chk("types:unused", not unused, 0, len(unused))
    return out


def run(cmd):
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0:
        raise RuntimeError(f"{os.path.basename(cmd[0])} exited {r.returncode}: {r.stderr.strip()[-600:]}")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("fixtures")
    ap.add_argument("bin_dir")
    ap.add_argument("work")
    ap.add_argument("--known-gaps", default=os.path.join(os.path.dirname(os.path.abspath(__file__)), "tiny-models-known-gaps.tsv"))
    ap.add_argument("--no-run", action="store_true", help="score the exports already in WORK")
    a = ap.parse_args()

    gaps = {}
    with open(a.known_gaps, newline="") as fh:
        for row in csv.reader((line for line in fh if line.strip() and not line.startswith("#")), delimiter="\t"):
            if row[:3] == ["release", "recipe", "check"]:
                continue
            gaps[tuple(row[:3])] = row[3] if len(row) > 3 else ""

    results = []  # (release, recipe, check, status, expected, got)
    errors = []
    # FIXTURES is one set's folder, or a folder holding several (recipe names differ between sets).
    answers = sorted(glob.glob(os.path.join(a.fixtures, "20[0-9][0-9]", "*.answer.json"))
                     + glob.glob(os.path.join(a.fixtures, "*", "20[0-9][0-9]", "*.answer.json")))
    if not answers:
        sys.exit(f"no answer keys under {a.fixtures}")
    for ans in answers:
        release, recipe = os.path.basename(os.path.dirname(ans)), os.path.basename(ans)[: -len(".answer.json")]
        out = os.path.join(a.work, release)
        os.makedirs(out, exist_ok=True)
        rvt, ifc, jsonl = ans[: -len(".answer.json")] + ".rvt", os.path.join(out, recipe + ".ifc"), os.path.join(out, recipe + ".jsonl")
        try:
            if not a.no_run:
                run([os.path.join(a.bin_dir, "rvt-ifc"), rvt, "-o", ifc])
                run([os.path.join(a.bin_dir, "rvt-schedule"), rvt, "--format", "jsonl", "-o", jsonl])
            dump_path = ans[: -len(".answer.json")] + ".api-dump.json"
            dump = {e["id"]: e for e in json.load(open(dump_path))["elements"]} if os.path.exists(dump_path) else None
            checks = score(json.load(open(ans)), ifc, jsonl, dump)
        except Exception as e:
            errors.append((release, recipe, f"{type(e).__name__}: {e}"))
            continue
        for name, ok, exp, got in checks:
            key = (release, recipe, name)
            status = "pass" if ok and key not in gaps else "stale-gap" if ok else "known-gap" if key in gaps else "fail"
            results.append((release, recipe, name, status, exp, got))

    seen = {(r[0], r[1], r[2]) for r in results}
    missing = sorted(k for k in gaps if k not in seen and (k[0], k[1]) not in {(e[0], e[1]) for e in errors})
    fails = [r for r in results if r[3] == "fail"]
    stale = [r for r in results if r[3] == "stale-gap"]

    for r in fails:
        print(f"FAIL {r[0]} {r[1]} {r[2]}: expected {json.dumps(r[4])}, got {json.dumps(r[5])}")
    for r in stale:
        print(f"NOW PASSES {r[0]} {r[1]} {r[2]}: now passes, remove it from known-gaps")
    for k in missing:
        print(f"UNKNOWN GAP {k[0]} {k[1]} {k[2]}: no such check ran, remove it from known-gaps")
    for e in errors:
        print(f"ERROR {e[0]} {e[1]}: {e[2]}")

    # Summary: one row per release and recipe.
    tally = collections.defaultdict(collections.Counter)
    for r in results:
        tally[(r[0], r[1])]["pass" if r[3] in ("pass", "stale-gap") else r[3]] += 1
    lines = ["## Tiny reference models", "",
             f"{len(answers)} models; checks: {sum(t['pass'] for t in tally.values())} pass, "
             f"{sum(t['known-gap'] for t in tally.values())} known gap, {len(fails)} fail, "
             f"{len(stale)} known gap now passing, {len(errors)} models not scored.", "",
             "| release | recipe | pass | known gap | fail |", "|---|---|---|---|---|"]
    for (rel, rec), t in sorted(tally.items()):
        lines.append(f"| {rel} | {rec} | {t['pass']} | {t['known-gap']} | {'**%d**' % t['fail'] if t['fail'] else 0} |")
    for e in errors:
        lines.append(f"| {e[0]} | {e[1]} | - | - | **not scored**: {e[2][:200]} |")
    if fails or stale or missing:
        lines += ["", "### To fix", ""]
        lines += [f"- fail: {r[0]} {r[1]} `{r[2]}`: expected `{json.dumps(r[4])}`, got `{json.dumps(r[5])}`" for r in fails]
        lines += [f"- now passes, remove it from known-gaps: {r[0]} {r[1]} `{r[2]}`" for r in stale]
        lines += [f"- not a check that ran, remove it from known-gaps: {k[0]} {k[1]} `{k[2]}`" for k in missing]
    by_check = collections.Counter(r[2].split(":")[-1] for r in results if r[3] == "known-gap")
    if by_check:
        lines += ["", "### Known gaps by check", "", "| check | count |", "|---|---|"]
        lines += [f"| {k} | {n} |" for k, n in sorted(by_check.items())]
    text = "\n".join(lines) + "\n"
    print(text)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as fh:
            fh.write(text)
    with open(os.path.join(a.work, "tiny-models-checks.tsv"), "w", newline="") as fh:
        w = csv.writer(fh, delimiter="\t", lineterminator="\n")
        w.writerow(["release", "recipe", "check", "status", "expected", "got"])
        w.writerows([r[:4] + (json.dumps(r[4]), json.dumps(r[5])) for r in results])
    sys.exit(1 if fails or stale or missing or errors else 0)


if __name__ == "__main__":
    main()
