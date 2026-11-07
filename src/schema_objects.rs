//! Every serialized object of a partition read through the file's own
//! schema: the schema-driven frontend of the reader.
//!
//! # Framing
//!
//! A partition stream is mostly *data objects*, each
//!
//! ```text
//! id · u32 Adler-32 · u32 size · payload (size bytes) · u32 size
//! ```
//!
//! where `id` is the object's ElementId, a `u64` from Revit 2024 on
//! ([`IdWidth::Wide`]) and a `u32` before ([`IdWidth::Narrow`],
//! measured on 2023), and the Adler-32 covers the payload. The payload opens
//! with the object's class as a `u16` schema tag
//! ([`crate::formats::schema_layouts`]). On `rac_basic_sample_project` the
//! objects cover 84.3 % of the partition bytes on Revit 2024 (97,401 objects)
//! and 89.1 % on 2023 (170,914); the rest are id tables and the element-record
//! table, kept as [`PartitionRead::gaps`].
//!
//! # Payload
//!
//! The rest of the payload is the object written field by field in schema
//! order, base class first:
//!
//! | schema type | wire |
//! |---|---|
//! | bool, kind 2 | 1 byte |
//! | kind 3 | 2 bytes |
//! | u32 / i32 (kinds 4, 5), f32 (6) | 4 bytes |
//! | f64 (7), u64 / i64 (0x0b) | 8 bytes |
//! | GUID (9) | 16 bytes |
//! | string (8) | `u32 n` · UTF-16LE × n |
//! | object held by value | its own fields, same rules (`ElementId` is its `Identifier`, 4 or 8 bytes by the schema) |
//! | fixed array (modifier 0x1_) | `count` × element |
//! | container (modifier 0x5_) | `u32 n` · n × element |
//! | pointer (modifier 1, 2) | `u32 0` (null), or `u32 marker · u16 tag` |
//! | document reference (modifier 3) | `u32` |
//!
//! A pointer whose marker is `ffffffff` introduces a new object of class
//! `tag`. Any other marker is an object number: the first pointer with a
//! number introduces that object, later ones refer to it. Introduced objects
//! are not inline: they follow the object that introduced them, breadth first
//! (the root's pointees in field order, then theirs).
//!
//! # Fail closed, nothing dropped
//!
//! An object is read until a type the reader does not know, or bytes that do
//! not fit, stop it. What was read before the stop is kept, the stop is
//! reported, and the unread bytes of the payload are kept as a preservation
//! record ([`SchemaObject::unparsed`]) owned by the object. Every object and
//! field carries its byte span.

use crate::formats::{SchemaFieldType, SchemaLayouts};
use crate::{Result, RevitFile};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

/// First release whose object ids are 64-bit on the wire.
pub const FIRST_WIDE_ID_RELEASE: u32 = 2024;

/// Tag of `ElementId`.
const ELEMENT_ID_TAG: u16 = 20;
/// Most elements a container may hold before the read is refused.
const MAX_COUNT: usize = 1 << 24;
/// Deepest by-value nesting followed.
const MAX_DEPTH: usize = 48;
/// Most objects one data object may introduce.
const MAX_OBJECTS: usize = 1 << 16;

/// Queue tag for an Extensible Storage entity payload. `ESEntity.m_blob`
/// is written as a pointer marker followed by the entity's 16-byte schema GUID
/// (not a u16 class tag), and the payload it introduces is in Extensible
/// Storage's own format, which `Formats/Latest` does not describe: it is kept
/// as bytes ([`Value::Bytes`]) and must be the last object of its frame.
pub const ES_BLOB_TAG: u16 = u16::MAX;

/// How an object header writes its id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IdWidth {
    /// `u32` (before Revit 2024).
    Narrow,
    /// `u64` (Revit 2024 and later).
    Wide,
}

impl IdWidth {
    /// The width a release writes.
    pub fn of_release(version: u32) -> Self {
        if version >= FIRST_WIDE_ID_RELEASE {
            IdWidth::Wide
        } else {
            IdWidth::Narrow
        }
    }

    fn bytes(self) -> usize {
        match self {
            IdWidth::Narrow => 4,
            IdWidth::Wide => 8,
        }
    }
}

/// A data object's header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectHeader {
    /// The object's ElementId.
    pub id: u64,
    pub width: IdWidth,
    /// Adler-32 of the payload.
    pub adler: u32,
    /// Payload bytes.
    pub size: u32,
    /// Schema tag of the root class (the payload's first two bytes).
    pub tag: u16,
}

