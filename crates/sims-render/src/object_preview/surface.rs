//! 4e5544 / 4e572c: temporary color/depth target binding. These are renderer
//! globals, not VM state. Numeric buffer addresses model the native arithmetic;
//! this module never dereferences them or accesses process memory.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Globals {
    pub color: u32,
    pub color_pitch: i32,
    pub depth: u32,
    pub depth_pitch: i32,
    /// Native packed shorts at 6f0c10: top, left, bottom, right.
    pub clip: [i16; 4],
    pub anchor: [i32; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Plane {
    Color,
    Depth,
}

/// Image virtual methods used at this boundary. A locked image can still return
/// a null buffer; lock success and buffer availability must remain separate.
pub trait Image {
    fn bounds(&self) -> [i32; 4];
    fn lock(&mut self, plane: Plane, flags: u32) -> bool;
    fn buffer(&mut self, plane: Plane) -> u32;
    fn pitch(&mut self, plane: Plane) -> i32;
    fn unlock(&mut self, plane: Plane, flags: u32);
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Context {
    pub globals: Globals,
    saved: Option<Globals>,
    active: bool,
}

impl Context {
    pub fn new(globals: Globals) -> Self {
        Self {
            globals,
            saved: None,
            active: false,
        }
    }

    pub fn active(&self) -> bool {
        self.active
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.active && self.saved.is_none() {
            return Err("bound object preview target lost saved globals".into());
        }
        Ok(())
    }

    /// A failed bind does NOT restore globals in the recovered routine. The
    /// saved values remain, the active guard clears, and finish becomes a no-op.
    /// Retaining that detail avoids silently repairing original failure state.
    pub fn begin(&mut self, image: &mut impl Image, rect: [i32; 4]) -> bool {
        if self.active {
            return false;
        }
        self.saved = Some(self.globals);
        self.active = true;
        self.globals.color = 0;
        self.globals.color_pitch = 0;
        self.globals.depth = 0;
        self.globals.depth_pitch = 0;

        let bounds = image.bounds();
        // 5de95f first checks overlap, and returns four ZERO coordinates when
        // there is none. This differs from just clamping all four endpoints.
        let clipped = if bounds[0] < rect[2]
            && rect[0] < bounds[2]
            && bounds[1] < rect[3]
            && rect[1] < bounds[3]
        {
            [
                rect[0].max(bounds[0]),
                rect[1].max(bounds[1]),
                rect[2].min(bounds[2]),
                rect[3].min(bounds[3]),
            ]
        } else {
            [0; 4]
        };
        if clipped == rect {
            let color = image.lock(Plane::Color, 64);
            // This second lock is attempted even if the first failed.
            let depth = image.lock(Plane::Depth, 64);
            if color {
                if depth {
                    self.globals.color = image.buffer(Plane::Color);
                    self.globals.color_pitch = image.pitch(Plane::Color);
                    self.globals.depth = image.buffer(Plane::Depth);
                    self.globals.depth_pitch = image.pitch(Plane::Depth);
                    if self.globals.color != 0 && self.globals.depth != 0 {
                        let [left, top, right, bottom] = rect;
                        self.globals.clip = [
                            0,
                            0,
                            (bottom as i16).wrapping_sub(top as i16),
                            (right as i16).wrapping_sub(left as i16),
                        ];
                        self.globals.anchor = [100, 100];
                        self.globals.color = self
                            .globals
                            .color
                            .wrapping_add(top.wrapping_mul(self.globals.color_pitch) as u32)
                            .wrapping_add(left.wrapping_mul(2) as u32);
                        self.globals.depth = self
                            .globals
                            .depth
                            .wrapping_add(top.wrapping_mul(self.globals.depth_pitch) as u32)
                            .wrapping_add(left.wrapping_mul(2) as u32);
                        return true;
                    }
                }
                image.unlock(Plane::Color, 64);
            }
            if depth {
                image.unlock(Plane::Depth, 64);
            }
        }
        self.active = false;
        false
    }

    pub fn finish(&mut self, image: &mut impl Image) {
        if self.active {
            image.unlock(Plane::Color, 64);
            image.unlock(Plane::Depth, 64);
            self.active = false;
            self.globals = self.saved.expect("active target retains saved globals");
        }
    }
}
