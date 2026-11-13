use crate::{Error, Result};
use std::fmt;
use std::path::{Path, PathBuf};

/// How the game's files are arranged on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// The layout the CD installers produce (the Complete Collection and the
    /// individual discs): one directory with `GameData`, `ExpansionShared`
    /// and one directory per expansion pack, each holding its FAR archives
    /// directly. Checked against a real Complete Collection install.
    Cd,
}

/// The installed expansion packs, as the original's edition bits.
///
/// The bit values and directory names follow the original's mount code
/// (0x43ae81). The original reads them from the registry. Here they come
/// from which pack directories hold a FAR archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Expansions(pub u8);

impl Expansions {
    /// Every pack, in bit order, with the directory it installs to.
    pub const ALL: [(u8, &'static str, &'static str); 8] = [
        (1, "ExpansionPack", "Livin' Large"),
        (2, "ExpansionPack2", "House Party"),
        (4, "ExpansionPack3", "Hot Date"),
        (8, "ExpansionPack4", "Vacation"),
        (16, "Deluxe", "Deluxe Edition"),
        (32, "ExpansionPack5", "Unleashed"),
        (64, "ExpansionPack6", "Superstar"),
        (128, "ExpansionPack7", "Makin' Magic"),
    ];

    pub fn contains(self, bit: u8) -> bool {
        self.0 & bit != 0
    }

    /// Names of the installed packs, in bit order.
    pub fn names(self) -> impl Iterator<Item = &'static str> {
        Self::ALL
            .into_iter()
            .filter(move |(bit, ..)| self.contains(*bit))
            .map(|(.., name)| name)
    }
}

impl fmt::Display for Expansions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<_> = self.names().collect();
        if names.is_empty() {
            write!(f, "none")
        } else {
            write!(f, "{}", names.join(", "))
        }
    }
}

/// A directory the original searches for objects, relative to the install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    /// Path relative to the install root, with `/` separators, as the
    /// original names it.
    pub path: String,
    /// Whether subdirectories are searched too.
    pub recursive: bool,
}

#[derive(Debug, Clone)]
pub struct Install {
    pub root: PathBuf,
    pub layout: Layout,
    pub expansions: Expansions,
}

impl Install {
    /// Recognizes the install at `root`.
    pub fn detect(root: &Path) -> Result<Self> {
        let not_an_install = |reason: &str| Error::NotAnInstall {
            root: root.to_path_buf(),
            reason: reason.to_owned(),
        };
        if !root.is_dir() {
            return Err(not_an_install("not a directory"));
        }
        if resolve(root, "GameData/Behavior.iff")?.is_none() {
            return Err(not_an_install("GameData/Behavior.iff not found"));
        }
        if resolve(root, "GameData/Objects")?.is_none() {
            return Err(not_an_install("GameData/Objects not found"));
        }
        let mut expansions = Expansions::default();
        for (bit, directory, _) in Expansions::ALL {
            if let Some(path) = resolve(root, directory)?
                && holds_archive(&path)?
            {
                expansions.0 |= bit;
            }
        }
        Ok(Self {
            root: root.to_path_buf(),
            layout: Layout::Cd,
            expansions,
        })
    }

    /// The user objects directory, which neighborhood characters follow.
    pub(crate) const USER_OBJECTS: &'static str = "GameData/UserObjects";

    /// The directories objects come from, in the order the original scans
    /// them (0x43c609): the base game, Downloads, the packs in bit order,
    /// the shared directories, and Makin' Magic last. `GameData/Global`
    /// comes first here although the object scan does not visit it: it
    /// holds the global and semi-global behavior files.
    ///
    /// The original registers the same directories with its file manager
    /// in another order (0x43ae81). That order does not show: each
    /// directory has its own file table.
    pub fn mounts(&self) -> Vec<Mount> {
        let mount = |path: &str, recursive| Mount {
            path: path.to_owned(),
            recursive,
        };
        let mut mounts = vec![
            mount("GameData/Global", true),
            mount("GameData/Objects", true),
            mount(Self::USER_OBJECTS, true),
            mount("Downloads", true),
        ];
        let (magic, earlier) = Expansions::ALL.split_last().expect("eight packs");
        for &(bit, directory, _) in earlier {
            if self.expansions.contains(bit) {
                mounts.push(mount(directory, true));
            }
        }
        // The original compares the edition as a signed number: anything
        // beyond Livin' Large and House Party scans the shared directories.
        if self.expansions.0 > 3 {
            mounts.push(mount("ExpansionShared", true));
            mounts.push(mount("TemplateNPCs", false));
        }
        if self.expansions.contains(magic.0) {
            mounts.push(mount(magic.1, true));
        }
        mounts
    }

    /// Finds a file by its path relative to the install, ignoring case.
    pub fn file(&self, path: &str) -> Result<PathBuf> {
        resolve(&self.root, path)?.ok_or_else(|| Error::Missing {
            root: self.root.clone(),
            path: path.to_owned(),
        })
    }
}

/// Resolves a `/`-separated relative path one component at a time,
/// preferring an exact match and otherwise matching ASCII case-insensitively,
/// as Windows does. Returns None when any component is absent.
pub(crate) fn resolve(root: &Path, relative: &str) -> Result<Option<PathBuf>> {
    let mut path = root.to_path_buf();
    for component in relative.split('/').filter(|c| !c.is_empty()) {
        let exact = path.join(component);
        if exact.symlink_metadata().is_ok() {
            path = exact;
            continue;
        }
        let entries = std::fs::read_dir(&path).map_err(|e| Error::io(&path, e))?;
        let mut found = None;
        for entry in entries {
            let entry = entry.map_err(|e| Error::io(&path, e))?;
            if entry
                .file_name()
                .as_encoded_bytes()
                .eq_ignore_ascii_case(component.as_bytes())
            {
                found = Some(entry.path());
                break;
            }
        }
        match found {
            Some(next) => path = next,
            None => return Ok(None),
        }
    }
    Ok(Some(path))
}

fn holds_archive(directory: &Path) -> Result<bool> {
    if !directory.is_dir() {
        return Ok(false);
    }
    let entries = std::fs::read_dir(directory).map_err(|e| Error::io(directory, e))?;
    for entry in entries {
        let entry = entry.map_err(|e| Error::io(directory, e))?;
        let name = entry.file_name();
        let name = name.as_encoded_bytes();
        if name.len() > 4 && name[name.len() - 4..].eq_ignore_ascii_case(b".far") {
            return Ok(true);
        }
    }
    Ok(false)
}
