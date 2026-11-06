//! The USD binary ("Crate", `.usdc`) layer format, written from AOUSD Core
//! Specification 1.0.1 section 16.3 and checked against what OpenUSD 26.11
//! reads and writes. Only what [`super::usd_writer`] emits:
//! prim and attribute specs, the scalar, array, dictionary, token-vector and
//! prepend-path-list values it uses. Crate version 0.8.0, the lowest the spec
//! allows writing.
//!
//! Where the spec text and OpenUSD's files differ, the files win (probed with
//! `crate_dump.py`): an LZ4 buffer starts with a one-byte chunk
//! count (the spec says 8 bytes); the PATHS section holds two counts before
//! its arrays; the root path's element token is the empty token; flags are
//! array `0x80`, inlined `0x40`, compressed `0x20`; dictionary keys index
//! STRINGS, not TOKENS; an integer array under 16 elements is never
//! compressed; dictionary and other
//! value offsets are relative to the offset field itself.

use std::collections::HashMap;

/// Value type ids (spec 16.3.10.1).
pub(super) mod ty {
    pub const BOOL: u8 = 1;
    pub const INT: u8 = 3;
    pub const INT64: u8 = 5;
    pub const FLOAT: u8 = 8;
    pub const DOUBLE: u8 = 9;
    pub const STRING: u8 = 10;
    pub const TOKEN: u8 = 11;
    pub const MATRIX4D: u8 = 15;
    pub const FLOAT3: u8 = 24;
    pub const DICTIONARY: u8 = 31;
    pub const PATH_LIST_OP: u8 = 34;
    pub const TOKEN_VECTOR: u8 = 41;
    pub const SPECIFIER: u8 = 42;
    pub const VARIABILITY: u8 = 44;
}

/// Spec forms (spec 16.3.8.4.6).
pub(super) const FORM_ATTRIBUTE: u32 = 1;
pub(super) const FORM_PRIM: u32 = 6;
pub(super) const FORM_LAYER: u32 = 7;

const ARRAY: u64 = 0x80;
const INLINED: u64 = 0x40;
const COMPRESSED: u64 = 0x20;
/// LZ4's largest block input; OpenUSD splits bigger buffers into chunks.
const LZ4_MAX_INPUT: usize = 0x7E00_0000;

/// One path element: a prim (`/`) or property (`.`) under its parent.
struct PathNode {
    token: u32,
    property: bool,
    children: Vec<u32>,
}

/// A crate layer being written.
pub(super) struct Crate {
    data: Vec<u8>,
    tokens: Vec<String>,
    token_ix: HashMap<String, u32>,
    strings: Vec<u32>,
    string_ix: HashMap<u32, u32>,
    fields: Vec<(u32, u64)>,
    field_ix: HashMap<(u32, u64), u32>,
    fieldsets: Vec<u32>,
    fieldset_ix: HashMap<Vec<u32>, u32>,
    paths: Vec<PathNode>,
    specs: Vec<(u32, u32, u32)>,
}

impl Crate {
    pub(super) fn new() -> Self {
        let mut c = Crate {
            // header (8 + 8), bootstrap TOC offset (8), the rest reserved
            data: vec![0; 88],
            tokens: Vec::new(),
            token_ix: HashMap::new(),
            strings: Vec::new(),
            string_ix: HashMap::new(),
            fields: Vec::new(),
            field_ix: HashMap::new(),
            fieldsets: Vec::new(),
            fieldset_ix: HashMap::new(),
            paths: Vec::new(),
            specs: Vec::new(),
        };
        // Token 0 is referenced by nothing (spec 16.3.8.4.1).
        c.tokens.push(";-)".into());
        let root = c.token("");
        c.paths.push(PathNode {
            token: root,
            property: false,
            children: Vec::new(),
        });
        c
    }

    pub(super) fn token(&mut self, s: &str) -> u32 {
        if let Some(i) = self.token_ix.get(s) {
            return *i;
        }
        let i = self.tokens.len() as u32;
        self.tokens.push(s.to_string());
        self.token_ix.insert(s.to_string(), i);
        i
    }

    fn string(&mut self, s: &str) -> u32 {
        let t = self.token(s);
        if let Some(i) = self.string_ix.get(&t) {
            return *i;
        }
        let i = self.strings.len() as u32;
        self.strings.push(t);
        self.string_ix.insert(t, i);
        i
    }

