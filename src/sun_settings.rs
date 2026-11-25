//! The file's sun settings and each view's graphic display options.
//!
//! Read through the file's own schema ([`crate::schema_objects`]):
//!
//! - **`SunAndShadowSettings`** (the API's `SunAndShadowSettings`): one element per named
//!   preset (solstices, equinoxes, solar studies, lighting presets) and one `<In-session, ...>`
//!   copy per view that has its own setting. Fields: `m_sName`, `m_dAzimuth` and `m_dAltitude`
//!   (radians, the Lighting mode's sun), `m_startDateTime` and `m_endDateTime` (the clock
//!   time Revit shows, packed into 32 bits, [`packed_date_time`]), `m_eSunStudyTimeInterval`, `m_type`, `m_activeFrame`, `m_idLevel` (the ground
//!   plane's level), `m_bGroundPlane`, `m_bRelativeToView`, `m_sunriseToSunset`,
//!   `m_sharedSetting`. Revit 2010 to 2012 name the dates `m_tDateAndTime` and `m_endDateAndTime`.
//! - **Each view's display** (`DBView::m_pViewDisplayMgr`, a `ViewDisplayMgr`, 2014 on):
//!   `m_lights` (`ViewDisplayLighting`: the view's `SunAndShadowSettings` id, sun, ambient and
//!   shadow intensity), `m_shadows` (`ViewDisplayShadows`: cast and ambient shadows),
//!   `m_model` (`ViewDisplayModel`: surfaces, edges, transparency, silhouettes),
//!   `m_sketchyLines` (`ViewDisplaySketchyLines`), `m_fog` (`ViewDisplayFog`: depth cueing),
//!   `m_background` (`ViewDisplayBackground`) and `m_exposure` (`ViewDisplayExposure`).
//!   Revit 2010 to 2013 keep sun and shadow intensity and the background flat on the manager.
//!
//! Nothing here comes from Revit binaries: the field names are the file's own schema, and the
//! values were checked against the public Revit API (`SunAndShadowSettings`, `View`) read from
//! files we may open.

use crate::RevitFile;
use crate::formats::SchemaLayouts;
use crate::schema_objects::{Fields, SchemaObject, Value, for_each_object_where};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One `SunAndShadowSettings` element.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SunSetting {
    pub id: u64,
    /// The name Revit shows: its own, or `<In-session, Lighting>` (and so on) for a view's own
    /// setting.
    pub name: String,
    /// The name as stored (a view's own setting keeps the preset name it was made from, or
    /// `<In-session: ...>`).
    pub stored_name: String,
    /// One of the project's named presets (`DBViewInfo::m_aidSunAndShadowSettings`).
    pub is_preset: bool,
    /// The API's `SunAndShadowType`: `StillImage`, `OneDayStudy`, `MultiDayStudy` or
    /// `Lighting`; `None` for a stored value with no known name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sun_and_shadow_type: Option<String>,
    /// The stored type value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_value: Option<i64>,
    /// Start and end (the API's `StartDateAndTime`, `EndDateAndTime`) as the clock time Revit
    /// shows, the project's local time: `YYYY-MM-DDTHH:MM`, no zone, to the minute.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<String>,
    /// The stored start and end values ([`packed_date_time`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_value: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_value: Option<i64>,
    /// The API's `TimeInterval` (`SunStudyTimeInterval`) name, and its stored value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_interval: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_interval_value: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_frame: Option<f64>,
    /// The Lighting mode's sun, degrees: azimuth from true north (or the view, see
    /// `relative_to_view`), altitude above the horizon.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub azimuth: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub altitude: Option<f64>,
    pub uses_ground_plane: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ground_plane_level_id: Option<u64>,
    pub relative_to_view: bool,
    pub sunrise_to_sunset: bool,
    pub shares_settings: bool,
}

/// Sketchy lines of a view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SketchyLines {
    pub enabled: bool,
    pub jitter: i64,
    pub extension: i64,
}

/// Depth cueing of a view (stored as `ViewDisplayFog`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DepthCueing {
    pub enabled: bool,
    /// Start and end, percent of the view depth; fade limit, percent.
    pub start: i64,
    pub end: i64,
    pub fade_to: i64,
}

/// A view's background.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Background {
    /// `None`, `Sky`, `Gradient` or `Image` (the stored value when unknown, as `Type<n>`).
    pub kind: String,
    pub type_value: i64,
    /// `#rrggbb` colours: sky, horizon (the gradient's middle) and ground.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sky_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub horizon_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ground_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_path: Option<String>,
}

