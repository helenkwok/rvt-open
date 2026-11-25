//! The built-in parameter catalog is filled from public sources only, and each field says which.
//!
//! A field is filled exactly where its source code (`R` the Revit 2027 API reference, `F` the values
//! public files store, `E` Revit's IFC export of public samples, `H` machine-matched help terms,
//! `-` none) says a source gave it. The count of fields each source fills is pinned, so a field
//! filled from anywhere else (the Revit API dump of the public samples is the answer key only:
//! `tools/builtin-parameters/score.py`) changes a count and fails here.

use rvt::builtin_parameters::{
    LOCALIZED_LANGUAGES, enum_text, localized_name, parameter, parameter_ids,
};
use std::collections::BTreeMap;
use std::path::Path;

/// Entries, then fields filled per source: name, enum name (API reference), storage type
/// (files), spec and group (IFC export), localized names (help).
const ENTRIES: usize = 3812;
const FILLED: [usize; 6] = [3805, 3805, 486, 412, 670, LOCALIZED];
/// Ids with a help term in at least one language, and the (id, language) terms in all: the terms
/// the Revit API dump of the public samples shows wrong in German, Japanese or Chinese are left out.
const LOCALIZED: usize = 1455;
const LOCALIZED_TERMS: usize = 16934;

#[test]
fn every_field_is_from_its_public_source_or_empty() {
    let mut filled = [0usize; 6];
    let mut entries = 0;
    for id in parameter_ids() {
        entries += 1;
        let p = parameter(id).expect("listed id");
        let codes: Vec<char> = p.sources.chars().collect();
        assert_eq!(codes.len(), 6, "{id}: {}", p.sources);
        let has_help = LOCALIZED_LANGUAGES
            .iter()
            .any(|l| localized_name(id, l).is_some());
        let values = [
            !p.name.is_empty(),
            !p.enum_name.is_empty(),
            !p.storage_type.is_empty(),
            !p.spec_type_id.is_empty(),
            !p.group.is_empty(),
            has_help,
        ];
        for (k, (&on, allowed)) in values
            .iter()
            .zip(['R', 'R', 'F', 'E', 'E', 'H'])
            .enumerate()
        {
            let want = if on { allowed } else { '-' };
            assert_eq!(codes[k], want, "{id} field {k}: {}", p.sources);
            filled[k] += usize::from(on);
        }
    }
    assert_eq!(entries, ENTRIES);
    assert_eq!(filled, FILLED);
}

#[test]
fn ids_no_public_source_names_are_kept_with_no_name() {
    for id in [
        -1_154_658, -1_154_657, -1_140_313, -1_017_024, -1_017_023, -1_006_800, -1_005_051,
    ] {
        let p = parameter(id).expect("kept id");
        assert_eq!((p.name, p.enum_name), ("", ""), "{id}");
        assert_eq!(&p.sources[..2], "--", "{id}");
    }
}

/// The rows of a tab-separated public input of `tools/builtin-parameters/inputs/`, without its comments.
fn input(name: &str) -> Vec<Vec<String>> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tools/builtin-parameters/inputs")
        .join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| l.split('\t').map(str::to_string).collect())
        .collect()
}

