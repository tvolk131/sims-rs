use serde::Deserialize;
use sims_sim::pose::portrait::view::{Input, View, inverse};

#[derive(Deserialize)]
struct Case {
    input: Input,
    original: Native,
}
#[derive(Deserialize)]
struct Native {
    #[serde(flatten)]
    view: View,
    portrait_light: [f32; 3],
}
#[derive(Deserialize)]
struct Report {
    cases: Vec<Case>,
}
fn check(actual: &[f32], expected: &[f32], case: usize, name: &str) {
    for (i, (&a, &b)) in actual.iter().zip(expected).enumerate() {
        assert_eq!(
            a,
            b,
            "case {case}, {name}[{i}], actual bits {:08x}, expected {:08x}",
            a.to_bits(),
            b.to_bits()
        );
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_house_camera_preserves_viewport_scroll_zoom_rotation_and_inverse() {
    let report: Report = serde_json::from_str(include_str!("fixtures/house-camera.json")).unwrap();
    assert_eq!(report.cases.len(), 180);
    for (n, c) in report.cases.into_iter().enumerate() {
        let got = c.input.build().unwrap();
        let want = c.original.view;
        check(
            &got.portrait_light().unwrap(),
            &c.original.portrait_light,
            n,
            "portrait light",
        );
        check(
            &[got.far, got.distance],
            &[want.far, want.distance],
            n,
            "depth",
        );
        check(&got.direction, &want.direction, n, "direction");
        check(&got.position, &want.position, n, "position");
        check(&got.angles, &want.angles, n, "angles");
        for (name, a, b) in [
            ("view", got.view, want.view),
            ("projection", got.projection, want.projection),
            ("screen", got.screen, want.screen),
            ("inverse", got.inverse, want.inverse),
            ("rotation", got.rotation_matrix, want.rotation_matrix),
        ] {
            check(&a.values, &b.values, n, name);
            assert_eq!(a.kind, b.kind);
        }
        let restored: View = serde_json::from_slice(&serde_json::to_vec(&got).unwrap()).unwrap();
        assert_eq!(restored, got);
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_camera_matrix_inverse_matches_each_native_screen_matrix() {
    let report: Report = serde_json::from_str(include_str!("fixtures/house-camera.json")).unwrap();
    for (n, c) in report.cases.into_iter().enumerate() {
        let got = inverse(c.original.view.screen).unwrap();
        check(&got.values, &c.original.view.inverse.values, n, "inverse");
        assert_eq!(got.kind, c.original.view.inverse.kind);
    }
}

#[test]
fn unsupported_camera_inputs_and_singular_matrix_are_explicit_errors() {
    let input = Input {
        viewport: [0, 0, 800, 600],
        center: [0, 0],
        zoom: 1,
        rotation: 0,
    };
    for bad in [
        Input { zoom: 0, ..input },
        Input { zoom: 4, ..input },
        Input {
            rotation: 4,
            ..input
        },
        Input {
            viewport: [0, 0, 0, 600],
            ..input
        },
        Input {
            center: [i32::MIN, 0],
            ..input
        },
    ] {
        assert!(bad.build().is_err());
    }
    let mut singular = sims_sim::pose::model::Matrix::IDENTITY;
    singular.values[0] = 0.;
    assert!(inverse(singular).is_err());
    singular.values[0] = f32::NAN;
    assert!(inverse(singular).is_err());
}
