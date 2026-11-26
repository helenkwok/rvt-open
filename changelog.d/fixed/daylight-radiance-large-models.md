- **Radiance daylight runs finish on large models, and a failed run says why
  in one sentence.** The scene Radiance reads leaves out zero-area triangles,
  triangles with a corner that is not a number, and exact copies of an earlier
  triangle, and counts them in the result and its export: on Autodesk's
  Technical School sample 1,410 zero-area triangles and 1,176,000 copies of
  2,883,321 triangles were left out, and on Core Interior 7,746 copies of
  38,074, up to 12 deep. Copies lying on one another made oconv subdivide every
  voxel they shared to its finest level: Technical School ran out of
  WebAssembly memory ("oconv exited 2") and Core Interior wrote an octree over
  128 MiB, past what the page could collect, after 224 s. oconv now also stops
  at 1/1024 of the scene's size (`-r 1024`; on the demo building the daylight
  factor is bit-identical) and its output is gathered in chunks. A failure
  shows the program's fatal line first ("Radiance could not build the scene:
  …"), repeated warnings counted ("1,410 zero-area polygons skipped"), the
  whole log behind Details and in the JSON export; a trap, a failed fetch or a
  bad weather file ends in a message too, never a silent stop. Each worker's
  share of the memory sets how many Radiance workers run.
