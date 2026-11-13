use crate::index::sorted_entries;
use crate::install::resolve;
use crate::{Catalog, Error, Result};
use sims_formats::iff::Iff;
use sims_formats::lot::{self, Array, HouseView, ObjectTypes, Simulator};
use sims_formats::neighborhood::Family;
use sims_formats::saved_objects::Objects;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// A neighborhood directory (`UserData`, `UserData2`, ...): its families,
/// house files and character files.
#[derive(Debug, Clone)]
pub struct Neighborhood {
    pub directory: PathBuf,
    /// Families by FAMI chunk ID, in chunk ID order.
    pub families: BTreeMap<u16, Family>,
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
        Ok(Self {
            directory: directory.to_path_buf(),
            families,
        })
    }

    /// The character directory, whose files define this neighborhood's Sims.
    /// Add it to the index with [`crate::Index::add_characters`] before
    /// building the catalog.
    pub fn characters(&self) -> Result<Option<PathBuf>> {
        resolve(&self.directory, "Characters")
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

fn read_iff(path: &Path) -> Result<Iff> {
    let bytes = std::fs::read(path).map_err(|e| Error::io(path, e))?;
    Iff::parse(bytes).map_err(|e| Error::format(&path.display().to_string(), e))
}
