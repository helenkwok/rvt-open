//! GlobalIds and IFCX node paths that do not move when the same
//! model is saved again.
//!
//! BasicFileInfo's `Unique Document GUID` is the newest editing episode, so
//! a save changes it; the GlobalIds generated for entities Revit gives none
//! used to be derived from it. They are now derived from the GUID
//! `Global/History` keeps at `+0x16`, which a save does not change, and
//! IfcProject, IfcSite and IfcBuilding take the GlobalIds Revit's own
//! exporter gives them.
//!
//! Real files: `Snowdon Towers Sample Architectural.rvt` (Revit 2024 sample,
//! `RVT_REVIT2024_SAMPLES_DIR`, with Revit's own IFC export beside it) and its
//! second save `2024/Snowdon_Towers_Sample_Architectural__pf2024.rvt`
//! (`RVT_REVIT_EXTRA_DIR`). Skips when either is absent.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

fn pair() -> Option<(PathBuf, PathBuf)> {
    let a = PathBuf::from(std::env::var_os("RVT_REVIT2024_SAMPLES_DIR")?)
        .join("Snowdon Towers Sample Architectural.rvt");
    let b = PathBuf::from(std::env::var_os("RVT_REVIT_EXTRA_DIR")?)
        .join("2024/Snowdon_Towers_Sample_Architectural__pf2024.rvt");
    (a.exists() && b.exists()).then_some((a, b))
}

/// `rvt-ifc file -o dir/tag.ifc --ifcx dir/tag.ifcx`.
fn export(file: &Path, dir: &Path, tag: &str) -> (String, serde_json::Value) {
    let ifc = dir.join(format!("{tag}.ifc"));
    let ifcx = dir.join(format!("{tag}.ifcx"));
    let r = Command::new(env!("CARGO_BIN_EXE_rvt-ifc"))
        .arg(file)
        .arg("-o")
        .arg(&ifc)
        .arg("--ifcx")
        .arg(&ifcx)
        // the saved graphics and family documents are the file's
        // own content, which a resave does not carry, so the stable-id
        // question is asked of the record boxes both saves share.
        .args([
            "--no-saved-meshes",
            "--no-family-geometry",
            "--no-saved-materials",
            "--no-proxy-shapes",
        ])
        .output()
        .expect("spawn rvt-ifc");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let x = serde_json::from_slice(&std::fs::read(&ifcx).unwrap()).unwrap();
    (std::fs::read_to_string(ifc).unwrap(), x)
}

/// Every rooted entity's GlobalId, with its class: `#n=IFCX('gid',...`.
fn global_ids(ifc: &str) -> Vec<(String, String)> {
    ifc.lines()
        .filter_map(|l| {
            let (_, rest) = l.split_once('=')?;
            let rest = rest.trim_start();
            let (class, args) = rest.split_once("('")?;
            let gid = args.get(..22)?;
            // A GlobalId: 22 characters of IFC's base-64 alphabet, the first
            // 0-3 (128 bits); not a 22-character name such as a material's.
            let alphabet = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '$';
            (args.as_bytes().get(22) == Some(&b'\'')
                && gid.chars().all(alphabet)
                && matches!(gid.as_bytes()[0], b'0'..=b'3'))
            .then(|| (gid.to_string(), class.to_string()))
        })
        .collect()
}

fn paths(x: &serde_json::Value) -> BTreeSet<String> {
    x["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["path"].as_str().unwrap().to_string())
        .collect()
}

/// A GlobalId as the UUID text the IFCX writer uses for its node path.
fn uuid_text(global_id: &str) -> String {
    let v = rvt::research::revit_global_ids::expand_ifc_guid(global_id).unwrap();
    let h: String = v.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    )
}

fn first(ifc: &str, class: &str) -> String {
    global_ids(ifc)
        .into_iter()
        .find(|(_, c)| c == class)
        .unwrap_or_else(|| panic!("no {class}"))
        .0
}

