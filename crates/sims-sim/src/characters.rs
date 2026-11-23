//! Person types the game makes while it runs (Make New Character,
//! primitive 19, 0x4260e1).
//!
//! The original copies a template's file to `Characters/User%05d.iff`,
//! gives each object definition in the copy a new GUID (0x4725d5),
//! registers the file and edits the new definition's body strings. The
//! port keeps no files: a made type is its template's definition, looked
//! up under the new GUID, with the GUID words and the edited body strings
//! kept here.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The OBJD words that hold a definition's GUID (byte 0x1c, 0x4725e1),
/// low word first.
pub const GUID_WORDS: usize = 14;
/// The OBJD words that keep the GUID a definition was copied from (byte
/// 0x8c, written by 0x4725e4), low word first.
pub const ORIGINAL_GUID_WORDS: usize = 70;

/// The body strings Make New Character writes, counting from 0: the
/// skeleton's name, the body, the head, the gender, the age and the skin
/// tone.
pub const SKELETON: usize = 0;
pub const BODY: usize = 1;
pub const HEAD: usize = 2;
pub const GENDER: usize = 12;
pub const AGE: usize = 13;
pub const SKIN: usize = 14;

/// The made person types and the neighborhood's file counter.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Characters {
    /// Each made type, by its new GUID, in the order made.
    pub made: MadeTypes,
    /// The number the next copy tries first (neighborhood +0x160). The
    /// neighborhood's load sets it to 0 (0x424e8e); each copy attempt adds
    /// one, and it goes back to 0 at 32000 (0x4262c0).
    pub next_file: i32,
}

/// The made person types by GUID, in the order they were made.
///
/// The order is the original's: making a type (0x4265fd, and 0x426e65,
/// 0x42cf14, 0x4f6727) appends its GUID to the interface's table of person
/// types (0x4e8a3a), whose places break the relationship panel's ties
/// (0x4ea035). Two types made in one tick can have descending GUIDs
/// (0x4260e1 draws the low bits), so the GUID does not give the order.
///
/// It encodes as a map, as a `BTreeMap` did: its length, then each GUID
/// and type, here in the order made. A build that keeps types by GUID
/// would read it and save it back in GUID order, so each stored encoding
/// that carries it has a version an older build refuses: the
/// neighborhood store (format 5), the save records (format 3), the named
/// snapshot records (record format 33) and `Saved` (format 11). An encoding of an older
/// version reads back in GUID order ([`MadeTypes::sort_by_guid`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MadeTypes(Vec<(u32, Character)>);

impl MadeTypes {
    pub fn get(&self, guid: &u32) -> Option<&Character> {
        self.0.iter().find(|(g, _)| g == guid).map(|(_, c)| c)
    }

    pub fn get_mut(&mut self, guid: &u32) -> Option<&mut Character> {
        self.0.iter_mut().find(|(g, _)| g == guid).map(|(_, c)| c)
    }

    pub fn contains_key(&self, guid: &u32) -> bool {
        self.0.iter().any(|(g, _)| g == guid)
    }

    /// Adds a type at the end, or replaces the one with its GUID in place.
    pub fn insert(&mut self, guid: u32, character: Character) -> Option<Character> {
        match self.get_mut(&guid) {
            Some(old) => Some(std::mem::replace(old, character)),
            None => {
                self.0.push((guid, character));
                None
            }
        }
    }

    pub fn remove(&mut self, guid: &u32) -> Option<Character> {
        let index = self.0.iter().position(|(g, _)| g == guid)?;
        Some(self.0.remove(index).1)
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = (&u32, &Character)> + ExactSizeIterator {
        self.0.iter().map(|(g, c)| (g, c))
    }

    /// The GUIDs in the order made.
    pub fn keys(&self) -> impl DoubleEndedIterator<Item = &u32> + ExactSizeIterator {
        self.0.iter().map(|(g, _)| g)
    }

    pub fn values(&self) -> impl DoubleEndedIterator<Item = &Character> + ExactSizeIterator {
        self.0.iter().map(|(_, c)| c)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Puts the types in GUID order, the order an encoding made before the
    /// order made was kept has them in.
    pub fn sort_by_guid(&mut self) {
        self.0.sort_by_key(|&(guid, _)| guid);
    }
}

impl std::ops::Index<&u32> for MadeTypes {
    type Output = Character;

    /// The type with `guid`; panics if none was made, as a map does.
    fn index(&self, guid: &u32) -> &Character {
        self.get(guid).expect("no made type with this GUID")
    }
}

impl FromIterator<(u32, Character)> for MadeTypes {
    fn from_iter<I: IntoIterator<Item = (u32, Character)>>(iter: I) -> Self {
        let mut made = Self::default();
        for (guid, character) in iter {
            made.insert(guid, character);
        }
        made
    }
}

impl<'a> IntoIterator for &'a MadeTypes {
    type Item = (&'a u32, &'a Character);
    type IntoIter = std::iter::Map<
        std::slice::Iter<'a, (u32, Character)>,
        fn(&'a (u32, Character)) -> (&'a u32, &'a Character),
    >;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter().map(|(g, c)| (g, c))
    }
}

impl Serialize for MadeTypes {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_map(self.0.iter().map(|(g, c)| (g, c)))
    }
}

impl<'de> Deserialize<'de> for MadeTypes {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = MadeTypes;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a map of made person types")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut made = MadeTypes::default();
                while let Some((guid, character)) = map.next_entry()? {
                    made.insert(guid, character);
                }
                Ok(made)
            }
        }
        deserializer.deserialize_map(Visitor)
    }
}

/// One made person type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Character {
    /// The template it was copied from: TemplatePerson, TemplateDog or
    /// TemplateCat.
    pub template: u32,
    /// The number of its file, `Characters/User%05d.iff`.
    pub file: i32,
    /// The body strings (the string table definition word 19 names) Make
    /// New Character wrote, by index counting from 0; the others are the
    /// template's.
    pub strings: BTreeMap<usize, String>,
}

impl Characters {
    /// The definition `guid` is looked up as: its template for a made
    /// type, otherwise itself.
    pub fn definition(&self, guid: u32) -> u32 {
        self.made.get(&guid).map_or(guid, |made| made.template)
    }

    /// Body string `index` of type `guid`, when it differs from its
    /// template's.
    pub fn string(&self, guid: u32, index: usize) -> Option<&str> {
        self.made
            .get(&guid)?
            .strings
            .get(&index)
            .map(String::as_str)
    }
}

/// Writes `guid` to a definition's words at `at`, low word first.
pub fn put_guid(words: &mut [i16], at: usize, guid: u32) {
    words[at] = guid as u16 as i16;
    words[at + 1] = (guid >> 16) as u16 as i16;
}
