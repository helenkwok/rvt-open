# Daylight in the viewer

The viewer's **Daylight** panel (right-hand side, under the schedules) runs daylight and sun analyses on the open model, in the tab. Nothing is uploaded and nothing is fetched.

## Instant tier

The engine is the `daylight-rt` crate, compiled into the viewer's wasm module and run in a pool of Web Workers (one per core, up to eight), each started from the module the page already compiled.

| Analysis | What it computes | Checked against Radiance 6.0.3 on the CC0 demo building (391 sensors) |
|---|---|---|
| Sun position and shadows | The sun for a date and a time (local standard time), moving the scene's light, which casts shadows; a sun path of the 21st of each month | Sun directions within 0.014° of `gensky` over 36 positions |
| Sun hours | Hours of direct sun per sensor over a range of days, every 5 to 60 minutes; glass lets the sun through, anything opaque blocks it | Equal on 391 of 391 sensors (`rtrace -ab 0 -dj 0`) |
| Sky view factor | The share of a uniform sky above the horizon a sensor sees, glass weighted by its transmittance | RMSE 0.0016 (means 0.0125 and 0.0125) |
| Daylight factor (preview) | DF % under the CIE overcast sky, ground reflectance 0.2, two diffuse bounces (Radiance's `-ab 2`), by Monte Carlo paths | Mean within 0.1 %; RMSE 0.13 DF points at 4,096 paths per sensor (Radiance against itself: 0.036) |

The tests are `daylight-rt/tests/demo_building.rs` (CI job "daylight-rt") and `viewer/tests/daylight.spec.ts`.

- **Site**: the file's site location (latitude, longitude, elevation, time zone, angle to true north). Each value can be changed in the panel, which says whether it is the file's, set by you, or assumed (a file whose location was not read: all zero). A location equal to the one new Revit projects start with is pointed out.
- **Sensors**: a grid over each room's floor at the height given (default 0.76 m, 0.5 m apart); where the model has no rooms, over each floor's top face, kept 0.35 m from its edge. The table gives each room's number, name and level, sensor count, mean, minimum, maximum and uniformity (minimum over mean).
- **Exports**: JSON (analysis, parameters, the site with the source of each value, materials, per-storey and per-room statistics, every sensor's position and value, the stages and their times, the known limits) and CSV (one row per sensor). Both always hold every level. A failed run's JSON export holds the error, the stage it stopped in and Radiance's whole log.
- **Storey**: "All levels" or one level that has sensors, with its elevation. One level shows only its grid, puts the cut just above its sensors and turns the camera to it from above; the ↓ and ↑ buttons, or the Down and Up keys on the control, step through the levels. The table and the legend follow the level; "Same colour scale on every level" keeps the scale of the whole run so floors compare fairly. A height typed in "Cut the model at" overrides the cut until another level is picked.

## Progress

Every run shows, beside Run and Stop (which stay in view at the bottom of the panel however far it scrolls), the stage it is in with a count, and a bar that moves with the sensors:

| Stage | What the panel says |
|---|---|
| Scene | "Building the scene and the sensor grids", then the surfaces, sensors, rooms and levels |
| Sun positions (sun hours) | the days and the step |
| Loading the scene into the workers (instant tier) | workers ready of the total; once per scene, later runs skip it |
| Loading Radiance | files loaded of the total and megabytes (the first Radiance run only; later runs reuse the compiled programs) |
| Sky | gensky or gendaylit |
| Radiance text | the surfaces written, and afterwards how many zero-area and duplicate triangles were left out |
| oconv | the octree of the scene, then the sky added to it; a later run on the same scene reuses the scene's octree and only adds its sky |
| Tracing | sensors done of the total, overall and per worker; the grid fills in as values come back (grey until known) |
| Collecting | the values, the grid, the legend and the table |

Below it: the elapsed time, the age of the last update, and once 3 % of the sensors are done, the time left. With no progress for 60 s the panel asks "still working, or stuck?" and points to Stop. A run that fails says so in one sentence: for Radiance, "Radiance could not build the scene: <reason>" from the program's fatal line, with repeated warnings counted ("1,410 zero-area polygons skipped"), and the whole log behind **Details**. The end: "Done in X s: N sensors, mean …", with the time of each stage.

A Radiance run estimated at more than two minutes (from the sensors, the surfaces and the workers) asks first, with the counts and the estimate: Run again starts it. Nothing is capped; a wider grid spacing makes fewer sensors.

## Radiance tier

For numbers to report, the panel runs Radiance itself: gensky, gendaylit, oconv and rtrace from the sources of Radiance 6.0.3 (commit 84f888ec), compiled to WebAssembly by `tools/radiance/build-wasm.sh` with Emscripten 7.0.1 (its official image, pinned by digest) and one source change (`tools/radiance/o_default.patch`: WebAssembly checks the signature of an indirect call, and oconv calls `o_default` with two arguments it did not declare). The files are served from the viewer's `radiance/` folder and fetched only when a Radiance analysis runs; Radiance's licence (Radiance Software License 2.0) and a SOURCE.md naming the build ship beside them and are linked from the panel and the page footer.

| Analysis | Recipe |
|---|---|
| Daylight factor (Radiance) | `gensky -ang 45 0 -c -B 55.866` (CIE overcast, 10,000 lux unobstructed horizontal) with sky and ground glow, `oconv`, `rtrace -I -h`; DF % = lux / 100 |
| Illuminance at a time (Radiance) | `gendaylit M D H -a <lat> -o <lon west> -m <meridian> -W <DNI> <DHI>` from the weather file's hour nearest the time asked, `oconv`, `rtrace -I -h`; lux = 179 x (0.265 R + 0.670 G + 0.065 B) |

**The scene Radiance reads** leaves out triangles it cannot use, and counts them in the result: zero-area triangles (as printed, to 0.01 mm) or ones with a corner that is not a number, and exact copies of an earlier triangle (the same corners in any order; where copies have different materials, the first is kept). Copies lying on one another cannot be separated by oconv's octree, which then subdivides every voxel they share to its finest level. oconv runs with `-r 1024` (the octree stops subdividing at 1/1024 of the scene's size), which keeps the octree small: on the demo building the daylight factor is bit-identical to the default. Where more surfaces than a voxel can list meet at that size, oconv leaves some out ("set overflow": 71,511 on Technical School); the octree is then built again at oconv's default resolution and kept if it leaves out fewer, and any overflows left are counted in the result and named in the status line. The number of Radiance workers follows each one's memory (about seven times the octree plus 64 MiB, within three quarters of the memory the browser reports).

rtrace parameters: **preview** `-ab 2 -aa 0.1 -ad 2048 -ar 64` (the daylight-factor defaults of common daylighting recipes) or **validated** `-ab 5 -ad 2048 -as 1024 -ar 256 -aa 0.15 -lr 6 -lw 2e-4`. The scene and sensors are the instant tier's, written as Radiance text by `daylight-rt`. The sensors are split into one contiguous share per worker; each worker builds its own irradiance cache. The page compiles each program once and hands the compiled module to every worker, which only instantiates it.

Check (CI job "Radiance (WebAssembly) against native Radiance", `tools/radiance/check-demo.mjs`): the daylight factor at the preview parameters on the CC0 demo building has a mean of 1.0894 % and 1.0907 % in two WebAssembly runs against 1.0900 % for native Radiance 6.0.3, with RMSE 0.035 and 0.036 DF points, while two native runs (one process and eight) differ by 0.036. The check fails above the native noise plus 0.01, or a mean off by more than 0.5 %. The runs are not bit-identical because the Monte Carlo samples come from the C library's random-number generator, which differs between glibc and Emscripten's libc.

**Weather** for the illuminance at a time: a PVGIS 5.3 typical meteorological year (the default) or an EPW file of your own, chosen in the file dialog or dropped on the panel (or anywhere on the page while this analysis is chosen). The last file loaded for a site (latitude and longitude to 4 decimals) is kept in the browser's own storage (IndexedDB, nothing leaves the page) and loads by itself for the next model at that site; the panel says so and **Forget the kept weather file** removes it. The file input has no type filter, so the system dialog does not match every file of a folder against types (`.epw` has none registered); a file that is neither format is refused with the reason. PVGIS answers no requests from web pages, so the panel's link opens the official service (`https://re.jrc.ec.europa.eu/api/v5_3/tmy?lat=…&lon=…&usehorizon=1&outputformat=json`) in a new tab and you choose the saved file. The panel records the provenance with the result (source, radiation and meteorological databases, latitude, longitude and elevation, the years of the typical year, the horizon option, the file's date) and shows the credit **PVGIS © European Communities, 2001-2026** with every result computed from it. PVGIS hours are UTC; the panel moves them to the site's standard time with the file's stated irradiance time offset. EPW hours are local standard time, each the hour ending at its stamp.

**Materials**: the panel lists every material with its reflectance (or glass transmittance) and whether its colour is the file's or a placeholder rvt-open chose ("assumed").

## Tested on public samples

Every analysis, through the panel in headless Chromium on Linux (`viewer/tests/daylight-samples.spec.ts`), at the default 0.5 m grid. Times are from Run to the grid, with 8 to 12 cores; Technical School's Radiance runs used 4 workers (`?daylight-workers=4`).

| Model | Sensors | Levels | Sun hours | Sky view | DF preview | DF (Radiance) | Illuminance (Radiance) |
|---|---|---|---|---|---|---|---|
| CC0 demo building (Revit 2027) | 391 | 1 | passed, 0.8 s | passed, 0.7 s | passed, 1.9 s | passed, 12.4 s | passed, 14.8 s |
| Core Interior (Revit 2024, 32.2 MB) | 7,520 | 10 | passed, 4.1 s | passed, 69 s | passed, 151 s | passed, 41 s | passed, 47 s |
| Technical School (Revit 2026, 16.3 MB) | 18,327 | 3 | passed, 31 s | passed, 516 s | passed, 1,116 s | passed, 788 s | passed, 903 s |

Before this panel's progress and scene changes, Radiance's daylight factor failed on Core Interior after 234 s ("oconv exited 2: write error in oputint", with nothing shown while it ran) and on Technical School ("oconv exited 2": out of memory in oconv after 229 set overflows); on the demo building it took the same 12.7 s.

Where Radiance's time goes, on these three: rtrace is 95 % or more of every run (the demo building 12.0 of 12.4 s, Core Interior 38 of 41 s, Technical School 711 of 788 s); loading the programs takes 0.1 s or less once the browser has them; oconv 0.1 s, 1.3 s and 49 s (Technical School builds its octree twice, see above); writing the scene as Radiance text 9 s on Technical School. rtrace's own time is the irradiance cache: on the demo building one process takes 22.6 s and six 12.0 s, because each builds much of the same cache.

Measured and not adopted, against native Radiance's answer on the demo building:

- A shared ambient file: an `-af` pre-pass on every 4th, 8th or 16th sensor, merged and handed to every worker, stayed within the CI bounds but was no faster (12.1 to 13.1 s against 12.0 s with six workers): the sparse pre-pass sensors reuse little of each other's cache.
- Smaller parts handed to whichever worker is free (three per worker): no faster on Technical School (rtrace 726 s against 711 s), because each part builds its cache again.

Adopted: the scene's octree is kept, so a second run on the same scene (another sky, or the illuminance at another time) skips oconv; the grid fills in while rtrace runs; and the CI job above runs the instant tier and Radiance's daylight factor on the demo building, Core Interior (1 m grid) and Technical School (2 m grid) on every pull request. A request-only job (`workflow_dispatch` with `daylight_sweep`) runs them on every public sample CI fetches, in eight shards.

## Known limits

- A preview for design checks, not a compliance calculation.
- The daylight factor has no irradiance cache and no sampling toward windows, so each sensor is noisier than Radiance at the same time.
- Materials: reflectance is each material's viewer colour (kept between 0.02 and 0.85), glass transmittance is one minus its opacity; a material whose colour rvt-open chose (not the file's) is marked assumed.
- No surroundings: neighbouring buildings, trees and terrain are not in the scene.
- The sun follows Radiance's `gensky` formulas for a non-leap year, in local standard time without daylight saving; it is a point, not a disc.
- Elevation is shown but not used: no sky generator takes it.
- Radiance tier: each worker's irradiance cache covers only its share of the sensors, which moves the result by a fraction of a percent against one process; each worker holds its own copy of the octree, so a large scene runs fewer workers (Technical School's octree is about 270 MiB, about 1.5 GiB per worker). Before a Radiance run on a scene of more than 200,000 triangles, the instant tier's workers are ended to free their memory; the next instant run loads them again.
- A share of sensors can take much longer than the others (rooms full of furniture): the run ends with the slowest worker, and the time left follows it once every share is under way.
- Radiance in several workers: in Chrome on a laptop the daylight factor at the preview parameters takes 18.9 s in one worker and 9.6 s in eight; in headless Chromium on Linux in an 8-core container it took 24 s in one and 19 to 24 s in eight, whether the workers share the page's compiled module or compile their own, and with the baseline compiler off. The cause there is not known.
- Point in time uses one hour of the weather file (the nearest to the time asked), not an interpolation; there is no annual analysis in the browser yet.
