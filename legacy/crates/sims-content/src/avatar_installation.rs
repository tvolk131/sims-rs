//! Read-only virtual filesystem for the recovered asset manager. Bindings are
//! explicit host inputs, not inferred installer execution. The host supplies
//! dot entries followed by Windows-1252 names sorted by ASCII-insensitive key;
//! native scanning, mounting and hash-map iteration then determine precedence.
use crate::{
    Index,
    avatar_catalog::{Catalog, Directories},
    avatar_mount::Sources,
};
use anyhow::{Context, Result, ensure};
use sims_sim::pose::clothing::{
    catalog_mount::{Found, Manager},
    catalog_paths::{self, Host, Kind, Paths},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
mod objects;
pub use objects::{ObjectVisit, ObjectVisitor};

/// Physical directory search within this closed virtual installation. Native
/// ResolveResFile uses `directory/*.*` only to establish directory availability;
/// mounted archive members do not fabricate physical companion directories.
#[derive(Debug, Clone)]
pub struct DirectorySearch {
    directories: BTreeSet<Vec<u8>>,
}
impl sims_sim::object_catalog::directory::Host for DirectorySearch {
    type Search = ();
    fn find_first(&mut self, pattern: &[u8]) -> Result<Option<()>, String> {
        let directory = pattern
            .strip_suffix(b"*.*")
            .ok_or("unsupported directory availability pattern")?;
        if directory.iter().any(|b| matches!(b, b'*' | b'?')) {
            return Err("wildcards inside a directory path are unsupported".into());
        }
        let key = physical_key(directory).map_err(|e| e.to_string())?;
        Ok(self.directories.contains(&key).then_some(()))
    }
    fn find_close(&mut self, _: ()) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct Binding {
    pub directory: Vec<u8>,
    pub physical: PathBuf,
}

/// Explicit adapter for this project's Complete Collection cabinet extraction.
/// These are asset roots, not a reconstructed installation of every cabinet
/// group (for example Creator and installer support files are excluded).
/// This recipe claims path bindings, not that the installer was executed.
pub fn complete_collection_asset_bindings(root: &Path) -> Vec<Binding> {
    [
        ("GameData_ALL_Recursive", "GameData"),
        ("Downloads", "Downloads"),
        ("ExpansionPack1", "ExpansionPack"),
        ("ExpansionPack2", "ExpansionPack2"),
        ("ExpansionPack3", "ExpansionPack3"),
        ("ExpansionPack4", "ExpansionPack4"),
        ("ExpansionPackGOLD", "Deluxe"),
        ("ExpansionPack5", "ExpansionPack5"),
        ("ExpansionPack6", "ExpansionPack6"),
        ("ExpansionShared", "ExpansionShared"),
        ("TemplateNPCs", "TemplateNPCs"),
        ("ExpansionPack7", "ExpansionPack7"),
    ]
    .into_iter()
    .map(|(physical, directory)| Binding {
        directory: format!("./{directory}/").into_bytes(),
        physical: root.join(physical),
    })
    .collect()
}
#[derive(Debug)]
struct Node {
    physical: PathBuf,
    // None is a file; Some(empty) is a real empty directory.
    entries: Option<Vec<Found>>,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Namespace(BTreeMap<Vec<u8>, NamespaceNode>);

#[derive(Clone, PartialEq, Eq)]
struct NamespaceNode {
    physical: PathBuf,
    entries: Option<Vec<(Vec<u8>, u32)>>,
}

pub struct Installation<'a> {
    inputs: Option<crate::startup_inputs::Session>,
    index: &'a Index,
    pub bindings: Vec<Binding>,
    tree: BTreeMap<Vec<u8>, Node>,
    loose: BTreeMap<PathBuf, usize>,
    sources: Sources<'a>,
    prepared: BTreeSet<Vec<u8>>,
    pub manager: Manager,
}
impl<'a> Installation<'a> {
    /// Snapshot the supplied namespace. Unknown paths are known absent inside
    /// this closed virtual installation, never looked up in the process cwd.
    pub fn new(index: &'a Index, bindings: Vec<Binding>) -> Result<Self> {
        let mut tree = BTreeMap::new();
        for binding in &bindings {
            let key = physical_key(&binding.directory)?;
            ensure!(
                key != b".\\",
                "bind named installation directories, not the host root"
            );
            let owned = index
                .resources
                .iter()
                .filter(|r| r.owned.is_some())
                .any(|r| {
                    r.locator.split('/').next().is_some_and(|directory| {
                        binding.physical.starts_with(index.root.join(directory))
                    })
                });
            if owned {
                snapshot_owned(index, &binding.physical, &key, &mut tree)?;
            } else {
                ensure!(
                    binding.physical.is_dir(),
                    "binding directory absent: {}",
                    binding.physical.display()
                );
                snapshot(&binding.physical, &key, &mut tree)?;
            }
        }
        let loose = index
            .resources
            .iter()
            .enumerate()
            .filter(|(_, r)| !r.archived)
            .map(|(i, r)| (r.source.clone(), i))
            .collect();
        Ok(Self {
            inputs: None,
            index,
            bindings,
            tree,
            loose,
            sources: Sources::new(index),
            prepared: BTreeSet::new(),
            manager: Manager::default(),
        })
    }

    pub(crate) fn namespace(&self) -> Namespace {
        Namespace(
            self.tree
                .iter()
                .map(|(key, node)| {
                    (
                        key.clone(),
                        NamespaceNode {
                            physical: node.physical.clone(),
                            entries: node.entries.as_ref().map(|entries| {
                                entries
                                    .iter()
                                    .map(|entry| (entry.name.clone(), entry.attributes))
                                    .collect()
                            }),
                        },
                    )
                })
                .collect(),
        )
    }

    pub(crate) fn observe(&mut self, inputs: &crate::startup_inputs::Session) {
        self.inputs = Some(inputs.clone());
        self.sources.observe(inputs.clone());
    }
    pub fn physical_exists(&self, path: &[u8]) -> Result<bool> {
        let path = path.split(|&b| b == 0).next().unwrap();
        if path.is_empty() {
            return Ok(false);
        }
        let key = physical_key(path)?;
        Ok(key == b".\\"
            || self.tree.get(&key).is_some_and(|node| {
                !matches!(path.last(), Some(b'/' | b'\\')) || node.entries.is_some()
            }))
    }
    pub fn exists(&self, path: &[u8]) -> Result<bool> {
        self.manager
            .exists(path, |p| self.physical_exists(p).map_err(|e| e.to_string()))
            .map_err(anyhow::Error::msg)
    }
    pub fn registered(&self, path: &[u8]) -> Result<bool> {
        self.manager.registered(path).map_err(anyhow::Error::msg)
    }
    /// This is an explicit engine registration operation. Construction alone
    /// does not mount all archives or reorder registrations behind the caller.
    pub fn register(&mut self, path: &[u8], recursive: bool, archives: bool) -> Result<()> {
        self.prepare(path)?;
        self.sources
            .scan(&mut self.manager, path, recursive, archives)
    }
    pub fn directories(&self) -> Result<Directories> {
        self.sources.catalog_directories(&self.manager)
    }
    pub fn directory_search(&self) -> DirectorySearch {
        let mut directories: BTreeSet<_> = self
            .tree
            .iter()
            .filter(|(_, node)| node.entries.is_some())
            .map(|(key, _)| key.clone())
            .collect();
        directories.insert(b".\\".to_vec());
        DirectorySearch { directories }
    }
    pub fn open_failures(&self) -> &[Vec<u8>] {
        &self.sources.open_failures
    }

    /// Native initial registration order. This mounts sources only; it does not
    /// fabricate the subsequent ordered object catalog or its provider vector.
    pub fn initial_mounts(
        &mut self,
        prefix: &[u8],
        expansions: i16,
        split_fars: bool,
    ) -> Result<Vec<Vec<u8>>> {
        let mut host = CatalogHost {
            installation: self,
            providers: &[],
            catalog: Catalog::default(),
            flags: 0,
            visits: [0; 2],
        };
        catalog_paths::initial_mounts(prefix, expansions, split_fars, &mut host)
            .map_err(anyhow::Error::msg)
    }

    /// The fixed leading texture paths from 413ccc, before its object-provider
    /// suffix. A hit here can terminate 5c1c65's plain-file pass; a miss does not
    /// establish absence from the full native search or its named fallback.
    pub fn texture_prefix(&mut self, prefix: &[u8], expansions: i16) -> Result<Vec<Vec<u8>>> {
        let mut host = CatalogHost {
            installation: self,
            providers: &[],
            catalog: Catalog::default(),
            flags: 0,
            visits: [0; 2],
        };
        catalog_paths::texture_paths(prefix, expansions, &[], &mut host).map_err(anyhow::Error::msg)
    }

    fn prepare(&mut self, directory: &[u8]) -> Result<()> {
        let directory = sims_sim::pose::clothing::name_map::Files::<()>::key(directory)
            .map_err(anyhow::Error::msg)?;
        ensure!(
            directory.ends_with(b"\\"),
            "registration needs a terminated directory path"
        );
        if self.prepared.contains(&directory) {
            return Ok(());
        }
        let key = physical_key(&directory)?;
        let entries = self.tree.get(&key).and_then(|n| n.entries.clone());
        let Some(entries) = entries else {
            self.sources.set_directory(&directory, vec![])?;
            self.prepared.insert(directory);
            return Ok(());
        };
        let mut enumeration = vec![
            Found {
                name: b".".to_vec(),
                attributes: 16,
            },
            Found {
                name: b"..".to_vec(),
                attributes: 16,
            },
        ];
        enumeration.extend(entries.clone());
        self.sources.set_directory(&directory, enumeration)?;
        for entry in entries {
            let full = [directory.as_slice(), entry.name.as_slice()].concat();
            if entry.attributes & 16 != 0 {
                self.prepare(&[full.as_slice(), b"\\"].concat())?;
            } else {
                let physical = &self.tree[&physical_key(&full)?].physical;
                if entry.name.to_ascii_lowercase().ends_with(b"far") {
                    self.sources.bind_archive(&full, physical)?;
                } else {
                    let id = *self.loose.get(physical).with_context(|| {
                        format!("virtual file is not indexed: {}", physical.display())
                    })?;
                    self.sources.bind_loose(&full, id)?;
                }
            }
        }
        self.prepared.insert(directory);
        Ok(())
    }

    /// Execute the two verified path phases using explicit provider state and
    /// the manager's existing mounts. Initial object scans remain the caller's
    /// responsibility; missing provider state must not be replaced with defaults.
    pub fn load_catalogs(
        &mut self,
        prefix: &[u8],
        root: &[u8],
        expansions: i16,
        providers: &[Vec<u8>],
    ) -> Result<Loaded> {
        let mut host = CatalogHost {
            installation: self,
            providers,
            catalog: Catalog::default(),
            flags: 0,
            visits: [0; 2],
        };
        let textures = catalog_paths::texture_paths(prefix, expansions, providers, &mut host)
            .map_err(anyhow::Error::msg)?;
        let mut paths = None;
        Paths::load_once(&mut paths, root, expansions, &mut host).map_err(anyhow::Error::msg)?;
        Ok(Loaded {
            catalog: host.catalog,
            paths: paths.unwrap(),
            textures,
            visits: host.visits,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Loaded {
    pub catalog: Catalog,
    pub paths: Paths,
    pub textures: Vec<Vec<u8>>,
    pub visits: [usize; 2],
}
struct CatalogHost<'a, 'b> {
    installation: &'a mut Installation<'b>,
    providers: &'a [Vec<u8>],
    catalog: Catalog,
    flags: u32,
    visits: [usize; 2],
}
impl Host for CatalogHost<'_, '_> {
    fn exists(&mut self, path: &[u8]) -> Result<bool, String> {
        self.installation.exists(path).map_err(|e| e.to_string())
    }
    fn providers(&mut self) -> Result<Vec<Vec<u8>>, String> {
        Ok(self.providers.to_vec())
    }
    fn registered(&mut self, path: &[u8]) -> Result<bool, String> {
        self.installation
            .registered(path)
            .map_err(|e| e.to_string())
    }
    fn register(&mut self, path: &[u8], recursive: bool, archives: bool) -> Result<(), String> {
        self.installation
            .register(path, recursive, archives)
            .map_err(|e| e.to_string())
    }
    fn load(&mut self, kind: Kind, paths: &[Vec<u8>]) -> Result<(), String> {
        let indexed = kind == Kind::Animations;
        self.visits[usize::from(indexed)] = self
            .catalog
            .load_startup_paths_observed(
                self.installation.index,
                &mut self.installation.directories().map_err(|e| e.to_string())?,
                paths,
                indexed,
                if indexed { 2 } else { 1 },
                &mut self.flags,
                self.installation.inputs.as_ref(),
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// Managed-save directory enumeration uses the saved closed namespace. Do
/// not inspect the original Characters directory, even when it still exists.
fn snapshot_owned(
    index: &Index,
    physical: &Path,
    key: &[u8],
    tree: &mut BTreeMap<Vec<u8>, Node>,
) -> Result<()> {
    ensure!(
        !tree.contains_key(key),
        "overlapping owned installation binding"
    );
    let mut children = BTreeMap::<Vec<u8>, (Vec<u8>, PathBuf, bool)>::new();
    for r in index.resources.iter().filter(|r| r.owned.is_some()) {
        let Ok(relative) = r.source.strip_prefix(physical) else {
            continue;
        };
        let mut parts = relative.components();
        let Some(first) = parts.next() else {
            continue;
        };
        let name = first
            .as_os_str()
            .to_str()
            .context("owned filename is not Unicode")?;
        let (bytes, _, invalid) = encoding_rs::WINDOWS_1252.encode(name);
        ensure!(
            !invalid && !bytes.iter().any(|b| matches!(b, b'/' | b'\\' | b':' | 0)),
            "invalid owned filename"
        );
        let directory = parts.next().is_some();
        let value = (bytes.to_vec(), physical.join(name), directory);
        if let Some(old) = children.insert(bytes.to_ascii_lowercase(), value.clone()) {
            ensure!(old == value, "case-colliding owned installation entry");
        }
    }
    let mut entries = vec![];
    for (name, path, directory) in children.into_values() {
        let child_key = [key, b"\\", name.to_ascii_lowercase().as_slice()].concat();
        if directory {
            snapshot_owned(index, &path, &child_key, tree)?;
        } else {
            ensure!(
                tree.insert(
                    child_key,
                    Node {
                        physical: path,
                        entries: None
                    }
                )
                .is_none(),
                "duplicate owned installation file"
            );
        }
        entries.push(Found {
            name,
            attributes: if directory { 16 } else { 0 },
        });
    }
    tree.insert(
        key.to_vec(),
        Node {
            physical: physical.into(),
            entries: Some(entries),
        },
    );
    Ok(())
}

fn snapshot(physical: &Path, key: &[u8], tree: &mut BTreeMap<Vec<u8>, Node>) -> Result<()> {
    ensure!(
        !tree.contains_key(key),
        "overlapping or case-colliding virtual binding: {}",
        String::from_utf8_lossy(key)
    );
    let metadata = std::fs::symlink_metadata(physical)?;
    ensure!(
        !metadata.file_type().is_symlink(),
        "symlink is not an installation binding: {}",
        physical.display()
    );
    let mut entries = None;
    if metadata.is_dir() {
        let mut children = BTreeMap::new();
        for child in std::fs::read_dir(physical)? {
            let child = child?;
            let filename = child.file_name();
            let name = filename
                .to_str()
                .context("installation name is not Unicode")?;
            let (bytes, _, invalid) = encoding_rs::WINDOWS_1252.encode(name);
            ensure!(!invalid, "installation name is not Windows-1252: {name}");
            ensure!(
                !bytes.iter().any(|b| matches!(b, b'/' | b'\\' | b':' | 0)),
                "invalid virtual filename"
            );
            let sort = bytes.to_ascii_lowercase();
            ensure!(
                !children.contains_key(&sort),
                "case-colliding installation names: {name}"
            );
            children.insert(sort, (bytes.into_owned(), child.path()));
        }
        let mut found = vec![];
        for (name, child) in children.into_values() {
            let metadata = std::fs::symlink_metadata(&child)?;
            let child_key = [key, b"\\", &name.to_ascii_lowercase()].concat();
            snapshot(&child, &child_key, tree)?;
            found.push(Found {
                name,
                attributes: u32::from(metadata.permissions().readonly())
                    | if metadata.is_dir() { 16 } else { 0 },
            });
        }
        entries = Some(found);
    } else {
        ensure!(metadata.is_file(), "non-file installation entry");
    }
    tree.insert(
        key.to_vec(),
        Node {
            physical: physical.into(),
            entries,
        },
    );
    Ok(())
}

// OS-side normalization only. Never use this to change the engine's mounted
// directory keys: 5b9c9f treats those as exact normalized strings.
fn physical_key(path: &[u8]) -> Result<Vec<u8>> {
    let path = path.split(|&b| b == 0).next().unwrap();
    ensure!(
        !path.starts_with(b"/") && !path.starts_with(b"\\") && !path.contains(&b':'),
        "virtual path must be installation-relative"
    );
    let mut key = b".".to_vec();
    for part in path.split(|b| matches!(b, b'/' | b'\\')) {
        if part.is_empty() || part == b"." {
            continue;
        }
        ensure!(part != b"..", "virtual path escapes its binding");
        key.push(b'\\');
        key.extend(part.iter().map(u8::to_ascii_lowercase));
    }
    if key.len() == 1 {
        key.push(b'\\');
    }
    Ok(key)
}
