//! A body the glTF exporter draws whole in its category's colour, made only of transparent
//! materials, takes their alpha, in a material named `Category <class> transparent`.
//!
//! Real file: Autodesk's `rac_basic_sample_family` (Revit 2024). No public model reaches that
//! rule (the exporter draws their glass from their own materials, part by part), so this checks
//! the shape of any such material the output holds, and that the output is still a valid glb.
//! Skips when the corpus is absent.

mod common;

#[test]
fn a_transparent_category_material_is_blended_and_marked_as_the_files_alpha() {
    let input = common::sample_for_year(2024);
    if !input.exists() {
        eprintln!("skipping: {} absent", input.display());
        return;
    }
    let dir = std::env::temp_dir().join(format!("rvt-gltf-alpha-{}", std::process::id()));
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
    let _ = std::fs::remove_dir_all(&dir);
    let jl = u32::from_le_bytes(g[12..16].try_into().unwrap()) as usize;
    let j: serde_json::Value = serde_json::from_slice(&g[20..20 + jl]).unwrap();
    let materials = j["materials"].as_array().unwrap();
    let transparent: Vec<usize> = materials
        .iter()
        .enumerate()
        .filter(|(_, m)| {
            m["name"]
                .as_str()
                .is_some_and(|n| n.starts_with("Category ") && n.ends_with(" transparent"))
        })
        .map(|(i, _)| i)
        .collect();
    for &i in &transparent {
        let m = &materials[i];
        assert_eq!(m["alphaMode"], "BLEND");
        assert_eq!(m["extras"]["alphaSource"], "file");
        assert_eq!(m["extras"]["colourSource"], "rvt-open");
        assert!(
            m["pbrMetallicRoughness"]["baseColorFactor"][3]
                .as_f64()
                .unwrap()
                < 1.0
        );
    }
}
