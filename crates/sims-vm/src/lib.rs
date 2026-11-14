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
//! executes the top frame's instruction, [`Interpreter::turn`] runs the
//! thread until it yields or returns, and [`Interpreter::run_tree`] runs a
//! tree on an object to completion. Primitives without an implementation,
//! and memory scopes over state the simulation doesn't hold yet, fail with
//! a [`Fault`] that names them.
//!
//! [`Interpreter::create`] is the original's object factory, and
//! [`Interpreter::remove_now`] and [`Interpreter::delete`] take an object
//! off the lot and destroy it. [`Interpreter::resize`],
//! [`Interpreter::clear`], [`Interpreter::read_types`],
//! [`Interpreter::reconcile_controllers`], [`Interpreter::assign_rooms`]
//! and [`Interpreter::create_indicators`] are the lot load's steps that
//! run scripts or need the linked code.

mod error;
mod factory;
mod fault;
mod interpreter;
mod load;
mod memory;
mod objects;
mod primitives;
mod program;
mod removal;

pub use error::Error;
pub use factory::TURN_BUDGET;
pub use fault::{Fault, Location, Trap};
pub use interpreter::{Interpreter, MAX_STACK, Status, Stream};
pub use load::{INDICATORS, Reconciled, Roomed};
pub use memory::scope_name;
pub use objects::SavedRead;
pub use primitives::primitive_name;
pub use program::{Callback, DEFINITION_WORDS, Library, Program, Tree};

pub type Result<T> = std::result::Result<T, Error>;
