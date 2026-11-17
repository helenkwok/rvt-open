use rvt::RevitFile;
use rvt::field_parameters::field_parameters;
use rvt::schema_objects::{ParamValue, for_each_object};
use std::io::Write;

fn main() -> anyhow::Result<()> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("file"))?;
    let mut rf = RevitFile::open(&path)?;
    let layouts = rf.schema_layouts()?;
    let out = std::io::stdout();
    let mut w = std::io::BufWriter::new(out.lock());
    let mut emit = |id: i64, o: char, p: i64, v: &ParamValue, c: &str| {
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
        let _ = writeln!(w, "{id}\t{p}\t{o}\t{t}\t{s}\t{c}");
    };
    for_each_object(&mut rf, |o| {
        let Some(id) = o.element() else { return };
        let cls = o
            .root()
            .and_then(|r| layouts.classes.get(&r.tag))
            .map_or("", |c| c.name.as_str())
            .to_string();
        for (p, v) in o.stored_parameters() {
            emit(id, 'S', p, &v, &cls);
        }
        for (p, v) in o.nested_parameters() {
            emit(id, 'N', p, &v, &cls);
        }
        let class = o
            .root()
            .and_then(|r| layouts.classes.get(&r.tag))
            .map_or("", |c| c.name.as_str());
        for (p, v) in field_parameters(class, &o.objects) {
            emit(id, 'F', p, &v, &cls);
        }
    })?;
    Ok(())
}