#[test]
fn a_second_save_keeps_every_global_id_and_ifcx_path() {
    let Some((a, b)) = pair() else {
        eprintln!("skip: RVT_REVIT2024_SAMPLES_DIR / RVT_REVIT_EXTRA_DIR not set");
        return;
    };
    let dir = std::env::temp_dir().join(format!("rvt-s326-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (ia, xa) = export(&a, &dir, "a");
    let (ib, xb) = export(&b, &dir, "b");

    // GlobalIds are unique within each file.
    for (tag, ifc) in [("first save", &ia), ("second save", &ib)] {
        let ids = global_ids(ifc);
        let mut seen = BTreeMap::new();
        for (g, c) in &ids {
            if let Some(prev) = seen.insert(g.clone(), c.clone()) {
                panic!("{tag}: GlobalId {g} on {prev} and {c}");
            }
        }
        assert!(ids.len() > 10_000, "{tag}: {} GlobalIds", ids.len());
    }

    // IfcProject, IfcSite and IfcBuilding: Revit's own (its IFC export of
    // the first save), in both saves.
    let revit = std::fs::read_to_string(a.with_extension("ifc")).ok();
    for class in ["IFCPROJECT", "IFCSITE", "IFCBUILDING"] {
        assert_eq!(
            first(&ia, class),
            first(&ib, class),
            "{class} moved on a save"
        );
        if let Some(revit) = &revit {
            let line = revit
                .lines()
                .find(|l| l.contains(&format!("={class}('")))
                .unwrap();
            let at = line.find("('").unwrap() + 2;
            assert_eq!(
                first(&ia, class),
                line[at..at + 22],
                "{class} is not Revit's"
            );
        }
    }

    // The same GlobalIds and the same IFCX node paths in both saves, but for
    // what the second save itself changed: the three type objects of the
    // `Tree - Honey-Locust:15' Canopy` instances take Revit's own GlobalIds
    // (their original symbols' UniqueIds), and the second save holds
    // those symbols under a new editing episode; with them move their
    // IfcRelDefinesByType and their type property sets. Measured:
    // exactly these 12 GlobalIds and the 3 type nodes' paths; since
    // sets: measured 5 sets a type in the first save and 4 in the second,
    // IfcGeographicElementType, whose first-save sets measure 22 (3 + 22 + 3).
    let ga: BTreeMap<_, _> = global_ids(&ia).into_iter().collect();
    let gb: BTreeMap<_, _> = global_ids(&ib).into_iter().collect();
    for (tag, x, y, ifc) in [("first", &ga, &gb, &ia), ("second", &gb, &ga, &ib)] {
        let mut by_class: BTreeMap<&str, usize> = BTreeMap::new();
        let mut type_paths = BTreeSet::new();
        for (g, c) in x.iter().filter(|(g, _)| !y.contains_key(*g)) {
            *by_class.entry(c.as_str()).or_default() += 1;
            if c == "IFCGEOGRAPHICELEMENTTYPE" {
                let line = ifc.lines().find(|l| l.contains(g.as_str())).unwrap();
                assert!(
                    line.contains("'Tree - Honey-Locust:15'' Canopy'"),
                    "{tag} save: type {g} moved: {line}"
                );
                type_paths.insert(uuid_text(g));
            }
        }
        // no base-quantity set or wall body differs; the second
        // save's walls with a stale `WallRunTimeData` curve read as the first.
        // 369 split the canopy family into 10 types by
        // instance mesh, the 7 others each carrying the `Data` set only the
        // type (the instances share meshes exactly instead), so the first
        // save's own sets are back to 15.
        let expected = BTreeMap::from([
            ("IFCGEOGRAPHICELEMENTTYPE", 3),
            ("IFCPROPERTYSET", if tag == "first" { 15 } else { 12 }),
            ("IFCRELDEFINESBYTYPE", 3),
        ]);
        assert_eq!(by_class, expected, "{tag} save: GlobalIds only in it");
        let (p, q) = if tag == "first" {
            (paths(&xa), paths(&xb))
        } else {
            (paths(&xb), paths(&xa))
        };
        let only: BTreeSet<String> = p.difference(&q).cloned().collect();
        assert_eq!(only, type_paths, "{tag} save: IFCX paths only in it");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
