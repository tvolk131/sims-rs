//! The tuning the interface reads without the simulation's program
//! agrees with what the program reads. Needs `SIMS_INSTALL`.

use sims_content::{Catalog, Index, Install};
use sims_vm::boot::{Frames, Lot};
use std::path::PathBuf;

#[test]
fn the_viewers_friendship_threshold_is_the_programs() {
    let Some(root) = std::env::var_os("SIMS_INSTALL").map(PathBuf::from) else {
        eprintln!("SIMS_INSTALL is not set; skipping");
        return;
    };
    let install = Install::detect(&root).unwrap();
    let index = Index::build(&install).unwrap();
    let catalog = Catalog::build(&index).unwrap();
    let read = sims_ui::household::friendship_threshold(&index, &catalog).unwrap();
    let house = sims_game::game::goth_house(&root);
    let lot = Lot::load(&root, &house, &[3], Frames::default()).unwrap();
    assert_eq!(
        read,
        lot.program().autonomy_constants().friendship_threshold
    );
    // The install's tuning holds the entry, so this is not the default
    // (25) the reader falls back on.
    assert_eq!(read, 50);
}
