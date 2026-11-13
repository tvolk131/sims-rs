use std::fmt;

#[derive(Debug)]
pub enum Error {
    Content(sims_content::Error),
    /// An object whose GUID the catalog has no definition for.
    MissingDefinition(u32),
    /// Content that needs something of the original that is not ported.
    Unsupported(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Content(error) => error.fmt(f),
            Self::MissingDefinition(guid) => write!(f, "no definition for GUID 0x{guid:08X}"),
            Self::Unsupported(message) => write!(f, "unsupported: {message}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<sims_content::Error> for Error {
    fn from(error: sims_content::Error) -> Self {
        Self::Content(error)
    }
}
