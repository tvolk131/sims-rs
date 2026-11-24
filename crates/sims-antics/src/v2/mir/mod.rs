//! MIR: the typed, structured middle representation of one lowered v2 tree body.
//!
//! A body is a control-flow tree of [`Stmt`]s: simple instructions (each with a typed view,
//! [`InstKind`]: declaration, store, `return` / `break` / `continue` exit, or other), `if` chains and
//! `while` loops. Every branch of the tree is a structured edge.
//!
//! The modelled subset is exactly what `parse_body` accepts. Lines are trimmed and blank lines
//! skipped; every check below looks at the line with its string literals' contents removed. Each
//! line must be one of:
//! - an `if COND {` or `while COND {` header (`COND` holding no `;`, `{` or `}`), which opens a
//!   block;
//! - a `}`, `} else if COND {` or `} else {` line, which closes the open block. Every block must be
//!   closed by the matching kind (`} else ...` only after an `if` arm, then a final `}`), so a
//!   stray or missing closer rejects the body;
//! - one simple statement: exactly one `;`, at the end, and no braces.
//!
//! A line is also rejected (and `parse_body` returns `None`) when it holds `//`, `/*`, `*/`, `->`
//! or `#`, or when [`names_local_slot`] holds for it. That function is the definition; it is a
//! conservative token scan that errs towards rejecting. Roughly, it rejects any `op` token that
//! is not a `.field` or an `op:` argument name (so even `let op = 1;`), except the operand-free
//! spelling `op N (0, 0, 0, 0, 0, 0, 0, 0)` of the opcodes in `OPERAND_FREE_OPS`. It also rejects
//! a `scope(N, ...)` unless `N` is a decimal or lowercase-`0x` literal other than 25 (the locals
//! scope).
//!
//! An accepted statement keeps its whole line as text, and [`classify`] gives its typed view of
//! that whole line. A numbered-local operand is not rejected and takes part in that view like
//! any other (`local[0] := x;` is an assignment). A same-line label is not split off: it stays
//! part of the text `classify` sees, so `done: return true;` is [`InstKind::Other`] while
//! `done:local[0] := x;` is an assignment to `done:local[0]`.
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
//! - [`entry_save`](mod@entry_save): the Sim-tree `stack_obj` entry save;
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
