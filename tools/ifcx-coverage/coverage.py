#!/usr/bin/env python3
"""How much of what the Revit API reports about a model each export carries.

Each tiny reference model (sets 1 and 2, tools/ci/fetch-tiny-models.sh) comes
with `<recipe>.api-dump.json`: every element the Revit API returns for the
file, with all its parameters. This script sorts those elements into kinds
(docs/ifcx-coverage.md) and counts, per kind, how many objects and how many
parameter values (name and value) an export carries:

- an object is carried when the export holds an object with its ElementId
  (IFC `Tag`, IFCX `ifc-src::attr::Tag`) or its IFC GlobalId (Revit's
  `IfcGUID` parameter, or the GlobalId Revit derives from its UniqueId; IFCX:
  that GlobalId as the node's UUID); a material also when an IFC4 file
  holds an `IfcMaterial` of its name;
- a parameter is carried when its object is carried and holds a property of
  the same name (case, spaces and punctuation ignored) whose value is the
  parameter's, in any of the units Revit or IFC may use. A type's parameter
  may sit on its type object or, where the export has none, on the type's
  elements.

Exports compared: rvt-open's IFC4 and IFCX (run with `--bin`, or read from
`--exports`), and Revit's own IFC4 Reference View (`--revit-ifc`, optional).

Usage:
  coverage.py FIXTURES_DIR WORK_DIR [--bin BIN_DIR [--rvt-ifc-arg ARG ...]] [--exports DIR]
              [--revit-ifc DIR ...] [--floor TSV] [--write-floor TSV]

Writes WORK_DIR/ifcx-coverage.tsv (one row per model and kind) and
WORK_DIR/ifcx-coverage.md (totals per set and kind), and the markdown to
$GITHUB_STEP_SUMMARY when set. With `--floor`, exits 1 when a total of
rvt-open's IFCX (per set and kind, objects or parameters) is below the
floor's, or when IFCX carries fewer than rvt-open's IFC4 for a kind.
"""
import argparse
import collections
import csv
import glob
import json
import math
import os
import re
import subprocess
import sys

import ifcopenshell
import ifcopenshell.guid

FT = 0.3048
# Factors from a Revit internal value to the units an export may write it in:
# feet to metres or millimetres, square and cubic feet to SI, flow, pressure,
# power and voltage as rvt-open converts them.
FACTORS = (1.0, FT, FT * 1000, FT**2, FT**2 * 1e6, FT**3, FT**3 * 1e9, 1 / FT)
EXPORTS = ("revit_ifc4", "rvtopen_ifc4", "rvtopen_ifcx")

# Kinds, by API class (last dotted part).
CLASS_KINDS = {
    "ProjectInfo": "project info",
    "Phase": "phases",
    "DesignOption": "design options",
    "Workset": "worksets",
    "Group": "groups",
    "GroupType": "groups",
    "RevitLinkInstance": "links",
    "RevitLinkType": "links",
    "ImportInstance": "links",
    "CADLinkType": "links",
    "Material": "materials",
    "Revision": "revisions",
    "ViewSchedule": "schedules",
    "ViewSheet": "sheets",
    "View": "views",
    "ViewPlan": "views",
    "ViewSection": "views",
    "View3D": "views",
    "ViewDrafting": "views",
    "Level": "levels and grids",
    "Grid": "levels and grids",
    "BasePoint": "site and location",
    "InternalOrigin": "site and location",
    "ProjectLocation": "site and location",
    "SiteLocation": "site and location",
}
# Categories of placed elements that are not building elements: drawing aids,
# sketches and markers.
NOT_ELEMENTS = {
    "Internal Origin", "Survey Point", "Project Base Point", "Center Line", "Center line",
    "Railing Rail Path Extension Lines", "Reference Planes", "Cameras", "Curtain Wall Grids",
    "Sun Path", "Views", "Lines", "Work Plane Grid", "Elevation Marks", "Section Marks",
}
KINDS = (
    "element instances", "type objects", "unused type objects", "levels and grids",
    "site and location", "project info", "materials", "phases", "design options", "views",
    "schedules", "sheets", "revisions", "worksets", "groups", "links",
)


def norm(name):
    return re.sub(r"[^0-9a-z]", "", str(name).lower())


