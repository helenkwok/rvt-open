- **The Daylight panel shows how a run is going, one storey at a time, and
  takes a weather file without the file dialog.** Beside Run and Stop, which
  stay in view at the bottom of the panel, every run names its stage (the
  scene, loading Radiance with files and megabytes, the sky, the Radiance
  text, oconv, tracing with sensors done overall and per worker, collecting),
  with a bar that moves with the sensors, the elapsed time, the age of the last
  update and, once 3 % is done, the time left (the slowest worker's); after 60 s without progress it
  asks whether the run is still working and points to Stop. The grid fills in
  as values come back. It ends "Done in X s: N sensors, mean …" with the time
  of each stage, and a Radiance run estimated at more than two minutes asks
  first with the counts and the estimate. A **Storey** control shows one level
  that has sensors (or all): its grid only, the cut just above it and the
  camera on it, stepped with ↓ and ↑ or the Down and Up keys; the table and
  legend follow, with an option for one colour scale on every level, and the
  exports keep every level. A weather file can be dropped on the panel (or on
  the page while the analysis that uses one is chosen), and the last one for a
  site is kept in the browser and loads by itself, with a button to forget it.
  A second Radiance run on the same scene reuses its octree and only adds the
  new sky.
