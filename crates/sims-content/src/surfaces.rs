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
use crate::{Error, Install, Resource, Result};
use sims_formats::iff::Iff;
use std::collections::BTreeMap;

/// Floors or walls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceKind {
    Floor,
    Wall,
}

impl SurfaceKind {
    /// The built-in file, the suffix the scan keeps, the base game's own
    /// directory, the sprite chunk counted, and the free key ranges in the
    /// order they are seeded (0x49ff66, 0x4c4c93).
    fn profile(
        self,
    ) -> (
        &'static str,
        &'static [u8],
        &'static str,
        [u8; 4],
        [(u16, u16); 2],
    ) {
        match self {
            Self::Floor => (
                "floors.iff",
                b".flr",
                "GameData/Floors/",
                *b"SPR2",
                [(30, 254), (256, 65534)],
            ),
            Self::Wall => (
                "walls.iff",
                b".wll",
                "GameData/Walls/",
                *b"SPR#",
                [(31, 247), (256, 65534)],
            ),
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
}

/// One kind's catalog: its pattern files by index.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SurfaceCatalog {
    pub files: Vec<SurfaceFile>,
}

impl SurfaceCatalog {
    /// Builds the catalog of `kind` for an install.
    pub fn build(install: &Install, kind: SurfaceKind) -> Result<Self> {
        let (builtin, suffix, own, sprite, seeds) = kind.profile();
        let mut files = vec![SurfaceFile {
            name: builtin.to_owned(),
            location: format!("GameData/{builtin}"),
            keys: BTreeMap::new(),
        }];
        // The free list: 0x58055b puts each range at the front, and a key
        // is taken from the front range (0x4a1a08).
        let mut free: Vec<(u16, u16)> = seeds.iter().rev().copied().collect();
        let mut directories = Vec::new();
        for (bit, directory, _) in crate::Expansions::ALL {
            if install.expansions.contains(bit) {
                directories.push((directory.to_owned(), "wallsandfloors\\"));
            }
        }
        if install.expansions.0 > 3 {
            directories.push(("ExpansionShared".to_owned(), "wallsandfloors\\"));
        }
        directories.push((own.to_owned(), ""));
        for (directory, at) in directories {
            let Some(path) = resolve(&install.root, &directory)? else {
                continue;
            };
            if !path.is_dir() {
                continue;
            }
            let (listed, skipped) = Index::listing(&path, at.as_bytes())?;
            if let Some(reason) = skipped.into_iter().next() {
                return Err(Error::Unsupported(reason));
            }
            for (name, resource) in listed {
                if !name.ends_with(suffix) {
                    continue;
                }
                if let Some(file) = Self::add(&resource, name, sprite, &mut free, &install.root)? {
                    files.push(file);
                }
            }
        }
        Ok(Self { files })
    }

    /// Reads one pattern file (0x4a0744, 0x4c552e), or None for a file the
    /// original leaves out: one with no pattern.
    fn add(
        resource: &Resource,
        name: Vec<u8>,
        sprite: [u8; 4],
        free: &mut Vec<(u16, u16)>,
        root: &std::path::Path,
    ) -> Result<Option<SurfaceFile>> {
        let location = resource.location(root);
        // A file the original cannot open is left out too. Which files our
        // reader refuses that the original opens, or the reverse, is not
        // known, so a refusal stops the build.
        let iff = Iff::parse(resource.read()?).map_err(|e| Error::format(&location, e))?;
        let count = iff.all(&sprite).count() as i16 / 3;
        if count < 1 {
            return Ok(None);
        }
        let mut keys = BTreeMap::new();
        for number in 1..=count as u16 {
            let Some((low, high)) = free.first().copied() else {
                break;
            };
            free.remove(0);
            if low < high {
                free.insert(0, (low + 1, high));
            }
            if iff.get(&sprite, number).is_some() {
                keys.insert(number, low);
            }
        }
        Ok(Some(SurfaceFile {
            name: String::from_utf8_lossy(&name).into_owned(),
            location,
            keys,
        }))
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

/// Both catalogs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Surfaces {
    pub floors: SurfaceCatalog,
    pub walls: SurfaceCatalog,
}

impl Surfaces {
    pub fn build(install: &Install) -> Result<Self> {
        Ok(Self {
            floors: SurfaceCatalog::build(install, SurfaceKind::Floor)?,
            walls: SurfaceCatalog::build(install, SurfaceKind::Wall)?,
        })
    }
}
