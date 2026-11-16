//! What the schema reader reads of one Revit file, as JSON facts for
//! `tools/full-decode/full_decode.py`, which compares them with the file's
//! Revit API dump and accounts for every byte.
//!
//! ```text
//! cargo run --example full_decode -- model.rvt > facts.json
//! ```
//!
//! Written: the release; every class by schema tag, with its base class; per partition its
//! inflated length, every data object (id, class, span, the bytes the schema
//! read, the unread rest and why it stopped) and the byte ranges between
//! objects, with the element-header records and run markers read from them;
//! every element object's root fields, stored parameters and the parameters
//! its fields and header give ([`rvt::field_parameters`]); the
//! element table's rows; the built-in parameter ids by enum name. Nothing is interpreted here: the script decides what
//! matches the API.

use rvt::RevitFile;
use rvt::field_parameters::{field_parameters, header_parameters};
use rvt::schema_objects::{Fields, IdWidth, ParamValue, Value, for_each_object, header_records};
use serde_json::{Value as J, json};
use std::path::PathBuf;

fn value(v: &Value) -> J {
    match v {
        Value::Bool(b) => json!(b),
        Value::Int(i) => json!(i),
        Value::Float(f) if f.is_finite() => json!(f),
        Value::Float(f) => json!(f.to_string()),
        Value::Text(s) => json!(s),
        Value::Guid(g) => json!({ "guid": hex(g) }),
        Value::Id(i) => json!({ "id": i }),
        Value::Null => J::Null,
        Value::New { tag, index, .. } => json!({ "new": tag, "index": index }),
        Value::Ref { marker, tag } => json!({ "ref": marker, "tag": tag }),
        Value::DocRef(r) => json!({ "docref": r }),
        Value::EsPtr { schema, index, .. } => json!({ "es": hex(schema), "index": index }),
        Value::Bytes(b) => json!({ "bytes": b.len() }),
        Value::Object(f) => fields(f),
        Value::List(l) => J::Array(l.iter().map(value).collect()),
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Fields as `[[name, value], ...]`: names repeat along a class chain.
fn fields(f: &Fields) -> J {
    json!({
        "tag": f.tag,
        "fields": f.values.iter().map(|(n, v)| json!([n, value(v)])).collect::<Vec<_>>(),
    })
}

fn param(v: &ParamValue) -> J {
    match v {
        ParamValue::Double(f) if f.is_finite() => json!(["Double", f]),
        ParamValue::Double(f) => json!(["Double", f.to_string()]),
        ParamValue::Integer(i) => json!(["Integer", i]),
        ParamValue::String(s) => json!(["String", s]),
        ParamValue::ElementId(i) => json!(["ElementId", i]),
    }
}

fn main() -> anyhow::Result<()> {
    let path: PathBuf = std::env::args_os()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: full_decode <file.rvt>"))?
        .into();
    let mut rf = RevitFile::open(&path)?;
    let release = rf.basic_file_info().map(|b| b.version).unwrap_or(0);
    let layouts = rf.schema_layouts()?;
    let classes: serde_json::Map<String, J> = layouts
        .classes
        .iter()
        .map(|(t, c)| (t.to_string(), json!(c.name)))
        .collect();
    let bases: serde_json::Map<String, J> = layouts
        .classes
        .iter()
        .filter_map(|(t, c)| Some((t.to_string(), json!(c.base?))))
        .collect();

    let mut objects: Vec<J> = Vec::new();
    let mut elements: Vec<J> = Vec::new();
    let gaps = for_each_object(&mut rf, |o| {
        let element = o.element();
        objects.push(json!({
            "stream": o.stream,
            "id": o.frame.header.id,
            "tag": o.frame.header.tag,
            "span": [o.frame.span.start, o.frame.span.end],
            "payload": [o.frame.payload.start, o.frame.payload.end],
            "unparsed": o.unparsed.as_ref().map(|r| [r.start, r.end]),
            "stopped": o.stopped,
            "objects": o.objects.len(),
            "element": element.is_some(),
        }));
        if let (Some(id), Some(root)) = (element, o.root()) {
            elements.push(json!({
                "id": id,
                "tag": root.tag,
                "stream": o.stream,
                "offset": o.frame.span.start,
                "exact": o.exact,
                "root": fields(root),
                "params": o.stored_parameters().iter().map(|(p, v)| json!([p, param(v)])).collect::<Vec<_>>(),
                "field_params": field_parameters(
                    layouts.classes.get(&root.tag).map_or("", |c| c.name.as_str()),
                    root,
                )
                .iter()
                .map(|(p, v)| json!([p, param(v)]))
                .collect::<Vec<_>>(),
            }));
        }
    })?;
    let width = IdWidth::of_release(release);
    let mut partitions = Vec::new();
    let mut headers = Vec::new();
    for (stream, g) in gaps {
        let inflated = rf.inflated_partition(&stream)?;
        let buf = inflated.bytes();
        let (mut markers, mut rest) = (Vec::new(), Vec::new());
        for gap in &g {
            let h = header_records(&layouts, &stream, buf, gap.clone(), width);
            markers.extend(h.markers.iter().map(|r| [r.start, r.end]));
            rest.extend(h.rest.iter().map(|r| [r.start, r.end]));
            for o in &h.records {
                headers.push(json!({
                    "stream": stream,
                    "id": o.frame.header.id,
                    "tag": o.frame.header.tag,
                    "span": [o.frame.span.start, o.frame.span.end],
                    "payload": [o.frame.payload.start, o.frame.payload.end],
                    "unparsed": o.unparsed.as_ref().map(|r| [r.start, r.end]),
                    "stopped": o.stopped,
                    "root": o.root().map(fields),
                    "header_params": o.root().map(|r| {
                        header_parameters(r)
                            .iter()
                            .map(|(p, v)| json!([p, param(v)]))
                            .collect::<Vec<_>>()
                    }),
                }));
            }
        }
        partitions.push(json!({
            "stream": stream,
            "inflated": buf.len(),
            "gaps": g.iter().map(|r| [r.start, r.end]).collect::<Vec<_>>(),
            "header_markers": markers,
            "header_rest": rest,
        }));
    }
    let rows: Vec<J> = rvt::elem_table::parse_records(&mut rf)?
        .iter()
        .map(|r| {
            json!([
                r.offset,
                r.id_primary,
                r.id_secondary,
                r.owner_id,
                r.raw.len(),
                hex(&r.raw)
            ])
        })
        .collect();
    let builtin_ids: serde_json::Map<String, J> = rvt::builtin_parameters::parameter_ids()
        .filter_map(rvt::builtin_parameters::parameter)
        .filter(|p| !p.enum_name.is_empty())
        .map(|p| (p.enum_name.to_string(), json!(p.id)))
        .collect();
    let unique_ids: serde_json::Map<String, J> =
        rvt::research::revit_global_ids::revit_unique_ids(&mut rf)?
            .into_iter()
            .map(|(id, u)| (id.to_string(), json!(u)))
            .collect();
    let out = json!({
        "unique_ids": unique_ids,
        "release": release,
        "builtin_ids": builtin_ids,
        "classes": classes,
        "bases": bases,
        "partitions": partitions,
        "objects": objects,
        "elements": elements,
        "headers": headers,
        "elem_table": rows,
    });
    println!("{out}");
    Ok(())
}
