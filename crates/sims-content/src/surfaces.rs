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

/// One kind's catalog: its pattern files by index.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SurfaceCatalog {
    pub files: Vec<SurfaceFile>,
}

impl SurfaceCatalog {
    /// Builds the catalog of `kind` for an install.
    pub fn build(install: &Install, kind: SurfaceKind) -> Result<Self> {
        let Profile {
            builtin,
            suffix,
            own,
            sprite,
            seeds,
        } = kind.profile();
        let mut files = vec![SurfaceFile {
            name: builtin.to_owned(),
            location: format!("GameData/{builtin}"),
            keys: BTreeMap::new(),
            price: None,
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
        let price = price(&iff);
        Ok(Some(SurfaceFile {
            name: sims_formats::far::decode_name(&name),
            location,
            keys,
            price,
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

/// A pattern file's own price (see [`SurfaceFile::price`]).
fn price(iff: &Iff) -> Option<i32> {
    let chunk = iff.get(b"STR#", 0)?;
    let table = StringTable::parse_chunk(iff.data(chunk), chunk.flags).ok()?;
    Some(atoi(&table.entry(1, US_ENGLISH)?.raw_text))
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
    }
}