/// A located data object: its header and spans in the inflated stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub header: ObjectHeader,
    /// Whole object, header to trailing size.
    pub span: Range<usize>,
    /// Payload (class tag first), without the trailing size.
    pub payload: Range<usize>,
}

fn u32_at(buf: &[u8], at: usize) -> Option<u32> {
    buf.get(at..at.checked_add(4)?)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// The data object whose header starts at `p`, if its trailing size and
/// Adler-32 verify.
pub fn frame_at(buf: &[u8], p: usize, width: IdWidth) -> Option<Frame> {
    let frame = frame_unchecked(buf, p, width)?;
    let mut sum = simd_adler32::Adler32::new();
    sum.write(&buf[frame.payload.clone()]);
    (sum.finish() == frame.header.adler).then_some(frame)
}

/// [`frame_at`] without the Adler-32: for offsets a scan already verified.
fn frame_unchecked(buf: &[u8], p: usize, width: IdWidth) -> Option<Frame> {
    let w = width.bytes();
    let id_lo = u32_at(buf, p)?;
    if width == IdWidth::Wide && u32_at(buf, p + 4)? != 0 {
        return None;
    }
    let adler = u32_at(buf, p + w)?;
    let size = u32_at(buf, p + w + 4)?;
    let start = p + w + 8;
    let end = start.checked_add(size as usize)?;
    if size < 2 || u32_at(buf, end)? != size {
        return None;
    }
    Some(Frame {
        header: ObjectHeader {
            id: u64::from(id_lo),
            width,
            adler,
            size,
            tag: u16::from_le_bytes([buf[start], buf[start + 1]]),
        },
        span: p..end + 4,
        payload: start..end,
    })
}

/// Every data object of `buf` in one pass, and the byte ranges between them.
pub fn frames(buf: &[u8], width: IdWidth) -> (Vec<Frame>, Vec<Range<usize>>) {
    let w = width.bytes();
    let (mut out, mut gaps) = (Vec::new(), Vec::new());
    let (mut p, mut gap_start) = (0, 0);
    while p + w + 8 <= buf.len() {
        // Cheap check before the checksum: the trailing size must agree.
        let plausible = u32_at(buf, p + w + 4).is_some_and(|size| {
            (p + w + 8)
                .checked_add(size as usize)
                .and_then(|end| u32_at(buf, end))
                == Some(size)
        });
        match plausible.then(|| frame_at(buf, p, width)).flatten() {
            Some(frame) => {
                if gap_start < p {
                    gaps.push(gap_start..p);
                }
                p = frame.span.end;
                gap_start = p;
                out.push(frame);
            }
            None => p += 1,
        }
    }
    if gap_start < buf.len() {
        gaps.push(gap_start..buf.len());
    }
    (out, gaps)
}

/// [`frames`] of partition `name` (inflated bytes `buf`). The first scan of a
/// partition sums every object's Adler-32; it keeps the header offsets it
/// verified ([`RevitFile`]), so the many later passes over the same partition
/// rebuild the same frames and gaps from them without summing again.
fn partition_frames(
    rf: &mut RevitFile,
    name: &str,
    buf: &[u8],
    width: IdWidth,
) -> (Vec<Frame>, Vec<Range<usize>>) {
    if let Some(starts) = rf.frame_starts(name, width) {
        let found: Vec<Frame> = starts
            .iter()
            .filter_map(|&p| frame_unchecked(buf, p, width))
            .collect();
        let (mut gaps, mut gap_start) = (Vec::new(), 0);
        for frame in &found {
            if gap_start < frame.span.start {
                gaps.push(gap_start..frame.span.start);
            }
            gap_start = frame.span.end;
        }
        if gap_start < buf.len() {
            gaps.push(gap_start..buf.len());
        }
        return (found, gaps);
    }
    let (found, gaps) = frames(buf, width);
    rf.keep_frame_starts(name, width, found.iter().map(|f| f.span.start).collect());
    (found, gaps)
}

/// One field value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
    Guid([u8; 16]),
    /// An `ElementId`, 64-bit whatever its wire width.
    Id(i64),
    /// A null pointer.
    Null,
    /// A pointer that introduces an object of class `tag`, at `index` of
    /// [`SchemaObject::objects`]; `marker` is `ffffffff` or its number.
    New {
        tag: u16,
        index: usize,
        marker: u32,
    },
    /// A pointer to an object introduced elsewhere.
    Ref {
        marker: u32,
        tag: u16,
    },
    /// A document reference.
    DocRef(u32),
    /// `ESEntity.m_blob`: a pointer to an Extensible Storage payload, with
    /// its schema GUID; the payload is object `index` (class [`ES_BLOB_TAG`]).
    EsPtr {
        marker: u32,
        schema: [u8; 16],
        index: usize,
    },
    /// Bytes kept as they are (the Extensible Storage payload).
    Bytes(Vec<u8>),
    /// An object held by value.
    Object(Fields),
    /// An array or container.
    List(Vec<Value>),
}

