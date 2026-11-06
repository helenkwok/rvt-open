//! `rvt-gltf --format pdf3d` writes a PDF whose structure re-reads (header,
//! every cross-reference offset, the `/3D` annotation, its `/U3D` stream,
//! default view and poster), and whose U3D stream decodes, with a decoder
//! written here from ECMA-363 clause 10, to the meshes of `rvt-gltf
//! --no-instancing --no-edges` for the same file: one model node per glTF node
//! that draws triangles, the same triangle count, every index in range, and
//! the element ids as node metadata.

use std::collections::HashMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};

fn sample() -> Option<PathBuf> {
    let dir = std::env::var_os("RVT_REVIT2024_SAMPLES_DIR").map(PathBuf::from)?;
    let p = dir.join("racbasicsampleproject.rvt");
    p.exists().then_some(p)
}

fn tier1() -> Option<PathBuf> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("corpus/tier1/architectural-2024/architectural-2024.rvt");
    p.exists().then_some(p)
}

fn run(input: &Path, out: &Path, args: &[&str]) {
    let st = std::process::Command::new(env!("CARGO_BIN_EXE_rvt-gltf"))
        .arg(input)
        .arg("-o")
        .arg(out)
        .args(args)
        .output()
        .expect("spawn rvt-gltf");
    assert!(
        st.status.success(),
        "{}",
        String::from_utf8_lossy(&st.stderr)
    );
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    hay.get(from..)?
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

/// The integer after `key ` in `dict`.
fn int_after(dict: &[u8], key: &[u8]) -> usize {
    let at = find(dict, key, 0).expect("key") + key.len();
    let s: String = dict[at..]
        .iter()
        .map(|&b| b as char)
        .skip_while(|c| *c == ' ')
        .take_while(char::is_ascii_digit)
        .collect();
    s.parse().expect("integer")
}

/// The object `n 0 obj ... endobj` as bytes, located through the xref.
fn object(pdf: &[u8], offsets: &[usize], n: usize) -> Vec<u8> {
    let at = offsets[n - 1];
    let head = format!("{n} 0 obj\n");
    assert!(pdf[at..].starts_with(head.as_bytes()), "xref entry {n}");
    let end = find(pdf, b"\nendobj", at).expect("endobj");
    pdf[at + head.len()..end].to_vec()
}

/// A stream object's decoded data.
fn stream_data(obj: &[u8]) -> Vec<u8> {
    let len = int_after(obj, b"/Length");
    let start = find(obj, b"stream\n", 0).expect("stream") + 7;
    let raw = &obj[start..start + len];
    assert!(find(obj, b"/FlateDecode", 0).is_some());
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(raw)
        .read_to_end(&mut out)
        .expect("inflate");
    out
}

/// The PDF's xref offsets after checking the header and trailer.
fn xref(pdf: &[u8]) -> Vec<usize> {
    assert!(pdf.starts_with(b"%PDF-1.7\n"));
    assert!(pdf.ends_with(b"%%EOF\n"));
    let sx = find(pdf, b"startxref\n", 0).expect("startxref") + 10;
    let at: usize = std::str::from_utf8(&pdf[sx..])
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .parse()
        .unwrap();
    assert!(pdf[at..].starts_with(b"xref\n0 "));
    let text = std::str::from_utf8(&pdf[at..]).unwrap();
    let mut lines = text.lines().skip(1);
    let count: usize = lines.next().unwrap()[2..].parse().unwrap();
    assert_eq!(lines.next().unwrap(), "0000000000 65535 f ");
    (1..count)
        .map(|_| {
            let l = lines.next().unwrap();
            assert_eq!(l.len(), 19, "xref entry is 20 bytes with its newline");
            l[..10].parse().unwrap()
        })
        .collect()
}

// ---- U3D bit decoder (ECMA-363 clause 10, read side) ----

const STATIC_FULL: u32 = 0x400;
const MAX_RANGE: u32 = STATIC_FULL + 0x3FFF;

struct Histogram {
    counts: Vec<u32>,
    total: u32,
}

impl Histogram {
    fn add(&mut self, symbol: u32) {
        if symbol >= 0xFFFF {
            return;
        }
        if self.total >= 0x1FFF {
            for c in &mut self.counts {
                *c >>= 1;
            }
            self.counts[0] += 1;
            self.total = self.counts.iter().sum();
        }
        let s = symbol as usize;
        if self.counts.len() <= s {
            self.counts.resize(s + 1, 0);
        }
        self.counts[s] += 1;
        self.total += 1;
    }
}

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    high: u32,
    low: u32,
    code: u32,
    histograms: HashMap<u32, Histogram>,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        let mut b = Self {
            data,
            pos: 0,
            high: 0xFFFF,
            low: 0,
            code: 0,
            histograms: HashMap::new(),
        };
        for _ in 0..16 {
            b.code = (b.code << 1) | b.bit();
        }
        b
    }

    fn bit(&mut self) -> u32 {
        let v = self
            .data
            .get(self.pos / 8)
            .map_or(0, |byte| u32::from((byte >> (self.pos % 8)) & 1));
        self.pos += 1;
        v
    }

    /// The next symbol (value + 1, or 0 for escape) in `context`.
    fn symbol(&mut self, context: u32) -> u32 {
        let range = u64::from(self.high) + 1 - u64::from(self.low);
        let (low, code) = (u64::from(self.low), u64::from(self.code));
        let static_total = match context {
            0 => Some(256),
            c if c >= STATIC_FULL => Some(c - STATIC_FULL),
            _ => None,
        };
        let (symbol, cum, freq, total) = if let Some(total) = static_total {
            // The largest cumulative frequency whose low bound is <= code.
            let (mut lo, mut hi) = (0u32, total - 1);
            while lo < hi {
                let mid = (lo + hi).div_ceil(2);
                if low + range * u64::from(mid) / u64::from(total) <= code {
                    lo = mid;
                } else {
                    hi = mid - 1;
                }
            }
            (lo + 1, lo, 1, total)
        } else {
            let h = self.histograms.entry(context).or_insert(Histogram {
                counts: vec![1],
                total: 1,
            });
            let mut cum = 0;
            let mut found = None;
            for (s, &f) in h.counts.iter().enumerate() {
                let top = low + range * u64::from(cum + f) / u64::from(h.total);
                if f > 0 && code < top {
                    found = Some((s as u32, cum, f));
                    break;
                }
                cum += f;
            }
            let (s, c, f) = found.expect("symbol in the dynamic histogram");
            (s, c, f, h.total)
        };
        self.high = (low + range * u64::from(cum + freq) / u64::from(total) - 1) as u32;
        self.low = (low + range * u64::from(cum) / u64::from(total)) as u32;
        assert!(
            self.low <= self.code && self.code <= self.high,
            "decoder lost sync"
        );
        if static_total.is_none() {
            self.histograms.get_mut(&context).unwrap().add(symbol);
        }
        loop {
            if (self.high & 0x8000) == (self.low & 0x8000) {
                self.high = ((self.high & 0x7FFF) << 1) | 1;
                self.low = (self.low & 0x7FFF) << 1;
                self.code = ((self.code & 0x7FFF) << 1) | self.bit();
            } else if (self.high & 0x4000) == 0 && (self.low & 0x4000) != 0 {
                self.high = ((self.high & 0x3FFF) << 1) | 0x8001;
                self.low = (self.low & 0x3FFF) << 1;
                self.code = ((self.code - 0x4000) << 1) | self.bit();
            } else {
                break;
            }
        }
        symbol
    }

    fn u8(&mut self) -> u8 {
        ((self.symbol(0) - 1) as u8).reverse_bits()
    }

    fn u16(&mut self) -> u16 {
        u16::from_le_bytes([self.u8(), self.u8()])
    }

    fn u32(&mut self) -> u32 {
        u32::from_le_bytes([self.u8(), self.u8(), self.u8(), self.u8()])
    }

    fn f32(&mut self) -> f32 {
        f32::from_bits(self.u32())
    }

    fn string(&mut self) -> String {
        let n = self.u16();
        let bytes: Vec<u8> = (0..n).map(|_| self.u8()).collect();
        String::from_utf8(bytes).expect("UTF-8 string")
    }

    fn compressed_u32(&mut self, context: u32) -> u32 {
        if context == 0 || context >= MAX_RANGE {
            return self.u32();
        }
        let s = self.symbol(context);
        if s != 0 {
            return s - 1;
        }
        let v = self.u32();
        if context < STATIC_FULL {
            self.histograms
                .get_mut(&context)
                .unwrap()
                .add(v.wrapping_add(1));
        }
        v
    }
}

