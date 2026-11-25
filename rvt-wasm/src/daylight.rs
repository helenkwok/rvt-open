//! The daylight panel's bindings (`daylight-rt`): the scene, sensor grids and
//! site from an opened model, and the instant tier's runs, which the viewer
//! spreads over a pool of workers.

use super::{err_str, install_panic_hook};
use daylight_rt::glb::{GridOptions, read};
use daylight_rt::sun::{self, Site};
use daylight_rt::{Model, Modifier, Sensor, Surface, Tracer, Triangle};
use rvt::ifc::IfcModel;
use rvt::ifc::gltf::{GltfOptions, model_to_glb_with};
use std::cell::RefCell;
use wasm_bindgen::prelude::*;

fn set(o: &js_sys::Object, k: &str, v: &JsValue) {
    let _ = js_sys::Reflect::set(o, &JsValue::from_str(k), v);
}

fn num(v: &JsValue, k: &str, default: f64) -> f64 {
    js_sys::Reflect::get(v, &JsValue::from_str(k))
        .ok()
        .and_then(|x| x.as_f64())
        .unwrap_or(default)
}

/// The scene, the sensor grids and the site of `model`.
///
/// `options`: `{ spacing, offset, inset }` metres (defaults 0.5, 0.76, 0.35).
/// Returns `{ positions: Float64Array (9 per triangle, metres, Z up, the
/// model's frame), modifier: Uint32Array (per triangle), materials: [{ id,
/// label, kind: "plastic" | "glass", rgb, value (reflectance or pane
/// transmittance), assumed }], sensors: Float64Array (x y z dx dy dz),
/// sensorZone: Uint32Array, zones: [{ kind, name, number, roomName, level,
/// elementId, floorZ }], byCategory: [[name, triangles]], site }`. `site` is the
/// file's `{ latitude, longitude, elevation, timeZone, trueNorth,
/// locationName }`, or null where the file's site location was not read; `sun` is the
/// file's sun setting a viewer starts from (see `file_sun`), or null.
#[wasm_bindgen(js_name = daylightPrepare)]
pub fn daylight_prepare(model: JsValue, options: JsValue) -> Result<JsValue, JsValue> {
    install_panic_hook();
    let model: IfcModel = serde_wasm_bindgen::from_value(model).map_err(err_str)?;
    let opts = GridOptions {
        spacing: num(&options, "spacing", 0.5).max(0.05),
        offset: num(&options, "offset", 0.76),
        inset: num(&options, "inset", 0.35).max(0.0),
    };
    let glb = model_to_glb_with(&model, &GltfOptions::new(false, false, false));
    let p = read(&glb, &opts).map_err(err_str)?;
    drop(glb);
    let rooms = rvt::ifc::schedule_csv::room_rows(&model);

    let out = js_sys::Object::new();
    let mut pos = Vec::with_capacity(p.model.triangles.len() * 9);
    for t in &p.model.triangles {
        pos.extend(t.v.iter().flatten());
    }
    set(
        &out,
        "positions",
        &js_sys::Float64Array::from(&pos[..]).into(),
    );
    let mods: Vec<u32> = p.model.triangles.iter().map(|t| t.modifier).collect();
    set(
        &out,
        "modifier",
        &js_sys::Uint32Array::from(&mods[..]).into(),
    );
    let mats = js_sys::Array::new();
    for m in &p.model.modifiers {
        let o = js_sys::Object::new();
        set(&o, "id", &m.id.as_str().into());
        set(&o, "label", &m.label.as_str().into());
        let (kind, rgb, value) = match m.surface {
            Surface::Plastic { rgb, spec, .. } => {
                ("plastic", rgb, daylight_rt::grey(rgb) * (1.0 - spec))
            }
            Surface::Glass { tn } => {
                // the pane's normal transmittance: each of its two faces passes the glass's own
                let (_, t) = daylight_rt::glass_rt(tn[0], 1.0);
                ("glass", tn, t * t)
            }
        };
        set(&o, "kind", &kind.into());
        set(&o, "rgb", &js_sys::Float64Array::from(&rgb[..]).into());
        set(&o, "value", &value.into());
        set(&o, "assumed", &m.assumed.into());
        mats.push(&o);
    }
    set(&out, "materials", &mats);
    let mut sens = Vec::with_capacity(p.sensors.len() * 6);
    for s in &p.sensors {
        sens.extend(s.p);
        sens.extend(s.n);
    }
    set(
        &out,
        "sensors",
        &js_sys::Float64Array::from(&sens[..]).into(),
    );
    set(
        &out,
        "sensorZone",
        &js_sys::Uint32Array::from(&p.sensor_zone[..]).into(),
    );
    let zones = js_sys::Array::new();
    for z in &p.zones {
        let o = js_sys::Object::new();
        set(&o, "kind", &z.kind.into());
        set(&o, "name", &z.name.as_str().into());
        set(&o, "floorZ", &z.floor_z.into());
        if let Some(id) = z.element_id {
            set(&o, "elementId", &(id as f64).into());
        }
        if let Some(r) = z
            .entity_index
            .and_then(|e| rooms.iter().find(|r| r.entity_index as u64 == e))
        {
            set(&o, "number", &r.number.as_str().into());
            set(&o, "roomName", &r.name.as_str().into());
            set(&o, "level", &r.level.as_str().into());
        }
        zones.push(&o);
    }
    set(&out, "zones", &zones);
    let cats = js_sys::Array::new();
    for (c, n) in &p.by_category {
        cats.push(&js_sys::Array::of2(&c.as_str().into(), &(*n as f64).into()));
    }
    set(&out, "byCategory", &cats);
    let site = match &model.georeference {
        Some(g) => {
            let o = js_sys::Object::new();
            set(&o, "latitude", &g.latitude.into());
            set(&o, "longitude", &g.longitude.into());
            set(&o, "elevation", &g.elevation.into());
            set(&o, "timeZone", &g.time_zone.into());
            set(&o, "trueNorth", &g.true_north.into());
            set(&o, "locationName", &g.location_name.as_str().into());
            o.into()
        }
        None => JsValue::NULL,
    };
    set(&out, "site", &site);
    set(&out, "sun", &file_sun(&model));
    Ok(out.into())
}

