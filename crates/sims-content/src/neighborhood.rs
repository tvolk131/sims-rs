use crate::index::sorted_entries;
use crate::install::resolve;
use crate::{Catalog, Error, Result};
use sims_formats::iff::Iff;
use sims_formats::lot::{self, Array, HouseView, ObjectTypes, Simulator};
use sims_formats::neighborhood::Family;
use sims_formats::saved_objects::Objects;
use sims_formats::strings::StringTable;
use sims_formats::tatt::Tatt;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// A neighborhood directory (`UserData`, `UserData2`, ...): its families,
/// house files and character files.
#[derive(Debug, Clone)]
pub struct Neighborhood {
    pub directory: PathBuf,
    /// Families by FAMI chunk ID, in chunk ID order.
    pub families: BTreeMap<u16, Family>,
    /// The saved type attributes (TATT #1), when the file has them.
    pub type_attributes: Option<Tatt>,
}

impl Neighborhood {
    pub fn open(directory: &Path) -> Result<Self> {
        let path = resolve(directory, "Neighborhood.iff")?.ok_or_else(|| Error::Missing {
            root: directory.to_path_buf(),
            path: "Neighborhood.iff".into(),
        })?;
        let iff = read_iff(&path)?;
        let mut families = BTreeMap::new();
        for chunk in iff.all(b"FAMI") {
            let family = Family::parse(iff.data(chunk))
                .map_err(|e| Error::format(&format!("{} FAMI #{}", path.display(), chunk.id), e))?;
            families.insert(chunk.id, family);
        }
        let type_attributes = iff
            .get(b"TATT", 1)
            .map(|chunk| {
                Tatt::parse(iff.data(chunk))
                    .map_err(|e| Error::format(&format!("{} TATT #1", path.display()), e))
            })
            .transpose()?;
        Ok(Self {
            directory: directory.to_path_buf(),
            families,
            type_attributes,
        })
    }

    /// The character directory, whose files define this neighborhood's Sims.
    /// Add it to the index with [`crate::Index::add_characters`] before
    /// building the catalog.
    pub fn characters(&self) -> Result<Option<PathBuf>> {
        resolve(&self.directory, "Characters")
    }

    /// Each lot's zoning by lot number, from the neighborhood's
    /// `LotZoning.iff`: 1 residential, 2 community. Empty when the
    /// neighborhood has no such file.
    ///
    /// The file's STR# 1 holds one `lot, zone` row per line in its first
    /// language set. A zone naming "community" and not "residential" is 2,
    /// anything else is 1, and the first row for a lot wins. When no row covers the Magic Town
    /// lots 90 to 92 and `expansions` (the installed-expansions mask, bit 7
    /// Makin' Magic) includes that pack, lots 90 to 92 are added as
    /// residential and 93 to 98 as community.
    ///
    /// Ported from the old prototype (legacy/crates/sims-vm/src/zoning.rs,
    /// recovered from 0x424fb9 and 0x42b9e3). **Unverified**: the prototype
    /// read the install's `GameData/MasterLotZoning.iff` instead; for the
    /// shipped neighborhoods both files give the same table.
    pub fn zoning(&self, expansions: u16) -> Result<BTreeMap<i16, i32>> {
        let mut table = BTreeMap::new();
        let Some(path) = resolve(&self.directory, "LotZoning.iff")? else {
            return Ok(table);
        };
        let iff = read_iff(&path)?;
        let Some(chunk) = iff.get(b"STR#", 1) else {
            return Ok(table);
        };
        let strings = StringTable::parse(iff.data(chunk))
            .map_err(|e| Error::format(&format!("{} STR# #1", path.display()), e))?;
        let mut magic = false;
        for entry in strings.entries.iter().filter(|e| e.language == 1) {
            let Some((lot, zone)) = entry.text.split_once(',') else {
                continue;
            };
            let lot = leading_number(lot);
            let zone = zone.to_ascii_lowercase();
            let zone = if !zone.contains("residential") && zone.contains("community") {
                2
            } else {
                1
            };
            magic |= (90..=92).contains(&lot);
            if let Ok(lot) = i16::try_from(lot) {
                table.entry(lot).or_insert(zone);
            }
        }
        if !magic && expansions & 0x80 != 0 {
            for lot in 90..=98 {
                table.entry(lot).or_insert(if lot <= 92 { 1 } else { 2 });
            }
        }
        Ok(table)
    }

