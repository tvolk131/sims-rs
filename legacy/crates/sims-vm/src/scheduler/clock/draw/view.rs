//! 4d4ffd's unconditional frame-word clear and mode branch. Recording mode1
//! returns its actual next branch; this helper does not implement recording.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct View {
    pub mode: u32,
    pub frame_word: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    Return,
    Recording,
}
impl View {
    pub fn begin(&mut self) -> Next {
        self.frame_word = 0;
        if self.mode == 1 {
            Next::Recording
        } else {
            Next::Return
        }
    }
}

/// 4ffbb3's initial test, before any active loading work is attempted.
pub fn unload_returns(mode: u32) -> bool {
    mode == 0
}
