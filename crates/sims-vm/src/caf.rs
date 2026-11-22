//! Create-a-Family (`CAF`) character synthesis, household commit, Family Bin
//! deletion, and `.FAM` export/import backend for Complete Collection 1.91.
//!
//! All synthesized formats and invariants are cross-checked field-for-field
//! against Complete Collection 1.91's `Sims.exe` and the verified CAF oracle
//! (`caf-create-family-20261121`: `User00172.iff`, `Neighborhood.iff`, and
//! `Export/Alice_7.FAM`):
//!
//! - **Character IFF (`User%05d.iff`)**: cloned from `TemplatePerson.iff`
//!   (`0x7fd96b54`) without touching `SIMS_INSTALL` on disk; patches `OBJD 128`
//!   (`0x1c..0x20` new GUID, `0x8c..0x90` template GUID, label
//!   `user%05d - <first_name>`), localized `CTSS 2000` (`[first_name, bio]`),
//!   35-row `STR# 200` (`bodystring`, refreshed via
//!   [`sims_content::family_import::refresh_outfits`] when a wardrobe is
//!   supplied), and six 24-bit bottom-up Windows BMP portrait chunks
//!   (`BMP_ 2002..=2007`: `"faces"`, `"rel. images"`, `"newimage"`,
//!   `"speech_med"`, `"speech_large"`, `"web_image"`).
//! - **Neighbor record (`NBRS`)**: version 10 (`version_extra = Some(9)`),
//!   `person_mode = 9` ([`SAVED_REVISION`]), 256 `person_data` words with
//!   asymmetric personality (`0..=10` scaled to `0..=1000` in words
//!   `[7, 6, 3, 5, 2]`), native default interests drawn via
//!   [`crate::objects::Draws::default_traits`] (`0x459a9c`: 10 words `46..=55`
//!   with `[4, 3, 3]` low/mid/high + 5 words `[13, 14, 16, 20, 26]` with
//!   `[1, 2, 2]`), age (`27`/`9` in word 58), skin tone index (`1`/`3`/`2` in
//!   word 60), family ID (word 61), gender (`0`/`1` in word 65), and child
//!   school defaults (`-1`/`4` in words 56–57).
//! - **Household (`FAMI`) & Family Bin commit**: version 9, `house = 0`,
//!   `number = max_existing + 1`, starting funds from `Global.far FCNS #4`
//!   `"money for new family"` (default `$20,000`), `flags = 0x18`
//!   ([`CAF_INITIAL_FAMILY_FLAGS`]), 1..=8 member GUIDs with at least one adult
//!   human Sim, and 16-byte zeroed v9 tail (`secondary_house` + 6 magic words).
//!   Persisted into REDB [`NeighborhoodStore`] via a single-transaction atomic
//!   checked commit (`NeighborhoodStore::save_characters_and_neighborhood_checked`)
//!   that verifies `(instance_id, revision)` before writing all `User%05d.iff`
//!   character blobs and the updated `StoredNeighborhood` together — leaving
//!   zero partial writes on concurrent revision conflicts or `import_directory`
//!   replaces — so the household immediately appears in
//!   `StoredNeighborhood::family_bin()`, passes [`crate::lifecycle::move_in`],
//!   and boots cleanly into any vacant residential lot.
//! - **Delete Family (`delete_bin_family`)**: removes an un-housed playable
//!   Family Bin household, vacates its members' `NBRS` slots, scrubs reciprocal
//!   relationships and inventories, bumps `deletion_epoch`, and records
//!   `deleted_characters` + `deleted_at_epoch` tombstones.

use crate::objects::{Draws, PERSON_WORDS};
use crate::people::SAVED_REVISION;
use sims_content::family_import::{TEMPLATE_PERSON, refresh_outfits};
use sims_content::{Catalog, ImportWardrobe, Index, Install};
use sims_core::CrtRng;
use sims_formats::bitmap::{Image, bmp};
use sims_formats::family_file::FamilyFile;
use sims_formats::far::{
    decode_name, directory as far_directory, directory_len, encode_name, manifest_offset,
};
use sims_formats::iff::{ChunkData, Iff, write as write_iff};
use sims_formats::neighborhood::{Family, Inventory, InventoryItem, Neighbor, Relationship};
use sims_formats::strings::{Format, StringTable, decode};
use sims_formats::tuning::Constants;
use sims_sim::neighborhood_store::{
    CheckedCommitError, FAMILY_FLAG_BIT_3, NeighborhoodStore, StoredNeighborhood,
};
use sims_sim::patch_import::allocate_record;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;
use std::sync::Mutex;

/// Serializes multi-step CAF REDB writes (`commit_family`, `import_family_fam`,
/// `delete_bin_family`) within a process. REDB itself enforces exclusive file
/// locking across processes and handles, so this prevents two concurrent
/// in-process CAF callers from allocating overlapping `User%05d.iff` filenames
/// or overwriting each other's staged character blobs.
static CAF_COMMIT_LOCK: Mutex<()> = Mutex::new(());

/// `TemplatePerson.iff` object definition GUID (`0x7fd96b54`).
pub const TEMPLATE_PERSON_GUID: u32 = TEMPLATE_PERSON;

/// Native Complete Collection 1.91 starting household funds (`Global.far`
/// `FCNS #4` `"money for new family"`).
pub const DEFAULT_STARTING_FUNDS: i32 = 20_000;

/// Maximum members allowed in a household created in Create-a-Family.
pub const MAX_HOUSEHOLD_MEMBERS: usize = 8;

/// Maximum points per personality trait slider (`0..=10`).
pub const TRAIT_MAX: u8 = 10;

/// Total personality point budget in Create-a-Sim (`0..=25`).
pub const PERSONALITY_BUDGET: u8 = 25;

/// Maximum characters for a family last name or Sim first name (`sims-ui::caf`).
pub const MAX_NAME_LEN: usize = 32;

/// Maximum characters for a family or Sim biography (`sims-ui::caf`).
pub const MAX_BIO_LEN: usize = 254;

/// Default Complete Collection 1.91 language code (`1` = US English).
pub const DEFAULT_LANGUAGE: u8 = 1;

/// `FAMI.flags` bit 4 (`0x10`): user-created household flag set by CAF and
/// preserved across move-in and saves (observed on `FAMI 7` = `0x18` and
/// `FAMI 1` Newbie = `0x11`).
pub const FAMILY_FLAG_CAF_CREATED: u32 = 1 << 4;

/// Initial `FAMI.flags` written when committing a brand-new CAF household
/// (`0x18` = [`FAMILY_FLAG_BIT_3`] | [`FAMILY_FLAG_CAF_CREATED`]), verified
/// byte-for-byte against `01-after-create/Neighborhood.iff` `FAMI 7`.
pub const CAF_INITIAL_FAMILY_FLAGS: u32 = FAMILY_FLAG_BIT_3 | FAMILY_FLAG_CAF_CREATED;

/// Character file number wrap limit (`0x7d00` = `32000`, `0x42615b`).
const CHARACTER_FILE_LIMIT: i32 = 0x7d00;

/// `Global.far` `FCNS` chunk holding `"starting money for new family"`.
const NEIGHBORHOOD_TUNING_ID: u16 = 4;

/// `IFF` chunk flag bit marking converted little-endian resources (`0x10`).
const CHUNK_CONVERTED: u16 = 0x10;

/// `NBRS` / `NGBH` / `FINV` Complete Collection stream version (`73` = `0x49`).
const STREAM_VERSION_COMPLETE_COLLECTION: u32 = 73;

/// `EXPi` stream version with trailing language byte (`0x3f` = `63`).
const EXPI_VERSION: u32 = 0x3f;

/// Person data word indices written into freshly synthesized `NBRS` records.
pub mod person_word {
    pub const NICE: usize = 2;
    pub const ACTIVE: usize = 3;
    pub const PLAYFUL: usize = 5;
    pub const OUTGOING: usize = 6;
    pub const NEAT: usize = 7;
    pub const JOB: usize = 56;
    pub const JOB_LEVEL_OR_GRADE: usize = 57;
    pub const AGE: usize = 58;
    pub const SKIN: usize = 60;
    pub const FAMILY: usize = 61;
    pub const GENDER: usize = 65;
    pub const ZODIAC: usize = 70;
}

/// Child starting career (`-1` = school) and starting report card grade (`4` = C).
const CHILD_SCHOOL_CAREER: i16 = -1;
const CHILD_START_GRADE: i16 = 4;

/// Each zodiac sign's native traits in `[Outgoing, Nice, Playful, Neat, Active]`
/// order (`0x6c5bf8`, Aries = index 0 through Pisces = index 11).
const ZODIAC_PROFILES: [[f32; 5]; 12] = [
    [8.0, 3.0, 3.0, 5.0, 6.0],  // 1: Aries
    [5.0, 4.0, 8.0, 5.0, 3.0],  // 2: Taurus
    [7.0, 3.0, 3.0, 4.0, 8.0],  // 3: Gemini
    [3.0, 6.0, 4.0, 6.0, 6.0],  // 4: Cancer
    [10.0, 3.0, 4.0, 4.0, 4.0], // 5: Leo
    [2.0, 5.0, 3.0, 9.0, 6.0],  // 6: Virgo
    [8.0, 7.0, 6.0, 2.0, 2.0],  // 7: Libra
    [5.0, 3.0, 3.0, 6.0, 8.0],  // 8: Scorpio
    [3.0, 4.0, 7.0, 2.0, 9.0],  // 9: Sagittarius
    [4.0, 5.0, 8.0, 7.0, 1.0],  // 10: Capricorn
    [4.0, 6.0, 7.0, 4.0, 4.0],  // 11: Aquarius
    [3.0, 7.0, 3.0, 5.0, 7.0],  // 12: Pisces
];

/// English zodiac names indexed by `(sign - 1)` for diagnostics and testing.
const ZODIAC_NAMES: [&str; 12] = [
    "Aries",
    "Taurus",
    "Gemini",
    "Cancer",
    "Leo",
    "Virgo",
    "Libra",
    "Scorpio",
    "Sagittarius",
    "Capricorn",
    "Aquarius",
    "Pisces",
];

/// Sim age category in Create-a-Family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Age {
    Adult,
    Child,
}

impl Age {
    /// Skeleton name written to `STR# 200` row 0 (`"adult"` or `"child"`).
    #[must_use]
    pub const fn skeleton_name(self) -> &'static str {
        match self {
            Self::Adult => "adult",
            Self::Child => "child",
        }
    }

    /// Numeric age stored in `STR# 200` row 13 and `NBRS person_data[58]`.
    #[must_use]
    pub const fn age_years(self) -> i16 {
        match self {
            Self::Adult => 27,
            Self::Child => 9,
        }
    }

    /// String age stored in `STR# 200` row 13 (`"27"` or `"9"`).
    #[must_use]
    pub const fn age_string(self) -> &'static str {
        match self {
            Self::Adult => "27",
            Self::Child => "9",
        }
    }

    /// Single-letter skin appearance/texture age code (`'a'` or `'c'`).
    #[must_use]
    pub const fn code(self) -> char {
        match self {
            Self::Adult => 'a',
            Self::Child => 'c',
        }
    }
}

/// Sim gender category in Create-a-Family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Gender {
    Male,
    Female,
}

impl Gender {
    /// Profile gender string stored in `STR# 200` row 12 (`"male"` or `"female"`).
    #[must_use]
    pub const fn profile_name(self) -> &'static str {
        match self {
            Self::Male => "male",
            Self::Female => "female",
        }
    }

    /// Gender word stored in `NBRS person_data[65]` (`0` male, `1` female).
    #[must_use]
    pub const fn nbrs_value(self) -> i16 {
        match self {
            Self::Male => 0,
            Self::Female => 1,
        }
    }

    /// Single-letter skin appearance/texture gender code (`'m'` or `'f'`).
    #[must_use]
    pub const fn code(self) -> char {
        match self {
            Self::Male => 'm',
            Self::Female => 'f',
        }
    }
}

/// Sim skin-tone category in Create-a-Family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SkinTone {
    Light,
    Medium,
    Dark,
}

impl SkinTone {
    /// Three-letter skin tone code stored in `STR# 200` row 14.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Light => "lgt",
            Self::Medium => "med",
            Self::Dark => "drk",
        }
    }

    /// One-based `PersonGlobals STR# 0x2000` skin tone index stored in
    /// `NBRS person_data[60]` (`1` = `lgt`, `2` = `drk`, `3` = `med`).
    #[must_use]
    pub const fn nbrs_index(self) -> i16 {
        match self {
            Self::Light => 1,
            Self::Dark => 2,
            Self::Medium => 3,
        }
    }

    /// Parses a three-letter skin tone string (case-insensitively).
    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        if code.eq_ignore_ascii_case("lgt") {
            Some(Self::Light)
        } else if code.eq_ignore_ascii_case("med") {
            Some(Self::Medium)
        } else if code.eq_ignore_ascii_case("drk") {
            Some(Self::Dark)
        } else {
            None
        }
    }
}

/// The five Create-a-Sim personality traits (`0..=10` each, total `<= 25`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Personality {
    pub neat: u8,
    pub outgoing: u8,
    pub active: u8,
    pub playful: u8,
    pub nice: u8,
}

impl Personality {
    /// Constructs and validates a personality (`0..=10` per trait, total `<= 25`).
    pub fn new(
        neat: u8,
        outgoing: u8,
        active: u8,
        playful: u8,
        nice: u8,
    ) -> Result<Self, CafError> {
        let personality = Self {
            neat,
            outgoing,
            active,
            playful,
            nice,
        };
        personality.validate()?;
        Ok(personality)
    }

    /// Validates that each trait is in `0..=10` and the sum does not exceed 25.
    pub fn validate(&self) -> Result<(), CafError> {
        for (name, value) in [
            ("neat", self.neat),
            ("outgoing", self.outgoing),
            ("active", self.active),
            ("playful", self.playful),
            ("nice", self.nice),
        ] {
            if value > TRAIT_MAX {
                return Err(CafError::InvalidPersonality(format!(
                    "trait {name} value {value} exceeds maximum {TRAIT_MAX}"
                )));
            }
        }
        let total = self.total();
        if total > PERSONALITY_BUDGET {
            return Err(CafError::InvalidPersonality(format!(
                "personality total {total} exceeds budget {PERSONALITY_BUDGET}"
            )));
        }
        Ok(())
    }

    /// Total personality points spent (`0..=25`).
    #[must_use]
    pub const fn total(&self) -> u8 {
        self.neat + self.outgoing + self.active + self.playful + self.nice
    }

    /// Native zodiac sign (`1` = Aries .. `12` = Pisces) computed via `0x4841fc`.
    #[must_use]
    pub fn zodiac(&self) -> u8 {
        compute_zodiac(*self)
    }

    /// Maps the 0..=10 sliders to Complete Collection `NBRS` `person_data`
    /// `(word_index, value * 100)` pairs.
    #[must_use]
    pub const fn to_nbrs_words(&self) -> [(usize, i16); 5] {
        [
            (person_word::NEAT, (self.neat as i16) * 100),
            (person_word::OUTGOING, (self.outgoing as i16) * 100),
            (person_word::ACTIVE, (self.active as i16) * 100),
            (person_word::PLAYFUL, (self.playful as i16) * 100),
            (person_word::NICE, (self.nice as i16) * 100),
        ]
    }
}

/// Computes the native zodiac sign (`1` = Aries .. `12` = Pisces) for
/// `personality`, matching `Sims.exe` `0x4841fc` (`f32` squared Euclidean
/// distance against [`ZODIAC_PROFILES`], first sign wins ties).
#[must_use]
pub fn compute_zodiac(personality: Personality) -> u8 {
    let traits = [
        i16::from(personality.outgoing) * 100,
        i16::from(personality.nice) * 100,
        i16::from(personality.playful) * 100,
        i16::from(personality.neat) * 100,
        i16::from(personality.active) * 100,
    ];
    let scale = f32::from_bits(0x3c23_d70a);
    let points = traits.map(|t| f32::from(t) * scale);
    let mut best = f32::from_bits(0x7e96_7699);
    let mut sign = 1u8;
    for (index, profile) in ZODIAC_PROFILES.iter().enumerate() {
        let distance = points.iter().zip(profile).fold(0.0f32, |sum, (p, q)| {
            let d = p - q;
            d * d + sum
        });
        if distance < best {
            best = distance;
            sign = index as u8 + 1;
        }
    }
    sign
}

/// Returns the English zodiac name for sign `1..=12` (clamped to `1..=12`).
#[must_use]
pub fn zodiac_name(sign: u8) -> &'static str {
    ZODIAC_NAMES[usize::from(sign.clamp(1, 12) - 1)]
}

/// The six native `BMP_` portrait resources stored inside `User%05d.iff`
/// (`2002..=2007`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PortraitKind {
    Faces2002,
    Relationship2003,
    NewImage2004,
    SpeechMed2005,
    SpeechLarge2006,
    WebImage2007,
}

impl PortraitKind {
    /// All six portrait resources in native chunk-ID order (`2002..=2007`).
    pub const ALL: [Self; 6] = [
        Self::Faces2002,
        Self::Relationship2003,
        Self::NewImage2004,
        Self::SpeechMed2005,
        Self::SpeechLarge2006,
        Self::WebImage2007,
    ];

