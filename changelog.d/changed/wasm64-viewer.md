- **The viewer runs the 64-bit decoder where the browser supports it.** The
  viewer carried only the 32-bit build, which holds at most 4 GiB of memory,
  though the browser package has also shipped a 64-bit build (memory64, up to
  16 GiB). The viewer now loads the 64-bit build where the browser accepts
  memory64 and falls back to the 32-bit one where it does not, or where the
  64-bit one will not start. `?bits=32` or `?bits=64` asks for one build, the
  About line says which is running, and the out-of-memory messages name the
  running build's limit. The Pages site and the viewer tests use the same
  package as the release, and CI runs the whole browser suite on both builds.
  The measurements are in the pull request.