/// The fields of one object, chain base first, in schema order, each with
/// its byte span in the inflated stream.
#[derive(Debug, Clone, PartialEq)]
pub struct Fields {
    pub tag: u16,
    pub span: Range<usize>,
    pub values: Vec<(String, Value)>,
    pub spans: Vec<Range<usize>>,
}

impl Fields {
    /// The first field called `name`.
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.values.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    fn id(&self, name: &str) -> Option<i64> {
        match self.get(name)? {
            Value::Id(id) => Some(*id),
            _ => None,
        }
    }
}

/// A data object read by the schema.
#[derive(Debug, Clone, PartialEq)]
pub struct SchemaObject {
    pub stream: String,
    pub frame: Frame,
    /// The root object, then every object it introduced, in wire order.
    pub objects: Vec<Fields>,
    /// True when every payload byte was read.
    pub exact: bool,
    /// Why reading stopped early, `None` when it did not.
    pub stopped: Option<String>,
    /// Payload bytes not read (preservation record owned by this object).
    pub unparsed: Option<Range<usize>>,
}

/// One stored parameter value of an element.
#[derive(Debug, Clone, PartialEq)]
pub enum ParamValue {
    Double(f64),
    Integer(i64),
    String(String),
    ElementId(i64),
}

impl SchemaObject {
    /// The root object.
    pub fn root(&self) -> Option<&Fields> {
        self.objects.first()
    }

    /// The root's ElementId when its class derives from `Element` and its
    /// `m_id` equals the header's id; `None` otherwise.
    pub fn element(&self) -> Option<i64> {
        let id = self.root()?.id("m_id")?;
        (u64::try_from(id).ok() == Some(self.frame.header.id)).then_some(id)
    }

    /// The stored parameters: the entries of the root's `m_pParamValueSet*`
    /// objects, parameter first (a BuiltInParameter, or a project or shared
    /// parameter's ElementId). Empty unless [`Self::element`].
    pub fn stored_parameters(&self) -> Vec<(i64, ParamValue)> {
        let mut out = Vec::new();
        let Some(root) = self.root().filter(|_| self.element().is_some()) else {
            return out;
        };
        for field in [
            "m_pParamValueSetDouble",
            "m_pParamValueSetInt",
            "m_pParamValueSetAString",
            "m_pParamValueSetElementId",
        ] {
            let Some(Value::New { index, .. }) = root.get(field) else {
                continue;
            };
            let Some(Value::List(entries)) =
                self.objects.get(*index).and_then(|o| o.get("m_paramSet"))
            else {
                continue;
            };
            for entry in entries {
                let Value::Object(e) = entry else { continue };
                let (Some(param), Some(value)) = (e.id("m_paramId"), e.get("m_value")) else {
                    continue;
                };
                let value = match value {
                    Value::Float(v) => ParamValue::Double(*v),
                    Value::Int(v) => ParamValue::Integer(*v),
                    Value::Text(v) => ParamValue::String(v.clone()),
                    Value::Id(v) => ParamValue::ElementId(*v),
                    _ => continue,
                };
                out.push((param, value));
            }
        }
        out
    }
}

struct Reader<'a> {
    layouts: &'a SchemaLayouts,
    buf: &'a [u8],
    at: usize,
    end: usize,
    queue: Vec<u16>,
    numbered: BTreeSet<u32>,
}

type R<T> = std::result::Result<T, String>;

