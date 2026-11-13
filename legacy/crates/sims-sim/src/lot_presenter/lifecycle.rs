//! Original 4a814d walls-down, 4a817e cutaway and 4a80c7 floor ownership.
//! Requests retain native call boundaries and are acknowledged after services
//! finish. This does not invent automatic per-frame camera dispatch.
use super::{hover::History, visibility};
use serde::{Deserialize, Serialize};
pub mod roof;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub level: i32,
    pub all_walls: u8,
    pub history: History,
    pub visibility: visibility::State,
}
impl State {
    pub fn validate(&self) -> Result<(), String> {
        self.history.validate()?;
        if self.visibility.mask.first_row != 0 || self.visibility.mask.rows.len() != 64 {
            return Err("native view lifecycle mask unavailable".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    WallState {
        enabled: u8,
        level: i32,
    },
    ClearMask,
    RefreshRooms {
        level: i32,
    },
    Compose {
        include_visible: u8,
    },
    InvalidateView,
    ResolveSurface,
    /// Stable caller token, not a serialized native pointer.
    FlushSurface {
        surface: u32,
    },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Done,
    Surface(u32),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    WallsClear,
    WallsInvalidate,
    ModeInvalidate,
    ModeCompose(u8),
    FloorWalls(i32),
    FloorClear,
    FloorRefresh,
    FloorCompose(u8),
    FloorInvalidate,
    FloorSurface,
    FloorFlush(u32),
    Done,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    target_floor: Option<i32>,
    stage: Stage,
}
impl Cursor {
    ///4a814d stores+48 first. An active cutaway mask is cleared before+49
    ///is reset and before dirty/invalidate, with live per-tile callbacks.
    pub fn walls_down(state: &mut State, enabled: u8) -> Result<Self, String> {
        state.validate()?;
        let stage = if state.all_walls == enabled {
            Stage::Done
        } else {
            state.all_walls = enabled;
            if state.visibility.enabled != 0 {
                Stage::WallsClear
            } else {
                state.visibility.dirty = 1;
                Stage::WallsInvalidate
            }
        };
        Ok(Self {
            target_floor: None,
            stage,
        })
    }
    pub fn cutaway(state: &mut State, enabled: u8) -> Result<Self, String> {
        state.validate()?;
        let stage = if state.visibility.enabled == enabled {
            Stage::Done
        } else {
            state.all_walls = 0;
            state.visibility.enabled = enabled;
            state.visibility.dirty = 1;
            Stage::ModeInvalidate
        };
        Ok(Self {
            target_floor: None,
            stage,
        })
    }
    pub fn floor(state: &State, level: i32, force: u8) -> Result<Self, String> {
        state.validate()?;
        Ok(Self {
            target_floor: Some(level),
            stage: if state.level == level && force == 0 {
                Stage::Done
            } else {
                Stage::FloorWalls(state.level)
            },
        })
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Done
    }
    pub fn validate(&self, state: &State) -> Result<(), String> {
        state.validate()?;
        let mode = matches!(
            self.stage,
            Stage::ModeInvalidate
                | Stage::ModeCompose(_)
                | Stage::WallsClear
                | Stage::WallsInvalidate
        );
        if self.stage != Stage::Done && mode == self.target_floor.is_some() {
            return Err("invalid native view lifecycle continuation".into());
        }
        Ok(())
    }
    pub fn request(&self) -> Option<Request> {
        Some(match self.stage {
            Stage::ModeInvalidate | Stage::FloorInvalidate | Stage::WallsInvalidate => {
                Request::InvalidateView
            }
            Stage::ModeCompose(include_visible) | Stage::FloorCompose(include_visible) => {
                Request::Compose { include_visible }
            }
            Stage::FloorWalls(level) => Request::WallState { enabled: 0, level },
            Stage::FloorClear | Stage::WallsClear => Request::ClearMask,
            Stage::FloorRefresh => Request::RefreshRooms {
                level: self.target_floor?,
            },
            Stage::FloorSurface => Request::ResolveSurface,
            Stage::FloorFlush(surface) => Request::FlushSurface { surface },
            Stage::Done => return None,
        })
    }
    fn switch_floor(&mut self, state: &mut State) {
        state.level = self.target_floor.expect("validated floor continuation");
        self.stage = Stage::FloorRefresh;
    }
    pub fn acknowledge(
        &mut self,
        state: &mut State,
        request: &Request,
        reply: Reply,
    ) -> Result<(), String> {
        self.validate(state)?;
        if self.request().as_ref() != Some(request) {
            return Err("native view lifecycle acknowledgement mismatch".into());
        }
        if matches!(self.stage, Stage::FloorSurface) {
            let Reply::Surface(surface) = reply else {
                return Err("native view lifecycle surface reply required".into());
            };
            self.stage = Stage::FloorFlush(surface);
            return Ok(());
        }
        if reply != Reply::Done {
            return Err("unexpected native view lifecycle surface reply".into());
        }
        match self.stage {
            Stage::WallsClear => {
                state.visibility.enabled = 0;
                state.visibility.dirty = 1;
                self.stage = Stage::WallsInvalidate;
            }
            Stage::WallsInvalidate => self.stage = Stage::Done,
            Stage::ModeInvalidate => {
                state.history.clear();
                state.visibility.mask.rows.fill(0);
                // The callback may have changed this byte. Native reads it now.
                self.stage = Stage::ModeCompose(state.visibility.include_visible);
            }
            Stage::ModeCompose(_) | Stage::FloorFlush(_) => self.stage = Stage::Done,
            Stage::FloorWalls(_) => {
                state.history.clear();
                state.visibility.dirty = 1;
                if state.visibility.enabled != 0 {
                    self.stage = Stage::FloorClear;
                } else {
                    self.switch_floor(state);
                }
            }
            Stage::FloorClear => self.switch_floor(state),
            Stage::FloorRefresh => {
                self.stage = if state.visibility.enabled != 0 {
                    Stage::FloorCompose(state.visibility.include_visible)
                } else {
                    Stage::FloorInvalidate
                };
            }
            Stage::FloorCompose(_) => self.stage = Stage::FloorInvalidate,
            Stage::FloorInvalidate => self.stage = Stage::FloorSurface,
            Stage::FloorSurface | Stage::Done => unreachable!(),
        }
        Ok(())
    }
}
