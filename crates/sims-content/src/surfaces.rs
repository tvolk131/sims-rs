//! The floor and wall pattern catalogs the original builds at start-up
//! (floors 0x4a01cb, walls 0x4c512d), as far as a house load reads them.
//!
//! A house names the patterns it uses by file and number in the file. The
//! load turns those into the catalog's keys (0x4a0e3b, 0x4c5d35; see
//! `sims_sim`'s `renumber_surfaces`), which is what the lot's floor and wall
//! cells hold. A catalog is the list of pattern files in the order the
//! original found them, with the key it gave each pattern.
//!
//! File 0 is the base game's built-in file (`GameData/floors.iff`,
//! `GameData/walls.iff`), whose patterns have fixed keys below 256. A load
//! never looks a pattern up in it by number, so its patterns are not listed
//! here. The other files come from these directories, in this order, each
//! listed in the order of the original's file table and filtered by suffix
//! (`.FLR` at 0x6c67c8, `.WLL` at 0x6c7740; 0x41286a):
//!
//! 1. `WallsAndFloors/` of each installed pack, in edition-bit order (path
//!    IDs 0x1c, 0x1f, 0x25, 0x2b, 0x31, 0x37, 0x3d and 0x5b of 0x415336);
//! 2. `ExpansionShared/WallsAndFloors/` when the edition is above 3 (0x43);
//! 3. `GameData/Floors/` or `GameData/Walls/` (0x12 or 0x13).
//!
//! For each file (0x4a0744, 0x4c552e), a third of its sprites (`SPR2` for
//! floors, `SPR#` for walls; one sprite per zoom) is its pattern count. A
//! file with none is left out. Each pattern number from 1 takes the next key
//! from the catalog's free list, and is listed when the file has the sprite
//! with that ID; a key taken for a missing sprite is lost.
//!
//! Checked against the original running on the Complete Collection: the
//! number of files, the file at each place where the guest's memory still
//! held its name (150 of 208 floor files and 186 of 272 wall files), and
//! every key. The file system's order of a directory on disk
//! (`GameData/Floors`, `GameData/Walls`) only decides which of two names
//! sharing a bucket of the file table comes first, and no two entries of
//! either share one in this install. The colour
//! each pattern is matched by when a house's file is missing (+0x2c, from
//! 0x49fd95) is not ported.

use crate::index::Index;
use crate::install::resolve;
use crate::pool::{ArchivePool, worker_count};
use crate::ui_graphics::US_ENGLISH;
use crate::{Error, Install, Resource, Result};
use sims_formats::iff::Iff;
use sims_formats::strings::StringTable;
use std::collections::BTreeMap;

/// Floors or walls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceKind {
    Floor,
    Wall,
}

/// What differs between the two catalogs.
struct Profile {
    /// The built-in file, file 0.
    builtin: &'static str,
    /// The suffix the scan keeps, in key form.
    suffix: &'static [u8],
    /// The base game's own directory.
    own: &'static str,
    /// The sprite chunk counted.
    sprite: [u8; 4],
    /// The free key ranges, in the order they are seeded (0x49ff66,
    /// 0x4c4c93).
    seeds: [(u16, u16); 2],
}

impl SurfaceKind {
    fn profile(self) -> Profile {
        match self {
            Self::Floor => Profile {
                builtin: "floors.iff",
                suffix: b".flr",
                own: "GameData/Floors/",
                sprite: *b"SPR2",
                seeds: [(30, 254), (256, 65534)],
            },
            Self::Wall => Profile {
                builtin: "walls.iff",
                suffix: b".wll",
                own: "GameData/Walls/",
                sprite: *b"SPR#",
                seeds: [(31, 247), (256, 65534)],
            },
        }
    }
}

/// One pattern file of a catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceFile {
    /// The file's name, without its directory, in the file table's key
    /// form (lower case). A house's map names files this way.
    pub name: String,
    /// Where the file was found, for messages.
    pub location: String,
    /// Each pattern's key by its number in the file.
    pub keys: BTreeMap<u16, u16>,
    /// The price each of the file's patterns is given (0x4a0a84,
    /// 0x4c59a0): string 1, from 0, of the file's `STR#` with ID 0
    /// ([`StringTable::parse_chunk`]), read as C's `atoi` reads its bytes, or None when the file lacks that
    /// table or string (the pattern then costs the catalog's default).
    pub price: Option<i32>,
}

