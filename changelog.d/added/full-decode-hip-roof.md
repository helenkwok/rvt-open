- **A full-decode measure of one file, run by CI with a floor.**
  `tools/full-decode/full_decode.py` accounts for every byte of the tiny
  model `1-hip-roof` (Revit 2024), locates each of the 5,601 elements of its
  Revit API dump in the file and compares their attributes and parameter
  values (docs/full-decode.md). Baseline: no unknown byte in the container or
  the streams as stored (every page code recomputed, every gzip member and
  partition marker checked), 6,145,745 of 36,832,946 inflated bytes between
  partition objects not yet read; all 5,601 elements located, level, phases
  and design option matching on all; 2,076 parameter values matching.
