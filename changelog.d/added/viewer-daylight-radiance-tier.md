- **The Daylight panel runs Radiance itself in the browser.** Radiance 6.0.3's
  gensky, gendaylit, oconv and rtrace, compiled to WebAssembly in CI
  (Emscripten 7.0.1 pinned by digest, one source fix in
  `tools/radiance/o_default.patch`) and fetched only when a Radiance analysis
  runs, give the daylight factor under the CIE overcast sky and the
  illuminance at a time under a Perez sky from a weather file, with the
  preview (`-ab 2`) or validated (`-ab 5`) parameters, on the instant tier's
  scene and sensors split over workers that share one compiled module. Weather
  is a PVGIS typical year (the panel links the official service; the file is
  read in the tab, its provenance recorded and "PVGIS © European Communities,
  2001-2026" shown with the result) or an EPW file. On the CC0 demo building
  the WebAssembly daylight factor agrees with native Radiance to RMSE 0.035 DF
  points (two native runs: 0.036). Radiance's licence ships beside the files
  and is linked from the panel and the footer.
