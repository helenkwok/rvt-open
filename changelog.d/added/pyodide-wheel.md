- **The Python bindings build as a Pyodide wheel, so example notebook 01
  runs in a browser with no install.** `rvt-py` now builds for
  `wasm32-unknown-emscripten` (a thread-count switch, the byte-source
  `Send + Sync` bounds and the command-line tools' allocator follow the
  emscripten target instead of the browser one; native and
  `wasm32-unknown-unknown` builds are unchanged). CI builds the wheel
  with pinned tools, runs notebook 01's calls under Node and Pyodide, and
  opens the notebook exported with `marimo export html-wasm` in three
  browsers. See `examples/notebooks/README.md`.
