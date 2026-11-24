//! MIR: the typed, structured middle representation of one lowered v2 tree body.
//!
//! A body is a control-flow tree of [`Stmt`]s: simple instructions (each with a typed view,
//! [`InstKind`]: declaration, store, `return` / `break` / `continue` exit, or other), `if` chains and
//! `while` loops. Every branch of the tree is a structured edge; the only unstructured control
//! flow `.air` has (labels, `->` targets, several statements per line) puts a body outside the
//! modelled subset, and every pass leaves such a body untouched.
//!
//! The modules:
//! - [`body`]: the node tree and block walkers;
//! - [`text`]: `.air` body text <-> MIR (`parse_body`, `print_nodes`);
//! - [`inst`]: the typed instruction view and operand facts;
//! - [`effects`]: `stack_obj` and last-result effects;
//! - [`liveness`]: local liveness for dead stores, copy forwarding and coalescing;
//! - [`frame`]: frame allocation (pinned slot, `@slot(N)` placement, local renaming);
//! - [`entry_save`]: the Sim-tree `stack_obj` entry save;
//! - [`emit`]: the emitter lowering writes a body through;
//! - [`pipeline`]: the per-tree backend pipeline from a lowered body to final `.air`.
//!
//! The optimizer passes over the MIR live in `v2::opt`.

use std::collections::{BTreeMap, BTreeSet};

mod body;
mod effects;
mod emit;
mod entry_save;
mod frame;
mod inst;
mod liveness;
mod pipeline;
#[cfg(test)]
mod tests;
mod text;

pub(crate) use self::body::*;
pub(crate) use self::effects::*;
pub(crate) use self::emit::*;
pub(crate) use self::entry_save::*;
pub(crate) use self::frame::*;
pub(crate) use self::inst::*;
pub(crate) use self::liveness::*;
pub(crate) use self::pipeline::*;
pub(crate) use self::text::*;