/// The spec type id of an IFC value type of Revit's export.
fn spec_of(ifc_type: &str) -> &'static str {
    match ifc_type {
        "IFCLENGTHMEASURE" => "autodesk.spec.aec:length-2.0.0",
        "IFCAREAMEASURE" => "autodesk.spec.aec:area-2.0.0",
        "IFCVOLUMEMEASURE" => "autodesk.spec.aec:volume-2.0.0",
        "IFCPLANEANGLEMEASURE" => "autodesk.spec.aec:angle-2.0.0",
        "IFCBOOLEAN" => "autodesk.spec:spec.bool-1.0.0",
        "IFCINTEGER" => "autodesk.spec:spec.int64-2.0.0",
        "IFCREAL" => "autodesk.spec.aec:number-2.0.0",
        "IFCELECTRICCURRENTMEASURE" => "autodesk.spec.aec.electrical:current-2.0.0",
        "IFCELECTRICVOLTAGEMEASURE" => "autodesk.spec.aec.electrical:potential-2.0.0",
        "IFCPOWERMEASURE" => "autodesk.spec.aec.electrical:power-2.0.0",
        "IFCVOLUMETRICFLOWRATEMEASURE" => "autodesk.spec.aec.hvac:airFlow-2.0.0",
        "IFCPRESSUREMEASURE" => "autodesk.spec.aec.hvac:pressure-2.0.0",
        _ => "",
    }
}

#[test]
fn every_value_is_a_row_of_a_public_input() {
    let mut reference: BTreeMap<i64, (String, String)> = BTreeMap::new();
    for r in input("bip_enum.tsv") {
        let id: i64 = r[1].parse().expect("id");
        reference
            .entry(id)
            .or_insert((r[0].clone(), r.get(2).cloned().unwrap_or_default()));
    }
    let storage: BTreeMap<i64, String> = input("storage.tsv")
        .into_iter()
        .map(|r| (r[0].parse().expect("id"), r[1].clone()))
        .collect();
    let group_spec: BTreeMap<i64, (String, String)> = input("group-spec.tsv")
        .into_iter()
        .map(|r| (r[0].parse().expect("id"), (r[1].clone(), r[2].clone())))
        .collect();
    for id in parameter_ids() {
        let p = parameter(id).expect("listed id");
        let (enum_name, label) = reference.get(&id).cloned().unwrap_or_default();
        assert_eq!(
            (p.name, p.enum_name),
            (label.as_str(), enum_name.as_str()),
            "{id}"
        );
        assert_eq!(
            p.storage_type,
            storage.get(&id).map_or("", String::as_str),
            "{id}"
        );
        let (group, ifc_type) = group_spec.get(&id).cloned().unwrap_or_default();
        assert_eq!(
            (p.group, p.spec_type_id),
            (group.as_str(), spec_of(&ifc_type)),
            "{id}"
        );
    }
    for r in input("enum-texts.tsv") {
        let (id, value): (i64, i64) = (r[0].parse().expect("id"), r[1].parse().expect("value"));
        assert_eq!(enum_text(id, value), Some(r[2].as_str()), "{id} {value}");
    }
    let mut terms = 0;
    for r in input("help-terms.tsv") {
        let id: i64 = r[0].parse().expect("id");
        assert_eq!(
            localized_name(id, &r[1]),
            Some((r[2].as_str(), r[3].as_str())),
            "{id} {}",
            r[1]
        );
        terms += 1;
    }
    assert_eq!(terms, LOCALIZED_TERMS);
}