/// The file's sun a viewer starts from, `{ source: "view" | "project", viewName, viewId,
/// settingName, settingId, sunAndShadowType, start, end (seconds since 1970 UTC),
/// timeInterval, azimuth, altitude (degrees), relativeToView, castShadows, ambientShadows,
/// sunlightIntensity, shadowIntensity, displayStyle }`, or null where none was read.
fn file_sun(model: &IfcModel) -> JsValue {
    let Some((source, view, s)) = model.sun.as_ref().and_then(|x| x.viewer_default()) else {
        return JsValue::NULL;
    };
    let o = js_sys::Object::new();
    set(&o, "source", &source.into());
    set(&o, "settingName", &s.name.as_str().into());
    set(&o, "settingId", &(s.id as f64).into());
    if let Some(t) = &s.sun_and_shadow_type {
        set(&o, "sunAndShadowType", &t.as_str().into());
    }
    if let Some(t) = &s.time_interval {
        set(&o, "timeInterval", &t.as_str().into());
    }
    for (k, v) in [
        ("start", s.start_unix.map(|t| t as f64)),
        ("end", s.end_unix.map(|t| t as f64)),
        ("azimuth", s.azimuth),
        ("altitude", s.altitude),
    ] {
        if let Some(v) = v {
            set(&o, k, &v.into());
        }
    }
    set(&o, "relativeToView", &s.relative_to_view.into());
    if let Some(v) = view {
        set(&o, "viewName", &v.name.as_str().into());
        set(&o, "viewId", &(v.id as f64).into());
        set(&o, "castShadows", &v.cast_shadows.into());
        set(&o, "ambientShadows", &v.ambient_shadows.into());
        if let Some(d) = &v.display_style {
            set(&o, "displayStyle", &d.as_str().into());
        }
        for (k, x) in [
            ("sunlightIntensity", v.sunlight_intensity),
            ("shadowIntensity", v.shadow_intensity),
        ] {
            if let Some(x) = x {
                set(&o, k, &(x as f64).into());
            }
        }
    }
    o.into()
}

thread_local! {
    static TRACER: RefCell<Option<(u32, Tracer)>> = const { RefCell::new(None) };
}

/// Builds and keeps the tracer for scene `id` from `daylightPrepare`'s
/// `positions` and `modifier`, and `materials` as 4 numbers per material:
/// kind (0 plastic, 1 glass) and the three channels (reflectance, or the
/// glass's transmissivity).
#[wasm_bindgen(js_name = daylightLoad)]
pub fn daylight_load(id: u32, positions: &[f64], modifier: &[u32], materials: &[f64]) {
    install_panic_hook();
    let tracer = Tracer::new(&model_of(positions, modifier, materials));
    TRACER.with(|t| *t.borrow_mut() = Some((id, tracer)));
}

/// The same scene as Radiance text, for the validated tier: `[materials,
/// scene]` (`plastic`/`glass` modifiers `m0`, `m1`, ... and one `polygon` per
/// triangle), from the arrays `daylightLoad` takes.
#[wasm_bindgen(js_name = daylightRadianceText)]
pub fn daylight_radiance_text(
    positions: &[f64],
    modifier: &[u32],
    materials: &[f64],
) -> Vec<String> {
    install_panic_hook();
    let model = model_of(positions, modifier, materials);
    vec![
        daylight_rt::radiance::write_materials(&model),
        daylight_rt::radiance::write_triangles(&model),
    ]
}

