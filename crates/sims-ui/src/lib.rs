//! The game's interface, drawn from the original artwork.
//!
//! - [`hud`]: the live-mode control panel.
//! - [`album`]: the Photo Album / Family Storybook modal viewer and editor.
//! - [`caf`]: the Create-a-Family and Create-a-Sim screens.
//! - [`neighborhood`]: the neighborhood top controls bar and lot hover popup card.
//! - [`family_bin`]: the Select a Family / Family Bin picker screen.
//! - [`coop`]: the co-op multiplayer Host/Join lobby dialog and in-lot HUD overlay.
//! - [`saves`]: the Save & rotating autosave slot picker dialog.
//! - [`shop`]: the travel shop, wardrobe, pet adoption, and Magic Town vendor picker.
//! - [`cheat_console`]: the classic Sims 1 `Ctrl+Shift+C` cheat console overlay and parser.
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

pub mod album;
pub mod art;
pub mod balloon;
pub mod build;
pub mod buy;
pub mod caf;
pub mod cheat_console;
pub mod command;
pub mod coop;
pub mod dialog;
pub mod family_bin;
pub mod font;
pub mod frame;
pub mod house;
pub mod household;
pub mod hud;
pub mod neighborhood;
pub mod options;
pub mod pie;
pub mod queue;
pub mod saves;
pub mod shop;
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
