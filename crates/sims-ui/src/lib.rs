//! The game's interface, drawn from the original artwork.
//!
//! - [`hud`]: the live-mode control panel.
//! - [`dialog`]: the dialogs scripts show.
//! - [`balloon`]: the help balloons the panels open over the lot.
//! - [`pie`]: the pie menu of what a Sim can do with an object.
//! - [`queue`]: the selected Sim's actions, which a press cancels.
//! - [`thumbnail`]: objects' catalog pictures, as the queue shows them.
//! - [`command`]: what the player's clicks ask for.
//! - [`household`]: what the panel shows, read from the simulation.
//! - [`art`] and [`font`]: the original bitmaps and fonts.
//! - [`frame`]: the pixel buffer everything is drawn into.
//!
//! The interface only reads the simulation: it copies what it shows out
//! of a [`sims_sim::World`] into a [`household::Snapshot`], draws into a
//! [`frame::Frame`] on the CPU (so tests need no window), and turns a
//! press into a [`command::Command`] for the host to carry out.

pub mod art;
pub mod balloon;
pub mod build;
pub mod buy;
pub mod command;
pub mod dialog;
pub mod font;
pub mod frame;
pub mod house;
pub mod household;
pub mod hud;
pub mod pie;
pub mod queue;
pub mod thumbnail;

/// Why art could not be loaded.
#[derive(Debug)]
pub enum Error {
    Content(sims_content::Error),
    Format(sims_formats::Error),
    Io(std::io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Content(e) => e.fmt(f),
            Self::Format(e) => e.fmt(f),
            Self::Io(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;