def export_guid(unique_id):
    """The GlobalId Revit gives an element: the UniqueId's GUID with its last 32 bits XORed with the id."""
    try:
        head, eid = unique_id[:36].replace("-", ""), int(unique_id[37:], 16)
        return ifcopenshell.guid.compress(head[:24] + "%08x" % (int(head[24:], 16) ^ eid))
    except (ValueError, IndexError, TypeError):
        return None


def guid_uuid(gid):
    h = ifcopenshell.guid.expand(gid)
    return "%s-%s-%s-%s-%s" % (h[:8], h[8:12], h[12:16], h[16:20], h[20:])


def kind_of(e, used_types):
    cls = e["class"].rsplit(".", 1)[-1]
    if cls in CLASS_KINDS:
        return CLASS_KINDS[cls]
    if e.get("is_type"):
        return "type objects" if e["id"] in used_types else "unused type objects"
    cat = e.get("category")
    props = e.get("properties") or {}
    if (cat and not cat.startswith("<") and cat not in NOT_ELEMENTS and not props.get("ViewSpecific")
            and (e.get("bbox") or e.get("location"))):
        return "element instances"
    return None


def type_id(e):
    for p in e.get("parameters", []):
        if p.get("builtin") in ("ELEM_TYPE_PARAM", "SYMBOL_ID_PARAM") and isinstance(p.get("value"), int):
            if p["value"] > 0:
                return p["value"]
    return None


def param_items(e):
    """(normalised name, GlobalId if an IfcGUID parameter, candidates) of every parameter with a value, once per name and value."""
    seen, out = set(), []
    for p in e.get("parameters", []):
        if not p.get("has_value") or p.get("value") is None or p.get("value") == "":
            continue
        st, v, text = p.get("storage"), p["value"], p.get("as_value_string")
        if st == "Double":
            nums = {v * f for f in FACTORS} | {math.degrees(v)}
            m = re.match(r"\s*(-?[0-9][0-9,]*\.?[0-9]*)", text or "")
            if m:
                try:
                    nums.add(float(m.group(1).replace(",", "")))
                except ValueError:
                    pass
            cand = ("num", tuple(sorted(nums)))
        elif st == "Integer":
            texts = {str(v)} | ({text} if text else set())
            if "bool" in (p.get("spec") or ""):
                texts |= {"true" if v else "false"}
            cand = ("int", (v, frozenset(t.strip().lower() for t in texts)))
        elif st == "ElementId":
            texts = {str(v)} | ({text} if text else set())
            cand = ("text", frozenset(t.strip().lower() for t in texts))
        else:
            cand = ("text", frozenset({str(v).strip().lower()}))
        # A canonical key: a frozenset's repr depends on the hash seed.
        key = (norm(p["name"]), cand[0], repr(sorted(cand[1][1]) if st == "Integer" else sorted(cand[1])),
               cand[1][0] if st == "Integer" else None)
        if key in seen:
            continue
        seen.add(key)
        guid = v if p.get("builtin") in ("IFC_GUID", "IFC_TYPE_GUID") else None
        out.append((norm(p["name"]), guid, cand))
    return out


def value_matches(cand, values):
    kind, c = cand
    for x in values:
        if isinstance(x, bool):
            if kind == "int" and (int(x) == c[0] or ("true" if x else "false") in c[1]):
                return True
            if kind == "text" and ("true" if x else "false") in c:
                return True
        elif isinstance(x, (int, float)):
            if kind == "num" and any(abs(x - n) <= 1e-6 + 1e-5 * abs(n) for n in c):
                return True
            if kind == "int" and x == c[0]:
                return True
            if kind == "text" and str(int(x) if float(x).is_integer() else x) in c:
                return True
        elif isinstance(x, str):
            s = x.strip().lower()
            if kind == "text" and s in c:
                return True
            if kind == "int" and s in c[1]:
                return True
            if kind == "num":
                try:
                    f = float(s)
                except ValueError:
                    continue
                if any(abs(f - n) <= 1e-6 + 1e-5 * abs(n) for n in c):
                    return True
    return False


# ------------------------------------------------------------------ exports

