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

## Known limits

- A preview for design checks, not a compliance calculation.
- The daylight factor has no irradiance cache and no sampling toward windows, so each sensor is noisier than Radiance at the same time.
- Materials: reflectance is each material's viewer colour (kept between 0.02 and 0.85), glass transmittance is one minus its opacity; a material whose colour rvt-open chose (not the file's) is marked assumed.
- No surroundings: neighbouring buildings, trees and terrain are not in the scene.
- The sun follows Radiance's `gensky` formulas for a non-leap year, in local standard time without daylight saving; it is a point, not a disc.
- Elevation is shown but not used: no sky generator takes it.
