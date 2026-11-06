# Source

`craft-atomic` (crash-safe atomic file writes), vendored whole from craft-libs:

- Repository: https://github.com/storytold/craft-libs
- Commit: `fe4e2ff3efe2dfc5c6f4871c968a9fb561fd5305` (2026-11-04)
- Path: `crates/atomic` (`src/`, `tests/`), with the repository's `LICENSE-APACHE`,
  `LICENSE-MIT`, `rustfmt.toml` and `clippy.toml`
- Licence: MIT OR Apache-2.0 (rvt-open uses it under Apache-2.0); copyright ArtCraft Team and the
  craft-libs contributors (`NOTICE`)

Local changes, none in the code:

- The crate's `README.md` is left out: it tells the crate's history in other applications; its
  guarantees and usage are the same text as the crate documentation in `src/lib.rs`.

- `Cargo.toml`: the `workspace = true` keys are written out with craft-libs' workspace values, its
  workspace lints are copied in, and `publish = false` is added.
- `Cargo.toml`: `rust-version` is rvt-open's (1.89) instead of craft-libs' workspace value (1.95);
  the crate builds and passes its tests on 1.89.
- `clippy.toml`: the `disallowed-types` entry for a UI crate this crate does not use is left out.

To update: copy the same paths from a newer craft-libs commit and change the commit above.
