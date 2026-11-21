//! The house view's camera and screen size, and the commands that move
//! it. No window here, so it is tested headless.

use sims_render::{Camera, Focus, Walls, Zoom};

/// A command that moves the camera.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Move {
    ZoomIn,
    ZoomOut,
    /// A quarter turn: true clockwise.
    Rotate(bool),
    LevelUp,
    LevelDown,
    Walls(Walls),
    /// Move the lot by screen pixels.
    Scroll([i32; 2]),
    /// Move the lot so this screen point comes to the middle of the
    /// screen (0x4a7777 and 0x4a7996 at once, as without the slide).
    Centre([i32; 2]),
    /// Show this floor, 1 or 2 (0x4a80c7).
    Floor(i32),
}

/// How far the lot's origin may move from the screen's corner, in pixels,
/// so the projection's arithmetic stays far from overflowing.
const ORIGIN_LIMIT: i32 = 1 << 16;

/// The largest canvas side drawn, in the original's pixels.
const SIZE_LIMIT: i32 = 1 << 14;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct View {
    pub camera: Camera,
    /// The canvas size in the original's pixels.
    pub size: [i32; 2],
}

impl View {
    /// A view of `center` (sixteenths of a tile) on a canvas of `size`.
    /// The original opens a house at the far zoom, on the first floor,
    /// with the walls cut away.
    pub fn new(size: [i32; 2], center: [i32; 2], rotation: u8) -> Self {
        let mut camera = Camera {
            zoom: Zoom::Far,
            rotation: rotation & 3,
            level: 1,
            origin: [0, 0],
            walls: Walls::Cutaway,
            focus: Focus::default(),
        };
        let size = size.map(|n| n.clamp(1, SIZE_LIMIT));
        camera.center_on(center, [size[0] / 2, size[1] / 2]);
        let mut view = Self { camera, size };
        view.bound();
        view
    }

    fn bound(&mut self) {
        self.camera.origin = self
            .camera
            .origin
            .map(|n| n.clamp(-ORIGIN_LIMIT, ORIGIN_LIMIT));
    }

    fn middle(&self) -> [i32; 2] {
        [self.size[0] / 2, self.size[1] / 2]
    }

    /// Resizes the canvas, keeping the lot point in the middle there.
    pub fn resize(&mut self, size: [i32; 2]) {
        let fine = self.camera.unproject_fine(self.middle());
        self.size = size.map(|n| n.clamp(1, SIZE_LIMIT));
        let middle = self.middle();
        self.camera.center_on(fine, middle);
        self.bound();
    }

    /// Applies a move. Returns whether the picture changes.
    pub fn apply(&mut self, command: Move) -> bool {
        let before = self.camera;
        let middle = self.middle();
        match command {
            Move::ZoomIn => self.camera = self.camera.zoomed(self.camera.zoom.zoom_in(), middle),
            Move::ZoomOut => self.camera = self.camera.zoomed(self.camera.zoom.zoom_out(), middle),
            Move::Rotate(clockwise) => {
                self.camera = self.camera.rotated(if clockwise { 1 } else { 3 }, middle)
            }
            Move::LevelUp => self.camera.level = (self.camera.level + 1).min(2),
            Move::LevelDown => self.camera.level = (self.camera.level - 1).max(1),
            Move::Walls(walls) => self.camera.walls = walls,
            Move::Scroll([dx, dy]) => {
                self.camera.origin[0] = self.camera.origin[0].saturating_add(dx);
                self.camera.origin[1] = self.camera.origin[1].saturating_add(dy);
            }
            Move::Centre([x, y]) => {
                self.camera.origin[0] = self.camera.origin[0].saturating_add(middle[0] - x);
                self.camera.origin[1] = self.camera.origin[1].saturating_add(middle[1] - y);
            }
            Move::Floor(level) => self.camera.level = level.clamp(1, 2),
        }
        // The original forgets the pointer's rooms when the floor changes.
        if self.camera.level != before.level {
            self.camera.focus.forget();
        }
        // A change of zoom keeps the rooms' cutaway masks as they were
        // built (0x4a7c9f); turning the view (0x4a8006) or changing the
        // floor (0x4a8116) builds them again at the zoom shown.
        if self.camera.zoom != before.zoom {
            let built = before.focus.masks_built_at.unwrap_or(before.zoom);
            self.camera.focus.masks_built_at = (built != self.camera.zoom).then_some(built);
        }
        if self.camera.rotation != before.rotation || self.camera.level != before.level {
            self.camera.focus.masks_built_at = None;
        }
        self.bound();
        self.camera != before
    }

