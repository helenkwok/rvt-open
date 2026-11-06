//! A type's name read from its parameter value block by the file's own
//! schema.
//!
//! The partition streams hold, for every element type, a *value block*: a
//! serialized `Element` whose class derives from `Symbol`. It opens with the
//! type's ElementId (`01 00 00 00 · u64 id` is how the objects are found,
//! `crate::partition_id_objects`). The class is named by the tag at `+0x4a`
//! of the type's element record ([`class_tags`]), and the file's
//! `Formats/Latest` gives that class's fields, base first
//! ([`crate::formats::schema_layouts`]).
//!
//! # The grammar
//!
//! ```text
//! block   = header · flat · tail · name
//! header  = u64 id · 7 × u64 ElementId · 3 × u8 0          (0x43 bytes)
//! flat    = the fields of the class chain below Element, in schema order
//! tail    = [D] [I] [S] [E] [W] [F] [C]                     (each optional)
//! name    = u32 n · UTF-16LE × n                            (n >= 1)
//! ```
//!
//! *header* is Element's own fields without its eight pointers: the id and
//! the seven other ElementIds it declares, then `m_locked`, `m_moribund` and
//! `m_dummy`. The pointers' contents are in the *tail*.
//!
//! *flat* gives each field the bytes its schema type has: `bool` 1, kind-2
//! byte 1, `u32` 4, kind-5 colour 4, `f64` 8, a by-value object the sum of
//! its own chain (`ElementId`, tag 20, is 8), a fixed array `n` times its
//! element, a string `u32 n · UTF-16 × n`. A pointer is `u32 0` when null
//! and `u32 marker · u16 tag` otherwise (`marker` is `ffffffff` for a new
//! object, a small number for a repeat); the content of a pointer is not in
//! *flat*. Any other type (containers, kinds never met in a parsed block)
//! stops the read: no name.
//!
//! *tail* is the content of Element's pointers that the type has, in this
//! order, each only when present:
//!
//! | | segment | bytes |
//! |---|---|---|
//! | D | doubles | `u32 n` · n × (`f64 value` · `i64 parameter`) |
//! | I | integers | `u32 n` · n × (`i64 parameter` · `u32 value`) |
//! | S | strings | `u32 n` · n × (`i64 parameter` · `u32 len` · UTF-16 × len) |
//! | E | ElementIds | `u32 n` · n × (`i64 parameter` · `u64 id`) |
//! | W | wall type constant | 71 bytes, identical in every wall type block |
//! | F | filler | `ff × 8 · 00 × 17` |
//! | C | cell list | `u32 n` · n × (`ff ff ff ff` · `u16 tag`) |
//!
//! A *parameter* is a BuiltInParameter (negative, below -1,000,000) or the
//! ElementId of a project or shared parameter (below 2^26). An empty segment
//! is `u32 0` and cannot be told from another empty one: it does not advance
//! the order. Presence is not flagged anywhere that was found; the parser
//! tries every order-respecting reading and keeps a name only where nothing
//! else parses at that position.
//!
//! # Fail closed
//!
//! No name is returned when the header, the class, a field type, or the tail
//! does not parse, or when no position past the tail holds a name. Among the
//! positions that do, the furthest is taken.
//!
//! # What is not decoded
//!
//! The *family* part of `Family:Type` is not in this block. The content of
//! the pointers other than Element's (compound structure layers, cells) is
//! not read. Block W is a 71-byte constant found byte-identical in all 252
//! wall type blocks of the 27 sample models; what it holds is not known.

use crate::RevitFile;
use crate::formats::{SchemaFieldType, SchemaLayouts};
use crate::partition_id_objects::find_id_objects;
use std::collections::{BTreeMap, BTreeSet};

/// The one Revit release the grammar was measured on: the 27 sample models
/// are all Revit 2024. [`read_type_names`] does not check it; callers that
/// name types do.
pub const MEASURED_REVIT_VERSION: u32 = 2024;

