//! `rvt::ifc::sheet::plan_drawing` drawing one plan view: walls cut at the openings the cut plane
//! crosses, the categories and elements the view hides left out, and layers by Revit category.
#![cfg(feature = "dwg")]

use rvt::ifc::IfcModel;
use rvt::ifc::entities::{Extrusion, IfcEntity, Property, PropertySet, PropertyValue};
use rvt::ifc::sheet::{SheetOptions, ViewWindow, plan_drawing};
use rvt_dwg::Geom;

fn element(
    ifc_type: &str,
    id: &str,
    category: &str,
    at: [f64; 3],
    box_feet: [f64; 3],
    host: Option<usize>,
    external: Option<bool>,
) -> IfcEntity {
    let mut properties = vec![Property {
        name: "Category".into(),
        value: PropertyValue::Label(category.into()),
    }];
    if let Some(e) = external {
        properties.push(Property {
            name: "IsExternal".into(),
            value: PropertyValue::Boolean(e),
        });
    }
    IfcEntity::BuildingElement {
        ifc_type: ifc_type.into(),
        name: format!("{category} {id}"),
        type_guid: Some(id.into()),
        predefined_type: None,
        storey_index: Some(0),
        material_index: None,
        property_set: Some(PropertySet {
            name: "Other".into(),
            properties,
        }),
        location_feet: Some(at),
        rotation_radians: None,
        extrusion: Some(Extrusion::rectangle(box_feet[0], box_feet[1], box_feet[2])),
        host_element_index: host,
        material_layer_set_index: None,
        material_profile_set_index: None,
        solid_shape: None,
        representation_map_index: None,
    }
}

#[test]
fn a_plan_view_cuts_walls_at_openings_and_leaves_out_what_it_hides() {
    let model = IfcModel {
        entities: vec![
            // An interior wall 10 ft long, with a window across the cut plane (4 ft) and a
            // door-height opening that ends below it.
            element("IFCWALL", "1", "Walls", [0.0, 0.0, 0.0], [10.0, 1.0, 8.0], None, Some(false)),
            element("IFCWINDOW", "2", "Windows", [0.0, 0.0, 3.0], [2.0, 1.0, 2.0], Some(0), None),
            element("IFCWINDOW", "3", "Windows", [3.0, 0.0, 0.5], [1.0, 1.0, 1.0], Some(0), None),
            element("IFCBUILDINGELEMENTPROXY", "4", "Planting", [20.0, 0.0, 0.0], [1.0, 1.0, 1.0], None, None),
            element("IFCSTAIR", "5", "Stairs", [30.0, 0.0, 0.0], [1.0, 1.0, 1.0], None, None),
            element("IFCFURNITURE", "6", "Furniture", [40.0, 0.0, 0.0], [1.0, 1.0, 1.0], None, None),
        ],
        ..Default::default()
    };
    let options = SheetOptions {
        window: Some(ViewWindow {
            bottom_feet: 0.0,
            top_feet: 4.0,
            hidden_categories: vec!["Planting".into()],
            hidden_elements: vec![6],
            ..Default::default()
        }),
        ..Default::default()
    };
    let d = plan_drawing(&model, &options);
    let on = |layer: &str| -> Vec<(f64, f64)> {
        d.entities
            .iter()
            .filter(|e| e.layer == layer)
            .filter_map(|e| match &e.geom {
                Geom::Polyline { pts, .. } => Some(pts.iter().fold(
                    (f64::INFINITY, f64::NEG_INFINITY),
                    |(a, b), p| (a.min(p[0]), b.max(p[0])),
                )),
                _ => None,
            })
            .collect()
    };
    // The wall is drawn in two pieces either side of the window the cut plane crosses (the low
    // window stays inside the right piece), on the interior wall layer.
    let mut wall = on("I-WALL");
    wall.sort_by(|a, b| a.0.total_cmp(&b.0));
    assert_eq!(wall.len(), 2, "{wall:?}");
    let mm = 304.8;
    assert!((wall[0].0 + 5.0 * mm).abs() < 1e-6 && (wall[0].1 + 1.0 * mm).abs() < 1e-6, "{wall:?}");
    assert!((wall[1].0 - 1.0 * mm).abs() < 1e-6 && (wall[1].1 - 5.0 * mm).abs() < 1e-6, "{wall:?}");
    assert!(on("A-WALL").is_empty());
    assert_eq!(on("A-GLAZ").len(), 2);
    assert_eq!(on("S-STRS").len(), 1, "stairs on Revit's stairs layer");
    assert!(on("L-PLNT").is_empty() && on("A-GENM").is_empty(), "hidden category");
    assert!(on("I-FURN").is_empty(), "hidden element");
}