    /// A lot update built the rooms' cutaway masks again for the levels in
    /// `levels` (bits, 1 for level 1, 2 for level 2;
    /// [`sims_vm::Update::masks_rebuilt`]): if the floor shown is one,
    /// its masks are now at the zoom shown. Returns whether the picture
    /// changes.
    pub fn masks_rebuilt(&mut self, levels: u8) -> bool {
        let bit = 1 << (self.camera.level.clamp(1, 2) - 1);
        let rebuilt = levels & bit != 0 && self.camera.focus.masks_built_at.is_some();
        if rebuilt {
            self.camera.focus.masks_built_at = None;
        }
        rebuilt
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_and_rotation_keep_the_middle_of_the_screen() {
        let mut v = View::new([800, 600], [400, 400], 0);
        let middle = v.camera.unproject_fine([400, 300]);
        assert_eq!(middle, [400, 400]);
        for command in [
            Move::ZoomIn,
            Move::Rotate(true),
            Move::ZoomIn,
            Move::ZoomOut,
            Move::Rotate(false),
            Move::Rotate(false),
        ] {
            assert!(v.apply(command));
            assert_eq!(v.camera.unproject_fine([400, 300]), middle);
        }
        assert_eq!(v.camera.rotation, 3);
        assert_eq!(v.camera.zoom, Zoom::Medium);
    }

    #[test]
    fn levels_walls_scrolling_and_limits() {
        let mut v = View::new([800, 600], [400, 400], 1);
        assert!(v.apply(Move::LevelUp));
        assert!(!v.apply(Move::LevelUp));
        assert!(v.apply(Move::Walls(Walls::Down)));
        let origin = v.camera.origin;
        assert!(v.apply(Move::Scroll([10, -5])));
        assert_eq!(v.camera.origin, [origin[0] + 10, origin[1] - 5]);
        v.apply(Move::Scroll([i32::MAX, i32::MIN]));
        assert!(v.camera.origin.iter().all(|n| n.abs() <= ORIGIN_LIMIT));
        v.resize([i32::MAX, 0]);
        assert_eq!(v.size, [SIZE_LIMIT, 1]);
    }

    #[test]
    fn centring_brings_a_screen_point_to_the_middle() {
        let mut v = View::new([800, 600], [400, 400], 0);
        let fine = v.camera.unproject_fine([100, 50]);
        assert!(v.apply(Move::Centre([100, 50])));
        assert_eq!(v.camera.unproject_fine([400, 300]), fine);
        assert!(!v.apply(Move::Centre([400, 300])));
        assert!(v.apply(Move::Floor(2)));
        assert_eq!(v.camera.level, 2);
        assert!(!v.apply(Move::Floor(5)));
    }

    #[test]
    fn a_change_of_zoom_keeps_the_masks_and_a_turn_or_a_floor_builds_them() {
        let mut v = View::new([800, 600], [32 * 16, 32 * 16], 0);
        let built = |v: &View| v.camera.focus.masks_built_at;
        assert_eq!(built(&v), None);
        v.apply(Move::ZoomIn);
        assert_eq!(built(&v), Some(Zoom::Far));
        v.apply(Move::ZoomIn);
        assert_eq!(built(&v), Some(Zoom::Far));
        // Back at the zoom they were built at, they are the zoom shown's.
        v.apply(Move::ZoomOut);
        v.apply(Move::ZoomOut);
        assert_eq!(built(&v), None);
        v.apply(Move::ZoomIn);
        v.apply(Move::Rotate(true));
        assert_eq!(built(&v), None);
        v.apply(Move::ZoomIn);
        assert_eq!(built(&v), Some(Zoom::Medium));
        // A scroll or a change of walls builds nothing.
        v.apply(Move::Scroll([5, 5]));
        v.apply(Move::Walls(Walls::Down));
        assert_eq!(built(&v), Some(Zoom::Medium));
        v.apply(Move::LevelUp);
        assert_eq!(built(&v), None);
        // A room update builds the masks of the levels it counted: here
        // only the floor shown counts.
        v.apply(Move::ZoomOut);
        assert!(!v.masks_rebuilt(0));
        assert!(!v.masks_rebuilt(1));
        assert_eq!(built(&v), Some(Zoom::Near));
        assert!(v.masks_rebuilt(2));
        assert_eq!(built(&v), None);
        v.apply(Move::ZoomIn);
        assert!(v.masks_rebuilt(3));
        assert!(!v.masks_rebuilt(3));
    }
}
