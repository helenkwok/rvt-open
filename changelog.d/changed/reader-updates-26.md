- **Unit tests are counted, and more checks run end to end.** CI now
  fails when a source file gains a unit test (412 in 68 files are listed
  in `tools/ci/unit-test-counts.txt`, a number that only falls). The
  history-stream readers' malformed-byte tests moved from the source to a
  `hardening` target, which runs them; the 3D camera reader's refusal of a
  skewed basis or a disagreeing view plane and the path scrub's
  idempotence now run as end-to-end targets too, as do the stream evidence
  harness's own tests; the MCP server's empty `RVT_MCP_ROOT` case is
  checked on the built server.