    /// IFF `BMP_` chunk ID (`2002..=2007`).
    #[must_use]
    pub const fn chunk_id(self) -> u16 {
        match self {
            Self::Faces2002 => 2002,
            Self::Relationship2003 => 2003,
            Self::NewImage2004 => 2004,
            Self::SpeechMed2005 => 2005,
            Self::SpeechLarge2006 => 2006,
            Self::WebImage2007 => 2007,
        }
    }

    /// Native IFF chunk label stored in `User00172.iff`.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Faces2002 => "faces",
            Self::Relationship2003 => "rel. images",
            Self::NewImage2004 => "newimage",
            Self::SpeechMed2005 => "speech_med",
            Self::SpeechLarge2006 => "speech_large",
            Self::WebImage2007 => "web_image",
        }
    }

    /// Expected `[width, height]` dimensions of this `BMP_` resource.
    #[must_use]
    pub const fn expected_size(self) -> [u32; 2] {
        match self {
            Self::Faces2002 => [105, 41],
            Self::Relationship2003 => [200, 40],
            Self::NewImage2004 => [25, 25],
            Self::SpeechMed2005 => [16, 16],
            Self::SpeechLarge2006 => [32, 32],
            Self::WebImage2007 => [45, 45],
        }
    }

    /// Native fallback background color used by [`PortraitBitmaps::placeholder`].
    #[must_use]
    const fn default_bg(self) -> [u8; 3] {
        match self {
            Self::Faces2002 => [0x00, 0x00, 0x50],
            Self::Relationship2003 => [0x40, 0x5c, 0x58],
            Self::NewImage2004 => [0x00, 0x00, 0xf8],
            Self::SpeechMed2005 | Self::SpeechLarge2006 => [0xf8, 0xfc, 0xf8],
            Self::WebImage2007 => [0xc8, 0xc4, 0xc0],
        }
    }
}

/// Complete set of six encoded 24-bit Windows BMP portraits (`BMP_ 2002..=2007`)
/// to embed into a synthesized `User%05d.iff` character file.
///
/// Hosts that render 3D Sim portraits via `sims_people::caf_preview::render_portrait`
/// pass the six resulting RGBA canvases through [`PortraitBitmaps::from_images`].
/// Fields are private so every instance is guaranteed to hold canonical
/// uncompressed 24-bit bottom-up BMPs at exact native dimensions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortraitBitmaps {
    faces_2002: Vec<u8>,
    relationship_2003: Vec<u8>,
    new_image_2004: Vec<u8>,
    speech_med_2005: Vec<u8>,
    speech_large_2006: Vec<u8>,
    web_image_2007: Vec<u8>,
}

impl PortraitBitmaps {
    /// Encodes all six RGBA portrait images (e.g. from `sims_people::caf_preview`)
    /// into uncompressed 24-bit bottom-up Windows BMP payloads matching
    /// `User00172.iff`, validating every image's dimensions.
    pub fn from_images(
        faces_2002: &Image,
        relationship_2003: &Image,
        new_image_2004: &Image,
        speech_med_2005: &Image,
        speech_large_2006: &Image,
        web_image_2007: &Image,
    ) -> Result<Self, CafError> {
        let encode_checked = |kind: PortraitKind, image: &Image| -> Result<Vec<u8>, CafError> {
            let [w, h] = kind.expected_size();
            if image.width != w || image.height != h {
                return Err(CafError::InvalidPortrait(format!(
                    "BMP_ {} ({}) expected {}x{}, got {}x{}",
                    kind.chunk_id(),
                    kind.label(),
                    w,
                    h,
                    image.width,
                    image.height
                )));
            }
            encode_bmp24(image)
        };
        Ok(Self {
            faces_2002: encode_checked(PortraitKind::Faces2002, faces_2002)?,
            relationship_2003: encode_checked(PortraitKind::Relationship2003, relationship_2003)?,
            new_image_2004: encode_checked(PortraitKind::NewImage2004, new_image_2004)?,
            speech_med_2005: encode_checked(PortraitKind::SpeechMed2005, speech_med_2005)?,
            speech_large_2006: encode_checked(PortraitKind::SpeechLarge2006, speech_large_2006)?,
            web_image_2007: encode_checked(PortraitKind::WebImage2007, web_image_2007)?,
        })
    }

    /// Validates six already-encoded Windows BMP payloads (confirming each
    /// decodes cleanly via [`sims_formats::bitmap::bmp`] and matches its native
    /// dimensions) and normalizes every payload to Complete Collection's
    /// uncompressed 24-bit bottom-up BMP layout via [`encode_bmp24`].
    pub fn from_raw_bmps(
        faces_2002: Vec<u8>,
        relationship_2003: Vec<u8>,
        new_image_2004: Vec<u8>,
        speech_med_2005: Vec<u8>,
        speech_large_2006: Vec<u8>,
        web_image_2007: Vec<u8>,
    ) -> Result<Self, CafError> {
        let normalize = |kind: PortraitKind, bytes: &[u8]| -> Result<Vec<u8>, CafError> {
            let decoded = bmp(bytes).map_err(|e| {
                CafError::InvalidPortrait(format!("BMP_ {} invalid BMP: {e}", kind.chunk_id()))
            })?;
            let [w, h] = kind.expected_size();
            if decoded.width != w || decoded.height != h {
                return Err(CafError::InvalidPortrait(format!(
                    "BMP_ {} expected {}x{}, got {}x{}",
                    kind.chunk_id(),
                    w,
                    h,
                    decoded.width,
                    decoded.height
                )));
            }
            encode_bmp24(&decoded)
        };
        Ok(Self {
            faces_2002: normalize(PortraitKind::Faces2002, &faces_2002)?,
            relationship_2003: normalize(PortraitKind::Relationship2003, &relationship_2003)?,
            new_image_2004: normalize(PortraitKind::NewImage2004, &new_image_2004)?,
            speech_med_2005: normalize(PortraitKind::SpeechMed2005, &speech_med_2005)?,
            speech_large_2006: normalize(PortraitKind::SpeechLarge2006, &speech_large_2006)?,
            web_image_2007: normalize(PortraitKind::WebImage2007, &web_image_2007)?,
        })
    }

    /// Builds a valid set of six 24-bit BMP portraits at exact native
    /// dimensions using the native per-resource background colors, for headless
    /// callers or tests without a 3D renderer.
    #[must_use]
    pub fn placeholder() -> Self {
        let make = |kind: PortraitKind| {
            let [w, h] = kind.expected_size();
            let [r, g, b] = kind.default_bg();
            let mut rgba = Vec::with_capacity((w as usize) * (h as usize) * 4);
            for _ in 0..(w * h) {
                rgba.extend_from_slice(&[r, g, b, 255]);
            }
            encode_bmp24(&Image {
                width: w,
                height: h,
                rgba,
            })
            .expect("native placeholder dimensions are valid")
        };
        Self {
            faces_2002: make(PortraitKind::Faces2002),
            relationship_2003: make(PortraitKind::Relationship2003),
            new_image_2004: make(PortraitKind::NewImage2004),
            speech_med_2005: make(PortraitKind::SpeechMed2005),
            speech_large_2006: make(PortraitKind::SpeechLarge2006),
            web_image_2007: make(PortraitKind::WebImage2007),
        }
    }

    /// Returns the encoded BMP bytes for `kind`.
    #[must_use]
    pub fn chunk_bytes(&self, kind: PortraitKind) -> &[u8] {
        match kind {
            PortraitKind::Faces2002 => &self.faces_2002,
            PortraitKind::Relationship2003 => &self.relationship_2003,
            PortraitKind::NewImage2004 => &self.new_image_2004,
            PortraitKind::SpeechMed2005 => &self.speech_med_2005,
            PortraitKind::SpeechLarge2006 => &self.speech_large_2006,
            PortraitKind::WebImage2007 => &self.web_image_2007,
        }
    }
}

/// Encodes a top-down RGBA [`Image`] into an uncompressed 24-bit bottom-up
/// Windows BMP matching Complete Collection 1.91's `BMP_ 2002..=2007` chunks
/// (`bfSize = 0`, `bfOffBits = 54`, `biSize = 40`, `biBitCount = 24`,
/// `biCompression = 0`, `biSizeImage = 0`, BGR rows padded to 4 bytes).
pub fn encode_bmp24(image: &Image) -> Result<Vec<u8>, CafError> {
    if image.width == 0 || image.height == 0 {
        return Err(CafError::InvalidPortrait(
            "portrait image dimensions must be non-zero".into(),
        ));
    }
    let w = image.width as usize;
    let h = image.height as usize;
    let expected_rgba = w
        .checked_mul(h)
        .and_then(|px| px.checked_mul(4))
        .ok_or_else(|| CafError::InvalidPortrait("portrait image size overflows".into()))?;
    if image.rgba.len() != expected_rgba {
        return Err(CafError::InvalidPortrait(format!(
            "portrait RGBA buffer len {} does not match {}x{}x4 ({expected_rgba})",
            image.rgba.len(),
            image.width,
            image.height
        )));
    }
    let width_i32 = i32::try_from(image.width)
        .map_err(|_| CafError::InvalidPortrait("portrait width exceeds i32::MAX".into()))?;
    let height_i32 = i32::try_from(image.height)
        .map_err(|_| CafError::InvalidPortrait("portrait height exceeds i32::MAX".into()))?;
    let row_bytes = w
        .checked_mul(3)
        .ok_or_else(|| CafError::InvalidPortrait("portrait row overflows".into()))?;
    let stride = row_bytes.div_ceil(4) * 4;
    let pad = stride - row_bytes;
    let total_len = 54usize
        .checked_add(
            stride
                .checked_mul(h)
                .ok_or_else(|| CafError::InvalidPortrait("portrait BMP size overflows".into()))?,
        )
        .ok_or_else(|| CafError::InvalidPortrait("portrait BMP size overflows".into()))?;

    let mut out = Vec::with_capacity(total_len);
    // 14-byte BITMAPFILEHEADER (native Sims.exe leaves bfSize = 0).
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&0u32.to_le_bytes()); // bfSize
    out.extend_from_slice(&0u16.to_le_bytes()); // bfReserved1
    out.extend_from_slice(&0u16.to_le_bytes()); // bfReserved2
    out.extend_from_slice(&54u32.to_le_bytes()); // bfOffBits
    // 40-byte BITMAPINFOHEADER.
    out.extend_from_slice(&40u32.to_le_bytes()); // biSize
    out.extend_from_slice(&width_i32.to_le_bytes()); // biWidth
    out.extend_from_slice(&height_i32.to_le_bytes()); // biHeight (positive = bottom-up)
    out.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
    out.extend_from_slice(&24u16.to_le_bytes()); // biBitCount
    out.extend_from_slice(&0u32.to_le_bytes()); // biCompression (BI_RGB)
    out.extend_from_slice(&0u32.to_le_bytes()); // biSizeImage
    out.extend_from_slice(&0i32.to_le_bytes()); // biXPelsPerMeter
    out.extend_from_slice(&0i32.to_le_bytes()); // biYPelsPerMeter
    out.extend_from_slice(&0u32.to_le_bytes()); // biClrUsed
    out.extend_from_slice(&0u32.to_le_bytes()); // biClrImportant

    // Bottom-up BGR rows with 4-byte alignment padding.
    for stored_row in 0..h {
        let src_y = h - 1 - stored_row;
        let row_start = src_y * w * 4;
        for x in 0..w {
            let p = row_start + x * 4;
            out.push(image.rgba[p + 2]);
            out.push(image.rgba[p + 1]);
            out.push(image.rgba[p]);
        }
        out.resize(out.len() + pad, 0);
    }
    Ok(out)
}

/// Input specification for one Sim created in Create-a-Sim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimSpec {
    pub first_name: String,
    pub bio: String,
    pub age: Age,
    pub gender: Gender,
    pub skin_tone: SkinTone,
    /// Everyday body appearance stem (e.g. `"b002fafit_01"`).
    pub body_appearance: String,
    /// Everyday body texture stem (e.g. `"b002fafitmed_mom02"`).
    pub body_texture: String,
    /// Head appearance stem (e.g. `"c809fa_beth"`).
    pub head_appearance: String,
    /// Head texture stem (e.g. `"c809famed_beth3"`).
    pub head_texture: String,
    pub personality: Personality,
    /// Optional pre-rendered 3D portraits (`BMP_ 2002..=2007`). When `None`,
    /// [`PortraitBitmaps::placeholder`] is embedded.
    pub portraits: Option<PortraitBitmaps>,
}

/// Input specification for a household created in Create-a-Family.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FamilySpec {
    pub family_name: String,
    pub family_bio: String,
    pub members: Vec<SimSpec>,
    /// One-based Complete Collection language code (`1` = US English).
    pub language: u8,
    /// Optional starting funds override; when `None`, uses the install's
    /// `FCNS #4` `"starting money for new family"` (default `$20,000`).
    pub starting_funds: Option<i32>,
}

impl FamilySpec {
    /// Creates a new `FamilySpec` with US English (`language = 1`) and default
    /// starting funds.
    #[must_use]
    pub fn new(
        family_name: impl Into<String>,
        family_bio: impl Into<String>,
        members: Vec<SimSpec>,
    ) -> Self {
        Self {
            family_name: family_name.into(),
            family_bio: family_bio.into(),
            members,
            language: DEFAULT_LANGUAGE,
            starting_funds: None,
        }
    }

    /// Validates household-level and per-Sim constraints matching Complete
    /// Collection 1.91 CAF rules (`1..=8` members, at least one adult human Sim,
    /// non-empty names, valid traits, valid appearance strings, valid language).
    pub fn validate(&self) -> Result<(), CafError> {
        if !(1..=20).contains(&self.language) {
            return Err(CafError::InvalidText(format!(
                "language code {} outside native 1..=20 range",
                self.language
            )));
        }
        let family_name = self.family_name.trim();
        if family_name.is_empty() {
            return Err(CafError::EmptyFamilyName);
        }
        if family_name.chars().count() > MAX_NAME_LEN {
            return Err(CafError::InvalidText(format!(
                "family name exceeds {MAX_NAME_LEN} characters"
            )));
        }
        if self.family_bio.chars().count() > MAX_BIO_LEN {
            return Err(CafError::InvalidText(format!(
                "family bio exceeds {MAX_BIO_LEN} characters"
            )));
        }
        encode_for_language(family_name, self.language, "family name")?;
        encode_for_language(&self.family_bio, self.language, "family bio")?;

        if let Some(funds) = self.starting_funds
            && funds < 0
        {
            return Err(CafError::InvalidText(format!(
                "starting funds must be non-negative, got {funds}"
            )));
        }
        if self.members.is_empty() || self.members.len() > MAX_HOUSEHOLD_MEMBERS {
            return Err(CafError::InvalidMemberCount {
                count: self.members.len(),
            });
        }
        if !self.members.iter().any(|m| m.age == Age::Adult) {
            return Err(CafError::NoAdultMember);
        }
        for (index, member) in self.members.iter().enumerate() {
            let first = member.first_name.trim();
            if first.is_empty() {
                return Err(CafError::EmptyFirstName { member: index });
            }
            if first.chars().count() > MAX_NAME_LEN {
                return Err(CafError::InvalidText(format!(
                    "member {index} first name exceeds {MAX_NAME_LEN} characters"
                )));
            }
            if member.bio.chars().count() > MAX_BIO_LEN {
                return Err(CafError::InvalidText(format!(
                    "member {index} bio exceeds {MAX_BIO_LEN} characters"
                )));
            }
            encode_for_language(first, self.language, "Sim first name")?;
            encode_for_language(&member.bio, self.language, "Sim bio")?;
            member.personality.validate()?;
            for (field, value) in [
                ("body_appearance", member.body_appearance.trim()),
                ("body_texture", member.body_texture.trim()),
                ("head_appearance", member.head_appearance.trim()),
                ("head_texture", member.head_texture.trim()),
            ] {
                if value.is_empty() {
                    return Err(CafError::InvalidAppearance(format!(
                        "member {index} {field} must not be empty"
                    )));
                }
                encode_w1252(value, field)?;
            }
        }
        Ok(())
    }
}

/// Result for one synthesized Sim in a newly created household.
#[derive(Debug, Clone)]
pub struct CreatedSim {
    pub guid: u32,
    pub neighbor_id: i16,
    pub file_number: i32,
    pub file_name: String,
    pub first_name: String,
    pub bio: String,
    /// Computed native zodiac sign (`1` = Aries .. `12` = Pisces).
    pub zodiac: u8,
    pub neighbor: Neighbor,
    pub iff_bytes: Vec<u8>,
}

/// Summary returned by [`commit_family`], [`commit_family_with_context`], and
/// [`import_family_fam`].
#[derive(Debug, Clone)]
pub struct CreatedHousehold {
    pub family_id: u16,
    pub family_name: String,
    pub family_bio: String,
    pub family: Family,
    /// Encoded `FAMs` chunk payload (`STR#` `-3` localized table).
    pub fams_chunk: Vec<u8>,
    pub members: Vec<CreatedSim>,
    pub revision: u64,
}

/// Summary returned by [`delete_bin_family`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletedFamilyReport {
    pub family_id: u16,
    pub deleted_guids: Vec<u32>,
    pub vacated_neighbor_ids: Vec<i16>,
    pub deletion_epoch: u64,
    pub revision: u64,
}

