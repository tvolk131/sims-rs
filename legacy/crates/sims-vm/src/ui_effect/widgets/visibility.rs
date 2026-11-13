//! 50a7a2 panel visibility. 4e5f5d returns a fixed singleton and4e5ff2 reads
//! its byte+1; neither getter invokes callbacks. The virtual child stays explicit.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Call {
    pub window: u32,
    pub slot: u16,
    pub arguments: Vec<u32>,
}
impl Call {
    pub fn new(window: u32, slot: u16, arguments: &[u32]) -> Self {
        Self {
            window,
            slot,
            arguments: arguments.to_vec(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Visibility {
    window: u32,
    show: u32,
    started: bool,
    pending: Option<Call>,
}
impl Visibility {
    pub fn matches(&self, window: u32, show: u32) -> bool {
        self.window == window && self.show == show
    }
    pub fn new(window: u32, show: u32) -> Self {
        Self {
            window,
            show,
            started: false,
            pending: None,
        }
    }
    pub fn complete(&self) -> bool {
        self.started && self.pending.is_none()
    }
    pub fn request(&self) -> Option<&Call> {
        self.pending.as_ref()
    }
    pub fn validate(&self) -> Result<(), String> {
        if let Some(c) = &self.pending {
            let valid = c.window == self.window
                && self.window != 0
                && self.started
                && ((c.slot == 0xa8 && c.arguments == [self.show, 0, 0])
                    || (c.slot == if self.show as u8 != 0 { 0x94 } else { 0x98 }
                        && c.arguments.is_empty()));
            if !valid {
                return Err("panel visibility lost its virtual child".into());
            }
        }
        Ok(())
    }
    pub fn acknowledge(&mut self, c: &Call) -> Result<(), String> {
        self.validate()?;
        if self.pending.as_ref() != Some(c) {
            return Err("panel visibility reply differs".into());
        }
        self.pending = None;
        Ok(())
    }
    pub fn advance(&mut self, policy: u8) -> Result<Option<Call>, String> {
        self.validate()?;
        if self.started {
            return Ok(self.pending.clone());
        }
        self.started = true;
        if self.window != 0 {
            self.pending = Some(if policy != 0 {
                Call::new(self.window, 0xa8, &[self.show, 0, 0])
            } else {
                Call::new(
                    self.window,
                    if self.show as u8 != 0 { 0x94 } else { 0x98 },
                    &[],
                )
            });
        }
        Ok(self.pending.clone())
    }
}
