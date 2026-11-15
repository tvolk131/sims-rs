//! Depth: how far a point is from the camera, in sixteenths of a sprite
//! depth byte.
//!
//! The original's object sprites (SPR2) carry a depth byte per pixel, 255
//! for nothing. The scale was fitted from the game's own sprites: for the
//! lowest pixel of each sprite column, which mostly touches the floor, the
//! byte falls 1.5 per screen pixel below the object's anchor at the near
//! zoom, 3 at medium and 6 at far, and is about 200 level with the anchor.
//! So one byte is a fixed distance in the world, and with the view's 30°
//! tilt that gives, in bytes:
//!
//! ```text
//! depth = -3 * (fx + fy) - e / 2
//! ```
//!
//! for a point at view position (fx, fy) in sixteenths of a tile and `e`
//! near-zoom pixels above the ground level. A sprite pixel is
//! `anchor depth + (byte - ANCHOR_BYTE)`.
//!
//! **Unverified:** the original builds its own depth from per-row tables
//! (the old prototype's reading in
//! `legacy/crates/sims-sim/src/lot_presenter/tile_depth.rs`); this scale
//! matches its sprites, not its tables.

use crate::camera::Camera;

/// The depth byte of a sprite pixel level with the object's anchor.
pub const ANCHOR_BYTE: i32 = 200;

/// Sixteenths of a byte per byte.
pub const UNIT: i32 = 16;

/// The depth of a view position (sixteenths of a tile) raised `elevation`
/// near-zoom pixels.
pub fn point(view: [i32; 2], elevation: i32) -> i32 {
    -3 * UNIT * (view[0] + view[1]) - UNIT / 2 * elevation
}

/// The depth of a ground pixel on screen row `y`, on ground raised
/// `elevation` near-zoom pixels.
pub fn ground(camera: &Camera, y: i32, elevation: i32) -> i32 {
    let s = camera.zoom.near_scale();
    -3 * UNIT / 2 * s * (y - camera.origin[1]) - 2 * UNIT * elevation
}

/// The depth of a pixel on screen row `y` of an upright surface whose foot
/// is on row `foot`, on ground raised `elevation` near-zoom pixels.
pub fn upright(camera: &Camera, y: i32, foot: i32, elevation: i32) -> i32 {
    let s = camera.zoom.near_scale();
    ground(camera, foot, elevation) - UNIT / 2 * s * (foot - y)
}

/// The depth of a sprite pixel with depth byte `byte`, for an object whose
/// anchor has depth `anchor`.
pub fn sprite(anchor: i32, byte: u8) -> i32 {
    anchor + (i32::from(byte) - ANCHOR_BYTE) * UNIT
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::{Walls, Zoom};

    #[test]
    fn ground_rows_agree_with_projected_points() {
        for zoom in Zoom::ALL {
            let c = Camera {
                zoom,
                rotation: 0,
                level: 1,
                origin: [0, 37],
                walls: Walls::Up,
                focus: crate::Focus::default(),
            };
            for view in [[16, 16], [40, 8], [200, 300]] {
                let elevation = 16 * c.zoom.near_scale();
                let y = c.project_fine(view, 1, elevation)[1];
                assert_eq!(ground(&c, y, elevation), point(view, elevation));
            }
        }
    }

    #[test]
    fn nearer_and_higher_points_are_closer() {
        assert!(point([20, 20], 0) < point([10, 10], 0));
        assert!(point([10, 10], 50) < point([10, 10], 0));
        assert_eq!(sprite(1000, 200), 1000);
        assert_eq!(sprite(1000, 201), 1016);
    }
}