class Export:
    """Objects of one export, by GlobalId and by Tag: (object, normalised property name -> values)."""

    def __init__(self, uuid_keyed=False):
        self.by_tag, self.by_gid, self.materials = {}, {}, set()
        # IFCX: by_gid is keyed by the node path, a GlobalId as a UUID.
        self.uuid_keyed = uuid_keyed

    def key(self, gid):
        if not self.uuid_keyed:
            return gid
        try:
            return guid_uuid(gid)
        except Exception:
            return None


def guids(e):
    """The GlobalIds an element may have, in order: its IfcGUID parameters, then the one Revit derives."""
    out = [p["value"] for p in e.get("parameters", [])
           if p.get("builtin") in ("IFC_GUID", "IFC_TYPE_GUID") and isinstance(p.get("value"), str)]
    out.append(export_guid(e.get("unique_id") or ""))
    return [g for i, g in enumerate(out) if g and len(g) == 22 and g not in out[:i]]


def find_in(export, e):
    for g in guids(e):
        k = export.key(g)
        if k in export.by_gid:
            return export.by_gid[k]
    return export.by_tag.get(str(e["id"]))


def ifc_value(v):
    return getattr(v, "wrappedValue", v)


def load_ifc(path):
    out = Export()
    f = ifcopenshell.open(path)
    for m in f.by_type("IfcMaterial"):
        out.materials.add(norm(m.Name))
    for o in f.by_type("IfcRoot"):
        if not hasattr(o, "Name"):
            continue
        props = collections.defaultdict(list)
        sets = []
        for r in getattr(o, "IsDefinedBy", None) or []:
            if r.is_a("IfcRelDefinesByProperties"):
                sets.append(r.RelatingPropertyDefinition)
        sets += list(getattr(o, "HasPropertySets", None) or [])
        for r in getattr(o, "IsTypedBy", None) or []:
            sets += list(r.RelatingType.HasPropertySets or [])
        for s in sets:
            for p in getattr(s, "HasProperties", None) or []:
                if p.is_a("IfcPropertySingleValue"):
                    props[norm(p.Name)].append(ifc_value(p.NominalValue))
                elif p.is_a("IfcPropertyEnumeratedValue"):
                    props[norm(p.Name)] += [ifc_value(x) for x in p.EnumerationValues or []]
            for q in getattr(s, "Quantities", None) or []:
                props[norm(q.Name)].append(q[3])
        rec = (o, props)
        out.by_gid[o.GlobalId] = rec
        tag = getattr(o, "Tag", None)
        if tag:
            out.by_tag.setdefault(str(tag), rec)
    return out


def ifcx_values(v):
    if isinstance(v, dict):
        if "value" in v and len(v) <= 3:
            return ifcx_values(v["value"])
        return []
    if isinstance(v, list):
        return [x for x in v if not isinstance(x, (dict, list))]
    return [v]


def load_ifcx(path):
    out = Export(uuid_keyed=True)
    with open(path, encoding="utf-8") as fh:
        doc = json.load(fh)
    nodes = collections.defaultdict(lambda: collections.defaultdict(list))
    inherits = collections.defaultdict(list)
    for n in doc.get("data", []):
        attrs = nodes[n["path"]]
        for k, v in (n.get("attributes") or {}).items():
            attrs[norm(k.rsplit("::", 1)[-1])] += ifcx_values(v)
            attrs["\0" + k] = v
        inherits[n["path"]] += list((n.get("inherits") or {}).values())
    # A node holds what it inherits (its type's values) as well as its own.
    for path, parents in inherits.items():
        for parent in parents:
            for k, v in list(nodes.get(parent, {}).items()):
                if k not in nodes[path]:
                    nodes[path][k] = v
    for path, attrs in nodes.items():
        rec = (path, attrs)
        out.by_gid[path] = rec
        tag = attrs.get("\0ifc-src::attr::Tag")
        if tag is not None:
            out.by_tag.setdefault(str(tag), rec)
    return out


def holds_guid(export, rec, guid):
    """Whether the object `rec` has the IFC GlobalId `guid`: an `IfcGUID` parameter is carried as the GlobalId."""
    if export.uuid_keyed:
        return rec[0] == export.key(guid)
    return getattr(rec[0], "GlobalId", None) == guid


