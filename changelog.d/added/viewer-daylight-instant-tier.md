- **The viewer has a Daylight panel: sun, shadows, sun hours, sky view factor
  and a daylight-factor preview, computed in the tab.** The site comes from the
  file's location (latitude, longitude, elevation, time zone, angle to true
  north; each can be overridden, and the panel says which values are the
  file's, set by the user or assumed). The sun's position for a date and time
  moves the scene's light and casts shadows; sun hours over a range of days,
  the sky view factor and the daylight factor under the CIE overcast sky run on
  sensor grids over the rooms (or the floors) in a pool of Web Workers, drawn as
  a coloured grid with a legend, a readout on hover, a table per room, and JSON
  and CSV exports. The engine is the new `daylight-rt` crate, built into the
  wasm module. On the CC0 demo building (now a gallery sample) against Radiance
  6.0.3: sun hours equal on all 391 sensors, sun directions within 0.014° of
  `gensky`, sky view factor RMSE 0.0016, daylight-factor mean within 0.1 %
  (RMSE 0.13 DF points at 4,096 paths per sensor, a preview: Radiance against
  itself is 0.036).