/// Bytes of the header: the id, seven more ElementIds, three bytes.
pub const HEADER_LEN: usize = 0x43;

/// Longest name read, in UTF-16 units.
const MAX_NAME_UNITS: usize = 255;
/// Longest string in a string segment, in UTF-16 units.
const MAX_SEGMENT_STRING: usize = 600;
/// Most entries in a segment.
const MAX_ENTRIES: usize = 300;

const W_BLOCK: [u8; 71] = {
    let mut w = [0u8; 71];
    w[0] = 1;
    w[4] = 0xff;
    w[5] = 0xff;
    w[6] = 0xff;
    w[7] = 0xff;
    w[8] = 0xa3;
    w[9] = 0x10;
    // 32 zero bytes
    let mut i = 42;
    while i < 70 {
        w[i] = 2;
        i += 4;
    }
    w[70] = 9;
    w
};

/// A segment of the tail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Segment {
    Doubles,
    Integers,
    Strings,
    ElementIds,
    WallConstant,
    Filler,
    Cells,
}

const SEGMENTS: [Segment; 7] = [
    Segment::Doubles,
    Segment::Integers,
    Segment::Strings,
    Segment::ElementIds,
    Segment::WallConstant,
    Segment::Filler,
    Segment::Cells,
];

/// Why no name was read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoName {
    /// The id does not open a value block at this position.
    NotABlock,
    /// The schema has no class with this tag, or it does not derive from
    /// `Symbol`, or the schema has no `Element`.
    UnknownClass,
    /// A field whose bytes the grammar does not know.
    Unsupported(String),
    /// The flat part ran past the block.
    Truncated,
    /// No name where the tail ends.
    NoCandidate,
    /// Two blocks of one partition, the newest, name the type differently.
    Conflict,
}

/// A name read from a block, and what the read used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeNameRead {
    pub name: String,
    /// The schema class of the type's record.
    pub class_name: String,
    /// Offset of the name's length prefix from the id.
    pub name_at: usize,
    /// Offset where the tail starts.
    pub flat_end: usize,
    /// The segments present, in order, with their counts (0 for W, F).
    pub segments: Vec<(Segment, u32)>,
    /// The flat field types the read relied on, for coverage tables.
    pub features: BTreeSet<String>,
    /// The pointers of the class chain, `Class.field`, with whether each is
    /// set. `HostObjAttr.m_pCompoundStructure` is set exactly when the type
    /// has compound layers.
    pub pointers: Vec<(String, bool)>,
}

/// Where the flat part ends, the field types it relied on, and its pointers.
type Flat = (usize, BTreeSet<String>, Vec<(String, bool)>);

/// The names found by the readings of the tail, by offset, with the segments
/// each reading passed.
type Found = BTreeMap<usize, (String, Vec<(Segment, u32)>)>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wire {
    Fixed(usize),
    Str,
    Pointer,
}

/// The byte layout of the classes of one file's schema.
#[derive(Debug)]
pub struct TypeNameGrammar<'a> {
    layouts: &'a SchemaLayouts,
}

impl<'a> TypeNameGrammar<'a> {
    pub fn new(layouts: &'a SchemaLayouts) -> Self {
        Self { layouts }
    }

    /// Bytes of a class held by value, or `None` when any field is not fixed.
    fn object_size(&self, tag: u16, depth: usize) -> Option<usize> {
        if depth > 8 {
            return None;
        }
        if tag == 20 {
            return Some(8);
        }
        let mut total = 0;
        for class in self.layouts.chain(tag)? {
            for field in &class.fields {
                match self.wire(&field.ty, depth + 1)? {
                    Wire::Fixed(n) => total += n,
                    _ => return None,
                }
            }
        }
        Some(total)
    }

