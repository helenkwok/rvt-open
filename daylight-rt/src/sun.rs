//! The solar position as Radiance's `gensky` computes it (its `sun.c`): the
//! declination and the equation of time from the day of the year, the solar
//! time from the standard meridian, then altitude and azimuth.
//!
//! Times are local standard time (no daylight saving), days of a non-leap year,
//! as gensky takes them. Directions are unit vectors toward the sun with x east,
//! y north, z up in the true frame; [`to_project`] turns them into the model's
//! frame.

use crate::V;
use std::f64::consts::PI;

/// Where the site is: degrees north, degrees east, hours east of UTC (standard time).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Site {
    pub latitude: f64,
    pub longitude: f64,
    pub time_zone: f64,
}

impl Site {
    /// The standard meridian, degrees west, as gensky's `-m` takes it.
    pub fn meridian_west(&self) -> f64 {
        -15.0 * self.time_zone
    }
    /// The longitude, degrees west, as gensky's `-o` takes it.
    pub fn longitude_west(&self) -> f64 {
        -self.longitude
    }
}

const MONTH_START: [u32; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
const MONTH_DAYS: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

/// Day of the year (1 to 365) as gensky's `jdate` counts it.
pub fn day_of_year(month: u32, day: u32) -> u32 {
    MONTH_START[(month.clamp(1, 12) - 1) as usize] + day
}

/// Month and day of a day of the year (1 to 365).
pub fn month_day(doy: u32) -> (u32, u32) {
    let doy = doy.clamp(1, 365);
    let m = MONTH_START.iter().rposition(|&s| s < doy).unwrap_or(0);
    (m as u32 + 1, doy - MONTH_START[m])
}

/// Days in `month` of a non-leap year.
pub fn days_in_month(month: u32) -> u32 {
    MONTH_DAYS[(month.clamp(1, 12) - 1) as usize]
}

/// Solar altitude and azimuth (radians; azimuth from south, positive toward
/// west) at local standard time `hour` on day `doy`.
pub fn altitude_azimuth(site: &Site, doy: u32, hour: f64) -> (f64, f64) {
    let jd = f64::from(doy);
    let lat = site.latitude.to_radians();
    let decl = 0.4093 * (2.0 * PI * (jd - 81.0) / 368.0).sin();
    let eot = 0.170 * (4.0 * PI * (jd - 80.0) / 373.0).sin()
        - 0.129 * (2.0 * PI * (jd - 8.0) / 355.0).sin();
    let st = hour + eot + 12.0 * (site.meridian_west() - site.longitude_west()).to_radians() / PI;
    let h = st * PI / 12.0;
    let alt = (lat.sin() * decl.sin() - lat.cos() * decl.cos() * h.cos()).asin();
    let azi =
        -(decl.cos() * h.sin()).atan2(-lat.cos() * decl.sin() - lat.sin() * decl.cos() * h.cos());
    (alt, azi)
}

/// The unit vector toward the sun, true frame (x east, y north, z up).
pub fn direction(site: &Site, doy: u32, hour: f64) -> V {
    let (alt, azi) = altitude_azimuth(site, doy, hour);
    [-azi.sin() * alt.cos(), -azi.cos() * alt.cos(), alt.sin()]
}

/// Turns a true-frame direction into the model's frame, given Revit's angle to
/// true north: the angle, counter-clockwise in degrees, of the shared
/// coordinates' east axis in the model's coordinates.
pub fn to_project(v: V, true_north_deg: f64) -> V {
    let (s, c) = true_north_deg.to_radians().sin_cos();
    [v[0] * c - v[1] * s, v[0] * s + v[1] * c, v[2]]
}

/// Sun directions (model frame) above the horizon from day `from` to day `to`
/// (days of the year, inclusive; wraps past the year's end), every `step_min`
/// minutes from midnight, standard time. Each stands for `step_min` minutes of sun.
pub fn positions(site: &Site, true_north_deg: f64, from: u32, to: u32, step_min: u32) -> Vec<V> {
    let step = step_min.clamp(1, 24 * 60);
    let mut out = Vec::new();
    let (from, to) = (from.clamp(1, 365), to.clamp(1, 365));
    let n = if to >= from {
        to - from + 1
    } else {
        365 - from + 1 + to
    };
    for k in 0..n {
        let doy = (from - 1 + k) % 365 + 1;
        let mut m = 0;
        while m < 24 * 60 {
            let d = direction(site, doy, f64::from(m) / 60.0);
            if d[2] > 0.0 {
                out.push(to_project(d, true_north_deg));
            }
            m += step;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days() {
        assert_eq!(day_of_year(1, 1), 1);
        assert_eq!(day_of_year(12, 31), 365);
        assert_eq!(month_day(80), (3, 21));
        assert_eq!(month_day(365), (12, 31));
        for doy in 1..=365 {
            let (m, d) = month_day(doy);
            assert_eq!(day_of_year(m, d), doy);
        }
    }

    #[test]
    fn noon_on_the_equinox_at_the_equator_is_overhead_and_north_turns() {
        let site = Site {
            latitude: 0.0,
            longitude: 0.0,
            time_zone: 0.0,
        };
        // solar noon differs from 12:00 by the equation of time; search the day for the highest sun
        let top = (0..1440)
            .map(|m| direction(&site, day_of_year(3, 21), f64::from(m) / 60.0)[2])
            .fold(0.0, f64::max);
        assert!(top > 0.9999, "{top}");
        // in a model turned so true east is 90 degrees counter-clockwise of its x axis, east becomes +y
        let v = to_project([1.0, 0.0, 0.0], 90.0);
        assert!((v[0]).abs() < 1e-12 && (v[1] - 1.0).abs() < 1e-12);
        let summer = positions(
            &Site {
                latitude: 51.5,
                longitude: 0.0,
                time_zone: 0.0,
            },
            0.0,
            172,
            172,
            60,
        );
        assert_eq!(
            summer.len(),
            17,
            "about 16.5 hours of sun at London on 21 June"
        );
    }
}