    /// The path `parent` + `/name` (prim) or `.name` (property).
    pub(super) fn path(&mut self, parent: u32, name: &str, property: bool) -> u32 {
        let token = self.token(name);
        let i = self.paths.len() as u32;
        self.paths.push(PathNode {
            token,
            property,
            children: Vec::new(),
        });
        self.paths[parent as usize].children.push(i);
        i
    }

    fn inline(t: u8, payload: u32) -> u64 {
        u64::from(payload) | (u64::from(t) << 48) | (INLINED << 56)
    }

    fn at(&self, t: u8, flags: u64) -> u64 {
        self.data.len() as u64 | (u64::from(t) << 48) | (flags << 56)
    }

    fn align8(&mut self) {
        while !self.data.len().is_multiple_of(8) {
            self.data.push(0);
        }
    }

    pub(super) fn bool(v: bool) -> u64 {
        Self::inline(ty::BOOL, u32::from(v))
    }

    pub(super) fn specifier(v: i32) -> u64 {
        Self::inline(ty::SPECIFIER, v as u32)
    }

    pub(super) fn variability(v: i32) -> u64 {
        Self::inline(ty::VARIABILITY, v as u32)
    }

    pub(super) fn string_value(&mut self, s: &str) -> u64 {
        let i = self.string(s);
        Self::inline(ty::STRING, i)
    }

    pub(super) fn token_value(&mut self, s: &str) -> u64 {
        let i = self.token(s);
        Self::inline(ty::TOKEN, i)
    }

    pub(super) fn int64(&mut self, v: i64) -> u64 {
        if let Ok(small) = i32::try_from(v) {
            return Self::inline(ty::INT64, small as u32);
        }
        let rep = self.at(ty::INT64, 0);
        self.data.extend_from_slice(&v.to_le_bytes());
        rep
    }

    pub(super) fn double(&mut self, v: f64) -> u64 {
        if f64::from(v as f32).to_bits() == v.to_bits() {
            return Self::inline(ty::DOUBLE, (v as f32).to_bits());
        }
        let rep = self.at(ty::DOUBLE, 0);
        self.data.extend_from_slice(&v.to_le_bytes());
        rep
    }

    /// An uncompressed array of fixed-size elements.
    fn array(&mut self, t: u8, count: usize, bytes: impl IntoIterator<Item = u8>) -> u64 {
        if count == 0 {
            return u64::from(t) << 48 | ARRAY << 56;
        }
        self.align8();
        let rep = self.at(t, ARRAY);
        self.data.extend_from_slice(&(count as u64).to_le_bytes());
        self.data.extend(bytes);
        rep
    }

    /// An integer array, compressed from 16 elements on. The spec says
    /// "16 bytes or smaller should not be compressed"; OpenUSD 26.11 reads an
    /// array of fewer than 16 elements as uncompressed whatever its flag
    /// (probed), so the bound is 16 elements.
    pub(super) fn int_array(&mut self, t: u8, values: &[i64]) -> u64 {
        let width = if t == ty::INT { 4 } else { 8 };
        if values.len() < 16 {
            let bytes: Vec<u8> = values
                .iter()
                .flat_map(|v| {
                    if width == 4 {
                        (*v as i32).to_le_bytes().to_vec()
                    } else {
                        v.to_le_bytes().to_vec()
                    }
                })
                .collect();
            return self.array(t, values.len(), bytes);
        }
        self.align8();
        let rep = self.at(t, ARRAY | COMPRESSED);
        self.data
            .extend_from_slice(&(values.len() as u64).to_le_bytes());
        let packed = compressed_ints(values, width);
        self.data.extend_from_slice(&packed);
        rep
    }

    pub(super) fn bool_array(&mut self, v: &[bool]) -> u64 {
        self.array(ty::BOOL, v.len(), v.iter().map(|b| u8::from(*b)))
    }

    pub(super) fn double_array(&mut self, v: &[f64]) -> u64 {
        self.array(ty::DOUBLE, v.len(), v.iter().flat_map(|x| x.to_le_bytes()))
    }

    pub(super) fn float_array(&mut self, v: &[f64]) -> u64 {
        self.array(
            ty::FLOAT,
            v.len(),
            v.iter().flat_map(|x| (*x as f32).to_le_bytes()),
        )
    }

    pub(super) fn float3_array(&mut self, v: &[[f64; 3]]) -> u64 {
        self.array(
            ty::FLOAT3,
            v.len(),
            v.iter().flatten().flat_map(|x| (*x as f32).to_le_bytes()),
        )
    }

    pub(super) fn string_array(&mut self, v: &[String]) -> u64 {
        let ix: Vec<u32> = v.iter().map(|s| self.string(s)).collect();
        self.array(
            ty::STRING,
            ix.len(),
            ix.into_iter().flat_map(u32::to_le_bytes),
        )
    }

