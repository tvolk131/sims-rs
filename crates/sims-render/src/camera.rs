//! The house view's camera: zoom, rotation, the floor shown, and where the
//! lot sits on the screen.
//!
//! The projection is the original's (0x4b3c88, read by the old prototype in
//! `legacy/crates/sims-sim/src/lot_presenter/room_masks.rs`). A tile at
//! column `x`, row `y` and level `z`, after the view's rotation, has its top
//! corner at
//!
//! ```text
//! screen x = (x - y) * (8 << zoom) + origin x
//! screen y = (x + y) * (4 << zoom) + origin y - floor height * (z - 1)
//! ```
//!
//! with floor heights 58, 116 and 232 pixels at zooms 1, 2 and 3, and the
//! terrain raising the corner by its height times `2 << zoom`. Zoom 1 is
//! the far view (tiles 32 pixels wide), zoom 3 the near one (128 pixels).

use crate::cutaway::Focus;

/// One of the original's three zooms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Zoom {
    /// Tiles 32 pixels wide.
    Far = 1,
    /// Tiles 64 pixels wide.
    Medium = 2,
    /// Tiles 128 pixels wide.
    Near = 3,
}

impl Zoom {
    pub const ALL: [Zoom; 3] = [Zoom::Far, Zoom::Medium, Zoom::Near];

    /// The original's zoom number, 1 to 3.
    pub fn level(self) -> u32 {
        self as u32
    }

    /// 0 for far, 1 for medium, 2 for near: the order of each surface's
    /// three sprites in the game's files.
    pub fn index(self) -> usize {
        self as usize - 1
    }

    /// A tile's width in pixels.
    pub fn tile_width(self) -> i32 {
        16 << self.level()
    }

    /// A tile's height in pixels: half its width.
    pub fn tile_height(self) -> i32 {
        8 << self.level()
    }

    /// The screen height of one storey (View+124 to +130 in the original).
    pub fn floor_height(self) -> i32 {
        [58, 116, 232][self.index()]
    }

    /// Pixels of the near zoom per pixel of this zoom.
    pub fn near_scale(self) -> i32 {
        1 << (3 - self.level())
    }

    pub fn zoom_in(self) -> Self {
        match self {
            Zoom::Far => Zoom::Medium,
            _ => Zoom::Near,
        }
    }

    pub fn zoom_out(self) -> Self {
        match self {
            Zoom::Near => Zoom::Medium,
            _ => Zoom::Far,
        }
    }

    /// [`Zoom::zoom_out`] on house `house` (global 10): the original zooms
    /// out only while the zoom is above 1, or above 2 on house 99, the
    /// neighborhood's own lot (`0x4cedc0`, `0x4ce9f8`).
    pub fn zoom_out_on(self, house: i16) -> Self {
        let farthest = if house == 99 { Zoom::Medium } else { Zoom::Far };
        self.zoom_out().max(farthest).min(self)
    }
}

/// How walls are drawn, as the original's three wall buttons choose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Walls {
    /// Every wall at full height.
    Up,
    /// Walls that would hide a room's inside are cut down to a base strip.
    Cutaway,
    /// Every wall cut down.
    Down,
}

/// The side of the lot's arrays. The rotation table is defined on it,
/// whatever the lot's active size.
pub const ARRAY_SIZE: i32 = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Camera {
    pub zoom: Zoom,
    /// Quarter turns, 0 to 3.
    pub rotation: u8,
    /// The highest level shown. Levels below it show too.
    pub level: i32,
    /// Screen position of the top corner of tile (0, 0) on level 1, after
    /// rotation.
    pub origin: [i32; 2],
    pub walls: Walls,
    /// What the cutaway looks at: the pointer's rooms and the selected Sim.
    pub focus: Focus,
}

impl Camera {
    /// Rotates a tile into the view (the original's tile table at
    /// 0x407704, which works on the 64-tile array whatever the lot's size).
    pub fn rotate_tile(&self, [x, y]: [i32; 2]) -> [i32; 2] {
        let m = ARRAY_SIZE - 1;
        match self.rotation & 3 {
            1 => [m - y, x],
            2 => [m - x, m - y],
            3 => [y, m - x],
            _ => [x, y],
        }
    }

