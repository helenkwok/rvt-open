//! U3D writer: a scene of triangle meshes as a Universal 3D file (ECMA-363,
//! 4th edition, June 2007), the 3D stream format of a PDF `/3D` annotation.
//!
//! Written from the standard's text alone:
//!
//! - every block goes through the bit encoder of clause 10 ([`BitWriter`]):
//!   uncompressed values as bit-reversed bytes in the static context of range
//!   256, compressed values in their named context, static (range `R`, given
//!   as `0x400 + R`, written uncompressed above `R = 0x3FFE`) or dynamic
//!   (ids 1 to `0x3FF`), and a U32 zero flushing a block that holds a
//!   compressed value;
//! - a mesh is one CLOD Mesh Declaration (`0xFFFFFF31`) in a model resource
//!   modifier chain (`0xFFFFFF14`, type 1) and one CLOD Base Mesh Continuation
//!   (`0xFFFFFF3B`): the minimum resolution equals the final maximum, so the
//!   base mesh is the whole mesh, positions and normals are plain F32 (the base
//!   mesh is not quantized), and there is no progressive block;
//! - a node is a node modifier chain (type 0) holding a Model Node
//!   (`0xFFFFFF22`, child of the default world group node, its placement as
//!   the parent transform) and a Shading Modifier (`0xFFFFFF45`) naming one
//!   Lit Texture Shader (`0xFFFFFF53`) per shading index of its mesh; each
//!   shader names a Material Resource (`0xFFFFFF54`). Several nodes may name
//!   one mesh resource, so instancing is kept;
//! - a node's metadata (clause 9.2.6) carries its key/value strings, which a
//!   viewer shows beside its model tree.
//!
//! The version written is 0.0 (clause 9.4.1.1.1 asks for a negative major
//! version from an encoder whose compliance has not been validated; 0 is
//! written because a reader that takes the field as unsigned would read -1
//! as a newer version).

/// One triangle mesh: positions, triangles, and for each triangle its
/// shading index, which [`Mesh::shaders`] maps to a material.
#[derive(Debug, Clone, Default)]
pub struct Mesh {
    pub positions: Vec<[f32; 3]>,
    pub triangles: Vec<[u32; 3]>,
    /// Per triangle, an index into `shaders`.
    pub shading: Vec<u32>,
    /// Per shading index, an index into [`Scene::materials`].
    pub shaders: Vec<usize>,
}

/// A placed mesh: one model node.
#[derive(Debug, Clone)]
pub struct Node {
    /// Unique among the scene's nodes, not empty.
    pub name: String,
    pub mesh: usize,
    /// Column-major 4 x 4 placement in the world frame.
    pub matrix: [f32; 16],
    /// Key/value strings written as the node block's metadata.
    pub metadata: Vec<(String, String)>,
}

/// What [`write_u3d`] writes.
#[derive(Debug, Clone, Default)]
pub struct Scene {
    /// sRGB `[r, g, b, a]`, 0 to 1.
    pub materials: Vec<[f32; 4]>,
    pub meshes: Vec<Mesh>,
    pub nodes: Vec<Node>,
}

pub const BLOCK_FILE_HEADER: u32 = 0x0044_3355;
pub const BLOCK_MODIFIER_CHAIN: u32 = 0xFFFF_FF14;
pub const BLOCK_MODEL_NODE: u32 = 0xFFFF_FF22;
pub const BLOCK_CLOD_MESH_DECLARATION: u32 = 0xFFFF_FF31;
pub const BLOCK_CLOD_BASE_MESH: u32 = 0xFFFF_FF3B;
pub const BLOCK_SHADING_MODIFIER: u32 = 0xFFFF_FF45;
pub const BLOCK_LIT_TEXTURE_SHADER: u32 = 0xFFFF_FF53;
pub const BLOCK_MATERIAL: u32 = 0xFFFF_FF54;

/// The dynamic context of a face's shading id (`cShading`).
pub const CONTEXT_SHADING: u32 = 1;
/// Static contexts are `STATIC_FULL + R` for the range 0 to `R - 1`.
pub const STATIC_FULL: u32 = 0x400;
const CONTEXT8: u32 = 0;
const MAX_RANGE: u32 = STATIC_FULL + 0x3FFF;
const HALF: u32 = 0x8000;
const QUARTER: u32 = 0x4000;
/// A dynamic histogram holding this many occurrences is halved (10.4.9).
const ELEPHANT: u32 = 0x1FFF;

/// A dynamic context's histogram: `counts[s]` occurrences of symbol `s`
/// (value + 1; 0 is the escape symbol, which starts at 1).
#[derive(Debug, Clone)]
struct Histogram {
    counts: Vec<u32>,
    total: u32,
}

