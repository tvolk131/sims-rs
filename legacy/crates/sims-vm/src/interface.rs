//! Original interface kernels. The root owner below implements4ce44f's
//! mode-dirty (mask2) update for an existing Live frontend. The mode_change,
//! tool and buy modules retain separate transition/selection semantics. The
//! publication module retains4ce390's caller, with world/UI children explicit;
//! none of these silently extends the fixed Live frontend host.
use serde::{Deserialize, Serialize};
pub mod buy;
pub mod current_tool;
pub mod drawing;
pub mod live_panel;
pub mod mode_change;
pub mod options;
pub mod panel;
pub mod panel_dispatch;
pub mod panel_switch;
pub mod person_selection;
pub mod property_refresh;
pub mod publication;
pub mod speed;
pub mod state;
pub mod tool;
pub mod transport;
pub mod view_control;
pub mod window_pause;
pub mod world_publication;

/// Read afresh after each synchronous service, as the original routine does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inputs {
    /// App+90,+f5,+f8. The closing gate is checked only on entry.
    pub menu_present: bool,
    pub menu_active: u8,
    pub closing: u8,
    /// Interface+8,+40; View+1f8.
    pub mode: i32,
    pub paused: u8,
    pub include_visible: u8,
}
impl Inputs {
    fn menu_blocks(self) -> bool {
        self.menu_present && self.menu_active == 0
    }
    fn live(self) -> Result<(), String> {
        if self.mode == 2 {
            Ok(())
        } else {
            Err("interface mode-dirty owner requires Live mode; other mode branches are unavailable".into())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    ///4ab992, including the selected-person/room visibility inputs.
    RoomMask(u8),
    ///4c4222(0), including its conditional83 notification.
    DrawingMode,
    ///57a8e3, whose disabled-audio branch is an original early return.
    ResumeAudio,
    ///World globals19=0 at4ce56f, after the preceding callbacks.
    PublishLiveMode,
    ///57a6fb(-1).
    StopAudio,
    ///57a54c("bkground_live",-1).
    PlayLiveBackground,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Drawing,
    Audio,
    Publish,
    MusicGate,
    Background,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveModeUpdate {
    stage: Stage,
    pending: Option<Request>,
}
impl Default for LiveModeUpdate {
    fn default() -> Self {
        Self {
            stage: Stage::Entry,
            pending: None,
        }
    }
}
impl LiveModeUpdate {
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete && self.pending.is_none()
    }
    pub fn request(&self) -> Option<&Request> {
        self.pending.as_ref()
    }
    pub fn validate(&self) -> Result<(), String> {
        let valid = match (&self.pending, self.stage) {
            (None, _) => true,
            (Some(Request::RoomMask(_)), Stage::Drawing)
            | (Some(Request::DrawingMode), Stage::Audio)
            | (Some(Request::ResumeAudio), Stage::Publish)
            | (Some(Request::PublishLiveMode), Stage::MusicGate)
            | (Some(Request::StopAudio), Stage::Background)
            | (Some(Request::PlayLiveBackground), Stage::Complete) => true,
            _ => false,
        };
        if valid {
            Ok(())
        } else {
            Err("interface update lost its pending service".into())
        }
    }
    /// A service is acknowledged only after its real host operation completes.
    pub fn acknowledge(&mut self, request: &Request) -> Result<(), String> {
        self.validate()?;
        if self.pending.as_ref() != Some(request) {
            return Err("interface update reply differs".into());
        }
        self.pending = None;
        Ok(())
    }
    pub fn advance(&mut self, input: Inputs) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending.clone());
        }
        let request = loop {
            match self.stage {
                Stage::Entry => {
                    if input.menu_blocks() || input.closing != 0 {
                        self.stage = Stage::Complete;
                        return Ok(None);
                    }
                    input.live()?;
                    self.stage = Stage::Drawing;
                    break Request::RoomMask(input.include_visible);
                }
                Stage::Drawing => {
                    input.live()?;
                    self.stage = Stage::Audio;
                    break Request::DrawingMode;
                }
                Stage::Audio => {
                    input.live()?;
                    self.stage = Stage::Publish;
                    if input.paused == 0 {
                        break Request::ResumeAudio;
                    }
                }
                Stage::Publish => {
                    input.live()?;
                    self.stage = Stage::MusicGate;
                    break Request::PublishLiveMode;
                }
                Stage::MusicGate => {
                    // Native rechecks App+90/+f5 here, but not App+f8.
                    if input.menu_blocks() {
                        self.stage = Stage::Complete;
                        return Ok(None);
                    }
                    self.stage = Stage::Background;
                    break Request::StopAudio;
                }
                Stage::Background => {
                    input.live()?;
                    self.stage = Stage::Complete;
                    break Request::PlayLiveBackground;
                }
                Stage::Complete => return Ok(None),
            }
        };
        self.pending = Some(request.clone());
        Ok(Some(request))
    }
}