/// `(type, data, metadata)` of each block in `bytes`.
fn blocks(bytes: &[u8]) -> Vec<(u32, &[u8], &[u8])> {
    let mut out = Vec::new();
    let mut at = 0;
    let u = |b: &[u8], i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
    while at < bytes.len() {
        let (ty, ds, ms) = (
            u(bytes, at),
            u(bytes, at + 4) as usize,
            u(bytes, at + 8) as usize,
        );
        let data = &bytes[at + 12..at + 12 + ds];
        let m0 = at + 12 + ds.div_ceil(4) * 4;
        out.push((ty, data, &bytes[m0..m0 + ms]));
        at = m0 + ms.div_ceil(4) * 4;
    }
    assert_eq!(at, bytes.len(), "blocks end where the data ends");
    out
}

/// A model node: its name, mesh name, metadata and shader list count.
type NodeRow = (String, String, Vec<(String, String)>, u32);

#[derive(Default)]
struct U3d {
    /// Mesh name to (faces, positions, shading count).
    declared: HashMap<String, (u32, u32, u32)>,
    /// Mesh name to faces decoded from the base mesh block.
    decoded: HashMap<String, u32>,
    nodes: Vec<NodeRow>,
    shaders: usize,
    materials: usize,
}

fn read_metadata(meta: &[u8]) -> Vec<(String, String)> {
    if meta.is_empty() {
        return Vec::new();
    }
    let mut b = Bits::new(meta);
    (0..b.u32())
        .map(|_| {
            assert_eq!(b.u32(), 0);
            (b.string(), b.string())
        })
        .collect()
}