    /// House file paths by house number, in number order.
    pub fn houses(&self) -> Result<BTreeMap<u32, PathBuf>> {
        let mut houses = BTreeMap::new();
        let Some(directory) = resolve(&self.directory, "Houses")? else {
            return Ok(houses);
        };
        for path in sorted_entries(&directory)? {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            let lower = name.to_ascii_lowercase();
            if let Some(number) = lower
                .strip_prefix("house")
                .and_then(|n| n.strip_suffix(".iff"))
                .and_then(|n| n.parse().ok())
            {
                houses.insert(number, path);
            }
        }
        Ok(houses)
    }

    /// The family living in a house, if any. A family whose house number is
    /// 0 has no house: in the shipped neighborhoods every such family lacks
    /// the moved-in flag (bit 0) and every family with it has a nonzero house.
    pub fn family_in(&self, house: u32) -> Option<(u16, &Family)> {
        if house == 0 {
            return None;
        }
        self.families
            .iter()
            .find(|(_, f)| u32::try_from(f.house).ok() == Some(house))
            .map(|(&id, f)| (id, f))
    }
}

/// A saved lot, as stored in an original house file.
#[derive(Debug, Clone)]
pub struct House {
    pub path: PathBuf,
    pub simulator: Simulator,
    pub types: ObjectTypes,
    pub objects: Objects,
    /// Lot arrays (floors, walls, terrain and more) by Arry chunk ID.
    pub arrays: BTreeMap<u16, Array>,
    pub view: Option<HouseView>,
    pub model: Option<lot::House>,
}

impl House {
    /// Reads a house file. Saved objects are matched to their definitions
    /// through `catalog`, which must already include the neighborhood's
    /// characters; an object whose GUID it lacks is an error.
    pub fn load(path: &Path, catalog: &Catalog) -> Result<Self> {
        let iff = read_iff(path)?;
        let location = |tag: &str, id: u16| format!("{} {tag} #{id}", path.display());
        let one = |tag: &'static [u8; 4]| {
            iff.chunks
                .iter()
                .find(|c| &c.tag == tag)
                .ok_or_else(|| Error::Missing {
                    root: path.to_path_buf(),
                    path: format!("{} chunk", String::from_utf8_lossy(tag)),
                })
        };
        let chunk = one(b"SIMI")?;
        let simulator = Simulator::parse(iff.data(chunk))
            .map_err(|e| Error::format(&location("SIMI", chunk.id), e))?;
        let chunk = one(b"objt")?;
        let types = ObjectTypes::parse(iff.data(chunk))
            .map_err(|e| Error::format(&location("objt", chunk.id), e))?;
        let chunk = one(b"ObjM")?;
        let multipart = |guid| catalog.object(guid).map(|o| o.master != 0);
        let objects = Objects::parse(iff.data(chunk), &types, multipart)
            .map_err(|e| Error::format(&location("ObjM", chunk.id), e))?;
        let mut arrays = BTreeMap::new();
        for chunk in iff.all(b"Arry") {
            let array = Array::parse(iff.data(chunk))
                .map_err(|e| Error::format(&location("Arry", chunk.id), e))?;
            arrays.insert(chunk.id, array);
        }
        let (mut view, mut model) = (None, None);
        for chunk in iff.all(b"HOUS") {
            let at = location("HOUS", chunk.id);
            match chunk.id {
                0 => {
                    view =
                        Some(HouseView::parse(iff.data(chunk)).map_err(|e| Error::format(&at, e))?)
                }
                _ => {
                    model = Some(
                        lot::House::parse(iff.data(chunk)).map_err(|e| Error::format(&at, e))?,
                    )
                }
            }
        }
        Ok(Self {
            path: path.to_path_buf(),
            simulator,
            types,
            objects,
            arrays,
            view,
            model,
        })
    }
}

/// The number a zoning row starts with, read as C's `atoi` reads it:
/// leading white space, an optional sign, then digits up to the first
/// other character. No digits give 0. A number too large for 32 bits
/// saturates here; what the original's `atoi` gives for one is not modeled.
fn leading_number(text: &str) -> i32 {
    let text = text.trim_start_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c']);
    let (negative, digits) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    let mut value = 0i64;
    for byte in digits.bytes().take_while(u8::is_ascii_digit) {
        value = (value * 10 + i64::from(byte - b'0')).min(i64::from(i32::MAX) + 1);
    }
    let value = if negative { -value } else { value };
    value.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

fn read_iff(path: &Path) -> Result<Iff> {
    let bytes = std::fs::read(path).map_err(|e| Error::io(path, e))?;
    Iff::parse(bytes).map_err(|e| Error::format(&path.display().to_string(), e))
}