impl Reader<'_> {
    fn take(&mut self, n: usize) -> R<&[u8]> {
        let end = self.at.checked_add(n).filter(|e| *e <= self.end);
        let end = end.ok_or_else(|| format!("{n} bytes past the payload at {:#x}", self.at))?;
        let out = &self.buf[self.at..end];
        self.at = end;
        Ok(out)
    }

    fn u32(&mut self) -> R<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u16(&mut self) -> R<u16> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn i64(&mut self) -> R<i64> {
        let b = self.take(8)?;
        Ok(i64::from_le_bytes(b.try_into().expect("8 bytes")))
    }

    fn count(&mut self) -> R<usize> {
        let n = self.u32()? as usize;
        // Every element takes at least one byte except empty classes, so a
        // count larger than what is left is a misread.
        if n > MAX_COUNT || n > (self.end - self.at).max(64) {
            return Err(format!("count {n} at {:#x}", self.at - 4));
        }
        Ok(n)
    }

    fn scalar(&mut self, kind: u8) -> R<Value> {
        Ok(match kind {
            1 => Value::Bool(self.take(1)?[0] != 0),
            2 => Value::Int(i64::from(self.take(1)?[0])),
            3 => Value::Int(i64::from(self.u16()?)),
            4 => Value::Int(i64::from(self.u32()? as i32)),
            5 => Value::Int(i64::from(self.u32()?)),
            6 => Value::Float(f64::from(f32::from_bits(self.u32()?))),
            7 => Value::Float(f64::from_bits(self.i64()? as u64)),
            8 => {
                let n = self.count()?;
                let raw = self.take(2 * n)?;
                let units: Vec<u16> = raw
                    .chunks_exact(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .collect();
                Value::Text(String::from_utf16_lossy(&units))
            }
            9 => Value::Guid(self.take(16)?.try_into().expect("16 bytes")),
            0x0b => Value::Int(self.i64()?),
            k => return Err(format!("scalar kind {k:#04x}")),
        })
    }

    fn es_pointer(&mut self) -> R<Value> {
        let marker = self.u32()?;
        if marker == 0 {
            return Ok(Value::Null);
        }
        let schema: [u8; 16] = self.take(16)?.try_into().expect("16 bytes");
        if self.queue.len() >= MAX_OBJECTS {
            return Err("too many objects".into());
        }
        self.queue.push(ES_BLOB_TAG);
        Ok(Value::EsPtr {
            marker,
            schema,
            index: self.queue.len(),
        })
    }

    fn pointer(&mut self, modifier: u8) -> R<Value> {
        if modifier & 0x0f == 3 {
            return Ok(Value::DocRef(self.u32()?));
        }
        let marker = self.u32()?;
        if marker == 0 {
            return Ok(Value::Null);
        }
        let tag = self.u16()?;
        // A numbered object is written once, where it is first met; later
        // pointers with its number refer to it.
        if marker != u32::MAX && !self.numbered.insert(marker) {
            return Ok(Value::Ref { marker, tag });
        }
        if self.queue.len() >= MAX_OBJECTS {
            return Err("too many objects".into());
        }
        self.queue.push(tag);
        Ok(Value::New {
            tag,
            index: self.queue.len(),
            marker,
        })
    }

    /// `one` once, `count` times, or `u32 n` times, by the modifier's high nibble.
    fn repeat(
        &mut self,
        modifier: u8,
        count: Option<u32>,
        one: &mut dyn FnMut(&mut Self) -> R<Value>,
    ) -> R<Value> {
        let n = match modifier & 0xf0 {
            0x00 | 0x60 => return one(self),
            0x10 => count.unwrap_or(0) as usize,
            0x50 => self.count()?,
            m => return Err(format!("modifier {m:#04x}")),
        };
        (0..n)
            .map(|_| one(self))
            .collect::<R<Vec<_>>>()
            .map(Value::List)
    }

    fn element_id(&mut self, depth: usize) -> R<Value> {
        let fields = self.fields(ELEMENT_ID_TAG, depth + 1)?;
        // ElementId { Identifier m_id { integer m_id(64) } }
        let mut v = fields.values.first().map(|(_, v)| v.clone());
        while let Some(Value::Object(inner)) = v {
            v = inner.values.first().map(|(_, v)| v.clone());
        }
        match v {
            Some(Value::Int(id)) => Ok(Value::Id(id)),
            _ => Err("ElementId is not an integer".into()),
        }
    }

    fn value(&mut self, ty: &SchemaFieldType, depth: usize) -> R<Value> {
        if depth > MAX_DEPTH {
            return Err("nesting too deep".into());
        }
        match ty {
            SchemaFieldType::Scalar { kind: 8, .. } => self.scalar(8),
            SchemaFieldType::Scalar {
                kind,
                modifier,
                count,
            } => self.repeat(*modifier, *count, &mut |r| r.scalar(*kind)),
            SchemaFieldType::Object {
                tag,
                modifier,
                count,
            } => self.repeat(*modifier, *count, &mut |r| {
                if *tag == ELEMENT_ID_TAG {
                    r.element_id(depth)
                } else {
                    r.fields(*tag, depth + 1).map(Value::Object)
                }
            }),
            SchemaFieldType::Pointer { modifier, count } => {
                self.repeat(*modifier, *count, &mut |r| r.pointer(*modifier))
            }
            SchemaFieldType::Array {
                modifier,
                count,
                inner,
                ..
            } => self.repeat(*modifier, *count, &mut |r| r.value(inner, depth + 1)),
        }
    }

    fn fields(&mut self, tag: u16, depth: usize) -> R<Fields> {
        let chain = self
            .layouts
            .chain(tag)
            .ok_or_else(|| format!("no class {tag}"))?;
        let start = self.at;
        let (mut values, mut spans) = (Vec::new(), Vec::new());
        for class in chain {
            for field in class.fields.iter().filter(|f| f.is_written()) {
                let at = self.at;
                let value = if class.name == "ESEntity" && field.name == "m_blob" {
                    self.es_pointer()
                } else {
                    self.value(&field.ty, depth)
                }
                .map_err(|e| format!("{}.{}: {e}", class.name, field.name))?;
                values.push((field.name.clone(), value));
                spans.push(at..self.at);
            }
        }
        Ok(Fields {
            tag,
            span: start..self.at,
            values,
            spans,
        })
    }
}

