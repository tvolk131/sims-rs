//! Retained cXWorld+10, separate from backing array dimensions and global23.
//! The415e20 caller ignores requests below4.4778fc publishes a changed size
//! before its array work; this type owns that field, not the array/room effects.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub active_size: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resize {
    pub old: i32,
    pub new: i32,
}

impl State {
    /// Raw415e20 argument. The earlier global23 caller maps zero to64 before
    /// entering415e20; do not apply that conversion a second time here.
    pub fn begin_resize(&mut self, requested: i32) -> Option<Resize> {
        if requested < 4 || requested == self.active_size {
            return None;
        }
        let update = Resize {
            old: self.active_size,
            new: requested,
        };
        self.active_size = requested;
        Some(update)
    }

    /// Stable original-lot adapters support the installed64-stride profile.
    /// An unsupported intermediate allocation extent stays an explicit error.
    pub fn checked_size(self, stride: usize) -> Result<usize, String> {
        let size =
            usize::try_from(self.active_size).map_err(|_| "negative retained active world size")?;
        if size < 4 || size > stride {
            return Err("retained active world size exceeds supported backing arrays".into());
        }
        Ok(size)
    }
}
