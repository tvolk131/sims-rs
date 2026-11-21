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
        let mut wardrobe = Self::default();
        for name in assets.names() {
            if name.ends_with(".bmp") || name.ends_with(".tga") {
                wardrobe.textures.insert(name.to_owned());
            }
            let catalog = name.ends_with(".cmx") || name.ends_with(".bcf");
            if !catalog {
                continue;
            }
            for resource in assets.sources(name) {
                // A binary file is read for its suits alone, as a file
                // that holds animations too still gives its suits
                // ([`crate::people`]'s reader).
                let mut suits = BTreeSet::new();
                let read = resource
                    .read()
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
                if let Err(error) = read {
                    wardrobe.errors.insert(name.to_owned(), error);
                }
                wardrobe.suits.extend(
                    suits
                        .into_iter()
                        .map(|n| n.to_ascii_lowercase())
                        .filter(|n| !n.starts_with("suit-")),
                );
            }
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

    /// The distinct suit names that contribute at least one entry to
    /// [`Wardrobe::count`].
    pub fn matching_suits(&self, head: bool, female: bool, child: bool, skin: &str) -> Vec<&str> {
        let (first, length) = if head { ('c', 6) } else { ('b', 9) };
        let sex = if female { 'f' } else { 'm' };
        let age = if child { 'c' } else { 'a' };
        let skin = skin.to_ascii_lowercase();
        self.suits
            .iter()
            .filter(|suit| {
                let chars: Vec<char> = suit.chars().take(length).collect();
                let matches = chars.len() == length
                    && chars[0] == first
                    && (chars[4] == sex || chars[4] == 'u')
                    && chars[5] == age
                    && !(head && chars[1] == 't');
                if !matches {
                    return false;
                }
                let part: String = chars.into_iter().collect();
                let prefix = format!("{part}{skin}");
                self.textures.iter().any(|t| t.starts_with(&prefix))
            })
            .map(String::as_str)
            .collect()
    }

    /// The distinct pet suit names that contribute at least one entry to
    /// [`Wardrobe::count_pet`].
    pub fn matching_pet_suits(&self, cat: bool) -> Vec<&str> {
        let species = if cat {
            ['c', 'a', 't']
        } else {
            ['d', 'o', 'g']
        };
        self.suits
            .iter()
            .filter(|suit| {
                let chars: Vec<char> = suit.chars().take(7).collect();
                if !(chars.len() == 7 && chars[0] == 'b' && chars[4..7] == species) {
                    return false;
                }
                let prefix: String = chars.into_iter().collect();
                self.textures.iter().any(|t| t.starts_with(&prefix))
            })
            .map(String::as_str)
            .collect()
    }

    /// Whether a `STR# 200` body (`head == false`, `0x40d0a4`) or head
    /// (`head == true`, `0x40d408`) outfit row matches an entry in the human
    /// wardrobe list built by `0x409663` / `0x40a364`.
    ///
    /// Follows `0x472ec4` → `0x5874ca` (strip `;...`, split on `,`) and
    /// `0x472f92` → `0x46f874` (ASCII case-insensitive match against
    /// `(suit, "BODY=" + stem)` or `(suit, "HEAD-HEAD=" + stem)`), including
    /// the native 64-byte fixed buffers (63 text characters + NUL at
    /// `0x472e7e`, `0x472ed2`, `0x472ee2`, `0x46f79a`).
    pub fn contains_outfit(
        &self,
        head: bool,
        female: bool,
        child: bool,
        skin: &str,
        raw_row: &str,
    ) -> bool {
        fn trunc63(s: &str) -> String {
            s.chars().take(63).collect()
        }
        let before_semi = raw_row.split(';').next().unwrap_or(raw_row);
        let Some((raw_suit, raw_tex)) = before_semi.split_once(',') else {
            return false;
        };
        let row_suit = trunc63(&raw_suit.to_ascii_lowercase());
        let row_tex = trunc63(&raw_tex.to_ascii_lowercase());
        let tag = if head { "head-head=" } else { "body=" };
        if !row_tex.starts_with(tag) {
            return false;
        }
        let (first, length) = if head { ('c', 6) } else { ('b', 9) };
        let sex = if female { 'f' } else { 'm' };
        let age = if child { 'c' } else { 'a' };
        let skin = skin.to_ascii_lowercase();
        for suit in &self.suits {
            if trunc63(suit) != row_suit {
                continue;
            }
            let chars: Vec<char> = suit.chars().take(length).collect();
            let matches = chars.len() == length
                && chars[0] == first
                && (chars[4] == sex || chars[4] == 'u')
                && chars[5] == age
                && !(head && chars[1] == 't');
            if !matches {
                continue;
            }
            let part: String = chars.into_iter().collect();
            let prefix = format!("{part}{skin}");
            for tex in &self.textures {
                if !tex.starts_with(&prefix) {
                    continue;
                }
                let Some(stem) = tex
                    .strip_suffix(".bmp")
                    .or_else(|| tex.strip_suffix(".tga"))
                else {
                    continue;
                };
                if trunc63(&format!("{tag}{stem}")) == row_tex {
                    return true;
                }
            }
        }
        false
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

    #[test]
    fn contains_outfit_matches_body_and_head_rows_like_0x472f92() {
        let w = wardrobe(
            &["b006fafat_01", "c001fa_mercedes"],
            &["b006fafatlgt_tt1.bmp", "c001falgt_shannon01.tga"],
        );
        assert!(w.contains_outfit(
            false,
            true,
            false,
            "lgt",
            "b006fafat_01,BODY=B006FAFatlgt_TT1"
        ));
        assert!(w.contains_outfit(
            true,
            true,
            false,
            "LGT",
            "c001fa_mercedes,HEAD-HEAD=C001FAlgt_Shannon01;extra"
        ));
        // Wrong tag, wrong skin tone, missing comma, or NPC suit not in wardrobe.
        assert!(!w.contains_outfit(
            true,
            true,
            false,
            "lgt",
            "c001fa_mercedes,BODY=C001FAlgt_Shannon01"
        ));
        assert!(!w.contains_outfit(
            true,
            true,
            false,
            "drk",
            "c001fa_mercedes,HEAD-HEAD=C001FAlgt_Shannon01"
        ));
        assert!(!w.contains_outfit(true, true, false, "lgt", "c001fa_mercedes"));
        assert!(!w.contains_outfit(
            true,
            true,
            false,
            "lgt",
            "c999fa_masseuse,HEAD-HEAD=c999falgt_masseuse"
        ));

        // 64-byte fixed buffers (63 text bytes + NUL at 0x472e7e / 0x472ee2):
        // body and head texture names differing only past byte 63 still match.
        let long_body_tex = format!("b006fafatlgt_{}_aaa.bmp", "x".repeat(50));
        let long_body_row = format!("b006fafat_01,BODY=b006fafatlgt_{}_bbb", "x".repeat(50));
        let long_head_tex = format!("c001falgt_{}_aaa.bmp", "y".repeat(50));
        let long_head_row = format!("c001fa_mercedes,HEAD-HEAD=c001falgt_{}_bbb", "y".repeat(50));
        let w_long = wardrobe(
            &["b006fafat_01", "c001fa_mercedes"],
            &[&long_body_tex, &long_head_tex],
        );
        assert!(w_long.contains_outfit(false, true, false, "lgt", &long_body_row));
        assert!(w_long.contains_outfit(true, true, false, "lgt", &long_head_row));
    }
}