/// One plug-in floor (`.flr`) or wallpaper (`.wll`) pattern indexed by its
/// catalog key, carrying its [`Resource`] and localized `STR# 0` metadata for
/// rendering and the Build Mode catalog (`0x4a0cdc`, `0x4c5bbf`).
#[derive(Debug, Clone)]
pub struct SurfacePlugin {
    /// Catalog key assigned during the original's scan (`30..=254` / `31..=247`
    /// or `256..=65534`).
    pub key: u16,
    /// 1-based pattern number inside the `.flr` / `.wll` file.
    pub local: u16,
    /// Lower-case file name without directory.
    pub file_name: String,
    /// Underlying loose file or FAR archive member.
    pub resource: Resource,
    /// Pattern display name (string 0 of `STR# 0`, US English), if present.
    pub name: Option<String>,
    /// Raw Windows-1252 name bytes (for native `0x4cd9a5` sort comparison).
    pub raw_name: Option<Vec<u8>>,
    /// Pattern price (string 1 of `STR# 0`, parsed as C `atoi`), if present.
    pub price: Option<i32>,
    /// Pattern description (string 2 of `STR# 0`, US English), if present.
    pub description: Option<String>,
}

/// One kind's catalog: its pattern files by index.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SurfaceCatalog {
    pub files: Vec<SurfaceFile>,
}

struct ParsedSurface {
    name: String,
    location: String,
    count: i16,
    present: Vec<u16>,
    meta: FileMetadata,
}

fn parse_surface(
    resource: &Resource,
    name: &[u8],
    sprite: [u8; 4],
    root: &std::path::Path,
    pool: &ArchivePool,
    collect_plugins: bool,
) -> Result<Option<ParsedSurface>> {
    let location = resource.location(root);
    // A file the original cannot open is left out too. Which files our
    // reader refuses that the original opens, or the reverse, is not
    // known, so a refusal stops the build.
    let iff = Iff::parse(resource.read_with(pool)?).map_err(|e| Error::format(&location, e))?;
    let count = iff.all(&sprite).count() as i16 / 3;
    if count < 1 {
        return Ok(None);
    }
    let present = (1..=count as u16)
        .filter(|&number| iff.get(&sprite, number).is_some())
        .collect();
    let meta = if collect_plugins {
        metadata(&iff)
    } else {
        FileMetadata {
            price: metadata_price(&iff),
            ..FileMetadata::default()
        }
    };
    Ok(Some(ParsedSurface {
        name: sims_formats::far::decode_name(name),
        location,
        count,
        present,
        meta,
    }))
}

type SurfaceDirectoryListing = (Vec<(Vec<u8>, Resource)>, Vec<String>);

fn scan_surface_directory(
    install: &Install,
    directory: &str,
    at: &'static str,
) -> Result<SurfaceDirectoryListing> {
    match resolve(&install.root, directory)?.filter(|p| p.is_dir()) {
        Some(path) => Index::listing(&path, at.as_bytes()),
        None => Ok((Vec::new(), Vec::new())),
    }
}

