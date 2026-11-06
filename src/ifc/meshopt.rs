//! lossless `EXT_meshopt_compression` for glTF.
//!
//! The vertex codec (version 0) and the index-sequence codec (version 1) of
//! meshoptimizer, as the extension specifies them
//! (KhronosGroup/glTF `extensions/2.0/Vendor/EXT_meshopt_compression`), with
//! no filter: decoding gives back every byte. three.js decodes it with
//! `GLTFLoader.setMeshoptDecoder(MeshoptDecoder)`.

use super::gltf::{Buffer, GltfDocument};
use serde_json::json;

const BYTE_GROUP: usize = 16;
const BLOCK_MAX: usize = 256;
const TAIL_MIN: usize = 32;

fn zigzag8(v: u8) -> u8 {
    ((v as i8 >> 7) as u8) ^ (v << 1)
}

/// Bytes `encode_group` writes for 16 values at `bits` (0, 2, 4 or 8).
fn group_size(values: &[u8], bits: u32) -> usize {
    match bits {
        0 => {
            if values.iter().all(|&v| v == 0) {
                0
            } else {
                usize::MAX
            }
        }
        8 => BYTE_GROUP,
        _ => {
            let sentinel = (1u8 << bits) - 1;
            BYTE_GROUP * bits as usize / 8 + values.iter().filter(|&&v| v >= sentinel).count()
        }
    }
}

fn encode_group(out: &mut Vec<u8>, values: &[u8], bits: u32) {
    match bits {
        0 => {}
        8 => out.extend_from_slice(values),
        _ => {
            let sentinel = (1u8 << bits) - 1;
            let per_byte = 8 / bits as usize;
            for chunk in values.chunks(per_byte) {
                let mut byte = 0u8;
                for &v in chunk {
                    byte = (byte << bits) | v.min(sentinel);
                }
                out.push(byte);
            }
            out.extend(values.iter().copied().filter(|&v| v >= sentinel));
        }
    }
}

fn encode_bytes(out: &mut Vec<u8>, buffer: &[u8]) {
    debug_assert!(buffer.len().is_multiple_of(BYTE_GROUP));
    let groups = buffer.len() / BYTE_GROUP;
    let header_at = out.len();
    out.resize(header_at + groups.div_ceil(4), 0);
    for (g, values) in buffer.chunks(BYTE_GROUP).enumerate() {
        let (mut best, mut best_size) = (3u8, BYTE_GROUP);
        for (log, bits) in [(0u8, 0u32), (1, 2), (2, 4)] {
            let size = group_size(values, bits);
            if size < best_size {
                (best, best_size) = (log, size);
            }
        }
        out[header_at + g / 4] |= best << ((g % 4) * 2);
        encode_group(out, values, [0, 2, 4, 8][best as usize]);
    }
}

/// meshopt_encodeVertexBuffer, version 0.
pub fn encode_vertex_buffer(data: &[u8], count: usize, stride: usize) -> Vec<u8> {
    assert!(stride > 0 && stride <= 256 && stride.is_multiple_of(4));
    assert_eq!(data.len(), count * stride);
    let mut out = vec![0xa0];
    let mut last: Vec<u8> = data.get(..stride).map_or(vec![0; stride], <[u8]>::to_vec);
    let first = last.clone();
    let block = ((8192 / stride) & !(BYTE_GROUP - 1)).min(BLOCK_MAX);
    let mut buffer = vec![0u8; BLOCK_MAX];
    let mut start = 0;
    while start < count {
        let n = block.min(count - start);
        let aligned = n.div_ceil(BYTE_GROUP) * BYTE_GROUP;
        for k in 0..stride {
            let mut p = last[k];
            for (i, slot) in buffer[..n].iter_mut().enumerate() {
                let v = data[(start + i) * stride + k];
                *slot = zigzag8(v.wrapping_sub(p));
                p = v;
            }
            let pad = buffer[n - 1];
            buffer[n..aligned].fill(pad);
            encode_bytes(&mut out, &buffer[..aligned]);
        }
        last.copy_from_slice(&data[(start + n - 1) * stride..(start + n) * stride]);
        start += n;
    }
    out.resize(out.len() + TAIL_MIN.saturating_sub(stride), 0);
    out.extend_from_slice(&first);
    out
}

fn vbyte(out: &mut Vec<u8>, mut v: u32) {
    loop {
        let byte = (v & 127) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 128);
    }
}

