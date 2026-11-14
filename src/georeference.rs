//! The model's georeference: Revit's site location, shared coordinates,
//! survey point, project base point, true north and coordinate reference
//! system.
//!
//! Read from the element data records by the shape of their values, and
//!
//! Each element's data follows a prologue `u32, ElementId (u32 before 2024,
//! u64 from 2024), u64, class tag u16, 00 00`, the class tag being the
//! definition ordinal of `GeoLocation`, `GeoSite` or `BasePoint` in the file's
//! own `Formats/Latest`; the newest partition copy wins.
//!
//! - **ProjectLocation** (the API's `ProjectLocation`, schema `GeoLocation`):
//!   a length-prefixed UTF-16 name, then a transform of twelve `f64` (a
//!   row-major 3x3 rotation about Z, then the origin, feet) that equals the
//!   API's `GetTotalTransform` (shared coordinates to internal ones), then the
//!   ElementId of its `SiteLocation`.
//! - **SiteLocation** (schema `GeoSite`): latitude and longitude (radians), time zone (hours),
//!   elevation (feet), one more value and `1.0`, as six `f64`; further on a
//!   length-prefixed UTF-16 coordinate-system definition (an OSGeo MapGuide
//!   `Dictionary`) when one was set.
//! - **BasePoint** (survey point and project base point): the `GPoint` that
//!   holds the point's internal position (`04 00 08 00`, then three `f64`
//!   feet), in a record that names the file's `GPoint` class after
//!   `03 00 00 00`. Which one is the survey point is the schema field
//!   `BasePoint::m_locationType` (`u32`, 1 = survey point, the API's
//!   `IsShared`; 0 = project base point), followed by the `bool`
//!   `m_isClippedToShared`. Where `m_oGPoint` is a field of
//!   `BasePoint` itself (2018, 2019) the two sit just before the `GPoint`
//!   mark; where it is not (2022 on) they follow the mark's six bytes and the
//!   24-byte `m_transformedPosition`.
//! - The active location: the record `ff ff ff ff <tag> 03 00 00 00` and
//!   three ElementIds, the first the API's active location, another one the
//!   location named "Project".
//! - The EPSG code: the `CoordinateSystem` alias in the `EPSG Code`
//!   namespace of the site's definition; else the `GeoRefEPSGCode` of the IFC
//!   export configuration stored in the file (JSON in UTF-16).
//!
//! Nothing here comes from Revit binaries; record shapes were found with our
//! own probes on files we may open and checked against the public API.

use crate::RevitFile;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const FT: f64 = 0.3048;

/// A point in internal coordinates and in shared coordinates, metres.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GeoPoint {
    /// The base point's ElementId.
    #[serde(default)]
    pub element_id: u64,
    /// Position in the model's internal coordinates (what the geometry uses).
    pub internal: [f64; 3],
    /// Position in shared coordinates: east, north, elevation.
    pub shared: [f64; 3],
}

/// The coordinate reference system the model names.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Crs {
    /// The CRS's own name (`PA83-SF`, `ETRS89.GK25FIN`), or `EPSG:<code>`.
    pub name: String,
    /// EPSG code, when the file gives one.
    pub epsg: Option<u32>,
    pub description: Option<String>,
    /// The unit of the CRS's axes as the file names it (`FOOT`, `METER`,
    /// `METRE`), when it does.
    pub map_unit: Option<String>,
    /// `SiteLocation` (Revit's coordinate system) or `IfcExportConfiguration`.
    pub source: String,
}

impl Crs {
    /// Feet-based (CS-MAP `FOOT` is the US survey foot), else metres. The
    /// IFC4 writer names that unit `US SURVEY FOOT`.
    pub fn in_feet(&self) -> bool {
        self.map_unit
            .as_deref()
            .is_some_and(|u| u.to_ascii_uppercase().contains("FOOT") || u.contains("FEET"))
    }

