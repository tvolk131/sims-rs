//! The suit and texture names the original lists for Make New Character
//! (primitive 19), which picks a new person's body and head from them.
//!
//! - **Suits.** The avatar registry ([0x6f6bb4]+0x28) holds the name of
//!   every suit (appearance) that the `.cmx` and `.bcf` files of its
//!   search path define, lowercased (0x59f844, 0x59f087, 0x5950f5). The
//!   body and head lists walk it (0x409255), leaving out names that start
//!   with "suit-" (0x6c0f84).
//! - **Textures.** The file name of every `.bmp` and `.tga` on the texture
//!   search path (0x40895a, the path built at 0x414633 to 0x415056).
//!
//! Both are read here from every appearance catalog and `.bmp` and `.tga`
//! file of [`Assets`], whose sources cover both search paths. That these
//! are exactly the original's names is **unverified**:
//!
//! - `Assets` keeps one name for a file two folders hold, where the
//!   original may list it twice (in the Complete Collection, six texture
//!   names).
//! - `Assets` lists whole folders the search paths name only parts of (an
//!   expansion's folder, where the texture path names its `Skins` and
//!   `Accessories`), so a texture or suit outside the original's paths
//!   counts here. In the Complete Collection, that changes no count.
//!
//! Only whether a count is 0 changes the simulation: the draws are taken
//! either way, and which body and head they pick is presentation. The
//! order is not kept: the original's is its hash table's and its file
//! system's, and only that pick depends on it.

use crate::Assets;
use crate::people::read_suits;
use crate::pool::{ArchivePool, worker_count};
use sims_formats::avatar::read_cmx;
use std::collections::{BTreeMap, BTreeSet};

/// The suits and textures a new person's body and head are picked from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Wardrobe {
    /// Every suit name, lowercased, but those starting with "suit-".
    pub suits: BTreeSet<String>,
    /// Every texture's file name, lowercased, with its extension.
    pub textures: BTreeSet<String>,
    /// Each appearance catalog that could not be read, by file name, with
    /// the error: its suits are missing from `suits`. Or the install's
    /// person files, when they could not be listed ([`Wardrobe::unread`]).
    pub errors: BTreeMap<String, String>,
}

impl Wardrobe {
    /// No names, as the install's person files could not be listed
    /// ([`Assets::scan`] failing with `error`).
    pub fn unread(error: String) -> Self {
        Self {
            errors: BTreeMap::from([("the install's person files".to_owned(), error)]),
            ..Self::default()
        }
    }

    /// Reads the names from every avatar file (`.cmx`, `.bcf`) and texture
    /// (`.bmp`, `.tga`, 0x6c0f80, 0x6c0f7c) of `assets`, as the registry's
    /// scan loads every `.cmx` and `.bcf` (0x59f844).
    pub fn load(assets: &Assets) -> Self {
        Self::load_with_pool(assets, &ArchivePool::new())
    }

    /// Like [`Wardrobe::load`], reusing open FAR handles from `pool` and
    /// reading/parsing catalogs in parallel while merging in exact sorted
    /// name and source order.
    pub fn load_with_pool(assets: &Assets, pool: &ArchivePool) -> Self {
        let mut wardrobe = Self::default();
        let mut items: Vec<(&str, &crate::Resource)> = Vec::new();
        for name in assets.names() {
            if name.ends_with(".bmp") || name.ends_with(".tga") {
                wardrobe.textures.insert(name.to_owned());
            }
            if name.ends_with(".cmx") || name.ends_with(".bcf") {
                for resource in assets.sources(name) {
                    items.push((name, resource));
                }
            }
        }
        let decode_one =
            |&(name, resource): &(&str, &crate::Resource)| -> (BTreeSet<String>, Option<String>) {
                let mut suits = BTreeSet::new();
                if name.ends_with(".bcf") {
                    match resource.read_head_with(pool, 8) {
                        Ok(head) if head.len() == 8 && head == [0; 8] => {
                            return (suits, None);
                        }
                        Ok(_) => {}
                        Err(error) => return (suits, Some(error.to_string())),
                    }
                }
                let read = resource
                    .read_with(pool)
                    .map_err(|e| e.to_string())
                    .and_then(|bytes| {
                        if name.ends_with(".bcf") {
                            read_suits(&bytes, &mut suits).map_err(|e| e.to_string())
                        } else {
                            let catalog = read_cmx(&bytes).map_err(|e| e.to_string())?;
                            suits.extend(catalog.appearances.into_iter().map(|a| a.name));
                            Ok(())
                        }
                    });
                (suits, read.err())
            };
        let workers = worker_count(items.len());
        let results: Vec<(BTreeSet<String>, Option<String>)> = if workers <= 1 {
            items.iter().map(decode_one).collect()
        } else {
            let chunk_size = items.len().div_ceil(workers);
            std::thread::scope(|scope| {
                let handles: Vec<_> = items
                    .chunks(chunk_size)
                    .map(|chunk| {
                        scope.spawn(move || chunk.iter().map(decode_one).collect::<Vec<_>>())
                    })
                    .collect();
                let mut out = Vec::with_capacity(items.len());
                for handle in handles {
                    out.extend(handle.join().expect("wardrobe worker panicked"));
                }
                out
            })
        };
        for ((name, _), (suits, error)) in items.into_iter().zip(results) {
            if let Some(error) = error {
                wardrobe.errors.insert(name.to_owned(), error);
            }
            wardrobe.suits.extend(
                suits
                    .into_iter()
                    .map(|n| n.to_ascii_lowercase())
                    .filter(|n| !n.starts_with("suit-")),
            );
        }
        wardrobe
    }