impl Histogram {
    fn new() -> Self {
        Self {
            counts: vec![1],
            total: 1,
        }
    }

    fn frequency(&self, symbol: u32) -> u32 {
        self.counts.get(symbol as usize).copied().unwrap_or(0)
    }

    fn cumulative(&self, symbol: u32) -> u32 {
        self.counts.iter().take(symbol as usize).sum()
    }

    fn add(&mut self, symbol: u32) {
        // Symbols from 0xFFFF up are never counted.
        if symbol >= 0xFFFF {
            return;
        }
        if self.total >= ELEPHANT {
            for c in &mut self.counts {
                *c >>= 1;
            }
            self.counts[0] += 1;
            self.total = self.counts.iter().sum();
        }
        let s = symbol as usize;
        if self.counts.len() <= s {
            self.counts.resize(s + 1, 0);
        }
        self.counts[s] += 1;
        self.total += 1;
    }
}

/// The bit encoder of ECMA-363 clause 10: one per block's data.
#[derive(Debug)]
pub struct BitWriter {
    out: Vec<u8>,
    bits: usize,
    high: u32,
    low: u32,
    underflow: u32,
    compressed: bool,
    histograms: Vec<Option<Histogram>>,
}

impl Default for BitWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl BitWriter {
    pub fn new() -> Self {
        Self {
            out: Vec::new(),
            bits: 0,
            high: 0xFFFF,
            low: 0,
            underflow: 0,
            compressed: false,
            histograms: vec![None; STATIC_FULL as usize],
        }
    }

    fn write_bit(&mut self, bit: u32) {
        let byte = self.bits / 8;
        if byte == self.out.len() {
            self.out.push(0);
        }
        self.out[byte] |= ((bit & 1) as u8) << (self.bits % 8);
        self.bits += 1;
    }

    fn is_static(context: u32) -> bool {
        context == CONTEXT8 || context >= STATIC_FULL
    }

    fn histogram(&mut self, context: u32) -> &mut Histogram {
        self.histograms[context as usize].get_or_insert_with(Histogram::new)
    }

    /// Writes `value` as a symbol in `context` (10.4.4); true when the
    /// escape symbol was written instead.
    fn write_symbol(&mut self, context: u32, value: u32) -> bool {
        let mut symbol = value.wrapping_add(1);
        let (total, cumulative, frequency) = if Self::is_static(context) {
            let total = if context == CONTEXT8 {
                256
            } else {
                context - STATIC_FULL
            };
            (total, symbol - 1, 1)
        } else {
            let h = self.histogram(context);
            let mut f = h.frequency(symbol);
            let mut c = h.cumulative(symbol);
            if f == 0 {
                symbol = 0;
                c = 0;
                f = h.frequency(0);
            }
            (h.total, c, f)
        };
        let range = u64::from(self.high) + 1 - u64::from(self.low);
        let low = u64::from(self.low);
        self.high = (low + range * u64::from(cumulative + frequency) / u64::from(total) - 1) as u32;
        self.low = (low + range * u64::from(cumulative) / u64::from(total)) as u32;
        if !Self::is_static(context) {
            self.histogram(context).add(symbol);
        }
        while (self.high & HALF) == (self.low & HALF) {
            let bit = self.low >> 15;
            self.high = ((self.high & !HALF) << 1) | 1;
            self.write_bit(bit);
            while self.underflow > 0 {
                self.underflow -= 1;
                self.write_bit(!bit & 1);
            }
            self.low = (self.low & !HALF) << 1;
        }
        while (self.high & QUARTER) == 0 && (self.low & QUARTER) == QUARTER {
            self.high = ((self.high & !HALF) << 1) | HALF | 1;
            self.low = ((self.low << 1) & !HALF) & 0xFFFF;
            self.underflow += 1;
        }
        symbol == 0
    }

    pub fn u8(&mut self, v: u8) {
        self.write_symbol(CONTEXT8, u32::from(v.reverse_bits()));
    }

    pub fn u16(&mut self, v: u16) {
        for b in v.to_le_bytes() {
            self.u8(b);
        }
    }

    pub fn u32(&mut self, v: u32) {
        for b in v.to_le_bytes() {
            self.u8(b);
        }
    }

    pub fn u64(&mut self, v: u64) {
        for b in v.to_le_bytes() {
            self.u8(b);
        }
    }

    pub fn f32(&mut self, v: f32) {
        self.u32(v.to_bits());
    }

    /// A U16 byte count and the UTF-8 bytes (cut at a character boundary
    /// below 65,536 bytes).
    pub fn string(&mut self, s: &str) {
        let mut end = s.len().min(0xFFFF);
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        self.u16(end as u16);
        for &b in &s.as_bytes()[..end] {
            self.u8(b);
        }
    }

