//! Files the game keeps for a saves file, beside it.
//!
//! Each saves file (`slot.redb`) has a folder named for its whole file
//! name, `slot.redb.userdata`, so saves files whose names differ only in
//! extension (`slot.redb`, `slot.backup`) never share a file. Older builds
//! kept files as siblings named by the saves file's stem
//! ([`legacy_sibling`]), which those saves files did share.

use std::path::{Path, PathBuf};

/// `<saves file name>.userdata`, beside `saves`.
pub fn userdata_dir(saves: &Path) -> PathBuf {
    let mut dir = saves.as_os_str().to_owned();
    dir.push(".userdata");
    PathBuf::from(dir)
}

/// Where the user's Options (`Options.iff`, `Optn` 128) are saved:
/// `<saves file name>.userdata/Options.iff`.
pub fn options_path(saves: &Path) -> PathBuf {
    userdata_dir(saves).join("Options.iff")
}

/// The Options file to load: [`options_path`] once it exists, else the
/// `<saves stem>.options.iff` an older build saved, else None. Saving
/// always writes [`options_path`], so the old file is only ever read, and
/// only until the first save.
///
/// The old file names no saves file: every saves file with that stem
/// wrote and read it, so each of them may load it here, as before, until
/// it saves Options of its own.
pub fn options_to_load(saves: &Path) -> Option<PathBuf> {
    [options_path(saves), legacy_sibling(saves, "options.iff")]
        .into_iter()
        .find(|path| path.is_file())
}

/// `<saves stem>.<extension>`, where older builds kept a file beside
/// `saves`. Saves files whose names differ only in extension share it.
pub fn legacy_sibling(saves: &Path, extension: &str) -> PathBuf {
    saves.with_extension(extension)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);
    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "sims-game-user-files-{name}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("temp dir");
            Self(dir)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn saves_files_differing_in_extension_keep_their_own_options() {
        let dir = Path::new("/saves");
        let redb = options_path(&dir.join("slot.redb"));
        let backup = options_path(&dir.join("slot.backup"));
        assert_eq!(redb, dir.join("slot.redb.userdata/Options.iff"));
        assert_eq!(backup, dir.join("slot.backup.userdata/Options.iff"));
        assert_eq!(
            legacy_sibling(&dir.join("slot.redb"), "options.iff"),
            legacy_sibling(&dir.join("slot.backup"), "options.iff"),
            "the old sibling was shared"
        );
    }

    #[test]
    fn options_load_from_the_old_sibling_until_saved_in_the_folder() {
        let dir = TempDir::new("options");
        let saves = dir.0.join("slot.redb");
        assert_eq!(options_to_load(&saves), None, "no options yet");

        let old = dir.0.join("slot.options.iff");
        std::fs::write(&old, b"old").expect("old options");
        assert_eq!(options_to_load(&saves), Some(old.clone()));

        let new = options_path(&saves);
        std::fs::create_dir_all(new.parent().expect("a folder")).expect("folder");
        std::fs::write(&new, b"new").expect("new options");
        assert_eq!(options_to_load(&saves), Some(new), "the saved file wins");
        assert!(old.is_file(), "the old file is left as it was");
    }
}
