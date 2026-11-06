# Examples

**Using the library.** Start with these. Each runs as-is on the synthetic
project file the repository ships, which has no element records, so it
lists and exports none; CI runs all three on it and on `Revit_IFC5_Einhoven.rvt`
(Revit 2023, from magnetar-io/revit-test-datasets, MIT), where they do:

- [`basics_inspect.rs`](basics_inspect.rs): what a file is (release, build,
  title, last save, schema).
- [`basics_elements.rs`](basics_elements.rs): its building elements, by IFC
  type, with ElementIds.
- [`basics_export.rs`](basics_export.rs): IFC4, glTF, a CSV schedule and the
  export diagnostics.

```sh
cargo run --example basics_inspect -- path/to/model.rvt
```

**Checks and measurements.** These back CI gates and profiling runs:

- [`box_check.rs`](box_check.rs): counts, per sample file and Revit
  category, the elements an export draws as a box, and fails when a count
  rises above the committed baseline.
- [`dhat_schema.rs`](dhat_schema.rs): heap-profiles the schema parser on a
  real Revit file (needs the `dhat-heap` feature).
- [`perf_budget_op.rs`](perf_budget_op.rs): runs one operation per process
  for `tools/perf_budget.py`, which measures its wall time and peak memory.
- [`stream_evidence.rs`](stream_evidence.rs): writes page-strip evidence
  JSON for a Revit file, using the `tools/stream_evidence` harness.

Some of these use `rvt::research`, the unstable readers behind the
`unstable-research` feature, which examples get automatically. Expect their
APIs and output to change.
