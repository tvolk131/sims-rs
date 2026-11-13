//! CPState+230 and original4cfc75's ordered setter. Widget publication follows
//! in the frontend's dirty-bit consumer; this is not the whole4ce390 frame.
use serde::{Deserialize, Serialize};
pub const DIRTY: u32 = 0x100000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fields {
    pub value: i32,
    pub flags: u32,
}
impl Fields {
    ///4ce0b9 initializes+230 to1. Other CPState fields are not reconstructed by
    /// this helper; the caller supplies the actual retained dirty flags.
    pub fn constructed(flags: u32) -> Self {
        Self { value: 1, flags }
    }
}
#[derive(Debug, Clone, Copy)]
pub struct Inputs {
    pub app_present: bool,
    pub window_present: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    ///57a54c("UI_whoosh",-4), before the field or dirty flag is written.
    Whoosh,
    ///4d3c39(requested==1), after publication; only with App and App+84.
    WindowLayout(bool),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Write,
    Window,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Setter {
    value: i32,
    force: u8,
    stage: Stage,
    pending: Option<Request>,
}
impl Setter {
    pub fn new(value: i32, force: u8) -> Self {
        Self {
            value,
            force,
            stage: Stage::Entry,
            pending: None,
        }
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete && self.pending.is_none()
    }
    pub fn request(&self) -> Option<&Request> {
        self.pending.as_ref()
    }
    pub fn validate(&self) -> Result<(), String> {
        let valid = match (&self.pending, self.stage) {
            (None, _) => true,
            (Some(Request::Whoosh), Stage::Write) => self.value == 0 && self.force == 0,
            (Some(Request::WindowLayout(collapsed)), Stage::Complete) => {
                *collapsed == (self.value == 1)
            }
            _ => false,
        };
        if valid {
            Ok(())
        } else {
            Err("panel setter lost its service".into())
        }
    }
    pub fn acknowledge(&mut self, request: &Request) -> Result<(), String> {
        self.validate()?;
        if self.pending.as_ref() != Some(request) {
            return Err("panel setter reply differs".into());
        }
        self.pending = None;
        Ok(())
    }
    pub fn advance(
        &mut self,
        fields: &mut Fields,
        input: Inputs,
    ) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending.clone());
        }
        loop {
            match self.stage {
                Stage::Entry => {
                    if fields.value == self.value && self.force == 0 {
                        self.stage = Stage::Complete;
                        return Ok(None);
                    }
                    self.stage = Stage::Write;
                    if self.force == 0 && self.value == 0 {
                        self.pending = Some(Request::Whoosh);
                        break;
                    }
                }
                Stage::Write => {
                    // Read flags after the sound callback, preserving its writes.
                    fields.flags |= DIRTY;
                    fields.value = self.value;
                    self.stage = Stage::Window;
                }
                Stage::Window => {
                    self.stage = Stage::Complete;
                    if input.app_present && input.window_present {
                        self.pending = Some(Request::WindowLayout(self.value == 1));
                        break;
                    }
                }
                Stage::Complete => return Ok(None),
            }
        }
        Ok(self.pending.clone())
    }
}

///53a2c3's Live-button branch when already in mode2; it reads the actual
/// ViewControl+1a0, which can differ from the unpublished CPState value.
pub fn live_button_target(application_mode: i32, displayed: i32) -> Result<i32, String> {
    if application_mode != 2 {
        return Err("Live button requires its mode-change transaction".into());
    }
    Ok(i32::from(displayed == 0))
}

///53ab6d only supplies0/1 to53b2eb. Other values do not become invented UI modes.
pub fn publication(fields: &Fields) -> Option<i32> {
    (fields.flags & DIRTY != 0 && matches!(fields.value, 0 | 1)).then_some(fields.value)
}
