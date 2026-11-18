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
    /// Each made type, by its new GUID.
    pub made: BTreeMap<u32, Character>,
    /// The number the next copy tries first (neighborhood +0x160). The
    /// neighborhood's load sets it to 0 (0x424e8e); each copy attempt adds
    /// one, and it goes back to 0 at 32000 (0x4262c0).
    pub next_file: i32,
}

/// One made person type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Character {
    /// The template it was copied from: TemplatePerson, TemplateDog or
    /// TemplateCat.
    pub template: u32,
    /// The number of its file, `Characters/User%05d.iff`.
    pub file: i32,
    /// The body strings (the string table definition word 19 names) that
    /// differ from the template's, by index counting from 0.
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
