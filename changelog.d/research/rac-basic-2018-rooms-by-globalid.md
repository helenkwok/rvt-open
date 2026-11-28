- **The 2018 `rac_basic` sample's rooms are paired by GlobalId with a public
  Revit 2018 export of it** (measurement only): a forum user's export,
  pinned by sha256 and byte count in `research/rac-basic-2018-export.tsv`, is
  fetched for a run by `tools/ci/fetch-rac-basic-2018-export.sh` and never
  stored, and `tools/ci/measure-reference-models.sh` scores the sample's
  export against it with `tools/re/spaces_by_globalid_vs_ifc.py`, counts
  only: all 14 of rvt-open's `IfcSpace` are among Revit's 17 (14 rooms
  and 3 MEP Spaces).