/// A storey whose name holds a tab is written to `names.tsv` as the same name `ifcjoin.py` writes in
/// the option texts (tab as a space), so `derive_ifc.py` drops `Up to level: A B`, not keeps it.
#[test]
fn a_model_name_with_a_tab_is_still_dropped_from_the_option_texts() {
    use std::process::Command;
    if Command::new("python3").arg("--version").output().is_err() {
        eprintln!("skipping: no python3");
        return;
    }
    let tools = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/builtin-parameters");
    let dir = std::env::temp_dir().join(format!("rvt-open-names-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (ifc, census, pairs) = (dir.join("ifc"), dir.join("census"), dir.join("pairs"));
    for d in [&ifc, &census, &pairs] {
        std::fs::create_dir_all(d).expect("dir");
    }
    std::fs::write(
        ifc.join("m.ifc"),
        "ISO-10303-21;\nDATA;\n#1=IFCBUILDINGSTOREY('g',$,'A\\X2\\0009\\X0\\B',$,$,$,$,$,.ELEMENT.,0.);\nENDSEC;\n",
    )
    .expect("ifc");
    std::fs::write(dir.join("bip.tsv"), "# name\tvalue\tlabel\n").expect("bip");
    // An empty census file: gzip of nothing.
    let gz = Command::new("python3")
        .args([
            "-c",
            "import gzip,sys;sys.stdout.buffer.write(gzip.compress(b''))",
        ])
        .output()
        .expect("gzip");
    std::fs::write(census.join("m.tsv.gz"), gz.stdout).expect("census");
    let join = Command::new("python3")
        .arg(tools.join("ifcjoin.py"))
        .args([&ifc, &census, &pairs, &dir.join("bip.tsv")])
        .output()
        .expect("ifcjoin");
    assert!(
        join.status.success(),
        "{}",
        String::from_utf8_lossy(&join.stderr)
    );
    let names = std::fs::read_to_string(pairs.join("m.names.tsv")).expect("names");
    assert!(names.contains("storey\tA B\n"), "{names:?}");
    // The sample's option text embeds that storey: it must be dropped.
    std::fs::write(pairs.join("m.props.tsv"), "#set\tprop\tifctype\tn\n").expect("props");
    std::fs::write(
        pairs.join("m.pairs.tsv"),
        "#tag\tset\tprop\tifctype\tifcvalue\tparam_id\torigin\tfiletype\tfilevalue\tmethod\n1\tc\tp\tIFCLABEL\tUp to level: A B\t-1001103\tS\tInteger\t-1\tL\n",
    )
    .expect("pairs");
    let out = dir.join("out");
    let derive = Command::new("python3")
        .arg(tools.join("derive_ifc.py"))
        .args([&pairs, &dir.join("bip.tsv"), &out])
        .output()
        .expect("derive_ifc");
    assert!(
        derive.status.success(),
        "{}",
        String::from_utf8_lossy(&derive.stderr)
    );
    let texts = std::fs::read_to_string(dir.join("out.enum-texts.tsv")).expect("texts");
    assert!(!texts.contains("A B"), "{texts:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A label several ids share goes to the id whose enum name starts with the element's category
/// (`DOOR_` for `Doors`), a property set that is a schedule never votes, and the
/// newest release's naming wins (`Mechanical` became `Mechanical - Flow`).
#[test]
fn category_newest_release_and_schedules_decide_the_group_and_spec() {
    use std::process::Command;
    if Command::new("python3").arg("--version").output().is_err() {
        eprintln!("skipping: no python3");
        return;
    }
    let tools = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/builtin-parameters");
    let dir = std::env::temp_dir().join(format!("rvt-open-category-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let pairs = dir.join("pairs");
    std::fs::create_dir_all(&pairs).expect("dir");
    let write = |name: &str, text: &str| std::fs::write(pairs.join(name), text).expect(name);
    for m in ["a", "b", "c"] {
        write(&format!("{m}.pairs.tsv"), "#tag\n");
        write(&format!("{m}.names.tsv"), "#kind\tname\n");
    }
    // a: release 2018; b and c: release 2024. `Door Schedule` is in the newest samples with more votes.
    write(
        "a.cprops.tsv",
        "#c\nDoors\tDimensions\tHeight\tIFCLENGTHMEASURE\t3\nDoors\tDoor Schedule\tHeight\tIFCREAL\t9\n",
    );
    let newest = "#c\nDoors\tDimensions\tHeight\tIFCLENGTHMEASURE\t1\nDoors\tDoor Schedule\tHeight\tIFCREAL\t9\n";
    write("b.cprops.tsv", newest);
    write("c.cprops.tsv", newest);
    write(
        "a.props.tsv",
        "#s\nMechanical\tThing\tIFCLENGTHMEASURE\t5\nDimensions\tHeight\tIFCLENGTHMEASURE\t3\nDoor Schedule\tHeight\tIFCREAL\t9\n",
    );
    write(
        "b.props.tsv",
        "#s\nMechanical - Flow\tThing\tIFCLENGTHMEASURE\t1\nDimensions\tHeight\tIFCLENGTHMEASURE\t1\nDoor Schedule\tHeight\tIFCREAL\t9\n",
    );
    write(
        "c.props.tsv",
        "#s\nMechanical - Flow\tThing\tIFCLENGTHMEASURE\t1\nMechanical\tThing\tIFCLENGTHMEASURE\t1\nDimensions\tHeight\tIFCLENGTHMEASURE\t1\nDoor Schedule\tHeight\tIFCREAL\t9\n",
    );
    std::fs::write(dir.join("releases.tsv"), "a\t2018\nb\t2024\nc\t2024\n").expect("releases");
    std::fs::write(
        dir.join("bip.tsv"),
        "DOOR_HEIGHT\t-1001300\tHeight\nGENERIC_HEIGHT\t-1001301\tHeight\nWALL_THING\t-1001400\tThing\n",
    )
    .expect("bip");
    let derive = Command::new("python3")
        .arg(tools.join("derive_ifc.py"))
        .args([&pairs, &dir.join("bip.tsv"), &dir.join("out")])
        .arg("--releases")
        .arg(dir.join("releases.tsv"))
        .output()
        .expect("derive_ifc");
    assert!(
        derive.status.success(),
        "{}",
        String::from_utf8_lossy(&derive.stderr)
    );
    let table = std::fs::read_to_string(dir.join("out.group-spec.tsv")).expect("table");
    assert!(
        table.contains("-1001300\tDimensions\tIFCLENGTHMEASURE\tcategory+label\n"),
        "{table:?}"
    );
    assert!(!table.contains("-1001301"), "{table:?}");
    assert!(
        table.contains("-1001400\tMechanical - Flow\tIFCLENGTHMEASURE\tlabel\n"),
        "{table:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A sample `--releases` does not list takes the release of its `__pf<year>` name, a `-<variant>`
/// suffix included: its newer votes decide over an older listed sample's.
#[test]
fn an_unlisted_sample_takes_the_release_of_its_saved_name() {
    use std::process::Command;
    if Command::new("python3").arg("--version").output().is_err() {
        eprintln!("skipping: no python3");
        return;
    }
    let tools = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/builtin-parameters");
    let dir = std::env::temp_dir().join(format!("rvt-open-pf-release-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let pairs = dir.join("pairs");
    std::fs::create_dir_all(&pairs).expect("dir");
    let write = |name: &str, text: &str| std::fs::write(pairs.join(name), text).expect(name);
    for (m, set, n) in [("a", "Mechanical", 9), ("b__pf2024-variant", "Mechanical - Flow", 1)] {
        write(&format!("{m}.pairs.tsv"), "#tag\n");
        write(&format!("{m}.names.tsv"), "#kind\tname\n");
        write(&format!("{m}.props.tsv"), &format!("#s\n{set}\tThing\tIFCLENGTHMEASURE\t{n}\n"));
    }
    std::fs::write(dir.join("releases.tsv"), "a\t2018\n").expect("releases");
    std::fs::write(dir.join("bip.tsv"), "WALL_THING\t-1001400\tThing\n").expect("bip");
    let derive = Command::new("python3")
        .arg(tools.join("derive_ifc.py"))
        .args([&pairs, &dir.join("bip.tsv"), &dir.join("out")])
        .arg("--releases")
        .arg(dir.join("releases.tsv"))
        .output()
        .expect("derive_ifc");
    assert!(
        derive.status.success(),
        "{}",
        String::from_utf8_lossy(&derive.stderr)
    );
    let table = std::fs::read_to_string(dir.join("out.group-spec.tsv")).expect("table");
    assert!(
        table.contains("-1001400\tMechanical - Flow\tIFCLENGTHMEASURE\tlabel\n"),
        "{table:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
