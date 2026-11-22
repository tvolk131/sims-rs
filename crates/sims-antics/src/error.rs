use std::fmt;

/// 1-based line and column location in a `.antics` or `object.toml` source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub line: usize,
    pub column: usize,
}

impl Span {
    #[must_use]
    pub const fn new(line: usize, column: usize) -> Self {
        Self { line, column }
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.column)
    }
}

/// Errors produced while lexing, parsing, lowering, encoding, or lifting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub span: Option<Span>,
    pub message: String,
}

impl Error {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            span: None,
            message: message.into(),
        }
    }

    pub fn at(span: Span, message: impl Into<String>) -> Self {
        Self {
            span: Some(span),
            message: message.into(),
        }
    }

    pub fn with_context(self, context: impl fmt::Display) -> Self {
        Self {
            span: self.span,
            message: format!("{context}: {}", self.message),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.span {
            Some(span) => write!(f, "{span}: {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for Error {}

impl From<sims_formats::Error> for Error {
    fn from(error: sims_formats::Error) -> Self {
        Self::new(error.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