fn read_block(u: &mut U3d, ty: u32, data: &[u8], meta: &[u8]) {
    match ty {
        0xFFFF_FF14 => {
            let mut b = Bits::new(data);
            let name = b.string();
            let chain_type = b.u32();
            assert_eq!(b.u32(), 0);
            let at = (2 + name.len() + 8).div_ceil(4) * 4;
            let count = u32::from_le_bytes(data[at..at + 4].try_into().unwrap());
            let inner = blocks(&data[at + 4..]);
            assert_eq!(inner.len(), count as usize);
            assert!(chain_type <= 1);
            for (t, d, m) in inner {
                read_block(u, t, d, m);
            }
        }
        0xFFFF_FF31 => {
            let mut b = Bits::new(data);
            let name = b.string();
            assert_eq!(b.u32(), 0, "chain index");
            assert_eq!(b.u32(), 0, "faces carry normals");
            let (faces, positions, normals) = (b.u32(), b.u32(), b.u32());
            assert_eq!(normals, faces);
            assert_eq!((b.u32(), b.u32(), b.u32()), (0, 0, 0));
            let shading = b.u32();
            for s in 0..shading {
                assert_eq!((b.u32(), b.u32(), b.u32()), (0, 0, s));
            }
            assert_eq!(
                (b.u32(), b.u32()),
                (positions, positions),
                "base mesh is whole"
            );
            u.declared.insert(name, (faces, positions, shading));
        }
        0xFFFF_FF3B => {
            let mut b = Bits::new(data);
            let name = b.string();
            let &(faces, positions, shading) = u.declared.get(&name).expect("declared mesh");
            assert_eq!(b.u32(), 0);
            assert_eq!((b.u32(), b.u32(), b.u32()), (faces, positions, faces));
            assert_eq!((b.u32(), b.u32(), b.u32()), (0, 0, 0));
            for _ in 0..positions * 3 {
                assert!(b.f32().is_finite());
            }
            for _ in 0..faces {
                let n = [b.f32(), b.f32(), b.f32()];
                let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                assert!((l - 1.0).abs() < 1e-3, "normal length {l}");
            }
            for f in 0..faces {
                assert!(b.compressed_u32(1) < shading.max(1));
                for _ in 0..3 {
                    assert!(b.compressed_u32(STATIC_FULL + positions) < positions);
                    assert_eq!(b.compressed_u32(STATIC_FULL + faces), f);
                }
            }
            assert_eq!(b.u32(), 0, "flush");
            u.decoded.insert(name, faces);
        }
        0xFFFF_FF22 => {
            let mut b = Bits::new(data);
            let name = b.string();
            assert_eq!(b.u32(), 1);
            assert_eq!(b.string(), "");
            for _ in 0..16 {
                assert!(b.f32().is_finite());
            }
            let mesh = b.string();
            assert_eq!(b.u32(), 3);
            u.nodes.push((name, mesh, read_metadata(meta), 0));
        }
        0xFFFF_FF45 => {
            let mut b = Bits::new(data);
            let name = b.string();
            assert_eq!((b.u32(), b.u32()), (1, 1));
            let lists = b.u32();
            for _ in 0..lists {
                assert_eq!(b.u32(), 1);
                assert!(b.string().starts_with("Shader "));
            }
            let node = u.nodes.last_mut().expect("model node first");
            assert_eq!(node.0, name);
            node.3 = lists;
        }
        0xFFFF_FF53 => u.shaders += 1,
        0xFFFF_FF54 => u.materials += 1,
        other => panic!("unexpected block {other:#x}"),
    }
}