# ------------------------------------------------------------------ scoring

def score(dump, exports):
    """{kind: {"objects": [total, per export...], "parameters": [...]}} for one model."""
    elements = dump["elements"]
    by_id = {e["id"]: e for e in elements}
    used = {}
    for e in elements:
        if not e.get("is_type"):
            t = type_id(e)
            if t is not None and t in by_id and by_id[t].get("is_type"):
                used.setdefault(t, []).append(e)
    def blank():
        return [0] + [0 if name in exports else None for name in EXPORTS]

    res = {k: {"objects": blank(), "parameters": blank()} for k in KINDS}
    for e in elements:
        kind = kind_of(e, used)
        if kind is None:
            continue
        # A type counts only once one of its instances is an element instance.
        if kind == "type objects" and not any(kind_of(i, used) == "element instances" for i in used[e["id"]]):
            kind = "unused type objects"
        r = res[kind]
        params = param_items(e) if kind != "unused type objects" else []
        r["objects"][0] += 1
        r["parameters"][0] += len(params)
        for i, name in enumerate(EXPORTS):
            x = exports.get(name)
            if x is None:
                continue
            rec = find_in(x, e)
            if kind == "materials" and rec is None:
                r["objects"][i + 1] += norm(e.get("name") or "") in x.materials
                continue
            pools = [rec[1]] if rec else []
            if kind == "type objects":
                pools += [h[1] for h in (find_in(x, i) for i in used[e["id"]]) if h]
            if rec is None and not pools:
                continue
            r["objects"][i + 1] += rec is not None
            r["parameters"][i + 1] += sum(
                1 for n, guid, c in params
                if (guid and rec and holds_guid(x, rec, guid))
                or any(value_matches(c, pool.get(n, ())) for pool in pools))
    return res


def run_rvt_ifc(bin_dir, rvt, out_dir, extra=()):
    os.makedirs(out_dir, exist_ok=True)
    stem = os.path.join(out_dir, os.path.basename(rvt)[:-4])
    ifc, ifcx = stem + ".ifc", stem + ".ifcx"
    proc = subprocess.run([os.path.join(bin_dir, "rvt-ifc"), rvt, "-o", ifc, "--ifcx", ifcx, *extra],
                          capture_output=True, text=True, timeout=900)
    if proc.returncode != 0:
        sys.stderr.write("rvt-ifc failed on %s: %s\n" % (rvt, proc.stderr[-2000:]))
    return ifc, ifcx