fn scan_surface_directories(
    install: &Install,
    directories: &[(String, &'static str)],
) -> Vec<Result<SurfaceDirectoryListing>> {
    let scan_one =
        |(directory, at): &(String, &'static str)| scan_surface_directory(install, directory, at);
    let workers = worker_count(directories.len());
    if workers <= 1 {
        directories.iter().map(scan_one).collect()
    } else {
        let chunk_size = directories.len().div_ceil(workers);
        std::thread::scope(|scope| {
            let handles: Vec<_> = directories
                .chunks(chunk_size)
                .map(|chunk| scope.spawn(move || chunk.iter().map(scan_one).collect::<Vec<_>>()))
                .collect();
            let mut out = Vec::with_capacity(directories.len());
            for handle in handles {
                out.extend(handle.join().expect("surface directory worker panicked"));
            }
            out
        })
    }
}

impl SurfaceCatalog {
    /// Builds the catalog of `kind` for an install.
    pub fn build(install: &Install, kind: SurfaceKind) -> Result<Self> {
        Self::build_with_pool(install, kind, &ArchivePool::new())
    }

    /// Scans the plug-in floor (`.flr`) or wall (`.wll`) patterns of `kind`
    /// for `install`, keyed by their exact catalog keys (`0x4a01cb`,
    /// `0x4c512d`).
    pub fn plugins(install: &Install, kind: SurfaceKind) -> Result<BTreeMap<u16, SurfacePlugin>> {
        Self::plugins_with_pool(install, kind, &ArchivePool::new())
    }

    /// Like [`Self::plugins`], reusing open FAR archive handles from `pool`.
    pub fn plugins_with_pool(
        install: &Install,
        kind: SurfaceKind,
        pool: &ArchivePool,
    ) -> Result<BTreeMap<u16, SurfacePlugin>> {
        let (_, plugins) = Self::scan_with_pool(install, kind, true, pool)?;
        Ok(plugins)
    }

    pub(crate) fn build_with_pool(
        install: &Install,
        kind: SurfaceKind,
        pool: &ArchivePool,
    ) -> Result<Self> {
        let (catalog, _) = Self::scan_with_pool(install, kind, false, pool)?;
        Ok(catalog)
    }

    fn scan_with_pool(
        install: &Install,
        kind: SurfaceKind,
        collect_plugins: bool,
        pool: &ArchivePool,
    ) -> Result<(Self, BTreeMap<u16, SurfacePlugin>)> {
        let Profile { own, .. } = kind.profile();
        let mut directories = shared_surface_directories(install);
        directories.push((own.to_owned(), ""));
        let listings = scan_surface_directories(install, &directories);
        let mut ok_dirs = Vec::with_capacity(listings.len());
        let mut trailing_err = None;
        for listing in listings {
            match listing {
                Ok(dir) => ok_dirs.push(dir),
                Err(error) => {
                    trailing_err = Some(error);
                    break;
                }
            }
        }
        let refs: Vec<&SurfaceDirectoryListing> = ok_dirs.iter().collect();
        Self::from_listings(install, kind, &refs, trailing_err, collect_plugins, pool)
    }

    fn from_listings(
        install: &Install,
        kind: SurfaceKind,
        listings: &[&SurfaceDirectoryListing],
        trailing_error: Option<Error>,
        collect_plugins: bool,
        pool: &ArchivePool,
    ) -> Result<(Self, BTreeMap<u16, SurfacePlugin>)> {
        let Profile {
            builtin,
            suffix,
            sprite,
            seeds,
            ..
        } = kind.profile();
        // Collect matching files only up to the first directory with a skipped
        // archive so later directories never mask an earlier directory's parse
        // or scan error, while still parsing all reachable files in parallel.
        let mut matching: Vec<(usize, &[u8], &Resource)> = Vec::new();
        for (dir_idx, (listed, skipped)) in listings.iter().enumerate() {
            if !skipped.is_empty() {
                break;
            }
            for (name, resource) in listed {
                if name.ends_with(suffix) {
                    matching.push((dir_idx, name.as_slice(), resource));
                }
            }
        }
        let workers = worker_count(matching.len());
        let parsed: Vec<(usize, &Resource, Result<Option<ParsedSurface>>)> = if workers <= 1 {
            matching
                .iter()
                .map(|&(dir_idx, name, resource)| {
                    (
                        dir_idx,
                        resource,
                        parse_surface(resource, name, sprite, &install.root, pool, collect_plugins),
                    )
                })
                .collect()
        } else {
            let chunk_size = matching.len().div_ceil(workers);
            std::thread::scope(|scope| {
                let handles: Vec<_> = matching
                    .chunks(chunk_size)
                    .map(|chunk| {
                        scope.spawn(move || {
                            chunk
                                .iter()
                                .map(|&(dir_idx, name, resource)| {
                                    (
                                        dir_idx,
                                        resource,
                                        parse_surface(
                                            resource,
                                            name,
                                            sprite,
                                            &install.root,
                                            pool,
                                            collect_plugins,
                                        ),
                                    )
                                })
                                .collect::<Vec<_>>()
                        })
                    })
                    .collect();
                let mut out = Vec::with_capacity(matching.len());
                for handle in handles {
                    out.extend(handle.join().expect("surface parse worker panicked"));
                }
                out
            })
        };
        let mut parsed_iter = parsed.into_iter().peekable();
        let mut files = vec![SurfaceFile {
            name: builtin.to_owned(),
            location: format!("GameData/{builtin}"),
            keys: BTreeMap::new(),
            price: None,
        }];
        let mut plugins = BTreeMap::new();
        // The free list: 0x58055b puts each range at the front, and a key
        // is taken from the front range (0x4a1a08).
        let mut free: Vec<(u16, u16)> = seeds.iter().rev().copied().collect();
        for (dir_idx, (_, skipped)) in listings.iter().enumerate() {
            if let Some(reason) = skipped.first() {
                return Err(Error::Unsupported(reason.clone()));
            }
            while let Some((item_dir, _, _)) = parsed_iter.peek() {
                if *item_dir != dir_idx {
                    break;
                }
                let (_, resource, item) = parsed_iter.next().expect("peeked above");
                let Some(surface) = item? else {
                    continue;
                };
                let mut keys = BTreeMap::new();
                let mut present_idx = 0usize;
                for number in 1..=surface.count as u16 {
                    let Some((low, high)) = free.first().copied() else {
                        break;
                    };
                    free.remove(0);
                    if low < high {
                        free.insert(0, (low + 1, high));
                    }
                    if surface.present.get(present_idx) == Some(&number) {
                        keys.insert(number, low);
                        present_idx += 1;
                    }
                }
                if collect_plugins {
                    for (&local, &key) in &keys {
                        plugins.insert(
                            key,
                            SurfacePlugin {
                                key,
                                local,
                                file_name: surface.name.clone(),
                                resource: resource.clone(),
                                name: surface.meta.name.clone(),
                                raw_name: surface.meta.raw_name.clone(),
                                price: surface.meta.price,
                                description: surface.meta.description.clone(),
                            },
                        );
                    }
                }
                files.push(SurfaceFile {
                    name: surface.name,
                    location: surface.location,
                    keys,
                    price: surface.meta.price,
                });
            }
        }
        if let Some(error) = trailing_error {
            return Err(error);
        }
        Ok((Self { files }, plugins))
    }
    /// The first file whose name is `name`, ignoring ASCII case, as the
    /// load matches a house's map entry (0x4a1d25, comparing at 0x46f891).
    pub fn file(&self, name: &str) -> Option<usize> {
        self.files
            .iter()
            .position(|file| file.name.as_bytes().eq_ignore_ascii_case(name.as_bytes()))
    }

    /// The key of pattern `number` of file `file` (0x4a0c82), or None.
    pub fn key(&self, file: usize, number: u16) -> Option<u16> {
        self.files.get(file)?.keys.get(&number).copied()
    }
}

fn shared_surface_directories(install: &Install) -> Vec<(String, &'static str)> {
    let mut directories = Vec::new();
    for (bit, directory, _) in crate::Expansions::ALL {
        if install.expansions.contains(bit) {
            directories.push((directory.to_owned(), "wallsandfloors\\"));
        }
    }
    if install.expansions.0 > 3 {
        directories.push(("ExpansionShared".to_owned(), "wallsandfloors\\"));
    }
    directories
}

/// Both catalogs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Surfaces {
    pub floors: SurfaceCatalog,
    pub walls: SurfaceCatalog,
}

impl Surfaces {
    pub fn build(install: &Install) -> Result<Self> {
        Self::build_with_pool(install, &ArchivePool::new())
    }

    /// Builds both floor and wall catalogs, scanning shared expansion
    /// `WallsAndFloors` directories once and parsing pattern IFFs in parallel
    /// through `pool`.
    pub fn build_with_pool(install: &Install, pool: &ArchivePool) -> Result<Self> {
        let (surfaces, _, _) = Self::build_all_with_pool(install, false, pool)?;
        Ok(surfaces)
    }

    /// Builds both floor and wall catalogs together with their plug-in metadata
    /// maps in one shared parallel scan through `pool`.
    #[allow(clippy::type_complexity)]
    pub fn build_with_plugins_and_pool(
        install: &Install,
        pool: &ArchivePool,
    ) -> Result<(
        Self,
        BTreeMap<u16, SurfacePlugin>,
        BTreeMap<u16, SurfacePlugin>,
    )> {
        Self::build_all_with_pool(install, true, pool)
    }

    /// Scans plug-in floor and wall metadata maps in one shared parallel scan
    /// through `pool`, returning an empty map for any kind whose scan fails
    /// (matching [`SurfaceCatalog::plugins`] with `.unwrap_or_default()`).
    pub fn plugins_with_pool(
        install: &Install,
        pool: &ArchivePool,
    ) -> (BTreeMap<u16, SurfacePlugin>, BTreeMap<u16, SurfacePlugin>) {
        let (floors, walls) = Self::scan_both_with_pool(install, true, pool, false);
        (
            floors.map(|(_, plugins)| plugins).unwrap_or_default(),
            walls.map(|(_, plugins)| plugins).unwrap_or_default(),
        )
    }

    #[allow(clippy::type_complexity)]
    fn build_all_with_pool(
        install: &Install,
        collect_plugins: bool,
        pool: &ArchivePool,
    ) -> Result<(
        Self,
        BTreeMap<u16, SurfacePlugin>,
        BTreeMap<u16, SurfacePlugin>,
    )> {
        let (floors, walls) = Self::scan_both_with_pool(install, collect_plugins, pool, true);
        let (floors, floor_plugins) = floors?;
        let (walls, wall_plugins) = walls?;
        Ok((Self { floors, walls }, floor_plugins, wall_plugins))
    }

    #[allow(clippy::type_complexity)]
    fn scan_both_with_pool(
        install: &Install,
        collect_plugins: bool,
        pool: &ArchivePool,
        fail_fast_wall_on_floor_err: bool,
    ) -> (
        Result<(SurfaceCatalog, BTreeMap<u16, SurfacePlugin>)>,
        Result<(SurfaceCatalog, BTreeMap<u16, SurfacePlugin>)>,
    ) {
        let shared = shared_surface_directories(install);
        let shared_len = shared.len();
        let mut all_dirs = shared;
        all_dirs.push((SurfaceKind::Floor.profile().own.to_owned(), ""));
        all_dirs.push((SurfaceKind::Wall.profile().own.to_owned(), ""));
        let mut listings = scan_surface_directories(install, &all_dirs);
        let wall_own = listings.pop().expect("wall directory listing");
        let floor_own = listings.pop().expect("floor directory listing");
        debug_assert_eq!(listings.len(), shared_len);

        // Keep every leading `Ok` shared directory and capture the exact first
        // shared `Error` by value without retrying any filesystem scan.
        let mut ok_shared = Vec::with_capacity(shared_len);
        let mut first_shared_err = None;
        for listing in listings {
            match listing {
                Ok(dir) => ok_shared.push(dir),
                Err(error) => {
                    first_shared_err = Some(error);
                    break;
                }
            }
        }
        if let Some(shared_err) = first_shared_err {
            if !fail_fast_wall_on_floor_err {
                // Both independent `SurfaceCatalog::plugins` calls fail on the
                // same shared directory before reading `own`.
                return (
                    Err(Error::Unsupported(shared_err.to_string())),
                    Err(shared_err),
                );
            }
            let floor_refs: Vec<&SurfaceDirectoryListing> = ok_shared.iter().collect();
            let floors = SurfaceCatalog::from_listings(
                install,
                SurfaceKind::Floor,
                &floor_refs,
                Some(shared_err),
                collect_plugins,
                pool,
            );
            return (floors, Ok((SurfaceCatalog::default(), BTreeMap::new())));
        }
        let (floor_own_ok, floor_err) = match floor_own {
            Ok(dir) => (Some(dir), None),
            Err(error) => (None, Some(error)),
        };
        let (wall_own_ok, wall_err) =
            if fail_fast_wall_on_floor_err && floor_own_ok.is_none() && floor_err.is_some() {
                (None, None)
            } else {
                match wall_own {
                    Ok(dir) => (Some(dir), None),
                    Err(error) => (None, Some(error)),
                }
            };
        let mut floor_refs: Vec<&SurfaceDirectoryListing> = ok_shared.iter().collect();
        if let Some(ref own) = floor_own_ok {
            floor_refs.push(own);
        }
        let mut wall_refs: Vec<&SurfaceDirectoryListing> = ok_shared.iter().collect();
        if let Some(ref own) = wall_own_ok {
            wall_refs.push(own);
        }
        std::thread::scope(|scope| {
            let floor_handle = scope.spawn(|| {
                SurfaceCatalog::from_listings(
                    install,
                    SurfaceKind::Floor,
                    &floor_refs,
                    floor_err,
                    collect_plugins,
                    pool,
                )
            });
            let walls = SurfaceCatalog::from_listings(
                install,
                SurfaceKind::Wall,
                &wall_refs,
                wall_err,
                collect_plugins,
                pool,
            );
            let floors = floor_handle.join().expect("floor catalog worker panicked");
            (floors, walls)
        })
    }
}

#[derive(Default)]
struct FileMetadata {
    name: Option<String>,
    raw_name: Option<Vec<u8>>,
    price: Option<i32>,
    description: Option<String>,
}

fn metadata(iff: &Iff) -> FileMetadata {
    let Some(chunk) = iff.get(b"STR#", 0) else {
        return FileMetadata::default();
    };
    let Ok(table) = StringTable::parse_chunk(iff.data(chunk), iff.resource_flags(chunk)) else {
        return FileMetadata::default();
    };
    let name_entry = table.entry(0, US_ENGLISH);
    let name = name_entry.map(|e| e.text.clone()).filter(|s| !s.is_empty());
    let raw_name = name_entry
        .map(|e| e.raw_text.clone())
        .filter(|b| !b.is_empty());
    let price = table.entry(1, US_ENGLISH).map(|e| atoi(&e.raw_text));
    let description = table
        .entry(2, US_ENGLISH)
        .map(|e| e.text.clone())
        .filter(|s| !s.is_empty());
    FileMetadata {
        name,
        raw_name,
        price,
        description,
    }
}

/// A pattern file's own price (see [`SurfaceFile::price`]), in US
/// English (language 1), as everything here reads: the original takes
/// the game's language (0x47028b, 0x470256).
///
/// The table is the `STR#` with ID 0 and a flags' high byte of 0, as the
/// original's string loader looks it up (through 0x46ff01 and 0x41bab1,
/// which matches the key's ID and high byte at 0x41d94f to 0x41d95b).
fn metadata_price(iff: &Iff) -> Option<i32> {
    let chunk = iff.get(b"STR#", 0)?;
    let table = StringTable::parse_chunk(iff.data(chunk), iff.resource_flags(chunk)).ok()?;
    table.entry(1, US_ENGLISH).map(|e| atoi(&e.raw_text))
}

#[cfg(test)]
fn price(iff: &Iff) -> Option<i32> {
    metadata(iff).price
}

/// A number read from the start of a string's bytes as C's `atoi` reads
/// it (the original's 0x636029): leading white space, an optional sign,
/// then decimal digits up to the first other byte; 0 when there are none,
/// so a string that starts with anything else (a byte order mark, say)
/// reads 0. It wraps where the C runtime's would overflow.
pub fn atoi(text: &[u8]) -> i32 {
    let start = text
        .iter()
        .position(|b| !matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c))
        .unwrap_or(text.len());
    let rest = &text[start..];
    let (negative, rest) = match rest.first() {
        Some(b'-') => (true, &rest[1..]),
        Some(b'+') => (false, &rest[1..]),
        _ => (false, rest),
    };
    let mut value = 0i32;
    for &digit in rest.iter().take_while(|b| b.is_ascii_digit()) {
        value = value.wrapping_mul(10).wrapping_add(i32::from(digit - b'0'));
    }
    if negative {
        value.wrapping_neg()
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::{atoi, price};
    use sims_formats::iff::Iff;

    #[test]
    fn atoi_reads_as_c_does() {
        assert_eq!(atoi(b"20"), 20);
        assert_eq!(atoi(b"  -7 dollars"), -7);
        assert_eq!(atoi(b"+5"), 5);
        assert_eq!(atoi(b"Price"), 0);
        assert_eq!(atoi(b""), 0);
        assert_eq!(atoi(b"\xef\xbb\xbf25"), 0);
    }

    /// A version -1 string table of `texts`.
    fn table(texts: &[&str]) -> Vec<u8> {
        let mut data = (-1i16).to_le_bytes().to_vec();
        data.extend((texts.len() as u16).to_le_bytes());
        for text in texts {
            data.extend(text.as_bytes());
            data.push(0);
        }
        data
    }

    /// An IFF of `STR#` chunks by ID, in file order, each with flag 0x10
    /// (little-endian).
    fn iff(tables: &[(u16, Vec<u8>)]) -> Iff {
        let flagged: Vec<_> = tables
            .iter()
            .map(|(id, d)| (*id, 0x10, d.clone()))
            .collect();
        iff_flags(&flagged)
    }

    /// An IFF of `STR#` chunks by ID and flags, in file order.
    fn iff_flags(tables: &[(u16, u16, Vec<u8>)]) -> Iff {
        let mut bytes = b"IFF FILE 2.5:TYPE FOLLOWED BY SIZE\0 JAMIE DOLINE 1998".to_vec();
        bytes.resize(64, 0);
        for (id, flags, data) in tables {
            bytes.extend(b"STR#");
            bytes.extend((76 + data.len() as u32).to_be_bytes());
            bytes.extend(id.to_be_bytes());
            bytes.extend(flags.to_be_bytes());
            bytes.extend([0u8; 64]);
            bytes.extend(data);
        }
        Iff::parse(bytes).unwrap()
    }

    #[test]
    fn a_pattern_file_is_priced_by_its_string_table_0() {
        // Table 0 is read even when another comes first.
        let file = iff(&[(5, table(&["x", "99"])), (0, table(&["Name", "25"]))]);
        assert_eq!(price(&file), Some(25));
        // Without table 0, the default.
        assert_eq!(price(&iff(&[(1, table(&["x", "99"]))])), None);
        // Without its string 1, too.
        assert_eq!(price(&iff(&[(0, table(&["Name"]))])), None);
        // A byte order mark before the digits reads 0.
        assert_eq!(price(&iff(&[(0, table(&["Name", "\u{feff}25"]))])), Some(0));
        // Without flag 0x10 a version -1 table's count is big-endian.
        let mut big = table(&["Name", "25"]);
        big.swap(2, 3);
        assert_eq!(price(&iff_flags(&[(0, 0, big.clone())])), Some(25));
        assert_eq!(price(&iff_flags(&[(0, 0x10, big)])), None);
        // Only a table whose flags' high byte is 0, wherever it stands.
        let high = iff_flags(&[(0, 0x0110, table(&["Name", "25"]))]);
        assert_eq!(price(&high), None);
        for two in [
            [
                (0, 0x0010, table(&["Name", "25"])),
                (0, 0x0110, table(&["Name", "60"])),
            ],
            [
                (0, 0x0110, table(&["Name", "60"])),
                (0, 0x0010, table(&["Name", "25"])),
            ],
        ] {
            assert_eq!(price(&iff_flags(&two)), Some(25));
        }
        // A little-endian length-prefixed table: its count is its first word.
        let pascal = b"\x02\x00\x04Name\x0225".to_vec();
        assert_eq!(price(&iff_flags(&[(0, 0x10, pascal)])), Some(25));
    }
}