    /// A `matrix4d`, row-major, 16 doubles.
    pub(super) fn matrix4d(&mut self, m: &[f64; 16]) -> u64 {
        self.align8();
        let rep = self.at(ty::MATRIX4D, 0);
        for x in m {
            self.data.extend_from_slice(&x.to_le_bytes());
        }
        rep
    }

    /// A `token[]` value (`xformOpOrder`).
    pub(super) fn token_array(&mut self, v: &[&str]) -> u64 {
        let ix: Vec<u32> = v.iter().map(|s| self.token(s)).collect();
        self.array(
            ty::TOKEN,
            ix.len(),
            ix.into_iter().flat_map(u32::to_le_bytes),
        )
    }

    /// A dictionary of already-encoded values: each value representation sits
    /// right after its offset field, so every offset is 8. Keys are indices
    /// into STRINGS (OpenUSD's files; the spec says tokens).
    pub(super) fn dictionary(&mut self, entries: &[(&str, u64)]) -> u64 {
        if entries.is_empty() {
            return Self::inline(ty::DICTIONARY, 0);
        }
        let keys: Vec<u32> = entries.iter().map(|(k, _)| self.string(k)).collect();
        let rep = self.at(ty::DICTIONARY, 0);
        self.data
            .extend_from_slice(&(entries.len() as u64).to_le_bytes());
        for (k, (_, v)) in keys.iter().zip(entries) {
            self.data.extend_from_slice(&k.to_le_bytes());
            self.data.extend_from_slice(&8i64.to_le_bytes());
            self.data.extend_from_slice(&v.to_le_bytes());
        }
        rep
    }

    pub(super) fn token_vector(&mut self, names: &[u32]) -> u64 {
        let rep = self.at(ty::TOKEN_VECTOR, 0);
        self.data
            .extend_from_slice(&(names.len() as u64).to_le_bytes());
        for t in names {
            self.data.extend_from_slice(&t.to_le_bytes());
        }
        rep
    }

    /// A path list op with prepended items only.
    pub(super) fn prepend_paths(&mut self, paths: &[u32]) -> u64 {
        let rep = self.at(ty::PATH_LIST_OP, 0);
        self.data.push(0x20);
        self.data
            .extend_from_slice(&(paths.len() as u64).to_le_bytes());
        for p in paths {
            self.data.extend_from_slice(&p.to_le_bytes());
        }
        rep
    }

    /// A spec: its path, form and `(field name, value)` list.
    pub(super) fn spec(&mut self, path: u32, form: u32, fields: &[(&str, u64)]) {
        let mut set: Vec<u32> = Vec::with_capacity(fields.len() + 1);
        for (name, rep) in fields {
            let t = self.token(name);
            let i = match self.field_ix.get(&(t, *rep)) {
                Some(i) => *i,
                None => {
                    let i = self.fields.len() as u32;
                    self.fields.push((t, *rep));
                    self.field_ix.insert((t, *rep), i);
                    i
                }
            };
            set.push(i);
        }
        let start = match self.fieldset_ix.get(&set) {
            Some(s) => *s,
            None => {
                let s = self.fieldsets.len() as u32;
                self.fieldsets.extend_from_slice(&set);
                self.fieldsets.push(u32::MAX);
                self.fieldset_ix.insert(set, s);
                s
            }
        };
        self.specs.push((path, start, form));
    }

    /// The file: data, then the sections, then the table of contents.
    pub(super) fn finish(mut self) -> Result<Vec<u8>, String> {
        let mut toc: Vec<(&str, u64, u64)> = Vec::new();
        let mut section = |c: &mut Crate, name: &'static str, body: Vec<u8>| {
            toc.push((name, c.data.len() as u64, body.len() as u64));
            c.data.extend_from_slice(&body);
        };

        let mut raw = Vec::new();
        for t in &self.tokens {
            raw.extend_from_slice(t.as_bytes());
            raw.push(0);
        }
        let packed = lz4_buffer(&raw)?;
        let mut body = Vec::new();
        for n in [self.tokens.len(), raw.len(), packed.len()] {
            body.extend_from_slice(&(n as u64).to_le_bytes());
        }
        body.extend_from_slice(&packed);
        section(&mut self, "TOKENS", body);

        let mut body = (self.strings.len() as u64).to_le_bytes().to_vec();
        for s in &self.strings {
            body.extend_from_slice(&s.to_le_bytes());
        }
        section(&mut self, "STRINGS", body);

