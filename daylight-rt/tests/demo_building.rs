//! The instant tier against Radiance 6.0.3 on the CC0 demo building (a 40 x 30 ft
//! single-storey building released under CC0 1.0; see
//! `tests/data/demo-building/README.md`).
//!
//! The data folder holds the building's Radiance scene and its 391 sensors
//! (0.76 m above the floors, 0.5 m apart), as built from rvt-open's glTF export,
//! and native Radiance's answers on them:
//!
//! - `ref_sun.dat`: hours of direct sun per sensor over 36 hourly sun positions
//!   (20 March, 21 June, 21 December, 05:00 to 19:00 standard time), each from
//!   `gensky M D H -a 42.35866 -o 71.05674 -m 75 +s` and `rtrace -I -ab 0 -dj 0`;
//!   the sun directions themselves in `sun_dirs.txt`;
//! - `ref_svf.dat`: the sky view factor, `rtrace -I -ab 1 -aa 0 -ad 16384` under a
//!   uniform sky, divided by pi;
//! - `df_p1.dat` / `df_p1_n8.dat`: the daylight factor's irradiance under
//!   `gensky -ang 45 0 -c -B 55.866`, `-ab 2 -aa 0.1 -ad 2048 -ar 64`, one
//!   process and eight (the noise between two Radiance runs).
//!
//! `RVT_DAYLIGHT_DEMO_RVT` names the `.rvt` (`research/cc0-models.tsv`)
//! for the test that builds the scene from the file itself; it skips when unset,
//! and `RVT_DAYLIGHT_REQUIRE=1` makes a missing file fail instead.

use daylight_rt::{Sensor, Tracer, V, radiance, sun};
use std::path::PathBuf;

const DATA: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/demo-building");