fn model_of(positions: &[f64], modifier: &[u32], materials: &[f64]) -> Model {
    let modifiers = materials
        .chunks_exact(4)
        .enumerate()
        .map(|(i, c)| Modifier {
            id: format!("m{i}"),
            surface: if c[0] == 1.0 {
                Surface::Glass {
                    tn: [c[1], c[2], c[3]],
                }
            } else {
                Surface::Plastic {
                    rgb: [c[1], c[2], c[3]],
                    spec: 0.0,
                    rough: 0.0,
                }
            },
            label: String::new(),
            assumed: false,
        })
        .collect();
    let triangles = positions
        .chunks_exact(9)
        .zip(modifier)
        .map(|(p, &m)| Triangle {
            v: [[p[0], p[1], p[2]], [p[3], p[4], p[5]], [p[6], p[7], p[8]]],
            modifier: m,
        })
        .collect();
    Model {
        modifiers,
        triangles,
    }
}

/// Runs one job of the instant tier on `sensors` (6 numbers each), the first of
/// which is sensor `first` of the whole grid, on the tracer `daylightLoad` kept
/// for scene `id`. `kind` and `params`:
///
/// - `sun`: `params` are sun directions (3 numbers each, the model's frame);
///   returns the number that reach each sensor;
/// - `svf`: `[paths]`; returns the sky view factor (0 to 1);
/// - `df`: `[ab, paths, ground reflectance]`; returns the daylight factor (%).
#[wasm_bindgen(js_name = daylightRun)]
pub fn daylight_run(
    id: u32,
    sensors: &[f64],
    first: u32,
    kind: &str,
    params: &[f64],
) -> Result<Vec<f64>, JsValue> {
    install_panic_hook();
    let sensors: Vec<Sensor> = sensors
        .chunks_exact(6)
        .map(|s| Sensor {
            p: [s[0], s[1], s[2]],
            n: [s[3], s[4], s[5]],
        })
        .collect();
    TRACER.with(|t| {
        let t = t.borrow();
        let (loaded, tracer) = t.as_ref().ok_or_else(|| err_str("no scene loaded"))?;
        if *loaded != id {
            return Err(err_str("another scene is loaded"));
        }
        let p = |k: usize, d: f64| params.get(k).copied().unwrap_or(d);
        let first = first as usize;
        Ok(match kind {
            "sun" => {
                let suns: Vec<[f64; 3]> =
                    params.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
                daylight_rt::sun_hours(tracer, &sensors, &suns)
                    .into_iter()
                    .map(f64::from)
                    .collect()
            }
            "svf" => daylight_rt::sky_view(tracer, &sensors, first, p(0, 4096.0) as u32),
            "df" => daylight_rt::daylight_factor(
                tracer,
                &sensors,
                first,
                p(0, 2.0) as u32,
                p(1, 4096.0) as u32,
                p(2, 0.2),
            ),
            _ => return Err(err_str(format!("unknown job {kind}"))),
        })
    })
}

/// Sun directions in the model's frame for the site (`latitude` degrees north,
/// `longitude` degrees east, `timeZone` hours, `trueNorth` degrees) from day
/// `from` to day `to` of a non-leap year, every `stepMinutes`, standard time:
/// 3 numbers each, above the horizon only.
#[wasm_bindgen(js_name = daylightSunPositions)]
pub fn daylight_sun_positions(site: JsValue, from: u32, to: u32, step_minutes: u32) -> Vec<f64> {
    let (s, north) = site_of(&site);
    sun::positions(&s, north, from, to, step_minutes)
        .into_iter()
        .flatten()
        .collect()
}

/// The sun at one moment: `[x, y, z]` toward the sun in the model's frame, then
/// altitude and azimuth (degrees; azimuth clockwise from true north).
#[wasm_bindgen(js_name = daylightSunAt)]
pub fn daylight_sun_at(site: JsValue, day: u32, hour: f64) -> Vec<f64> {
    let (s, north) = site_of(&site);
    let (alt, azi) = sun::altitude_azimuth(&s, day, hour);
    let v = sun::to_project(sun::direction(&s, day, hour), north);
    vec![
        v[0],
        v[1],
        v[2],
        alt.to_degrees(),
        (180.0 + azi.to_degrees()).rem_euclid(360.0),
    ]
}

fn site_of(v: &JsValue) -> (Site, f64) {
    (
        Site {
            latitude: num(v, "latitude", 0.0),
            longitude: num(v, "longitude", 0.0),
            time_zone: num(v, "timeZone", 0.0),
        },
        num(v, "trueNorth", 0.0),
    )
}
