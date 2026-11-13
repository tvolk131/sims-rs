//! 4e7bb5..4e7c99. Each selected binding is rendered in rest pose, then its
//! matrix, mode and posed vertices are restored before the next binding.
use super::Matrix;
use serde::{Deserialize, Serialize};

/// Backend graphics state is explicit; none of these operations advances the
/// animation or simulation. Nonzero native graphics statuses are normal results,
/// distinct from unsupported/invalid host state reported as Err.
pub trait Host {
    fn begin(&mut self) -> Result<i32, String>;
    fn matrix(&mut self, binding: usize) -> Result<Matrix, String>;
    fn vertices(&mut self, binding: usize, rest: bool) -> Result<(), String>;
    fn publish_matrix(&mut self, binding: usize, matrix: Matrix) -> Result<(), String>;
    fn mode(&mut self, binding: usize) -> Result<u32, String>;
    fn set_mode(&mut self, binding: usize, mode: u32) -> Result<(), String>;
    fn draw(&mut self, binding: usize) -> Result<i32, String>;
    fn end(&mut self) -> Result<i32, String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outcome {
    pub success: bool,
    /// The native caller can still copy pixels after an individual draw failure.
    /// Failed End suppresses that copy; skipped Begin preserves the entry flag.
    pub copy: bool,
}

pub fn render(host: &mut impl Host, bindings: &[usize], matrix: Matrix) -> Result<Outcome, String> {
    let mut outcome = Outcome {
        success: true,
        copy: true,
    };
    if host.begin()? != 0 {
        return Ok(outcome);
    }
    for &binding in bindings {
        let saved_matrix = host.matrix(binding)?;
        host.vertices(binding, true)?;
        host.publish_matrix(binding, matrix)?;
        let saved_mode = host.mode(binding)?;
        host.set_mode(binding, 0)?;
        let result = host.draw(binding);
        // Restore even if this backend cannot complete the draw. A native
        // nonzero draw status does not skip subsequent bindings or End.
        host.set_mode(binding, saved_mode)?;
        host.publish_matrix(binding, saved_matrix)?;
        host.vertices(binding, false)?;
        match result {
            Ok(status) => outcome.success &= status == 0,
            Err(error) => {
                host.end()?;
                return Err(error);
            }
        }
    }
    outcome.copy = host.end()? == 0;
    outcome.success &= outcome.copy;
    Ok(outcome)
}
