- **CI reads every tiny reference model's container with a second reader,
  olefile, and holds rvt-open's to it.** `tools/ci/olefile_streams.py` lists
  each `.rvt` and `.rfa` of the tiny models and
  `tests/fixtures/families/empty.rfa` with olefile, runs `rvt-dump --raw` on
  the same file and fails on any difference in the stream names, in each
  stream's stored size, or in the stored bytes. It prints how many files and
  streams it compared, and a directory with no Revit file or a run that
  compared no stream fails. No export changes.
