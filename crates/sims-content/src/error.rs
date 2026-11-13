use std::fmt;
use std::path::PathBuf;

#[derive(Debug)]
pub enum Error {
    /// A file or directory could not be read.
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// A file could not be decoded.
    Format {
        location: String,
        source: sims_formats::Error,
    },
    /// The directory is not a recognized install.
    NotAnInstall { root: PathBuf, reason: String },
    /// A file the caller needs is absent.
    Missing { root: PathBuf, path: String },
}

impl Error {
    pub(crate) fn format(location: &str, source: sims_formats::Error) -> Self {
        Self::Format {
            location: location.to_owned(),
            source,
        }
    }
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::Format { location, source } => write!(f, "{location}: {source}"),
            Self::NotAnInstall { root, reason } => {
                write!(f, "{} is not a game install: {reason}", root.display())
            }
            Self::Missing { root, path } => {
                write!(
                    f,
                    "{path} is missing from the install at {}",
                    root.display()
                )
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Format { source, .. } => Some(source),
            _ => None,
        }
    }
}
