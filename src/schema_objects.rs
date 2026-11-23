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
//! | kind 3; class reference (0x0a), a schema class tag | 2 bytes |
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

/// `PHY_MATERIAL_PARAM_STRUCTURAL_DENSITY` and `PHY_MATERIAL_PARAM_UNIT_WEIGHT`.
const PHY_MATERIAL_PARAM_STRUCTURAL_DENSITY: i64 = -1_152_313;
const PHY_MATERIAL_PARAM_UNIT_WEIGHT: i64 = -1_140_309;

/// `TEXT_STYLE_BOLD`, `TEXT_STYLE_ITALIC` and `TEXT_STYLE_UNDERLINE` of dimension and spot
/// elevation styles: the file stores each of them as an integer in the Int set of some
/// elements (188,807 entries each on the 101 public sample files) and as a whole-number float in
/// the Double set of others (148,932 entries each, none with a fraction), and no other parameter
/// is kept in both. Their value is an integer either way (the flag fields beside them, such as
/// `m_bBold`, are flags), so an entry of the Double set is read as the integer it holds.
const INTEGER_IN_EITHER_SET: [i64; 3] = [-1_006_313, -1_006_312, -1_006_311];

/// A parameter value set entry's value as a [`ParamValue`]: the kind the entry stores, but
/// an [`INTEGER_IN_EITHER_SET`] parameter's whole-number float as an integer.
fn entry_value(param: i64, value: &Value) -> Option<ParamValue> {
    Some(match value {
        Value::Float(v) if INTEGER_IN_EITHER_SET.contains(&param) && v.fract() == 0.0 => {
            ParamValue::Integer(*v as i64)
        }
        Value::Float(v) => ParamValue::Double(*v),
        Value::Int(v) => ParamValue::Integer(*v),
        Value::Text(v) => ParamValue::String(v.clone()),
        Value::Id(v) => ParamValue::ElementId(*v),
        _ => return None,
    })
}
/// Standard gravity, 9.80665 m/s^2, in ft/s^2 (Revit's internal length unit).
const STANDARD_GRAVITY_FT_S2: f64 = 9.80665 / 0.3048;

/// First release whose object ids are 64-bit on the wire.
pub const FIRST_WIDE_ID_RELEASE: u32 = 2024;

/// Tag of `ElementId` from Revit 2012 on. The class is found by its name:
/// earlier schemas give it tag 19 (and tag 20 to `Identifier`, the class it
/// holds), so a fixed tag read every id of a Revit 2008 to 2011 file as an
/// object wrapping the id.
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
                let Some(value) = entry_value(param, value) else {
                    continue;
                };
                out.push((param, value));
            }
        }
        out
    }

    /// Parameters kept in parameter sets the element's other objects own
    /// (a property set element's `m_oParamSet`, ...): every `m_paramSet`
    /// entry with a parameter id and a value, outside the root's own
    /// `m_pParamValueSet*` objects ([`Self::stored_parameters`]). Empty
    /// unless [`Self::element`].
    pub fn nested_parameters(&self) -> Vec<(i64, ParamValue)> {
        let mut out = Vec::new();
        let Some(root) = self.root().filter(|_| self.element().is_some()) else {
            return out;
        };
        let own: Vec<usize> = [
            "m_pParamValueSetDouble",
            "m_pParamValueSetInt",
            "m_pParamValueSetAString",
            "m_pParamValueSetElementId",
        ]
        .iter()
        .filter_map(|f| match root.get(f) {
            Some(Value::New { index, .. }) => Some(*index),
            _ => None,
        })
        .collect();
        for (k, o) in self.objects.iter().enumerate().skip(1) {
            if own.contains(&k) {
                continue;
            }
            let Some(Value::List(entries)) = o.get("m_paramSet") else {
                continue;
            };
            let set_start = out.len();
            for entry in entries {
                let Value::Object(e) = entry else { continue };
                let (Some(param), Some(value)) = (e.id("m_paramId"), e.get("m_value")) else {
                    continue;
                };
                let Some(value) = entry_value(param, value) else {
                    continue;
                };
                out.push((param, value));
            }
            unit_weight_from_density(&mut out[set_start..]);
        }
        out
    }
}