/// Errors returned by Create-a-Family backend operations.
#[derive(Debug)]
pub enum CafError {
    EmptyFamilyName,
    InvalidMemberCount {
        count: usize,
    },
    NoAdultMember,
    EmptyFirstName {
        member: usize,
    },
    InvalidPersonality(String),
    InvalidAppearance(String),
    InvalidText(String),
    InvalidPortrait(String),
    NoFreeFamilyId,
    NoFreeCharacterFile,
    NoFreeGuid,
    NoFreeNeighborSlot,
    TemplateNotFound(u32),
    TemplateInvalid(String),
    NeighborhoodNotFound(String),
    FamilyNotFound {
        neighborhood: String,
        family: u16,
    },
    FamilyNotInBin {
        family: u16,
        house: i32,
    },
    ReservedFamily {
        family: u16,
    },
    NeighborhoodMismatch {
        expected: String,
        actual: String,
    },
    StaleNeighborhood {
        neighborhood: String,
        expected_revision: u64,
        current_revision: u64,
    },
    Store(sims_sim::Error),
    Format(sims_formats::Error),
    Content(sims_content::Error),
    Io(std::io::Error),
}

impl fmt::Display for CafError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyFamilyName => write!(f, "family name must not be empty"),
            Self::InvalidMemberCount { count } => write!(
                f,
                "household must have 1..={MAX_HOUSEHOLD_MEMBERS} members, got {count}"
            ),
            Self::NoAdultMember => {
                write!(f, "household must contain at least one adult human Sim")
            }
            Self::EmptyFirstName { member } => {
                write!(
                    f,
                    "household member {member} must have a non-empty first name"
                )
            }
            Self::InvalidPersonality(msg) => write!(f, "invalid personality: {msg}"),
            Self::InvalidAppearance(msg) => write!(f, "invalid appearance: {msg}"),
            Self::InvalidText(msg) => write!(f, "invalid text: {msg}"),
            Self::InvalidPortrait(msg) => write!(f, "invalid portrait: {msg}"),
            Self::NoFreeFamilyId => write!(f, "no free positive family ID available"),
            Self::NoFreeCharacterFile => {
                write!(f, "no free User%05d.iff character file number in 0..32000")
            }
            Self::NoFreeGuid => write!(f, "failed to allocate a unique character GUID"),
            Self::NoFreeNeighborSlot => write!(f, "no free positive NBRS neighbor slot available"),
            Self::TemplateNotFound(guid) => {
                write!(
                    f,
                    "person template {guid:#010x} not found in install catalog"
                )
            }
            Self::TemplateInvalid(msg) => write!(f, "invalid TemplatePerson IFF: {msg}"),
            Self::NeighborhoodNotFound(id) => {
                write!(f, "neighborhood {id:?} not found in store")
            }
            Self::FamilyNotFound {
                neighborhood,
                family,
            } => write!(
                f,
                "family {family} not found in neighborhood {neighborhood:?}"
            ),
            Self::FamilyNotInBin { family, house } => write!(
                f,
                "family {family} lives in house {house} and must be evicted before deleting"
            ),
            Self::ReservedFamily { family } => {
                write!(
                    f,
                    "family {family} is a reserved NPC/pool family and cannot be deleted"
                )
            }
            Self::NeighborhoodMismatch { expected, actual } => write!(
                f,
                "neighborhood instance changed concurrently (expected {expected}, got {actual})"
            ),
            Self::StaleNeighborhood {
                neighborhood,
                expected_revision,
                current_revision,
            } => write!(
                f,
                "neighborhood {neighborhood:?} changed concurrently (expected revision \
                 {expected_revision}, current revision {current_revision})"
            ),
            Self::Store(error) => write!(f, "{error}"),
            Self::Format(error) => write!(f, "{error}"),
            Self::Content(error) => write!(f, "{error}"),
            Self::Io(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for CafError {}

impl From<sims_sim::Error> for CafError {
    fn from(error: sims_sim::Error) -> Self {
        Self::Store(error)
    }
}

impl From<sims_formats::Error> for CafError {
    fn from(error: sims_formats::Error) -> Self {
        Self::Format(error)
    }
}

impl From<sims_content::Error> for CafError {
    fn from(error: sims_content::Error) -> Self {
        Self::Content(error)
    }
}

impl From<std::io::Error> for CafError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// Reusable install-backed resources for Create-a-Family synthesis:
/// `TemplatePerson.iff`, all registered catalog GUIDs, optional startup
/// [`ImportWardrobe`] for Complete Collection outfit synthesis, and the
/// install's starting-funds constant (`FCNS #4`).
#[derive(Debug, Clone)]
pub struct CafContext {
    pub template_iff: Iff,
    pub catalog_guids: BTreeSet<u32>,
    pub wardrobe: Option<ImportWardrobe>,
    pub starting_funds: i32,
}

impl CafContext {
    /// Loads `TemplatePerson.iff`, catalog GUIDs, `ImportWardrobe`, and
    /// `Global.far` `FCNS #4` `"starting money for new family"` read-only from
    /// `install_root`.
    pub fn from_install(install_root: &Path) -> Result<Self, CafError> {
        let install = Install::detect(install_root)?;
        let index = Index::build(&install)?;
        let catalog = Catalog::build(&index)?;
        let definition = catalog
            .object(TEMPLATE_PERSON_GUID)
            .ok_or(CafError::TemplateNotFound(TEMPLATE_PERSON_GUID))?;
        let template_bytes = index.resources[definition.resource].read()?;
        let template_iff = Iff::parse(template_bytes)?;
        let catalog_guids: BTreeSet<u32> = catalog.objects.iter().map(|o| o.guid).collect();
        let wardrobe = Some(ImportWardrobe::load(&install)?);
        let starting_funds = read_starting_funds(&index).unwrap_or(DEFAULT_STARTING_FUNDS);
        Ok(Self {
            template_iff,
            catalog_guids,
            wardrobe,
            starting_funds,
        })
    }

    /// Builds a self-contained context for headless unit tests without an
    /// install directory on disk.
    #[must_use]
    pub fn synthetic(template_iff: Iff, catalog_guids: BTreeSet<u32>, starting_funds: i32) -> Self {
        Self {
            template_iff,
            catalog_guids,
            wardrobe: None,
            starting_funds,
        }
    }
}

fn read_starting_funds(index: &Index) -> Option<i32> {
    let (_, resource) = index.named("Global.iff").next()?;
    let bytes = resource.read().ok()?;
    let iff = Iff::parse(bytes).ok()?;
    read_starting_funds_from_iff(&iff)
}

fn read_starting_funds_from_iff(iff: &Iff) -> Option<i32> {
    let chunk = iff.get(b"FCNS", NEIGHBORHOOD_TUNING_ID)?;
    let fcns = Constants::parse(iff.data(chunk)).ok()?;
    fcns.entries
        .iter()
        .find(|entry| entry.name.eq_ignore_ascii_case("money for new family"))
        .map(|entry| entry.value as i32)
}

/// Whether `family_id` is reserved for the default/unfamilied slot (`0`) or a
/// Complete Collection NPC/Townie/Tourist/Stray pool family (`2000`,
/// `3000..=3009`, `4000`, `5000`, `5500`, `6000`).
#[must_use]
pub const fn is_reserved_pool_family(family_id: u16) -> bool {
    matches!(
        family_id,
        0 | 2000 | 3000..=3009 | 4000 | 5000 | 5500 | 6000
    )
}

/// Allocates the next available `(family_id, family_number)` pair matching
/// `Sims.exe` `0x425dea`: lowest unused positive ID in `1..=i16::MAX` (skipping
/// reserved pool IDs) and `max(existing family.number, 0) + 1`.
pub fn allocate_family_id(families: &BTreeMap<u16, Family>) -> Result<(u16, i32), CafError> {
    let id = (1..=i16::MAX as u16)
        .find(|&candidate| {
            !is_reserved_pool_family(candidate) && !families.contains_key(&candidate)
        })
        .ok_or(CafError::NoFreeFamilyId)?;
    let number = families
        .values()
        .map(|family| family.number)
        .max()
        .unwrap_or(0)
        .max(0)
        .wrapping_add(1);
    Ok((id, number))
}

/// Parses a `User%05d.iff` (or `user%05d`) character file number, if it matches.
fn parse_user_file_number(name: &str) -> Option<i32> {
    let file_name = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let lower = file_name.to_ascii_lowercase();
    let digits = lower
        .strip_prefix("user")?
        .strip_suffix(".iff")
        .unwrap_or_else(|| lower.strip_prefix("user").unwrap_or(""));
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse::<i32>().ok()
}

/// Visits every loose `.iff` and `.far`-archived member payload inside
/// `character_files`, matching how `Index::add_character_files` mounts both
/// loose files and `.far` archives from a neighborhood's `Characters` store.
fn visit_character_blobs(
    character_files: &BTreeMap<String, Vec<u8>>,
    mut visit: impl FnMut(&str, &[u8]),
) {
    for (key, bytes) in character_files {
        if key.to_ascii_lowercase().ends_with(".far") {
            let file_len = bytes.len() as u64;
            if let Ok(offset) = manifest_offset(bytes, file_len)
                && directory_len(offset, file_len).is_ok()
                && let Ok(entries) = far_directory(&bytes[offset as usize..], file_len)
            {
                for entry in entries {
                    if !entry.within_payload(offset) {
                        continue;
                    }
                    let start = entry.offset as usize;
                    let end = start.saturating_add(entry.stored_size as usize);
                    if let Some(slice) = bytes.get(start..end) {
                        visit(&entry.name, slice);
                    }
                }
            }
        } else {
            visit(key, bytes);
        }
    }
}

/// Allocates the lowest unused character file number (`User%05d.iff`) in
/// `0..32000`, avoiding every existing character file in REDB (loose or inside
/// `.far` archives), every runtime-made character, every `NBRS` user filename,
/// and every number in `reserved`.
pub fn allocate_character_file_number(
    character_files: &BTreeMap<String, Vec<u8>>,
    stored: &StoredNeighborhood,
    reserved: &BTreeSet<i32>,
) -> Result<i32, CafError> {
    let mut used = reserved.clone();
    visit_character_blobs(character_files, |name, _| {
        if let Some(n) = parse_user_file_number(name) {
            used.insert(n);
        }
    });
    for made in stored.made_characters.made.values() {
        used.insert(made.file);
    }
    for neighbor in stored.neighbors.iter().flatten() {
        if let Some(n) = parse_user_file_number(&neighbor.name) {
            used.insert(n);
        }
    }
    (0..CHARACTER_FILE_LIMIT)
        .find(|candidate| !used.contains(candidate))
        .ok_or(CafError::NoFreeCharacterFile)
}

/// Collects every GUID already in use or tombstoned across the install catalog,
/// REDB character files (loose `.iff` and `.far` archives), neighborhood
/// families, `NBRS`, runtime-made characters, and deleted/tombstoned character
/// epochs.
pub fn collect_used_guids(
    catalog_guids: &BTreeSet<u32>,
    character_files: &BTreeMap<String, Vec<u8>>,
    stored: &StoredNeighborhood,
) -> BTreeSet<u32> {
    let mut used = catalog_guids.clone();
    used.insert(0);
    used.insert(TEMPLATE_PERSON_GUID);
    visit_character_blobs(character_files, |_, bytes| {
        if let Ok(iff) = Iff::parse(bytes.to_vec()) {
            for chunk in iff.all(b"OBJD") {
                let data = iff.data(chunk);
                if let Some(raw) = data.get(0x1c..0x20) {
                    used.insert(u32::from_le_bytes(raw.try_into().unwrap()));
                }
            }
        }
    });
    for family in stored.families.values() {
        used.extend(family.members.iter().copied());
    }
    for neighbor in stored.neighbors.iter().flatten() {
        used.insert(neighbor.guid);
    }
    used.extend(stored.made_characters.made.keys().copied());
    used.extend(stored.deleted_characters.iter().copied());
    used.extend(stored.deleted_at_epoch.keys().copied());
    used
}

/// Allocates a fresh unique non-zero GUID not in `used_guids`, matching
/// `Sims.exe` `0x4725ff`'s `(high << 16) ^ crt_rand15()` structure.
pub fn allocate_unique_guid(
    used_guids: &BTreeSet<u32>,
    rng: &mut CrtRng,
    high_word_seed: u16,
) -> Result<u32, CafError> {
    let mut high = if high_word_seed == 0 {
        0x4001u16
    } else {
        high_word_seed
    };
    for _ in 0..65_536u32 {
        let low = u32::from(rng.next_u15());
        let candidate = (u32::from(high) << 16) ^ low;
        if candidate != 0 && !used_guids.contains(&candidate) {
            return Ok(candidate);
        }
        high = high.wrapping_add(1);
        if high == 0 {
            high = 1;
        }
    }
    Err(CafError::NoFreeGuid)
}

/// Encodes a Windows-1252 string without interior NUL bytes.
fn encode_w1252(text: &str, field: &str) -> Result<Vec<u8>, CafError> {
    let bytes = encode_name(text).ok_or_else(|| {
        CafError::InvalidText(format!("{field} contains characters outside Windows-1252"))
    })?;
    if bytes.contains(&0) {
        return Err(CafError::InvalidText(format!(
            "{field} must not contain NUL bytes"
        )));
    }
    Ok(bytes)
}

/// Encodes `text` for Complete Collection string-table `language` (`1..=20`),
/// verifying that decoding the emitted bytes under `language`'s code page
/// round-trips verbatim (preventing Windows-1252 bytes such as `é` from being
/// tagged with non-1252 languages like Russian `13` where they decode as `й`).
fn encode_for_language(text: &str, language: u8, field: &str) -> Result<Vec<u8>, CafError> {
    if !(1..=20).contains(&language) {
        return Err(CafError::InvalidText(format!(
            "language code {language} outside native 1..=20 range"
        )));
    }
    let bytes = encode_w1252(text, field)?;
    if decode(&bytes, language) != text {
        return Err(CafError::InvalidText(format!(
            "{field} contains characters that cannot round-trip under language {language}"
        )));
    }
    Ok(bytes)
}

/// Encodes a Complete Collection localized (`-3`) string table (`STR#` / `CTSS` /
/// `FAMs`), including the native 1-byte-per-row trailing zero slack allocated
/// by `0x46fc9f` / `0x47090e`.
fn encode_localized_strings<'a>(
    rows: impl IntoIterator<Item = (u8, &'a [u8], &'a [u8])>,
) -> Result<Vec<u8>, CafError> {
    let mut data = vec![0xfd, 0xff, 0, 0];
    let mut count = 0usize;
    for (language, text, comment) in rows {
        data.push(language);
        data.extend_from_slice(text);
        data.push(0);
        data.extend_from_slice(comment);
        data.push(0);
        count += 1;
    }
    let count_word = i16::try_from(count)
        .map_err(|_| CafError::InvalidText("string table exceeds i16::MAX entries".into()))?;
    data[2..4].copy_from_slice(&count_word.to_le_bytes());
    data.resize(data.len() + count, 0);
    Ok(data)
}

/// Encodes a `FAMs` (or `CTSS #1000`) localized string table holding the family
/// last name in `language`, matching `01-after-create/Neighborhood.iff` `FAMs 7`.
pub fn encode_fams(family_name: &str, language: u8) -> Result<Vec<u8>, CafError> {
    let raw = encode_for_language(family_name.trim(), language, "family name")?;
    encode_localized_strings([(language, raw.as_slice(), &[][..])])
}

/// Serializes a [`Family`] into a native `FAMI` chunk payload (version 9
/// `IMAF`), matching `01-after-create/Neighborhood.iff` `FAMI 7` byte-for-byte.
#[must_use]
pub fn encode_fami(family: &Family) -> Vec<u8> {
    let mut upgraded = family.clone();
    upgraded.upgrade_to_v9();
    let mut out = Vec::with_capacity(40 + upgraded.members.len() * 4 + upgraded.tail.len());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&upgraded.version.to_le_bytes());
    out.extend_from_slice(b"IMAF");
    out.extend_from_slice(&upgraded.house.to_le_bytes());
    out.extend_from_slice(&upgraded.number.to_le_bytes());
    out.extend_from_slice(&upgraded.budget.to_le_bytes());
    out.extend_from_slice(&upgraded.architecture_value.to_le_bytes());
    out.extend_from_slice(&upgraded.friends.to_le_bytes());
    out.extend_from_slice(&upgraded.flags.to_le_bytes());
    out.extend_from_slice(&(upgraded.members.len() as u32).to_le_bytes());
    for &guid in &upgraded.members {
        out.extend_from_slice(&guid.to_le_bytes());
    }
    out.extend_from_slice(&upgraded.tail);
    out
}

/// Extracts the 3-letter body shape (`"fit"`, `"fat"`, `"skn"`, or `"chd"`)
/// from a CAF everyday body appearance name (e.g. `"b002fafit_01"` -> `"fit"`).
fn body_outfit_shape(age: Age, body_appearance: &str) -> &'static str {
    match age {
        Age::Child => "chd",
        Age::Adult => match body_appearance
            .get(6..9)
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("fat") => "fat",
            Some("skn") => "skn",
            _ => "fit",
        },
    }
}

