# Contributing to rvt-open

Thanks for your interest. This project is small and evolving quickly, so
the guidelines are intentionally light — but the gate below is the same one
CI applies, and everything in it runs without Revit, without the Autodesk SDK,
and without any private files: the synthetic fixtures under `corpus/tier1/`
drive the default checks, and corpus-gated tests skip themselves while
`RVT_PROJECT_CORPUS_DIR` is unset.

## Ten minutes to a first pull request

You need stable Rust 1.89 or newer (`rustup` installs it) and git.

1. Clone the repository (or your own GitHub copy of it):

   ```bash
   git clone <rvt-open repository URL>
   cd rvt-open
   ```

2. Run the local gate. The first run compiles everything, which takes a few
   minutes; later runs are incremental.

   ```bash
   tools/check-local.sh
   ```

   That runs `cargo fmt --check`, `cargo clippy` with `-D warnings`, rustdoc
   with `-D warnings`, and the integration tests under `tests/` — the same
   checks a pull request must pass. Green here means your machine is set up.

3. Pick something small. Tasks that fit in an afternoon and need no corpus
   files are the best start; the larger areas are the open items in
   [`ROADMAP.md`](ROADMAP.md).

4. Branch, change, and run the gate again:

   ```bash
   git switch -c fix/short-description
   # ...edit...
   tools/check-local.sh
   ```

5. Optional: `git config core.hooksPath .githooks` turns on a pre-commit hook
   that runs only `cargo fmt --check`, so the cheapest failure never reaches CI.

6. Commit with a [Conventional Commits](#commit-messages) message
   (`docs(...)`, `fix(...)`, `test(...)`), push your branch, and open a pull
   request against `main`. The pull-request template asks what changed and
   what you ran — fill in what applies and write "N/A" for the rest.

7. A maintainer reviews and squash-merges. GitHub holds the checks of a
   first-time contributor's pull request until a maintainer has looked at the
   proposed change and approved the workflow run, so a pull request whose
   checks say they are awaiting approval is waiting on us, not on you. Approval to run
   checks is separate from approval to merge. A draft pull request runs only the fast
   jobs; marking it ready runs the full set. Leave "Allow edits by
   maintainers" on and we can push small fix-ups to your branch (a rebase,
   a signed commit, a refreshed fixture) instead of bouncing it back; the
   squash commit keeps you as its author.
   [`docs/maintaining.md`](docs/maintaining.md) says how issues are triaged
   and labelled, what a pull request has to pass, and how releases are cut.

### Optional gates

`tools/check-local.sh` never needs the network by default. Opt into the
heavier checks when your change touches them:

```bash
tools/check-local.sh --viewer          # viewer typecheck + build
tools/check-local.sh --corpus          # corpus-backed tests (dirs must exist)
tools/check-local.sh --ifcopenshell    # require the ifcopenshell Python module
tools/check-local.sh --deny --audit    # require cargo-deny / cargo-audit
tools/check-local.sh --all-optional    # enable every optional gate
```

`tools/quality.sh` is the fuller pre-push path (optional supply-chain tools
when installed, plus `--full` bench compile); set `RVT_REQUIRE_AUDIT=1` or
`RVT_REQUIRE_DENY=1` there when missing tools should fail. Supply-chain rules
for Rust crates, viewer pnpm dependencies, advisory ignores, and GitHub Actions
pinning are documented in
[`docs/supply-chain-policy.md`](docs/supply-chain-policy.md).

Two CI behaviours are worth knowing before you read a red or amber check.
The `viewer dependency audit` job retries `pnpm audit --audit-level high` and
reports `NOT MEASURED` as a warning, passing the job, when the advisory
endpoint is unreachable — a measured high or critical advisory still fails it.
And CI on a pull request is deliberately narrower than on `main`: the
bench compile-check builds with `--profile ci`, the macOS and Windows test jobs
build `--lib --bins --tests` rather than `--all-targets`, and only the Linux
Python wheel builds, with the full platform matrix reserved for pushes to
`main`.

