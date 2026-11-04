//! Native43601c/436a52 dialog choices, independent of a particular widget toolkit.
use crate::ObjectId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Presentation {
    /// Captured before formatting/portrait callbacks; not a live owner query.
    pub anchored: bool,
    pub hide_title: bool,
    pub large: bool,
}
impl Presentation {
    pub fn capture(
        caller: Option<ObjectId>,
        owner: Option<ObjectId>,
        guid: Option<u32>,
        mode: u8,
    ) -> Self {
        Self {
            anchored: caller.is_some() && caller == owner,
            hide_title: caller.is_some() && guid == Some(0xc3249a1d),
            large: mode & 128 != 0,
        }
    }
    /// The original optional +120/+11c widget assignment. Timer/event
    /// consumption belongs to the presentation backend, not simulation ticks.
    pub fn widget_setting(self, mode: u8) -> Option<(u32, u32)> {
        (!matches!(mode & 127, 3 | 4) && !self.anchored).then_some((last_button(mode), 40000))
    }
}

pub fn supported(mode: u8) -> bool {
    matches!(mode & 127, 0 | 1 | 2 | 4)
}
pub fn button_count(mode: u8) -> usize {
    match mode & 127 {
        0 | 3 | 4 => 1,
        1 => 2,
        2 => 3,
        _ => 0,
    }
}
pub fn last_button(mode: u8) -> u32 {
    match mode & 127 {
        1 => 3,
        2 => 4,
        _ => 2,
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ButtonSource {
    pub result: u32,
    pub index: u8,
    pub fallback: u16,
}
/// Original construction order: third, secondary, then primary. The displayed
/// buttons can subsequently be ordered by their result IDs.
pub fn buttons(mode: u8, indices: [u8; 3]) -> Vec<ButtonSource> {
    let mode = mode & 127;
    let mut out = vec![];
    if mode == 2 {
        out.push(ButtonSource {
            result: 4,
            index: indices[2],
            fallback: 2,
        });
    }
    if matches!(mode, 1 | 2) {
        out.push(ButtonSource {
            result: 3,
            index: indices[1],
            fallback: 4,
        });
    }
    if mode <= 4 {
        out.push(ButtonSource {
            result: 2,
            index: indices[0],
            fallback: if matches!(mode, 1 | 2) { 3 } else { 1 },
        });
    }
    out
}
/// Native virtual-key/result pairs. Bindings are explicit user input, never a
/// synthesized response. Space only selects a single-button dialog.
pub fn keys(mode: u8) -> Vec<(u32, u32)> {
    let mut out = vec![(13, 2), (27, last_button(mode))];
    if !matches!(mode & 127, 1 | 2) {
        out.push((32, 2));
    }
    out
}