/// meshopt_encodeIndexSequence, version 1 (one baseline).
pub fn encode_index_sequence(indices: &[u32]) -> Vec<u8> {
    let mut out = vec![0xd1];
    let mut last = 0u32;
    for &index in indices {
        let d = index.wrapping_sub(last);
        let v = (d << 1) ^ ((d as i32 >> 31) as u32);
        vbyte(&mut out, v << 1);
        last = index;
    }
    out.extend_from_slice(&[0; 4]);
    out
}

/// Compress every vertex and index buffer view of `doc` (whose data is
/// `bin`) and return the new binary chunk. Views of other shapes stay as
/// they are.
pub fn compress_document(doc: &mut GltfDocument, bin: &[u8]) -> Vec<u8> {
    // Element size and component size of each view, from its accessor.
    let mut shape: Vec<Option<(usize, usize)>> = vec![None; doc.buffer_views.len()];
    for a in &doc.accessors {
        let component = match a.component_type {
            5120 | 5121 => 1,
            5122 | 5123 => 2,
            _ => 4,
        };
        let width = match a.type_.as_str() {
            "VEC2" => 2,
            "VEC3" => 3,
            "VEC4" => 4,
            "MAT4" => 16,
            _ => 1,
        };
        shape[a.buffer_view] = Some((component * width, component));
    }
    let mut out = Vec::with_capacity(bin.len() / 2);
    let mut fallback = 0usize;
    for (view, shape) in doc.buffer_views.iter_mut().zip(shape) {
        let data = &bin[view.byte_offset..view.byte_offset + view.byte_length];
        let index_view = view.target == Some(34963);
        let encoded = match shape {
            Some((_, c)) if index_view && (c == 2 || c == 4) => {
                let indices: Vec<u32> = data
                    .chunks_exact(c)
                    .map(|b| match c {
                        2 => u32::from(u16::from_le_bytes([b[0], b[1]])),
                        _ => u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                    })
                    .collect();
                Some((encode_index_sequence(&indices), "INDICES", c, indices.len()))
            }
            Some((e, _))
                if !index_view && e % 4 == 0 && e <= 256 && data.len().is_multiple_of(e) =>
            {
                Some((
                    encode_vertex_buffer(data, data.len() / e, e),
                    "ATTRIBUTES",
                    e,
                    data.len() / e,
                ))
            }
            _ => None,
        };
        let at = out.len();
        match encoded {
            Some((bytes, mode, stride, count)) => {
                out.extend_from_slice(&bytes);
                view.extensions = Some(json!({"EXT_meshopt_compression": {
                    "buffer": 0, "byteOffset": at, "byteLength": bytes.len(),
                    "byteStride": stride, "mode": mode, "count": count
                }}));
                view.buffer = 1;
                view.byte_offset = fallback;
                fallback += view.byte_length.next_multiple_of(4);
            }
            None => {
                out.extend_from_slice(data);
                view.byte_offset = at;
            }
        }
        while !out.len().is_multiple_of(4) {
            out.push(0);
        }
    }
    doc.buffers = vec![
        Buffer {
            byte_length: out.len(),
            uri: None,
            extensions: None,
        },
        Buffer {
            byte_length: fallback.max(4),
            uri: None,
            extensions: Some(json!({"EXT_meshopt_compression": {"fallback": true}})),
        },
    ];
    for list in [&mut doc.extensions_used, &mut doc.extensions_required] {
        list.push("EXT_meshopt_compression".into());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_sequence_is_zigzag_vbytes() {
        // 0, 1, 0, 300: deltas 0, +1, -1, +300.
        let e = encode_index_sequence(&[0, 1, 0, 300]);
        assert_eq!(e[0], 0xd1);
        assert_eq!(&e[1..4], &[0, 4, 2]);
        // 300 -> zigzag 600 -> <<1 1200 = 0b1001_0110000 -> 0xb0 0x09
        assert_eq!(&e[4..6], &[0xb0, 0x09]);
        assert_eq!(e.len(), 6 + 4);
    }

    #[test]
    fn vertex_buffer_ends_with_its_first_vertex() {
        let data: Vec<u8> = (0..12 * 40).map(|i| (i * 7 % 251) as u8).collect();
        let e = encode_vertex_buffer(&data, 40, 12);
        assert_eq!(e[0], 0xa0);
        assert_eq!(&e[e.len() - 12..], &data[..12]);
    }
}