/// Read one located data object by the schema.
pub fn read_frame(layouts: &SchemaLayouts, stream: &str, buf: &[u8], frame: Frame) -> SchemaObject {
    let mut r = Reader {
        layouts,
        buf,
        at: frame.payload.start + 2,
        end: frame.payload.end,
        queue: Vec::new(),
        numbered: BTreeSet::new(),
    };
    let mut out = SchemaObject {
        stream: stream.to_string(),
        frame,
        objects: Vec::new(),
        exact: false,
        stopped: None,
        unparsed: None,
    };
    let mut next = Some(out.frame.header.tag);
    let mut k = 0;
    while let Some(tag) = next {
        let read = if tag == ES_BLOB_TAG {
            if r.queue.len() == k {
                let start = r.at;
                let bytes = r.buf[start..r.end].to_vec();
                r.at = r.end;
                Ok(Fields {
                    tag,
                    span: start..r.end,
                    values: vec![("blob".to_string(), Value::Bytes(bytes))],
                    #[allow(clippy::single_range_in_vec_init)] // one span per value, there is one value
                    spans: vec![start..r.end],
                })
            } else {
                Err("Extensible Storage payload is not the last object".to_string())
            }
        } else {
            r.fields(tag, 0)
        };
        match read {
            Ok(fields) => out.objects.push(fields),
            Err(e) => {
                out.stopped = Some(e);
                break;
            }
        }
        next = r.queue.get(k).copied();
        k += 1;
    }
    // On a stop, `r.at` is wherever the failed field left it: the unread
    // part starts after the last object read in full.
    let read_to = match (&out.stopped, out.objects.last()) {
        (None, _) => r.at,
        (Some(_), Some(last)) => last.span.end,
        (Some(_), None) => out.frame.payload.start + 2,
    };
    out.exact = out.stopped.is_none() && read_to == out.frame.payload.end;
    if read_to < out.frame.payload.end {
        out.unparsed = Some(read_to..out.frame.payload.end);
        if out.stopped.is_none() {
            out.stopped = Some(format!(
                "{} bytes not described by the schema",
                out.frame.payload.end - read_to
            ));
        }
    }
    out
}

/// A partition's objects and the bytes between them.
#[derive(Debug, Clone, PartialEq)]
pub struct PartitionRead {
    pub stream: String,
    pub objects: Vec<SchemaObject>,
    /// Byte ranges outside every data object: preservation records owned by
    /// the partition.
    pub gaps: Vec<Range<usize>>,
}

