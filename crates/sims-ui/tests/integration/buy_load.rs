//! How long `BuyCatalog::load` takes over a real install, and its whole
//! result, for comparing two builds. Needs `SIMS_INSTALL`; skips without
//! it. With `SIMS_BUY_DUMP` set to a path, writes the loaded catalog
//! there (`{:#?}`); the file holds game text, so keep it out of the
//! repository.

use crate::common;
use sims_content::{Catalog, Index, Install};
use sims_ui::buy::BuyCatalog;
use std::time::{Duration, Instant};

#[test]
// Wall-clock timing is this test's purpose; it never feeds the simulation.
#[allow(clippy::disallowed_methods)]
fn buy_catalog_load_time_and_dump() {
    let Some(root) = common::install_root() else {
        eprintln!("skipping: set SIMS_INSTALL to time BuyCatalog::load");
        return;
    };
    let install = Install::detect(&root).unwrap();
    let index = Index::build(&install).unwrap();
    let catalog = Catalog::build(&index).unwrap();

    // The first load warms the file cache; the rest are timed.
    let first = BuyCatalog::load(&index, &catalog);
    let mut times: Vec<Duration> = (0..5)
        .map(|_| {
            let start = Instant::now();
            let loaded = BuyCatalog::load(&index, &catalog);
            let elapsed = start.elapsed();
            assert_eq!(format!("{loaded:?}"), format!("{first:?}"));
            elapsed
        })
        .collect();
    times.sort();
    eprintln!(
        "BuyCatalog::load: min {:?}, median {:?}, max {:?} (5 warm runs)",
        times[0], times[2], times[4]
    );

    if let Some(path) = std::env::var_os("SIMS_BUY_DUMP") {
        std::fs::write(&path, format!("{first:#?}\n")).unwrap();
    }
}
