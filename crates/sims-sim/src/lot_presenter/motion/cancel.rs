//! Original4b0333 cancellation ownership; graphics remain explicit services.
use crate::lot_presenter::centering::State;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Invalidate,
    ResolveSurface,
    FlushSurface { token: u32 },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Done,
    Surface(u32),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    request: Option<Request>,
}
impl Cursor {
    pub fn begin(state: &mut State) -> Self {
        if state.motion_disabled != 0 || state.camera_mode != 1 {
            return Self { request: None };
        }
        state.camera_mode = 0;
        state.motion_active = 0; //4c9b49; no curve/endpoint fields are cleared.
        let bits = state.saved_value_bits;
        // FLD/FSTP preserves finite/infinite bits and quiets a signaling NaN
        // under the original masked floating-point exception policy.
        state.application_value_bits = if bits & 0x7f800000 == 0x7f800000 && bits & 0x7fffff != 0 {
            bits | 0x400000
        } else {
            bits
        };
        state.moving = 0;
        Self {
            request: Some(Request::Invalidate),
        }
    }
    pub fn request(&self) -> Option<&Request> {
        self.request.as_ref()
    }
    pub fn complete(&self) -> bool {
        self.request.is_none()
    }
    pub fn acknowledge(&mut self, request: &Request, reply: Reply) -> Result<(), String> {
        if self.request.as_ref() != Some(request) {
            return Err("native camera cancellation acknowledgement mismatch".into());
        }
        let next = match (request, reply) {
            (Request::Invalidate, Reply::Done) => Some(Request::ResolveSurface),
            (Request::ResolveSurface, Reply::Surface(token)) => {
                Some(Request::FlushSurface { token })
            }
            (Request::FlushSurface { .. }, Reply::Done) => None,
            _ => return Err("native camera cancellation reply mismatch".into()),
        };
        self.request = next;
        Ok(())
    }
}
