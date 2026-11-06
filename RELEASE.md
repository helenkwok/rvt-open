# Release process

How rvt-open cuts, tags, and publishes a release. Written for maintainers;
also useful for downstream packagers who want to understand what every
version number means.

## Versioning

rvt-open follows [Semantic Versioning 2.0](https://semver.org/spec/v2.0.0.html).

Pre-1.0, the project uses the Cargo convention — breaking changes bump
the **minor** version, not the major. That is:

- **MAJOR** — reserved. `1.0.0` is the first release with a stability
  commitment. Until then, major stays at `0`.
- **MINOR** — breaking API changes (pre-1.0) / new backwards-compatible
  features (post-1.0). Any change that renames a public item, changes a
  function signature, removes a variant, or alters serialised output
  shape goes here.
- **PATCH** — backwards-compatible bug fixes, doc corrections,
  performance work, and internal refactors that don't change public
  behaviour.

The current version lives in `Cargo.toml` under `[package].version`. It
is repeated in `rvt-py/Cargo.toml` (the wheel's version, and its `=X.Y.Z`
dependency on `rvt`), `rvt-open-mcp/Cargo.toml` (the MCP server's version, and its
`=X.Y.Z` dependency on `rvt`), `rvt-wasm/Cargo.toml`, the `rvt` dependency of
`tools/stream_evidence/Cargo.toml`, `viewer/package.json`, `CITATION.cff`, and the workspace crates' entries in
`Cargo.lock` and `fuzz/Cargo.lock`. All of them must agree at tag time.

## Release cadence

There is no fixed cadence. Releases happen when material changes
stabilise — a batch of decoders lands, an audit-driven correctness
repair closes, a security fix ships, the schema layer covers a new
Revit year. The project does **not** promise monthly, quarterly, or
"every N weeks" releases.

If downstream packagers need a stable tag, pin to one. A fix ships in the next release.

## What's in a release

Every `vX.Y.Z` release:

- Has a signed git tag `vX.Y.Z` on `main`.
- A tag with a hyphen (`vX.Y.Z-rc.N`) runs the same workflow and is published as a pre-release, never marked latest: use one to check the release path before the real tag.
- Has a GitHub release built by `.github/workflows/release.yml`: CLI
  archives for Linux (x86_64, arm64), macOS (Apple silicon, Intel) and
  Windows (x86_64), the WebAssembly package `rvt-open-wasm-vX.Y.Z.tar.gz`,
  `SHA256SUMS`, and for each archive a build provenance attestation, a
  CycloneDX SBOM and a Trivy report. A ClamAV scan of every archive must be
  clean before anything is published; `scans.md` collects the results.
- Uses the version's `CHANGELOG.md` section as its release notes.
- Updates `CITATION.cff` (`version:` and `date-released:`).

The Rust library and the Python wheel are built from source
([`docs/install.md`](docs/install.md)); no registry package is published.

## The 1.0 gate

1.0.0 is tagged only when every requirement in
[`docs/release-1.0-criteria.md`](docs/release-1.0-criteria.md) has passed,
each with its evidence linked. It supports every Revit release from 2015 to
2027 and ships with no limitations. The stability commitment covers the
surfaces that page names (its S1), and each of them has a check that
fails on an unannounced breaking change (S2). A milestone's completion
percentage is not part of the gate.

## What 1.0 keeps stable

From 1.0.0, a breaking change to any surface below needs a new major
version.

| Surface | What 1.0 keeps stable | What it does not |
|---|---|---|
| The `rvt` crate | The modules the crate root's module overview lists: reading and identity, typed elements and the element walk, and export. Their public items keep their names, signatures and meaning | `rvt::research` and everything else behind the `unstable-research` feature: the partition, native and element-record readers behind the format findings, which change whenever a finding does. `rvt::cli`, which exists for the shipped CLIs and is hidden from the docs |
| The CLIs | The `[[bin]]` targets `Cargo.toml` declares, which the README lists: their names, flags and the JSON they print, as [`docs/schemas/`](docs/schemas/) describes it | Text meant for people to read |
| The Python package `rvt_open` | The classes and functions in [`python/rvt_open/__init__.pyi`](python/rvt_open/__init__.pyi) | |
| Output formats | Valid IFC4 (ISO 10303-21), binary glTF 2.0, plan SVG and CSV, written deterministically: the same file gives the same bytes, once `SOURCE_DATE_EPOCH` fixes the time an IFC records | Their content. A finding that recovers more elements, properties or geometry is a minor release |
| The viewer's wasm API | The JavaScript bindings [`docs/viewer-api.md`](docs/viewer-api.md) lists, with their arguments and the JSON shapes they return | `rvt-wasm` is not published, so code outside this repository builds it from source |

A minor release may add public items, research modules and recovered
content. It may also add a variant to a public enum, which is why they are
`#[non_exhaustive]`, and a value to a list of values in the JSON the CLIs
print, such as a status or a reason code: code that reads one handles a
value it does not know. A research module that moves into the stable modules is named in
`CHANGELOG.md` and from then on follows the table above.

## Release checklist (for maintainers)

1. **CI green on `main`.** `CI passed`, the required check, covers fmt,
   clippy, the test matrix, rustdoc, `cargo deny`, `cargo audit`, the Python
   wheel, the WebAssembly package and the IFC validators.
2. **Bump the version** in every file listed under [Versioning](#versioning).
3. **Update `CHANGELOG.md`.** Run `python3 tools/release/changelog.py
   assemble`, which moves the fragments under `changelog.d/` into
   `[Unreleased]`, then move them into a new `[X.Y.Z] — YYYY-MM-DD` section
   under an empty `[Unreleased]`, grouped by `Added` / `Changed` / `Fixed` /
   `Security` (and `Research`). Credit outside contributors by name.
4. **Update `CITATION.cff`** to the version and the date.
5. **Open the release pull request,** `chore(release): prepare vX.Y.Z`, with
   steps 2 to 4, and merge it.
6. **Tag and push.** `git tag -s vX.Y.Z -m "rvt-open X.Y.Z" origin/main`,
   then `git push origin vX.Y.Z`, which runs `release.yml`.
7. **Write the release page** from the version's `CHANGELOG.md` section
   (`gh release edit vX.Y.Z --notes-file <file>`).

## Checking a release

From a clean shell, with `X.Y.Z` the released version:

```bash
gh release download vX.Y.Z -R helenkwok/rvt-open -p 'rvt-open-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz' -p SHA256SUMS
sha256sum -c --ignore-missing SHA256SUMS
gh attestation verify rvt-open-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz -R helenkwok/rvt-open
```

Then unpack it, run `rvt-inspect --version`, and open a redistributable
sample with `rvt-inspect` and in the viewer.

## Release automation

- **CI** (`.github/workflows/ci.yml`) on every push to `main` and every pull
  request; `CI passed` passes only when every job did (see
  [`docs/maintaining.md`](docs/maintaining.md#pull-requests)).
- **Release** (`.github/workflows/release.yml`) on `v*` tags: the archives,
  attestations, SBOMs and scans above.
- **Security scans** (`.github/workflows/security-scans.yml`), **Measure**
  (`measure.yml`) and **Memory** (`memory.yml`).
- **Dependabot** (`.github/dependabot.yml`): see
  [`docs/maintaining.md`](docs/maintaining.md#security-and-automation-settings).