/// Builds the 35-row `STR# 200` (`bodystring`) table for a CAF Sim before
/// [`refresh_outfits`] populates the nude/underwear/hands/gloves/high-fashion
/// rows (`[15..=22]`, `[27..=29]`, `[34]`).
fn build_initial_body_strings(
    spec: &SimSpec,
    include_fallback_defaults: bool,
    use_cd_typo_sleepwear: bool,
) -> Vec<String> {
    let g = spec.gender.code();
    let tone = spec.skin_tone.code();
    let shape = body_outfit_shape(spec.age, spec.body_appearance.trim());
    let mut rows = vec![String::new(); 35];

    rows[0] = spec.age.skeleton_name().to_owned();
    rows[1] = format!(
        "{},BODY={}",
        spec.body_appearance.trim(),
        spec.body_texture.trim()
    );
    rows[2] = format!(
        "{},HEAD-HEAD={}",
        spec.head_appearance.trim(),
        spec.head_texture.trim()
    );
    rows[3] = "HEAD".to_owned();
    rows[4] = "Top".to_owned();
    rows[5] = "R_HAND".to_owned();
    rows[6] = "Palm".to_owned();
    rows[12] = spec.gender.profile_name().to_owned();
    rows[13] = spec.age.age_string().to_owned();
    rows[14] = tone.to_owned();

    // Formal (row 30), swimwear (row 31), sleepwear (row 32), winterwear (row 33),
    // high-fashion (row 34). Matches Complete Collection `User00172.iff` (adult)
    // and `User00165.iff`..`User00171.iff` (children).
    match spec.age {
        Age::Adult => {
            rows[30] = format!("f100{g}a{shape}_original,BODY=f100{g}a{shape}{tone}_original");
            rows[31] = format!("s100{g}a{shape}_original,BODY=s100{g}a{shape}{tone}_original");
            rows[32] = format!("l100{g}a{shape}_original,BODY=l100{g}a{shape}{tone}_original");
            rows[33] = format!("w100{g}a{shape}_original,BODY=w100{g}a{shape}{tone}_original");
            // Row 34 is filled by `refresh_outfits` (0x409315 high-fashion selector).
        }
        Age::Child => {
            // Complete Collection ships `L100FCChddrk_orignal.bmp` (missing the
            // second 'i' in "original") for dark-skinned female child sleepwear.
            let sleep_suffix = if use_cd_typo_sleepwear
                && spec.gender == Gender::Female
                && spec.skin_tone == SkinTone::Dark
            {
                "orignal"
            } else {
                "original"
            };
            rows[30] = "ADDED".to_owned();
            rows[31] = format!("s100{g}c{shape}_original,BODY=s100{g}c{shape}{tone}_original");
            rows[32] =
                format!("l100{g}c{shape}_original,BODY=l100{g}c{shape}{tone}_{sleep_suffix}");
            rows[33] = format!("w100{g}c{shape}_original,BODY=w100{g}c{shape}{tone}_original");
            rows[34] = "ADDED".to_owned();
        }
    }

    if include_fallback_defaults {
        let hand_g = if spec.age == Age::Child { 'u' } else { g };
        let nude_g = if spec.age == Age::Child { 'u' } else { g };
        rows[15] = format!("n{nude_g}{shape}_01,BODY=n{nude_g}{shape}{tone}_01");
        rows[16] = match spec.age {
            Age::Adult => format!("n{nude_g}{shape}_01,BODY=u{g}{shape}{tone}_lguard01"),
            Age::Child => format!("n{nude_g}{shape}_01,BODY=u{g}{shape}{tone}_undies01"),
        };
        rows[17] = format!("H{hand_g}LO,HAND=huao{tone}");
        rows[18] = format!("H{hand_g}RO,HAND=huao{tone}");
        rows[19] = format!("H{hand_g}LP,HAND=huap{tone}");
        rows[20] = format!("H{hand_g}RP,HAND=huap{tone}");
        rows[21] = format!("H{hand_g}LC,HAND=huac{tone}");
        rows[22] = format!("H{hand_g}RC,HAND=huac{tone}");
        if spec.age == Age::Adult {
            rows[27] = format!("f{g}{shape}_01,BODY=f{g}{shape}{tone}_01");
        } else {
            rows[27] = ",BODY=".to_owned();
        }
    }
    rows
}

/// Synthesizes a Complete Collection 1.91 character IFF (`User%05d.iff`) in
/// memory by cloning `template` (`TemplatePerson.iff`, GUID `0x7fd96b54`),
/// patching `OBJD 128`, `CTSS 2000`, `STR# 200` (with [`refresh_outfits`] when
/// `wardrobe` is `Some`), and embedding `BMP_ 2002..=2007`.
pub fn synthesize_character_iff(
    template: &Iff,
    spec: &SimSpec,
    guid: u32,
    file_number: i32,
    language: u8,
    wardrobe: Option<&ImportWardrobe>,
) -> Result<Vec<u8>, CafError> {
    if guid == 0 || guid == TEMPLATE_PERSON_GUID {
        return Err(CafError::NoFreeGuid);
    }
    if !(0..CHARACTER_FILE_LIMIT).contains(&file_number) {
        return Err(CafError::NoFreeCharacterFile);
    }
    if !(1..=20).contains(&language) {
        return Err(CafError::InvalidText(format!(
            "language code {language} outside native 1..=20 range"
        )));
    }
    let first_name = spec.first_name.trim();
    if first_name.is_empty() {
        return Err(CafError::EmptyFirstName { member: 0 });
    }
    let raw_name = encode_for_language(first_name, language, "Sim first name")?;
    let raw_bio = encode_for_language(&spec.bio, language, "Sim bio")?;

    // Copy every live resource from TemplatePerson except deleted slots and old portraits.
    let mut chunks = Vec::new();
    let tags: BTreeSet<[u8; 4]> = template.chunks.iter().map(|c| c.tag).collect();
    for tag in tags {
        let live = template.resource_order(&tag)?;
        for chunk in live {
            if tag == *b"XXXX" || (tag == *b"BMP_" && (2002..=2007).contains(&chunk.id)) {
                continue;
            }
            chunks.push(ChunkData {
                tag,
                id: chunk.id,
                flags: chunk.flags,
                label: chunk.label.clone(),
                data: template.data(chunk).to_vec(),
            });
        }
    }

    // Locate and patch the single default-variant person OBJD.
    let mut objds = chunks
        .iter_mut()
        .filter(|chunk| chunk.tag == *b"OBJD" && chunk.flags >> 8 == 0);
    let definition = match (objds.next(), objds.next()) {
        (Some(def), None) => def,
        _ => {
            return Err(CafError::TemplateInvalid(
                "expected exactly one default-variant OBJD in TemplatePerson".into(),
            ));
        }
    };
    if definition.data.len() < 0x90 {
        return Err(CafError::TemplateInvalid(
            "TemplatePerson OBJD shorter than 0x90 bytes".into(),
        ));
    }
    let obj_type = u16::from_le_bytes(definition.data[0x12..0x14].try_into().unwrap());
    let tpl_guid = u32::from_le_bytes(definition.data[0x1c..0x20].try_into().unwrap());
    let body_str_id = u16::from_le_bytes(definition.data[0x26..0x28].try_into().unwrap());
    let catalog_id = u16::from_le_bytes(definition.data[0x52..0x54].try_into().unwrap());
    if obj_type != 2 || tpl_guid != TEMPLATE_PERSON_GUID {
        return Err(CafError::TemplateInvalid(format!(
            "TemplatePerson OBJD type={obj_type} guid={tpl_guid:#010x} mismatch"
        )));
    }
    if body_str_id != 200 {
        return Err(CafError::TemplateInvalid(format!(
            "TemplatePerson OBJD body string table is {body_str_id}, expected 200"
        )));
    }
    definition.data[0x1c..0x20].copy_from_slice(&guid.to_le_bytes());
    definition.data[0x8c..0x90].copy_from_slice(&TEMPLATE_PERSON_GUID.to_le_bytes());
    let mut objd_label = format!("user{file_number:05} - ").into_bytes();
    objd_label.extend_from_slice(&raw_name);
    objd_label.truncate(63);
    definition.label = decode_name(&objd_label);

    // Build and replace STR# 200 (`bodystring`).
    let write_body_strings =
        |chunks: &mut Vec<ChunkData>, use_cd_typo: bool| -> Result<(), CafError> {
            let body_rows = build_initial_body_strings(spec, wardrobe.is_none(), use_cd_typo);
            let mut encoded_body_rows = Vec::with_capacity(body_rows.len());
            for row in &body_rows {
                encoded_body_rows.push(encode_w1252(row, "STR# 200 row")?);
            }
            let str200_data = encode_localized_strings(
                encoded_body_rows
                    .iter()
                    .map(|text| (DEFAULT_LANGUAGE, text.as_slice(), &[][..])),
            )?;
            upsert_chunk(
                chunks,
                ChunkData {
                    tag: *b"STR#",
                    id: 200,
                    flags: CHUNK_CONVERTED,
                    label: "bodystring".to_owned(),
                    data: str200_data,
                },
            );
            Ok(())
        };
    write_body_strings(&mut chunks, false)?;

    if let Some(wardrobe) = wardrobe
        && let Err(err) = refresh_outfits(&mut chunks, wardrobe)
    {
        if spec.age == Age::Child
            && spec.gender == Gender::Female
            && spec.skin_tone == SkinTone::Dark
        {
            write_body_strings(&mut chunks, true)?;
            refresh_outfits(&mut chunks, wardrobe)?;
        } else {
            return Err(err.into());
        }
    }

    // Build and replace CTSS (typically 2000) with [first_name, bio] in `language`.
    chunks.retain(|chunk| {
        !(chunk.tag == *b"CATS" && chunk.id == catalog_id && chunk.flags >> 8 == 0)
    });
    let ctss_data = encode_localized_strings([
        (language, raw_name.as_slice(), &[][..]),
        (language, raw_bio.as_slice(), &[][..]),
    ])?;
    upsert_chunk(
        &mut chunks,
        ChunkData {
            tag: *b"CTSS",
            id: catalog_id,
            flags: CHUNK_CONVERTED,
            label: String::new(),
            data: ctss_data,
        },
    );

    // Embed all six 24-bit Windows BMP portraits (BMP_ 2002..=2007).
    let fallback_portraits;
    let portraits = match &spec.portraits {
        Some(provided) => provided,
        None => {
            fallback_portraits = PortraitBitmaps::placeholder();
            &fallback_portraits
        }
    };
    for kind in PortraitKind::ALL {
        upsert_chunk(
            &mut chunks,
            ChunkData {
                tag: *b"BMP_",
                id: kind.chunk_id(),
                flags: CHUNK_CONVERTED,
                label: kind.label().to_owned(),
                data: portraits.chunk_bytes(kind).to_vec(),
            },
        );
    }

    Ok(write_iff(&chunks)?)
}

fn upsert_chunk(chunks: &mut Vec<ChunkData>, replacement: ChunkData) {
    if let Some(existing) = chunks.iter_mut().find(|chunk| {
        chunk.tag == replacement.tag && chunk.id == replacement.id && chunk.flags >> 8 == 0
    }) {
        *existing = replacement;
    } else {
        chunks.push(replacement);
    }
}

/// Synthesizes a Complete Collection 1.91 `NBRS` [`Neighbor`] record for a
/// freshly created CAF Sim, matching `01-after-create/Neighborhood.iff`
/// slot 464 (`user00172`):
///
/// - `version = 10`, `version_extra = Some(9)`, `person_mode = 9`
/// - 15 default interests drawn via [`Draws::default_traits`] (`0x459a9c`)
/// - 5 personality words (`[7, 6, 3, 5, 2]`) scaled `0..=10 -> 0..=1000`
/// - `age` (`[58]`), `skin` (`[60]`), `family` (`[61]`), `gender` (`[65]`)
/// - Child school career `-1` (`[56]`) and starting grade `4` (`[57]`)
/// - `zodiac` word (`[70]`) left `0` (computed on demand via `0x4841fc`, not
///   persisted in `NBRS`)
#[must_use]
pub fn synthesize_neighbor(
    spec: &SimSpec,
    id: i16,
    guid: u32,
    file_number: i32,
    family_id: u16,
    rng: &mut CrtRng,
) -> Neighbor {
    let mut person_data = vec![0i16; PERSON_WORDS];
    let mut draws = Draws {
        stream: rng,
        count: 0,
    };
    for (word, value) in draws.default_traits() {
        person_data[word] = value;
    }
    for (word, value) in spec.personality.to_nbrs_words() {
        person_data[word] = value;
    }
    person_data[person_word::AGE] = spec.age.age_years();
    person_data[person_word::SKIN] = spec.skin_tone.nbrs_index();
    person_data[person_word::FAMILY] = family_id as i16;
    person_data[person_word::GENDER] = spec.gender.nbrs_value();
    if spec.age == Age::Child {
        person_data[person_word::JOB] = CHILD_SCHOOL_CAREER;
        person_data[person_word::JOB_LEVEL_OR_GRADE] = CHILD_START_GRADE;
    }

    Neighbor {
        version: 10,
        version_extra: Some(SAVED_REVISION),
        name: format!("user{file_number:05}"),
        unknown_zero: 0,
        person_mode: SAVED_REVISION,
        person_data,
        id,
        guid,
        unknown_negative: -1,
        relationships: Vec::new(),
    }
}

/// Creates a household and all of its member character IFFs in `store` under
/// `neighborhood_id` using `install_root` read-only.
pub fn commit_family(
    install_root: &Path,
    store: &NeighborhoodStore,
    neighborhood_id: &str,
    spec: &FamilySpec,
    rng: &mut CrtRng,
) -> Result<CreatedHousehold, CafError> {
    spec.validate()?;
    let ctx = CafContext::from_install(install_root)?;
    commit_family_with_context(&ctx, store, neighborhood_id, spec, rng)
}

/// Performs a single checked neighborhood save, translating store version
/// mismatches into [`CafError`].
fn checked_save_neighborhood(
    store: &NeighborhoodStore,
    neighborhood_id: &str,
    expected_instance: u128,
    expected_revision: u64,
    state: &StoredNeighborhood,
) -> Result<(u128, u64), CafError> {
    match store.save_neighborhood_checked(
        neighborhood_id,
        expected_instance,
        expected_revision,
        state,
    )? {
        Ok(version) => Ok(version),
        Err(CheckedCommitError::InstanceMismatch { expected, current }) => {
            Err(CafError::NeighborhoodMismatch {
                expected: format!("{neighborhood_id}#{expected:032x}"),
                actual: format!("{neighborhood_id}#{current:032x}"),
            })
        }
        Err(CheckedCommitError::RevisionMismatch { expected, current }) => {
            Err(CafError::StaleNeighborhood {
                neighborhood: neighborhood_id.to_owned(),
                expected_revision: expected,
                current_revision: current,
            })
        }
    }
}

/// Persists synthesized character IFFs and `final_stored` atomically inside a
/// single REDB write transaction via
/// [`NeighborhoodStore::save_characters_and_neighborhood_checked`].
///
/// Checking `(origin_neighborhood.instance_id, origin_neighborhood.revision)`
/// inside the same write transaction that writes all `User%05d.iff` blobs and
/// the updated `StoredNeighborhood` record eliminates any partial-write window
/// against concurrent `import_directory`, `delete_neighborhood`, lot saves, or
/// crashes: on conflict, zero character files and zero neighborhood rows are
/// written.
fn persist_household_atomic(
    store: &NeighborhoodStore,
    neighborhood_id: &str,
    origin_neighborhood: &StoredNeighborhood,
    final_stored: &StoredNeighborhood,
    created_members: &[CreatedSim],
) -> Result<u64, CafError> {
    let files = created_members
        .iter()
        .map(|member| (member.file_name.as_str(), member.iff_bytes.as_slice()));
    match store.save_characters_and_neighborhood_checked(
        neighborhood_id,
        origin_neighborhood.instance_id,
        origin_neighborhood.revision,
        files,
        final_stored,
    )? {
        Ok((_, committed_revision)) => Ok(committed_revision),
        Err(CheckedCommitError::InstanceMismatch { expected, current }) => {
            Err(CafError::NeighborhoodMismatch {
                expected: format!("{neighborhood_id}#{expected:032x}"),
                actual: format!("{neighborhood_id}#{current:032x}"),
            })
        }
        Err(CheckedCommitError::RevisionMismatch { expected, current }) => {
            Err(CafError::StaleNeighborhood {
                neighborhood: neighborhood_id.to_owned(),
                expected_revision: expected,
                current_revision: current,
            })
        }
    }
}

/// Like [`commit_family`], reusing a pre-loaded [`CafContext`] (suitable for
/// repeated CAF commits or pure unit tests without `SIMS_INSTALL`).
pub fn commit_family_with_context(
    ctx: &CafContext,
    store: &NeighborhoodStore,
    neighborhood_id: &str,
    spec: &FamilySpec,
    rng: &mut CrtRng,
) -> Result<CreatedHousehold, CafError> {
    commit_family_with_context_hook(ctx, store, neighborhood_id, spec, rng, || {})
}

