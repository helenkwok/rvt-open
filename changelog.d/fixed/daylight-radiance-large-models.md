- **Radiance daylight runs finish on large models, and a failed run says why
  in one sentence.** The scene Radiance reads leaves out zero-area
  triangles, triangles with a corner that is not a number, and exact copies
  of an earlier triangle, and counts them in the result and its export: on
  Autodesk's Technical School sample 1,410 zero-area triangles and 1,175,173
  copies of 2,883,321 triangles were left out, and on Core Interior 7,746
  copies of 38,074, up to 12 deep. Copies lying on one another made oconv
  subdivide every voxel they shared to its finest level: Technical School
  ran out of WebAssembly memory ("oconv exited 2") and Core Interior wrote
  an octree over 128 MiB, past what the page could collect, after 224 s.
  oconv now stops at 1/1024 of the scene's size (`-r 1024`; on the demo
  building the daylight factor is bit-identical) for scenes under 500,000
  triangles, and uses its default resolution for larger ones or when the
  coarser build leaves surfaces out or runs out of memory ("set overflow":
  71,511 on Technical School, 91,898 on Snowdon Towers Electrical);
  Radiance's programs may grow to 4 GiB of WebAssembly memory instead of 2
  GiB, and oconv's output is gathered in chunks. Radiance's daylight factor
  now finishes on Core Interior (7,520 sensors) in about 41 to 63 s and on
  Technical School (18,327 sensors) in about 13 minutes with four workers. A
  failure shows the program's fatal line first ("Radiance could not build
  the scene: …"), repeated warnings counted ("1,410 zero-area polygons
  skipped"), the whole log behind Details and in the JSON export; a trap, a
  failed fetch or a bad weather file ends in a message too, never a silent
  stop. Each worker's share of the memory sets how many Radiance workers
  run.
