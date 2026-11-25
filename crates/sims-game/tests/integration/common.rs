//! Helpers the integration test modules share.

use sims_game::game::goth_house;
use sims_game::{Frames, Options};
use std::path::PathBuf;

/// The install `SIMS_INSTALL` names, or `None` to skip when it is unset or
/// not a folder.
pub fn install_root() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("SIMS_INSTALL")?);
    dir.is_dir().then_some(dir)
}

/// Options that open the Goth house below `install`, answering dialogs
/// with button 3 and without play mode.
pub fn goth_options(install: PathBuf) -> Options {
    Options {
        house: goth_house(&install),
        install,
        answers: vec![3],
        play: false,
        frames: Frames::default(),
    }
}
