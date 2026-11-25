//! The scene and sensor grids from a glTF binary in rvt-open's layout: element
//! nodes under a root that turns the model's Z-up feet into glTF's Y-up metres,
//! node extras with `category`, `builtInCategory`, `elementId` and
//! `entityIndex`, rooms under a node named `rooms` (in the second scene).
//!
//! - Each glTF material becomes a modifier: alpha below 0.99 is glass with pane
//!   transmittance 1 - alpha (glazing is a closed thin solid, so a ray crosses
//!   two faces and each gets the square root), otherwise plastic with the linear
//!   base colour as reflectance, kept in 0.02 to 0.85. A material whose
//!   `extras.colourSource` is `rvt-open` (a colour the reader chose, not the
//!   file's) is marked assumed.
//! - Sensors: a grid over each room's floor (all of a node's primitives are one zone) (the faces of its bottom) at
//!   `offset` above it; where the model has no rooms, over the top faces of each
//!   floor, kept `inset` metres in from the floor's edge.
//! - Coordinates go to the daylight frame (metres, Z up, the model's own north)
//!   and are rounded as the Radiance scene text prints them (5 decimals; sensors 4).

use crate::radiance::{round, round_trip};
use crate::{Model, Modifier, Sensor, Surface, Triangle, V, grid, transmissivity};
use serde_json::Value;

/// How the grids are laid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridOptions {
    /// Grid spacing, metres.
    pub spacing: f64,
    /// Height of the sensors above the floor, metres.
    pub offset: f64,
    /// Distance kept from a floor's edge when the grid falls back to floors, metres.
    pub inset: f64,
}

impl Default for GridOptions {
    fn default() -> Self {
        GridOptions {
            spacing: 0.5,
            offset: 0.76,
            inset: 0.35,
        }
    }
}

/// A set of sensors: a room, or a floor when the model has no rooms.
#[derive(Debug, Clone, PartialEq)]
pub struct Zone {
    /// `room` or `floor`.
    pub kind: &'static str,
    /// The glTF node's name.
    pub name: String,
    /// The node's `entityIndex` (rvt-open's model entity), when it has one.
    pub entity_index: Option<u64>,
    /// The node's Revit ElementId, when it has one.
    pub element_id: Option<i64>,
    /// The floor's height, metres (daylight frame).
    pub floor_z: f64,
}

/// What [`read`] returns.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Prepared {
    pub model: Model,
    pub sensors: Vec<Sensor>,
    /// The zone of each sensor, an index into `zones`.
    pub sensor_zone: Vec<u32>,
    pub zones: Vec<Zone>,
    /// Triangles per category name.
    pub by_category: Vec<(String, usize)>,
}

type M4 = [[f64; 4]; 4];
/// A primitive's glTF material and its triangles.
type Prim = (Option<u64>, Vec<[V; 3]>);
const I4: M4 = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

fn mul(a: &M4, b: &M4) -> M4 {
    let mut m = [[0.0; 4]; 4];
    for (r, row) in m.iter_mut().enumerate() {
        for (c, x) in row.iter_mut().enumerate() {
            *x = a[r][0] * b[0][c] + a[r][1] * b[1][c] + a[r][2] * b[2][c] + a[r][3] * b[3][c];
        }
    }
    m
}

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap_or(0.0)
}