fn read(name: &str) -> String {
    std::fs::read_to_string(format!("{DATA}/{name}")).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn column(name: &str) -> Vec<f64> {
    read(name)
        .lines()
        .filter_map(|l| l.split_whitespace().next()?.parse().ok())
        .collect()
}

/// Daylight factor (%) from Radiance's irradiance columns (RGB, 179 lm/W, 10,000 lux sky).
fn df_column(name: &str) -> Vec<f64> {
    read(name)
        .lines()
        .filter_map(|l| {
            let v: Vec<f64> = l
                .split_whitespace()
                .filter_map(|x| x.parse().ok())
                .collect();
            (v.len() >= 3).then(|| 100.0 * 179.0 * daylight_rt::grey([v[0], v[1], v[2]]) / 10_000.0)
        })
        .collect()
}

fn mean(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len() as f64
}

fn rmse(a: &[f64], b: &[f64]) -> f64 {
    assert_eq!(a.len(), b.len());
    (a.iter().zip(b).map(|(x, y)| (x - y).powi(2)).sum::<f64>() / a.len() as f64).sqrt()
}

fn scene() -> (Tracer, Vec<Sensor>) {
    let model = radiance::parse(&[&read("materials.rad"), &read("scene.rad")]).expect("scene");
    assert_eq!(model.triangles.len(), 1968);
    let sensors = radiance::parse_sensors(&read("grid.pts"));
    assert_eq!(sensors.len(), 391);
    (Tracer::new(&model), sensors)
}

const SITE: sun::Site = sun::Site {
    latitude: 42.35866,
    longitude: -71.05674,
    time_zone: -5.0,
};

/// The reference's 36 sun positions: three days, every hour from 05:00 to 19:00, above the horizon.
fn suns() -> Vec<(u32, u32, u32, V)> {
    let mut out = Vec::new();
    for (m, d) in [(3, 20), (6, 21), (12, 21)] {
        for h in 5..20u32 {
            let v = sun::direction(&SITE, sun::day_of_year(m, d), f64::from(h));
            if v[2] > 0.0 {
                out.push((m, d, h, v));
            }
        }
    }
    out
}

#[test]
fn sun_directions_match_gensky() {
    let ours = suns();
    let theirs: Vec<Vec<f64>> = read("sun_dirs.txt")
        .lines()
        .map(|l| {
            l.split_whitespace()
                .filter_map(|x| x.parse().ok())
                .collect()
        })
        .collect();
    assert_eq!(
        ours.len(),
        theirs.len(),
        "the same sun positions are above the horizon"
    );
    let mut worst = 0.0f64;
    for ((m, d, h, v), t) in ours.iter().zip(&theirs) {
        assert_eq!(
            [f64::from(*m), f64::from(*d), f64::from(*h)],
            [t[0], t[1], t[2]]
        );
        let c = (v[0] * t[3] + v[1] * t[4] + v[2] * t[5])
            / (t[3] * t[3] + t[4] * t[4] + t[5] * t[5]).sqrt();
        worst = worst.max(c.clamp(-1.0, 1.0).acos().to_degrees());
    }
    // gensky prints six decimals
    assert!(worst < 0.02, "worst angle to gensky's sun {worst} degrees");
    eprintln!(
        "sun directions: worst angle to gensky {worst:.4} degrees over {}",
        ours.len()
    );
}

#[test]
fn sun_hours_equal_radiance_on_every_sensor() {
    let (tracer, sensors) = scene();
    let dirs: Vec<V> = suns().into_iter().map(|s| s.3).collect();
    let ours = daylight_rt::sun_hours(&tracer, &sensors, &dirs);
    let theirs = column("ref_sun.dat");
    let same = ours
        .iter()
        .zip(&theirs)
        .filter(|(a, b)| f64::from(**a) == **b)
        .count();
    assert_eq!(
        same,
        sensors.len(),
        "sensors with the same sun hours as Radiance"
    );
    eprintln!(
        "sun hours: {same}/{} sensors equal to Radiance",
        sensors.len()
    );
}

#[test]
fn sky_view_factor_within_the_measured_rmse() {
    let (tracer, sensors) = scene();
    let ours = daylight_rt::sky_view(&tracer, &sensors, 0, 4096);
    let theirs = column("ref_svf.dat");
    let (e, d) = (rmse(&ours, &theirs), (mean(&ours) - mean(&theirs)).abs());
    eprintln!(
        "sky view factor: RMSE {e:.5}, means {:.5} / {:.5}",
        mean(&ours),
        mean(&theirs)
    );
    assert!(e <= 0.0020, "RMSE {e}");
    assert!(d <= 0.0005, "mean differs by {d}");
}

#[test]
fn daylight_factor_preview_mean_within_one_percent_of_radiance() {
    let (tracer, sensors) = scene();
    let ours = daylight_rt::daylight_factor(&tracer, &sensors, 0, 2, 4096, 0.2);
    let theirs = df_column("df_p1.dat");
    let floor = rmse(&theirs, &df_column("df_p1_n8.dat"));
    let (e, rel) = (rmse(&ours, &theirs), mean(&ours) / mean(&theirs) - 1.0);
    eprintln!(
        "daylight factor ab2, 4096 paths: means {:.4} / {:.4} %, RMSE {e:.3} (Radiance against itself {floor:.3})",
        mean(&ours),
        mean(&theirs)
    );
    assert!(rel.abs() <= 0.01, "mean off by {:.2} %", 100.0 * rel);
    assert!(e <= 0.16, "RMSE {e}");
}

#[test]
fn the_scene_built_from_the_file_is_the_reference_scene() {
    let Some(path) = std::env::var_os("RVT_DAYLIGHT_DEMO_RVT").map(PathBuf::from) else {
        assert!(
            std::env::var("RVT_DAYLIGHT_REQUIRE").as_deref() != Ok("1"),
            "RVT_DAYLIGHT_DEMO_RVT is not set"
        );
        eprintln!("skipping: RVT_DAYLIGHT_DEMO_RVT is not set");
        return;
    };
    let mut rf = rvt::RevitFile::open(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let opts = rvt::convert::GltfModelOptions {
        saved_meshes: true,
        family_geometry: true,
        saved_materials: true,
        proxy_shapes: true,
        walker_limits: Default::default(),
    };
    let (mut model, _) =
        rvt::convert::gltf_model(&mut rf, &opts, &rvt::control::WalkerControl::new())
            .expect("export");
    rvt::ifc::body_geometry::share_shapes_by_convention(&mut model);
    let geo = model
        .georeference
        .clone()
        .expect("the file has a site location");
    assert!(
        (geo.latitude - SITE.latitude).abs() < 1e-4
            && (geo.longitude - SITE.longitude).abs() < 1e-4,
        "{geo:?}"
    );
    let glb = rvt::ifc::gltf::model_to_glb_with(
        &model,
        &rvt::ifc::gltf::GltfOptions::new(false, false, false),
    );
    let p = daylight_rt::glb::read(&glb, &Default::default()).expect("scene");

    let (_, reference) = scene();
    eprintln!(
        "from the file: {} triangles, {} sensors",
        p.model.triangles.len(),
        p.sensors.len()
    );
    assert_eq!(p.sensors, reference, "the sensors are the reference grid");
    let tracer = Tracer::new(&p.model);
    let dirs: Vec<V> = suns().into_iter().map(|s| s.3).collect();
    let ours = daylight_rt::sun_hours(&tracer, &p.sensors, &dirs);
    let theirs = column("ref_sun.dat");
    let same = ours
        .iter()
        .zip(&theirs)
        .filter(|(a, b)| f64::from(**a) == **b)
        .count();
    assert_eq!(
        same,
        theirs.len(),
        "sensors with Radiance's sun hours, scene built from the file"
    );
    // the Radiance text the validated tier reads holds the same scene
    let again = radiance::parse(&[
        &radiance::write_materials(&p.model),
        &radiance::write_triangles(&p.model),
    ])
    .unwrap();
    assert_eq!(
        again.triangles.iter().map(|t| t.v).collect::<Vec<_>>(),
        p.model
            .triangles
            .iter()
            .zip(radiance::kept(&p.model).0)
            .filter(|(_, k)| *k)
            .map(|(t, _)| t.v)
            .collect::<Vec<_>>()
    );
}
