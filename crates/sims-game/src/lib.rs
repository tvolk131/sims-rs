//! The game: a booted lot, run at the original's tick rate and drawn with
//! its people and the live-mode interface, in one window.
//!
//! This crate joins the others through their public interfaces. It reads
//! the simulation's state to draw it and changes it only through the
//! lot's inputs (`sims_vm::boot::Lot`: stepping it, the player's orders,
//! the speed and the pause); the simulation never depends on it.
//!
//! - [`game`]: the game without its window, so it is tested headless.
//! - [`pace`]: the speed, as ticks owed for the time passed.
//! - [`view`]: the house view's camera.
//! - [`people`]: the people drawn into the lot, behind what is in front
//!   of them.
//! - [`hud`]: the control panel and dialogs over the lot.
//! - [`pick`]: which object or person a click lands on.
//! - [`sound`]: the simulation's sound events, mixed by `sims-audio`;
//!   `output` plays them on the device (on macOS, or with the `sound`
//!   feature).

// An app shell: it reads the wall clock and draws with ordinary floats;
// nothing it computes reaches the simulation (see the workspace lint
// table).
#![allow(clippy::disallowed_methods)]

pub mod game;
pub mod hud;
pub mod menu;
#[cfg(any(target_os = "macos", feature = "sound"))]
pub mod output;
pub mod pace;
pub mod people;
pub mod pick;
pub mod sound;
pub mod view;

pub use game::{Game, Options, Outcome};