/// Call `visit` with every data object of every partition of `rf`, read by
/// its schema, and return each partition's gaps. At most 4,096
/// objects are held at a time, so memory stays bounded by the largest
/// partition.
pub fn for_each_object(
    rf: &mut RevitFile,
    mut visit: impl FnMut(&SchemaObject),
) -> Result<Vec<(String, Vec<Range<usize>>)>> {
    let version = rf.basic_file_info().map(|b| b.version).unwrap_or(0);
    let width = IdWidth::of_release(version);
    let layouts = rf.schema_layouts()?;
    let mut out = Vec::new();
    for name in rf.stream_names() {
        if !name.starts_with("Partitions/") {
            continue;
        }
        let inflated = rf.inflated_partition(&name)?;
        let buf = inflated.bytes();
        let (found, gaps) = partition_frames(rf, &name, buf, width);
        read_frames_in_order(&layouts, &name, buf, &found, &mut visit);
        out.push((name, gaps));
    }
    Ok(out)
}

/// Data objects [`read_frames_in_order`] reads at once: enough to keep every
/// core busy, few enough that memory stays bounded by a slice of a partition.
const READ_CHUNK: usize = 4096;

/// `visit` each of `frames` read by its schema, in order. The frames are read
/// a chunk at a time on every core ([`crate::control::map_in_order`]), so
/// what `visit` sees and in what order does not depend on the thread count.
fn read_frames_in_order(
    layouts: &SchemaLayouts,
    stream: &str,
    buf: &[u8],
    frames: &[Frame],
    visit: &mut impl FnMut(&SchemaObject),
) {
    for chunk in frames.chunks(READ_CHUNK) {
        let objects = crate::control::map_in_order(chunk, |frame| {
            read_frame(layouts, stream, buf, frame.clone())
        });
        for object in &objects {
            visit(object);
        }
    }
}

/// [`for_each_object`] for the data objects `keep` accepts, given each
/// frame and the inflated stream it lies in; the others are located but not
/// read (a class or a byte pattern picks a few objects of many).
pub fn for_each_object_where(
    rf: &mut RevitFile,
    mut keep: impl FnMut(&Frame, &[u8]) -> bool,
    mut visit: impl FnMut(&SchemaObject),
) -> Result<()> {
    let version = rf.basic_file_info().map(|b| b.version).unwrap_or(0);
    let width = IdWidth::of_release(version);
    let layouts = rf.schema_layouts()?;
    for name in rf.stream_names() {
        if !name.starts_with("Partitions/") {
            continue;
        }
        let inflated = rf.inflated_partition(&name)?;
        let buf = inflated.bytes();
        let (mut found, _) = partition_frames(rf, &name, buf, width);
        found.retain(|frame| keep(frame, buf));
        read_frames_in_order(&layouts, &name, buf, &found, &mut visit);
    }
    Ok(())
}

/// Inflated bytes [`frames_streamed`] keeps ahead of its scan position. An
/// object whose end lies further ahead is checked by re-inflating the
/// members at its end (its trailing size), then its payload (Adler-32), and
/// only a verified object widens the window. 32 MiB: on DACH 2024 that leaves
/// about 294,000 such checks of 107 million positions scanned.
pub const STREAM_AHEAD: usize = 32 << 20;

