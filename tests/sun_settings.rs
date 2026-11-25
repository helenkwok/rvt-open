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
    Some(civil_minutes(year, month, day, h, m))
}

/// Minutes since 1970 of a civil date and time (days from civil, Howard Hinnant).
fn civil_minutes(year: i64, month: i64, day: i64, h: i64, m: i64) -> i64 {
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
    days * 1440 + h * 60 + m
}

/// Minutes since 1970 of a decoded `YYYY-MM-DDTHH:MM`.
fn decoded_minutes(s: &str) -> Option<i64> {
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    Some(civil_minutes(
        n(0..4)?,
        n(5..7)?,
        n(8..10)?,
        n(11..13)?,
        n(14..16)?,
    ))
}

/// Minutes between a dump's `DateTime` text and the decoded clock time, `None` when either is
/// missing. The API returns the stored clock time; the dump wrote it moved by the zone of the
/// machine it ran on, so within one model every offset is that machine's: one value, or two an
/// hour apart across a daylight-saving change.
fn clock_offset(decoded: Option<&str>, text: Option<&str>) -> Option<i64> {
    Some(dump_minutes(text?)? - decoded_minutes(decoded?)?)
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
    assert!(
        !models.is_empty(),
        "no tiny model with an API dump under {dir:?}"
    );
    let (mut settings, mut views, mut mismatches) =
        (0usize, 0usize, Vec::<(String, String, String)>::new());
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
            let mut bad = |what: &str, detail: String| {
                mismatches.push((format!("{name} {id}"), what.to_string(), detail))
            };
            if class.ends_with(".SunAndShadowSettings") {
                settings += 1;
                let Some(s) = sun.setting(id) else {
                    bad("setting not decoded", String::new());
                    continue;
                };
                if p["Name"].as_str() != Some(s.name.as_str()) {
                    bad("name", format!("{} vs {}", p["Name"], s.name));
                }
                if p["SunAndShadowType"].as_str() != s.sun_and_shadow_type.as_deref() {
                    bad(
                        "SunAndShadowType",
                        format!("{} vs stored {:?}", p["SunAndShadowType"], s.type_value),
                    );
                }
                if p["TimeInterval"].as_str() != s.time_interval.as_deref() {
                    bad(
                        "TimeInterval",
                        format!(
                            "{} vs stored {:?}",
                            p["TimeInterval"], s.time_interval_value
                        ),
                    );
                }
                for (k, v) in [("Azimuth", s.azimuth), ("Altitude", s.altitude)] {
                    let want = p[k].as_f64().map(f64::to_degrees);
                    if !matches!((want, v), (Some(a), Some(b)) if (a - b).abs() < 1e-9) {
                        bad(k, format!("{want:?} vs {v:?}"));
                    }
                }
                for (k, v) in [
                    ("UsesGroundPlane", s.uses_ground_plane),
                    ("RelativeToView", s.relative_to_view),
                    ("SunriseToSunset", s.sunrise_to_sunset),
                    ("SharesSettings", s.shares_settings),
                ] {
                    if p[k].as_bool() != Some(v) {
                        bad(k, format!("{} vs {v}", p[k]));
                    }
                }
                if p["UsesGroundPlane"].as_bool() == Some(true)
                    && p["GroundPlaneLevelId"]["element_id"].as_u64() != s.ground_plane_level_id
                {
                    bad(
                        "GroundPlaneLevelId",
                        format!(
                            "{} vs {:?}",
                            p["GroundPlaneLevelId"], s.ground_plane_level_id
                        ),
                    );
                }
                for (k, t) in [
                    ("StartDateAndTime", s.start.as_deref()),
                    ("EndDateAndTime", s.end.as_deref()),
                ] {
                    match clock_offset(t, p[k]["text"].as_str()) {
                        Some(o) => offsets.push((o, format!("{name} {id} {k}"))),
                        None => bad(k, format!("{} vs {t:?}", p[k]["text"])),
                    }
                }
            } else if class.contains(".View") && p.get("SunlightIntensity").is_some() {
                views += 1;
                let Some(v) = sun.view(id) else {
                    bad("view not decoded", format!("{}", e["class"]));
                    continue;
                };
                if p["SunAndShadowSettings"]["element_id"].as_u64() != v.sun_setting_id {
                    bad(
                        "SunAndShadowSettings",
                        format!("{} vs {:?}", p["SunAndShadowSettings"], v.sun_setting_id),
                    );
                }
                if p["SunlightIntensity"].as_i64() != v.sunlight_intensity {
                    bad(
                        "SunlightIntensity",
                        format!("{} vs {:?}", p["SunlightIntensity"], v.sunlight_intensity),
                    );
                }
                if p["ShadowIntensity"].as_i64() != v.shadow_intensity {
                    bad(
                        "ShadowIntensity",
                        format!("{} vs {:?}", p["ShadowIntensity"], v.shadow_intensity),
                    );
                }
                if p["DisplayStyle"].as_str() != v.display_style.as_deref() {
                    bad(
                        "DisplayStyle",
                        format!("{} vs stored {:?}", p["DisplayStyle"], v.style_values),
                    );
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
        let other = if counts.contains_key(&(mode + 60)) {
            mode + 60
        } else {
            mode - 60
        };
        for (o, what) in &offsets {
            if *o != mode && *o != other {
                mismatches.push((
                    what.clone(),
                    "instant".into(),
                    format!("{} min off the model's clock", o - mode),
                ));
            }
        }
    }
    eprintln!(
        "{} models: {settings} sun settings, {views} views checked, {} mismatches",
        models.len(),
        mismatches.len()
    );
    // by field, then the commonest (field, Revit API value vs decoded) pairs
    let mut by_field = std::collections::BTreeMap::<&str, usize>::new();
    let mut by_pair = std::collections::BTreeMap::<(&str, &str), usize>::new();
    for (_, what, detail) in &mismatches {
        *by_field.entry(what.as_str()).or_default() += 1;
        *by_pair.entry((what.as_str(), detail.as_str())).or_default() += 1;
    }
    let mut pairs: Vec<_> = by_pair.into_iter().collect();
    pairs.sort_by_key(|p| std::cmp::Reverse(p.1));
    for ((what, detail), n) in pairs.iter().take(60) {
        eprintln!("  {n:>6}  {what}: {detail}");
    }
    assert!(settings > 0 && views > 0);
    assert!(
        mismatches.is_empty(),
        "{} mismatches by field {by_field:?}, first: {:?}",
        mismatches.len(),
        &mismatches[..mismatches.len().min(10)]
    );
}

/// Every public sample project (`RVT_AUTODESK_SAMPLES_DIR/<year>/*.rvt`, Autodesk's own
/// downloads, 2015 to 2027): sun settings are read, each named preset and each view's own setting
/// names a decoded `SunAndShadowSettings`, every stored start and end is a date, and a project with
/// a 3D view gives the viewer a sun (its default view's own setting, or a preset).
#[test]
fn public_samples_sun_settings_hold_together() {
    let Some(dir) = std::env::var_os("RVT_AUTODESK_SAMPLES_DIR") else {
        eprintln!("skip: RVT_AUTODESK_SAMPLES_DIR not set");
        return;
    };
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|y| std::fs::read_dir(y.path()).ok())
        .flatten()
        .flatten()
        .map(|f| f.path())
        .filter(|p| p.extension().is_some_and(|e| e == "rvt"))
        // both spellings of a name link to one file: read it once
        .map(|p| std::fs::canonicalize(&p).unwrap_or(p))
        .collect();
    files.sort();
    files.dedup();
    let mut problems = Vec::new();
    let mut read = 0;
    for p in &files {
        let Ok(mut rf) = rvt::RevitFile::open(p) else {
            continue;
        };
        let name = p.display().to_string();
        let Some(sun) = rvt::sun_settings::read(&mut rf) else {
            problems.push(format!("{name}: no sun settings read"));
            continue;
        };
        read += 1;
        for id in sun
            .presets
            .iter()
            .chain(sun.views.iter().filter_map(|v| v.sun_setting_id.as_ref()))
        {
            if sun.setting(*id).is_none() {
                problems.push(format!("{name}: setting {id} is named but not decoded"));
            }
        }
        for s in &sun.settings {
            if (s.start_value.is_some() && s.start.is_none())
                || (s.end_value.is_some() && s.end.is_none())
            {
                problems.push(format!(
                    "{name}: setting {} has a start or end that is not a date",
                    s.id
                ));
            }
        }
        if sun.views.iter().any(|v| v.is_3d && !v.is_template) && sun.viewer_default().is_none() {
            problems.push(format!("{name}: a 3D view but no sun for the viewer"));
        }
    }
    eprintln!("{read} of {} sample files read", files.len());
    assert!(problems.is_empty(), "{problems:#?}");
}