    fn wire(&self, ty: &SchemaFieldType, depth: usize) -> Option<Wire> {
        match ty {
            SchemaFieldType::Scalar {
                kind,
                modifier,
                count,
            } => {
                if *kind == 8 && *modifier == 0x60 {
                    return Some(Wire::Str);
                }
                let size = match kind {
                    1 | 2 => 1,
                    4 | 5 => 4,
                    7 => 8,
                    _ => return None,
                };
                match (*modifier, count) {
                    (0, None) => Some(Wire::Fixed(size)),
                    (m, Some(n)) if m & 0xf0 == 0x10 => Some(Wire::Fixed(size * *n as usize)),
                    _ => None,
                }
            }
            SchemaFieldType::Object {
                tag,
                modifier,
                count,
            } => {
                if !matches!(modifier & 0xf0, 0x00 | 0x10) {
                    return None;
                }
                let size = self.object_size(*tag, depth)?;
                Some(Wire::Fixed(size * count.unwrap_or(1) as usize))
            }
            SchemaFieldType::Pointer { modifier, .. } => {
                (modifier & 0xf0 == 0 && matches!(modifier & 0x0f, 1..=3)).then_some(Wire::Pointer)
            }
            SchemaFieldType::Array {
                modifier,
                count,
                inner,
                ..
            } => {
                if modifier & 0xf0 != 0x10 {
                    return None;
                }
                match self.wire(inner, depth + 1)? {
                    Wire::Fixed(n) => Some(Wire::Fixed(n * (*count)? as usize)),
                    _ => None,
                }
            }
        }
    }

    /// Bytes of Element's header, from its fields without the pointers.
    fn header_len(&self) -> Option<usize> {
        let element = self.layouts.tag_of("Element")?;
        let mut total = 0;
        for field in &self.layouts.classes.get(&element)?.fields {
            match &field.ty {
                SchemaFieldType::Pointer { .. } => {}
                SchemaFieldType::Object { tag, .. } if *tag != 20 => {
                    // ControlledDocAccess: only a document pointer, no bytes
                    let class = self.layouts.classes.get(tag)?;
                    if !class
                        .fields
                        .iter()
                        .all(|f| matches!(f.ty, SchemaFieldType::Pointer { .. }))
                    {
                        return None;
                    }
                }
                other => match self.wire(other, 0)? {
                    Wire::Fixed(n) => total += n,
                    _ => return None,
                },
            }
        }
        Some(total)
    }

    /// The flat part: where it ends, and the field types it relied on.
    fn flat(&self, buf: &[u8], id_at: usize, class_tag: u16) -> Result<Flat, NoName> {
        let element = self.layouts.tag_of("Element").ok_or(NoName::UnknownClass)?;
        let chain = self.layouts.chain(class_tag).ok_or(NoName::UnknownClass)?;
        if chain.first().map(|c| c.tag) != Some(element)
            || !chain.iter().any(|c| c.name == "Symbol")
            || self.header_len() != Some(HEADER_LEN)
        {
            return Err(NoName::UnknownClass);
        }
        let mut at = id_at + HEADER_LEN;
        let mut features = BTreeSet::new();
        let mut pointers = Vec::new();
        for class in &chain[1..] {
            for field in &class.fields {
                let wire = self
                    .wire(&field.ty, 0)
                    .ok_or_else(|| NoName::Unsupported(format!("{}.{}", class.name, field.name)))?;
                match wire {
                    Wire::Fixed(n) => {
                        features.insert(feature(&field.ty));
                        at += n;
                    }
                    Wire::Str => {
                        features.insert("string".into());
                        let n = u32_at(buf, at).ok_or(NoName::Truncated)? as usize;
                        if n > MAX_SEGMENT_STRING * 4 {
                            return Err(NoName::Truncated);
                        }
                        at += 4 + 2 * n;
                    }
                    Wire::Pointer => {
                        let marker = u32_at(buf, at).ok_or(NoName::Truncated)?;
                        pointers.push((format!("{}.{}", class.name, field.name), marker != 0));
                        if marker == 0 {
                            features.insert("pointer null".into());
                            at += 4;
                        } else {
                            features.insert(
                                if marker == u32::MAX {
                                    "pointer new"
                                } else {
                                    "pointer repeat"
                                }
                                .into(),
                            );
                            at += 6;
                        }
                    }
                }
                if at > buf.len() {
                    return Err(NoName::Truncated);
                }
            }
        }
        Ok((at, features, pointers))
    }

