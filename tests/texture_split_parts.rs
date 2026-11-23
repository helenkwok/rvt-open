//! IFC4: a triangle mesh whose faces draw a textured material and a plain one
//! is written as one face set per material. The texture map lays the image on
//! exactly the textured material's triangles, the plain part keeps its colour
//! and no texture, and each part carries its own material's surface style.
//! Synthetic data only. With `RVT_TEXTURE_OUT` set, the first test's output
//! is also written there.

use rvt::ifc::entities::{BrepTriangle, IfcEntity, SolidShape};
use rvt::ifc::{IfcModel, MaterialInfo, MaterialTexture};
use std::collections::BTreeMap;

/// A 2 x 2 RGB PNG.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x08, 0x02, 0x00, 0x00, 0x00, 0xfd, 0xd4, 0x9a,
    0x73, 0x00, 0x00, 0x00, 0x11, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0xf8, 0xcf, 0x00, 0x04,
    0x20, 0xe2, 0x3f, 0x10, 0x00, 0x00, 0x1e, 0xef, 0x05, 0xfb, 0x35, 0x81, 0xa0, 0xc0, 0x00, 0x00,
    0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

fn material(name: &str, colour: u32, png: bool) -> MaterialInfo {
    MaterialInfo {
        name: name.into(),
        color_packed: Some(colour),
        transparency: None,
        texture: png
            .then(|| MaterialTexture::new(PNG.to_vec(), None, Some([2.0, 2.0])))
            .flatten(),
    }
}

/// Two squares, the lower drawn in `low`, the upper in `high` (face
/// materials), four distinct triangles in all.
fn shelf(name: &str, material_index: Option<usize>, low: usize, high: usize) -> IfcEntity {
    let mut v = Vec::new();
    let mut uv = Vec::new();
    for z in [0.0, 1.0] {
        v.extend([[0.0, 0.0, z], [1.0, 0.0, z], [1.0, 1.0, z], [0.0, 1.0, z]]);
        uv.extend([[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]);
    }
    let t = vec![
        BrepTriangle(0, 1, 2),
        BrepTriangle(0, 2, 3),
        BrepTriangle(4, 5, 6),
        BrepTriangle(4, 6, 7),
    ];
    IfcEntity::BuildingElement {
        ifc_type: "IFCBUILDINGELEMENTPROXY".into(),
        name: name.into(),
        type_guid: None,
        predefined_type: Some("NOTDEFINED".into()),
        storey_index: None,
        material_index,
        property_set: None,
        location_feet: None,
        rotation_radians: None,
        extrusion: None,
        host_element_index: None,
        material_layer_set_index: None,
        material_profile_set_index: None,
        solid_shape: Some(SolidShape::TriangulatedFaceSet {
            vertices_feet: v,
            triangles: t,
            face_materials: vec![low, low, high, high],
            uvs: uv,
        }),
        representation_map_index: None,
    }
}

struct Step(BTreeMap<usize, String>);

impl Step {
    fn parse(text: &str) -> Self {
        Step(
            text.lines()
                .filter_map(|l| {
                    let (id, rest) = l.strip_prefix('#')?.split_once('=')?;
                    Some((id.parse().ok()?, rest.trim_end_matches(';').to_string()))
                })
                .collect(),
        )
    }
    fn refs(s: &str) -> Vec<usize> {
        s.split('#')
            .skip(1)
            .filter_map(|p| p.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok())
            .collect()
    }
    /// Entities of `kind` whose attributes reference `id`.
    fn pointing(&self, kind: &str, id: usize) -> Vec<&String> {
        self.0
            .values()
            .filter(|v| v.starts_with(kind) && Self::refs(v).contains(&id))
            .collect()
    }
    /// The face sets of the element named `name`'s Body representation.
    fn body_items(&self, name: &str) -> Vec<usize> {
        let el = self
            .0
            .values()
            .find(|v| v.starts_with("IFCBUILDINGELEMENTPROXY(") && v.contains(&format!("'{name}'")))
            .expect("element");
        let shape = *Self::refs(el).last().unwrap();
        Self::refs(&self.0[&shape])
            .into_iter()
            .filter(|r| self.0[r].contains("'Body'"))
            .flat_map(|r| Self::refs(&self.0[&r]).into_iter().skip(1))
            .collect()
    }
    fn triangles(&self, set: usize) -> usize {
        self.0[&set].matches("),(").count() + 1
    }
    /// The name of the surface style an `IfcStyledItem` gives item `id`.
    fn style_of(&self, id: usize) -> Vec<String> {
        self.0
            .values()
            .filter(|v| v.starts_with(&format!("IFCSTYLEDITEM(#{id},")))
            .map(|v| {
                let style = &self.0[&Self::refs(v)[1]];
                style.split('\'').nth(1).unwrap_or_default().to_string()
            })
            .collect()
    }
}

#[test]
fn a_mixed_mesh_maps_only_its_textured_triangles_and_styles_each_part() {
    let mut m = IfcModel {
        project_name: Some("split".into()),
        materials: vec![
            material("Checker", 0x7f7f7f, true),
            material("Plain", 0x2040c0, false),
        ],
        ..Default::default()
    };
    m.entities.push(shelf("Shelf", None, 0, 1));
    // the element's own material is the plain one, its faces naming none.
    m.entities.push(shelf("Owned", Some(1), 0, usize::MAX));
    let text = rvt::ifc::write_step(&m);
    // With `RVT_TEXTURE_OUT` set, also written there for outside validators.
    if let Some(dir) = std::env::var_os("RVT_TEXTURE_OUT") {
        std::fs::create_dir_all(&dir).expect("out dir");
        std::fs::write(std::path::Path::new(&dir).join("split-parts.ifc"), &text).expect("write");
    }
    let step = Step::parse(&text);
    for name in ["Shelf", "Owned"] {
        let items = step.body_items(name);
        assert_eq!(items.len(), 2, "{name}: one face set per material");
        let mut seen = Vec::new();
        for set in items {
            assert!(
                step.0[&set].starts_with("IFCTRIANGULATEDFACESET("),
                "{name}"
            );
            assert_eq!(step.triangles(set), 2, "{name}");
            let textures = step.pointing("IFCINDEXEDTRIANGLETEXTUREMAP(", set);
            let colours = step.pointing("IFCINDEXEDCOLOURMAP(", set);
            let style = step.style_of(set);
            if textures.is_empty() {
                assert_eq!(colours.len(), 1, "{name}: the plain part keeps its colour");
                assert_eq!(style, ["Plain"], "{name}");
            } else {
                assert_eq!(textures.len(), 1, "{name}");
                assert!(colours.is_empty(), "{name}");
                assert_eq!(style, ["Checker"], "{name}");
            }
            seen.extend(style);
        }
        seen.sort();
        assert_eq!(seen, ["Checker", "Plain"], "{name}");
    }
}

#[test]
fn a_mesh_of_one_material_is_not_split_or_styled_per_item() {
    let mut m = IfcModel {
        materials: vec![material("Checker", 0x7f7f7f, true)],
        ..Default::default()
    };
    m.entities.push(shelf("Whole", None, 0, 0));
    let text = rvt::ifc::write_step(&m);
    let step = Step::parse(&text);
    let items = step.body_items("Whole");
    assert_eq!(items.len(), 1);
    assert_eq!(step.triangles(items[0]), 4);
    assert!(step.style_of(items[0]).is_empty());
    assert_eq!(text.matches("IFCINDEXEDTRIANGLETEXTUREMAP(").count(), 1);
}