fn node_matrix(nd: &Value) -> M4 {
    if let Some(a) = nd
        .get("matrix")
        .and_then(Value::as_array)
        .filter(|a| a.len() == 16)
    {
        let mut m = [[0.0; 4]; 4];
        for (r, row) in m.iter_mut().enumerate() {
            for (c, x) in row.iter_mut().enumerate() {
                *x = f(&a[c * 4 + r]);
            }
        }
        return m;
    }
    let mut m = I4;
    if let Some(s) = nd
        .get("scale")
        .and_then(Value::as_array)
        .filter(|a| a.len() == 3)
    {
        let mut d = I4;
        (0..3).for_each(|k| d[k][k] = f(&s[k]));
        m = mul(&d, &m);
    }
    if let Some(q) = nd
        .get("rotation")
        .and_then(Value::as_array)
        .filter(|a| a.len() == 4)
    {
        let (x, y, z, w) = (f(&q[0]), f(&q[1]), f(&q[2]), f(&q[3]));
        let r = [
            [
                1.0 - 2.0 * (y * y + z * z),
                2.0 * (x * y - z * w),
                2.0 * (x * z + y * w),
                0.0,
            ],
            [
                2.0 * (x * y + z * w),
                1.0 - 2.0 * (x * x + z * z),
                2.0 * (y * z - x * w),
                0.0,
            ],
            [
                2.0 * (x * z - y * w),
                2.0 * (y * z + x * w),
                1.0 - 2.0 * (x * x + y * y),
                0.0,
            ],
            [0.0, 0.0, 0.0, 1.0],
        ];
        m = mul(&r, &m);
    }
    if let Some(t) = nd
        .get("translation")
        .and_then(Value::as_array)
        .filter(|a| a.len() == 3)
    {
        let mut tm = I4;
        (0..3).for_each(|k| tm[k][3] = f(&t[k]));
        m = mul(&tm, &m);
    }
    m
}

struct Glb<'a> {
    j: Value,
    bin: &'a [u8],
}

impl Glb<'_> {
    fn accessor(&self, i: usize) -> Result<(Vec<f64>, usize), String> {
        let a = &self.j["accessors"][i];
        let bv = &self.j["bufferViews"][a["bufferView"]
            .as_u64()
            .ok_or("accessor without a buffer view")?
            as usize];
        let nc = match a["type"].as_str() {
            Some("SCALAR") => 1,
            Some("VEC2") => 2,
            Some("VEC3") => 3,
            Some("VEC4") => 4,
            t => return Err(format!("accessor type {t:?}")),
        };
        let (size, ct) = match a["componentType"].as_u64() {
            Some(5126) => (4, 'f'),
            Some(5125) => (4, 'I'),
            Some(5123) => (2, 'H'),
            Some(5121) => (1, 'B'),
            c => return Err(format!("component type {c:?}")),
        };
        let count = a["count"].as_u64().unwrap_or(0) as usize;
        let start = bv["byteOffset"].as_u64().unwrap_or(0) as usize
            + a["byteOffset"].as_u64().unwrap_or(0) as usize;
        let stride = bv["byteStride"].as_u64().map_or(nc * size, |s| s as usize);
        let mut out = Vec::with_capacity(count * nc);
        for e in 0..count {
            for k in 0..nc {
                let o = start + e * stride + k * size;
                let b = self
                    .bin
                    .get(o..o + size)
                    .ok_or("accessor past the binary chunk")?;
                out.push(match ct {
                    'f' => f64::from(f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
                    'I' => f64::from(u32::from_le_bytes([b[0], b[1], b[2], b[3]])),
                    'H' => f64::from(u16::from_le_bytes([b[0], b[1]])),
                    _ => f64::from(b[0]),
                });
            }
        }
        Ok((out, nc))
    }

    /// Triangles of mesh `mesh` placed by `m`, in the daylight frame, with their glTF material.
    fn triangles(&self, mesh: usize, m: &M4) -> Result<Vec<Prim>, String> {
        let mut out = Vec::new();
        for p in self.j["meshes"][mesh]["primitives"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if p["mode"].as_u64().unwrap_or(4) != 4 {
                continue; // edges (LINES) are not surfaces
            }
            let (v, _) = self.accessor(
                p["attributes"]["POSITION"]
                    .as_u64()
                    .ok_or("primitive without POSITION")? as usize,
            )?;
            let n = v.len() / 3;
            let idx: Vec<usize> = match p["indices"].as_u64() {
                Some(i) => self
                    .accessor(i as usize)?
                    .0
                    .into_iter()
                    .map(|x| x as usize)
                    .collect(),
                None => (0..n).collect(),
            };
            let place = |i: usize| -> Result<V, String> {
                let q = v.get(3 * i..3 * i + 3).ok_or("index past the positions")?;
                let w = |r: usize| m[r][0] * q[0] + m[r][1] * q[1] + m[r][2] * q[2] + m[r][3];
                // glTF is Y up; the daylight frame is Z up with +Y the model's north (glTF -Z)
                Ok([w(0), -w(2), w(1)])
            };
            let mut tris = Vec::with_capacity(idx.len() / 3);
            for c in idx.chunks_exact(3) {
                tris.push([place(c[0])?, place(c[1])?, place(c[2])?]);
            }
            out.push((p["material"].as_u64(), tris));
        }
        Ok(out)
    }
}