/// A view's realistic exposure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Exposure {
    /// `Automatic` or `Manual` (the stored value when unknown, as `Type<n>`).
    pub kind: String,
    pub type_value: i64,
    /// The manual exposure value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
}

/// The graphic display options of one view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewDisplay {
    pub id: u64,
    pub name: String,
    /// The view's schema class (`DBView3d`, `DBViewPlan`, `DBViewSection`, ...).
    pub class: String,
    pub is_3d: bool,
    pub is_template: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view_template_id: Option<u64>,
    /// The view's own `SunAndShadowSettings` (the API's `View.SunAndShadowSettings`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sun_setting_id: Option<u64>,
    /// The API's `DisplayStyle` name ([`display_style`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_style: Option<String>,
    /// The stored visual-style values: the manager's hidden-line and lighting flags and the
    /// model display's surfaces and edges.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style_values: Option<[i64; 4]>,
    pub cast_shadows: bool,
    pub ambient_shadows: bool,
    /// 0 to 100.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shadow_intensity: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sunlight_intensity: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ambient_intensity: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sketchy_lines: Option<SketchyLines>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depth_cueing: Option<DepthCueing>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<Background>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exposure: Option<Exposure>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lighting_scheme_id: Option<u64>,
}

/// The sun settings and view display options of a model.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SunAndDisplay {
    /// Every `SunAndShadowSettings` element, by id.
    pub settings: Vec<SunSetting>,
    /// The project's named presets (`DBViewInfo::m_aidSunAndShadowSettings`), in stored order.
    pub presets: Vec<u64>,
    /// Every project view with display options, by id.
    pub views: Vec<ViewDisplay>,
    /// The 3D view the viewer starts from: `{3D}` when the project has it, else the first
    /// 3D view that is not a template.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_view: Option<u64>,
}

impl SunAndDisplay {
    pub fn setting(&self, id: u64) -> Option<&SunSetting> {
        self.settings.iter().find(|s| s.id == id)
    }
    pub fn view(&self, id: u64) -> Option<&ViewDisplay> {
        self.views.iter().find(|v| v.id == id)
    }
    /// The sun a viewer should start from, with where it came from: the default 3D view's own
    /// setting (`"view"`), else the project's first named preset (`"project"`). The view is the
    /// default view (whose options still apply with a preset), `None` without one.
    pub fn viewer_default(&self) -> Option<(&'static str, Option<&ViewDisplay>, &SunSetting)> {
        let default = self.default_view.and_then(|i| self.view(i));
        if let Some((v, s)) = default.and_then(|v| Some((v, self.setting(v.sun_setting_id?)?))) {
            return Some(("view", Some(v), s));
        }
        let s = self.presets.iter().find_map(|i| self.setting(*i))?;
        Some(("project", default, s))
    }
}

/// The API's `SunAndShadowType` names, by stored value.
pub fn sun_and_shadow_type(v: i64) -> Option<&'static str> {
    Some(match v {
        0 => "StillImage",
        1 => "OneDayStudy",
        2 => "MultiDayStudy",
        3 => "Lighting",
        _ => return None,
    })
}

/// The API's `SunStudyTimeInterval` names, by stored value (the API reference gives each
/// member's value; a negative one is stored as its 32-bit two's complement).
pub fn time_interval(v: i64) -> Option<&'static str> {
    let v = if v > i64::from(i32::MAX) {
        v - (1 << 32)
    } else {
        v
    };
    Some(match v {
        -4 => "Seconds15",
        -3 => "Seconds30",
        -2 => "Seconds45",
        -1 => "Minute",
        0 => "Minutes15",
        1 => "Minutes30",
        2 => "Minutes45",
        3 => "Hour",
        4 => "Day",
        5 => "Week",
        6 => "Month",
        _ => return None,
    })
}

/// The API's `DisplayStyle` names, by value (the API reference gives each member's value). A
/// view's model display stores its visual style as this value (`ViewDisplayModel::m_surfaces`;
/// `ViewDisplayMgr::m_viewDisplayRenderType` before 2014).
pub fn display_style(v: i64) -> Option<&'static str> {
    Some(match v {
        1 => "Wireframe",
        2 => "HLR",
        3 => "Shading",
        4 => "ShadingWithEdges",
        5 => "Rendering",
        6 => "Realistic",
        7 => "FlatColors",
        8 => "RealisticWithEdges",
        9 => "Textures",
        _ => return None,
    })
}

