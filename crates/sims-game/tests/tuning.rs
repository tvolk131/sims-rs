//! The tuning a program that links the global file alone reads, as the
//! viewer links it for a world it did not boot, agrees with the booted
//! lot's. Needs `SIMS_INSTALL`.

use sims_content::{Catalog, Index, Install};
use sims_vm::Program;
use sims_vm::boot::{Frames, Lot};
use std::path::PathBuf;

#[test]
fn the_global_file_alone_gives_the_lots_friendship_threshold() {
    let Some(root) = std::env::var_os("SIMS_INSTALL").map(PathBuf::from) else {
        eprintln!("SIMS_INSTALL is not set; skipping");
        return;
    };
    let install = Install::detect(&root).unwrap();
    let index = Index::build(&install).unwrap();
    let catalog = Catalog::build(&index).unwrap();
    let alone = Program::link(&index, &catalog, []).unwrap();
    let house = sims_game::game::goth_house(&root);
    let lot = Lot::load(&root, &house, &[3], Frames::default()).unwrap();
    let threshold = lot.program().autonomy_constants().friendship_threshold;
    assert_eq!(alone.autonomy_constants().friendship_threshold, threshold);
    // The install's tuning holds the entry: not the default, 25.
    assert_eq!(threshold, 50);
}
