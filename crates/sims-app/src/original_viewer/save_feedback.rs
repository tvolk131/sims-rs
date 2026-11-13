//! Only failures at the player-save output boundary are recoverable.
use super::{atomic_save, validate_output};
use anyhow::Result;
use std::{
    fmt,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy)]
enum Operation {
    Validate,
    Directory,
    Write,
}

/// A typed context attached exclusively after Viewer has prepared and captured
/// its state. Never classify arbitrary I/O errors from the engine as output errors.
#[derive(Debug)]
struct OutputFailure {
    destination: PathBuf,
    operation: Operation,
}

impl fmt::Display for OutputFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let operation = match self.operation {
            Operation::Validate => "destination validation failed",
            Operation::Directory => "could not create the destination directory",
            Operation::Write => "the output write failed",
        };
        write!(
            f,
            "Save failed for {}: {operation}",
            self.destination.display()
        )
    }
}

impl std::error::Error for OutputFailure {}

pub(super) fn write(root: &Path, destination: &Path, bytes: &[u8]) -> Result<()> {
    write_checked(root, destination, bytes, || Ok(()))
}

pub(super) fn write_copy(root: &Path, destination: &Path, canonical: &Path,
    input: Option<&sims_runtime::original_session::player_input::Observation>, bytes: &[u8]) -> Result<()>
{
    write_checked(root, destination, bytes, || super::save_copy::validate(destination, canonical, input))
}

fn write_checked(root: &Path, destination: &Path, bytes: &[u8], alternate: impl FnOnce() -> Result<()>) -> Result<()> {
    let output = |operation| OutputFailure {
        destination: destination.to_path_buf(),
        operation,
    };
    validate_output(root, destination)
        .map_err(|error| error.context(output(Operation::Validate)))?;
    alternate().map_err(|error| error.context(output(Operation::Validate)))?;
    if let Some(parent) = destination.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|error| anyhow::Error::new(error).context(output(Operation::Directory)))?;
    }
    atomic_save(destination, bytes).map_err(|error| error.context(output(Operation::Write)))
}

/// The Message::Save arm consumes this status; all unmarked errors retain the
/// existing fatal fallthrough, as do failures in the subsequent common tail.
pub(super) fn recover(error: anyhow::Error) -> Result<String> {
    recover_for(error, "Save")
}

pub(super) fn recover_copy(error: anyhow::Error) -> Result<String> {
    recover_for(error, "Save a Copy… with this path or another destination")
}

fn recover_for(error: anyhow::Error, action: &str) -> Result<String> {
    let Some(output) = error.downcast_ref::<OutputFailure>() else {
        return Err(error);
    };
    let completion = match output.operation {
        Operation::Validate | Operation::Directory => "No save was written.",
        // atomic_save writes/syncs a new temporary file, then renames it, with
        // no fallible work after rename. Do not promise that every filesystem
        // reports a failed rename without having changed the destination.
        Operation::Write => {
            "Write completion was not confirmed; check the destination before retrying."
        }
    };
    Ok(format!(
        "{error:#}. {completion} Check the destination path, permissions and free space, then retry {action}."
    ))
}

#[cfg(test)]
#[path = "save_feedback/tests.rs"]
mod tests;