    /// Whether a value block of a `Symbol` opens at `id_at`: the id, the
    /// header, and a new `SymbolInfo` object as the first field after it.
    pub fn is_block(&self, buf: &[u8], id_at: usize, id: u32) -> bool {
        let Some(info) = self.layouts.tag_of("SymbolInfo") else {
            return false;
        };
        block_header_ok(buf, id_at, id)
            && u32_at(buf, id_at + HEADER_LEN) == Some(u32::MAX)
            && u16_at(buf, id_at + HEADER_LEN + 4) == Some(info)
    }

    /// The name of the type whose value block opens at `id_at` (the offset
    /// of the `u64` id), of class `class_tag`.
    pub fn read(
        &self,
        buf: &[u8],
        id_at: usize,
        id: u32,
        class_tag: u16,
    ) -> Result<TypeNameRead, NoName> {
        if !self.is_block(buf, id_at, id) {
            return Err(NoName::NotABlock);
        }
        let (flat_end, features, pointers) = self.flat(buf, id_at, class_tag)?;
        let mut found: BTreeMap<usize, (String, Vec<(Segment, u32)>)> = BTreeMap::new();
        let mut path = Vec::new();
        candidates(buf, flat_end, 0, &mut path, &mut found);
        let (&name_at, (name, segments)) = found.iter().next_back().ok_or(NoName::NoCandidate)?;
        let class_name = self
            .layouts
            .classes
            .get(&class_tag)
            .map(|c| c.name.clone())
            .unwrap_or_default();
        Ok(TypeNameRead {
            name: name.clone(),
            class_name,
            name_at: name_at - id_at,
            flat_end: flat_end - id_at,
            segments: segments.clone(),
            features,
            pointers,
        })
    }
}

fn feature(ty: &SchemaFieldType) -> String {
    match ty {
        SchemaFieldType::Scalar {
            kind,
            modifier,
            count,
        } => format!(
            "scalar kind {kind}{}{}",
            if *modifier == 0 { "" } else { " fixed array" },
            count.map(|n| format!(" x{n}")).unwrap_or_default()
        ),
        SchemaFieldType::Object { tag, count, .. } => {
            if *tag == 20 {
                format!(
                    "ElementId{}",
                    count.map(|n| format!(" x{n}")).unwrap_or_default()
                )
            } else {
                format!("object by value {tag}")
            }
        }
        SchemaFieldType::Pointer { .. } => "pointer".into(),
        SchemaFieldType::Array { .. } => "array of arrays".into(),
    }
}

fn u32_at(buf: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        buf.get(at..at.checked_add(4)?)?.try_into().ok()?,
    ))
}

fn i64_at(buf: &[u8], at: usize) -> Option<i64> {
    Some(i64::from_le_bytes(
        buf.get(at..at.checked_add(8)?)?.try_into().ok()?,
    ))
}

fn u64_at(buf: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(
        buf.get(at..at.checked_add(8)?)?.try_into().ok()?,
    ))
}

fn u16_at(buf: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        buf.get(at..at.checked_add(2)?)?.try_into().ok()?,
    ))
}

/// The id, then the 56 bytes of other ids and the three zero bytes.
fn block_header_ok(buf: &[u8], id_at: usize, id: u32) -> bool {
    u64_at(buf, id_at) == Some(u64::from(id))
        && buf
            .get(id_at + HEADER_LEN - 3..id_at + HEADER_LEN)
            .is_some_and(|b| b == [0, 0, 0])
}

fn is_parameter(p: i64) -> bool {
    (-3_500_000 < p && p < -1_000_000) || (0 < p && p < 1 << 26)
}

