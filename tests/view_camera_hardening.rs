//! Hand-made view records the 3D camera reader must refuse: `Camera::from` given a skewed
//! basis, or a view plane that disagrees with the viewer, reports no camera, and a target off
//! the looking axis is not reported. No real file holds these records; real files drive the
//! same reader through the view catalog in `tests/views.rs`.

use rvt::research::views::Camera;
use serde_json::json;

#[test]
fn camera_needs_an_orthonormal_basis_pointing_at_the_target() {
    let view = json!({
        "m_horzDir": [1.0, 0.0, 0.0],
        "m_vertDir": [0.0, 1.0, 0.0],
        "m_viewDir": [0.0, 0.0, 1.0]
    });
    let fields = json!({
        "m_boundedSpace": {
            "m_orig": [0.0, 0.0, 10.0],
            "m_basis": [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0]]
        },
        "m_targetPos": [0.0, 0.0, 0.0],
        "m_projMethodType": 1,
        "m_projDist": 0.1
    });
    let camera = Camera::from(&fields, &view).expect("camera");
    assert_eq!(camera.target, Some([0.0, 0.0, 0.0]));
    // A target off the axis is not reported, but the camera's basis is.
    let mut wrong = fields.clone();
    wrong["m_targetPos"] = json!([1.0, 0.0, 0.0]);
    assert_eq!(Camera::from(&wrong, &view).expect("camera").target, None);
    // A skewed basis is not a camera.
    let mut skewed = fields.clone();
    skewed["m_boundedSpace"]["m_basis"][1] = json!([0.6, 0.8, 0.0]);
    assert!(Camera::from(&skewed, &view).is_none());
    // The view plane and the viewer must agree: with the good basis, so the view-plane check
    // is what refuses it.
    let inconsistent = json!({
        "m_horzDir": [0.0, 1.0, 0.0],
        "m_vertDir": [0.0, 0.0, 1.0],
        "m_viewDir": [1.0, 0.0, 0.0]
    });
    assert!(Camera::from(&fields, &inconsistent).is_none());
}
