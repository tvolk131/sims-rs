//! Original4a7996 centering owner,4c9c15/4c9c0b motion setters and4b02fd
//! start notification. Floor, projection, scroll and motion curves are supplied
//! services; the owner retains their ordering and live callback state reads.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub level: i32,
    pub origin: [i32; 2],
    pub pending: [i32; 2],
    pub viewport: [i32; 4],
    pub bounds: [i32; 4],
    pub zoom: u32,
    pub graphics_ready: u32,
    /// View+184 motion object: active+20, policy+24, duration+28.
    pub motion_active: u32,
    pub motion_policy: u32,
    pub motion_duration: i32,
    /// Global6c7788, read when the duration setter is called.
    pub duration_input: i32,
    pub moving: u8,
    pub motion_disabled: u8,
    pub motion_allowed: u8,
    pub camera_mode: u8,
    /// Opaque float-storage bits at global6fe888 and View+1b8. The start
    /// owner copies then clears this value; no application meaning is inferred.
    pub application_value_bits: u32,
    pub saved_value_bits: u32,
}
impl State {
    fn can_project(&self, point: [i32; 2]) -> bool {
        let [x, y] = [point[1] >> 4, point[0] >> 4].map(|v| i32::from(v as i8));
        self.graphics_ready != 0
            && x >= self.bounds[0]
            && x <= self.bounds[1]
            && y >= self.bounds[2]
            && y <= self.bounds[3]
    }
    fn started(&mut self) {
        self.moving = 1;
        //4b02fd tests disabled against exactly one, not generic nonzero.
        if self.motion_disabled != 1 && self.motion_allowed != 0 && self.camera_mode == 0 {
            self.camera_mode = 1;
            self.saved_value_bits = self.application_value_bits;
            self.application_value_bits = 0;
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    ChangeFloor {
        level: i32,
        flag: u8,
    },
    RefreshFloorUi {
        level: i32,
        flag: u8,
    },
    Project {
        point: [i32; 2],
        level: i32,
        zoom: u32,
    },
    Scroll {
        position: [i32; 2],
        immediate: u8,
    },
    SampleMotion {
        axis: u8,
    },
    ConfigureMotion {
        start: [i32; 2],
        target: [i32; 2],
        style: u8,
    },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Done,
    /// Vertical first, horizontal second, matching the native short pair.
    Screen([i16; 2]),
    Coordinate(i32),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Floor,
    FloorUi,
    Project { level: i32, zoom: u32 },
    Scroll,
    SampleY,
    SampleX,
    Configure { starts_motion: bool },
    Done,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    point: [i32; 2],
    requested_level: i32,
    style: u8,
    immediate: u8,
    start: [i32; 2],
    target: [i32; 2],
    stage: Stage,
}
impl Cursor {
    pub fn begin(state: &State, point: [i32; 2], level: i32, style: u8, immediate: u8) -> Self {
        let mut cursor = Self {
            point,
            requested_level: level,
            style,
            immediate,
            start: [0, 0],
            target: [0, 0],
            stage: Stage::Floor,
        };
        if state.level == level {
            cursor.project(state);
        }
        cursor
    }
    fn project(&mut self, state: &State) {
        self.stage = if state.can_project(self.point) {
            Stage::Project {
                level: state.level,
                zoom: state.zoom,
            }
        } else {
            Stage::Done
        };
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Done
    }
    pub fn request(&self) -> Option<Request> {
        Some(match self.stage {
            Stage::Floor => Request::ChangeFloor {
                level: self.requested_level,
                flag: 0,
            },
            Stage::FloorUi => Request::RefreshFloorUi {
                level: self.requested_level,
                flag: 1,
            },
            Stage::Project { level, zoom } => Request::Project {
                point: self.point,
                level,
                zoom,
            },
            Stage::Scroll => Request::Scroll {
                position: self.target,
                immediate: 1,
            },
            Stage::SampleY => Request::SampleMotion { axis: 1 },
            Stage::SampleX => Request::SampleMotion { axis: 0 },
            Stage::Configure { .. } => Request::ConfigureMotion {
                start: self.start,
                target: self.target,
                style: self.style,
            },
            Stage::Done => return None,
        })
    }
    pub fn acknowledge(
        &mut self,
        state: &mut State,
        request: &Request,
        reply: Reply,
    ) -> Result<(), String> {
        if self.request().as_ref() != Some(request) {
            return Err("native centering acknowledgement mismatch".into());
        }
        let valid = match (&self.stage, &reply) {
            (Stage::Project { .. }, Reply::Screen(_))
            | (Stage::SampleX | Stage::SampleY, Reply::Coordinate(_)) => true,
            (Stage::Project { .. } | Stage::SampleX | Stage::SampleY, _) => false,
            (_, Reply::Done) => true,
            _ => false,
        };
        if !valid {
            return Err("native centering reply type mismatch".into());
        }
        match self.stage {
            Stage::Floor => self.stage = Stage::FloorUi,
            Stage::FloorUi => self.project(state),
            Stage::Project { .. } => {
                let Reply::Screen([vertical, horizontal]) = reply else {
                    unreachable!()
                };
                self.start =
                    std::array::from_fn(|n| state.origin[n].wrapping_add(state.pending[n]));
                let half = [
                    state.viewport[2].wrapping_sub(state.viewport[0]) / 2,
                    state.viewport[3].wrapping_sub(state.viewport[1]) / 2,
                ];
                let screen = [horizontal, vertical].map(i32::from);
                self.target = std::array::from_fn(|n| {
                    half[n].wrapping_sub(screen[n]).wrapping_add(self.start[n])
                });
                self.stage = if self.immediate != 0 {
                    Stage::Scroll
                } else {
                    let active = state.motion_active != 0;
                    state.motion_policy = 1;
                    state.motion_duration = state.duration_input;
                    if active {
                        Stage::SampleY
                    } else {
                        Stage::Configure {
                            starts_motion: true,
                        }
                    }
                };
            }
            Stage::SampleY => {
                let Reply::Coordinate(y) = reply else {
                    unreachable!()
                };
                self.start[1] = y;
                self.stage = Stage::SampleX;
            }
            Stage::SampleX => {
                let Reply::Coordinate(x) = reply else {
                    unreachable!()
                };
                self.start[0] = x;
                self.stage = Stage::Configure {
                    starts_motion: false,
                };
            }
            Stage::Configure { starts_motion } => {
                if starts_motion {
                    state.started();
                }
                self.stage = Stage::Done;
            }
            Stage::Scroll => self.stage = Stage::Done,
            Stage::Done => unreachable!(),
        }
        Ok(())
    }
}