fn get<'a>(f: &'a Fields, name: &str) -> Option<&'a Value> {
    f.get(name)
}
fn int(f: &Fields, name: &str) -> Option<i64> {
    match get(f, name)? {
        Value::Int(i) => Some(*i),
        Value::Bool(b) => Some(*b as i64),
        _ => None,
    }
}
fn float(f: &Fields, name: &str) -> Option<f64> {
    match get(f, name)? {
        Value::Float(v) if v.is_finite() => Some(*v),
        _ => None,
    }
}
fn flag(f: &Fields, name: &str) -> bool {
    matches!(get(f, name), Some(Value::Bool(true)) | Some(Value::Int(1)))
}
fn id(f: &Fields, name: &str) -> Option<u64> {
    match get(f, name)? {
        Value::Id(i) if *i >= 0 => Some(*i as u64),
        _ => None,
    }
}
fn text(f: &Fields, name: &str) -> Option<String> {
    match get(f, name)? {
        Value::Text(s) => Some(s.clone()),
        _ => None,
    }
}
/// The fields of a by-value object or of the object a pointer introduced.
fn sub<'a>(o: &'a SchemaObject, f: &'a Fields, name: &str) -> Option<&'a Fields> {
    match get(f, name)? {
        Value::Object(x) => Some(x),
        Value::New { index, .. } => o.objects.get(*index),
        _ => None,
    }
}
/// A stored colour (`0x00bbggrr`) as `#rrggbb`.
fn colour(f: &Fields, name: &str) -> Option<String> {
    let c = int(f, name)? as u32;
    Some(format!(
        "#{:02x}{:02x}{:02x}",
        c & 0xff,
        (c >> 8) & 0xff,
        (c >> 16) & 0xff
    ))
}

/// A stored date and time, `YYYY-MM-DDTHH:MM`: one 32-bit value packing the minute (bits 0-5),
/// hour (6-10), day (11-15), month (16-19) and year (20-31). `None` when a part is out of range.
pub fn packed_date_time(v: i64) -> Option<String> {
    let v = u32::try_from(v).ok()?;
    let (minute, hour, day, month, year) = (
        v & 63,
        (v >> 6) & 31,
        (v >> 11) & 31,
        (v >> 16) & 15,
        v >> 20,
    );
    (minute < 60 && hour < 24 && (1..=31).contains(&day) && (1..=12).contains(&month))
        .then(|| format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}"))
}

/// The label Revit gives a view's own setting of each `SunAndShadowType`.
fn in_session_label(t: Option<&str>) -> &'static str {
    match t {
        Some("StillImage") => "Still",
        Some("OneDayStudy") => "Single Day",
        Some("MultiDayStudy") => "Multi-Day",
        _ => "Lighting",
    }
}

fn sun_setting(id_: u64, f: &Fields) -> SunSetting {
    let start = int(f, "m_startDateTime").or_else(|| int(f, "m_tDateAndTime"));
    let end = int(f, "m_endDateTime").or_else(|| int(f, "m_endDateAndTime"));
    let ty = int(f, "m_type");
    let iv = int(f, "m_eSunStudyTimeInterval").or_else(|| int(f, "m_sunStudyTimeInterval"));
    SunSetting {
        id: id_,
        name: String::new(),
        stored_name: text(f, "m_sName").unwrap_or_default(),
        is_preset: false,
        sun_and_shadow_type: ty.and_then(sun_and_shadow_type).map(str::to_string),
        type_value: ty,
        start: start.and_then(packed_date_time),
        end: end.and_then(packed_date_time),
        start_value: start,
        end_value: end,
        time_interval: iv.and_then(time_interval).map(str::to_string),
        time_interval_value: iv,
        active_frame: float(f, "m_activeFrame"),
        azimuth: float(f, "m_dAzimuth").map(f64::to_degrees),
        altitude: float(f, "m_dAltitude").map(f64::to_degrees),
        uses_ground_plane: flag(f, "m_bGroundPlane"),
        ground_plane_level_id: id(f, "m_idLevel"),
        relative_to_view: flag(f, "m_bRelativeToView"),
        sunrise_to_sunset: flag(f, "m_sunriseToSunset"),
        shares_settings: flag(f, "m_sharedSetting"),
    }
}