        let mut body = (self.fields.len() as u64).to_le_bytes().to_vec();
        let names: Vec<i64> = self.fields.iter().map(|(t, _)| i64::from(*t)).collect();
        body.extend_from_slice(&compressed_ints(&names, 4));
        let reps: Vec<u8> = self
            .fields
            .iter()
            .flat_map(|(_, r)| r.to_le_bytes())
            .collect();
        let packed = lz4_buffer(&reps)?;
        body.extend_from_slice(&(packed.len() as u64).to_le_bytes());
        body.extend_from_slice(&packed);
        section(&mut self, "FIELDS", body);

        let mut body = (self.fieldsets.len() as u64).to_le_bytes().to_vec();
        let sets: Vec<i64> = self
            .fieldsets
            .iter()
            .map(|i| i64::from(*i as i32))
            .collect();
        body.extend_from_slice(&compressed_ints(&sets, 4));
        section(&mut self, "FIELDSETS", body);

        let (ix, el, jump) = self.encode_paths();
        let mut body = Vec::new();
        for _ in 0..2 {
            body.extend_from_slice(&(ix.len() as u64).to_le_bytes());
        }
        for a in [&ix, &el, &jump] {
            body.extend_from_slice(&compressed_ints(a, 4));
        }
        section(&mut self, "PATHS", body);

        let mut body = (self.specs.len() as u64).to_le_bytes().to_vec();
        let cols: [Vec<i64>; 3] = [
            self.specs.iter().map(|s| i64::from(s.0)).collect(),
            self.specs.iter().map(|s| i64::from(s.1)).collect(),
            self.specs.iter().map(|s| i64::from(s.2)).collect(),
        ];
        for c in &cols {
            body.extend_from_slice(&compressed_ints(c, 4));
        }
        section(&mut self, "SPECS", body);

        let toc_at = self.data.len() as u64;
        self.data
            .extend_from_slice(&(toc.len() as u64).to_le_bytes());
        for (name, start, size) in toc {
            let mut n = [0u8; 16];
            n[..name.len()].copy_from_slice(name.as_bytes());
            self.data.extend_from_slice(&n);
            self.data.extend_from_slice(&start.to_le_bytes());
            self.data.extend_from_slice(&size.to_le_bytes());
        }
        self.data[..8].copy_from_slice(b"PXR-USDC");
        self.data[8..11].copy_from_slice(&[0, 8, 0]);
        self.data[16..24].copy_from_slice(&toc_at.to_le_bytes());
        Ok(self.data)
    }

    /// Depth-first: path index, element token (negative for a property) and
    /// jump (spec 16.3.8.4.5): -2 leaf, -1 child only, 0 sibling only, > 0
    /// the offset of the next sibling when there is a child too.
    fn encode_paths(&self) -> (Vec<i64>, Vec<i64>, Vec<i64>) {
        let mut ix = Vec::with_capacity(self.paths.len());
        let mut el = Vec::with_capacity(self.paths.len());
        let mut jump = Vec::with_capacity(self.paths.len());
        // (node, has a next sibling, the earlier sibling waiting for this
        // node's index); an explicit stack, as the tree can be deep. A node's
        // subtree is placed before its next sibling, which is then the top.
        let mut stack: Vec<(u32, bool, Option<usize>)> = vec![(0, false, None)];
        while let Some((node, sibling, waiting)) = stack.pop() {
            let at = ix.len();
            if let Some(w) = waiting {
                jump[w] = (at - w) as i64;
            }
            let n = &self.paths[node as usize];
            ix.push(i64::from(node));
            let t = i64::from(n.token);
            el.push(if n.property { -t } else { t });
            let child = !n.children.is_empty();
            jump.push(match (child, sibling) {
                (false, false) => -2,
                (true, false) => -1,
                (false, true) | (true, true) => 0,
            });
            if child
                && sibling
                && let Some(next) = stack.last_mut()
            {
                next.2 = Some(at);
            }
            let kids = &n.children;
            for (k, c) in kids.iter().enumerate().rev() {
                stack.push((*c, k + 1 < kids.len(), None));
            }
        }
        (ix, el, jump)
    }
}

/// An LZ4 buffer as OpenUSD stores it: a chunk-count byte (0: one block),
/// then the LZ4 block.
fn lz4_buffer(src: &[u8]) -> Result<Vec<u8>, String> {
    if src.len() > LZ4_MAX_INPUT {
        return Err(format!(
            "usdc: a {}-byte buffer needs LZ4 chunking, which this writer does not do",
            src.len()
        ));
    }
    let mut out = vec![0u8];
    lz4_block(src, &mut out);
    Ok(out)
}