/// [`frames`] over a [`StreamedPartition`](crate::compression::StreamedPartition) read member by member, holding a
/// window of `ahead` inflated bytes (plus the largest object) instead of the
/// stream: the same frames in the same order and the same gaps. `visit` gets
/// each frame with spans relative to the window it is handed and the
/// window's offset in the stream. Returns the gaps (stream offsets).
pub fn frames_streamed(
    part: &mut crate::compression::StreamedPartition,
    width: IdWidth,
    ahead: usize,
    mut visit: impl FnMut(&Frame, &[u8], usize),
) -> Result<Vec<Range<usize>>> {
    let w = width.bytes();
    let len = part.len();
    let (mut window, mut base, mut next) = (Vec::<u8>::new(), 0usize, 0usize);
    let mut gaps = Vec::new();
    let (mut p, mut gap_start) = (0usize, 0usize);
    // Append members until the window reaches `to` (or the stream's end).
    let fill = |window: &mut Vec<u8>,
                base: usize,
                next: &mut usize,
                part: &mut crate::compression::StreamedPartition,
                to: usize|
     -> Result<()> {
        while base + window.len() < to && *next < part.index.members.len() {
            part.read_member(*next, window)?;
            *next += 1;
        }
        Ok(())
    };
    while p + w + 8 <= len {
        if p - base >= (ahead / 4).max(1 << 20) {
            window.drain(..p - base);
            base = p;
        }
        fill(
            &mut window,
            base,
            &mut next,
            part,
            p.saturating_add(ahead).max(p + w + 8),
        )?;
        let lp = p - base;
        let size = u32_at(&window, lp + w + 4).unwrap_or(0) as usize;
        let end = (p + w + 8).checked_add(size);
        let found = match end {
            None => None,
            Some(end) if end + 4 <= base + window.len() => frame_at(&window, lp, width),
            Some(end) if end + 4 > len => None,
            Some(end) => {
                // Far end: the cheap conditions of `frame_at` first, then the
                // trailing size and the checksum read from the members.
                let wide_ok = width == IdWidth::Narrow || u32_at(&window, lp + 4) == Some(0);
                if size < 2 || !wide_ok {
                    None
                } else {
                    let mut tail = Vec::with_capacity(4);
                    part.for_range(end..end + 4, |b| tail.extend_from_slice(b))?;
                    if u32_at(&tail, 0) != Some(size as u32) {
                        None
                    } else {
                        let mut sum = simd_adler32::Adler32::new();
                        part.for_range(p + w + 8..end, |b| sum.write(b))?;
                        if Some(sum.finish()) != u32_at(&window, lp + w) {
                            None
                        } else {
                            fill(&mut window, base, &mut next, part, end + 4)?;
                            frame_at(&window, lp, width)
                        }
                    }
                }
            }
        };
        match found {
            Some(frame) => {
                if gap_start < p {
                    gaps.push(gap_start..p);
                }
                p = base + frame.span.end;
                gap_start = p;
                visit(&frame, &window, base);
            }
            None => p += 1,
        }
    }
    if gap_start < len {
        gaps.push(gap_start..len);
    }
    Ok(gaps)
}

/// Shift every span of `o` by `by` (window offsets to stream offsets).
fn rebase(o: &mut SchemaObject, by: usize) {
    fn fields(f: &mut Fields, by: usize) {
        f.span = f.span.start + by..f.span.end + by;
        for s in &mut f.spans {
            *s = s.start + by..s.end + by;
        }
        for (_, v) in &mut f.values {
            value(v, by);
        }
    }
    fn value(v: &mut Value, by: usize) {
        match v {
            Value::Object(f) => fields(f, by),
            Value::List(items) => items.iter_mut().for_each(|v| value(v, by)),
            _ => {}
        }
    }
    let shift = |r: &mut Range<usize>| *r = r.start + by..r.end + by;
    shift(&mut o.frame.span);
    shift(&mut o.frame.payload);
    if let Some(r) = o.unparsed.as_mut() {
        shift(r);
    }
    o.objects.iter_mut().for_each(|f| fields(f, by));
}

/// [`for_each_object_where`] reading each partition member by member
/// ([`crate::reader::RevitFile::streamed_partition`], [`frames_streamed`]):
/// the same objects with the same (stream) spans, in the same order, without
/// inflating a partition whole or keeping it. `keep` sees each frame with
/// spans relative to the window it is handed. Returns each partition's gaps.
pub fn for_each_object_where_streamed(
    rf: &mut RevitFile,
    mut keep: impl FnMut(&Frame, &[u8]) -> bool,
    mut visit: impl FnMut(&SchemaObject),
) -> Result<Vec<(String, Vec<Range<usize>>)>> {
    let version = rf.basic_file_info().map(|b| b.version).unwrap_or(0);
    let width = IdWidth::of_release(version);
    let layouts = rf.schema_layouts()?;
    let mut out = Vec::new();
    for name in rf.stream_names() {
        if !name.starts_with("Partitions/") {
            continue;
        }
        let mut part = rf.streamed_partition(&name)?;
        let gaps = frames_streamed(&mut part, width, STREAM_AHEAD, |frame, window, base| {
            if keep(frame, window) {
                let mut object = read_frame(&layouts, &name, window, frame.clone());
                rebase(&mut object, base);
                visit(&object);
            }
        })?;
        out.push((name, gaps));
    }
    Ok(out)
}

/// [`for_each_object`], streamed ([`for_each_object_where_streamed`]).
pub fn for_each_object_streamed(
    rf: &mut RevitFile,
    visit: impl FnMut(&SchemaObject),
) -> Result<Vec<(String, Vec<Range<usize>>)>> {
    for_each_object_where_streamed(rf, |_, _| true, visit)
}