fn commit_family_with_context_hook(
    ctx: &CafContext,
    store: &NeighborhoodStore,
    neighborhood_id: &str,
    spec: &FamilySpec,
    rng: &mut CrtRng,
    on_after_snapshot: impl FnOnce(),
) -> Result<CreatedHousehold, CafError> {
    spec.validate()?;

    let _guard = CAF_COMMIT_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let snapshot = store
        .load_lot_snapshot(neighborhood_id, 0)?
        .ok_or_else(|| CafError::NeighborhoodNotFound(neighborhood_id.to_owned()))?;
    on_after_snapshot();
    let origin_neighborhood = snapshot.neighborhood.clone();
    let mut stored = snapshot.neighborhood;
    let origin_revision = stored.revision;
    let character_files = snapshot.character_files;

    let (family_id, family_number) = allocate_family_id(&stored.families)?;
    let family_name = spec.family_name.trim().to_owned();
    let fams_chunk = encode_fams(&family_name, spec.language)?;
    let starting_funds = spec.starting_funds.unwrap_or(ctx.starting_funds);

    let mut used_guids = collect_used_guids(&ctx.catalog_guids, &character_files, &stored);
    let mut reserved_files = BTreeSet::new();
    let mut created_members = Vec::with_capacity(spec.members.len());
    let mut member_guids = Vec::with_capacity(spec.members.len());

    // Synthesize every character IFF and NBRS record in memory before writing
    // anything to REDB so validation or synthesis errors leave the store untouched.
    let base_high = ((origin_revision as u16) ^ family_id.wrapping_mul(0x0101)).max(1);
    for (idx, member_spec) in spec.members.iter().enumerate() {
        let file_number =
            allocate_character_file_number(&character_files, &stored, &reserved_files)?;
        reserved_files.insert(file_number);

        let high_seed = base_high.wrapping_add((idx as u16).wrapping_mul(0x13));
        let guid = allocate_unique_guid(&used_guids, rng, high_seed)?;
        used_guids.insert(guid);

        let iff_bytes = synthesize_character_iff(
            &ctx.template_iff,
            member_spec,
            guid,
            file_number,
            spec.language,
            ctx.wardrobe.as_ref(),
        )?;

        let placeholder_neighbor =
            synthesize_neighbor(member_spec, 0, guid, file_number, family_id, rng);
        let neighbor_id = allocate_record(&mut stored.neighbors, placeholder_neighbor)
            .map_err(|_| CafError::NoFreeNeighborSlot)?;
        let neighbor = stored.neighbors[usize::from((neighbor_id - 1) as u16)]
            .clone()
            .expect("allocated slot is occupied");

        stored.deleted_characters.remove(&guid);

        member_guids.push(guid);
        created_members.push(CreatedSim {
            guid,
            neighbor_id,
            file_number,
            file_name: format!("User{file_number:05}.iff"),
            first_name: member_spec.first_name.trim().to_owned(),
            bio: member_spec.bio.clone(),
            zodiac: member_spec.personality.zodiac(),
            neighbor,
            iff_bytes,
        });
    }

    let family = Family {
        version: 9,
        house: 0,
        number: family_number,
        budget: starting_funds,
        architecture_value: 0,
        friends: 0,
        flags: CAF_INITIAL_FAMILY_FLAGS,
        members: member_guids,
        tail: vec![0; 16],
    };
    stored.families.insert(family_id, family.clone());

    let committed_revision = persist_household_atomic(
        store,
        neighborhood_id,
        &origin_neighborhood,
        &stored,
        &created_members,
    )?;

    Ok(CreatedHousehold {
        family_id,
        family_name,
        family_bio: spec.family_bio.clone(),
        family,
        fams_chunk,
        members: created_members,
        revision: committed_revision,
    })
}

/// Deletes an un-housed playable Family Bin household (`house == 0`,
/// `secondary_house() == 0`, not a reserved pool family) from `store`:
///
/// - Removes the `FAMI` entry.
/// - For every member GUID not referenced by any remaining household:
///   - Vacates its `NBRS` slot(s) in place (`None`), preserving all other
///     neighbor IDs.
///   - Scrubs reciprocal relationships in all other `NBRS` entries and removes
///     its `NGBH` inventory entry.
///   - Removes any runtime-made entry in `made_characters`.
///   - Advances `deletion_epoch` and records `deleted_characters` +
///     `deleted_at_epoch` tombstones so reloading any lot never resurrects the
///     deleted Sim even while character `.iff` blobs remain in REDB.
pub fn delete_bin_family(
    store: &NeighborhoodStore,
    neighborhood_id: &str,
    family_id: u16,
) -> Result<DeletedFamilyReport, CafError> {
    if is_reserved_pool_family(family_id) {
        return Err(CafError::ReservedFamily { family: family_id });
    }

    let _guard = CAF_COMMIT_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let mut stored = store
        .load_neighborhood(neighborhood_id)?
        .ok_or_else(|| CafError::NeighborhoodNotFound(neighborhood_id.to_owned()))?;
    let origin_instance_id = stored.instance_id;
    let origin_revision = stored.revision;

    let family = stored
        .families
        .get(&family_id)
        .ok_or_else(|| CafError::FamilyNotFound {
            neighborhood: neighborhood_id.to_owned(),
            family: family_id,
        })?;
    if family.house != 0 || family.secondary_house() != 0 {
        let house = if family.house != 0 {
            family.house
        } else {
            family.secondary_house()
        };
        return Err(CafError::FamilyNotInBin {
            family: family_id,
            house,
        });
    }

    let removed_family = stored
        .families
        .remove(&family_id)
        .expect("verified present above");
    let still_referenced: BTreeSet<u32> = stored
        .families
        .values()
        .flat_map(|f| f.members.iter().copied())
        .collect();

    let mut deleted_guids = Vec::new();
    let mut deleted_guid_set = BTreeSet::new();
    for guid in removed_family.members {
        if guid != 0 && !still_referenced.contains(&guid) && deleted_guid_set.insert(guid) {
            deleted_guids.push(guid);
        }
    }

    let mut vacated_neighbor_ids = Vec::new();
    let mut vacated_id_set = BTreeSet::new();
    if !deleted_guids.is_empty() {
        stored.deletion_epoch = stored.deletion_epoch.saturating_add(1);
        let current_epoch = stored.deletion_epoch;

        for slot in &mut stored.neighbors {
            if let Some(neighbor) = slot.as_ref()
                && deleted_guid_set.contains(&neighbor.guid)
            {
                vacated_neighbor_ids.push(neighbor.id);
                vacated_id_set.insert(i32::from(neighbor.id));
                *slot = None;
            }
        }
        // Scrub reciprocal relationships targeting any deleted neighbor ID.
        if !vacated_id_set.is_empty() {
            for neighbor in stored.neighbors.iter_mut().flatten() {
                neighbor
                    .relationships
                    .retain(|row| !vacated_id_set.contains(&row.target));
            }
            stored.inventories.retain(|inv| {
                inv.as_ref()
                    .is_none_or(|entry| !vacated_id_set.contains(&i32::from(entry.neighbor)))
            });
        }
        for &guid in &deleted_guids {
            stored.made_characters.made.remove(&guid);
            stored.deleted_characters.insert(guid);
            stored.deleted_at_epoch.insert(guid, current_epoch);
        }
    }

    // `save_neighborhood_checked` atomically purges the removed family's photo
    // album and advances its household lifetime epoch inside the same REDB
    // write transaction.
    let (_, committed_revision) = checked_save_neighborhood(
        store,
        neighborhood_id,
        origin_instance_id,
        origin_revision,
        &stored,
    )?;

    Ok(DeletedFamilyReport {
        family_id,
        deleted_guids,
        vacated_neighbor_ids,
        deletion_epoch: stored.deletion_epoch,
        revision: committed_revision,
    })
}

/// Serializes `NBRS` (version 73 `SRBN`) in Complete Collection 1.91 layout so
/// [`Neighbors::parse`] round-trips it cleanly.
fn encode_nbrs(neighbors: &[Option<Neighbor>]) -> Result<Vec<u8>, CafError> {
    let mut out = Vec::new();
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&STREAM_VERSION_COMPLETE_COLLECTION.to_le_bytes());
    out.extend_from_slice(b"SRBN");
    out.extend_from_slice(&(neighbors.len() as u32).to_le_bytes());
    for slot in neighbors {
        let Some(n) = slot else {
            out.extend_from_slice(&0i32.to_le_bytes());
            continue;
        };
        out.extend_from_slice(&1i32.to_le_bytes());
        out.extend_from_slice(&n.version.to_le_bytes());
        if n.version == 10 {
            out.extend_from_slice(&n.version_extra.unwrap_or(SAVED_REVISION).to_le_bytes());
        }
        let raw_name = encode_w1252(&n.name, "neighbor name")?;
        out.extend_from_slice(&raw_name);
        out.push(0);
        if out.len() & 1 != 0 {
            out.push(0);
        }
        out.extend_from_slice(&n.unknown_zero.to_le_bytes());
        out.extend_from_slice(&n.person_mode.to_le_bytes());
        if n.person_mode > 0 {
            let expected = if n.version == 4 { 80 } else { PERSON_WORDS };
            for i in 0..expected {
                let word = n.person_data.get(i).copied().unwrap_or(0);
                out.extend_from_slice(&word.to_le_bytes());
            }
        }
        out.extend_from_slice(&n.id.to_le_bytes());
        out.extend_from_slice(&n.guid.to_le_bytes());
        out.extend_from_slice(&n.unknown_negative.to_le_bytes());
        out.extend_from_slice(&(n.relationships.len() as u32).to_le_bytes());
        for rel in &n.relationships {
            out.extend_from_slice(&1u32.to_le_bytes());
            out.extend_from_slice(&rel.target.to_le_bytes());
            out.extend_from_slice(&(rel.values.len() as u32).to_le_bytes());
            for &val in &rel.values {
                out.extend_from_slice(&val.to_le_bytes());
            }
        }
    }
    Ok(out)
}

/// Serializes `FINV #2` (version 73 `VNIF`) holding the exported family's
/// neighbor inventories, matching `Export/Alice_7.FAM`.
fn encode_finv(member_ids: &[u16], inventories: &[Option<Inventory>]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&STREAM_VERSION_COMPLETE_COLLECTION.to_le_bytes());
    out.extend_from_slice(b"VNIF");
    out.extend_from_slice(&(member_ids.len() as u32).to_le_bytes());
    for &mid in member_ids {
        let mid_i16 = mid as i16;
        let existing = inventories
            .iter()
            .flatten()
            .find(|inv| inv.neighbor == mid_i16);
        out.extend_from_slice(&1i32.to_le_bytes());
        out.extend_from_slice(&mid_i16.to_le_bytes());
        if let Some(inv) = existing {
            out.extend_from_slice(&(inv.items.len() as u32).to_le_bytes());
            for item in &inv.items {
                out.extend_from_slice(&item.kind.to_le_bytes());
                out.extend_from_slice(&item.guid.to_le_bytes());
                out.extend_from_slice(&item.count.to_le_bytes());
            }
        } else {
            out.extend_from_slice(&0u32.to_le_bytes());
        }
    }
    out
}

/// Parses `FINV #2` (version 73 `VNIF`) holding exported family neighbor
/// inventories, matching `Export/Alice_7.FAM` and `Neighborhood::parse`.
fn parse_finv(bytes: &[u8]) -> Result<Vec<Option<Inventory>>, CafError> {
    let mut pos = 0usize;
    let take = |pos: &mut usize, len: usize| -> Result<&[u8], CafError> {
        let end = pos
            .checked_add(len)
            .ok_or_else(|| CafError::TemplateInvalid("FINV offset overflow".into()))?;
        let slice = bytes
            .get(*pos..end)
            .ok_or_else(|| CafError::TemplateInvalid("truncated FINV #2 chunk".into()))?;
        *pos = end;
        Ok(slice)
    };
    let read_u16 = |pos: &mut usize| -> Result<u16, CafError> {
        Ok(u16::from_le_bytes(take(pos, 2)?.try_into().unwrap()))
    };
    let read_i16 = |pos: &mut usize| -> Result<i16, CafError> {
        Ok(i16::from_le_bytes(take(pos, 2)?.try_into().unwrap()))
    };
    let read_u32 = |pos: &mut usize| -> Result<u32, CafError> {
        Ok(u32::from_le_bytes(take(pos, 4)?.try_into().unwrap()))
    };
    let read_i32 = |pos: &mut usize| -> Result<i32, CafError> {
        Ok(i32::from_le_bytes(take(pos, 4)?.try_into().unwrap()))
    };

    let _mode = read_u32(&mut pos)?;
    let _version = read_u32(&mut pos)?;
    if take(&mut pos, 4)? != b"VNIF" {
        return Err(CafError::TemplateInvalid(
            "FINV #2 chunk missing VNIF tag".into(),
        ));
    }
    let count = read_u32(&mut pos)? as usize;
    if count.saturating_mul(4) > bytes.len().saturating_sub(pos) {
        return Err(CafError::TemplateInvalid(
            "FINV #2 entry count exceeds chunk length".into(),
        ));
    }
    let mut inventories = Vec::with_capacity(count);
    let mut seen_neighbors = BTreeSet::new();
    for _ in 0..count {
        match read_i32(&mut pos)? {
            0 => inventories.push(None),
            1 => {
                let neighbor = read_i16(&mut pos)?;
                let item_count = read_u32(&mut pos)? as usize;
                if item_count.saturating_mul(10) > bytes.len().saturating_sub(pos) {
                    return Err(CafError::TemplateInvalid(
                        "FINV #2 item count exceeds chunk length".into(),
                    ));
                }
                if !seen_neighbors.insert(neighbor) {
                    return Err(CafError::TemplateInvalid(format!(
                        "FINV #2 contains duplicate inventory row for neighbor {neighbor}"
                    )));
                }
                let mut items = Vec::with_capacity(item_count);
                for _ in 0..item_count {
                    let kind = read_i32(&mut pos)?;
                    let guid = read_u32(&mut pos)?;
                    let item_qty = read_u16(&mut pos)?;
                    items.push(InventoryItem {
                        kind,
                        guid,
                        count: item_qty,
                    });
                }
                inventories.push(Some(Inventory { neighbor, items }));
            }
            other => {
                return Err(CafError::TemplateInvalid(format!(
                    "FINV #2 unknown presence flag {other}"
                )));
            }
        }
    }
    if pos != bytes.len() {
        return Err(CafError::TemplateInvalid(format!(
            "FINV #2 has {} unexpected trailing bytes",
            bytes.len() - pos
        )));
    }
    Ok(inventories)
}

/// Revision-gated `NBRS` person fields matching `sims_content::family_import`
/// (`0x423827`).
const IMPORT_PERSON_FIELDS: &[(i32, &[usize])] = &[
    (
        1,
        &[
            2, 3, 4, 5, 6, 7, 9, 10, 11, 12, 16, 20, 15, 26, 17, 18, 46, 47, 48, 49, 50, 51, 52,
            53, 13, 14, 58, 60, 61,
        ],
    ),
    (2, &[56, 57, 63]),
    (3, &[65]),
    (4, &[67, 68]),
    (5, &[69]),
    (6, &[74]),
    (7, &[54, 55, 75, 76]),
    (8, &[80, 81]),
    (9, &[82]),
    (10, &[83, 84, 85, 86]),
];

/// Allocates the destination `(family_id, family_number)` for an imported
/// `.FAM` archive matching `sims_content::family_import::family_numbers`
/// (`0x425dea`, `0x42913f`): verifies a free non-reserved family ID exists,
/// retains `exported_id` when it is positive, non-reserved, and unoccupied,
/// and otherwise falls back to the lowest free non-reserved ID.
fn allocate_imported_family_id(
    families: &BTreeMap<u16, Family>,
    exported_id: u16,
) -> Result<(u16, i32), CafError> {
    let (free_id, number) = allocate_family_id(families)?;
    if exported_id == 0 || exported_id > i16::MAX as u16 {
        return Err(CafError::TemplateInvalid(format!(
            "invalid exported family ID {exported_id}"
        )));
    }
    let id = if !is_reserved_pool_family(exported_id) && !families.contains_key(&exported_id) {
        exported_id
    } else {
        free_id
    };
    Ok((id, number))
}

/// Allocates a destination `NBRS` slot for an imported `.FAM` member matching
/// `sims_content::family_import::allocate_neighbor` (`0x425884`): tries the
/// exported one-based slot first (extending with `None` if past the end), and
/// if occupied pops the last vacant slot or appends a new slot.
fn allocate_imported_neighbor_slot(
    neighbors: &mut Vec<Option<Neighbor>>,
    exported_id: u16,
    placeholder: Neighbor,
) -> Result<i16, CafError> {
    if exported_id == 0 || exported_id > i16::MAX as u16 {
        return Err(CafError::TemplateInvalid(format!(
            "invalid exported neighbor ID {exported_id}"
        )));
    }
    let requested = usize::from(exported_id);
    let slot = if requested > neighbors.len() {
        neighbors.resize(requested, None);
        requested - 1
    } else if neighbors[requested - 1].is_none() {
        requested - 1
    } else if let Some(vacancy) = neighbors.iter().rposition(Option::is_none) {
        vacancy
    } else {
        neighbors.push(None);
        neighbors.len() - 1
    };
    let id = i16::try_from(slot + 1)
        .ok()
        .filter(|&n| n > 0)
        .ok_or(CafError::NoFreeNeighborSlot)?;
    let mut record = placeholder;
    record.id = id;
    neighbors[slot] = Some(record);
    Ok(id)
}

