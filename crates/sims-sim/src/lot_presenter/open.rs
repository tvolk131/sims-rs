//! Original view message4a82db(F6), with its camera/wall/presenter services.
//! The native entry uses the secondary interface at View+8. Callbacks may
//! mutate the live state; later arguments are captured when the original reads
//! them. Dispatching F6 from the application's house-loading owner is separate.
use super::lifecycle;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub floor: lifecycle::State,
    pub origin: [i32; 2],
    pub pending: [i32; 2],
    /// Original global32-bit fields. Zoom services may impose narrower bounds.
    pub zoom: u32,
    pub display_rotation: u32,
    /// View+58,+5c,+60,+64.
    pub widths: [i32; 4],
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    ///Direct publication calls, not steps of the F6 opening sequence.
    WallsDown(u8),
    Cutaway(u8),
    Roof {
        enabled: u8,
        force: u8,
    },
    InvalidateView,
    Zoom {
        zoom: u32,
        force: u8,
        refresh_ui: u8,
    },
    Rotate {
        rotation: u32,
        force: u8,
        refresh_ui: u8,
    },
    Floor {
        level: i32,
        force: u8,
    },
    Scroll {
        position: [i32; 2],
        immediate: u8,
    },
    CenterPerson {
        point: [i32; 2],
        level: i32,
    },
    MotionFrame,
    WallState {
        enabled: u8,
        level: i32,
    },
    ClassifyObjects,
    DrainIndicators,
}
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    request: Option<Request>,
}
impl Cursor {
    pub fn begin(state: &State) -> Result<Self, String> {
        state.floor.validate()?;
        Ok(Self {
            request: Some(Request::InvalidateView),
        })
    }
    pub fn request(&self) -> Option<&Request> {
        self.request.as_ref()
    }
    pub fn complete(&self) -> bool {
        self.request.is_none()
    }
    pub fn returned(&self) -> Option<u16> {
        self.complete().then_some(1)
    }
    pub fn acknowledge(&mut self, state: &mut State, request: &Request) -> Result<(), String> {
        state.floor.validate()?;
        if self.request.as_ref() != Some(request) {
            return Err("native house-open acknowledgement mismatch".into());
        }
        self.request = match request {
            Request::WallsDown(_) | Request::Cutaway(_) | Request::Roof { .. } => {
                return Err("wall controls are not house-open children".into());
            }
            Request::CenterPerson { .. } | Request::MotionFrame => {
                return Err("live camera calls are not house-open children".into());
            }
            Request::InvalidateView => {
                state.widths = [16i32, 8, 8, 4].map(|n| n.wrapping_shl(state.zoom));
                state.floor.visibility.mask.rows.fill(0);
                //49e99b/407161 clears cCutawaySet's history without reallocating.
                state.floor.history.clear();
                state.floor.visibility.dirty = 1;
                Some(Request::Zoom {
                    zoom: state.zoom,
                    force: 1,
                    refresh_ui: 1,
                })
            }
            Request::Zoom { .. } => Some(Request::Rotate {
                rotation: state.display_rotation,
                force: 1,
                refresh_ui: 1,
            }),
            Request::Rotate { .. } => Some(Request::Floor {
                level: state.floor.level,
                force: 1,
            }),
            Request::Floor { .. } => {
                state.pending = [0, 0];
                Some(Request::Scroll {
                    position: state.origin,
                    immediate: 1,
                })
            }
            Request::Scroll { .. } => Some(Request::WallState {
                enabled: state.floor.all_walls,
                level: state.floor.level,
            }),
            Request::WallState { .. } => {
                //5408a6 only resolves the wall-record owner; it does not run a
                //separate wall pass before54051c. The callback finishes first.
                state.floor.visibility.dirty = 1;
                Some(Request::ClassifyObjects)
            }
            Request::ClassifyObjects => Some(Request::DrainIndicators),
            Request::DrainIndicators => None,
        };
        Ok(())
    }
}
