//! What the viewer shows and how keys change it. No window here, so it is
//! tested headless.

use sims_render::{Camera, Walls, Zoom};

/// A command from the keyboard or mouse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    ZoomIn,
    ZoomOut,
    /// Turn the view a quarter turn: true clockwise.
    Rotate(bool),
    LevelUp,
    LevelDown,
    /// Walls up, cutaway, down, as the original's three buttons.
    Walls(Walls),
    /// Move the lot by screen pixels.
    Scroll([i32; 2]),
    /// Save the frame as a PNG.
    Save,
}

/// The viewer's camera and screen size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct View {
    pub camera: Camera,
    /// The canvas size in the original's pixels.
    pub size: [i32; 2],
}

impl View {
    /// A view of `center` (sixteenths of a tile) on a canvas of `size`.
    pub fn new(size: [i32; 2], center: [i32; 2], rotation: u8) -> Self {
        let mut camera = Camera {
            zoom: Zoom::Medium,
            rotation: rotation & 3,
            level: 1,
            origin: [0, 0],
            walls: Walls::Cutaway,
        };
        camera.center_on(center, [size[0] / 2, size[1] / 2]);
        Self { camera, size }
    }

    fn middle(&self) -> [i32; 2] {
        [self.size[0] / 2, self.size[1] / 2]
    }

    /// Resizes the canvas, keeping the lot point in the middle there.
    pub fn resize(&mut self, size: [i32; 2]) {
        let fine = self.camera.unproject_fine(self.middle());
        self.size = size;
        let middle = self.middle();
        self.camera.center_on(fine, middle);
    }

    /// Applies a command. Returns whether the picture changes.
    pub fn apply(&mut self, command: Command) -> bool {
        let before = self.camera;
        let middle = self.middle();
        match command {
            Command::ZoomIn => self.camera = self.camera.zoomed(self.camera.zoom.zoom_in(), middle),
            Command::ZoomOut => {
                self.camera = self.camera.zoomed(self.camera.zoom.zoom_out(), middle)
            }
            Command::Rotate(clockwise) => {
                self.camera = self.camera.rotated(if clockwise { 1 } else { 3 }, middle)
            }
            Command::LevelUp => self.camera.level = (self.camera.level + 1).min(2),
            Command::LevelDown => self.camera.level = (self.camera.level - 1).max(1),
            Command::Walls(walls) => self.camera.walls = walls,
            Command::Scroll([dx, dy]) => {
                self.camera.origin[0] += dx;
                self.camera.origin[1] += dy;
            }
            Command::Save => return false,
        }
        self.camera != before
    }

    /// A one-line summary for the window title.
    pub fn describe(&self) -> String {
        let zoom = match self.camera.zoom {
            Zoom::Far => "far",
            Zoom::Medium => "medium",
            Zoom::Near => "near",
        };
        let walls = match self.camera.walls {
            Walls::Up => "walls up",
            Walls::Cutaway => "cutaway",
            Walls::Down => "walls down",
        };
        format!(
            "zoom {zoom}, rotation {}, floor {}, {walls}",
            self.camera.rotation, self.camera.level
        )
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
            Command::ZoomIn,
            Command::Rotate(true),
            Command::ZoomOut,
            Command::ZoomOut,
            Command::Rotate(false),
            Command::Rotate(false),
        ] {
            assert!(v.apply(command));
            assert_eq!(v.camera.unproject_fine([400, 300]), middle);
        }
        assert_eq!(v.camera.rotation, 3);
        assert_eq!(v.camera.zoom, Zoom::Far);
        assert!(!v.apply(Command::ZoomOut));
    }

    #[test]
    fn levels_walls_and_scrolling() {
        let mut v = View::new([800, 600], [400, 400], 1);
        assert!(v.apply(Command::LevelUp));
        assert!(!v.apply(Command::LevelUp));
        assert_eq!(v.camera.level, 2);
        assert!(v.apply(Command::Walls(Walls::Down)));
        assert_eq!(v.describe(), "zoom medium, rotation 1, floor 2, walls down");
        let origin = v.camera.origin;
        assert!(v.apply(Command::Scroll([10, -5])));
        assert_eq!(v.camera.origin, [origin[0] + 10, origin[1] - 5]);
        v.resize([400, 300]);
        assert_eq!(v.size, [400, 300]);
    }
}
