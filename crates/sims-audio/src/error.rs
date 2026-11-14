use std::fmt;
use std::path::{Path, PathBuf};

/// Why a sound file could not be read or decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The bytes do not hold what the format needs. `format` names the
    /// format, `offset` the byte where the problem was found.
    Format {
        format: &'static str,
        offset: usize,
        message: String,
    },
    /// A file could not be read.
    Io { path: PathBuf, message: String },
}

impl Error {
    pub(crate) fn format(format: &'static str, offset: usize, message: impl Into<String>) -> Self {
        Self::Format {
            format,
            offset,
            message: message.into(),
        }
    }

    pub(crate) fn io(path: &Path, error: std::io::Error) -> Self {
        Self::Io {
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Format {
                format,
                offset,
                message,
            } => write!(f, "{format} at byte {offset}: {message}"),
            Self::Io { path, message } => write!(f, "{}: {message}", path.display()),
        }
    }
}

impl std::error::Error for Error {}
