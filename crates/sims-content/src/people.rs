//! What the original needs, besides a house file and the neighborhood's
//! records, to construct the people a house's saved objects include: their
//! skeletons and the profiles their resets read.

use crate::index::{far_members, sorted_entries};
use crate::install::resolve;
use crate::{Catalog, Error, Index, Install, Resource, Result};
use sims_formats::avatar::read_bcf;
use sims_formats::iff::Iff;
use sims_formats::strings::StringTable;
use std::collections::BTreeMap;

/// The skeleton a type of person is built on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skeleton {
    /// The skeleton's name, from the type's body strings.
    pub name: String,
    /// The file that holds it, as a path relative to the install.
    pub file: String,
    /// Its place among the skeletons of that file.
    pub place: usize,
    /// For every skeleton of the file, in the file's order, the wiggle
    /// scale of each bone (the integer at bone offset 0x78).
    pub scales: Vec<Vec<i32>>,
}

/// What a person's reset reads from the person type's files when it
/// reloads the person's appearance (0x5888d3).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Profile {
    /// Strings 12, 13 and 14 of the type's body strings, counting from 0:
    /// the gender, the age and the skin tone. None is a string the table
    /// does not have; an empty string is still a string.
    pub gender: Option<String>,
    pub age: Option<String>,
    pub skin: Option<String>,
    /// The skin tone names: string table 8192 of the type's semi-global
    /// file, in order. Empty when the file or the table is missing.
    pub tones: Vec<String>,
}

/// The skeletons and profiles of a set of person types.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct People {
    /// Each person type's skeleton, by the type's GUID.
    pub skeletons: BTreeMap<u32, Skeleton>,
    /// Each person type's profile, by the type's GUID.
    pub profiles: BTreeMap<u32, Profile>,
}

/// The string table an object definition of type 2 names its body in: the
/// definition's word 19 (0x457f60). Its first string is the skeleton's
/// name.
const BODY_STRINGS: usize = 19 * 2;

/// The body strings a reset reads (0x5888d3), counting from 0.
const GENDER: usize = 12;
const AGE: usize = 13;
const SKIN: usize = 14;

/// The string table of a semi-global file that names the skin tones.
const SKIN_TONES: u16 = 8192;

impl People {
    /// Finds the skeleton and profile of each person type in `guids`. A
    /// GUID that is not a person type (OBJD type 2) is skipped.
    ///
    /// A skeleton is looked up by name among the skeleton files (`.bcf`
    /// members with at least one skeleton) of the archives in
    /// `GameData/Animation` and of the indexed object directories. The
    /// original resolves the name through its registry of avatar files,
    /// whose search order is not ported; so a name that two files define
    /// is an error here, and one that a single file defines is that file
    /// whatever the order. That these are the files the registry searches
    /// is **unverified**. Text skeleton files (`.cmx`) are not searched.
    pub fn load(
        install: &Install,
        index: &Index,
        catalog: &Catalog,
        guids: impl IntoIterator<Item = u32>,
    ) -> Result<Self> {
        let mut names = BTreeMap::new();
        let mut profiles = BTreeMap::new();
        for guid in guids {
            let Some(object) = catalog.object(guid) else {
                continue;
            };
            if object.kind != 2 || names.contains_key(&guid) {
                continue;
            }
            let resource = &index.resources[object.resource];
            let body = body_strings(index, resource, object.chunk)?;
            let row = |n: usize| body.get(n, 1).map(str::to_owned);
            let tones = match catalog.semi_globals.get(&object.resource) {
                Some(&Some(semi_global)) => skin_tones(index, &index.resources[semi_global])?,
                _ => Vec::new(),
            };
            profiles.insert(
                guid,
                Profile {
                    gender: row(GENDER),
                    age: row(AGE),
                    skin: row(SKIN),
                    tones,
                },
            );
            let location = resource.location(&index.root);
            let name = body
                .get(0, 1)
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
                .ok_or_else(|| {
                    Error::Unsupported(format!(
                        "{location}: the person's body strings name no skeleton"
                    ))
                })?;
            names.insert(guid, name);
        }
        let mut skeletons = BTreeMap::new();
        if !names.is_empty() {
            let files = skeleton_files(install, index)?;
            for (guid, name) in names {
                let mut found = files.iter().flat_map(|(file, skeletons)| {
                    skeletons
                        .iter()
                        .enumerate()
                        .filter(|(_, (n, _))| n.eq_ignore_ascii_case(&name))
                        .map(move |(place, _)| (file, place, skeletons))
                });
                let Some((file, place, all)) = found.next() else {
                    return Err(Error::Unsupported(format!(
                        "no skeleton file defines the skeleton {name:?}"
                    )));
                };
                if let Some((other, ..)) = found.next() {
                    return Err(Error::Unsupported(format!(
                        "the skeleton {name:?} is defined by {file} and by {other}; choosing \
                         between them needs the original's avatar registry, which is not ported"
                    )));
                }
                skeletons.insert(
                    guid,
                    Skeleton {
                        name,
                        file: file.clone(),
                        place,
                        scales: all.iter().map(|(_, scales)| scales.clone()).collect(),
                    },
                );
            }
        }
        Ok(Self {
            skeletons,
            profiles,
        })
    }
}