    /// A shared coordinate (metres) in the CRS's unit. Revit keeps shared
    /// coordinates in feet; a feet-based CRS reads them as its own unit.
    pub fn map_value(&self, metres: f64) -> f64 {
        if self.in_feet() { metres / FT } else { metres }
    }
}

/// The georeference of a model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Georeference {
    /// The active project location's name.
    pub location_name: String,
    /// Site latitude and longitude, degrees; elevation, metres; time zone, hours.
    pub latitude: f64,
    pub longitude: f64,
    pub elevation: f64,
    pub time_zone: f64,
    /// Revit's Angle to True North: the angle of the shared coordinates'
    /// east axis in internal coordinates, degrees in [0, 360).
    pub true_north: f64,
    /// Shared coordinates (east, north, elevation; metres) of the internal
    /// origin.
    pub origin_shared: [f64; 3],
    pub survey_point: Option<GeoPoint>,
    pub project_base_point: Option<GeoPoint>,
    pub crs: Option<Crs>,
    /// When the CRS is unknown because the file's saved IFC export
    /// configurations name different EPSG codes (no stored flag
    /// says which is in use), one CRS per code, for the caller to report.
    /// Not written to any output.
    #[serde(skip)]
    pub crs_candidates: Vec<Crs>,
    /// The elements the georeference was read from, for IFCX. Not written
    /// to any other output.
    #[serde(skip)]
    pub elements: SiteElements,
}

/// The project locations, site locations and base points of a model.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SiteElements {
    /// Each project location (`GeoLocation`): its ElementId, name and the
    /// ElementId of its site location.
    pub locations: Vec<(u64, String, u64)>,
    /// Each site location's (`GeoSite`) ElementId.
    pub sites: Vec<u64>,
    /// Each base point: its ElementId, its position in the active location's
    /// shared coordinates (east, north, elevation; feet) and its
    /// `m_locationType` (1 survey point, 0 project base point).
    pub base_points: Vec<(u64, [f64; 3], Option<u32>)>,
}

/// Source of a CRS chosen with `rvt-ifc --epsg`.
pub const USER_OVERRIDE: &str = "UserOverride";

impl Georeference {
    /// Force the projected CRS to EPSG `code` (`rvt-ifc --epsg`).
    /// The CRS is marked as user-supplied (`source` [`USER_OVERRIDE`], and
    /// the description says so). Its unit is the decoded CRS's when that is
    /// in feet, else metres: no EPSG database is consulted. Returns a warning
    /// when the file itself declares a different single code.
    pub fn override_epsg(&mut self, code: u32) -> Option<String> {
        let decoded = self.crs.take();
        let warning = decoded
            .as_ref()
            .and_then(|c| c.epsg)
            .filter(|&e| e != code)
            .map(|e| {
                format!(
                    "--epsg {code} contradicts EPSG {e} declared by the file ({}); using {code}",
                    decoded.as_ref().map_or("", |c| c.source.as_str())
                )
            });
        let map_unit = match &decoded {
            Some(c) if c.in_feet() => c.map_unit.clone(),
            _ => Some("METRE".into()),
        };
        self.crs = Some(Crs {
            name: format!("EPSG:{code}"),
            epsg: Some(code),
            description: Some(format!(
                "EPSG:{code}, user-supplied (rvt-ifc --epsg), not stored in the file"
            )),
            map_unit,
            source: USER_OVERRIDE.into(),
        });
        warning
    }

    /// The warning for a CRS left unknown between saved configurations that
    /// name different codes, `None` otherwise.
    pub fn unresolved_crs_warning(&self) -> Option<String> {
        if self.crs.is_some() || self.crs_candidates.len() < 2 {
            return None;
        }
        let names: Vec<String> = self
            .crs_candidates
            .iter()
            .map(|c| {
                format!(
                    "EPSG {} ({})",
                    c.epsg.map_or("?".into(), |e| e.to_string()),
                    c.name
                )
            })
            .collect();
        Some(format!(
            "the file's saved IFC export configurations name {}; CRS left unknown; pass --epsg to choose",
            names.join(" and ")
        ))
    }