    /// The inverse of [`Camera::rotate_tile`].
    pub fn unrotate_tile(&self, [x, y]: [i32; 2]) -> [i32; 2] {
        let m = ARRAY_SIZE - 1;
        match self.rotation & 3 {
            1 => [y, m - x],
            2 => [m - x, m - y],
            3 => [m - y, x],
            _ => [x, y],
        }
    }

    /// Rotates a fine position (16 units to a tile) into the view.
    pub fn rotate_fine(&self, [x, y]: [i32; 2]) -> [i32; 2] {
        let m = ARRAY_SIZE * 16;
        match self.rotation & 3 {
            1 => [m - y, x],
            2 => [m - x, m - y],
            3 => [y, m - x],
            _ => [x, y],
        }
    }

    /// The screen position of a view tile's top corner on `level`, with the
    /// corner raised `height` terrain steps.
    pub fn project(&self, [x, y]: [i32; 2], level: i32, height: i32) -> [i32; 2] {
        let shift = self.zoom.level();
        [
            (x - y) * (8 << shift) + self.origin[0],
            (x + y) * (4 << shift) + self.origin[1]
                - self.zoom.floor_height() * (level - 1)
                - (height << (shift + 1)),
        ]
    }

    /// The screen position of a fine view position on `level`, raised
    /// `elevation` near-zoom pixels.
    pub fn project_fine(&self, [x, y]: [i32; 2], level: i32, elevation: i32) -> [i32; 2] {
        let shift = self.zoom.level();
        let scale = self.zoom.near_scale();
        [
            ((x - y) * (8 << shift)).div_euclid(16) + self.origin[0],
            ((x + y) * (4 << shift)).div_euclid(16) + self.origin[1]
                - self.zoom.floor_height() * (level - 1)
                - elevation.div_euclid(scale),
        ]
    }

    /// [`Camera::project_fine`] without its rounding, for points between
    /// pixels (a person's mesh): the screen position of fine view position
    /// `fine` on `level`, raised `elevation` near-zoom pixels. The pixel a
    /// point falls in is the floor of each coordinate. At whole positions
    /// this agrees with `project_fine` when the elevation is a whole
    /// number of this zoom's pixels; otherwise `project_fine` drops the
    /// elevation's fraction (it floors it on its own) and this keeps it.
    pub fn project_fine_f32(&self, [x, y]: [f32; 2], level: i32, elevation: f32) -> [f32; 2] {
        let shift = self.zoom.level();
        let scale = self.zoom.near_scale() as f32;
        [
            (x - y) * (8 << shift) as f32 / 16.0 + self.origin[0] as f32,
            (x + y) * (4 << shift) as f32 / 16.0 + self.origin[1] as f32
                - (self.zoom.floor_height() * (level - 1)) as f32
                - elevation / scale,
        ]
    }

    /// The camera rotated a quarter turn, keeping the screen point
    /// `center` over the same spot of the lot.
    pub fn rotated(&self, turns: u8, center: [i32; 2]) -> Self {
        let fine = self.unproject_fine(center);
        let mut camera = Self {
            rotation: (self.rotation + turns) & 3,
            ..*self
        };
        camera.center_on(fine, center);
        camera
    }

    /// The camera at another zoom, keeping the screen point `center` over
    /// the same spot of the lot.
    pub fn zoomed(&self, zoom: Zoom, center: [i32; 2]) -> Self {
        let fine = self.unproject_fine(center);
        let mut camera = Self { zoom, ..*self };
        camera.center_on(fine, center);
        camera
    }

    /// Moves the origin so the unrotated fine ground position `fine`
    /// (level 1) shows at screen point `at`.
    pub fn center_on(&mut self, fine: [i32; 2], at: [i32; 2]) {
        self.origin = [0, 0];
        let p = self.project_fine(self.rotate_fine(fine), 1, 0);
        self.origin = [at[0] - p[0], at[1] - p[1]];
    }

