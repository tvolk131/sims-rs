use std::fmt;

#[derive(Debug)]
pub enum Error {
    /// A saved object whose GUID the catalog has no definition for.
    MissingDefinition { object: u16, guid: u32 },
    /// Saved objects were read onto a lot that still holds objects.
    NotCleared,
    /// The snapshot store failed.
    Store(String),
    /// A snapshot that does not decode, or was written by another format
    /// version.
    Snapshot(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingDefinition { object, guid } => {
                write!(f, "object {object} has no definition for GUID 0x{guid:08X}")
            }
            Self::NotCleared => {
                write!(f, "saved objects can only be read onto a cleared lot")
            }
            Self::Store(message) => write!(f, "snapshot store: {message}"),
            Self::Snapshot(message) => write!(f, "snapshot: {message}"),
        }
    }
}

impl std::error::Error for Error {}
