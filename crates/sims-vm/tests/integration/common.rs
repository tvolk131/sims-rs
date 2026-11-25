//! What several suites read from the install and none changes, built once
//! per process.
//!
//! Only immutable values are cached: a suite that needs to change one
//! clones it (a [`Program`] clone is far cheaper than linking again). A
//! lot, a world or a store is never shared; each test boots or opens its
//! own.

use sims_content::{Catalog, Index, Install, Neighborhood};
use sims_vm::Program;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The install `SIMS_INSTALL` names, if any.
pub fn install_root() -> Option<PathBuf> {
    std::env::var_os("SIMS_INSTALL").map(PathBuf::from)
}

/// The install with its `UserData` neighborhood's characters, as a lot's
/// load indexes it: the index, the catalog built from it, and the program
/// linked over the whole catalog, before any per-lot field (zoning,
/// expansions, animations, people) is set.
pub struct Base {
    pub install: Install,
    pub neighborhood: Neighborhood,
    pub index: Index,
    pub catalog: Catalog,
    pub program: Program,
}

/// [`Base`] for `root`, built at its first use. Every caller passes the
/// install `SIMS_INSTALL` names, so one per process is enough; another
/// root fails loudly rather than reading the wrong files.
pub fn base(root: &Path) -> &'static Base {
    static BASE: OnceLock<Base> = OnceLock::new();
    let base = BASE.get_or_init(|| {
        let install = Install::detect(root).unwrap();
        let neighborhood = Neighborhood::open(&install.file("UserData").unwrap()).unwrap();
        let mut index = Index::build(&install).unwrap();
        index
            .add_characters(&neighborhood.characters().unwrap().unwrap())
            .unwrap();
        let catalog = Catalog::build(&index).unwrap();
        let program =
            Program::link(&index, &catalog, catalog.objects.iter().map(|o| o.guid)).unwrap();
        Base {
            install,
            neighborhood,
            index,
            catalog,
            program,
        }
    });
    assert_eq!(
        base.install.root,
        Install::detect(root).unwrap().root,
        "the cached install base is for another install"
    );
    base
}