    /// The number of bodies (`head` false) or heads a human of skin tone
    /// `skin` can wear (0x408bf5 asked for list 0 or 1, built by 0x409663
    /// or 0x40a364): each suit matching the list's pattern, paired with
    /// each texture whose name starts with the suit's matched part and the
    /// skin tone (0x4727f1, a prefix match folding A to Z to lowercase).
    ///
    /// - A body's pattern is "b", any three characters, "m" or "u" for a
    ///   male ("f" or "u" for a female), "c" for a child or "a", and any
    ///   three characters: nine in all.
    /// - A head's is "c", any three, the same sex and age letters: six in
    ///   all. A name whose second character is "t" (the costumes,
    ///   0x40a453) is no head.
    ///
    /// The original's names are single bytes to a character (Windows-1252),
    /// so the places count characters here, not UTF-8 bytes.
    pub fn count(&self, head: bool, female: bool, child: bool, skin: &str) -> usize {
        let (first, length) = if head { ('c', 6) } else { ('b', 9) };
        let sex = if female { 'f' } else { 'm' };
        let age = if child { 'c' } else { 'a' };
        let skin = skin.to_ascii_lowercase();
        self.suits
            .iter()
            .filter_map(|suit| {
                let chars: Vec<char> = suit.chars().take(length).collect();
                let matches = chars.len() == length
                    && chars[0] == first
                    && (chars[4] == sex || chars[4] == 'u')
                    && chars[5] == age
                    && !(head && chars[1] == 't');
                let part: String = chars.into_iter().collect();
                matches.then(|| format!("{part}{skin}"))
            })
            .map(|prefix| {
                self.textures
                    .iter()
                    .filter(|t| t.starts_with(&prefix))
                    .count()
            })
            .sum()
    }

    /// The number of bodies a dog (`cat` false) or cat can wear (0x408bf5
    /// asked for list 0 with gender 2..=5, built by 0x409663): each suit
    /// matching `"b"`, any three characters, and `"dog"` or `"cat"` (seven
    /// characters in all), paired with each texture whose name starts with
    /// those seven characters (no skin tone suffix, 0x40971e..0x409722 and
    /// 0x4097e5..0x4097ec).
    pub fn count_pet(&self, cat: bool) -> usize {
        let species = if cat {
            ['c', 'a', 't']
        } else {
            ['d', 'o', 'g']
        };
        self.suits
            .iter()
            .filter_map(|suit| {
                let chars: Vec<char> = suit.chars().take(7).collect();
                let matches = chars.len() == 7 && chars[0] == 'b' && chars[4..7] == species;
                matches.then(|| chars.into_iter().collect::<String>())
            })
            .map(|prefix| {
                self.textures
                    .iter()
                    .filter(|t| t.starts_with(&prefix))
                    .count()
            })
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wardrobe(suits: &[&str], textures: &[&str]) -> Wardrobe {
        Wardrobe {
            suits: suits.iter().map(|s| s.to_string()).collect(),
            textures: textures.iter().map(|s| s.to_string()).collect(),
            errors: BTreeMap::new(),
        }
    }

    #[test]
    fn a_body_pairs_each_matching_suit_with_each_texture_of_its_prefix_and_tone() {
        let w = wardrobe(
            &[
                "b001mafit_01",
                "b001mafit_02",
                "b002uafat_01",
                "b003fafit_01",
                "b004mcfit_01",
                "b005ma",
            ],
            &[
                "b001mafitlgt_01.bmp",
                "b001mafitlgt_02.bmp",
                "b001mafitdrk_01.bmp",
                "b002uafatlgt.bmp",
                "b003fafitlgt.bmp",
                "b004mcfitlgt.bmp",
            ],
        );
        // Two b001 suits with two textures each, and b002 (either sex).
        assert_eq!(w.count(false, false, false, "lgt"), 5);
        assert_eq!(w.count(false, false, false, "LGT"), 5);
        assert_eq!(w.count(false, true, false, "lgt"), 2);
        assert_eq!(w.count(false, false, true, "lgt"), 1);
        assert_eq!(w.count(false, false, false, "drk"), 2);
        assert_eq!(w.count(false, false, false, "unknown"), 0);
    }

    #[test]
    fn a_head_is_six_characters_and_no_costume() {
        let w = wardrobe(
            &["c001ma_bob", "c002fa", "ct-c003ma", "c00", "b001mafit_01"],
            &["c001malgt_bob.bmp", "c002falgt.bmp", "ct-c003malgt.bmp"],
        );
        assert_eq!(w.count(true, false, false, "lgt"), 1);
        assert_eq!(w.count(true, true, false, "lgt"), 1);
        assert_eq!(w.count(true, false, true, "lgt"), 0);
    }

    #[test]
    fn the_places_count_characters_not_bytes() {
        let w = wardrobe(
            &["b00émafit_01", "c0é1ma_bob"],
            &["b00émafitlgt.bmp", "c0é1malgt_bob.tga"],
        );
        assert_eq!(w.count(false, false, false, "lgt"), 1);
        assert_eq!(w.count(true, false, false, "lgt"), 1);
    }

    #[test]
    fn a_pet_body_matches_seven_character_species_prefix_without_skin_tone() {
        let w = wardrobe(
            &[
                "b001dog_lab",
                "b002cat_siamese",
                "b00édog_accent",
                "b001mafit_01",
                "c001dog_head",
            ],
            &[
                "b001dog_lab_01.bmp",
                "b001dog_lab_02.tga",
                "b002cat_siamese.bmp",
                "b00édog_accent.bmp",
                "b001mafitlgt.bmp",
                "c001dog_head.bmp",
            ],
        );
        assert_eq!(w.count_pet(false), 3);
        assert_eq!(w.count_pet(true), 1);
    }
}
