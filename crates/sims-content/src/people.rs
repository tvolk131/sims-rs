//! What the original needs, besides a house file and the neighborhood's
//! records, to construct the people a house's saved objects include: their
//! skeletons and the profiles their resets read.

use crate::index::{all_far_members, sorted_entries};
use crate::install::resolve;
use crate::{Catalog, Error, Index, Install, Resource, Result};
use sims_formats::avatar::catalog::{Asset, Binary};
use sims_formats::avatar::index::Kind;
use sims_formats::avatar::read_bcf;
use sims_formats::iff::Iff;
use sims_formats::strings::StringTable;
use std::collections::{BTreeMap, BTreeSet};

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
    /// The skeleton's bones, in the file's order, as far as posing reads
    /// them: what the walk's strides are worked out from (0x58831d).
    pub bones: Vec<Bone>,
}

/// One bone of a skeleton, as posing reads it. Floats are kept as the
/// bits of the file's single-precision values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bone {
    /// The bone's name, which a clip's motion names it by.
    pub name: String,
    /// The bone it hangs from, by place; None for the root.
    pub parent: Option<usize>,
    /// Its rest position relative to its parent.
    pub translation: [u32; 3],
    /// Its rest rotation relative to its parent, a quaternion.
    pub rotation: [u32; 4],
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
    /// Strings 3 to 6 of the type's body strings, counting from 0, which
    /// the animator binds its anchors to (0x5924a4): the bone that holds
    /// the head's point ("HEAD"), that point's name ("Top"), the bone that
    /// holds the hand's ("R_HAND") and that point's name ("Palm"). None
    /// is a string the table does not have.
    pub rig: [Option<String>; 4],
}

/// The skeletons and profiles of a set of person types.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct People {
    /// Each person type's skeleton, by the type's GUID.
    pub skeletons: BTreeMap<u32, Skeleton>,
    /// Each person type's profile, by the type's GUID.
    pub profiles: BTreeMap<u32, Profile>,
    /// The names of the suits (appearances) the avatar files define,
    /// lowered in ASCII: what an accessory put on by name can be. They
    /// come from the same files as the skeletons, so the same caveat
    /// holds: that these are the names the original's avatar registry
    /// holds is **unverified**.
    pub suits: BTreeSet<String>,
    /// Each avatar file whose skeletons or suits stopped decoding, by
    /// location, with the error: its suits after that point are missing
    /// from `suits`.
    pub suit_errors: BTreeMap<String, String>,
}

/// The string table an object definition of type 2 names its body in: the
/// definition's word 19 (0x457f60). Its first string is the skeleton's
/// name.
const BODY_STRINGS: usize = 19 * 2;

