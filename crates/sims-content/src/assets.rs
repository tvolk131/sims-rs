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
//! That this order picks what the original draws is **unverified**: the
//! original's texture search puts the expansions before the base game
//! (0x413ccc), the reverse of this order. The Complete Collection has 47
//! such names (accessory textures, a few animation and mesh files); on the
//! Goth family every file it reads either has one source or identical
//! copies (see the gated tests). Appearance catalogs are loaded in the
//! original's own order instead (see sims-people's `load_appearance`).

use crate::pool::worker_count;
use crate::{Member, Resource};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Where person files are searched, in order, relative to the install.
/// A directory is listed (with its subdirectories); every FAR archive in
/// it adds its members too.
pub const SOURCES: &[&str] = &[
    "GameData/Skins",
    "GameData/Animation",
    "GameData/Textures2",
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
/// `.tga` names a texture only Make New Character's lists read
/// ([`crate::Wardrobe`]).
const KINDS: &[&str] = &[".bcf", ".cmx", ".bmf", ".skn", ".bmp", ".tga", ".cfp"];

/// Every person file of an install, by lowercase name.
#[derive(Debug, Clone, Default)]
pub struct Assets {
    /// The install the files were found under.
    root: PathBuf,
    files: BTreeMap<String, Vec<Resource>>,
    /// Where each appearance is loaded from, by lowercase name: the
    /// catalog's file name and source. Built on first use
    /// (sims-people's `load_appearance`).
    appearances: OnceLock<BTreeMap<String, (String, Resource)>>,
}

impl Assets {
    /// Lists the person files under `root` ([`SOURCES`]). A source that is
    /// missing adds nothing; an archive that cannot be read is an error.
    pub fn scan(root: &Path) -> Result<Self, String> {
        let scan_one = |(mount, source): (usize, &str)| -> Result<Vec<Resource>, String> {
            let Some(directory) = resolve(root, source) else {
                return Ok(Vec::new());
            };
            let mut paths = Vec::new();
            list(&directory, &mut paths)?;
            let mut found = Vec::new();
            for path in paths {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_ascii_lowercase())
                    .unwrap_or_default();
                if name.ends_with(".far") {
                    for member in far_members(&path)? {
                        found.push(Resource {
                            mount,
                            path: path.clone(),
                            member: Some(member),
                            inline: None,
                        });
                    }
                } else {
                    found.push(Resource {
                        mount,
                        path,
                        member: None,
                        inline: None,
                    });
                }
            }
            Ok(found)
        };
        let indexed: Vec<(usize, &str)> = SOURCES.iter().copied().enumerate().collect();
        let workers = worker_count(indexed.len());
        let per_source: Vec<Result<Vec<Resource>, String>> = if workers <= 1 {
            indexed.into_iter().map(scan_one).collect()
        } else {
            let chunk_size = indexed.len().div_ceil(workers);
            std::thread::scope(|scope| {
                let handles: Vec<_> = indexed
                    .chunks(chunk_size)
                    .map(|chunk| {
                        scope.spawn(move || chunk.iter().copied().map(scan_one).collect::<Vec<_>>())
                    })
                    .collect();
                let mut out = Vec::with_capacity(indexed.len());
                for handle in handles {
                    out.extend(handle.join().expect("assets scan worker panicked"));
                }
                out
            })
        };
        let mut assets = Self {
            root: root.to_path_buf(),
            ..Self::default()
        };
        for batch in per_source {
            for resource in batch? {
                assets.add(resource);
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

    /// The folder `resource` is in as the original sees it: its directory
    /// under the install, and for an archive member the directory its name
    /// holds, added to the archive's own directory (an archive's members
    /// are mounted where the archive is). In lowercase, with `/` between
    /// the parts: `gamedata/skins` for `GameData/Skins/b.cmx`,
    /// `deluxe/skins` for `Deluxe/Deluxe.far`'s `Skins\b.cmx.bcf`.
    pub fn folder(&self, resource: &Resource) -> String {
        let directory = resource
            .path
            .parent()
            .and_then(|p| p.strip_prefix(&self.root).ok())
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        let inner = resource
            .member
            .as_ref()
            .and_then(|m| m.name.rsplit_once(['\\', '/']))
            .map(|(dir, _)| dir.replace('\\', "/"));
        let folder = match inner {
            Some(inner) if directory.is_empty() => inner,
            Some(inner) => format!("{directory}/{inner}"),
            None => directory,
        };
        folder.to_ascii_lowercase()
    }

    /// The appearance index, built by `build` the first time it is asked
    /// for (sims-people's `load_appearance` keeps it here).
    pub fn appearances(
        &self,
        build: impl FnOnce() -> BTreeMap<String, (String, Resource)>,
    ) -> &BTreeMap<String, (String, Resource)> {
        self.appearances.get_or_init(build)
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
        let entries = crate::index::sorted_entries(&path).ok()?;
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
    let paths = crate::index::sorted_entries(directory).map_err(|e| e.to_string())?;
    for path in paths {
        if path.is_dir() {
            list(&path, out)?;
        } else {
            out.push(path);
        }
    }
    Ok(())
}

/// An archive's members, read by this crate's reader: the directory is
/// at most 64 MiB ([`sims_formats::far::MAX_DIRECTORY`]) and is bounded
/// before it is read. A member that reader leaves out (compressed, or with
/// a name too long or too deep) is not found by name here. Each member is
/// read later through [`Resource::read`], which caps it at 128 MiB.
fn far_members(path: &Path) -> Result<Vec<Member>, String> {
    let (members, _skipped) = crate::far_members(path).map_err(|e| e.to_string())?;
    Ok(members)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_drops_the_member_directory_and_case() {
        assert_eq!(key("Skins\\B009FCChd_01.cmx.bcf"), "b009fcchd_01.cmx.bcf");
        assert_eq!(key("adult-skeleton.cmx.bcf"), "adult-skeleton.cmx.bcf");
        assert_eq!(key("a/B.BMP"), "b.bmp");
    }

    #[test]
    fn a_file_is_in_its_folder_and_a_member_in_its_archives() {
        let assets = Assets {
            root: PathBuf::from("/install"),
            ..Assets::default()
        };
        let resource = |path: &str, member: Option<&str>| Resource {
            mount: 0,
            path: PathBuf::from(path),
            member: member.map(|name| Member {
                name: name.into(),
                offset: 0,
                size: 0,
            }),
            inline: None,
        };
        let folder = |path, member| assets.folder(&resource(path, member));
        assert_eq!(
            folder("/install/GameData/Skins/B.cmx", None),
            "gamedata/skins"
        );
        assert_eq!(
            folder("/install/Deluxe/Deluxe.far", Some("Skins\\b.cmx.bcf")),
            "deluxe/skins"
        );
        assert_eq!(
            folder(
                "/install/GameData/Textures/Textures.far",
                Some("bottle.bmp")
            ),
            "gamedata/textures"
        );
        assert_eq!(folder("/install/x.far", Some("A/B\\c.bmp")), "a/b");
        // A file outside the install has no folder but its own.
        assert_eq!(folder("/elsewhere/x.far", Some("Skins\\c.bmp")), "skins");
    }

    /// A FAR 1a archive with the directory right after the header.
    /// Each member is `(name, stored bytes, unpacked size)`.
    fn far(members: &[(&str, &[u8], u32)]) -> Vec<u8> {
        let payload: usize = members.iter().map(|(_, bytes, _)| bytes.len()).sum();
        let mut out = b"FAR!byAZ".to_vec();
        out.extend(1u32.to_le_bytes());
        out.extend(((16 + payload) as u32).to_le_bytes());
        let mut offset = 16u32;
        let mut directory = (members.len() as u32).to_le_bytes().to_vec();
        for (name, bytes, unpacked) in members {
            out.extend_from_slice(bytes);
            directory.extend(unpacked.to_le_bytes());
            directory.extend((bytes.len() as u32).to_le_bytes());
            directory.extend(offset.to_le_bytes());
            directory.extend((name.len() as u32).to_le_bytes());
            directory.extend(name.as_bytes());
            offset += bytes.len() as u32;
        }
        out.extend(directory);
        out
    }

    /// A file in the system's temporary directory, removed when dropped.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            Self(
                std::env::temp_dir().join(format!("sims-people-{name}-{}.far", std::process::id())),
            )
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn a_far_directory_over_64_mib_is_refused_before_it_is_read() {
        let file = Scratch::new("sparse");
        std::fs::write(&file.0, far(&[])).unwrap();
        // Sparse: the directory claims the rest of a file over 64 MiB,
        // which the old reader would have read whole.
        std::fs::File::options()
            .write(true)
            .open(&file.0)
            .unwrap()
            .set_len(16 + 64 * 1024 * 1024 + 1)
            .unwrap();
        let error = far_members(&file.0).unwrap_err();
        assert!(error.contains("64 MiB"), "{error}");
    }

    #[test]
    fn a_compressed_far_member_is_left_out() {
        let file = Scratch::new("compressed");
        std::fs::write(
            &file.0,
            far(&[("a.bmp", b"plain", 5), ("Skins\\b.bmp", b"packed", 60)]),
        )
        .unwrap();
        let members = far_members(&file.0).unwrap();
        let names: Vec<&str> = members.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["a.bmp"]);
        assert_eq!((members[0].offset, members[0].size), (16, 5));
    }
}
