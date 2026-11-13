//! Pending camera displacement branch4a8d25..4a8ddc in original frame4a8935.
//! Enter only after the draw loop selects nonzero pending displacement. Graphics
//! and57a9d4 notification remain acknowledged services, preserving live callback
//! writes and the exact point at which pending displacement is cleared.
use crate::lot_presenter::centering::State;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Passes {
    /// Native frame local-30 and local-28, not simulation tick counters.
    pub count: i32,
    pub phase: i32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    FlushSurface,
    NotifyScroll,
    Synchronize,
    DrawingEnabled,
    CopyBackground,
    SetDrawMode {
        first: u32,
        mask: u32,
        enabled: u32,
    },
    ScrollSurface {
        viewport: [i32; 4],
        include_background: u32,
    },
    Invalidate,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Done,
    Enabled(u8),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    request: Option<Request>,
    include_background: u32,
}
impl Cursor {
    pub fn begin(state: &State) -> Result<Self, String> {
        if state.pending == [0, 0] {
            return Err("native displacement branch requires pending scroll".into());
        }
        Ok(Self {
            request: Some(Request::FlushSurface),
            include_background: 0,
        })
    }
    fn draw_mode(&mut self) {
        self.request = Some(Request::SetDrawMode {
            first: 0,
            mask: 15,
            enabled: 0,
        });
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
        passes: &mut Passes,
        request: &Request,
        reply: Reply,
    ) -> Result<(), String> {
        if self.request.as_ref() != Some(request) {
            return Err("native displacement acknowledgement mismatch".into());
        }
        match (request, reply) {
            (Request::FlushSurface, Reply::Done) => self.request = Some(Request::NotifyScroll),
            (Request::NotifyScroll, Reply::Done) => self.request = Some(Request::Synchronize),
            (Request::Synchronize, Reply::Done) => {
                // Native abs636cbf leaves INT_MIN negative. Both differences
                // and comparisons are signed32; do not use unsigned_abs here.
                if state.pending[0].wrapping_abs()
                    < state.viewport[2].wrapping_sub(state.viewport[0])
                    && state.pending[1].wrapping_abs()
                        < state.viewport[3].wrapping_sub(state.viewport[1])
                {
                    self.include_background = u32::from(state.camera_mode == 0);
                    self.request = Some(Request::DrawingEnabled);
                } else {
                    self.request = Some(Request::Invalidate);
                }
            }
            (Request::DrawingEnabled, Reply::Enabled(enabled)) => {
                if enabled != 0 {
                    self.request = Some(Request::CopyBackground);
                } else {
                    self.draw_mode();
                }
            }
            (Request::CopyBackground, Reply::Done) => self.draw_mode(),
            (Request::SetDrawMode { mask: 15, .. }, Reply::Done) => {
                self.request = Some(Request::SetDrawMode {
                    first: 0,
                    mask: 8,
                    enabled: 1,
                });
            }
            (Request::SetDrawMode { mask: 8, .. }, Reply::Done) => {
                passes.count = passes.count.wrapping_add(1);
                passes.phase = 1;
                self.request = Some(Request::ScrollSurface {
                    viewport: state.viewport,
                    include_background: self.include_background,
                });
            }
            (Request::ScrollSurface { .. } | Request::Invalidate, Reply::Done) => {
                state.pending = [0, 0];
                self.request = None;
            }
            _ => return Err("native displacement reply mismatch".into()),
        }
        Ok(())
    }
}
