//! Camera preparation inside original frame4a8935, bytes4a8b3e..4a8bad.
//!
//! This stage follows wall/refresh/classification work. It samples active motion
//! Y then X, requests deferred scroll, publishes pending displacement, and locks
//! scrolling for rendering. A curve expiring during sampling is cancelled on
//! the *next* frame, not again in this stage. The later draw loop consumes the
//! pending displacement and unlocks scrolling; it is not supplied here.
use crate::lot_presenter::centering::State;
use serde::{Deserialize, Serialize};
pub mod displacement;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    SampleMotion { axis: u8 },
    Scroll { position: [i32; 2], immediate: u8 },
    CancelMotion,
    Synchronize,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Coordinate(i32),
    Done,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    request: Option<Request>,
    y: i32,
}
impl Cursor {
    pub fn begin(state: &mut State) -> Self {
        let mut result = Self {
            request: None,
            y: 0,
        };
        if state.motion_active != 0 {
            result.request = Some(Request::SampleMotion { axis: 1 });
        } else if state.moving != 0 {
            result.request = Some(Request::CancelMotion);
        } else {
            result.publish_pending(state);
        }
        result
    }
    fn publish_pending(&mut self, state: &mut State) {
        if state.pending != [0, 0] {
            for axis in 0..2 {
                state.origin[axis] = state.origin[axis].wrapping_add(state.pending[axis]);
            }
            // This call is unconditional; renderer readiness is checked by
            //4a7b22 itself. Pending deltas survive until the later drawing pass.
            self.request = Some(Request::Synchronize);
        } else {
            self.finish(state);
        }
    }
    fn finish(&mut self, state: &mut State) {
        state.motion_disabled = 1;
        self.request = None;
    }
    pub fn request(&self) -> Option<&Request> {
        self.request.as_ref()
    }
    pub fn complete(&self) -> bool {
        self.request.is_none()
    }
    pub fn acknowledge(
        &mut self,
        state: &mut State,
        request: &Request,
        reply: Reply,
    ) -> Result<(), String> {
        if self.request.as_ref() != Some(request) {
            return Err("native camera frame acknowledgement mismatch".into());
        }
        match (request, reply) {
            (Request::SampleMotion { axis: 1 }, Reply::Coordinate(y)) => {
                self.y = y;
                self.request = Some(Request::SampleMotion { axis: 0 });
            }
            (Request::SampleMotion { axis: 0 }, Reply::Coordinate(x)) => {
                self.request = Some(Request::Scroll {
                    position: [x, self.y],
                    immediate: 0,
                });
            }
            (Request::Scroll { .. } | Request::CancelMotion, Reply::Done) => {
                self.publish_pending(state);
            }
            (Request::Synchronize, Reply::Done) => self.finish(state),
            _ => return Err("native camera frame reply mismatch".into()),
        }
        Ok(())
    }
}
