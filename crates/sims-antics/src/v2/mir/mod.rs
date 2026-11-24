//! MIR: the typed, structured middle representation of one lowered v2 tree body.
//!
//! A body is a control-flow tree of [`Stmt`]s: simple instructions (each with a typed view,
//! [`InstKind`]: declaration, store, `return` / `break` / `continue` exit, or other), `if` chains and
//! `while` loops. Every branch of the tree is a structured edge.
//!
//! The modelled subset is exactly what `parse_body` accepts. Each non-blank line must be one of:
//! - an `if COND {`, `} else if COND {`, `} else {` or `while COND {` header, or a `}` closer;
//! - one simple statement: exactly one `;`, at the end, and no braces.
//!
//! A line is rejected (and `parse_body` returns `None`) when, outside string literals, it holds
//! `//`, `/*`, `*/`, `->` or `#`, or when `names_local_slot` holds for it: a raw `op` or a
//! `scope(N, ...)` whose scope is the locals scope (25) or not a literal. (`local[N]` and a
//! same-line label such as `done: return true;` still parse, as opaque instructions.)
//!
//! The passes that need the tree (the optimizer, `pin_in_place`) leave a rejected body untouched.
//! The others still run on it: `drop_unread_entry_save` works line by line (checking definitions
//! on a one-statement-per-line split when that parses), and `assign_frame_slots` uses the full
//! `.air` parser and rewrites tokens.
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