/// The body strings of the person definition `chunk` of a character
/// file.
fn body_strings(index: &Index, resource: &Resource, chunk: u16) -> Result<StringTable> {
    let location = resource.location(&index.root);
    let iff = Iff::parse(resource.read()?).map_err(|e| Error::format(&location, e))?;
    let missing = |what: &str| Error::Unsupported(format!("{location}: {what}"));
    let definition = iff
        .get(b"OBJD", chunk)
        .ok_or_else(|| missing("the person's definition is gone"))?;
    let table = iff
        .data(definition)
        .get(BODY_STRINGS..BODY_STRINGS + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or_else(|| missing("the person's definition names no body strings"))?;
    let strings = iff
        .get(b"STR#", table)
        .ok_or_else(|| missing("the person's body strings are missing"))?;
    StringTable::parse(iff.data(strings))
        .map_err(|e| Error::format(&format!("{location} STR# #{table}"), e))
}

/// The skin tone names of a semi-global file: its string table 8192.
fn skin_tones(index: &Index, resource: &Resource) -> Result<Vec<String>> {
    let location = resource.location(&index.root);
    let iff = Iff::parse(resource.read()?).map_err(|e| Error::format(&location, e))?;
    let Some(chunk) = iff.get(b"STR#", SKIN_TONES) else {
        return Ok(Vec::new());
    };
    let table = StringTable::parse(iff.data(chunk))
        .map_err(|e| Error::format(&format!("{location} STR# #{SKIN_TONES}"), e))?;
    Ok(table
        .entries
        .iter()
        .filter(|entry| entry.language == 1)
        .map(|entry| entry.text.clone())
        .collect())
}

/// A skeleton file's skeletons: each one's name and bone scales.
type Skeletons = Vec<(String, Vec<i32>)>;

/// Every skeleton file the lookup searches, by path relative to the
/// install, in path order.
fn skeleton_files(install: &Install, index: &Index) -> Result<BTreeMap<String, Skeletons>> {
    let mut resources = Vec::new();
    if let Some(directory) = resolve(&install.root, "GameData/Animation")? {
        for path in sorted_entries(&directory)? {
            let is_archive = path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("far"));
            if is_archive && path.is_file() {
                for member in far_members(&path)? {
                    resources.push(Resource {
                        mount: 0,
                        path: path.clone(),
                        member: Some(member),
                    });
                }
            }
        }
    }
    resources.extend(index.resources.iter().cloned());
    let mut files = BTreeMap::new();
    for resource in resources {
        if !resource.name().to_ascii_lowercase().ends_with(".bcf") {
            continue;
        }
        let bytes = resource.read()?;
        // A file starts with how many skeletons it holds. Most hold none.
        if bytes.first_chunk::<4>().is_none_or(|n| *n == [0; 4]) {
            continue;
        }
        let location = resource.location(&install.root);
        // A file that does not decode is not one the game could use.
        let Ok(parsed) = read_bcf(&bytes) else {
            continue;
        };
        let skeletons = parsed
            .skeletons
            .into_iter()
            .map(|s| {
                let scales = s.bones.iter().map(|b| b.wiggle.gradient_scale).collect();
                (s.name, scales)
            })
            .collect();
        files.insert(location, skeletons);
    }
    Ok(files)
}