    /// The unrotated fine ground position (level 1, flat) under a screen
    /// point.
    pub fn unproject_fine(&self, at: [i32; 2]) -> [i32; 2] {
        let shift = self.zoom.level();
        let sx = (at[0] - self.origin[0]) * 16;
        let sy = (at[1] - self.origin[1]) * 16;
        // sx = (x - y) * 8 << shift, sy = (x + y) * 4 << shift.
        let d = sx / (8 << shift);
        let s = sy / (4 << shift);
        let view = [(s + d).div_euclid(2), (s - d).div_euclid(2)];
        let m = ARRAY_SIZE * 16;
        let [x, y] = view;
        match self.rotation & 3 {
            1 => [y, m - x],
            2 => [m - x, m - y],
            3 => [m - y, x],
            _ => [x, y],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn house_99_zooms_out_no_farther_than_medium() {
        assert_eq!(Zoom::Near.zoom_out_on(5), Zoom::Medium);
        assert_eq!(Zoom::Medium.zoom_out_on(5), Zoom::Far);
        assert_eq!(Zoom::Far.zoom_out_on(5), Zoom::Far);
        assert_eq!(Zoom::Near.zoom_out_on(99), Zoom::Medium);
        assert_eq!(Zoom::Medium.zoom_out_on(99), Zoom::Medium);
        // Already farther (a camera loaded so): it stays.
        assert_eq!(Zoom::Far.zoom_out_on(99), Zoom::Far);
    }

    fn camera(rotation: u8) -> Camera {
        Camera {
            zoom: Zoom::Near,
            rotation,
            level: 1,
            origin: [400, 100],
            walls: Walls::Up,
            focus: Focus::default(),
        }
    }

    #[test]
    fn float_projection_floors_to_the_whole_one() {
        for zoom in Zoom::ALL {
            let c = Camera {
                zoom,
                origin: [-37, 411],
                ..camera(0)
            };
            for level in 1..=3 {
                for (x, y) in [(0, 0), (5, 3), (17, 900), (1023, 1), (-9, 40)] {
                    for elevation in [0, 8, 16, 88, -24] {
                        let whole = c.project_fine([x, y], level, elevation);
                        let fine =
                            c.project_fine_f32([x as f32, y as f32], level, elevation as f32);
                        assert_eq!(fine.map(|v| v.floor() as i32), whole, "{zoom:?} ({x}, {y})");
                    }
                }
            }
        }
    }

    #[test]
    fn float_projection_keeps_fractions() {
        for zoom in Zoom::ALL {
            let c = Camera { zoom, ..camera(0) };
            let scale = zoom.near_scale() as f32;
            let [x0, y0] = c.project_fine_f32([10.0, 20.0], 2, 0.0);
            let [x1, y1] = c.project_fine_f32([10.5, 20.25], 2, 3.0);
            let across = (8 << zoom.level()) as f32 / 16.0;
            assert_eq!(x1 - x0, 0.25 * across);
            assert_eq!(y1 - y0, 0.75 * across / 2.0 - 3.0 / scale);
        }
    }

    #[test]
    fn tiles_and_fine_positions_rotate_back() {
        for rotation in 0..4 {
            let c = camera(rotation);
            for tile in [[0, 0], [5, 9], [63, 1], [30, 63]] {
                assert_eq!(c.unrotate_tile(c.rotate_tile(tile)), tile);
                let fine = [tile[0] * 16 + 8, tile[1] * 16 + 8];
                let view = c.rotate_tile(tile);
                assert_eq!(c.rotate_fine(fine), [view[0] * 16 + 8, view[1] * 16 + 8]);
            }
        }
    }

    #[test]
    fn projection_matches_the_original_tile_sizes() {
        let c = camera(0);
        assert_eq!(c.project([0, 0], 1, 0), [400, 100]);
        assert_eq!(c.project([1, 0], 1, 0), [464, 132]);
        assert_eq!(c.project([0, 1], 1, 0), [336, 132]);
        assert_eq!(c.project([0, 0], 2, 0), [400, 100 - 232]);
        assert_eq!(c.project([0, 0], 1, 1), [400, 84]);
        assert_eq!(c.project_fine([8, 8], 1, 0), [400, 132]);
        let far = Camera {
            zoom: Zoom::Far,
            ..c
        };
        assert_eq!(far.project([1, 1], 1, 0), [400, 116]);
    }

    #[test]
    fn rotating_and_zooming_keep_the_center_still() {
        let mut c = camera(0);
        c.center_on([20 * 16, 30 * 16], [320, 240]);
        assert_eq!(c.unproject_fine([320, 240]), [320, 480]);
        for turns in 1..4 {
            let r = c.rotated(turns, [320, 240]);
            assert_eq!(r.unproject_fine([320, 240]), [320, 480]);
        }
        for zoom in Zoom::ALL {
            let z = c.zoomed(zoom, [320, 240]);
            assert_eq!(z.unproject_fine([320, 240]), [320, 480]);
        }
    }
}