    /// Shared coordinates (metres) of an internal point (metres):
    /// `R^T p + origin_shared`, R the rotation by [`Self::true_north`].
    pub fn to_shared(&self, p: [f64; 3]) -> [f64; 3] {
        let (s, c) = self.true_north.to_radians().sin_cos();
        let [e, n, h] = self.origin_shared;
        [c * p[0] + s * p[1] + e, -s * p[0] + c * p[1] + n, p[2] + h]
    }
}

/// One saved IFC export configuration's CRS: EPSG code, name, description,
/// map unit.
type ExportCrs = (String, Option<String>, Option<String>, Option<String>);

#[derive(Clone, Copy)]
enum Kind {
    Location,
    Site,
    BasePoint,
}

/// One `ProjectLocation` record.
#[derive(Debug, Clone)]
struct Location {
    name: String,
    /// Row-major rotation and origin (feet): internal = R * shared + o.
    m: [f64; 12],
    site: u64,
}

/// One `SiteLocation` record.
#[derive(Debug, Clone)]
struct Site {
    lat: f64,
    lon: f64,
    tz: f64,
    elev: f64,
    definition: Option<String>,
}

fn f64_at(b: &[u8], o: usize) -> Option<f64> {
    b.get(o..o + 8)
        .map(|x| f64::from_le_bytes(x.try_into().expect("8 bytes")))
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    b.get(o..o + 4)
        .map(|x| u32::from_le_bytes(x.try_into().expect("4 bytes")))
}

fn id_at(b: &[u8], o: usize, wide: bool) -> Option<u64> {
    if wide {
        b.get(o..o + 8)
            .map(|x| u64::from_le_bytes(x.try_into().expect("8 bytes")))
    } else {
        u32_at(b, o).map(u64::from)
    }
}

/// A length-prefixed UTF-16 string at `o` of `1..=max` printable characters.
fn utf16_at(b: &[u8], o: usize, max: u32) -> Option<String> {
    let n = u32_at(b, o)?;
    if n == 0 || n > max {
        return None;
    }
    let bytes = b.get(o + 4..o + 4 + 2 * n as usize)?;
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let s = String::from_utf16(&units).ok()?;
    s.chars()
        .all(|c| !c.is_control() || c == '\n' || c == '\r' || c == '\t')
        .then_some(s)
}

/// The six-value block of a `SiteLocation`: latitude, longitude, time zone,
/// elevation, one more value, `1.0`; then `ff`x8 and a `u32` 0 or 1 (2024
/// on, 64-bit ids), or a `u32` 0, 16 bytes and `ff`x4 (before 2024).
fn site_block(b: &[u8], wide: bool) -> Option<usize> {
    (16..b.len().min(260)).find(|&o| {
        let after = if wide {
            b.get(o + 48..o + 56) == Some(&[0xff; 8][..])
                && u32_at(b, o + 56).is_some_and(|v| v <= 1)
        } else {
            u32_at(b, o + 48) == Some(0) && b.get(o + 68..o + 72) == Some(&[0xff; 4][..])
        };
        let v: Option<Vec<f64>> = (0..6).map(|j| f64_at(b, o + 8 * j)).collect();
        let Some(v) = v else { return false };
        v[0].abs() <= std::f64::consts::FRAC_PI_2
            && v[1].abs() <= std::f64::consts::PI
            && (-14.0..=14.0).contains(&v[2])
            && (v[2] * 4.0).fract() == 0.0
            && v[3].is_finite()
            && v[3].abs() < 1e5
            && (0.0..=720.0).contains(&v[4])
            && v[5] == 1.0
            && after
    })
}

