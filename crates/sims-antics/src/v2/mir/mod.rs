//! MIR: the typed, structured middle representation of one lowered v2 tree body.
//!
//! A body is a control-flow tree of [`Stmt`]s: simple instructions (each with a typed view,
//! [`InstKind`]: declaration, store, `return` / `break` / `continue` exit, or other), `if` chains and
//! `while` loops. Every branch of the tree is a structured edge. A body outside the modelled
//! subset is one `parse_body` returns `None` for: one with `->` branch targets, comments, `#`, a
//! line that is not one `;`-terminated statement or a block header, or a numbered local slot. (A
//! same-line label such as `done: return true;` still parses, as one opaque instruction.) The
//! passes that need the tree (the optimizer, `pin_in_place`) leave such a body untouched. The
//! others still run on it: `drop_unread_entry_save` works line by line (checking definitions on a
//! one-statement-per-line split when that parses), and `assign_frame_slots` uses the full `.air`
//! parser and rewrites tokens.
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
