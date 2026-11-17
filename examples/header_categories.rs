//! The category of each element, as `element id<TAB>BuiltInCategory id` lines: the one the
//! exporter reads from the element's partition record (`ifc::revit_identity::element_categories`).
//!
//! ```text
//! cargo run --release --example header_categories -- model.rvt > categories.tsv
//! ```
//!
//! Used to join an element with its category in Revit's own IFC export of a public sample
//! (`tools/builtin-parameters/derive_categories.py`).

use rvt::RevitFile;
use rvt::ifc::revit_identity::element_categories;
use std::io::Write;

fn main() -> anyhow::Result<()> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: header_categories <file.rvt>"))?;
    let mut rf = RevitFile::open(&path)?;
    let out = std::io::stdout();
    let mut w = std::io::BufWriter::new(out.lock());
    for (element, category) in element_categories(&mut rf) {
        writeln!(w, "{element}\t{category}")?;
    }
    w.flush()?;
    Ok(())
}