fn read_site(b: &[u8], wide: bool) -> Option<Site> {
    let o = site_block(b, wide)?;
    let definition = memchr::memmem::find(b, &utf16_bytes("<?xml"))
        .filter(|&x| x >= 4)
        .and_then(|x| utf16_at(b, x - 4, 1 << 20));
    Some(Site {
        lat: f64_at(b, o)?,
        lon: f64_at(b, o + 8)?,
        tz: f64_at(b, o + 16)?,
        elev: f64_at(b, o + 24)?,
        definition,
    })
}

/// A rotation about Z with an origin: twelve `f64`, row-major rotation first.
fn rigid_z(m: &[f64; 12]) -> bool {
    let (c, s) = (m[0], m[3]);
    m[2] == 0.0
        && m[5] == 0.0
        && m[6] == 0.0
        && m[7] == 0.0
        && (m[8] - 1.0).abs() < 1e-12
        && (m[4] - c).abs() < 1e-12
        && (m[1] + s).abs() < 1e-12
        && ((c * c + s * s) - 1.0).abs() < 1e-9
        && m[9..].iter().all(|v| v.is_finite() && v.abs() < 1e9)
}

/// A location candidate: a named record holding a rigid transform followed by
/// an ElementId (kept when that id is a `SiteLocation`).
fn read_location(b: &[u8], wide: bool) -> Option<Location> {
    let w = if wide { 8 } else { 4 };
    for o in 16..b.len().saturating_sub(96 + w) {
        if !f64_at(b, o + 64).is_some_and(|v| (v - 1.0).abs() < 1e-12) {
            continue;
        }
        let mut m = [0.0; 12];
        for (j, v) in m.iter_mut().enumerate() {
            *v = f64_at(b, o + 8 * j)?;
        }
        if !rigid_z(&m) {
            continue;
        }
        let site = id_at(b, o + 96, wide)?;
        // The name: `ff ff ff ff <tag> 00 00 00 00`, then the string.
        let name = (10..o)
            .filter(|&q| b[q - 10..q - 6] == [0xff; 4] && b[q - 4..q] == [0; 4])
            .find_map(|q| utf16_at(b, q, 256))
            .unwrap_or_default();
        return Some(Location { name, m, site });
    }
    None
}