/// Revit shows a structural asset's unit weight as its density times
/// standard gravity, not the unit-weight entry the set stores (T: 43 of 45
/// property sets on 1-hip-roof, Revit 2024; the other two store no density).
/// Rewrites the unit-weight entry of one parameter set that holds both; a set
/// without a density, or without a unit-weight entry, is left as stored.
fn unit_weight_from_density(set: &mut [(i64, ParamValue)]) {
    let density = set.iter().find_map(|(p, v)| match v {
        ParamValue::Double(d) if *p == PHY_MATERIAL_PARAM_STRUCTURAL_DENSITY => Some(*d),
        _ => None,
    });
    if let Some(d) = density {
        for (p, v) in set.iter_mut() {
            if *p == PHY_MATERIAL_PARAM_UNIT_WEIGHT {
                *v = ParamValue::Double(d * STANDARD_GRAVITY_FT_S2);
            }
        }
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
            // `ClassDefinitionRef.m_ref`: the schema tag of a class. In every
            // element header of 1-hip-roof (Revit 2024) it is the class tag
            // of the element's own data object.
            0x0a => Value::Int(i64::from(self.u16()?)),
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

    /// Whether class `tag` is `ElementId`.
    fn is_element_id(&self, tag: u16) -> bool {
        self.layouts
            .classes
            .get(&tag)
            .map_or(tag == ELEMENT_ID_TAG, |c| c.name == "ElementId")
    }

    fn element_id(&mut self, tag: u16, depth: usize) -> R<Value> {
        let fields = self.fields(tag, depth + 1)?;
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
                if r.is_element_id(*tag) {
                    r.element_id(*tag, depth)
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

/// The object graph of a single-object stream (`Global/ElemTable`,
/// `Global/PartitionTable`, `Global/DocumentIncrementTable`, ...): the root
/// object whose class tag is at `start` of the inflated `buf`, then every
/// object it introduces, read as [`read_frame`] reads a data object. The
/// frame's `id` and `adler` are 0; `unparsed` holds what follows the graph.
pub fn read_graph(layouts: &SchemaLayouts, stream: &str, buf: &[u8], start: usize) -> SchemaObject {
    let end = buf.len();
    let frame = Frame {
        header: ObjectHeader {
            id: 0,
            width: IdWidth::Wide,
            adler: 0,
            size: u32::try_from(end.saturating_sub(start)).unwrap_or(u32::MAX),
            tag: buf
                .get(start..start + 2)
                .map_or(0, |b| u16::from_le_bytes([b[0], b[1]])),
        },
        span: start..end,
        payload: start..end,
    };
    if start + 2 > end {
        return SchemaObject {
            stream: stream.to_string(),
            frame,
            objects: Vec::new(),
            exact: false,
            stopped: Some("no class tag".to_string()),
            unparsed: Some(start.min(end)..end),
        };
    }
    read_frame(layouts, stream, buf, frame)
}

/// One embedded document of `Global/ContentDocuments`: its key object and
/// its document graph.
#[derive(Debug, Clone, PartialEq)]
pub struct ContentDocument {
    /// The key object (its class's fields: a pointer and the document's
    /// GUID, the GUID of its partition section).
    pub key: SchemaObject,
    /// The document's object graph (root class tag first), `size` bytes.
    pub graph: SchemaObject,
    /// The whole record: key, `u32 size`, graph, `u32 size`.
    pub span: Range<usize>,
}

/// The embedded documents of an inflated `Global/ContentDocuments`, and the
/// offset where the records end.
///
/// The stream is a run of records `key object · u32 size · graph (size
/// bytes) · u32 size`, one per embedded (family) document, then a closing key
/// object whose pointer is null and a `u32 0`. Measured on 1-hip-roof
/// (Revit 2024): 93 records (the file's 93 partition sections) fill all but
/// the closing 14 bytes, and every key and graph reads exactly.
pub fn content_documents(layouts: &SchemaLayouts, buf: &[u8]) -> (Vec<ContentDocument>, usize) {
    const STREAM: &str = "Global/ContentDocuments";
    let mut out = Vec::new();
    let mut p = 0;
    loop {
        let key = read_graph(layouts, STREAM, buf, p);
        let Some(after_key) = key.objects.last().map(|o| o.span.end) else {
            break;
        };
        let Some(size) = u32_at(buf, after_key) else {
            break;
        };
        let start = after_key + 4;
        let Some(end) = start.checked_add(size as usize) else {
            break;
        };
        if size < 2 || end + 4 > buf.len() || u32_at(buf, end) != Some(size) {
            break;
        }
        let graph = read_graph(layouts, STREAM, &buf[..end], start);
        out.push(ContentDocument {
            key,
            graph,
            span: p..end + 4,
        });
        p = end + 4;
    }
    (out, p)
}

/// The element-header records between a partition's data objects, read by
/// the schema.
///
/// The bytes between data objects ([`PartitionRead::gaps`]) hold runs of
/// element-header records, one per element:
///
/// ```text
/// id · u32 size · body (size bytes) · u32 size
/// ```
///
/// with `id` as wide or narrow as the data objects' and the body an object
/// (an `ElementHeader` on every record measured) whose class tag comes first,
/// with no Adler-32. A run starts with `FF × id width · u64 1 · u32 0` and
/// ends with `FF × id width · u64 0`. Measured on a Revit 2024 file
/// (1-hip-roof): the 189 ranges between objects, 6,145,721 bytes, are
/// 36,123 records (one per element object of the file) and these markers,
/// with no byte left over. Narrow files: the same grammar is tried, not
/// measured.
#[derive(Debug, Clone, PartialEq)]
pub struct HeaderRecords {
    /// Each record read by the schema; its frame's `adler` is 0 (none on the
    /// wire), `span` the whole record and `payload` the body.
    pub records: Vec<SchemaObject>,
    /// Run start and end markers.
    pub markers: Vec<Range<usize>>,
    /// Bytes neither a record nor a marker.
    pub rest: Vec<Range<usize>>,
}

/// The element-header records and run markers in `gap` of the inflated
/// partition `buf` ([`HeaderRecords`]).
pub fn header_records(
    layouts: &SchemaLayouts,
    stream: &str,
    buf: &[u8],
    gap: Range<usize>,
    width: IdWidth,
) -> HeaderRecords {
    let w = width.bytes();
    let mut out = HeaderRecords {
        records: Vec::new(),
        markers: Vec::new(),
        rest: Vec::new(),
    };
    let end = gap.end.min(buf.len());
    let mut p = gap.start;
    let mut rest_start = None;
    let u64_at = |at: usize| {
        buf.get(at..at + 8)
            .map(|b| u64::from_le_bytes(b.try_into().expect("8 bytes")))
    };
    while p < end {
        let marker = buf[p..end.min(p + w)].iter().all(|&b| b == 0xff) && p + w + 8 <= end;
        let taken = if marker {
            match u64_at(p + w) {
                Some(1) if p + w + 12 <= end && u32_at(buf, p + w + 8) == Some(0) => Some(w + 12),
                Some(0) => Some(w + 8),
                _ => None,
            }
            .inspect(|&n| out.markers.push(p..p + n))
        } else {
            record_at(buf, p, end, width).map(|frame| {
                let n = frame.span.len();
                out.records.push(read_frame(layouts, stream, buf, frame));
                n
            })
        };
        match taken {
            Some(n) => {
                if let Some(s) = rest_start.take() {
                    out.rest.push(s..p);
                }
                p += n;
            }
            None => {
                rest_start.get_or_insert(p);
                p += 1;
            }
        }
    }
    if let Some(s) = rest_start {
        out.rest.push(s..end);
    }
    out
}

/// The element-header record at `p` (before `end`), if its trailing size
/// repeats its size.
fn record_at(buf: &[u8], p: usize, end: usize, width: IdWidth) -> Option<Frame> {
    let w = width.bytes();
    let id_lo = u32_at(buf, p)?;
    if width == IdWidth::Wide && u32_at(buf, p + 4)? != 0 {
        return None;
    }
    let size = u32_at(buf, p + w)?;
    let start = p + w + 4;
    let stop = start.checked_add(size as usize)?;
    if size < 2 || stop + 4 > end || u32_at(buf, stop)? != size {
        return None;
    }
    Some(Frame {
        header: ObjectHeader {
            id: u64::from(id_lo),
            width,
            adler: 0,
            size,
            tag: u16::from_le_bytes([buf[start], buf[start + 1]]),
        },
        span: p..stop + 4,
        payload: start..stop,
    })
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
/// objects, and about a mebibyte of their payload, are held at a time, so
/// memory stays bounded by the largest partition.
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

/// Payload bytes [`read_frames_in_order`] reads at once, past the first
/// object of a chunk. A read object takes many times its payload, and some
/// partitions hold thousands of large objects: 4,096 of them at once raised
/// the peak memory of the properties pass by more than a gigabyte.
const READ_CHUNK_BYTES: usize = 1 << 20;

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
    let mut rest = frames;
    while !rest.is_empty() {
        let mut bytes = 0;
        let n = rest
            .iter()
            .take(READ_CHUNK)
            .take_while(|frame| {
                let fits = bytes == 0 || bytes + frame.payload.len() <= READ_CHUNK_BYTES;
                bytes += frame.payload.len();
                fits
            })
            .count();
        let (chunk, tail) = rest.split_at(n.max(1));
        rest = tail;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_weight_is_density_times_standard_gravity() {
        assert_eq!(
            crate::builtin_parameters::parameter(PHY_MATERIAL_PARAM_STRUCTURAL_DENSITY)
                .map(|p| p.enum_name),
            Some("PHY_MATERIAL_PARAM_STRUCTURAL_DENSITY")
        );
        assert_eq!(
            crate::builtin_parameters::parameter(PHY_MATERIAL_PARAM_UNIT_WEIGHT)
                .map(|p| p.enum_name),
            Some("PHY_MATERIAL_PARAM_UNIT_WEIGHT")
        );
        // 2,400 kg/m^3 = 67.96 kg/ft^3; times 32.174 ft/s^2.
        let mut set = vec![
            (PHY_MATERIAL_PARAM_UNIT_WEIGHT, ParamValue::Double(1.08e-5)),
            (
                PHY_MATERIAL_PARAM_STRUCTURAL_DENSITY,
                ParamValue::Double(67.960_420_8),
            ),
        ];
        unit_weight_from_density(&mut set);
        let ParamValue::Double(w) = set[0].1 else {
            panic!()
        };
        assert!((w - 67.960_420_8 * 9.80665 / 0.3048).abs() < 1e-9);
        let mut no_density = vec![(PHY_MATERIAL_PARAM_UNIT_WEIGHT, ParamValue::Double(1.0))];
        unit_weight_from_density(&mut no_density);
        assert_eq!(no_density[0].1, ParamValue::Double(1.0));
    }
    use crate::formats::{SchemaField, SchemaFieldType, SchemaLayout};

    /// A schema with one class, tag 7, holding one u32.
    fn layouts() -> SchemaLayouts {
        let mut classes = BTreeMap::new();
        classes.insert(
            7,
            SchemaLayout {
                tag: 7,
                name: "ElementHeader".into(),
                base: None,
                fields: vec![SchemaField {
                    name: "m_x".into(),
                    ty: SchemaFieldType::Scalar {
                        kind: 4,
                        modifier: 0,
                        count: None,
                    },
                    flags: 0,
                }],
            },
        );
        SchemaLayouts { classes }
    }

    #[test]
    fn header_runs_are_records_and_markers() {
        let mut buf = Vec::new();
        buf.extend([0xff; 8]);
        buf.extend(1u64.to_le_bytes());
        buf.extend(0u32.to_le_bytes());
        for (id, x) in [(5u64, 9u32), (6, 10)] {
            buf.extend(id.to_le_bytes());
            buf.extend(6u32.to_le_bytes());
            buf.extend(7u16.to_le_bytes());
            buf.extend(x.to_le_bytes());
            buf.extend(6u32.to_le_bytes());
        }
        buf.extend([0xff; 8]);
        buf.extend(0u64.to_le_bytes());
        let got = header_records(&layouts(), "P", &buf, 0..buf.len(), IdWidth::Wide);
        assert!(got.rest.is_empty(), "{:?}", got.rest);
        assert_eq!(got.markers, vec![0..20, buf.len() - 16..buf.len()]);
        assert_eq!(got.records.len(), 2);
        assert!(got.records.iter().all(|r| r.exact));
        assert_eq!(got.records[1].frame.header.id, 6);
        assert_eq!(
            got.records[1].root().unwrap().get("m_x"),
            Some(&Value::Int(10))
        );

        // A byte that fits neither is kept, and reading resumes after it.
        let mut odd = vec![0x42];
        odd.extend(&buf);
        let got = header_records(&layouts(), "P", &odd, 0..odd.len(), IdWidth::Wide);
        assert_eq!(got.rest, vec![0..1]);
        assert_eq!(got.records.len(), 2);
    }

    #[test]
    fn a_class_reference_is_two_bytes() {
        let mut l = layouts();
        let class = l.classes.get_mut(&7).unwrap();
        class.fields[0].ty = SchemaFieldType::Scalar {
            kind: 0x0a,
            modifier: 0,
            count: None,
        };
        let mut buf = Vec::new();
        buf.extend(5u64.to_le_bytes());
        buf.extend(4u32.to_le_bytes());
        buf.extend(7u16.to_le_bytes());
        buf.extend(0x0c77u16.to_le_bytes());
        buf.extend(4u32.to_le_bytes());
        let got = header_records(&l, "P", &buf, 0..buf.len(), IdWidth::Wide);
        assert!(got.records[0].exact);
        assert_eq!(
            got.records[0].root().unwrap().get("m_x"),
            Some(&Value::Int(0x0c77))
        );
    }

    #[test]
    fn a_graph_reads_from_its_root_and_keeps_the_rest() {
        let mut buf = Vec::new();
        buf.extend(7u16.to_le_bytes());
        buf.extend(9u32.to_le_bytes());
        buf.extend(0u32.to_le_bytes());
        let got = read_graph(&layouts(), "G", &buf, 0);
        assert_eq!(got.root().unwrap().get("m_x"), Some(&Value::Int(9)));
        assert_eq!(got.unparsed, Some(6..10));
        assert!(read_graph(&layouts(), "G", &buf, 9).objects.is_empty());
    }

    #[test]
    fn content_documents_are_key_size_graph_size_records() {
        let object = |x: u32| {
            let mut b = 7u16.to_le_bytes().to_vec();
            b.extend(x.to_le_bytes());
            b
        };
        let mut buf = Vec::new();
        for x in [1, 2] {
            buf.extend(object(x));
            buf.extend(6u32.to_le_bytes());
            buf.extend(object(10 + x));
            buf.extend(6u32.to_le_bytes());
        }
        let records_end = buf.len();
        buf.extend([0x7b, 0x03, 0, 0, 0, 0]);
        let (docs, end) = content_documents(&layouts(), &buf);
        assert_eq!(end, records_end);
        assert_eq!(docs.len(), 2);
        assert_eq!(docs[1].span, 20..40);
        assert_eq!(
            docs[1].graph.root().unwrap().get("m_x"),
            Some(&Value::Int(12))
        );
        assert!(docs.iter().all(|d| d.graph.exact));
    }

    /// Before Revit 2012 `ElementId` is tag 19 and tag 20 is `Identifier`,
    /// the class it holds: an id field still reads as an id.
    #[test]
    fn element_id_is_found_by_class_name() {
        let field = |name: &str, ty| SchemaField {
            name: name.into(),
            ty,
            flags: 0,
        };
        let class = |tag, name: &str, fields| SchemaLayout {
            tag,
            name: name.into(),
            base: None,
            fields,
        };
        let int = SchemaFieldType::Scalar {
            kind: 4,
            modifier: 0,
            count: None,
        };
        let object = |tag| SchemaFieldType::Object {
            tag,
            modifier: 0,
            count: None,
        };
        let mut classes = BTreeMap::new();
        classes.insert(19, class(19, "ElementId", vec![field("m_id", object(20))]));
        classes.insert(
            20,
            class(20, "Identifier", vec![field("m_id", int.clone())]),
        );
        classes.insert(7, class(7, "Thing", vec![field("m_famId", object(19))]));
        let layouts = SchemaLayouts { classes };
        let mut buf = 7u16.to_le_bytes().to_vec();
        buf.extend(42i32.to_le_bytes());
        let frame = Frame {
            header: ObjectHeader {
                id: 1,
                width: IdWidth::Narrow,
                adler: 0,
                size: buf.len() as u32,
                tag: 7,
            },
            span: 0..buf.len(),
            payload: 0..buf.len(),
        };
        let object = read_frame(&layouts, "Partitions/0", &buf, frame);
        assert!(object.exact, "{:?}", object.stopped);
        assert_eq!(
            object.root().and_then(|r| r.get("m_famId")),
            Some(&Value::Id(42))
        );
    }
}
