use anyhow::Result;
use glam::Vec3;
use sims_render::Camera;
use sims_sim::pose::portrait::view::Input;

#[test]
fn replacement_scene_camera_tracks_original_projection_at_all_zooms_and_rotations() -> Result<()> {
    let mut maximum_error = 0.0_f32;
    for viewport in [[0, 0, 1000, 740], [0, 0, 800, 600], [0, 0, 1920, 1080]] {
        for center in [[500, 370], [-317, 203], [1013, -449]] {
            for zoom in 1..=3 {
                for rotation in 0..4 {
                    let input = Input {
                        viewport,
                        center,
                        zoom,
                        rotation,
                    };
                    let native = input.build().map_err(anyhow::Error::msg)?;
                    for level in 1..=2 {
                        let camera = Camera::from_native(input, level)?;
                        for p in [
                            Vec3::ZERO,
                            Vec3::new(32., 0., 32.),
                            Vec3::new(19.5, 3., 42.25),
                            Vec3::new(63., 0.875, 63.),
                            Vec3::new(8., -1., 12.),
                        ] {
                            let expected = native
                                .screen
                                .point([p.z * 3., p.y * 3., p.x * 3.])
                                .map_err(anyhow::Error::msg)?;
                            let actual = camera.screen(p);
                            for axis in 0..2 {
                                maximum_error =
                                    maximum_error.max((expected[axis] - actual[axis]).abs());
                                assert!(
                                    (expected[axis] - actual[axis]).abs() < 0.25,
                                    "{input:?} {p:?}: native={expected:?}, actual={actual:?}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    println!(
        "Original-matrix screen projection: maximum error {maximum_error} pixels (quarter-pixel bound)"
    );
    Ok(())
}