def models(fixtures):
    for dump in sorted(glob.glob(os.path.join(fixtures, "rvt-open-tiny-models-*", "*", "*.api-dump.json"))):
        rel_dir = os.path.dirname(dump)
        release = os.path.basename(rel_dir)
        folder = os.path.basename(os.path.dirname(rel_dir))
        recipe = os.path.basename(dump)[: -len(".api-dump.json")]
        yield folder.rsplit("-", 1)[-1], folder, release, recipe, dump


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("fixtures")
    ap.add_argument("work")
    ap.add_argument("--bin", help="folder holding rvt-ifc: export each model to WORK_DIR")
    ap.add_argument("--rvt-ifc-arg", action="append", default=[],
                    help="extra rvt-ifc option with --bin, such as --ifcx-unused-types (repeatable)")
    ap.add_argument("--exports", help="folder of earlier exports: rvt-open-tiny-models-N/RELEASE/RECIPE.ifc and .ifcx")
    ap.add_argument("--revit-ifc", action="append", default=[],
                    help="SET=DIR: Revit's exports, DIR/RELEASE/RECIPE.ifc4rv.ifc (repeatable)")
    ap.add_argument("--floor", help="TSV of IFCX totals per set and kind that must not drop")
    ap.add_argument("--write-floor", help="write the IFCX totals per set and kind to this TSV")
    a = ap.parse_args()
    revit_dirs = dict(x.split("=", 1) for x in a.revit_ifc)
    os.makedirs(a.work, exist_ok=True)
    rows = []
    for tiny_set, folder, release, recipe, dump_path in models(a.fixtures):
        with open(dump_path, encoding="utf-8") as fh:
            dump = json.load(fh)
        exports = {}
        if a.bin:
            ifc, ifcx = run_rvt_ifc(a.bin, dump_path[: -len(".api-dump.json")] + ".rvt",
                                    os.path.join(a.work, "exports", folder, release), a.rvt_ifc_arg)
        elif a.exports:
            stem = os.path.join(a.exports, folder, release, recipe)
            ifc, ifcx = stem + ".ifc", stem + ".ifcx"
        else:
            ifc = ifcx = None
        for name, path, loader in (("rvtopen_ifc4", ifc, load_ifc), ("rvtopen_ifcx", ifcx, load_ifcx)):
            if path and os.path.exists(path):
                exports[name] = loader(path)
            elif path:
                exports[name] = Export()  # a failed export carries nothing
        if tiny_set in revit_dirs:
            p = os.path.join(revit_dirs[tiny_set], release, recipe + ".ifc4rv.ifc")
            if os.path.exists(p):
                exports["revit_ifc4"] = load_ifc(p)
        for kind, r in score(dump, exports).items():
            for what in ("objects", "parameters"):
                rows.append([tiny_set, release, recipe, kind, what] + r[what])
        print("%s %s %s" % (tiny_set, release, recipe), flush=True)
    head = ["set", "release", "recipe", "kind", "what", "api"] + list(EXPORTS)
    with open(os.path.join(a.work, "ifcx-coverage.tsv"), "w", newline="") as fh:
        w = csv.writer(fh, delimiter="\t", lineterminator="\n")
        w.writerow(head)
        w.writerows([["" if v is None else v for v in r] for r in rows])
    totals = collections.OrderedDict()
    for r in rows:
        t = totals.setdefault((r[0], r[3], r[4]), [None] * 4)
        for i, v in enumerate(r[5:]):
            if v is not None:
                t[i] = (t[i] or 0) + v
    md = summary(totals)
    with open(os.path.join(a.work, "ifcx-coverage.md"), "w") as fh:
        fh.write(md)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as fh:
            fh.write(md)
    print(md)
    if a.write_floor:
        with open(a.write_floor, "w") as fh:
            fh.write("# rvt-open's IFCX totals over every release of the tiny models: tools/ifcx-coverage/coverage.py fails when one drops. Raise them when a change carries more.\n")
            fh.write("set\tkind\twhat\tifcx\n")
            for (s, k, w), t in totals.items():
                if t[0]:
                    fh.write("%s\t%s\t%s\t%d\n" % (s, k, w, t[3] or 0))
    failed = []
    for (s, k, w), t in totals.items():
        if t[2] is not None and t[3] is not None and t[3] < t[2]:
            failed.append("set %s, %s %s: IFCX carries %d, IFC4 %d" % (s, k, w, t[3], t[2]))
    if a.floor:
        # A set whose fixtures hold no API dump is not measured (the public set 1 has none).
        measured = {k[0] for k in totals}
        with open(a.floor) as fh:
            for row in csv.reader((ln for ln in fh if not ln.startswith("#")), delimiter="\t"):
                if row[0] == "set" or row[0] not in measured:
                    continue
                t = totals.get((row[0], row[1], row[2]))
                got = (t[3] or 0) if t else 0
                if got < int(row[3]):
                    failed.append("set %s, %s %s: IFCX carries %d, floor %s" % (row[0], row[1], row[2], got, row[3]))
    for f in failed:
        print("FAIL " + f)
    return 1 if failed else 0


def summary(totals):
    def cell(v, api):
        return "" if v is None else "%d (%d%%)" % (v, round(100 * v / api)) if api else str(v)

    out = ["## IFCX coverage of the Revit API's information (tiny reference models, all releases)", ""]
    for s in sorted({k[0] for k in totals}):
        out += ["### Set %s" % s, "",
                "| Kind | What | API | Revit's IFC4 | rvt-open IFC4 | rvt-open IFCX |",
                "|---|---|---:|---:|---:|---:|"]
        for (ts, k, w), t in totals.items():
            if ts == s:
                out.append("| %s | %s | %d | %s | %s | %s |" % (k, w, t[0], cell(t[1], t[0]), cell(t[2], t[0]), cell(t[3], t[0])))
        out.append("")
    return "\n".join(out) + "\n"


if __name__ == "__main__":
    sys.exit(main())