/// A compressed integer array (spec 16.3.7.2): differences, the commonest
/// difference, 2-bit width codes, the values, LZ4, prefixed by its size.
pub(super) fn compressed_ints(values: &[i64], width: usize) -> Vec<u8> {
    let mut diffs = Vec::with_capacity(values.len());
    let mut prev = 0i64;
    for v in values {
        let d = if width == 4 {
            i64::from((*v as i32).wrapping_sub(prev as i32))
        } else {
            v.wrapping_sub(prev)
        };
        diffs.push(d);
        prev = *v;
    }
    let mut counts: HashMap<i64, usize> = HashMap::new();
    for d in &diffs {
        *counts.entry(*d).or_default() += 1;
    }
    let common = counts
        .iter()
        .max_by_key(|(d, n)| (**n, std::cmp::Reverse(**d)))
        .map(|(d, _)| *d)
        .unwrap_or(0);
    let mut raw = Vec::new();
    if width == 4 {
        raw.extend_from_slice(&(common as i32).to_le_bytes());
    } else {
        raw.extend_from_slice(&common.to_le_bytes());
    }
    let codes_at = raw.len();
    raw.resize(codes_at + (diffs.len() * 2).div_ceil(8), 0);
    for (k, d) in diffs.iter().enumerate() {
        let (code, bytes): (u8, Vec<u8>) = if *d == common {
            (0, Vec::new())
        } else if width == 4 && i8::try_from(*d).is_ok() {
            (1, (*d as i8).to_le_bytes().to_vec())
        } else if width == 8 && i16::try_from(*d).is_ok() {
            (1, (*d as i16).to_le_bytes().to_vec())
        } else if width == 4 && i16::try_from(*d).is_ok() {
            (2, (*d as i16).to_le_bytes().to_vec())
        } else if width == 8 && i32::try_from(*d).is_ok() {
            (2, (*d as i32).to_le_bytes().to_vec())
        } else if width == 4 {
            (3, (*d as i32).to_le_bytes().to_vec())
        } else {
            (3, d.to_le_bytes().to_vec())
        };
        raw[codes_at + k / 4] |= code << (2 * (k % 4));
        raw.extend_from_slice(&bytes);
    }
    let mut packed = vec![0u8];
    lz4_block(&raw, &mut packed);
    let mut out = (packed.len() as u64).to_le_bytes().to_vec();
    out.extend_from_slice(&packed);
    out
}

/// One LZ4 block (lz4.org block format): greedy matches found through a hash
/// of 4-byte sequences. The last 5 bytes are literals and no match starts in
/// the last 12, as the format requires.
// ponytail: greedy, one candidate per hash; a better parser compresses more.
fn lz4_block(src: &[u8], out: &mut Vec<u8>) {
    let n = src.len();
    let mut anchor = 0;
    let mut i = 0;
    let mut table = vec![usize::MAX; 1 << 16];
    let read = |p: usize| u32::from_le_bytes([src[p], src[p + 1], src[p + 2], src[p + 3]]);
    if n >= 13 {
        let limit = n - 12;
        while i < limit {
            let seq = read(i);
            let h = (seq.wrapping_mul(2_654_435_761) >> 16) as usize;
            let cand = table[h];
            table[h] = i;
            if cand != usize::MAX && i - cand <= 0xFFFF && read(cand) == seq {
                let mut len = 4;
                while i + len < n - 5 && src[cand + len] == src[i + len] {
                    len += 1;
                }
                sequence(out, &src[anchor..i], Some((i - cand, len)));
                i += len;
                anchor = i;
            } else {
                i += 1;
            }
        }
    }
    sequence(out, &src[anchor..], None);
}

fn sequence(out: &mut Vec<u8>, literals: &[u8], matched: Option<(usize, usize)>) {
    let lit = literals.len();
    let ml = matched.map_or(0, |(_, len)| len - 4);
    out.push(((lit.min(15) as u8) << 4) | ml.min(15) as u8);
    if lit >= 15 {
        length_bytes(out, lit - 15);
    }
    out.extend_from_slice(literals);
    if let Some((offset, _)) = matched {
        out.extend_from_slice(&(offset as u16).to_le_bytes());
        if ml >= 15 {
            length_bytes(out, ml - 15);
        }
    }
}

fn length_bytes(out: &mut Vec<u8>, mut n: usize) {
    while n >= 255 {
        out.push(255);
        n -= 255;
    }
    out.push(n as u8);
}
