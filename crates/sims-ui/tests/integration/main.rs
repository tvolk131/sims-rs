//! `sims-ui`'s integration tests, built as one binary.
//!
//! Each module was a `tests/<name>.rs` binary of its own; `common` opens a
//! saved house for them (and for `examples/hud.rs`).

mod album;
mod book;
mod build;
mod buy;
mod caf;
mod cheat_console;
mod common;
mod coop;
mod dialog;
mod family_bin;
mod hud;
mod invariants;
mod loading;
mod neighborhood;
mod options;
mod phonebook;
mod pie;
mod saves;
mod shop;
mod travel;
mod ui_ratchet;