fn modifiers(j: &Value) -> Vec<Modifier> {
    let mut out = Vec::new();
    for (i, m) in j["materials"].as_array().into_iter().flatten().enumerate() {
        let c: Vec<f64> = m["pbrMetallicRoughness"]["baseColorFactor"]
            .as_array()
            .filter(|a| a.len() == 4)
            .map_or(vec![0.5, 0.5, 0.5, 1.0], |a| a.iter().map(f).collect());
        let label = m["name"]
            .as_str()
            .map_or_else(|| format!("m{i}"), str::to_string);
        let assumed = m["extras"]["colourSource"].as_str() == Some("rvt-open");
        let surface = if c[3] < 0.99 {
            let t = transmissivity((1.0 - c[3]).sqrt()).min(1.0);
            Surface::Glass { tn: [t; 3] }
        } else {
            Surface::Plastic {
                rgb: [
                    c[0].clamp(0.02, 0.85),
                    c[1].clamp(0.02, 0.85),
                    c[2].clamp(0.02, 0.85),
                ],
                spec: 0.0,
                rough: 0.0,
            }
        };
        out.push(Modifier {
            id: format!("m{i}"),
            surface,
            label,
            assumed,
        });
    }
    out.push(Modifier {
        id: "mdefault".into(),
        surface: Surface::Plastic {
            rgb: [0.5; 3],
            spec: 0.0,
            rough: 0.0,
        },
        label: "no material".into(),
        assumed: true,
    });
    out
}

struct Collect {
    model: Model,
    rooms: Vec<(Zone, Vec<[V; 3]>)>,
    floors: Vec<(Zone, Vec<[V; 3]>)>,
    by_category: Vec<(String, usize)>,
}

fn zone(nd: &Value, kind: &'static str) -> Zone {
    Zone {
        kind,
        name: nd["name"].as_str().unwrap_or_default().to_string(),
        entity_index: nd["extras"]["entityIndex"].as_u64(),
        element_id: nd["extras"]["elementId"].as_i64(),
        floor_z: 0.0,
    }
}

fn walk(
    g: &Glb,
    roots: &[usize],
    parent: &M4,
    rooms_tag: bool,
    out: &mut Collect,
) -> Result<(), String> {
    for &i in roots {
        let nd = &g.j["nodes"][i];
        let m = mul(parent, &node_matrix(nd));
        let tag = rooms_tag || nd["name"].as_str() == Some("rooms");
        if nd
            .get("extensions")
            .and_then(|e| e.get("EXT_mesh_gpu_instancing"))
            .is_some()
        {
            return Err("instanced glTF nodes are not read: export with instancing off".into());
        }
        if let Some(mesh) = nd["mesh"].as_u64() {
            let ex = &nd["extras"];
            let bic = ex["builtInCategory"].as_str();
            let room = tag || matches!(bic, Some("OST_Rooms" | "OST_MEPSpaces"));
            let dflt = (out.model.modifiers.len() - 1) as u32;
            // a room's or a floor's faces, from all its primitives: one zone per element
            let mut sensing: Vec<[V; 3]> = Vec::new();
            for (mat, tris) in g.triangles(mesh as usize, &m)? {
                if room {
                    sensing.extend(tris);
                    continue;
                }
                let cat = ex["category"].as_str().unwrap_or("?").to_string();
                match out.by_category.iter_mut().find(|(c, _)| *c == cat) {
                    Some((_, n)) => *n += tris.len(),
                    None => out.by_category.push((cat, tris.len())),
                }
                if bic == Some("OST_Floors") {
                    sensing.extend_from_slice(&tris);
                }
                let modifier = mat.map_or(dflt, |k| (k as u32).min(dflt));
                for v in tris {
                    let c = crate::cross(crate::sub(v[1], v[0]), crate::sub(v[2], v[0]));
                    if crate::dot(c, c).sqrt() >= 1e-10 {
                        out.model.triangles.push(Triangle { v, modifier });
                    }
                }
            }
            if !sensing.is_empty() {
                if room {
                    out.rooms.push((zone(nd, "room"), sensing));
                } else {
                    out.floors.push((zone(nd, "floor"), sensing));
                }
            }
        }
        let kids: Vec<usize> = nd["children"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|c| c.as_u64().map(|c| c as usize))
            .collect();
        walk(g, &kids, &m, tag, out)?;
    }
    Ok(())
}

