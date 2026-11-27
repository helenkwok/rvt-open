#!/usr/bin/env python3
"""Pair the `IfcSpace` of an rvt-open export with Revit's by GlobalId.

Revit's exporter gives a space the GlobalId rvt-open rebuilds from the file,
so a room rvt-open writes and Revit's export holds shares it. This
counts the spaces on each side and those in both, and with `--list` names the
spaces only one side holds, to tell which rooms an export phase leaves out
(`rvt-ifc --phase`).

Usage:
    python3 tools/re/spaces_by_globalid_vs_ifc.py OURS.ifc REVIT.ifc [--list]
"""

import sys

import ifcopenshell


def spaces(path):
    return {space.GlobalId: space for space in ifcopenshell.open(path).by_type("IfcSpace")}


def label(space):
    return f"{space.GlobalId} '{space.Name}' '{space.LongName}'"


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    listing = "--list" in sys.argv
    ours_path, revit_path = args
    ours, revit = spaces(ours_path), spaces(revit_path)
    both = ours.keys() & revit.keys()
    print(
        f"IfcSpace: rvt-open {len(ours)}, Revit {len(revit)}, in both {len(both)}, "
        f"only rvt-open {len(ours.keys() - revit.keys())}, only Revit {len(revit.keys() - ours.keys())}"
    )
    if listing:
        for side, only in (("only rvt-open", ours.keys() - revit.keys()), ("only Revit", revit.keys() - ours.keys())):
            for global_id in sorted(only):
                space = (ours if side == "only rvt-open" else revit)[global_id]
                print(f"  {side}: {label(space)}")


if __name__ == "__main__":
    main()