fn read_u3d(bytes: &[u8]) -> U3d {
    assert_eq!(
        &bytes[..4],
        b"U3D\0",
        "file header block type, literal bytes"
    );
    let all = blocks(bytes);
    let (_, header, _) = all[0];
    let mut b = Bits::new(header);
    assert_eq!((b.u16(), b.u16(), b.u32()), (0, 0, 0));
    let declaration_size = b.u32() as usize;
    let file_size = u64::from(b.u32()) | (u64::from(b.u32()) << 32);
    assert_eq!(b.u32(), 106);
    assert_eq!(file_size as usize, bytes.len());
    let mut u = U3d::default();
    let mut at = 36;
    for &(ty, data, meta) in &all[1..] {
        let continuation = ty == 0xFFFF_FF3B;
        assert_eq!(
            continuation,
            at >= declaration_size,
            "declarations come first"
        );
        read_block(&mut u, ty, data, meta);
        at += 12 + data.len().div_ceil(4) * 4 + meta.len().div_ceil(4) * 4;
    }
    u
}

/// A glTF mesh's triangle primitives as (POSITION, indices, material).
type MeshKey = Vec<(Option<u64>, Option<u64>, Option<u64>)>;

/// Triangles per drawn node of a glb's default scene, its element ids, and
/// how many distinct meshes (by accessors and materials) the drawn nodes use.
fn glb_triangles(glb: &[u8]) -> (Vec<usize>, Vec<i64>, usize) {
    let jl = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
    let j: serde_json::Value = serde_json::from_slice(&glb[20..20 + jl]).unwrap();
    let acc = j["accessors"].as_array().unwrap();
    let per_mesh: Vec<usize> = j["meshes"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .map(|m| {
            m["primitives"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|p| p["mode"].as_u64().unwrap_or(4) == 4)
                .map(|p| match p["indices"].as_u64() {
                    Some(i) => acc[i as usize]["count"].as_u64().unwrap() as usize / 3,
                    None => {
                        acc[p["attributes"]["POSITION"].as_u64().unwrap() as usize]["count"]
                            .as_u64()
                            .unwrap() as usize
                            / 3
                    }
                })
                .sum()
        })
        .collect();
    let keys: Vec<MeshKey> = j["meshes"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .map(|m| {
            m["primitives"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|p| p["mode"].as_u64().unwrap_or(4) == 4)
                .map(|p| {
                    (
                        p["attributes"]["POSITION"].as_u64(),
                        p["indices"].as_u64(),
                        p["material"].as_u64(),
                    )
                })
                .collect()
        })
        .collect();
    let mut distinct = std::collections::HashSet::new();
    let nodes = j["nodes"].as_array().unwrap();
    let scene = j["scene"].as_u64().unwrap_or(0) as usize;
    let mut stack: Vec<usize> = j["scenes"][scene]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_u64().unwrap() as usize)
        .collect();
    let (mut drawn, mut ids) = (Vec::new(), Vec::new());
    while let Some(n) = stack.pop() {
        let node = &nodes[n];
        if let Some(id) = node["extras"]["elementId"].as_i64() {
            ids.push(id);
        }
        if let Some(m) = node["mesh"].as_u64() {
            let t = per_mesh[m as usize];
            if t > 0 {
                drawn.push(t);
                distinct.insert(&keys[m as usize]);
            }
        }
        if let Some(c) = node["children"].as_array() {
            stack.extend(c.iter().map(|c| c.as_u64().unwrap() as usize));
        }
    }
    let distinct = distinct.len();
    (drawn, ids, distinct)
}

#[test]
fn pdf_and_u3d_reread_to_the_glb_meshes() {
    let Some(input) = sample().or_else(tier1) else {
        eprintln!("skipping: tier-1 fixture missing");
        return;
    };
    let dir = std::env::temp_dir().join(format!("rvt-pdf3d-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (pdf_path, glb_path) = (dir.join("m.pdf"), dir.join("m.glb"));
    run(&input, &pdf_path, &["--format", "pdf3d"]);
    run(&input, &glb_path, &["--no-instancing", "--no-edges"]);
    let pdf = std::fs::read(&pdf_path).unwrap();
    let glb = std::fs::read(&glb_path).unwrap();

    let offsets = xref(&pdf);
    assert_eq!(offsets.len(), 7);
    let annot = object(&pdf, &offsets, 4);
    for key in [
        &b"/Subtype /3D "[..],
        b"/3DD 5 0 R",
        b"/3DV 6 0 R",
        b"/AP << /N 7 0 R >>",
    ] {
        assert!(
            find(&annot, key, 0).is_some(),
            "{}",
            String::from_utf8_lossy(key)
        );
    }
    let stream = object(&pdf, &offsets, 5);
    assert!(find(&stream, b"/Type /3D /Subtype /U3D", 0).is_some());
    let view = object(&pdf, &offsets, 6);
    for key in [
        &b"/Type /3DView"[..],
        b"/MS /M",
        b"/C2W [",
        b"/Subtype /CAD",
    ] {
        assert!(
            find(&view, key, 0).is_some(),
            "{}",
            String::from_utf8_lossy(key)
        );
    }
    let poster = String::from_utf8(stream_data(&object(&pdf, &offsets, 7))).unwrap();
    assert!(poster.starts_with("1 1 1 rg 0 0 "));

    let u = read_u3d(&stream_data(&stream));
    let (drawn, glb_ids, distinct) = glb_triangles(&glb);
    assert_eq!(
        u.nodes.len(),
        drawn.len(),
        "one model node per drawn glTF node"
    );
    assert_eq!(u.shaders, u.materials);
    let mut faces = 0usize;
    for (name, mesh, metadata, lists) in &u.nodes {
        assert!(!name.is_empty());
        let (f, _, shading) = u.declared[mesh];
        assert_eq!(u.decoded[mesh], f, "base mesh of {mesh}");
        assert_eq!(*lists, shading);
        faces += f as usize;
        for (k, v) in metadata {
            if k == "ElementId" {
                assert!(glb_ids.contains(&v.parse().unwrap()), "ElementId {v}");
            }
        }
    }
    assert_eq!(faces, drawn.iter().sum::<usize>(), "triangles");
    // With instancing off the glTF export gives every element its own mesh
    // object, but elements of one shape reuse the same accessors: the U3D
    // file declares one mesh per distinct (accessors, material) set, which
    // its nodes share.
    assert_eq!(
        u.declared.len(),
        distinct,
        "one U3D mesh per distinct glTF mesh content"
    );
    // Sharing is only possible where the model repeats a shape; a model of
    // all-unique shapes has as many meshes as nodes.
    if distinct < drawn.len() {
        assert!(u.declared.len() < u.nodes.len(), "nodes share meshes");
    }
    if sample().is_some() {
        assert!(faces > 0);
        assert!(
            !glb_ids.is_empty(),
            "the glb's node extras carry ElementIds"
        );
        assert!(
            u.nodes
                .iter()
                .any(|n| n.2.iter().any(|(k, _)| k == "ElementId"))
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}
