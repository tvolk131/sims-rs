//! Finds the files a person is drawn from: skeletons and appearances
//! (`.cmx.bcf`, `.cmx`), meshes (`.bmf`, `.skn`), skin textures (`.bmp`)
//! and animation samples (`.cfp`).
//!
//! The original resolves these names through its avatar registry, which
//! mounts the game's directories and archives in an order the old
//! prototype traced (legacy/crates/sims-sim/src/pose/clothing/catalog_paths.rs)
//! but which is not ported. Here every file is known by its own name,
//! ignoring ASCII case and any directory an archive member's name holds,
//! and the sources are searched in a fixed order ([`SOURCES`]): the base
//! game first, then the expansions in the order the original mounts their
//! object directories, then `Downloads`. A name two sources hold with
//! different bytes takes the first, and [`Assets::conflicts`] lists it.
//! That this order picks what the original draws is **unverified**; on the
//! Goth family every file it reads either has one source or identical
//! copies (see the gated tests).

use sims_content::{Member, Resource};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Where person files are searched, in order, relative to the install.
/// A directory is listed (with its subdirectories); every FAR archive in
/// it adds its members too.
pub const SOURCES: &[&str] = &[
    "GameData/Skins",
    "GameData/Animation",
    "GameData/Textures",
    "ExpansionPack",
    "ExpansionPack2",
    "ExpansionPack3",
    "ExpansionPack4",
    "Deluxe",
    "ExpansionPack5",
    "ExpansionPack6",
    "ExpansionPack7",
    "ExpansionShared",
    "Downloads",
];

/// The extensions of the files a person is drawn from.
const KINDS: &[&str] = &[".bcf", ".cmx", ".bmf", ".skn", ".bmp", ".cfp"];

/// Every person file of an install, by lowercase name.
#[derive(Debug, Clone, Default)]
pub struct Assets {
    files: BTreeMap<String, Vec<Resource>>,
}

impl Assets {
    /// Lists the person files under `root` ([`SOURCES`]). A source that is
    /// missing adds nothing; an archive that cannot be read is an error.
    pub fn scan(root: &Path) -> Result<Self, String> {
        let mut assets = Self::default();
        for (mount, source) in SOURCES.iter().enumerate() {
            let Some(directory) = resolve(root, source) else {
                continue;
            };
            let mut paths = Vec::new();
            list(&directory, &mut paths)?;
            for path in paths {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_ascii_lowercase())
                    .unwrap_or_default();
                if name.ends_with(".far") {
                    for member in far_members(&path)? {
                        assets.add(Resource {
                            mount,
                            path: path.clone(),
                            member: Some(member),
                        });
                    }
                } else {
                    assets.add(Resource {
                        mount,
                        path,
                        member: None,
                    });
                }
            }
        }
        Ok(assets)
    }

    fn add(&mut self, resource: Resource) {
        let name = key(&resource.name());
        if KINDS.iter().any(|kind| name.ends_with(kind)) {
            self.files.entry(name).or_default().push(resource);
        }
    }

    /// How many distinct names are known.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Whether no file is known.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Every known name, in lowercase and sorted.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }

    /// Whether a file of this name is known.
    pub fn contains(&self, name: &str) -> bool {
        self.files.contains_key(&key(name))
    }

    /// The file chosen for `name`: the first source's.
    pub fn find(&self, name: &str) -> Option<&Resource> {
        self.files.get(&key(name)).and_then(|all| all.first())
    }

    /// Reads the file chosen for `name`.
    pub fn read(&self, name: &str) -> Result<Vec<u8>, String> {
        let resource = self
            .find(name)
            .ok_or_else(|| format!("no person file is named {name:?}"))?;
        resource.read().map_err(|e| e.to_string())
    }

    /// Every source of `name`, in search order.
    pub fn sources(&self, name: &str) -> &[Resource] {
        self.files.get(&key(name)).map_or(&[], Vec::as_slice)
    }

    /// Whether the sources of `name` disagree: two of them hold different
    /// bytes. Reads every source.
    pub fn conflicts(&self, name: &str) -> Result<bool, String> {
        let mut first: Option<Vec<u8>> = None;
        for resource in self.sources(name) {
            let bytes = resource.read().map_err(|e| e.to_string())?;
            match &first {
                Some(f) if *f != bytes => return Ok(true),
                Some(_) => {}
                None => first = Some(bytes),
            }
        }
        Ok(false)
    }
}

/// A file's key: its own name, without any directory an archive member's
/// name holds, in ASCII lowercase.
fn key(name: &str) -> String {
    name.rsplit(['\\', '/'])
        .next()
        .unwrap_or(name)
        .to_ascii_lowercase()
}

/// Finds `relative` under `root`, matching each component without regard
/// to ASCII case, as the original's file system does.
fn resolve(root: &Path, relative: &str) -> Option<PathBuf> {
    let mut path = root.to_path_buf();
    for part in relative.split('/') {
        let exact = path.join(part);
        if exact.exists() {
            path = exact;
            continue;
        }
        // The first match in name order, so the choice doesn't depend on
        // the order the filesystem lists names in.
        let mut entries: Vec<PathBuf> = std::fs::read_dir(&path)
            .ok()?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .collect();
        entries.sort();
        path = entries.into_iter().find(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(part))
        })?;
    }
    Some(path)
}

/// Every file under `directory`, depth first, each directory's entries in
/// ASCII-uppercase name order.
fn list(directory: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = std::fs::read_dir(directory).map_err(|e| format!("{directory:?}: {e}"))?;
    let mut paths = Vec::new();
    for entry in entries {
        paths.push(entry.map_err(|e| format!("{directory:?}: {e}"))?.path());
    }
    paths.sort_by_cached_key(|p| {
        let name = p.file_name().unwrap_or_default().as_encoded_bytes();
        (name.to_ascii_uppercase(), name.to_vec())
    });
    for path in paths {
        if path.is_dir() {
            list(&path, out)?;
        } else {
            out.push(path);
        }
    }
    Ok(())
}

fn far_members(path: &Path) -> Result<Vec<Member>, String> {
    let fail = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let mut file = File::open(path).map_err(|e| fail(&e))?;
    let len = file.metadata().map_err(|e| fail(&e))?.len();
    let mut header = [0; 16];
    file.read_exact(&mut header).map_err(|e| fail(&e))?;
    let offset = sims_formats::far::manifest_offset(&header, len).map_err(|e| fail(&e))?;
    file.seek(SeekFrom::Start(u64::from(offset)))
        .map_err(|e| fail(&e))?;
    let mut directory = Vec::new();
    file.read_to_end(&mut directory).map_err(|e| fail(&e))?;
    let entries = sims_formats::far::directory(&directory, len).map_err(|e| fail(&e))?;
    Ok(entries
        .into_iter()
        .map(|e| Member {
            name: e.name,
            offset: u64::from(e.offset),
            size: u64::from(e.stored_size),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::key;

    #[test]
    fn a_key_drops_the_member_directory_and_case() {
        assert_eq!(key("Skins\\B009FCChd_01.cmx.bcf"), "b009fcchd_01.cmx.bcf");
        assert_eq!(key("adult-skeleton.cmx.bcf"), "adult-skeleton.cmx.bcf");
        assert_eq!(key("a/B.BMP"), "b.bmp");
    }
}