fn text_ok(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| !c.is_control() && c != '\u{fffd}' && c != '\u{ffff}')
}

fn utf16(buf: &[u8], at: usize, units: usize) -> Option<String> {
    let raw = buf.get(at..at.checked_add(2 * units)?)?;
    let u: Vec<u16> = raw
        .chunks_exact(2)
        .map(|p| u16::from_le_bytes([p[0], p[1]]))
        .collect();
    String::from_utf16(&u).ok()
}

/// The end of one segment at `at`, or `None` when it does not parse.
fn segment_end(buf: &[u8], at: usize, kind: Segment) -> Option<usize> {
    match kind {
        Segment::WallConstant => {
            (buf.get(at..at + W_BLOCK.len())? == W_BLOCK).then_some(at + W_BLOCK.len())
        }
        Segment::Filler => {
            let b = buf.get(at..at + 25)?;
            (b[..8].iter().all(|&x| x == 0xff) && b[8..].iter().all(|&x| x == 0)).then_some(at + 25)
        }
        Segment::Cells => {
            let n = u32_at(buf, at)? as usize;
            if !(1..=30).contains(&n) {
                return None;
            }
            let mut p = at + 4;
            for _ in 0..n {
                if buf.get(p..p + 4)? != [0xff; 4] || u16_at(buf, p + 4)? > 5000 {
                    return None;
                }
                p += 6;
            }
            Some(p)
        }
        _ => {
            let n = u32_at(buf, at)? as usize;
            if n > MAX_ENTRIES {
                return None;
            }
            let mut p = at + 4;
            for _ in 0..n {
                match kind {
                    Segment::Doubles => {
                        if !is_parameter(i64_at(buf, p + 8)?) {
                            return None;
                        }
                        p += 16;
                    }
                    Segment::ElementIds => {
                        let id = u64_at(buf, p + 8)?;
                        if !is_parameter(i64_at(buf, p)?) || (id > 1 << 32 && id != u64::MAX) {
                            return None;
                        }
                        p += 16;
                    }
                    Segment::Integers => {
                        if !is_parameter(i64_at(buf, p)?) {
                            return None;
                        }
                        buf.get(p + 8..p + 12)?;
                        p += 12;
                    }
                    _ => {
                        if !is_parameter(i64_at(buf, p)?) {
                            return None;
                        }
                        let len = u32_at(buf, p + 8)? as usize;
                        if len > MAX_SEGMENT_STRING {
                            return None;
                        }
                        if len > 0 {
                            let s = utf16(buf, p + 12, len)?;
                            if s.chars().any(|c| c.is_control()) {
                                return None;
                            }
                        } else {
                            buf.get(p + 12..p + 12)?;
                        }
                        p += 12 + 2 * len;
                    }
                }
            }
            Some(p)
        }
    }
}

fn name_at(buf: &[u8], at: usize) -> Option<String> {
    let n = u32_at(buf, at)? as usize;
    if !(1..=MAX_NAME_UNITS).contains(&n) {
        return None;
    }
    let s = utf16(buf, at + 4, n)?;
    text_ok(&s).then_some(s)
}

/// Every position past some reading of the tail where a name stands. A
/// position where a segment of a not-yet-passed kind parses is not a name.
fn candidates(
    buf: &[u8],
    at: usize,
    from: usize,
    path: &mut Vec<(Segment, u32)>,
    found: &mut Found,
) {
    let mut took = false;
    for (k, kind) in SEGMENTS.iter().enumerate().skip(from) {
        let Some(end) = segment_end(buf, at, *kind) else {
            continue;
        };
        if end <= at {
            continue;
        }
        took = true;
        let count = match kind {
            Segment::WallConstant | Segment::Filler => 0,
            _ => u32_at(buf, at).unwrap_or(0),
        };
        path.push((*kind, count));
        let empty = end == at + 4
            && !matches!(
                kind,
                Segment::WallConstant | Segment::Filler | Segment::Cells
            );
        candidates(buf, end, if empty { from } else { k + 1 }, path, found);
        path.pop();
    }
    if !took {
        if let Some(name) = name_at(buf, at) {
            found.entry(at).or_insert_with(|| (name, path.clone()));
        }
    }
}

