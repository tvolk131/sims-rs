//! New cases are authored input/owner contracts except the separately bound two
//! retained Goth matrices. They do not import Thread/VM controls or run ticks.
use super::*;
fn placement(base: [f32; 3]) -> Placement {
    Placement {
        base,
        previous: [11., 22., 33.],
        offset: [0.; 3],
        dirty: true,
    }
}
fn source() -> Source {
    Source {
        position: [-16, -16],
        facing: 0,
        level: 1,
        flags: [1; 2],
    }
}
fn view() -> View {
    View {
        rotation: 1,
        floor_vector: [0.5, 9., 0.5],
    }
}
fn absent() -> RetainedTerrain<'static> {
    RetainedTerrain {
        extent: None,
        corners: None,
    }
}
fn words(p: &Placement) -> Vec<u32> {
    p.base
        .iter()
        .chain(&p.previous)
        .chain(&p.offset)
        .map(|n| n.to_bits())
        .chain([u32::from(p.dirty)])
        .collect()
}
#[cfg(feature = "private-fixtures")]
#[test]
fn exact_two_retained_goth_ordinary_matrices_use_actual_single_control() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("retained-goth.json")).unwrap();
    assert_eq!(fixture["rows"].as_array().unwrap().len(), 2);
    for row in fixture["rows"].as_array().unwrap() {
        let floats = |key: &str| -> [f32; 3] {
            let b: [u32; 3] = serde_json::from_value(row[key].clone()).unwrap();
            b.map(f32::from_bits)
        };
        // The ordinary graphic temporary has no previous-position owner; these
        // zero scratch words are not asserted as observed person/native fields.
        let mut p = Placement {
            base: floats("base_bits"),
            previous: [0.; 3],
            offset: floats("offset_bits"),
            dirty: row["dirty"].as_bool().unwrap(),
        };
        let s = &row["fallback_source"];
        let s = Source {
            position: [
                s["x_i32"].as_i64().unwrap() as i32,
                s["y_i32"].as_i64().unwrap() as i32,
            ],
            facing: s["facing_word4c_i16"].as_i64().unwrap() as i16,
            level: s["level110_i32"].as_i64().unwrap() as i32,
            flags: [1; 2],
        };
        let v = View {
            rotation: row["rotation"].as_u64().unwrap() as u8,
            floor_vector: floats("floor_vector_bits"),
        };
        let before = p.clone();
        let got = p
            .transform_retained(s, v, absent(), row["fp_control"].as_u64().unwrap() as u16)
            .unwrap();
        let expected: [u32; 16] = serde_json::from_value(row["matrix_bits"].clone()).unwrap();
        assert_eq!(got.matrix.map(f32::to_bits), expected);
        assert_eq!(got.terrain_point, [-15, -15]);
        assert_eq!(got.pool_query, None);
        assert_eq!(p, before);
        let (mut cold, expected): (Placement, Transform) =
            serde_json::from_slice(&serde_json::to_vec(&(p.clone(), got.clone())).unwrap())
                .unwrap();
        assert_eq!(
            cold.transform_retained(s, v, absent(), 0x003f).unwrap(),
            expected
        );
        assert_eq!(cold, p);
    }
}
#[test]
fn legacy_transform_stays_extended_and_eagerly_requires_valid_terrain() {
    let mut old = placement([-16., -16., 0.]);
    let before = old.clone();
    assert_eq!(
        old.transform(
            source(),
            view(),
            Terrain {
                size: 0,
                corners: &[]
            }
        )
        .unwrap_err(),
        "invalid native animator terrain array"
    );
    assert_eq!(old, before);
    let corners = [[0; 4]; 4];
    let old_result = old
        .transform(
            source(),
            view(),
            Terrain {
                size: 2,
                corners: &corners,
            },
        )
        .unwrap();
    assert_eq!(old_result.matrix[0].to_bits(), 0x324cde2e);
    let mut retained = before;
    let got = retained
        .transform_retained(source(), view(), absent(), 0x003f)
        .unwrap();
    assert_eq!(got.matrix[0].to_bits(), 0x35033379);
}
#[test]
fn actual_first_tested_coordinate_skips_extent_but_other_negative_does_not() {
    let mut p = placement([-16., 0., 0.]);
    p.transform_retained(source(), view(), absent(), 0x003f)
        .unwrap();
    let mut p = placement([0., -16., 0.]);
    let before = p.clone();
    assert!(
        p.transform_retained(source(), view(), absent(), 0x003f)
            .unwrap_err()
            .contains("extent is unknown")
    );
    assert_eq!(p, before);
    p.transform_retained(
        source(),
        view(),
        RetainedTerrain {
            extent: Some(64),
            corners: None,
        },
        0x003f,
    )
    .unwrap();
}
#[test]
fn out_of_bounds_needs_only_the_owners_read_before_native_return() {
    for (base, extent) in [([128., 1., 0.], 8), ([0., 0., 0.], 0), ([1., 128., 0.], 8)] {
        placement(base)
            .transform_retained(
                source(),
                view(),
                RetainedTerrain {
                    extent: Some(extent),
                    corners: None,
                },
                0x003f,
            )
            .unwrap();
    }
    //2048>>4 narrows to signed byte-128; an unused supplied extent/directory
    // must not turn a no-read native branch into eager validation.
    placement([2048., 0., 0.])
        .transform_retained(
            source(),
            view(),
            RetainedTerrain {
                extent: Some(usize::MAX),
                corners: Some(&[]),
            },
            0x003f,
        )
        .unwrap();
}
#[test]
fn missing_or_unsupported_inbounds_owners_reject_without_cache_publication() {
    let corners = [[0; 4]; 4];
    for (terrain, message) in [
        (
            RetainedTerrain {
                extent: Some(2),
                corners: None,
            },
            "directory is unknown",
        ),
        (
            RetainedTerrain {
                extent: Some(2),
                corners: Some(&[]),
            },
            "directory extent",
        ),
        (
            RetainedTerrain {
                extent: Some(2),
                corners: Some(&corners),
            },
            "scale-cache owner",
        ),
        (
            RetainedTerrain {
                extent: Some(usize::MAX),
                corners: None,
            },
            "signed native domain",
        ),
    ] {
        let mut p = placement([-999.; 3]);
        let before = p.clone();
        let s = Source {
            position: [0; 2],
            ..source()
        };
        assert!(
            p.transform_retained(s, view(), terrain, 0x003f)
                .unwrap_err()
                .contains(message)
        );
        assert_eq!(p, before);
    }
}
#[test]
fn unsupported_controls_bad_views_and_conversions_are_atomic() {
    for control in [0, 0x037f, 0xffff] {
        let mut p = placement([-999.; 3]);
        let before = p.clone();
        assert!(
            p.transform_retained(source(), view(), absent(), control)
                .unwrap_err()
                .contains("x87 control")
        );
        assert_eq!(p, before);
    }
    let mut p = placement([-999.; 3]);
    let before = p.clone();
    assert!(
        p.transform_retained(
            source(),
            View {
                rotation: 4,
                ..view()
            },
            absent(),
            0x003f
        )
        .is_err()
    );
    assert_eq!(p, before);
    let mut p = placement([-16., 2147483648., 0.]);
    let before = p.clone();
    assert!(
        p.transform_retained(source(), view(), absent(), 0x003f)
            .unwrap_err()
            .contains("conversion overflow")
    );
    assert_eq!(p, before);
    let mut p = placement([-16., -16., f32::MAX]);
    let before = p.clone();
    assert!(
        p.transform_retained(source(), view(), absent(), 0x003f)
            .unwrap_err()
            .contains("transcendental range")
    );
    assert_eq!(p, before);
    let mut p = placement([-999.; 3]);
    p.offset[0] = f32::NAN;
    let before = words(&p);
    assert!(
        p.transform_retained(source(), view(), absent(), 0x003f)
            .is_err()
    );
    assert_eq!(words(&p), before);
}
#[test]
fn single_precision_arithmetic_does_not_invent_early_float_stores() {
    let least = f64::from(f32::from_bits(1));
    let product = retained_single(least * 0.1875);
    assert!(product > 0.);
    assert_eq!((product as f32).to_bits(), 0);
    let a = retained_single(
        retained_single(
            retained_single(retained_single(0. + 6.) * 45.) * f64::from(std::f32::consts::PI),
        ) * f64::from(f32::from_bits(0x3bb60b61)),
    ) as f32;
    assert_eq!(a.to_bits(), 0x4096cbe5);
    let (s, c) = retained_sin_cos(a).unwrap();
    assert_eq!(s.to_bits(), 0xbf800000);
    assert_eq!(c.to_bits(), 0x35033379);
}
