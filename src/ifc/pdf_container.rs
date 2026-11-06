//! A one-page PDF holding a 3D annotation (ISO 32000-1:2008, 13.6): the
//! page, a `/3D` annotation over it, its 3D stream (`/Subtype /U3D` or
//! `/PRC`), one default view (`/3DView`, camera given by its camera-to-world
//! matrix, perspective, CAD lighting, white background) and a poster
//! appearance stream that a viewer without 3D support draws instead.
//!
//! Self-contained: it depends on nothing in this crate, only on `flate2`
//! for the streams, so it can serve any 3D stream writer.

use std::fmt::Write as _;
use std::io::Write as _;

/// The default view of the annotation.
#[derive(Debug, Clone)]
pub struct View3d {
    /// Shown in a viewer's list of views.
    pub name: String,
    /// Camera to world (13.6.5): the camera's x (right), y (down) and z
    /// (forward) axes in world coordinates, then its position.
    pub c2w: [f64; 12],
    /// Distance from the camera to the centre of orbit, along its z axis.
    pub center_of_orbit: f64,
    /// Perspective field of view, degrees.
    pub field_of_view: f64,
}

/// What [`write_pdf`] writes.
#[derive(Debug, Clone)]
pub struct Page3d<'a> {
    /// The 3D stream, as its format defines it.
    pub stream: &'a [u8],
    /// `U3D` or `PRC`.
    pub subtype: &'a str,
    pub view: View3d,
    /// Content stream operators of the poster, in a box `poster_size` big
    /// with its origin at the bottom left.
    pub poster: &'a [u8],
    /// Page width and height, points.
    pub page_size: [f64; 2],
    /// The annotation's margin from the page edge, points.
    pub margin: f64,
    /// The annotation's text alternative.
    pub alt: &'a str,
}

/// A PDF number: at most six decimals, no exponent.
pub fn num(v: f64) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    let s = format!("{v:.6}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" || s.is_empty() {
        "0".into()
    } else {
        s.into()
    }
}

/// A text string, as UTF-16BE hex with its byte order mark.
fn text(s: &str) -> String {
    let mut out = String::from("<FEFF");
    for u in s.encode_utf16() {
        let _ = write!(out, "{u:04X}");
    }
    out.push('>');
    out
}

fn deflate(data: &[u8]) -> Vec<u8> {
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(data).expect("deflate into memory");
    z.finish().expect("deflate into memory")
}

/// The page as a PDF 1.7 file.
pub fn write_pdf(page: &Page3d) -> Vec<u8> {
    let [w, h] = page.page_size;
    let m = page.margin;
    let (aw, ah) = (w - 2.0 * m, h - 2.0 * m);
    let v = &page.view;
    let c2w: Vec<String> = v.c2w.iter().map(|&x| num(x)).collect();
    let view = format!(
        "<< /Type /3DView /XN {name} /IN {name} /MS /M /C2W [{c2w}] /CO {co} \
         /P << /Subtype /P /FOV {fov} /PS /Min >> /BG << /Type /3DBG /C [1 1 1] >> \
         /LS << /Type /3DLightingScheme /Subtype /CAD >> >>",
        name = text(&v.name),
        c2w = c2w.join(" "),
        co = num(v.center_of_orbit),
        fov = num(v.field_of_view),
    );
    let stream3d = deflate(page.stream);
    let poster = deflate(page.poster);

    let mut objects: Vec<Vec<u8>> = Vec::new();
    objects.push(b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());
    objects.push(b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec());
    objects.push(
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Resources << >> /Annots [4 0 R] >>",
            num(w),
            num(h)
        )
        .into_bytes(),
    );
    objects.push(
        format!(
            "<< /Type /Annot /Subtype /3D /Rect [{m} {m} {x} {y}] /F 4 /P 3 0 R /Contents {alt} \
             /3DD 5 0 R /3DV 6 0 R /3DA << /A /PO /DIS /PC >> /AP << /N 7 0 R >> >>",
            m = num(m),
            x = num(w - m),
            y = num(h - m),
            alt = text(page.alt),
        )
        .into_bytes(),
    );
    let mut s = format!(
        "<< /Type /3D /Subtype /{} /VA [6 0 R] /DV 0 /Filter /FlateDecode /Length {} >>\nstream\n",
        page.subtype,
        stream3d.len()
    )
    .into_bytes();
    s.extend_from_slice(&stream3d);
    s.extend_from_slice(b"\nendstream");
    objects.push(s);
    objects.push(view.into_bytes());
    let mut s = format!(
        "<< /Type /XObject /Subtype /Form /BBox [0 0 {} {}] /Resources << >> \
         /Filter /FlateDecode /Length {} >>\nstream\n",
        num(aw),
        num(ah),
        poster.len()
    )
    .into_bytes();
    s.extend_from_slice(&poster);
    s.extend_from_slice(b"\nendstream");
    objects.push(s);

    let mut out = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    let mut table = format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1);
    for o in offsets {
        let _ = writeln!(table, "{o:010} 00000 n ");
    }
    let _ = write!(
        table,
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        objects.len() + 1
    );
    out.extend_from_slice(table.as_bytes());
    out
}