/// Every partition of `rf`, every data object read by its schema. Holds every
/// object of the file; prefer [`for_each_object`] on large files.
pub fn read_partitions(rf: &mut RevitFile) -> Result<Vec<PartitionRead>> {
    let mut objects: BTreeMap<String, Vec<SchemaObject>> = BTreeMap::new();
    let gaps = for_each_object(rf, |o| {
        objects.entry(o.stream.clone()).or_default().push(o.clone())
    })?;
    Ok(gaps
        .into_iter()
        .map(|(stream, gaps)| PartitionRead {
            objects: objects.remove(&stream).unwrap_or_default(),
            stream,
            gaps,
        })
        .collect())
}

/// Every data object of every partition of `rf`, read by its schema.
pub fn read_objects(rf: &mut RevitFile) -> Result<Vec<SchemaObject>> {
    Ok(read_partitions(rf)?
        .into_iter()
        .flat_map(|p| p.objects)
        .collect())
}

/// Note carried by a stored parameter Revit recalculates (reported, marked,
/// not dropped).
pub const RECALCULATED_NOTE: &str = "stored, Revit recalculates";

/// BuiltInParameter `WALL_USER_HEIGHT_PARAM`, a wall's Unconnected Height.
pub const WALL_UNCONNECTED_HEIGHT: i64 = -1_001_105;

/// The stored parameters of an element whose stored value is not what Revit
/// reports, because Revit recalculates it. One rule, measured on the 102
/// API-keyed corpus models: a wall with a top constraint
/// (`m_upToLevelId` set) keeps an old Unconnected Height; on all 19,822
/// such walls whose stored value differs from the API, and on no
/// unconnected wall, the API's value is the constrained height.
pub fn recalculated_parameters(
    fields: &Fields,
    parameters: &BTreeMap<i64, ParamValue>,
) -> BTreeSet<i64> {
    let mut out = BTreeSet::new();
    let constrained = matches!(fields.get("m_upToLevelId"), Some(Value::Id(id)) if *id != -1);
    if constrained && parameters.contains_key(&WALL_UNCONNECTED_HEIGHT) {
        out.insert(WALL_UNCONNECTED_HEIGHT);
    }
    out
}

/// One element read by the schema: its class and stored parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct SchemaElement {
    pub id: i64,
    /// Schema class of the element object.
    pub class: String,
    /// The root object's fields.
    pub fields: Fields,
    /// Stored parameters by parameter id. A parameter stored twice with
    /// different values is left out.
    pub parameters: BTreeMap<i64, ParamValue>,
    /// Whether the whole data object was read.
    pub exact: bool,
    /// Partition stream and header offset of its data object.
    pub stream: String,
    pub offset: usize,
    /// Stored parameters Revit recalculates ([`recalculated_parameters`]):
    /// kept in `parameters`, marked here.
    pub recalculated: BTreeSet<i64>,
}

/// Every element of `rf` whose data object's root reads with its own id
/// ([`SchemaObject::element`]). An id held by two objects that disagree on
/// class is left out.
pub fn read_elements(rf: &mut RevitFile) -> Result<BTreeMap<i64, SchemaElement>> {
    let layouts = rf.schema_layouts()?;
    let mut out: BTreeMap<i64, SchemaElement> = BTreeMap::new();
    let mut dropped = BTreeSet::new();
    for_each_object(rf, |object| {
        let Some(id) = object.element() else { return };
        let root = object.root().expect("element has a root").clone();
        let class = layouts
            .classes
            .get(&root.tag)
            .map(|c| c.name.clone())
            .unwrap_or_default();
        let mut parameters = BTreeMap::new();
        let mut twice = Vec::new();
        for (p, v) in object.stored_parameters() {
            if let Some(old) = parameters.insert(p, v.clone()) {
                if old != v {
                    twice.push(p);
                }
            }
        }
        for p in twice {
            parameters.remove(&p);
        }
        let recalculated = recalculated_parameters(&root, &parameters);
        let element = SchemaElement {
            recalculated,
            id,
            class,
            fields: root,
            parameters,
            exact: object.exact,
            stream: object.stream.clone(),
            offset: object.frame.span.start,
        };
        match out.get(&id) {
            Some(old) if old.class != element.class => {
                dropped.insert(id);
            }
            Some(old) if old.exact && !element.exact => {}
            _ => {
                out.insert(id, element);
            }
        }
    })?;
    for id in dropped {
        out.remove(&id);
    }
    Ok(out)
}
