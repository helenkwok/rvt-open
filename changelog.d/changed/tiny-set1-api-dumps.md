- **CI measures IFCX coverage on the tiny reference models set 1 too.** The
  `fixtures-tiny-1` asset now holds each model's Revit API dump
  (`<recipe>.api-dump.json`), as set 2's does, so `tools/ifcx-coverage/coverage.py`
  checks set 1's floor in CI; set 1's element parameter floor is raised to the
  2706 that main carries now (2442 before).