/// The body strings a reset reads (0x5888d3), counting from 0.
const GENDER: usize = 12;
const AGE: usize = 13;
const SKIN: usize = 14;
/// The first of the four body strings the animator's anchors read
/// (0x5924a4), counting from 0.
const RIG: usize = 3;

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
        let (people, mut unresolved) = Self::load_each(install, index, catalog, guids)?;
        match unresolved.pop_first() {
            Some((_, reason)) => Err(Error::Unsupported(reason)),
            None => Ok(people),
        }
    }

    /// As [`People::load`], but a person type whose skeleton cannot be
    /// chosen (no skeleton file defines it, or two do) is left out of
    /// both maps instead of failing the whole load. Returns the types left
    /// out, by GUID, with the reason. Making a person of a type left out
    /// fails, as it would with no entry at all.
    pub fn load_each(
        install: &Install,
        index: &Index,
        catalog: &Catalog,
        guids: impl IntoIterator<Item = u32>,
    ) -> Result<(Self, BTreeMap<u32, String>)> {
        let mut unresolved = BTreeMap::new();
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
                    rig: std::array::from_fn(|i| row(RIG + i)),
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
        let avatar_files = avatar_files(install, index)?;
        let (suits, suit_errors) = suit_names(install, &avatar_files)?;
        if !names.is_empty() {
            let files = skeleton_files(install, &avatar_files)?;
            for (guid, name) in names {
                let mut found = files.iter().flat_map(|(file, skeletons)| {
                    skeletons
                        .iter()
                        .enumerate()
                        .filter(|(_, (n, ..))| n.eq_ignore_ascii_case(&name))
                        .map(move |(place, _)| (file, place, skeletons))
                });
                let Some((file, place, all)) = found.next() else {
                    unresolved.insert(
                        guid,
                        format!("no skeleton file defines the skeleton {name:?}"),
                    );
                    continue;
                };
                if let Some((other, ..)) = found.next() {
                    unresolved.insert(
                        guid,
                        format!(
                            "the skeleton {name:?} is defined by {file} and by {other}; \
                             choosing between them needs the original's avatar registry, \
                             which is not ported"
                        ),
                    );
                    continue;
                }
                skeletons.insert(
                    guid,
                    Skeleton {
                        name,
                        file: file.clone(),
                        place,
                        scales: all.iter().map(|(_, scales, _)| scales.clone()).collect(),
                        bones: all[place].2.clone(),
                    },
                );
            }
        }
        profiles.retain(|guid, _| !unresolved.contains_key(guid));
        Ok((
            Self {
                skeletons,
                profiles,
                suits,
                suit_errors,
            },
            unresolved,
        ))
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

/// A skeleton file's skeletons: each one's name, bone scales and bones.
type Skeletons = Vec<(String, Vec<i32>, Vec<Bone>)>;

/// Every skeleton file the lookup searches, by path relative to the
/// install, in path order.
/// The avatar files (`.bcf`) the skeletons and suits are looked up in:
/// the archives' members in `GameData/Animation`, then the indexed
/// object directories' files.
fn avatar_files(install: &Install, index: &Index) -> Result<Vec<Resource>> {
    let mut resources = Vec::new();
    if let Some(directory) = resolve(&install.root, "GameData/Animation")? {
        for path in sorted_entries(&directory)? {
            let is_archive = path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("far"));
            if is_archive && path.is_file() {
                for member in all_far_members(&path)? {
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
    resources.retain(|resource| resource.name().to_ascii_lowercase().ends_with(".bcf"));
    Ok(resources)
}

/// The suit names the avatar files define, lowered, and the files that
/// stopped decoding. Each file is read record by record (as the
/// original's catalog reader does, 0x5950f5 lowering each suit's name)
/// through its skeletons and suits; its animations are not read, so a
/// file holding them still gives its suits. A file that fails keeps the
/// suits read before the failure, and the error is kept with its
/// location.
fn suit_names(
    install: &Install,
    files: &[Resource],
) -> Result<(BTreeSet<String>, BTreeMap<String, String>)> {
    let mut suits = BTreeSet::new();
    let mut errors = BTreeMap::new();
    for resource in files {
        let bytes = resource.read()?;
        if let Err(error) = read_suits(&bytes, &mut suits) {
            errors.insert(resource.location(&install.root), error.to_string());
        }
    }
    Ok((suits, errors))
}

/// Adds the suits one avatar file defines to `suits`. A file too short
/// to hold a group's count ends there, as in the original.
fn read_suits(bytes: &[u8], suits: &mut BTreeSet<String>) -> sims_formats::Result<()> {
    let mut catalog = Binary::new(bytes);
    for kind in [Kind::Skeleton, Kind::Appearance] {
        let Some(count) = catalog.count(kind)? else {
            return Ok(());
        };
        for _ in 0..count {
            if let Asset::Appearance(appearance) = catalog.read(kind)? {
                suits.insert(appearance.name);
            }
        }
    }
    Ok(())
}

fn skeleton_files(
    install: &Install,
    resources: &[Resource],
) -> Result<BTreeMap<String, Skeletons>> {
    let mut files = BTreeMap::new();
    for resource in resources {
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
                let bones = s
                    .bones
                    .into_iter()
                    .map(|b| Bone {
                        name: b.name,
                        parent: b.parent,
                        translation: b.translation.map(f32::to_bits),
                        rotation: b.rotation.map(f32::to_bits),
                    })
                    .collect();
                (s.name, scales, bones)
            })
            .collect();
        files.insert(location, skeletons);
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An avatar file with no skeletons, the suits `names` (no bindings),
    /// and then `tail`.
    fn catalog(names: &[&str], tail: &[u8]) -> Vec<u8> {
        let mut bytes = 0u32.to_le_bytes().to_vec();
        bytes.extend((names.len() as u32).to_le_bytes());
        for name in names {
            bytes.push(name.len() as u8);
            bytes.extend(name.as_bytes());
            bytes.extend([0u8; 12]); // kind, reserved, no bindings
        }
        bytes.extend(tail);
        bytes
    }

    #[test]
    fn a_file_with_animations_still_gives_its_suits() {
        // One animation follows, which neither this reader nor
        // `read_bcf` decodes.
        let bytes = catalog(&["Suit-Book101", "hat"], &1u32.to_le_bytes());
        assert!(read_bcf(&bytes).is_err());
        let mut suits = BTreeSet::new();
        read_suits(&bytes, &mut suits).unwrap();
        assert_eq!(suits, ["hat", "suit-book101"].map(String::from).into());
    }

    #[test]
    fn a_file_that_fails_keeps_the_suits_before_the_failure() {
        let mut bytes = catalog(&["cane"], &[]);
        // Claims a second suit, whose name runs past the end.
        bytes[4] = 2;
        bytes.extend([9, b'x']);
        let mut suits = BTreeSet::new();
        assert!(read_suits(&bytes, &mut suits).is_err());
        assert_eq!(suits, ["cane".to_string()].into());
        // Too short for the first count: nothing, and no error.
        read_suits(&[0, 0], &mut suits).unwrap();
    }
}