/// Serializes `EXPi #0` (version `0x3f` `iPXE`), matching `Export/Alice_7.FAM`.
fn encode_expi(family_id: u16, member_ids: &[u16], language: u8) -> Vec<u8> {
    let mut out = Vec::with_capacity(31);
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&EXPI_VERSION.to_le_bytes());
    out.extend_from_slice(b"iPXE");
    out.extend_from_slice(&family_id.to_le_bytes());
    let mut slots = [0u16; MAX_HOUSEHOLD_MEMBERS];
    for (slot, &mid) in slots.iter_mut().zip(member_ids.iter()) {
        *slot = mid;
    }
    for mid in slots {
        out.extend_from_slice(&mid.to_le_bytes());
    }
    out.push(language);
    out
}

/// Exports a household from `stored` + `character_files` into a Complete
/// Collection 1.91 `.FAM` IFF container matching `Export/Alice_7.FAM`:
/// `FAMI`, `FAMs`, `CTSS #1000`, `EXPi #0`, per-member `uChr`, `CTSS`,
/// `CTSS + 2000` (localized copy), `BMP_` (`newimage` 2004), `NBRS #1`, and
/// optionally `FINV #2` (`include_finv = true` matches `Alice_7.FAM` chunk-for-chunk;
/// `include_finv = false` also passes `sims_content::family_import::import`).
pub fn export_family_fam(
    stored: &StoredNeighborhood,
    character_files: &BTreeMap<String, Vec<u8>>,
    family_id: u16,
    family_name: &str,
    language: u8,
    include_finv: bool,
) -> Result<Vec<u8>, CafError> {
    let family = stored
        .families
        .get(&family_id)
        .ok_or_else(|| CafError::FamilyNotFound {
            neighborhood: stored.name.clone(),
            family: family_id,
        })?;
    if family.members.is_empty() || family.members.len() > MAX_HOUSEHOLD_MEMBERS {
        return Err(CafError::InvalidMemberCount {
            count: family.members.len(),
        });
    }
    let fams = encode_fams(family_name, language)?;

    // Index character IFFs by OBJD GUID, scanning `.far` archives before loose
    // `.iff` files so loose character files take precedence over shadowed FAR
    // archive members (matching `Index::add_character_files`).
    let mut char_iffs_by_guid: BTreeMap<u32, Iff> = BTreeMap::new();
    let mut index_iff = |bytes: &[u8]| {
        if let Ok(iff) = Iff::parse(bytes.to_vec()) {
            for chunk in iff.all(b"OBJD") {
                if let Some(raw) = iff.data(chunk).get(0x1c..0x20) {
                    let guid = u32::from_le_bytes(raw.try_into().unwrap());
                    char_iffs_by_guid.insert(guid, iff.clone());
                }
            }
        }
    };
    for (key, bytes) in character_files {
        if key.to_ascii_lowercase().ends_with(".far") {
            let file_len = bytes.len() as u64;
            if let Ok(offset) = manifest_offset(bytes, file_len)
                && directory_len(offset, file_len).is_ok()
                && let Ok(entries) = far_directory(&bytes[offset as usize..], file_len)
            {
                for entry in entries {
                    if !entry.within_payload(offset) {
                        continue;
                    }
                    let start = entry.offset as usize;
                    let end = start.saturating_add(entry.stored_size as usize);
                    if let Some(slice) = bytes.get(start..end) {
                        index_iff(slice);
                    }
                }
            }
        }
    }
    for (key, bytes) in character_files {
        if !key.to_ascii_lowercase().ends_with(".far") {
            index_iff(bytes);
        }
    }

    let mut used_ctss_ids = BTreeSet::from([1000u16]);
    let mut member_ids = Vec::with_capacity(family.members.len());
    let mut member_chunks = Vec::new();
    for &guid in &family.members {
        let (slot_idx, neighbor) = stored
            .neighbors
            .iter()
            .enumerate()
            .filter_map(|(idx, slot)| slot.as_ref().map(|n| (idx, n)))
            .find(|(_, n)| n.guid == guid)
            .ok_or_else(|| {
                CafError::TemplateInvalid(format!(
                    "family {family_id} member GUID {guid:#010x} has no NBRS record"
                ))
            })?;
        let mid = u16::try_from(slot_idx + 1)
            .ok()
            .filter(|&id| id > 0 && id <= i16::MAX as u16 && (id as i16) == neighbor.id)
            .ok_or(CafError::NoFreeNeighborSlot)?;
        let localized_id = mid.checked_add(2000).ok_or_else(|| {
            CafError::TemplateInvalid(format!(
                "family {family_id} member slot {mid} overflows localized CTSS ID"
            ))
        })?;
        if !used_ctss_ids.insert(mid) || !used_ctss_ids.insert(localized_id) {
            return Err(CafError::TemplateInvalid(format!(
                "family {family_id} member slot {mid} collides with another CTSS chunk ID \
                 (1000 or member+2000)"
            )));
        }
        member_ids.push(mid);

        let iff = char_iffs_by_guid.get(&guid).ok_or_else(|| {
            CafError::TemplateInvalid(format!(
                "family {family_id} member GUID {guid:#010x} has no character IFF in store"
            ))
        })?;
        let str200 = iff.get(b"STR#", 200).ok_or_else(|| {
            CafError::TemplateInvalid(format!(
                "character {guid:#010x} is missing STR# 200 (bodystring)"
            ))
        })?;
        member_chunks.push(ChunkData {
            tag: *b"uChr",
            id: mid,
            flags: CHUNK_CONVERTED,
            label: "bodystring".to_owned(),
            data: iff.data(str200).to_vec(),
        });

        let ctss_chunk = iff.get(b"CTSS", 2000);
        let (ctss_bytes, name_bytes, bio_bytes, name_text, bio_text) =
            if let Some(chunk) = ctss_chunk {
                let raw = iff.data(chunk).to_vec();
                let parsed = StringTable::parse(&raw)?;
                let name = parsed
                    .entry(0, language)
                    .map(|e| e.raw_text.clone())
                    .unwrap_or_default();
                let bio = parsed
                    .entry(1, language)
                    .map(|e| e.raw_text.clone())
                    .unwrap_or_default();
                let name_s = decode(&name, language);
                let bio_s = decode(&bio, language);
                (raw, name, bio, name_s, bio_s)
            } else {
                let empty = encode_localized_strings([
                    (language, &[][..], &[][..]),
                    (language, &[][..], &[][..]),
                ])?;
                (empty, Vec::new(), Vec::new(), String::new(), String::new())
            };
        member_chunks.push(ChunkData {
            tag: *b"CTSS",
            id: mid,
            flags: CHUNK_CONVERTED,
            label: String::new(),
            data: ctss_bytes,
        });

        // Localized CTSS at `mid + 2000`: include every native language 1..=20
        // whose code page preserves `name_text` and `bio_text` verbatim (all 20
        // languages for ASCII text, matching `Alice_7.FAM` CTSS 2464; active
        // language + compatible Western languages for non-ASCII 1252 text).
        let mut all_langs = Vec::with_capacity(40);
        for lang in 1u8..=20 {
            if lang == language
                || (decode(&name_bytes, lang) == name_text && decode(&bio_bytes, lang) == bio_text)
            {
                all_langs.push((lang, name_bytes.as_slice(), &[][..]));
                all_langs.push((lang, bio_bytes.as_slice(), &[][..]));
            }
        }
        let ctss_2000 = encode_localized_strings(all_langs)?;
        member_chunks.push(ChunkData {
            tag: *b"CTSS",
            id: localized_id,
            flags: CHUNK_CONVERTED,
            label: String::new(),
            data: ctss_2000,
        });

        if let Some(bmp2004) = iff.get(b"BMP_", 2004) {
            member_chunks.push(ChunkData {
                tag: *b"BMP_",
                id: mid,
                flags: CHUNK_CONVERTED,
                label: String::new(),
                data: iff.data(bmp2004).to_vec(),
            });
        }
    }

    // Export clears neighborhood-local CAF/move-in flags (keeping only bit 0),
    // matching `Export/Alice_7.FAM` (`flags == 0`) and `family_import.rs`.
    let mut exported_family = family.clone();
    exported_family.flags &= 1;
    let mut chunks = vec![
        ChunkData {
            tag: *b"FAMI",
            id: family_id,
            flags: CHUNK_CONVERTED,
            label: String::new(),
            data: encode_fami(&exported_family),
        },
        ChunkData {
            tag: *b"FAMs",
            id: family_id,
            flags: CHUNK_CONVERTED,
            label: String::new(),
            data: fams.clone(),
        },
        ChunkData {
            tag: *b"CTSS",
            id: 1000,
            flags: CHUNK_CONVERTED,
            label: String::new(),
            data: fams,
        },
        ChunkData {
            tag: *b"EXPi",
            id: 0,
            flags: CHUNK_CONVERTED,
            label: String::new(),
            data: encode_expi(family_id, &member_ids, language),
        },
    ];
    chunks.extend(member_chunks);
    chunks.push(ChunkData {
        tag: *b"NBRS",
        id: 1,
        flags: CHUNK_CONVERTED,
        label: String::new(),
        data: encode_nbrs(&stored.neighbors)?,
    });
    if include_finv {
        chunks.push(ChunkData {
            tag: *b"FINV",
            id: 2,
            flags: CHUNK_CONVERTED,
            label: String::new(),
            data: encode_finv(&member_ids, &stored.inventories),
        });
    }

    let bytes = write_iff(&chunks)?;
    // Self-check that the exported .FAM parses cleanly.
    let _ = FamilyFile::parse(&bytes)?;
    Ok(bytes)
}

/// Imports a Complete Collection 1.91 Family Bin `.FAM` archive (such as
/// `Export/Alice_7.FAM` or bytes produced by [`export_family_fam`]) directly
/// into `store` under `neighborhood_id` as a new Family Bin household with
/// fresh unique GUIDs, `User%05d.iff` character files, `NBRS` records, and
/// remapped `FINV #2` inventories.
pub fn import_family_fam(
    ctx: &CafContext,
    store: &NeighborhoodStore,
    neighborhood_id: &str,
    fam_bytes: &[u8],
    language: u8,
    rng: &mut CrtRng,
) -> Result<CreatedHousehold, CafError> {
    if !(1..=20).contains(&language) {
        return Err(CafError::InvalidText(format!(
            "language code {language} outside native 1..=20 range"
        )));
    }
    let parsed = FamilyFile::parse(fam_bytes)?;
    if parsed.family.house != 0 || parsed.family.secondary_house() != 0 {
        let house = if parsed.family.house != 0 {
            parsed.family.house
        } else {
            parsed.family.secondary_house()
        };
        return Err(CafError::FamilyNotInBin {
            family: parsed.export_info.family_id,
            house,
        });
    }
    let family_name = parsed
        .iff
        .get(b"CTSS", 1000)
        .and_then(|c| StringTable::parse(parsed.iff.data(c)).ok())
        .and_then(|t| t.get(0, language).map(str::to_owned))
        .or_else(|| parsed.name.get(0, language).map(str::to_owned))
        .unwrap_or_default();
    if family_name.trim().is_empty() {
        return Err(CafError::EmptyFamilyName);
    }

    let member_chunk_ids: Vec<u16> = parsed
        .export_info
        .member_ids
        .into_iter()
        .take_while(|&id| id != 0)
        .collect();
    if member_chunk_ids.is_empty() || member_chunk_ids.len() > MAX_HOUSEHOLD_MEMBERS {
        return Err(CafError::InvalidMemberCount {
            count: member_chunk_ids.len(),
        });
    }
    let mut seen_exported_ids = BTreeSet::new();
    for &exported_id in &member_chunk_ids {
        if exported_id > i16::MAX as u16 || !seen_exported_ids.insert(exported_id) {
            return Err(CafError::TemplateInvalid(format!(
                "invalid or duplicate EXPi member ID {exported_id}"
            )));
        }
    }

    // Parse and validate optional FINV #2 up front before acquiring any locks or
    // writing anything to REDB.
    let exported_inventories = match parsed.iff.get(b"FINV", 2) {
        Some(chunk) => parse_finv(parsed.iff.data(chunk))?,
        None => Vec::new(),
    };

    let _guard = CAF_COMMIT_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let snapshot = store
        .load_lot_snapshot(neighborhood_id, 0)?
        .ok_or_else(|| CafError::NeighborhoodNotFound(neighborhood_id.to_owned()))?;
    let origin_neighborhood = snapshot.neighborhood.clone();
    let mut stored = snapshot.neighborhood;
    let origin_revision = stored.revision;
    let character_files = snapshot.character_files;

    let (family_id, family_number) =
        allocate_imported_family_id(&stored.families, parsed.export_info.family_id)?;
    let fams_chunk = encode_fams(&family_name, language)?;
    let mut used_guids = collect_used_guids(&ctx.catalog_guids, &character_files, &stored);
    let mut reserved_files = BTreeSet::new();

    // First pass: parse all exported members and allocate IDs/GUIDs/files.
    struct ExportedMemberData {
        exported_id: u16,
        first_name: String,
        bio: String,
        body_strings: Vec<String>,
        portrait_2004: Option<Vec<u8>>,
        source_neighbor: Neighbor,
        file_number: i32,
        guid: u32,
        neighbor_id: i16,
    }

    let mut staged = Vec::with_capacity(member_chunk_ids.len());
    let base_high = ((origin_revision as u16) ^ family_id.wrapping_mul(0x0101)).max(1);
    for (idx, &exported_id) in member_chunk_ids.iter().enumerate() {
        let member = parsed.members.get(&exported_id).ok_or_else(|| {
            CafError::TemplateInvalid(format!("FAM member {exported_id} missing uChr"))
        })?;
        let source_neighbor = parsed
            .neighbors
            .entries
            .get(usize::from(exported_id).saturating_sub(1))
            .and_then(Option::as_ref)
            .ok_or_else(|| {
                CafError::TemplateInvalid(format!("FAM member {exported_id} missing NBRS record"))
            })?
            .clone();
        if source_neighbor.person_mode < 0 {
            return Err(CafError::TemplateInvalid(format!(
                "FAM member {exported_id} has negative person revision {}",
                source_neighbor.person_mode
            )));
        }

        let (first_name, bio) = extract_fam_member_text(member, language)?;
        let body_strings: Vec<String> = member
            .body_strings
            .entries
            .iter()
            .filter(|e| member.body_strings.format != Format::Localized || e.language == language)
            .map(|e| e.text.clone())
            .collect();
        let body_strings = if body_strings.is_empty() {
            member
                .body_strings
                .entries
                .iter()
                .filter(|e| e.language == DEFAULT_LANGUAGE)
                .map(|e| e.text.clone())
                .collect()
        } else {
            body_strings
        };
        if let Some(gender) = body_strings.get(12)
            && (gender.to_ascii_lowercase().contains("dog")
                || gender.to_ascii_lowercase().contains("cat"))
        {
            return Err(CafError::InvalidAppearance(format!(
                "FAM member {exported_id} is a pet ({gender:?}); CAF import supports human Sims"
            )));
        }

        let file_number =
            allocate_character_file_number(&character_files, &stored, &reserved_files)?;
        reserved_files.insert(file_number);
        let guid = allocate_unique_guid(
            &used_guids,
            rng,
            base_high.wrapping_add((idx as u16).wrapping_mul(0x13)),
        )?;
        used_guids.insert(guid);

        let placeholder = Neighbor {
            version: 10,
            version_extra: Some(SAVED_REVISION),
            name: format!("user{file_number:05}"),
            unknown_zero: 0,
            person_mode: source_neighbor.person_mode,
            person_data: vec![0; PERSON_WORDS],
            id: 0,
            guid,
            unknown_negative: -1,
            relationships: Vec::new(),
        };
        let neighbor_id =
            allocate_imported_neighbor_slot(&mut stored.neighbors, exported_id, placeholder)?;

        staged.push(ExportedMemberData {
            exported_id,
            first_name,
            bio,
            body_strings,
            portrait_2004: member.portrait.clone(),
            source_neighbor,
            file_number,
            guid,
            neighbor_id,
        });
    }

    // Second pass: remap intra-family relationships (first row per exported
    // target in EXPi order, matching `family_import.rs::copy_neighbor`),
    // transfer revision-gated person_data fields, and synthesize character IFFs.
    let member_id_pairs: Vec<(u16, i16)> = staged
        .iter()
        .map(|m| (m.exported_id, m.neighbor_id))
        .collect();
    let id_map: BTreeMap<i16, i16> = member_id_pairs
        .iter()
        .map(|&(exported_id, new_id)| (exported_id as i16, new_id))
        .collect();

    // Validate and remap FINV #2 inventories before mutating REDB.
    let mut remapped_inventories = Vec::new();
    for inv in exported_inventories.into_iter().flatten() {
        if let Some(&new_neighbor) = id_map.get(&inv.neighbor) {
            if !inv.items.is_empty() {
                remapped_inventories.push(Inventory {
                    neighbor: new_neighbor,
                    items: inv.items,
                });
            }
        } else if !inv.items.is_empty() {
            return Err(CafError::TemplateInvalid(format!(
                "FINV #2 contains non-empty inventory for non-member neighbor {}",
                inv.neighbor
            )));
        }
    }

    let mut created_members = Vec::with_capacity(staged.len());
    let mut member_guids = Vec::with_capacity(staged.len());
    for m in staged {
        let mut person_data = vec![0i16; PERSON_WORDS];
        for &(_, indices) in IMPORT_PERSON_FIELDS
            .iter()
            .take_while(|(since, _)| *since <= m.source_neighbor.person_mode)
        {
            for &index in indices {
                person_data[index] = m
                    .source_neighbor
                    .person_data
                    .get(index)
                    .copied()
                    .unwrap_or(0);
            }
        }
        person_data[person_word::FAMILY] = family_id as i16;

        let relationships: Vec<Relationship> = member_id_pairs
            .iter()
            .filter_map(|&(target_exported, target_new)| {
                let row = m
                    .source_neighbor
                    .relationships
                    .iter()
                    .find(|row| row.target == i32::from(target_exported))?;
                (!row.values.is_empty()).then(|| Relationship {
                    target: i32::from(target_new),
                    values: row.values.clone(),
                })
            })
            .collect();

        let neighbor = Neighbor {
            version: 10,
            version_extra: Some(SAVED_REVISION),
            name: format!("user{:05}", m.file_number),
            unknown_zero: 0,
            person_mode: m.source_neighbor.person_mode,
            person_data: person_data.clone(),
            id: m.neighbor_id,
            guid: m.guid,
            unknown_negative: -1,
            relationships,
        };
        stored.neighbors[usize::from((m.neighbor_id - 1) as u16)] = Some(neighbor.clone());
        stored.deleted_characters.remove(&m.guid);

        let iff_bytes = synthesize_imported_member_iff(
            &ctx.template_iff,
            &m.first_name,
            &m.bio,
            &m.body_strings,
            m.portrait_2004.as_deref(),
            m.guid,
            m.file_number,
            language,
            ctx.wardrobe.as_ref(),
        )?;

        let personality = Personality {
            neat: ((person_data[person_word::NEAT].clamp(0, 1000)) / 100) as u8,
            outgoing: ((person_data[person_word::OUTGOING].clamp(0, 1000)) / 100) as u8,
            active: ((person_data[person_word::ACTIVE].clamp(0, 1000)) / 100) as u8,
            playful: ((person_data[person_word::PLAYFUL].clamp(0, 1000)) / 100) as u8,
            nice: ((person_data[person_word::NICE].clamp(0, 1000)) / 100) as u8,
        };

        member_guids.push(m.guid);
        created_members.push(CreatedSim {
            guid: m.guid,
            neighbor_id: m.neighbor_id,
            file_number: m.file_number,
            file_name: format!("User{:05}.iff", m.file_number),
            first_name: m.first_name,
            bio: m.bio,
            zodiac: personality.zodiac(),
            neighbor,
            iff_bytes,
        });
    }

    for inv in remapped_inventories {
        if let Some(existing) = stored
            .inventories
            .iter_mut()
            .flatten()
            .find(|entry| entry.neighbor == inv.neighbor)
        {
            existing.items = inv.items;
        } else {
            stored.inventories.push(Some(inv));
        }
    }

    let family = Family {
        version: 9,
        house: 0,
        number: family_number,
        budget: parsed.family.budget,
        architecture_value: parsed.family.architecture_value,
        friends: 0,
        flags: parsed.family.flags & 1,
        members: member_guids,
        tail: vec![0; 16],
    };
    stored.families.insert(family_id, family.clone());

    let committed_revision = persist_household_atomic(
        store,
        neighborhood_id,
        &origin_neighborhood,
        &stored,
        &created_members,
    )?;

    Ok(CreatedHousehold {
        family_id,
        family_name: family_name.trim().to_owned(),
        family_bio: String::new(),
        family,
        fams_chunk,
        members: created_members,
        revision: committed_revision,
    })
}

