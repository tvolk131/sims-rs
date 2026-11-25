//! Helpers shared by the `sims-audio` integration tests: the install that
//! `SIMS_INSTALL` names, and its sound files and modules read once per run.
//! No game data.

use sims_audio::files::SoundFiles;
use sims_audio::module::Module;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The install `SIMS_INSTALL` names, or None (saying which test skips)
/// when it is not set.
pub fn install(skipping: &str) -> Option<PathBuf> {
    match std::env::var_os("SIMS_INSTALL") {
        Some(path) => Some(PathBuf::from(path)),
        None => {
            eprintln!("SIMS_INSTALL is not set; skipping {skipping}");
            None
        }
    }
}

/// `SoundFiles::scan(root)` for the install, scanned once per run and
/// cloned for each caller. No test changes the install.
pub fn sound_files(root: &Path) -> SoundFiles {
    static FILES: OnceLock<SoundFiles> = OnceLock::new();
    let files = FILES.get_or_init(|| SoundFiles::scan(root).unwrap());
    assert_eq!(files.root, root, "sound_files serves one install per run");
    files.clone()
}

/// `Module::load_all` over [`sound_files`], loaded once per run and cloned
/// for each caller.
pub fn modules(root: &Path) -> Vec<Module> {
    static MODULES: OnceLock<(PathBuf, Vec<Module>)> = OnceLock::new();
    let (loaded_from, modules) = MODULES.get_or_init(|| {
        (
            root.to_path_buf(),
            Module::load_all(&sound_files(root)).unwrap(),
        )
    });
    assert_eq!(loaded_from, root, "modules serves one install per run");
    modules.clone()
}