fn kind_name(names: &[&'static str], v: i64) -> String {
    usize::try_from(v)
        .ok()
        .and_then(|i| names.get(i))
        .map_or_else(|| format!("Type{v}"), |s| (*s).to_string())
}

fn view_display(
    o: &SchemaObject,
    id_: u64,
    class: &str,
    root: &Fields,
    mgr: Option<&Fields>,
) -> ViewDisplay {
    let mut v = ViewDisplay {
        id: id_,
        name: text(root, "m_viewName").unwrap_or_default(),
        class: class.to_string(),
        is_3d: class.starts_with("DBView3d"),
        is_template: flag(root, "m_isTemplate"),
        view_template_id: id(root, "m_viewTemplateId"),
        sun_setting_id: None,
        display_style: None,
        style_values: None,
        cast_shadows: false,
        ambient_shadows: false,
        shadow_intensity: None,
        sunlight_intensity: None,
        ambient_intensity: None,
        sketchy_lines: None,
        depth_cueing: None,
        background: None,
        exposure: None,
        lighting_scheme_id: id(root, "m_lightSchemeId"),
    };
    let Some(m) = mgr else { return v };
    let model = sub(o, m, "m_model");
    let values = [
        i64::from(flag(m, "m_hlr")),
        i64::from(flag(m, "m_lighting")),
        model.and_then(|x| int(x, "m_surfaces")).unwrap_or(-1),
        model.and_then(|x| int(x, "m_edges")).unwrap_or(-1),
    ];
    v.style_values = Some(values);
    let style = model
        .and_then(|x| int(x, "m_surfaces"))
        .or_else(|| int(m, "m_viewDisplayRenderType"));
    v.display_style = style.and_then(display_style).map(str::to_string);
    if let Some(l) = sub(o, m, "m_lights") {
        v.sun_setting_id = id(l, "m_sunAndShadowSettingsId");
        v.sunlight_intensity = int(l, "m_sunlightIntensity");
        v.shadow_intensity = int(l, "m_shadowIntensity");
        v.ambient_intensity = int(l, "m_ambientLightIntensity");
    } else {
        // 2010 to 2013: flat on the manager
        v.sun_setting_id = id(m, "m_idShadowsSettings");
        v.sunlight_intensity = int(m, "m_nSunIntensity");
        v.shadow_intensity = int(m, "m_nShadowIntensity");
        v.ambient_intensity = int(m, "m_nAmbientIntensity");
        v.cast_shadows = flag(m, "m_bCastShadows");
        v.ambient_shadows = flag(m, "m_bUseAmbientLighting");
        if let Some(t) = int(m, "m_backgroundType") {
            v.background = Some(Background {
                kind: kind_name(&BACKGROUND_KINDS, t),
                type_value: t,
                sky_color: colour(m, "m_skyColor"),
                horizon_color: colour(m, "m_backgroundColor"),
                ground_color: colour(m, "m_groundColor"),
                image_path: None,
            });
        }
    }
    if let Some(s) = sub(o, m, "m_shadows") {
        v.cast_shadows = flag(s, "m_castShadows");
        v.ambient_shadows = flag(s, "m_ambientShadows");
    }
    if let Some(s) = sub(o, m, "m_sketchyLines") {
        v.sketchy_lines = Some(SketchyLines {
            enabled: flag(s, "m_enableSketchyLines"),
            jitter: int(s, "m_jitter").unwrap_or(0),
            extension: int(s, "m_extension").unwrap_or(0),
        });
    }
    if let Some(s) = sub(o, m, "m_fog") {
        v.depth_cueing = Some(DepthCueing {
            enabled: flag(s, "m_enable"),
            start: int(s, "m_start").unwrap_or(0),
            end: int(s, "m_end").unwrap_or(0),
            fade_to: int(s, "m_density").unwrap_or(0),
        });
    }
    if let Some(s) = sub(o, m, "m_background") {
        let t = int(s, "m_type").unwrap_or(0);
        let path = text(s, "m_imagePath").filter(|p| !p.is_empty());
        v.background = Some(Background {
            kind: kind_name(&BACKGROUND_KINDS, t),
            type_value: t,
            sky_color: colour(s, "m_skyColor"),
            horizon_color: colour(s, "m_backgroundColor"),
            ground_color: colour(s, "m_groundColor"),
            image_path: path,
        });
    }
    if let Some(s) = sub(o, m, "m_exposure") {
        let t = int(s, "m_type").unwrap_or(0);
        v.exposure = Some(Exposure {
            kind: kind_name(&EXPOSURE_KINDS, t),
            type_value: t,
            value: float(s, "m_exposureDouble").or_else(|| int(s, "m_exposure").map(|x| x as f64)),
        });
    }
    v
}

/// `ViewDisplayBackground::m_type` names, by stored value.
pub const BACKGROUND_KINDS: [&str; 4] = ["None", "Sky", "Gradient", "Image"];
/// `ViewDisplayExposure::m_type` names, by stored value.
pub const EXPOSURE_KINDS: [&str; 2] = ["Automatic", "Manual"];

/// The tag of every class that is, or derives from, one of `names`, with its own name.
fn tags(layouts: &SchemaLayouts, names: &[&str]) -> BTreeMap<u16, String> {
    let mut out = BTreeMap::new();
    for tag in layouts.classes.keys() {
        let Some(chain) = layouts.chain(*tag) else {
            continue;
        };
        let picked = chain.iter().any(|c| names.contains(&c.name.as_str()));
        match chain.last() {
            Some(leaf) if picked => {
                out.insert(*tag, leaf.name.clone());
            }
            _ => {}
        }
    }
    out
}

/// Read the model's sun settings and view display options. `None` when the file holds no
/// `SunAndShadowSettings` and no view display options (a family, or an unreadable schema).
pub fn read(rf: &mut RevitFile) -> Option<SunAndDisplay> {
    let layouts = rf.schema_layouts().ok()?;
    let wanted = tags(&layouts, &["SunAndShadowSettings", "DBView", "DBViewInfo"]);
    if wanted.is_empty() {
        return None;
    }
    // Partition sections after the first are loaded families' own documents.
    let mut first_section: BTreeMap<String, usize> = BTreeMap::new();
    for name in rf.partition_stream_names() {
        let at = rf
            .inflated_partition(&name)
            .ok()
            .and_then(|p| p.sections().first().map(|(at, _)| *at));
        if let Some(at) = at {
            first_section.insert(name, at);
        }
    }
    let mut settings: BTreeMap<u64, SunSetting> = BTreeMap::new();
    let mut views: BTreeMap<u64, ViewDisplay> = BTreeMap::new();
    let mut presets = Vec::new();
    let _ = for_each_object_where(
        rf,
        |f, _| wanted.contains_key(&f.header.tag),
        |o| {
            let Some(root) = o.root() else { return };
            if first_section
                .get(&o.stream)
                .is_some_and(|at| *at <= o.frame.span.start)
            {
                return;
            }
            let class = wanted[&o.frame.header.tag].as_str();
            let oid = o.frame.header.id;
            match class {
                "SunAndShadowSettings" => {
                    // the newest partition copy wins: later objects replace earlier ones
                    settings.insert(oid, sun_setting(oid, root));
                }
                "DBViewInfo" => {
                    if let Some(Value::List(l)) = root.get("m_aidSunAndShadowSettings") {
                        presets = l
                            .iter()
                            .filter_map(|v| match v {
                                Value::Id(i) if *i >= 0 => Some(*i as u64),
                                _ => None,
                            })
                            .collect();
                    }
                }
                _ => {
                    let mgr = sub(o, root, "m_pViewDisplayMgr");
                    if mgr.is_none() && !class.starts_with("DBView3d") {
                        return;
                    }
                    views.insert(oid, view_display(o, oid, class, root, mgr));
                }
            }
        },
    );
    if settings.is_empty() && views.is_empty() {
        return None;
    }
    // A view's own setting that is not a named preset shows as `<In-session, ...>`, whatever
    // name it keeps.
    let owned: std::collections::BTreeSet<u64> =
        views.values().filter_map(|v| v.sun_setting_id).collect();
    for s in settings.values_mut() {
        s.is_preset = presets.contains(&s.id);
        let in_session =
            s.stored_name.starts_with("<In-session") || (owned.contains(&s.id) && !s.is_preset);
        s.name = if !in_session {
            s.stored_name.clone()
        } else {
            format!(
                "<In-session, {}>",
                in_session_label(s.sun_and_shadow_type.as_deref())
            )
        };
    }
    let mut out = SunAndDisplay {
        settings: settings.into_values().collect(),
        presets,
        views: views.into_values().collect(),
        default_view: None,
    };
    out.default_view = out
        .views
        .iter()
        .find(|v| v.is_3d && !v.is_template && v.name == "{3D}")
        .or_else(|| out.views.iter().find(|v| v.is_3d && !v.is_template))
        .map(|v| v.id);
    Some(out)
}