fn extract_fam_member_text(
    member: &sims_formats::family_file::FamilyMember,
    language: u8,
) -> Result<(String, String), CafError> {
    let raw = member.localized_ctss.as_deref().or(member.ctss.as_deref());
    let Some(bytes) = raw else {
        return Ok((String::new(), String::new()));
    };
    let table = StringTable::parse(bytes)?;
    let name = table
        .entry(0, language)
        .map(|e| decode(&e.raw_text, language))
        .unwrap_or_default();
    let bio = table
        .entry(1, language)
        .map(|e| decode(&e.raw_text, language))
        .unwrap_or_default();
    Ok((name, bio))
}

#[allow(clippy::too_many_arguments)]
fn synthesize_imported_member_iff(
    template: &Iff,
    first_name: &str,
    bio: &str,
    body_strings: &[String],
    portrait_2004: Option<&[u8]>,
    guid: u32,
    file_number: i32,
    language: u8,
    wardrobe: Option<&ImportWardrobe>,
) -> Result<Vec<u8>, CafError> {
    let raw_name = encode_for_language(first_name, language, "Sim first name")?;
    let raw_bio = encode_for_language(bio, language, "Sim bio")?;

    let mut chunks = Vec::new();
    let tags: BTreeSet<[u8; 4]> = template.chunks.iter().map(|c| c.tag).collect();
    for tag in tags {
        for chunk in template.resource_order(&tag)? {
            if tag == *b"XXXX" || (tag == *b"BMP_" && (2002..=2007).contains(&chunk.id)) {
                continue;
            }
            chunks.push(ChunkData {
                tag,
                id: chunk.id,
                flags: chunk.flags,
                label: chunk.label.clone(),
                data: template.data(chunk).to_vec(),
            });
        }
    }

    let mut objds = chunks
        .iter_mut()
        .filter(|chunk| chunk.tag == *b"OBJD" && chunk.flags >> 8 == 0);
    let definition = match (objds.next(), objds.next()) {
        (Some(def), None) => def,
        _ => {
            return Err(CafError::TemplateInvalid(
                "expected exactly one default-variant OBJD in TemplatePerson".into(),
            ));
        }
    };
    if definition.data.len() < 0x90 {
        return Err(CafError::TemplateInvalid(
            "TemplatePerson OBJD shorter than 0x90 bytes".into(),
        ));
    }
    let obj_type = u16::from_le_bytes(definition.data[0x12..0x14].try_into().unwrap());
    let tpl_guid = u32::from_le_bytes(definition.data[0x1c..0x20].try_into().unwrap());
    let body_str_id = u16::from_le_bytes(definition.data[0x26..0x28].try_into().unwrap());
    let catalog_id = u16::from_le_bytes(definition.data[0x52..0x54].try_into().unwrap());
    if obj_type != 2 || tpl_guid != TEMPLATE_PERSON_GUID {
        return Err(CafError::TemplateInvalid(format!(
            "TemplatePerson OBJD type={obj_type} guid={tpl_guid:#010x} mismatch"
        )));
    }
    if body_str_id != 200 {
        return Err(CafError::TemplateInvalid(format!(
            "TemplatePerson OBJD body string table is {body_str_id}, expected 200"
        )));
    }
    definition.data[0x1c..0x20].copy_from_slice(&guid.to_le_bytes());
    definition.data[0x8c..0x90].copy_from_slice(&TEMPLATE_PERSON_GUID.to_le_bytes());
    let mut objd_label = format!("user{file_number:05} - ").into_bytes();
    objd_label.extend_from_slice(&raw_name);
    objd_label.truncate(63);
    definition.label = decode_name(&objd_label);

    let mut encoded_rows = Vec::with_capacity(body_strings.len());
    for row in body_strings {
        encoded_rows.push(encode_w1252(row, "imported uChr row")?);
    }
    let str200_data = encode_localized_strings(
        encoded_rows
            .iter()
            .map(|text| (DEFAULT_LANGUAGE, text.as_slice(), &[][..])),
    )?;
    upsert_chunk(
        &mut chunks,
        ChunkData {
            tag: *b"STR#",
            id: 200,
            flags: CHUNK_CONVERTED,
            label: "bodystring".to_owned(),
            data: str200_data,
        },
    );
    if let Some(wardrobe) = wardrobe {
        refresh_outfits(&mut chunks, wardrobe)?;
    }

    chunks.retain(|chunk| {
        !(chunk.tag == *b"CATS" && chunk.id == catalog_id && chunk.flags >> 8 == 0)
    });
    let ctss_data = encode_localized_strings([
        (language, raw_name.as_slice(), &[][..]),
        (language, raw_bio.as_slice(), &[][..]),
    ])?;
    upsert_chunk(
        &mut chunks,
        ChunkData {
            tag: *b"CTSS",
            id: catalog_id,
            flags: CHUNK_CONVERTED,
            label: String::new(),
            data: ctss_data,
        },
    );

    let mut portraits = PortraitBitmaps::placeholder();
    if let Some(raw_2004) = portrait_2004
        && let Ok(img) = bmp(raw_2004)
        && [img.width, img.height] == PortraitKind::NewImage2004.expected_size()
        && let Ok(normalized) = encode_bmp24(&img)
    {
        portraits.new_image_2004 = normalized;
    }
    for kind in PortraitKind::ALL {
        upsert_chunk(
            &mut chunks,
            ChunkData {
                tag: *b"BMP_",
                id: kind.chunk_id(),
                flags: CHUNK_CONVERTED,
                label: kind.label().to_owned(),
                data: portraits.chunk_bytes(kind).to_vec(),
            },
        );
    }

    Ok(write_iff(&chunks)?)
}

/// Ensures `Neighbors` imported from `sims_formats` stays in sync with `encode_nbrs`.
#[cfg(test)]
mod tests {
    use super::*;
    use sims_formats::neighborhood::Neighbors;

    fn synthetic_template_iff() -> Iff {
        let mut objd = vec![0u8; 216];
        objd[0..4].copy_from_slice(&138u32.to_le_bytes());
        objd[0x12..0x14].copy_from_slice(&2u16.to_le_bytes());
        objd[0x1c..0x20].copy_from_slice(&TEMPLATE_PERSON_GUID.to_le_bytes());
        objd[0x26..0x28].copy_from_slice(&200u16.to_le_bytes());
        objd[0x52..0x54].copy_from_slice(&2000u16.to_le_bytes());
        let bytes = write_iff(&[
            ChunkData {
                tag: *b"OBJD",
                id: 128,
                flags: CHUNK_CONVERTED,
                label: "TemplatePerson".to_owned(),
                data: objd,
            },
            ChunkData {
                tag: *b"STR#",
                id: 200,
                flags: CHUNK_CONVERTED,
                label: "bodystring".to_owned(),
                data: encode_localized_strings([(1, &b"adult"[..], &[][..])]).unwrap(),
            },
        ])
        .unwrap();
        Iff::parse(bytes).unwrap()
    }

    #[test]
    fn zodiac_matches_alice_oracle_and_all_twelve_profiles() {
        // Alice oracle: Neat=7, Outgoing=5, Active=3, Playful=4, Nice=6 -> Cancer (4).
        let alice = Personality::new(7, 5, 3, 4, 6).unwrap();
        assert_eq!(alice.total(), 25);
        assert_eq!(alice.zodiac(), 4);
        assert_eq!(zodiac_name(alice.zodiac()), "Cancer");

        // Each canonical zodiac profile maps back to its own sign 1..=12.
        for (idx, p) in ZODIAC_PROFILES.iter().enumerate() {
            let pers = Personality {
                outgoing: p[0] as u8,
                nice: p[1] as u8,
                playful: p[2] as u8,
                neat: p[3] as u8,
                active: p[4] as u8,
            };
            assert_eq!(pers.zodiac(), (idx as u8) + 1);
        }
    }

    #[test]
    fn encode_bmp24_round_trips_dimensions_and_pixels() {
        let mut img = Image {
            width: 3,
            height: 2,
            rgba: vec![
                10, 20, 30, 255, 40, 50, 60, 255, 70, 80, 90, 255, // top row
                11, 22, 33, 255, 44, 55, 66, 255, 77, 88, 99, 255, // bottom row
            ],
        };
        let encoded = encode_bmp24(&img).unwrap();
        // 54 header + 2 rows * ceil(9/4)*4 (12 bytes) = 78 bytes.
        assert_eq!(encoded.len(), 78);
        assert_eq!(&encoded[0..2], b"BM");
        assert_eq!(u32::from_le_bytes(encoded[2..6].try_into().unwrap()), 0);
        assert_eq!(u32::from_le_bytes(encoded[10..14].try_into().unwrap()), 54);
        let decoded = bmp(&encoded).unwrap();
        assert_eq!(decoded, img);
        img.rgba.pop();
        assert!(matches!(
            encode_bmp24(&img),
            Err(CafError::InvalidPortrait(_))
        ));
    }

    #[test]
    fn encode_nbrs_round_trips_via_formats_parser() {
        let spec = SimSpec {
            first_name: "Alice".into(),
            bio: "Bio".into(),
            age: Age::Adult,
            gender: Gender::Female,
            skin_tone: SkinTone::Medium,
            body_appearance: "b002fafit_01".into(),
            body_texture: "b002fafitmed_mom02".into(),
            head_appearance: "c809fa_beth".into(),
            head_texture: "c809famed_beth3".into(),
            personality: Personality::new(7, 5, 3, 4, 6).unwrap(),
            portraits: None,
        };
        let mut rng = CrtRng::from_state(12345);
        let neighbor = synthesize_neighbor(&spec, 2, 0x1234_5678, 172, 7, &mut rng);
        let raw = encode_nbrs(&[None, Some(neighbor.clone())]).unwrap();
        let parsed = Neighbors::parse(&raw).unwrap();
        assert_eq!(parsed.version, STREAM_VERSION_COMPLETE_COLLECTION);
        assert_eq!(parsed.entries.len(), 2);
        assert!(parsed.entries[0].is_none());
        assert_eq!(parsed.entries[1].as_ref(), Some(&neighbor));
    }

    #[test]
    fn synthetic_character_iff_patches_objd_ctss_str200_and_portraits() {
        let template = synthetic_template_iff();
        let spec = SimSpec {
            first_name: "Alice".into(),
            bio: "Track 4 Create-a-Family oracle Sim.".into(),
            age: Age::Adult,
            gender: Gender::Female,
            skin_tone: SkinTone::Medium,
            body_appearance: "b002fafit_01".into(),
            body_texture: "b002fafitmed_mom02".into(),
            head_appearance: "c809fa_beth".into(),
            head_texture: "c809famed_beth3".into(),
            personality: Personality::new(7, 5, 3, 4, 6).unwrap(),
            portraits: None,
        };
        let bytes = synthesize_character_iff(&template, &spec, 0xe305_0120, 172, 1, None).unwrap();
        let iff = Iff::parse(bytes).unwrap();
        let objd = iff.get(b"OBJD", 128).unwrap();
        assert_eq!(objd.label, "user00172 - Alice");
        let data = iff.data(objd);
        assert_eq!(
            u32::from_le_bytes(data[0x1c..0x20].try_into().unwrap()),
            0xe305_0120
        );
        assert_eq!(
            u32::from_le_bytes(data[0x8c..0x90].try_into().unwrap()),
            TEMPLATE_PERSON_GUID
        );
        let ctss = StringTable::parse(iff.data(iff.get(b"CTSS", 2000).unwrap())).unwrap();
        assert_eq!(ctss.get(0, 1), Some("Alice"));
        assert_eq!(ctss.get(1, 1), Some("Track 4 Create-a-Family oracle Sim."));
        for kind in PortraitKind::ALL {
            let chunk = iff.get(b"BMP_", kind.chunk_id()).unwrap();
            assert_eq!(chunk.label, kind.label());
            let decoded = bmp(iff.data(chunk)).unwrap();
            assert_eq!([decoded.width, decoded.height], kind.expected_size());
        }
    }

    #[test]
    fn fcns_reads_custom_money_for_new_family_tuning() {
        // Minimal FCNS version 1 payload containing `"money for new family" = 35000.0`
        // (NUL-terminated strings aligned to even stream offsets).
        let mut fcns = Vec::new();
        fcns.extend_from_slice(&0u32.to_le_bytes());
        fcns.extend_from_slice(&1u32.to_le_bytes());
        fcns.extend_from_slice(b"SNCF");
        fcns.extend_from_slice(&1u32.to_le_bytes());
        fcns.extend_from_slice(b"money for new family\0\0");
        fcns.extend_from_slice(&35_000.0f32.to_le_bytes());
        fcns.extend_from_slice(b"\0\0"); // empty comment, aligned
        let iff = Iff::parse(
            write_iff(&[ChunkData {
                tag: *b"FCNS",
                id: NEIGHBORHOOD_TUNING_ID,
                flags: CHUNK_CONVERTED,
                label: "Neighborhood Tuning".to_owned(),
                data: fcns,
            }])
            .unwrap(),
        )
        .unwrap();
        assert_eq!(read_starting_funds_from_iff(&iff), Some(35_000));
    }