**Unit tests are banned.** Do not add them. Changes are verified end to
end: against real Revit files and Revit's own exports, through the built
CLIs, and by the contract checks that keep the published claims honest.
`tools/ci/test-targets.txt` classes every file under `tests/` as `real`,
`cli`, `contract` or `unit`, and `tools/ci/verify-real-files.sh` runs all
but `unit`, in CI on every OS (with the family corpus on Linux and Windows)
and again against the project corpora in the tier-2 job. The script fails
on a file the manifest does not class, so a new target cannot slip past.
Every file under `tests/` is a module of `tests/integration/main.rs`, one
test binary, so a new file also needs its two lines there (the script fails
without them), and its tests are named `<file>::<test>`: `cargo test --test
integration -- proxy_ports::` runs one file.
Targets that read a corpus must skip cleanly when their environment
variable is unset: the macOS and MSRV runs have no corpus.

## What's welcome

- **Bug reports** with a minimal reproducer (the smallest `.rfa`
  or `.rvt` that triggers the issue). Security-sensitive reports
  go through [`SECURITY.md`](SECURITY.md), not public issues.
- **Performance regressions** caught by the benchmark harness in
  `tools/bench.sh` — open an issue with a before/after table.
- **New facts** about the file format, as a research reader and an end-to-end
  test that reproduces them from bytes (see
  [Reverse-engineering findings](#reverse-engineering-findings)).
- **Documentation improvements.** The README and inline doc comments
  are fair game.
- **End-to-end checks.** A test under `tests/` that runs the real entry
  point (a CLI, the library on a file, the exporter) against a fixture or a
  corpus file and checks what a user would see. Unit tests are not accepted:
  CI and `tools/check-local.sh` do not run them, so they would check nothing.
  For a format fact, the evidence is that test and the measurements in the
  pull request.

## Where help is most wanted

What actually works today is in [`docs/status.md`](docs/status.md) and the
executable [`docs/support-matrix.json`](docs/support-matrix.json); the
per-area starting points are in
[`ROADMAP.md`](ROADMAP.md). In short:

- **Small, self-contained tasks:** the documentation and the tiny reference
  models below are always a good first contribution.
- **Corpus.** Redistributable `.rvt` / `.rfa` files with known element counts
  are the scarcest input. See [`docs/corpus-intake.md`](docs/corpus-intake.md)
  and the corpus issue form, and never send a file you are not certain you
  may share — a local check you run yourself is the alternative.
  [`docs/reference-models.md`](docs/reference-models.md) lists the models
  every claim is measured on (`tools/fetch-reference-models.sh` fetches the
  licensed ones), five tiny models anyone with Revit can make for the open
  geometry questions, and how to report a problem with a model you cannot
  share: send `rvt-inspect --json --anonymize` and
  `rvt-ifc --diagnostics <file> --anonymize` output, never the building.
- **Decoder research.** A byte-level reader plus an evidence table for one
  class or partition pattern (decoder issue form). Typed extraction works on
  Revit 2023 to 2025 projects, and experimentally on 2015 to 2022, 2026 and
  2027 projects, and not yet on 2014 and earlier; [`ROADMAP.md`](ROADMAP.md)
  says what is partial and what is known not to work, so check before
  spending days on one.
- **Tests that prevent false-positive decode claims**, viewer accessibility,
  and plain-language documentation are always welcome.

## Questions

Questions, ideas and "is this a bug?" go to the maintainers; issues track
work with a definite outcome.

## What needs discussion first

Open an issue (or a draft PR) before starting work on any of:

- **Layer 5 itself** — the questions above are open research; a
  one-paragraph sketch of your approach in an issue saves everyone
  time before you spend days on it.
- **IFC exporter emission** (`src/ifc/`). Mapping decisions have
  to align with buildingSMART IFC schema conventions.
- **Layer 4c field-type decoder changes.** All but 9 to 12 fields
  per release are classified, and the rest are listed in the CI
  gate (`tests/field_type_coverage.rs`). If you think a
  pattern is misclassified, file an issue with byte evidence from
  the corpus — do not silently change the decoder.

## Coding conventions

- Rust 2024 edition.
- `cargo fmt` before every commit.
- `tools/check-local.sh` must pass: fmt, clippy, rustdoc, a build of every
  target, and the real-file, CLI and contract checks. The CI in
  `.github/workflows/` enforces the same. No unit tests.
- **No `unsafe` in the library crate.** If you genuinely need it,
  open an issue first to discuss.
- **No panics in parsing paths.** Malformed input must return an
  `Error`, never `panic!`.
- **No PII in tests.** Use synthetic fixtures — `testuser`,
  `111111`, `FY-20XX`, etc. The redaction tests in
  `src/redact.rs` are the canonical examples.
- **Keep public status honest.** If a change affects user-visible
  capability, update [`docs/status.md`](docs/status.md),
  [`ROADMAP.md`](ROADMAP.md), or [`docs/compatibility.md`](docs/compatibility.md)
  in the same PR.
- **Every research module under `src/research/`** gets a module-level doc
  comment explaining *what fact it reads* and *how to verify*
  the result against the corpus.

## Fuzzing

`rvt-open` parses untrusted on-disk byte streams, so coverage-guided
fuzzing is part of the safety story. A `cargo-fuzz` workspace
lives at `fuzz/` — see [`fuzz/README.md`](fuzz/README.md) for the
full reference.

`cargo-fuzz` drives libFuzzer against a single entry-point per
target: you write a small `fuzz_target!` that takes a `&[u8]` and
feeds it into one parser surface, and libFuzzer mutates a corpus
looking for any input that makes the target panic, abort, time
out, or OOM. The fuzz crate is a standalone workspace so that the
main `cargo build` does not need nightly Rust.

That isolation has a cost: `tools/check-local.sh` and the pull-request gate
never compile `fuzz/`, so a new field on a public type the targets construct
breaks them silently and the break only surfaces on the nightly Fuzz run. That
is what happened when `IfcEntity::BuildingElement` gained `predefined_type`
and `fuzz_step_writer` failed with E0063. If you add or remove
a field on a type a fuzz target builds, run
`cargo +nightly check --manifest-path fuzz/Cargo.toml` before pushing.

To add a new fuzz target:

1. Pick the parser surface you want to harden and check the
   "Current targets" list in `fuzz/README.md` so you do not duplicate
   one of the eleven existing targets.
2. Create `fuzz/fuzz_targets/<name>.rs` using the libfuzzer-sys
   template and register it as a `[[bin]]` entry in
   `fuzz/Cargo.toml`.
3. Run the target locally (`cargo +nightly fuzz run <name>`) for
   long enough to exercise mutation — a few minutes at minimum,
   longer for anything that touches decompression or XML.
4. Turn any reproducible crash into a stable regression test in
   `tests/fuzz_regressions.rs` — it runs without nightly in normal CI.
   `fuzz/corpus/` itself is git-ignored.

`.github/workflows/fuzz.yml` runs every target nightly with a bounded
budget and uploads the crash corpus as an artifact on failure.

## Commit messages

We use Conventional Commits:

- `feat(<scope>): ...` for new features
- `fix(<scope>): ...` for bug fixes
- `docs(<scope>): ...` for documentation
- `test(<scope>): ...` for test-only changes
- `refactor(<scope>): ...` for behavior-preserving internal changes
- `perf(<scope>): ...` for performance
- `chore(<scope>): ...` for infra / CI / build

Scopes that appear frequently: `formats`, `object_graph`,
`elem_table`, `partitions`, `ifc`, `readme`, `cli`.

A pull request that breaks the stable Rust API (`RELEASE.md`, "What 1.0
keeps stable") marks its title with `!`, as in `refactor(error)!: ...`, or
carries a `BREAKING CHANGE:` footer in its body. CI's `semver checks` job
compares the API with the pull request's base using cargo-semver-checks and
fails on a breaking change the pull request does not announce. After
retitling, re-run that job. The research modules behind `unstable-research`
are not checked.

## Reverse-engineering findings

When you discover something new about the file format:

1. Reproduce the finding from bytes. A new reader is a research module: put
   the file in `src/research/` and declare it in `src/research.rs` (not
   `src/lib.rs`); code that uses it imports it as `rvt::research::<module>`.
   Tests, examples and benches build with the `unstable-research` feature on;
   `cargo build` and the published crate do not. Add an end-to-end test under
   `tests/` that checks the fact on real files, with corpus paths taken from
   the `RVT_*` environment variables, never hardcoded. A reader moves into
   the stable API only by a decision recorded in `RELEASE.md` ("What 1.0
   keeps stable").
2. Record the evidence in the pull request: status (positive or negative),
   the files and their sha256, the reference export measured against, and
   counts against Revit's own export (matches, misses, tolerance). A
   negative result is worth reporting; it keeps the next person from
   repeating the search.
3. If the finding changes what rvt-open outputs, measure the change end to end
   and update the public status in the same pull request: `docs/status.md`,
   `docs/support-matrix.json`, the README and a changelog fragment under
   [`changelog.d/`](changelog.d/README.md) (not `CHANGELOG.md` itself).
   The scorers under `tools/re/` compare an export with Revit's;
   `tools/ci/measure-reference-models.sh` runs all of them on the licensed
   reference models that `tools/fetch-reference-models.sh` fetches:

   ```bash
   tools/ci/measure-reference-models.sh <dir with rvt-ifc, rvt-gltf, rvt-info> <models dir> <out dir>
   ```

   Run it before and after your change and compare the outputs. Before a
   change to any export merges, every scorer scores it on every model.

   The Measure workflow (`.github/workflows/measure.yml`) checks that models
   with very large partitions read like any other, and times `rvt-ifc` on the
   Revit 2015 to 2022 samples against `main` on the same runner, so a speed
   claim compares like with like.

This keeps every claim independently verifiable, which is the
whole point of open reverse-engineering work.

## Corpus env vars

Tests, benchmarks, and examples that need real Revit files resolve
their paths from environment variables so no contributor's home
directory leaks into the repo (the CI `PII guard` job enforces
this). Three variables are recognised, in decreasing specificity:

- `RVT_FAMILY_2024` — full path to a single `.rfa` sample
  (family-file checks).
- `RVT_SAMPLES_DIR` — directory holding the 11-release
  [`phi-ag/rvt`](https://github.com/phi-ag/rvt) corpus. Defaults
  to `_corpus/examples/Autodesk` under the crate root, where
  `git clone https://github.com/phi-ag/rvt _corpus` puts it.
- `RVT_PROJECT_CORPUS_DIR` — directory holding `.rvt` project
  files, such as the MIT-licensed
  [`magnetar-io/revit-test-datasets`](https://github.com/magnetar-io/revit-test-datasets)
  corpus. Defaults to `_project_corpus/Revit` under the crate root.
  `tools/check-local.sh` and CI use the same two locations, and both
  folders are git-ignored.

Tests and benches that need these files skip gracefully if the
path doesn't exist, so a fresh clone runs all non-corpus-dependent
suites green without any env setup. To enable the corpus suites:

```bash
# Family corpus (LFS-tracked, 11 releases 2016-2026)
git clone https://github.com/phi-ag/rvt /tmp/phiag
export RVT_SAMPLES_DIR=/tmp/phiag/examples/Autodesk

# Project corpus (LFS-tracked, 2023 and 2024 real .rvt files)
git clone https://github.com/magnetar-io/revit-test-datasets /tmp/magnetar
export RVT_PROJECT_CORPUS_DIR=/tmp/magnetar/Revit

tools/ci/verify-real-files.sh --profile ci          # real-file, CLI, contract checks
cargo bench --bench project_file                    # multi-MB project file
cargo run --release --example basics_inspect -- "$RVT_PROJECT_CORPUS_DIR/<file>.rvt"
```

Never hardcode absolute paths in test or example code — the PII
guard job scans for `/Users/<name>/` and `/home/<name>/`
patterns on every push.

## Legal note for contributors

rvt-open is Apache-2.0 licensed. By submitting a contribution, you
agree that your work is licensable under Apache-2.0 and that you
have the right to grant that license.

**Please do not submit any code, comments, tests, or documentation
that contains information derived from Autodesk proprietary
sources** (NDA'd SDKs, decompiled binaries beyond what the public
`RevitAPI.dll` symbol export trivially exposes, leaked internal
documents, etc.). This project operates strictly from public
on-disk byte observations.

Questions: open an issue. Security reports go through
[`SECURITY.md`](SECURITY.md).
