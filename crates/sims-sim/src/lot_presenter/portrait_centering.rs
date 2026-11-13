use super::super::{centering, motion::wide::Wide, scroll};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Project {
        point: [i32; 2],
        level: i32,
    },
    Bounds {
        target: [i32; 2],
    },
    World {
        point: [i32; 2],
        level: i32,
        style: u8,
        immediate: u8,
    },
    ConfigureMotion {
        start: [i32; 2],
        target: [i32; 2],
        style: u8,
    },
    Scroll {
        position: [i32; 2],
        immediate: u8,
    },
    InvalidateView,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Screen([i16; 2]),
    Bounds(scroll::Bounds),
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    request: Option<Request>,
    start: [i32; 2],
    animated: u8,
}

fn rotate([vertical, horizontal]: [i32; 2], rotation: u32) -> [i32; 2] {
    match rotation {
        1 => [horizontal, 1023i32.wrapping_sub(vertical)],
        2 => [
            1023i32.wrapping_sub(vertical),
            1023i32.wrapping_sub(horizontal),
        ],
        3 => [1023i32.wrapping_sub(horizontal), vertical],
        _ => [vertical, horizontal],
    }
}

fn height_adjustment(zoom: u32) -> i32 {
    Wide::from_f32(0x409c_c471)
        .unwrap()
        .times(Wide::integer(2))
        .times(Wide::integer(1i32.wrapping_shl(zoom.wrapping_add(1))))
        .ftol()
        / 2
}

impl Cursor {
    pub fn begin(
        state: &centering::State,
        point: [i32; 2],
        level: i32,
        object_type: i16,
        animated: u8,
        rotation: u32,
    ) -> Self {
        let point = rotate(point, rotation);
        Self {
            request: Some(if object_type == 2 && state.level == level {
                Request::Project { point, level }
            } else {
                Request::World {
                    point,
                    level,
                    style: 1,
                    immediate: u8::from(animated == 0),
                }
            }),
            start: [0, 0],
            animated,
        }
    }

    pub fn request(&self) -> Option<Request> {
        self.request.clone()
    }

    pub fn complete(&self) -> bool {
        self.request.is_none()
    }

    pub fn acknowledge(
        &mut self,
        state: &mut centering::State,
        request: &Request,
        reply: Reply,
    ) -> Result<(), String> {
        if self.request.as_ref() != Some(request) {
            return Err("native portrait centering acknowledgement mismatch".into());
        }
        self.request = match (request, reply) {
            (Request::Project { .. }, Reply::Screen([vertical, horizontal])) => {
                self.start = state.origin;
                let screen = [
                    i32::from(horizontal),
                    i32::from(vertical.wrapping_sub(height_adjustment(state.zoom) as i16)),
                ];
                let half = [
                    state.viewport[2].wrapping_sub(state.viewport[0]) / 2,
                    state.viewport[3].wrapping_sub(state.viewport[1]) / 2,
                ];
                Some(Request::Bounds {
                    target: std::array::from_fn(|axis| {
                        half[axis]
                            .wrapping_sub(screen[axis])
                            .wrapping_add(self.start[axis])
                    }),
                })
            }
            (Request::Bounds { target }, Reply::Bounds(bounds)) => {
                let target = std::array::from_fn(|axis| {
                    let limits = if axis == 0 { bounds.x } else { bounds.y };
                    if target[axis] < limits[0] {
                        limits[0]
                    } else if target[axis] > limits[1] {
                        limits[1]
                    } else {
                        target[axis]
                    }
                });
                if self.animated == 0 {
                    state.pending = [0, 0];
                    Some(Request::Scroll {
                        position: target,
                        immediate: 1,
                    })
                } else if state.motion_active == 0 {
                    state.motion_policy = 1;
                    state.motion_duration = state.duration_input;
                    Some(Request::ConfigureMotion {
                        start: self.start,
                        target,
                        style: 1,
                    })
                } else {
                    None
                }
            }
            (Request::ConfigureMotion { .. }, Reply::Done) => {
                state.moving = 1;
                if state.motion_disabled != 1 && state.motion_allowed != 0 && state.camera_mode == 0
                {
                    state.camera_mode = 1;
                    state.saved_value_bits = state.application_value_bits;
                    state.application_value_bits = 0;
                }
                None
            }
            (Request::Scroll { .. }, Reply::Done) => Some(Request::InvalidateView),
            (Request::World { .. } | Request::InvalidateView, Reply::Done) => None,
            _ => return Err("native portrait centering reply mismatch".into()),
        };
        Ok(())
    }
}
