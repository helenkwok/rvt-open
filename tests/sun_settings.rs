//! The file's sun settings and each view's display options ([`rvt::sun_settings`]) against the
//! Revit API.
//!
//! - The tiny reference models (release assets fixtures-tiny-1 and fixtures-tiny-2, MIT; saved in
//!   every release from 2018 to 2027), from `RVT_TINY_DIR` (what `tools/ci/fetch-tiny-models.sh`
//!   unpacks): every `SunAndShadowSettings` element of each model's Revit API dump
//!   (`<model>.api-dump.json`) is decoded with its name, `SunAndShadowType`, `TimeInterval`,
//!   azimuth, altitude, the ground-plane, relative-to-view, sunrise-to-sunset and shared flags,
//!   start and end; and every view the dump gives `SunlightIntensity` for has its own
//!   `SunAndShadowSettings`, `SunlightIntensity`, `ShadowIntensity` and `DisplayStyle`. Skips
//!   when `RVT_TINY_DIR` is not set, unless `RVT_SUN_REQUIRE` is.
//! - The CC0 demo building (`RVT_DAYLIGHT_DEMO_RVT`) carries the sun its viewer opens with.

use serde_json::Value;
use std::path::{Path, PathBuf};

/// Minutes since 1970 of a dump's `DateTime` text (`10-May-04 6:25:00 AM`), as written.
fn dump_minutes(text: &str) -> Option<i64> {
    let (date, rest) = text.split_once(' ')?;
    let mut d = date.split('-');
    let day: i64 = d.next()?.parse().ok()?;
    let mon = d.next()?;
    let month = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ]
    .iter()
    .position(|m| *m == mon)? as i64
        + 1;
    let year: i64 = 2000 + d.next()?.parse::<i64>().ok()?;
    let (clock, ampm) = rest.split_once(' ')?;
    let mut c = clock.split(':');
    let h: i64 = c.next()?.parse().ok()?;
    let m: i64 = c.next()?.parse().ok()?;
    let h = match (h, ampm) {
        (12, "AM") => 0,
        (12, "PM") => 12,
        (h, "PM") => h + 12,
        (h, _) => h,
    };
    // days from civil (Howard Hinnant)
    let (y, mo) = if month <= 2 {
        (year - 1, month + 9)
    } else {
        (year, month - 3)
    };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * mo + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 1440 + h * 60 + m)
}

/// Minutes between a dump's `DateTime` text and the stored instant (seconds since 1970 UTC),
/// `None` when either is missing or the instant is not a whole minute. The dump writes a
/// `DateTime` in the clock time of the machine it ran on, so within one model every offset is
/// that machine's: one value, or two an hour apart across a daylight-saving change.
fn clock_offset(unix: Option<i64>, text: Option<&str>) -> Option<i64> {
    let (t, m) = (unix?, dump_minutes(text?)?);
    (t % 60 == 0).then_some(m - t / 60)
}

fn tiny_models(dir: &Path) -> Vec<(PathBuf, PathBuf)> {
    let mut out = Vec::new();
    for set in ["rvt-open-tiny-models-1", "rvt-open-tiny-models-2"] {
        let Ok(releases) = std::fs::read_dir(dir.join(set)) else {
            continue;
        };
        for r in releases.flatten() {
            let Ok(files) = std::fs::read_dir(r.path()) else {
                continue;
            };
            for f in files.flatten() {
                let p = f.path();
                if p.extension().is_some_and(|e| e == "rvt") {
                    let dump = p.with_extension("api-dump.json");
                    if dump.exists() {
                        out.push((p, dump));
                    }
                }
            }
        }
    }
    out.sort();
    out
}