fn utf16_bytes(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

/// `(code, name, description, axis unit)` of a MapGuide coordinate-system
/// dictionary: the `CoordinateSystem` alias in the EPSG namespace.
type Definition = (Option<u32>, Option<String>, Option<String>, Option<String>);

fn epsg_of_definition(xml: &str) -> Definition {
    let tag = |t: &str, from: &str| -> Option<String> {
        let open = format!("<{t}>");
        let a = from.find(&open)? + open.len();
        let z = from[a..].find(&format!("</{t}>"))? + a;
        Some(from[a..z].trim().to_string())
    };
    let mut code = None;
    let mut rest = xml;
    while let Some(a) = rest.find("<Alias ") {
        let z = rest[a..].find("</Alias>").map_or(rest.len(), |z| a + z);
        let alias = &rest[a..z];
        if alias.contains("type=\"CoordinateSystem\"")
            && tag("Namespace", alias).as_deref() == Some("EPSG Code")
        {
            code = alias
                .split("id=\"")
                .nth(1)
                .and_then(|s| s.split('"').next())
                .and_then(|s| s.parse().ok());
            break;
        }
        rest = &rest[z..];
    }
    let system = ["ProjectedCoordinateSystem", "GeographicCoordinateSystem"]
        .iter()
        .find_map(|t| {
            let a = xml.find(&format!("<{t} "))?;
            let z = xml[a..]
                .find(&format!("</{t}>"))
                .map_or(xml.len(), |z| a + z);
            Some(&xml[a..z])
        });
    let name = system.and_then(|s| tag("Name", s));
    let description = system.and_then(|s| tag("Description", s));
    let unit = system
        .and_then(|s| s.split("<Axis uom=\"").nth(1))
        .and_then(|s| s.split('"').next())
        .map(str::to_string);
    (code, name, description, unit)
}

/// A JSON string value `"key":"value"` in UTF-16 text, from `at`.
fn json_value(units: &[u16], key: &str) -> Option<String> {
    let text = String::from_utf16_lossy(units);
    let k = format!("\"{key}\":\"");
    let a = text.find(&k)? + k.len();
    let z = text[a..].find('"')? + a;
    Some(text[a..z].to_string())
}

/// `BasePoint::m_locationType` of a base point record (`head`, from the class
/// tag on) whose `GPoint` mark is at `g`: a `u32` 0 or 1 followed by the
/// `bool` `m_isClippedToShared`. `own_gpoint`: the file's `BasePoint` class
/// itself declares `m_oGPoint` (2018, 2019), so the two fields end just
/// before the mark; otherwise they follow the six-byte mark and the 24-byte
/// `m_transformedPosition`.
fn location_type(head: &[u8], g: usize, own_gpoint: bool) -> Option<u32> {
    let t = if own_gpoint {
        g.checked_sub(5)?
    } else {
        g + 30
    };
    let (v, clip) = (u32_at(head, t)?, *head.get(t + 4)?);
    (v <= 1 && clip <= 1).then_some(v)
}

/// A base point's ElementId and internal position (feet).
type BasePointAt = (u64, [f64; 3]);

/// The survey point (`m_locationType` 1, the API's `IsShared`) and the
/// project base point (0) among the base points; `None` unless exactly one
/// of each.
fn split_base_points(
    points: &BTreeMap<u64, ([f64; 3], Option<u32>)>,
) -> Option<(BasePointAt, BasePointAt)> {
    let one = |k: u32| {
        let mut found = points.iter().filter(|(_, (_, t))| *t == Some(k));
        let (id, (p, _)) = found.next()?;
        found.next().is_none().then_some((*id, *p))
    };
    Some((one(1)?, one(0)?))
}

/// The georeference of a project file, `None` when its records are not found.
pub fn read(rf: &mut RevitFile) -> Option<Georeference> {
    let wide = rf.basic_file_info().ok()?.version >= 2024;
    let classes = rf.schema_classes().ok()?;
    let tag_of = |n: &str| classes.classes.iter().find(|c| c.name == n).map(|c| c.tag);
    let gpoint = tag_of("GPoint")?.to_le_bytes();
    // Where `m_locationType` sits relative to the `GPoint` mark.
    let own_gpoint = rf.schema_layouts().ok().is_some_and(|l| {
        l.classes
            .get(&tag_of("BasePoint").unwrap_or(0))
            .is_some_and(|c| c.fields.iter().any(|f| f.name == "m_oGPoint"))
    });
    let kinds = [
        (Kind::Location, tag_of("GeoLocation")?),
        (Kind::Site, tag_of("GeoSite")?),
        (Kind::BasePoint, tag_of("BasePoint")?),
    ];
    let w = if wide { 8 } else { 4 };
    let gpoint_mark = [3, 0, 0, 0, gpoint[0], gpoint[1]];
    let config_key = utf16_bytes("\"GeoRefEPSGCode\":\"");

    // Each element's data follows a prologue `u32, ElementId, u64, class tag
    // u16, 00 00`. Partitions are read oldest first: a later copy replaces.
    let mut sites: BTreeMap<u64, Site> = BTreeMap::new();
    let mut location_recs: BTreeMap<u64, Location> = BTreeMap::new();
    // Internal position and `m_locationType` (1 survey point, 0 project base
    // point) of each base point.
    let mut points: BTreeMap<u64, ([f64; 3], Option<u32>)> = BTreeMap::new();
    let mut configs: BTreeMap<u64, Vec<ExportCrs>> = BTreeMap::new();
    let mut streams = rf.partition_stream_names();
    streams.sort_by_key(|s| crate::partition_names::partition_number(s));
    for stream in &streams {
        let Ok(inflated) = rf.inflated_partition(stream) else {
            continue;
        };
        let number = crate::partition_names::partition_number(stream);
        let buf = inflated.bytes();
        for at in memchr::memmem::find_iter(buf, &config_key) {
            let end = (at + 4096).min(buf.len());
            let units: Vec<u16> = buf[at.saturating_sub(4096)..end]
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            if let Some(code) = json_value(&units, "GeoRefEPSGCode").filter(|c| !c.is_empty()) {
                configs.entry(number).or_default().push((
                    code,
                    json_value(&units, "GeoRefCRSName"),
                    json_value(&units, "GeoRefCRSDesc"),
                    json_value(&units, "GeoRefMapUnit"),
                ));
            }
        }
        for (kind, tag) in kinds {
            let t = tag.to_le_bytes();
            for at in memchr::memmem::find_iter(buf, &[t[0], t[1], 0, 0]) {
                let Some(id) = at
                    .checked_sub(8 + w)
                    .and_then(|o| id_at(buf, o, wide))
                    .filter(|id| (1..1 << 40).contains(id))
                else {
                    continue;
                };
                let data = &buf[at..buf.len().min(at + (1 << 16))];
                match kind {
                    Kind::Site => {
                        if let Some(site) = read_site(data, wide) {
                            sites.insert(id, site);
                        }
                    }
                    Kind::Location => {
                        if let Some(l) = read_location(&data[..data.len().min(1024)], wide) {
                            location_recs.insert(id, l);
                        }
                    }
                    Kind::BasePoint => {
                        let head = &data[..data.len().min(512)];
                        let Some(g) = memchr::memmem::find(head, &gpoint_mark) else {
                            continue;
                        };
                        let Some(p) = memchr::memmem::find(&head[g..], &[4, 0, 8, 0]) else {
                            continue;
                        };
                        let p = g + p + 4;
                        if let (Some(x), Some(y), Some(z)) =
                            (f64_at(head, p), f64_at(head, p + 8), f64_at(head, p + 16))
                        {
                            if [x, y, z].iter().all(|v| v.is_finite() && v.abs() < 1e9) {
                                points.insert(id, ([x, y, z], location_type(head, g, own_gpoint)));
                            }
                        }
                    }
                }
            }
        }
    }
    let locations: BTreeMap<u64, Location> = location_recs
        .into_iter()
        .filter(|(_, l)| sites.contains_key(&l.site))
        .collect();
    if locations.is_empty() {
        return None;
    }

    // The active location: the `ActiveGeoLocationTrackingElement`'s data
    // ends with two location ids, the active one (`m_activeGeoLocationId`)
    // then the project's (`m_projectGeoLocationId`).
    let tracking = tag_of("ActiveGeoLocationTrackingElement")?.to_le_bytes();
    let mut active = None;
    for stream in &streams {
        let Ok(inflated) = rf.inflated_partition(stream) else {
            continue;
        };
        let buf = inflated.bytes();
        for at in memchr::memmem::find_iter(buf, &[tracking[0], tracking[1], 0, 0]) {
            let Some(own) = at
                .checked_sub(8 + w)
                .and_then(|o| id_at(buf, o, wide))
                .filter(|id| (1..1 << 40).contains(id))
            else {
                continue;
            };
            let data = &buf[at..buf.len().min(at + 256)];
            let mut own_header = vec![1, 0, 0, 0];
            own_header.extend(&own.to_le_bytes()[..w]);
            let Some(h) = memchr::memmem::find(data, &own_header) else {
                continue;
            };
            let found = (h + own_header.len()..data.len().saturating_sub(2 * w)).find_map(|p| {
                let a = id_at(data, p, wide)?;
                let b = id_at(data, p + w, wide)?;
                (locations.contains_key(&a) && locations.contains_key(&b)).then_some(a)
            });
            if found.is_some() {
                // Partitions are read oldest first: a later copy replaces.
                active = found;
            }
        }
    }
    let active = active.or_else(|| locations.keys().next().copied())?;
    let loc = &locations[&active];
    let site = &sites[&loc.site];

    // The survey point is the base point whose `m_locationType` is 1, the
    // project base point the one whose is 0; anything else leaves both
    // unknown (202/202 base points on 101 models against the API's
    // `IsShared`).

    let m = loc.m;
    let true_north = m[3].atan2(m[0]).to_degrees().rem_euclid(360.0);
    // shared = R^T (internal - o), feet.
    let to_shared_ft = |p: [f64; 3]| -> [f64; 3] {
        let d = [p[0] - m[9], p[1] - m[10], p[2] - m[11]];
        [
            m[0] * d[0] + m[3] * d[1] + m[6] * d[2],
            m[1] * d[0] + m[4] * d[1] + m[7] * d[2],
            m[2] * d[0] + m[5] * d[1] + m[8] * d[2],
        ]
    };
    let metres = |p: [f64; 3]| p.map(|v| v * FT);
    let point = |(element_id, p): (u64, [f64; 3])| GeoPoint {
        element_id,
        internal: metres(p),
        shared: metres(to_shared_ft(p)),
    };
    let (survey_point, project_base_point) = match split_base_points(&points) {
        Some((s, b)) => (Some(point(s)), Some(point(b))),
        None => (None, None),
    };

    let config_crs = |f: &ExportCrs| Crs {
        name: f.1.clone().unwrap_or_else(|| format!("EPSG:{}", f.0)),
        epsg: f.0.parse().ok(),
        description: f.2.clone(),
        map_unit: f.3.clone(),
        source: "IfcExportConfiguration".into(),
    };
    let crs = site
        .definition
        .as_deref()
        .map(epsg_of_definition)
        .filter(|(code, name, _, _)| code.is_some() || name.is_some())
        .map(|(epsg, name, description, map_unit)| Crs {
            name: name.unwrap_or_else(|| format!("EPSG:{}", epsg.unwrap_or(0))),
            epsg,
            description,
            map_unit,
            source: "SiteLocation".into(),
        })
        .or_else(|| {
            // The IFC export configurations of the newest partition that holds
            // any, when they all name one code: two that disagree there leave
            // the CRS unknown (2 Nordic Office models, 3879 and
            // 3857 saved side by side).
            let (_, found) = configs.iter().next_back()?;
            let first = found.first()?;
            found
                .iter()
                .all(|f| f.0 == first.0)
                .then(|| config_crs(first))
        });
    let mut crs_candidates: Vec<Crs> = Vec::new();
    if crs.is_none() {
        if let Some((_, found)) = configs.iter().next_back() {
            for f in found {
                if !crs_candidates.iter().any(|c| c.epsg == f.0.parse().ok()) {
                    crs_candidates.push(config_crs(f));
                }
            }
        }
        if crs_candidates.len() < 2 {
            crs_candidates.clear();
        }
    }

    let elements = SiteElements {
        locations: locations
            .iter()
            .map(|(&id, l)| (id, l.name.clone(), l.site))
            .collect(),
        sites: sites.keys().copied().collect(),
        base_points: points
            .iter()
            .map(|(&id, &(p, kind))| (id, to_shared_ft(p), kind))
            .collect(),
    };
    Some(Georeference {
        location_name: loc.name.clone(),
        latitude: site.lat.to_degrees(),
        longitude: site.lon.to_degrees(),
        elevation: site.elev * FT,
        time_zone: site.tz,
        true_north,
        origin_shared: metres(to_shared_ft([0.0; 3])),
        survey_point,
        project_base_point,
        crs,
        crs_candidates,
        elements,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_coordinates_round_trip_through_the_rotation() {
        let g = Georeference {
            location_name: "x".into(),
            latitude: 0.0,
            longitude: 0.0,
            elevation: 0.0,
            time_zone: 0.0,
            true_north: 334.0,
            origin_shared: [100.0, 50.0, 3.0],
            survey_point: None,
            project_base_point: None,
            crs: None,
            crs_candidates: Vec::new(),
            elements: SiteElements::default(),
        };
        let s = g.to_shared([0.0, 0.0, 0.0]);
        for (a, b) in s.iter().zip([100.0, 50.0, 3.0]) {
            assert!((a - b).abs() < 1e-9, "{s:?}");
        }
    }

    #[test]
    fn survey_point_is_the_location_type_flag_not_the_element_id_order() {
        // 2018/2019 layout: position, m_locationType, m_isClippedToShared, mark.
        let mut old = vec![0u8; 24];
        old.extend([1, 0, 0, 0, 1, 3, 0, 0, 0, 0x54, 0x07]);
        assert_eq!(location_type(&old, 29, true), Some(1));
        // 2022 on: mark, position, m_locationType, m_isClippedToShared.
        let mut new = vec![3, 0, 0, 0, 0xf0, 0x07];
        new.extend([0u8; 24]);
        new.extend([0, 0, 0, 0, 1]);
        assert_eq!(location_type(&new, 0, false), Some(0));
        new[30] = 7;
        assert_eq!(location_type(&new, 0, false), None);

        // A survey point with the higher ElementId is still the survey point.
        let mut pts = BTreeMap::new();
        pts.insert(10, ([1.0, 0.0, 0.0], Some(0)));
        pts.insert(20, ([2.0, 0.0, 0.0], Some(1)));
        let (s, b) = split_base_points(&pts).expect("one of each");
        assert_eq!((s.0, b.0), (20, 10));
        // Two survey points: unknown.
        pts.insert(30, ([3.0, 0.0, 0.0], Some(1)));
        assert!(split_base_points(&pts).is_none());
    }

    #[test]
    fn epsg_override_marks_the_user_and_keeps_a_feet_unit() {
        let mut g = Georeference {
            location_name: "x".into(),
            latitude: 0.0,
            longitude: 0.0,
            elevation: 0.0,
            time_zone: 0.0,
            true_north: 0.0,
            origin_shared: [0.0; 3],
            survey_point: None,
            project_base_point: None,
            crs: None,
            crs_candidates: Vec::new(),
            elements: SiteElements::default(),
        };
        assert_eq!(g.override_epsg(3879), None);
        let c = g.crs.clone().unwrap();
        assert_eq!(
            (c.epsg, c.source.as_str(), c.map_unit.as_deref()),
            (Some(3879), USER_OVERRIDE, Some("METRE"))
        );
        assert!(!c.in_feet());
        // A file CRS in feet keeps its unit; a different code is reported.
        g.crs = Some(Crs {
            name: "PA83-SF".into(),
            epsg: Some(2272),
            description: None,
            map_unit: Some("FOOT".into()),
            source: "SiteLocation".into(),
        });
        let w = g.override_epsg(6565).expect("contradiction");
        assert!(w.contains("6565") && w.contains("2272"), "{w}");
        assert!(g.crs.as_ref().unwrap().in_feet());
        // The same code: no warning.
        assert_eq!(g.override_epsg(6565), None);
    }

    #[test]
    fn epsg_comes_from_the_coordinate_system_alias() {
        let xml = r#"<Dictionary><ProjectedCoordinateSystem id="PA83-SF"><Name>PA83-SF</Name>
            <Description>NAD83 Pennsylvania South</Description><Axis uom="FOOT"></Axis></ProjectedCoordinateSystem>
            <Alias id="6269" type="Datum"><ObjectId>NAD83</ObjectId><Namespace>EPSG Code</Namespace></Alias>
            <Alias id="2272" type="CoordinateSystem"><ObjectId>PA83-SF</ObjectId><Namespace>EPSG Code</Namespace></Alias>
            </Dictionary>"#;
        let (code, name, description, unit) = epsg_of_definition(xml);
        assert_eq!(code, Some(2272));
        assert_eq!(name.as_deref(), Some("PA83-SF"));
        assert_eq!(description.as_deref(), Some("NAD83 Pennsylvania South"));
        assert_eq!(unit.as_deref(), Some("FOOT"));
    }
}