fn indices(v: &Value) -> Vec<usize> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| c.as_u64().map(|c| c as usize))
        .collect()
}

/// Reads the scene and lays the sensor grids.
pub fn read(glb: &[u8], opts: &GridOptions) -> Result<Prepared, String> {
    let u32_at = |o: usize| {
        glb.get(o..o + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
    };
    if glb.get(0..4) != Some(b"glTF") {
        return Err("not a glTF binary".into());
    }
    let jl = u32_at(12).ok_or("truncated glTF binary")?;
    let j: Value = serde_json::from_slice(glb.get(20..20 + jl).ok_or("truncated JSON chunk")?)
        .map_err(|e| e.to_string())?;
    let bo = 20 + jl;
    let bin = match u32_at(bo) {
        Some(bl) => glb
            .get(bo + 8..bo + 8 + bl)
            .ok_or("truncated binary chunk")?,
        None => &[],
    };
    let g = Glb { j, bin };
    let mut c = Collect {
        model: Model {
            modifiers: modifiers(&g.j),
            triangles: Vec::new(),
        },
        rooms: Vec::new(),
        floors: Vec::new(),
        by_category: Vec::new(),
    };
    let scenes = g.j["scenes"].as_array().cloned().unwrap_or_default();
    let default = g.j["scene"].as_u64().unwrap_or(0) as usize;
    let roots = scenes
        .get(default)
        .map(|s| indices(&s["nodes"]))
        .unwrap_or_default();
    walk(&g, &roots, &I4, false, &mut c)?;
    // rooms live only in the glTF's second scene
    for s in &scenes {
        let extra: Vec<usize> = indices(&s["nodes"])
            .into_iter()
            .filter(|n| !roots.contains(n) && g.j["nodes"][*n]["name"].as_str() == Some("rooms"))
            .collect();
        walk(&g, &extra, &I4, false, &mut c)?;
    }
    round_trip(&mut c.model);

    let mut p = Prepared {
        model: c.model,
        by_category: c.by_category,
        ..Prepared::default()
    };
    for (z, faces) in c.rooms {
        lay(&mut p, z, faces, false, 0.0, opts);
    }
    if p.sensors.is_empty() {
        for (z, faces) in c.floors {
            lay(&mut p, z, faces, true, opts.inset, opts);
        }
    }
    Ok(p)
}

/// Lays one zone's grid over its bottom faces (a room) or its top faces (a floor).
fn lay(
    p: &mut Prepared,
    mut z: Zone,
    faces: Vec<[V; 3]>,
    top: bool,
    inset: f64,
    opts: &GridOptions,
) {
    let sel: Vec<[V; 3]> = faces
        .into_iter()
        .filter(|t| {
            if top {
                grid::normal_z(t) > 0.9
            } else {
                grid::normal_z(t) < -0.9
            }
        })
        .collect();
    let zs = sel.iter().flatten().map(|v| v[2]);
    let z0 = if top {
        zs.fold(f64::MIN, f64::max)
    } else {
        zs.fold(f64::MAX, f64::min)
    };
    let level: Vec<[V; 3]> = sel
        .into_iter()
        .filter(|t| ((t[0][2] + t[1][2] + t[2][2]) / 3.0 - z0).abs() < 0.05)
        .collect();
    let pts = grid::over(&level, z0 + opts.offset, opts.spacing, inset);
    if pts.is_empty() {
        return;
    }
    z.floor_z = z0;
    let k = p.zones.len() as u32;
    p.zones.push(z);
    for q in pts {
        p.sensors.push(Sensor {
            p: [round(q[0], 4), round(q[1], 4), round(q[2], 4)],
            n: [0.0, 0.0, 1.0],
        });
        p.sensor_zone.push(k);
    }
}
