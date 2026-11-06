//! every primitive of an `rvt-gltf` .glb carries NORMAL (a viewer
//! that does not compute flat normals draws a mesh without them black), with
//! as many normals as positions and each of unit length.

use std::path::{Path, PathBuf};

fn sample() -> Option<PathBuf> {
    // a model with geometry: the Autodesk 2024 sample when the corpus is there
    let dir = std::env::var_os("RVT_REVIT2024_SAMPLES_DIR").map(PathBuf::from)?;
    let p = dir.join("racbasicsampleproject.rvt");
    p.exists().then_some(p)
}

fn tier1() -> Option<PathBuf> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("corpus/tier1/architectural-2024/architectural-2024.rvt");
    p.exists().then_some(p)
}

#[test]
fn every_primitive_has_unit_normals() {
    let Some(input) = sample().or_else(tier1) else {
        eprintln!("skipping: tier-1 fixture missing");
        return;
    };
    let dir = std::env::temp_dir().join(format!("rvt-gltf-normals-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("m.glb");
    let st = std::process::Command::new(env!("CARGO_BIN_EXE_rvt-gltf"))
        .arg(&input)
        .arg("-o")
        .arg(&out)
        .output()
        .expect("spawn rvt-gltf");
    assert!(
        st.status.success(),
        "{}",
        String::from_utf8_lossy(&st.stderr)
    );
    let g = std::fs::read(&out).unwrap();
    let jl = u32::from_le_bytes(g[12..16].try_into().unwrap()) as usize;
    let j: serde_json::Value = serde_json::from_slice(&g[20..20 + jl]).unwrap();
    let bin = &g[20 + jl + 8..];
    let (acc, views) = (
        j["accessors"].as_array().unwrap(),
        j["bufferViews"].as_array().unwrap(),
    );
    let f32s = |a: &serde_json::Value| -> Vec<f32> {
        let v = &views[a["bufferView"].as_u64().unwrap() as usize];
        let off = v["byteOffset"].as_u64().unwrap_or(0) as usize
            + a["byteOffset"].as_u64().unwrap_or(0) as usize;
        let n = a["count"].as_u64().unwrap() as usize * 3;
        (0..n)
            .map(|i| f32::from_le_bytes(bin[off + 4 * i..off + 4 * i + 4].try_into().unwrap()))
            .collect()
    };
    let mut prims = 0;
    for m in j["meshes"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
        for p in m["primitives"].as_array().unwrap() {
            prims += 1;
            let a = &p["attributes"];
            let n = a.get("NORMAL").expect("primitive without NORMAL");
            let pos = &acc[a["POSITION"].as_u64().unwrap() as usize];
            let nrm = &acc[n.as_u64().unwrap() as usize];
            assert_eq!(pos["count"], nrm["count"]);
            for c in f32s(nrm).chunks(3) {
                let l = (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt();
                assert!((l - 1.0).abs() < 1e-3, "normal length {l}");
            }
        }
    }
    if sample().is_some() {
        assert!(prims > 0);
    }
}