/// The class tag of the element record of each wanted id: `+0x4a` of the
/// record whose `u32 0x59f` sits at `+0x0c`. An id with two
/// different tags has none.
pub fn class_tags(rf: &mut RevitFile, wanted: &BTreeSet<u32>) -> crate::Result<BTreeMap<u32, u16>> {
    let mut seen: BTreeMap<u32, BTreeSet<u16>> = BTreeMap::new();
    for stream in rf.partition_stream_names() {
        let Ok(inflated) = rf.inflated_partition(&stream) else {
            continue;
        };
        let buf = inflated.bytes();
        for m in memchr::memmem::find_iter(buf, &[0x9f, 0x05, 0, 0]) {
            let Some(s) = m.checked_sub(12) else { continue };
            let (Some(id), Some(high), Some(tag)) =
                (u32_at(buf, s), u32_at(buf, s + 4), u16_at(buf, s + 0x4a))
            else {
                continue;
            };
            if high == 0 && wanted.contains(&id) {
                seen.entry(id).or_default().insert(tag);
            }
        }
    }
    Ok(seen
        .into_iter()
        .filter_map(|(id, tags)| (tags.len() == 1).then(|| (id, *tags.iter().next().unwrap())))
        .collect())
}

/// Where a name was read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadAt {
    pub stream: String,
    pub id_at: usize,
    pub read: TypeNameRead,
}

/// The names of the wanted types, read from the value block in the
/// highest-numbered partition copy (a renamed type keeps older names below).
/// A type with no entry has no name read; the second map says why.
pub fn read_type_names(
    rf: &mut RevitFile,
    wanted: &BTreeSet<u32>,
) -> crate::Result<(BTreeMap<u32, ReadAt>, BTreeMap<u32, NoName>)> {
    let layouts = rf.schema_layouts()?;
    let grammar = TypeNameGrammar::new(&layouts);
    let tags = class_tags(rf, wanted)?;
    let mut streams: Vec<(u64, String)> = rf
        .partition_stream_names()
        .into_iter()
        .map(|s| (crate::partition_names::partition_number(&s), s))
        .collect();
    streams.sort();
    // The highest copy is the current one, as for the older name readers: a
    // renamed type keeps its old name in the partitions below. Two blocks of
    // the highest partition that name the type differently cancel.
    let mut best: BTreeMap<u32, (u64, String, usize, Result<TypeNameRead, NoName>)> =
        BTreeMap::new();
    for (number, stream) in &streams {
        let Ok(inflated) = rf.inflated_partition(stream) else {
            continue;
        };
        let buf = inflated.bytes();
        for (id, id_at) in find_id_objects(buf, wanted) {
            if !grammar.is_block(buf, id_at, id) {
                continue;
            }
            let result = match tags.get(&id) {
                Some(&tag) => grammar.read(buf, id_at, id, tag),
                None => Err(NoName::UnknownClass),
            };
            match best.get_mut(&id) {
                Some((held, _, _, kept)) if held == number => {
                    let same = matches!((&*kept, &result), (Ok(a), Ok(b)) if a.name == b.name);
                    if !same {
                        *kept = Err(NoName::Conflict);
                    }
                }
                _ => {
                    best.insert(id, (*number, stream.clone(), id_at, result));
                }
            }
        }
    }
    let mut ok = BTreeMap::new();
    let mut no = BTreeMap::new();
    for (id, (_, stream, id_at, result)) in best {
        match result {
            Ok(read) => {
                ok.insert(
                    id,
                    ReadAt {
                        stream,
                        id_at,
                        read,
                    },
                );
            }
            Err(why) => {
                no.insert(id, why);
            }
        }
    }
    Ok((ok, no))
}