    #[test]
    fn collect_used_guids_and_file_numbers_scan_far_archives() {
        let tpl = synthetic_template_iff();
        let spec = SimSpec {
            first_name: "Archived".into(),
            bio: String::new(),
            age: Age::Adult,
            gender: Gender::Female,
            skin_tone: SkinTone::Medium,
            body_appearance: "b002fafit_01".into(),
            body_texture: "b002fafitmed_mom02".into(),
            head_appearance: "c809fa_beth".into(),
            head_texture: "c809famed_beth3".into(),
            personality: Personality::new(5, 5, 5, 5, 5).unwrap(),
            portraits: None,
        };
        let archived_guid = 0x55aa_1234u32;
        let iff_bytes = synthesize_character_iff(&tpl, &spec, archived_guid, 7, 1, None).unwrap();

        // Build a minimal valid FAR 1a archive containing `User00007.iff`.
        let member_name = b"User00007.iff";
        let payload_offset = 16u32;
        let manifest_off = payload_offset + iff_bytes.len() as u32;
        let mut far = Vec::new();
        far.extend_from_slice(b"FAR!byAZ");
        far.extend_from_slice(&1u32.to_le_bytes());
        far.extend_from_slice(&manifest_off.to_le_bytes());
        far.extend_from_slice(&iff_bytes);
        far.extend_from_slice(&1u32.to_le_bytes());
        far.extend_from_slice(&(iff_bytes.len() as u32).to_le_bytes());
        far.extend_from_slice(&(iff_bytes.len() as u32).to_le_bytes());
        far.extend_from_slice(&payload_offset.to_le_bytes());
        far.extend_from_slice(&(member_name.len() as u32).to_le_bytes());
        far.extend_from_slice(member_name);

        let files = BTreeMap::from([("bundle.far".to_owned(), far)]);
        let empty = StoredNeighborhood {
            name: "FarTest".into(),
            families: BTreeMap::new(),
            neighbors: Vec::new(),
            inventories: Vec::new(),
            data: [0; 16],
            options: None,
            type_attributes: None,
            zoning: BTreeMap::new(),
            lot_rows: Vec::new(),
            made_characters: sims_sim::characters::Characters::default(),
            deleted_characters: BTreeSet::new(),
            deletion_epoch: 0,
            deleted_at_epoch: BTreeMap::new(),
            instance_id: 1,
            revision: 0,
        };
        let used = collect_used_guids(&BTreeSet::new(), &files, &empty);
        assert!(used.contains(&archived_guid));
        let reserved: BTreeSet<i32> = (0..7).collect();
        assert_eq!(
            allocate_character_file_number(&files, &empty, &reserved).unwrap(),
            8
        );
    }

    #[test]
    fn from_raw_bmps_normalizes_top_down_bmp_to_canonical_24bit_bottom_up() {
        let make_top_down = |kind: PortraitKind| -> Vec<u8> {
            let [w, h] = kind.expected_size();
            let mut img = Image {
                width: w,
                height: h,
                rgba: vec![0; (w * h * 4) as usize],
            };
            for y in 0..h {
                for x in 0..w {
                    let p = ((y * w + x) * 4) as usize;
                    img.rgba[p] = (x & 0xff) as u8;
                    img.rgba[p + 1] = (y & 0xff) as u8;
                    img.rgba[p + 2] = 0x55;
                    img.rgba[p + 3] = 255;
                }
            }
            let mut canonical = encode_bmp24(&img).unwrap();
            // Flip to top-down (negative biHeight) and set non-zero bfSize.
            let total = canonical.len() as u32;
            canonical[2..6].copy_from_slice(&total.to_le_bytes());
            canonical[22..26].copy_from_slice(&(-(h as i32)).to_le_bytes());
            let row_bytes = (w as usize) * 3;
            let stride = row_bytes.div_ceil(4) * 4;
            for y in 0..(h as usize / 2) {
                let a = 54 + y * stride;
                let b = 54 + (h as usize - 1 - y) * stride;
                for i in 0..stride {
                    canonical.swap(a + i, b + i);
                }
            }
            canonical
        };
        let normalized = PortraitBitmaps::from_raw_bmps(
            make_top_down(PortraitKind::Faces2002),
            make_top_down(PortraitKind::Relationship2003),
            make_top_down(PortraitKind::NewImage2004),
            make_top_down(PortraitKind::SpeechMed2005),
            make_top_down(PortraitKind::SpeechLarge2006),
            make_top_down(PortraitKind::WebImage2007),
        )
        .unwrap();
        for kind in PortraitKind::ALL {
            let bytes = normalized.chunk_bytes(kind);
            // Normalized header must have bfSize = 0, positive biHeight, 24-bit BI_RGB.
            assert_eq!(u32::from_le_bytes(bytes[2..6].try_into().unwrap()), 0);
            assert!(i32::from_le_bytes(bytes[22..26].try_into().unwrap()) > 0);
            assert_eq!(u16::from_le_bytes(bytes[28..30].try_into().unwrap()), 24);
        }
    }

    #[test]
    fn language_round_trip_rejects_mismatched_code_page_and_accepts_western() {
        // 'é' round-trips in French (3) and US English (1), but would become 'й' in Russian (13).
        assert!(encode_fams("René", 1).is_ok());
        assert!(encode_fams("René", 3).is_ok());
        assert!(matches!(
            encode_fams("René", 13),
            Err(CafError::InvalidText(_))
        ));
        // Pure ASCII round-trips in Russian (13).
        assert!(encode_fams("Alice", 13).is_ok());
    }

    #[test]
    fn export_family_fam_rejects_colliding_ctss_chunk_ids() {
        let tpl = synthetic_template_iff();
        let spec = SimSpec {
            first_name: "Collider".into(),
            bio: String::new(),
            age: Age::Adult,
            gender: Gender::Female,
            skin_tone: SkinTone::Light,
            body_appearance: "b002fafit_01".into(),
            body_texture: "b002fafitlgt_mom02".into(),
            head_appearance: "c809fa_beth".into(),
            head_texture: "c809falgt_beth3".into(),
            personality: Personality::new(5, 5, 5, 5, 5).unwrap(),
            portraits: None,
        };
        let mut rng = CrtRng::from_state(99);
        let guid = 0x7777_0001u32;
        let iff_bytes = synthesize_character_iff(&tpl, &spec, guid, 1, 1, None).unwrap();
        let mut neighbors = vec![None; 1000];
        neighbors[999] = Some(synthesize_neighbor(&spec, 1000, guid, 1, 1, &mut rng));
        let mut families = BTreeMap::new();
        families.insert(
            1,
            Family {
                version: 9,
                house: 0,
                number: 1,
                budget: 20_000,
                architecture_value: 0,
                friends: 0,
                flags: CAF_INITIAL_FAMILY_FLAGS,
                members: vec![guid],
                tail: vec![0; 16],
            },
        );
        let stored = StoredNeighborhood {
            name: "Collide".into(),
            families,
            neighbors,
            inventories: Vec::new(),
            data: [0; 16],
            options: None,
            type_attributes: None,
            zoning: BTreeMap::new(),
            lot_rows: Vec::new(),
            made_characters: sims_sim::characters::Characters::default(),
            deleted_characters: BTreeSet::new(),
            deletion_epoch: 0,
            deleted_at_epoch: BTreeMap::new(),
            instance_id: 1,
            revision: 0,
        };
        let chars = BTreeMap::from([("User00001.iff".to_owned(), iff_bytes)]);
        assert!(matches!(
            export_family_fam(&stored, &chars, 1, "Collide", 1, true),
            Err(CafError::TemplateInvalid(_))
        ));
    }

    #[test]
    fn parse_finv_rejects_duplicate_owner_rows() {
        let mut raw = Vec::new();
        raw.extend_from_slice(&0u32.to_le_bytes());
        raw.extend_from_slice(&STREAM_VERSION_COMPLETE_COLLECTION.to_le_bytes());
        raw.extend_from_slice(b"VNIF");
        raw.extend_from_slice(&2u32.to_le_bytes());
        for item_guid in [0x1111_1111u32, 0x2222_2222u32] {
            raw.extend_from_slice(&1i32.to_le_bytes());
            raw.extend_from_slice(&5i16.to_le_bytes()); // duplicate neighbor 5
            raw.extend_from_slice(&1u32.to_le_bytes());
            raw.extend_from_slice(&1i32.to_le_bytes());
            raw.extend_from_slice(&item_guid.to_le_bytes());
            raw.extend_from_slice(&1u16.to_le_bytes());
        }
        assert!(matches!(
            parse_finv(&raw),
            Err(CafError::TemplateInvalid(_))
        ));
    }

    #[test]
    fn export_family_fam_prefers_loose_iff_over_shadowed_far_member() {
        let tpl = synthetic_template_iff();
        let make_spec = |name: &str| SimSpec {
            first_name: name.into(),
            bio: String::new(),
            age: Age::Adult,
            gender: Gender::Female,
            skin_tone: SkinTone::Light,
            body_appearance: "b002fafit_01".into(),
            body_texture: "b002fafitlgt_mom02".into(),
            head_appearance: "c809fa_beth".into(),
            head_texture: "c809falgt_beth3".into(),
            personality: Personality::new(5, 5, 5, 5, 5).unwrap(),
            portraits: None,
        };
        let guid = 0x7777_0042u32;
        let stale_iff =
            synthesize_character_iff(&tpl, &make_spec("StaleFar"), guid, 1, 1, None).unwrap();
        let loose_iff =
            synthesize_character_iff(&tpl, &make_spec("FreshLoose"), guid, 1, 1, None).unwrap();

        // Put `stale_iff` inside `zzz_archive.far` (sorts lexicographically AFTER `User00001.iff`).
        let member_name = b"User00001.iff";
        let payload_offset = 16u32;
        let manifest_off = payload_offset + stale_iff.len() as u32;
        let mut far = Vec::new();
        far.extend_from_slice(b"FAR!byAZ");
        far.extend_from_slice(&1u32.to_le_bytes());
        far.extend_from_slice(&manifest_off.to_le_bytes());
        far.extend_from_slice(&stale_iff);
        far.extend_from_slice(&1u32.to_le_bytes());
        far.extend_from_slice(&(stale_iff.len() as u32).to_le_bytes());
        far.extend_from_slice(&(stale_iff.len() as u32).to_le_bytes());
        far.extend_from_slice(&payload_offset.to_le_bytes());
        far.extend_from_slice(&(member_name.len() as u32).to_le_bytes());
        far.extend_from_slice(member_name);

        let mut rng = CrtRng::from_state(7);
        let neighbor = synthesize_neighbor(&make_spec("FreshLoose"), 1, guid, 1, 1, &mut rng);
        let stored = StoredNeighborhood {
            name: "Precedence".into(),
            families: BTreeMap::from([(
                1,
                Family {
                    version: 9,
                    house: 0,
                    number: 1,
                    budget: 20_000,
                    architecture_value: 0,
                    friends: 0,
                    flags: CAF_INITIAL_FAMILY_FLAGS,
                    members: vec![guid],
                    tail: vec![0; 16],
                },
            )]),
            neighbors: vec![Some(neighbor)],
            inventories: Vec::new(),
            data: [0; 16],
            options: None,
            type_attributes: None,
            zoning: BTreeMap::new(),
            lot_rows: Vec::new(),
            made_characters: sims_sim::characters::Characters::default(),
            deleted_characters: BTreeSet::new(),
            deletion_epoch: 0,
            deleted_at_epoch: BTreeMap::new(),
            instance_id: 1,
            revision: 0,
        };
        let chars = BTreeMap::from([
            ("User00001.iff".to_owned(), loose_iff),
            ("zzz_archive.far".to_owned(), far),
        ]);
        let fam_bytes = export_family_fam(&stored, &chars, 1, "Precedence", 1, false).unwrap();
        let parsed = FamilyFile::parse(&fam_bytes).unwrap();
        let ctss =
            StringTable::parse(parsed.members.get(&1).unwrap().ctss.as_deref().unwrap()).unwrap();
        assert_eq!(ctss.get(0, 1), Some("FreshLoose"));
    }

    #[test]
    fn paused_caf_commit_returns_mismatch_and_preserves_replacement_byte_for_byte() {
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::sync::mpsc;

        static NEXT_PAUSE_DB: AtomicU64 = AtomicU64::new(0);
        let db_path = std::env::temp_dir().join(format!(
            "sims-vm-caf-paused-{}-{}.redb",
            std::process::id(),
            NEXT_PAUSE_DB.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&db_path);
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let _cleanup = Cleanup(db_path.clone());

        let store = NeighborhoodStore::open(&db_path).unwrap();
        let make_nh = |marker: i16| StoredNeighborhood {
            name: "PauseNH".into(),
            families: BTreeMap::from([(
                0,
                Family {
                    version: 9,
                    house: 0,
                    number: -1,
                    budget: 0,
                    architecture_value: 0,
                    friends: 0,
                    flags: 0,
                    members: Vec::new(),
                    tail: vec![0; 16],
                },
            )]),
            neighbors: Vec::new(),
            inventories: Vec::new(),
            data: [marker; 16],
            options: None,
            type_attributes: None,
            zoning: BTreeMap::from([(4, 1)]),
            lot_rows: vec![4],
            made_characters: sims_sim::characters::Characters::default(),
            deleted_characters: BTreeSet::new(),
            deletion_epoch: 0,
            deleted_at_epoch: BTreeMap::new(),
            instance_id: 1,
            revision: 0,
        };
        store.save_neighborhood("PauseNH", &make_nh(1)).unwrap();

        let ctx = CafContext::synthetic(
            synthetic_template_iff(),
            BTreeSet::from([TEMPLATE_PERSON_GUID]),
            DEFAULT_STARTING_FUNDS,
        );
        let spec = FamilySpec::new(
            "Stale",
            "",
            vec![SimSpec {
                first_name: "Alice".into(),
                bio: String::new(),
                age: Age::Adult,
                gender: Gender::Female,
                skin_tone: SkinTone::Light,
                body_appearance: "b002fafit_01".into(),
                body_texture: "b002fafitlgt_mom02".into(),
                head_appearance: "c809fa_beth".into(),
                head_texture: "c809falgt_beth3".into(),
                personality: Personality::new(5, 5, 5, 5, 5).unwrap(),
                portraits: None,
            }],
        );

        // Case 1: Pause CAF right after snapshot capture, replace the
        // neighborhood (simulating concurrent `import_directory`), resume CAF,
        // and assert deterministic `NeighborhoodMismatch` + byte-for-byte
        // preservation of the replacement neighborhood and zero leaked files.
        let ctx_ref = &ctx;
        let store_ref = &store;
        let spec_ref = &spec;
        let mut replacement_encoded = Vec::new();
        let mut replacement_chars = BTreeMap::new();
        let err = std::thread::scope(|scope| {
            let (snap_tx, snap_rx) = mpsc::channel();
            let (resume_tx, resume_rx) = mpsc::channel();
            let h = scope.spawn(move || {
                let mut rng = CrtRng::from_state(0x1122_3344);
                commit_family_with_context_hook(
                    ctx_ref,
                    store_ref,
                    "PauseNH",
                    spec_ref,
                    &mut rng,
                    || {
                        let _ = snap_tx.send(());
                        let _ = resume_rx.recv();
                    },
                )
            });
            snap_rx.recv().unwrap();
            assert!(store.delete_neighborhood("PauseNH").unwrap());
            store.save_neighborhood("PauseNH", &make_nh(77)).unwrap();
            store
                .save_character_file("PauseNH", "User00099.iff", b"replacement-char")
                .unwrap();
            replacement_encoded = store
                .load_neighborhood("PauseNH")
                .unwrap()
                .unwrap()
                .encode();
            replacement_chars = store.load_character_files("PauseNH").unwrap();
            resume_tx.send(()).unwrap();
            h.join().unwrap().unwrap_err()
        });
        assert!(
            matches!(err, CafError::NeighborhoodMismatch { .. }),
            "expected NeighborhoodMismatch, got {err}"
        );
        assert_eq!(
            store
                .load_neighborhood("PauseNH")
                .unwrap()
                .unwrap()
                .encode(),
            replacement_encoded
        );
        assert_eq!(
            store.load_character_files("PauseNH").unwrap(),
            replacement_chars
        );

        // Case 2: Pause CAF right after snapshot capture, bump same-instance
        // revision via `save_neighborhood_checked`, resume CAF, and assert
        // deterministic `StaleNeighborhood` + byte-for-byte preservation.
        let mut bumped_encoded = Vec::new();
        let err2 = std::thread::scope(|scope| {
            let (snap_tx2, snap_rx2) = mpsc::channel();
            let (resume_tx2, resume_rx2) = mpsc::channel();
            let h = scope.spawn(move || {
                let mut rng = CrtRng::from_state(0x5566_7788);
                commit_family_with_context_hook(
                    ctx_ref,
                    store_ref,
                    "PauseNH",
                    spec_ref,
                    &mut rng,
                    || {
                        let _ = snap_tx2.send(());
                        let _ = resume_rx2.recv();
                    },
                )
            });
            snap_rx2.recv().unwrap();
            let mut cur = store.load_neighborhood("PauseNH").unwrap().unwrap();
            cur.data[1] = 88;
            store
                .save_neighborhood_checked("PauseNH", cur.instance_id, cur.revision, &cur)
                .unwrap()
                .unwrap();
            bumped_encoded = store
                .load_neighborhood("PauseNH")
                .unwrap()
                .unwrap()
                .encode();
            resume_tx2.send(()).unwrap();
            h.join().unwrap().unwrap_err()
        });
        assert!(
            matches!(err2, CafError::StaleNeighborhood { .. }),
            "expected StaleNeighborhood, got {err2}"
        );
        assert_eq!(
            store
                .load_neighborhood("PauseNH")
                .unwrap()
                .unwrap()
                .encode(),
            bumped_encoded
        );
        assert_eq!(
            store.load_character_files("PauseNH").unwrap(),
            replacement_chars
        );
    }
}