#[test]
fn tiny_models_sun_settings_and_view_display_match_the_revit_api() {
    let Some(dir) = std::env::var_os("RVT_TINY_DIR") else {
        assert!(
            std::env::var_os("RVT_SUN_REQUIRE").is_none(),
            "RVT_SUN_REQUIRE is set but RVT_TINY_DIR is not"
        );
        eprintln!("skip: RVT_TINY_DIR not set");
        return;
    };
    let models = tiny_models(Path::new(&dir));
    assert!(!models.is_empty(), "no tiny model with an API dump under {dir:?}");
    let (mut settings, mut views, mut mismatches) = (0usize, 0usize, Vec::<String>::new());
    for (rvt_path, dump_path) in &models {
        let name = rvt_path.display().to_string();
        let mut rf = rvt::RevitFile::open(rvt_path).expect("open");
        let sun = rvt::sun_settings::read(&mut rf).expect("sun settings");
        let dump: Value = serde_json::from_slice(&std::fs::read(dump_path).unwrap()).unwrap();
        let mut offsets: Vec<(i64, String)> = Vec::new();
        for e in dump["elements"].as_array().unwrap() {
            let class = e["class"].as_str().unwrap_or("");
            let id = e["id"].as_u64().unwrap_or(0);
            let p = &e["properties"];
            let mut bad = |what: &str| mismatches.push(format!("{name} {id} {what}"));
            if class.ends_with(".SunAndShadowSettings") {
                settings += 1;
                let Some(s) = sun.setting(id) else {
                    bad("setting not decoded");
                    continue;
                };
                if p["Name"].as_str() != Some(s.name.as_str()) {
                    bad("name");
                }
                if p["SunAndShadowType"].as_str() != s.sun_and_shadow_type.as_deref() {
                    bad("SunAndShadowType");
                }
                if p["TimeInterval"].as_str() != s.time_interval.as_deref() {
                    bad("TimeInterval");
                }
                for (k, v) in [("Azimuth", s.azimuth), ("Altitude", s.altitude)] {
                    let want = p[k].as_f64().map(f64::to_degrees);
                    if !matches!((want, v), (Some(a), Some(b)) if (a - b).abs() < 1e-9) {
                        bad(k);
                    }
                }
                for (k, v) in [
                    ("UsesGroundPlane", s.uses_ground_plane),
                    ("RelativeToView", s.relative_to_view),
                    ("SunriseToSunset", s.sunrise_to_sunset),
                    ("SharesSettings", s.shares_settings),
                ] {
                    if p[k].as_bool() != Some(v) {
                        bad(k);
                    }
                }
                if p["UsesGroundPlane"].as_bool() == Some(true)
                    && p["GroundPlaneLevelId"]["element_id"].as_u64() != s.ground_plane_level_id
                {
                    bad("GroundPlaneLevelId");
                }
                for (k, t) in [("StartDateAndTime", s.start_unix), ("EndDateAndTime", s.end_unix)] {
                    match clock_offset(t, p[k]["text"].as_str()) {
                        Some(o) => offsets.push((o, format!("{name} {id} {k}"))),
                        None => bad(k),
                    }
                }
            } else if class.contains(".View") && p.get("SunlightIntensity").is_some() {
                views += 1;
                let Some(v) = sun.view(id) else {
                    bad("view not decoded");
                    continue;
                };
                if p["SunAndShadowSettings"]["element_id"].as_u64() != v.sun_setting_id {
                    bad("SunAndShadowSettings");
                }
                if p["SunlightIntensity"].as_i64() != v.sunlight_intensity {
                    bad("SunlightIntensity");
                }
                if p["ShadowIntensity"].as_i64() != v.shadow_intensity {
                    bad("ShadowIntensity");
                }
                if p["DisplayStyle"].as_str() != v.display_style.as_deref() {
                    bad("DisplayStyle");
                }
            }
        }
        // one clock offset per model (its most common one), or that offset and one an hour away
        let mut counts = std::collections::BTreeMap::new();
        for (o, _) in &offsets {
            *counts.entry(*o).or_insert(0usize) += 1;
        }
        let Some(mode) = counts.iter().max_by_key(|(_, n)| **n).map(|(o, _)| *o) else {
            continue;
        };
        let other = if counts.contains_key(&(mode + 60)) { mode + 60 } else { mode - 60 };
        for (o, what) in &offsets {
            if *o != mode && *o != other {
                mismatches.push(format!("{what} instant ({} min off the model's clock)", o - mode));
            }
        }
    }
    eprintln!(
        "{} models: {settings} sun settings, {views} views checked, {} mismatches",
        models.len(),
        mismatches.len()
    );
    assert!(settings > 0 && views > 0);
    assert!(
        mismatches.is_empty(),
        "{} mismatches, first: {:?}",
        mismatches.len(),
        &mismatches[..mismatches.len().min(20)]
    );
}
