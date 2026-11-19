//! The type of every parameter value a file stores, as
//! `element id<TAB>parameter id<TAB>origin<TAB>type<TAB>value<TAB>class` lines.
//!
//! ```text
//! cargo run --release --example param_census -- model.rvt > census.tsv
//! ```
//!
//! Origin `S` is the element's own value sets, `N` the other parameter sets of its objects, `F`
//! what `field_parameters` derives from the element's fields and `H` its element header's
//! parameters. A storage type is read only from `S` and `N`: they are the file's own types
//! (`tools/builtin-parameters/derive_storage.py`).

use rvt::RevitFile;
use rvt::field_parameters::{field_parameters, header_parameters};
use rvt::schema_objects::{IdWidth, ParamValue, for_each_object, header_records};
use std::io::Write;

fn main() -> anyhow::Result<()> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: param_census <file.rvt>"))?;
    let mut rf = RevitFile::open(&path)?;
    let release = rf.basic_file_info()?.version;
    let layouts = rf.schema_layouts()?;
    let out = std::io::stdout();
    let mut w = std::io::BufWriter::new(out.lock());
    let mut emit = |id: i64, origin: char, p: i64, v: &ParamValue, class: &str| {
        let (t, s) = match v {
            ParamValue::Double(f) => ("Double", f.to_string()),
            ParamValue::Integer(i) => ("Integer", i.to_string()),
            ParamValue::String(s) => (
                "String",
                s.chars()
                    .take(120)
                    .collect::<String>()
                    .replace(['\t', '\n', '\r'], " "),
            ),
            ParamValue::ElementId(i) => ("ElementId", i.to_string()),
        };
        if let Err(e) = writeln!(w, "{id}\t{p}\t{origin}\t{t}\t{s}\t{class}") {
            eprintln!("write failed: {e}");
            std::process::exit(1);
        }
    };
    let gaps = for_each_object(&mut rf, |o| {
        let Some(id) = o.element() else { return };
        let class = o
            .root()
            .and_then(|r| layouts.classes.get(&r.tag))
            .map_or("", |c| c.name.as_str());
        for (p, v) in o.stored_parameters() {
            emit(id, 'S', p, &v, class);
        }
        for (p, v) in o.nested_parameters() {
            emit(id, 'N', p, &v, class);
        }
        for (p, v) in field_parameters(class, &o.objects) {
            emit(id, 'F', p, &v, class);
        }
    })?;
    let width = IdWidth::of_release(release);
    for (stream, g) in gaps {
        let inflated = rf.inflated_partition(&stream)?;
        for gap in g {
            let headers = header_records(&layouts, &stream, inflated.bytes(), gap, width);
            for record in &headers.records {
                let Some(root) = record.root() else { continue };
                for (p, v) in header_parameters(root) {
                    emit(i64::try_from(record.frame.header.id)?, 'H', p, &v, "");
                }
            }
        }
    }
    w.flush()?;
    Ok(())
}
