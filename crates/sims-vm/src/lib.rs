//! The SimAntics interpreter.
//!
//! [`Program::link`] gathers the code a lot's objects run: the global
//! behaviors, each object's own file and its semi-global file. Tree IDs
//! resolve the way the original's do, and [`Program::signature`] reproduces
//! the per-type signatures saves store, which checks the linking against the
//! original's own data.

mod error;
mod program;

pub use error::Error;
pub use program::{Callback, Library, Program, Tree};

pub type Result<T> = std::result::Result<T, Error>;
