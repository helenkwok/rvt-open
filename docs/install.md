# Install Guide

rvt-open is built from source. This guide covers the command-line tools, the Rust library, the Python bindings and the browser (wasm) package. Use [user-guide.md](user-guide.md) afterwards to decide which workflow fits your file.


## Build From Source

Build the Rust CLIs and library with Cargo (Rust 1.89 or later).

```bash
git clone <rvt-open repository URL>
cd rvt-open
cargo build --release
./target/release/rvt-inspect --version
```

Run a local source smoke test:

```bash
./target/release/rvt-inspect path/to/model.rvt
./target/release/rvt-ifc path/to/model.rvt -o model.ifc --mode strict \
  --diagnostics model.diagnostics.json
```

## Build Python From Source

Use `maturin` when testing Python bindings from a source checkout:

```bash
python -m venv .venv
. .venv/bin/activate
python -m pip install --upgrade pip maturin
maturin develop --manifest-path rvt-py/Cargo.toml
python -c "import rvt_open; print(rvt_open.__version__)"
```

Build a wheel instead of installing into the active environment:

```bash
maturin build --release --manifest-path rvt-py/Cargo.toml --out dist
python -m pip install dist/rvt_open-*.whl
```

## Build The Viewer Locally

The viewer needs a WASM package and Node dependencies:

```bash
wasm-pack build rvt-wasm --target web --out-dir ../viewer/pkg --out-name rvt
cd viewer
pnpm install --frozen-lockfile
pnpm run typecheck
pnpm run build
```

`pnpm run build` writes the static site to `viewer/dist`. Use
`pnpm run dev` for local development. pnpm is the version `packageManager`
pins in `viewer/package.json`.
