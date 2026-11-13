//! The SimAntics interpreter.
//!
//! [`Program::link`] gathers the code a lot's objects run: the global
//! behaviors, each object's own file and its semi-global file. Tree IDs
//! resolve the way the original's do, and [`Program::signature`] reproduces
//! the per-type signatures saves store, which checks the linking against the
//! original's own data.
//!
//! [`Interpreter`] runs that code against a [`sims_sim::World`]. An
//! object's thread is its saved stack of frames: [`Interpreter::step`]
//! executes the top frame's instruction, and [`Interpreter::run_tree`] runs
//! a tree on an object to completion. Primitives without an implementation,
//! and memory scopes over state the simulation doesn't hold yet, fail with
//! a [`Fault`] that names them.

mod error;
mod fault;
mod interpreter;
mod memory;
mod primitives;
mod program;

pub use error::Error;
pub use fault::{Fault, Location, Trap};
pub use interpreter::{Interpreter, MAX_STACK, Status, Stream};
pub use memory::scope_name;
pub use primitives::primitive_name;
pub use program::{Callback, Library, Program, Tree};

pub type Result<T> = std::result::Result<T, Error>;
