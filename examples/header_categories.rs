//! The category each element's header names, as `element id<TAB>BuiltInCategory id` lines.
//!
//! ```text
//! cargo run --release --example header_categories -- model.rvt > categories.tsv
//! ```
//!
//! Used to join an element with its category in Revit's own IFC export of a public sample
//! (`tools/builtin-parameters/derive_categories.py`).

use rvt::RevitFile;
use rvt::field_parameters::{ELEM_CATEGORY_PARAM, header_parameters};
use rvt::schema_objects::{IdWidth, ParamValue, for_each_object, header_records};
use std::io::Write;

fn main() -> anyhow::Result<()> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: header_categories <file.rvt>"))?;
    let mut rf = RevitFile::open(&path)?;
    let release = rf.basic_file_info().map_or(0, |b| b.version);
    let layouts = rf.schema_layouts()?;
    let gaps = for_each_object(&mut rf, |_| {})?;
    let width = IdWidth::of_release(release);
    let out = std::io::stdout();
    let mut w = std::io::BufWriter::new(out.lock());
    for (stream, g) in gaps {
        let inflated = rf.inflated_partition(&stream)?;
        let buf = inflated.bytes();
        for gap in &g {
            for record in &header_records(&layouts, &stream, buf, gap.clone(), width).records {
                let Some(root) = record.root() else { continue };
                for (p, v) in header_parameters(root) {
                    if let (ELEM_CATEGORY_PARAM, ParamValue::ElementId(category)) = (p, v) {
                        writeln!(w, "{}\t{category}", record.frame.header.id)?;
                    }
                }
            }
        }
    }
    Ok(())
}