    /// A compressed U32 in `context` (10.4.5).
    pub fn compressed_u32(&mut self, context: u32, v: u32) {
        self.compressed = true;
        if context != CONTEXT8 && context < MAX_RANGE {
            if self.write_symbol(context, v) {
                self.u32(v);
                if !Self::is_static(context) {
                    self.histogram(context).add(v.wrapping_add(1));
                }
            }
        } else {
            self.u32(v);
        }
    }

    /// The data bytes: flushed (10.4.10) and padded to a whole byte.
    pub fn finish(mut self) -> Vec<u8> {
        if self.compressed {
            self.u32(0);
        }
        self.out
    }
}

/// The static context of the range 0 to `count - 1`.
pub fn range_context(count: usize) -> u32 {
    STATIC_FULL.saturating_add(u32::try_from(count).unwrap_or(u32::MAX))
}

fn pad4(out: &mut Vec<u8>, len: usize) {
    out.resize(out.len() + (4 - len % 4) % 4, 0);
}

/// A block (9.2): type, sizes, data and metadata, each padded to 4 bytes.
pub fn block(out: &mut Vec<u8>, block_type: u32, data: &[u8], metadata: &[u8]) {
    out.extend_from_slice(&block_type.to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&(metadata.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
    pad4(out, data.len());
    out.extend_from_slice(metadata);
    pad4(out, metadata.len());
}

/// A modifier chain block holding `modifiers` (already written blocks).
fn modifier_chain(out: &mut Vec<u8>, name: &str, chain_type: u32, modifiers: &[Vec<u8>]) {
    let mut w = BitWriter::new();
    w.string(name);
    w.u32(chain_type);
    w.u32(0); // no bounding sphere or box
    let mut data = w.finish();
    // Padding to the 32-bit boundary before Modifier Count.
    data.resize(data.len().div_ceil(4) * 4, 0);
    data.extend_from_slice(&(modifiers.len() as u32).to_le_bytes());
    for m in modifiers {
        data.extend_from_slice(m);
    }
    block(out, BLOCK_MODIFIER_CHAIN, &data, &[]);
}

fn metadata_bytes(pairs: &[(String, String)]) -> Vec<u8> {
    if pairs.is_empty() {
        return Vec::new();
    }
    let mut w = BitWriter::new();
    w.u32(pairs.len() as u32);
    for (k, v) in pairs {
        w.u32(0); // the value is a string
        w.string(k);
        w.string(v);
    }
    w.finish()
}

/// Each triangle's unit normal (`(0, 0, 1)` for a degenerate one).
pub fn face_normals(mesh: &Mesh) -> Vec<[f32; 3]> {
    mesh.triangles
        .iter()
        .map(|t| {
            let p = |i: u32| {
                mesh.positions
                    .get(i as usize)
                    .copied()
                    .unwrap_or_default()
                    .map(f64::from)
            };
            let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
            let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let n = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            if l > 0.0 && l.is_finite() {
                n.map(|x| (x / l) as f32)
            } else {
                [0.0, 0.0, 1.0]
            }
        })
        .collect()
}

pub fn mesh_name(i: usize) -> String {
    format!("Mesh {i}")
}

pub fn shader_name(i: usize) -> String {
    format!("Shader {i}")
}

pub fn material_name(i: usize) -> String {
    format!("Material {i}")
}

/// The scene as a U3D file. A mesh with no triangle is left out, with the
/// nodes that place it.
pub fn write_u3d(scene: &Scene) -> Vec<u8> {
    let drawn = |m: usize| scene.meshes.get(m).is_some_and(|m| !m.triangles.is_empty());
    let mut declarations = Vec::new();

    for (i, rgba) in scene.materials.iter().enumerate() {
        let mut w = BitWriter::new();
        w.string(&shader_name(i));
        w.u32(1); // lighting enabled
        w.f32(0.0); // alpha test reference
        w.u32(0x617); // alpha test ALWAYS
        w.u32(0x606); // alpha blend
        w.u32(1); // render pass 0
        w.u32(0); // no texture channels
        w.u32(0);
        w.string(&material_name(i));
        block(
            &mut declarations,
            BLOCK_LIT_TEXTURE_SHADER,
            &w.finish(),
            &[],
        );

        let mut w = BitWriter::new();
        let [r, g, b, a] = *rgba;
        w.string(&material_name(i));
        w.u32(0x3F); // every attribute below
        for c in [r * 0.25, g * 0.25, b * 0.25] {
            w.f32(c); // ambient
        }
        for c in [r, g, b] {
            w.f32(c); // diffuse
        }
        for _ in 0..3 {
            w.f32(0.1); // specular
        }
        for _ in 0..3 {
            w.f32(0.0); // emissive
        }
        w.f32(0.1); // reflectivity
        w.f32(a); // opacity
        block(&mut declarations, BLOCK_MATERIAL, &w.finish(), &[]);
    }

    let mut continuations = Vec::new();
    for (i, mesh) in scene.meshes.iter().enumerate() {
        if !drawn(i) {
            continue;
        }
        let name = mesh_name(i);
        let (faces, positions) = (mesh.triangles.len(), mesh.positions.len());
        let shading_count = mesh.shaders.len().max(1);

        let mut w = BitWriter::new();
        w.string(&name);
        w.u32(0); // chain index
        // Max Mesh Description.
        w.u32(0); // faces have normals
        w.u32(faces as u32);
        w.u32(positions as u32);
        w.u32(faces as u32); // one normal per face
        w.u32(0); // diffuse colours
        w.u32(0); // specular colours
        w.u32(0); // texture coordinates
        w.u32(shading_count as u32);
        for s in 0..shading_count {
            w.u32(0); // no per-vertex colours
            w.u32(0); // no texture layers
            w.u32(s as u32); // original shading id
        }
        // CLOD Description: the base mesh is the whole mesh.
        w.u32(positions as u32);
        w.u32(positions as u32);
        // Resource Description.
        for _ in 0..3 {
            w.u32(1000); // quality factors (informative)
        }
        for _ in 0..5 {
            w.f32(1.0); // inverse quantization (the base mesh is not quantized)
        }
        w.f32(0.9); // normal crease
        w.f32(0.5); // normal update
        w.f32(0.985); // normal tolerance
        w.u32(0); // no bones
        let mut declaration = Vec::new();
        block(
            &mut declaration,
            BLOCK_CLOD_MESH_DECLARATION,
            &w.finish(),
            &[],
        );
        modifier_chain(&mut declarations, &name, 1, &[declaration]);

        let normals = face_normals(mesh);
        let mut w = BitWriter::new();
        w.string(&name);
        w.u32(0); // chain index
        w.u32(faces as u32);
        w.u32(positions as u32);
        w.u32(faces as u32);
        w.u32(0);
        w.u32(0);
        w.u32(0);
        for p in &mesh.positions {
            p.iter().for_each(|&c| w.f32(c));
        }
        for n in &normals {
            n.iter().for_each(|&c| w.f32(c));
        }
        let (position_context, normal_context) = (range_context(positions), range_context(faces));
        for (f, t) in mesh.triangles.iter().enumerate() {
            let shading = mesh.shading.get(f).copied().unwrap_or(0);
            w.compressed_u32(CONTEXT_SHADING, shading.min(shading_count as u32 - 1));
            for &v in t {
                w.compressed_u32(position_context, v.min(positions as u32 - 1));
                w.compressed_u32(normal_context, f as u32);
            }
        }
        block(&mut continuations, BLOCK_CLOD_BASE_MESH, &w.finish(), &[]);
    }

    for node in &scene.nodes {
        if !drawn(node.mesh) {
            continue;
        }
        let mut w = BitWriter::new();
        w.string(&node.name);
        w.u32(1); // one parent: the world (default) group node
        w.string("");
        node.matrix.iter().for_each(|&c| w.f32(c));
        w.string(&mesh_name(node.mesh));
        w.u32(3); // front and back visible
        let mut model_node = Vec::new();
        block(
            &mut model_node,
            BLOCK_MODEL_NODE,
            &w.finish(),
            &metadata_bytes(&node.metadata),
        );

        let shaders = &scene.meshes[node.mesh].shaders;
        let mut w = BitWriter::new();
        w.string(&node.name);
        w.u32(1); // chain index
        w.u32(1); // applied to the mesh group
        w.u32(shaders.len() as u32);
        for &m in shaders {
            w.u32(1);
            w.string(&shader_name(m));
        }
        let mut shading = Vec::new();
        block(&mut shading, BLOCK_SHADING_MODIFIER, &w.finish(), &[]);
        modifier_chain(&mut declarations, &node.name, 0, &[model_node, shading]);
    }

    const HEADER_LEN: usize = 12 + 24;
    let declaration_size = HEADER_LEN + declarations.len();
    let file_size = declaration_size + continuations.len();
    let mut w = BitWriter::new();
    w.u16(0); // major version
    w.u16(0); // minor version
    w.u32(0); // base profile
    w.u32(declaration_size as u32);
    w.u64(file_size as u64);
    w.u32(106); // UTF-8
    let mut out = Vec::with_capacity(file_size);
    block(&mut out, BLOCK_FILE_HEADER, &w.finish(), &[]);
    debug_assert_eq!(out.len(), HEADER_LEN);
    out.extend_from_slice(&declarations);
    out.extend_from_slice(&continuations);
    out
}
