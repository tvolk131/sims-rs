//! Radio, television and music-mode stations: hit lists the original
//! builds from directories when it starts, not from the sound tables.
//!
//! *(code)* The sound manager registers each station with a directory
//! and a hit list number (0x574c37, called from 0x571780 on). It lists
//! the directory's subdirectories: each becomes a sub hit list, holding
//! one patch per file in it, named `RadioStationAutoPatch` and the file's
//! name (0x574f0e). A directory with no subdirectory is its own one sub
//! list. The station's hit list holds the sub lists that are not empty,
//! and is added to when the sound tables already list that number (they
//! list most of them, empty). Scripts then pick a sub list from the
//! station's list and a patch from the sub list (`smart_setlist`,
//! `smart_choose`, twice).
//!
//! The numbers the original gives sub lists and patches come from a
//! counter; here they count up from [`FIRST_ID`], which no sound table
//! uses. Directory entries are taken in [`crate::files::key`] order; the
//! original takes them in the order the file system lists them. Only the
//! loose files [`crate::files::SoundFiles`] lists (those with a sound
//! extension) become patches.
//!
//! Which neighbourhood music the original registers depends on a byte of
//! its configuration: `NhoodUS` in the memory image this was read from,
//! and here. Every expansion's stations are registered, as they are in
//! that image (every expansion bit set).

use crate::files::{SoundFiles, Source, key};
use std::collections::{BTreeMap, BTreeSet};

/// The first number given to a generated sub list or patch.
pub const FIRST_ID: u32 = 1 << 30;

/// The stations, in the original's order of registration: hit list
/// number and directory. (Two hit lists can share a directory.)
pub const STATIONS: [(u32, &str); 34] = [
    (232, "Music\\Stations\\Country"),
    (233, "Music\\Stations\\Rock"),
    (234, "Music\\Stations\\Latin"),
    (235, "Music\\Stations\\Classica"),
    (221, "Music\\Modes\\Buy"),
    (222, "Music\\Modes\\Build"),
    (223, "Music\\Modes\\NhoodUS"),
    (103_100, "Music\\Modes\\MakinMagic"),
    (103_101, "Music\\Modes\\MM_Credits"),
    (103_102, "Music\\Modes\\MM_Build"),
    (103_104, "Music\\Modes\\MM_Buy"),
    (103_000, "ExpansionPack6\\Sound\\TVStations\\Animal"),
    (103_001, "ExpansionPack6\\Sound\\TVStations\\Entertainment"),
    (103_002, "ExpansionPack6\\Sound\\TVStations\\News"),
    (103_003, "ExpansionPack6\\Sound\\TVStations\\Sports"),
    (100_100, "Music\\Stations\\Superstar"),
    (100_102, "Music\\Modes\\SuperStar"),
    (100_101, "Music\\Stations\\Superstar"),
    (20_900, "Music\\Stations\\UnlCheap"),
    (20_901, "Music\\Stations\\UnlSpeak"),
    (20_902, "Music\\Stations\\UnlSpeak"),
    (14_093, "Music\\Modes\\Vacation"),
    (6_100, "Music\\Modes\\dt_nhood"),
    (65_536, "Music\\Stations\\EZ"),
    (65_537, "Music\\Stations\\EZExp"),
    (529, "SoundData\\TVStations\\TV_Action"),
    (530, "SoundData\\TVStations\\TV_Comedy_Cartoon"),
    (528, "SoundData\\TVStations\\TV_Romance"),
    (531, "SoundData\\TVStations\\TV_Mystery"),
    (1_586, "Music\\Stations\\CountryD"),
    (1_587, "Music\\Stations\\Rap"),
    (1_588, "Music\\Stations\\Rave"),
    (1_589, "Music\\Stations\\Disco"),
    (1_590, "Music\\Stations\\Beach"),
];

/// The hit lists and patches the stations add.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StationLists {
    /// Hit list number to its entries, in order: a station's sub lists,
    /// or a sub list's patches.
    pub hit_lists: BTreeMap<u32, Vec<u32>>,
    /// Patch number to the path of its file.
    pub patches: BTreeMap<u32, String>,
}

impl StationLists {
    /// Builds every station of [`STATIONS`] from the files listed.
    pub fn build(files: &SoundFiles) -> Self {
        // Directory key to the loose files directly in it, and to its
        // subdirectories, each in key order. (The original lists the disk,
        // so archive members are not station files.)
        let mut contents: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut subdirectories: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for entry in files.iter() {
            if !matches!(entry.source, Source::Loose(_)) {
                continue;
            }
            let path = key(&entry.path);
            let Some((directory, _)) = path.rsplit_once('\\') else {
                continue;
            };
            contents
                .entry(directory.to_owned())
                .or_default()
                .push(entry.path.clone());
            let mut child = directory;
            while let Some((parent, _)) = child.rsplit_once('\\') {
                subdirectories
                    .entry(parent.to_owned())
                    .or_default()
                    .insert(child.to_owned());
                child = parent;
            }
        }
        let mut lists = Self::default();
        let mut next = FIRST_ID;
        for (number, directory) in STATIONS {
            let directory = key(directory);
            let subs: Vec<String> = match subdirectories.get(&directory) {
                Some(subs) => subs.iter().cloned().collect(),
                None => vec![directory],
            };
            let mut station = Vec::new();
            for sub in subs {
                let Some(paths) = contents.get(&sub).filter(|p| !p.is_empty()) else {
                    continue;
                };
                let list = next;
                next += 1;
                let mut patches = Vec::new();
                for path in paths {
                    lists.patches.insert(next, path.clone());
                    patches.push(next);
                    next += 1;
                }
                lists.hit_lists.insert(list, patches);
                station.push(list);
            }
            lists.hit_lists.entry(number).or_default().extend(station);
        }
        lists
    }
}
