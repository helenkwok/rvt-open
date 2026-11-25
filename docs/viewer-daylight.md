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
- **Exports**: JSON (analysis, parameters, the site with the source of each value, materials, per-room statistics, every sensor's position and value, the known limits) and CSV (one row per sensor).

## Radiance tier

For numbers to report, the panel runs Radiance itself: gensky, gendaylit, oconv and rtrace from the sources of Radiance 6.0.3 (commit 84f888ec), compiled to WebAssembly by `tools/radiance/build-wasm.sh` with Emscripten 7.0.1 (its official image, pinned by digest) and one source change (`tools/radiance/o_default.patch`: WebAssembly checks the signature of an indirect call, and oconv calls `o_default` with two arguments it did not declare). The files are served from the viewer's `radiance/` folder and fetched only when a Radiance analysis runs; Radiance's licence (Radiance Software License 2.0) and a SOURCE.md naming the build ship beside them and are linked from the panel and the page footer.

| Analysis | Recipe |
|---|---|
| Daylight factor (Radiance) | `gensky -ang 45 0 -c -B 55.866` (CIE overcast, 10,000 lux unobstructed horizontal) with sky and ground glow, `oconv`, `rtrace -I -h`; DF % = lux / 100 |
| Illuminance at a time (Radiance) | `gendaylit M D H -a <lat> -o <lon west> -m <meridian> -W <DNI> <DHI>` from the weather file's hour nearest the time asked, `oconv`, `rtrace -I -h`; lux = 179 x (0.265 R + 0.670 G + 0.065 B) |

rtrace parameters: **preview** `-ab 2 -aa 0.1 -ad 2048 -ar 64` (the daylight-factor defaults of common daylighting recipes) or **validated** `-ab 5 -ad 2048 -as 1024 -ar 256 -aa 0.15 -lr 6 -lw 2e-4`. The scene and sensors are the instant tier's, written as Radiance text by `daylight-rt`. The sensors are split into one contiguous share per worker; each worker builds its own irradiance cache. The page compiles each program once and hands the compiled module to every worker, which only instantiates it.

Check (CI job "Radiance (WebAssembly) against native Radiance", `tools/radiance/check-demo.mjs`): the daylight factor at the preview parameters on the CC0 demo building has a mean of 1.0894 % and 1.0907 % in two WebAssembly runs against 1.0900 % for native Radiance 6.0.3, with RMSE 0.035 and 0.036 DF points, while two native runs (one process and eight) differ by 0.036. The check fails above the native noise plus 0.01, or a mean off by more than 0.5 %. The runs are not bit-identical because the Monte Carlo samples come from the C library's random-number generator, which differs between glibc and Emscripten's libc.

**Weather** for the illuminance at a time: a PVGIS 5.3 typical meteorological year (the default) or an EPW file of your own. PVGIS answers no requests from web pages, so the panel's link opens the official service (`https://re.jrc.ec.europa.eu/api/v5_3/tmy?lat=…&lon=…&usehorizon=1&outputformat=json`) in a new tab and you choose the saved file. The panel records the provenance with the result (source, radiation and meteorological databases, latitude, longitude and elevation, the years of the typical year, the horizon option, the file's date) and shows the credit **PVGIS © European Communities, 2001-2026** with every result computed from it. PVGIS hours are UTC; the panel moves them to the site's standard time with the file's stated irradiance time offset. EPW hours are local standard time, each the hour ending at its stamp.

**Materials**: the panel lists every material with its reflectance (or glass transmittance) and whether its colour is the file's or a placeholder rvt-open chose ("assumed").

## Known limits

- A preview for design checks, not a compliance calculation.
- The daylight factor has no irradiance cache and no sampling toward windows, so each sensor is noisier than Radiance at the same time.
- Materials: reflectance is each material's viewer colour (kept between 0.02 and 0.85), glass transmittance is one minus its opacity; a material whose colour rvt-open chose (not the file's) is marked assumed.
- No surroundings: neighbouring buildings, trees and terrain are not in the scene.
- The sun follows Radiance's `gensky` formulas for a non-leap year, in local standard time without daylight saving; it is a point, not a disc.
- Elevation is shown but not used: no sky generator takes it.
- Radiance tier: each worker's irradiance cache covers only its share of the sensors, which moves the result by a fraction of a percent against one process; models much larger than the demo building need memory for one octree per worker.
- Radiance in several workers: in Chrome on a laptop the daylight factor at the preview parameters takes 18.9 s in one worker and 9.6 s in eight; in headless Chromium on Linux in an 8-core container it took 24 s in one and 19 to 24 s in eight, whether the workers share the page's compiled module or compile their own, and with the baseline compiler off. The cause there is not known.
- Point in time uses one hour of the weather file (the nearest to the time asked), not an interpolation; there is no annual analysis in the browser yet.
