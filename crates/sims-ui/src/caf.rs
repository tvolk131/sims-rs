//! Create-a-Family (`cWinEditFamily` / `cWinCreateFamily`) and Create-a-Sim
//! (`cWinEditPerson`) UI screens from The Sims Complete Collection 1.91.
//!
//! All artwork IDs come from `UIGraphics/Res_Nbhd.h` (`5022..=5040`, `5042`,
//! `5203`) plus the shared dialog chrome (`dialog::BUTTON` 17 and
//! `dialog::BACKGROUND` 3008). All labels, tooltips, confirmation prompts,
//! personality trait names, and zodiac sign names are loaded dynamically from
//! `GameData/UIText.iff` (`STR# 129`, `130`, `142`, `164`).
//!
//! Coordinates at `800×600` are cross-checked against the Complete Collection
//! 1.91 oracle capture (`docs/decisions/20261121-create-a-family-ui.md`).
//! Below `800×600`, [`CafLayout`] scales all control rectangles cleanly into
//! the available viewport; above `800×600`, the `800×600` canvas is centered.

use crate::art::Art;
use crate::dialog::{self, CARET_BLINK_US};
use crate::font::Font;
use crate::frame::{Frame, Rect};
use crate::household::{self, PERSONALITY, ZODIAC};
use crate::hud::{LABEL, rgb565};
use crate::{Error, Result};
use sims_content::Install;
use sims_formats::bitmap::Image;
use sims_formats::iff::Iff;
use sims_formats::strings::StringTable;

/// UIGraphics bitmap resource IDs for Create-a-Family and Create-a-Sim
/// (`UIGraphics/Res_Nbhd.h`).
pub mod id {
    /// Family overview background (`Nbhd/DsgnFamBkg.bmp`, 800×600).
    pub const FAMILY_BKG: i32 = 5022;
    /// Add-a-Sim button (`Nbhd/AddPersonBtn.bmp`, 4 cells of 139×103).
    pub const FAMILY_ADD: i32 = 5023;
    /// Delete-selected-Sim button (`Nbhd/DeletePersonBtn.bmp`, 4 cells of 149×99).
    pub const FAMILY_DELETE: i32 = 5024;
    /// Edit-selected-Sim button (`Nbhd/EditPersonBtn.bmp`, 4 cells of 137×99).
    pub const FAMILY_EDIT: i32 = 5025;
    /// Family Done button (`Nbhd/DesignFamDone.bmp`, 6 cells of 110×52).
    pub const FAMILY_DONE: i32 = 5026;
    /// Family Cancel button (`Nbhd/DesignFamCancel.bmp`, 6 cells of 149×52).
    pub const FAMILY_CANCEL: i32 = 5027;

    /// Create-a-Sim background (`Nbhd/CreateACharBack.bmp`, 800×600).
    pub const CHAR_BKG: i32 = 5042;
    /// Adult toggle button (`Nbhd/DesignCharAdultBtn.bmp`, 4 cells of 57×100).
    pub const CHAR_ADULT: i32 = 5028;
    /// Child toggle button (`Nbhd/DesignCharChildBtn.bmp`, 4 cells of 92×90).
    pub const CHAR_CHILD: i32 = 5029;
    /// Previous head/body arrow (`Nbhd/DesignCharSkinsLeftBtn.bmp`, 4 cells of 18×34).
    pub const CHAR_SKINS_LEFT: i32 = 5030;
    /// Next head/body arrow (`Nbhd/DesignCharSkinsRightBtn.bmp`, 4 cells of 18×34).
    pub const CHAR_SKINS_RIGHT: i32 = 5032;
    /// Sim Done button (`Nbhd/DesignCharDoneBtn.bmp`, 6 cells of 105×40).
    pub const CHAR_DONE: i32 = 5033;
    /// Sim Cancel button (`Nbhd/DesignCharCancelBtn.bmp`, 6 cells of 137×40).
    pub const CHAR_CANCEL: i32 = 5034;
    /// Female toggle button (`Nbhd/DesignCharFemaleBtn.bmp`, 4 cells of 57×121).
    pub const CHAR_FEMALE: i32 = 5035;
    /// Male toggle button (`Nbhd/DesignCharMaleBtn.bmp`, 4 cells of 98×111).
    pub const CHAR_MALE: i32 = 5036;
    /// Dark skin-tone toggle (`Nbhd/DesignCharDarkBtn.bmp`, 4 cells of 57×71).
    pub const CHAR_DARK: i32 = 5037;
    /// Medium skin-tone toggle (`Nbhd/DesignCharMediumBtn.bmp`, 4 cells of 46×71).
    pub const CHAR_MEDIUM: i32 = 5038;
    /// Light skin-tone toggle (`Nbhd/DesignCharLightBtn.bmp`, 4 cells of 55×71).
    pub const CHAR_LIGHT: i32 = 5039;
    /// Personality 10-pip LED bar (`Nbhd/DesignCharPersLED.bmp`, 59×25).
    pub const CHAR_PERS_LED: i32 = 5040;
    /// Remaining 25-point bar (`Nbhd/CACPointRemainingBars.bmp`, 149×25).
    pub const CHAR_POINTS_BAR: i32 = 5203;
}

/// All UIGraphics bitmap resources required by the Create-a-Family screens,
/// for [`Art::load`].
pub const RESOURCES: [i32; 22] = [
    id::FAMILY_BKG,
    id::FAMILY_ADD,
    id::FAMILY_DELETE,
    id::FAMILY_EDIT,
    id::FAMILY_DONE,
    id::FAMILY_CANCEL,
    id::CHAR_BKG,
    id::CHAR_ADULT,
    id::CHAR_CHILD,
    id::CHAR_SKINS_LEFT,
    id::CHAR_SKINS_RIGHT,
    id::CHAR_DONE,
    id::CHAR_CANCEL,
    id::CHAR_FEMALE,
    id::CHAR_MALE,
    id::CHAR_DARK,
    id::CHAR_MEDIUM,
    id::CHAR_LIGHT,
    id::CHAR_PERS_LED,
    id::CHAR_POINTS_BAR,
    dialog::BUTTON,
    dialog::BACKGROUND,
];

/// Native design resolution of the CAF screens.
pub const BASE_WIDTH: i32 = 800;
pub const BASE_HEIGHT: i32 = 600;

/// Maximum household members allowed in Create-a-Family.
pub const MAX_MEMBERS: usize = 8;
/// Total personality point budget in Create-a-Sim.
pub const PERSONALITY_BUDGET: u8 = 25;
/// Maximum points per personality trait.
pub const TRAIT_MAX: u8 = 10;
/// Maximum characters for family last name and Sim first name.
pub const NAME_MAX: usize = 32;
/// Maximum characters for family and Sim bio fields.
pub const BIO_MAX: usize = 254;

/// `UIText.iff` string table IDs used by CAF.
pub const FAMILY_STRINGS: u16 = 129;
pub const SIM_STRINGS: u16 = 130;
pub const ZODIAC_STRINGS: u16 = 164;

/// Native colors matching `dialog.rs` and `hud.rs`.
const ENTRY_FILL: [u8; 3] = rgb565(0x000a);
const WELL_BORDER: [u8; 3] = [0x46, 0x5c, 0x9c];
const FOCUS_BORDER: [u8; 3] = LABEL;
const LED_LIT: [u8; 3] = [0x24, 0xf2, 0xff];
const LED_DIM: [u8; 3] = [0x26, 0x2f, 0x5a];
const BUTTON_HOVER: [u8; 3] = [255, 255, 255];
const BUTTON_PRESSED: [u8; 3] = [0, 255, 255];
const BUTTON_DISABLED: [u8; 3] = [0x40, 0x5d, 0x5f];
const TOOLTIP_BG: [u8; 3] = [0x04, 0x10, 0x42];
const TOOLTIP_BORDER: [u8; 3] = [0x88, 0xb2, 0xe8];
const PORTRAIT_BG: [u8; 3] = [0x14, 0x1e, 0x52];
const PORTRAIT_SELECTED: [u8; 3] = [0x24, 0xf2, 0xff];
const FALLBACK_BG: [u8; 3] = [0x00, 0x0f, 0x5a];
const FALLBACK_BTN_BG: [u8; 3] = [0x08, 0x5c, 0x9c];
const FALLBACK_BTN_DISABLED: [u8; 3] = [0x18, 0x34, 0x5a];

/// Active screen within the Create-a-Family flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CafScreen {
    /// Household overview (`cWinEditFamily`).
    Family,
    /// Individual Sim editor (`cWinEditPerson`).
    EditSim,
}

/// Sim age toggle in Create-a-Sim.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Age {
    #[default]
    Adult,
    Child,
}

/// Sim gender toggle in Create-a-Sim.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Gender {
    #[default]
    Male,
    Female,
}

/// Sim skin-tone toggle in Create-a-Sim.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SkinTone {
    #[default]
    Light,
    Medium,
    Dark,
}

/// Household member species (Unleashed households may contain pets alongside
/// human Sims; Create-a-Family's `+` button creates human Sims).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MemberKind {
    #[default]
    Sim,
    Dog,
    Cat,
}

/// The five personality traits in native Create-a-Sim order (`STR# 130[17..22]`,
/// matching [`household::PERSONALITY`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trait {
    Neat = 0,
    Outgoing = 1,
    Active = 2,
    Playful = 3,
    Nice = 4,
}

impl Trait {
    /// All five traits in top-to-bottom display order.
    pub const ALL: [Self; 5] = [
        Self::Neat,
        Self::Outgoing,
        Self::Active,
        Self::Playful,
        Self::Nice,
    ];

    pub const fn index(self) -> usize {
        self as usize
    }
}

/// Five personality trait values (`0..=10` each, sum `<= 25`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Personality {
    values: [u8; 5],
}

impl Personality {
    /// Creates a personality from raw `[neat, outgoing, active, playful, nice]`
    /// values, clamping each trait to `0..=10` and trimming excess points in
    /// order so the total never exceeds [`PERSONALITY_BUDGET`].
    pub fn from_raw(raw: [u8; 5]) -> Self {
        let mut p = Self::default();
        for (i, &v) in raw.iter().enumerate() {
            let allow = p.remaining().min(TRAIT_MAX);
            p.values[i] = v.min(allow);
        }
        p
    }

    /// Current value (`0..=10`) of `trait_`.
    pub const fn get(&self, trait_: Trait) -> u8 {
        self.values[trait_.index()]
    }

    /// Raw array `[neat, outgoing, active, playful, nice]`.
    pub const fn values(&self) -> [u8; 5] {
        self.values
    }

    /// Points currently spent (`0..=25`).
    pub fn spent(&self) -> u8 {
        self.values.iter().copied().sum()
    }

    /// Points remaining in the 25-point budget (`0..=25`).
    pub fn remaining(&self) -> u8 {
        PERSONALITY_BUDGET.saturating_sub(self.spent())
    }

    /// Sets `trait_` to `target`, clamped to `0..=10` and bounded by the
    /// remaining budget. Returns the actual resulting value.
    pub fn set(&mut self, trait_: Trait, target: u8) -> u8 {
        let idx = trait_.index();
        let current = self.values[idx];
        let clamped = target.min(TRAIT_MAX);
        let next = if clamped > current {
            let max_allowed = current.saturating_add(self.remaining()).min(TRAIT_MAX);
            clamped.min(max_allowed)
        } else {
            clamped
        };
        self.values[idx] = next;
        next
    }

    /// Computes the live zodiac sign (`1` = Aries .. `12` = Pisces) using
    /// [`household::zodiac`] (`0x4841fc`), returning `None` when no points have
    /// been spent yet (matching the initial Create-a-Sim screen).
    pub fn zodiac(&self) -> Option<u8> {
        if self.spent() == 0 {
            return None;
        }
        let vals = self.values;
        household::zodiac(|word| {
            if word == sims_sim::neighbors::KIND || word == ZODIAC {
                return 0;
            }
            for (i, &pword) in PERSONALITY.iter().enumerate() {
                if word == pword {
                    return i16::from(vals[i]) * 100;
                }
            }
            0
        })
    }
}

/// Visual appearance selection for a Sim being edited in Create-a-Sim.
#[derive(Debug, Clone, PartialEq)]
pub struct SimAppearance {
    pub age: Age,
    pub gender: Gender,
    pub skin_tone: SkinTone,
    pub head_index: usize,
    pub body_index: usize,
    /// Available heads for the current age/gender/skin filter (`0` means no
    /// catalog entries are available and cycling is disabled).
    pub head_count: usize,
    /// Available bodies for the current age/gender/skin filter (`0` means no
    /// catalog entries are available and cycling is disabled).
    pub body_count: usize,
    /// Turntable preview yaw in degrees (`0.0..360.0`).
    pub preview_yaw_deg: f32,
}

impl Default for SimAppearance {
    fn default() -> Self {
        Self {
            age: Age::Adult,
            gender: Gender::Male,
            skin_tone: SkinTone::Light,
            head_index: 0,
            body_index: 0,
            head_count: 1,
            body_count: 1,
            preview_yaw_deg: 0.0,
        }
    }
}

impl SimAppearance {
    pub fn prev_head(&mut self) {
        if self.head_count > 0 {
            self.head_index = (self.head_index + self.head_count - 1) % self.head_count;
        }
    }

    pub fn next_head(&mut self) {
        if self.head_count > 0 {
            self.head_index = (self.head_index + 1) % self.head_count;
        }
    }

    pub fn prev_body(&mut self) {
        if self.body_count > 0 {
            self.body_index = (self.body_index + self.body_count - 1) % self.body_count;
        }
    }

    pub fn next_body(&mut self) {
        if self.body_count > 0 {
            self.body_index = (self.body_index + 1) % self.body_count;
        }
    }

    pub fn rotate_preview(&mut self, delta_deg: f32) {
        if delta_deg.is_finite() {
            self.preview_yaw_deg = (self.preview_yaw_deg + delta_deg).rem_euclid(360.0);
        }
    }

    /// Updates catalog counts (e.g. after switching age/gender/skin) and
    /// clamps `head_index` / `body_index` into range.
    pub fn set_catalog_counts(&mut self, head_count: usize, body_count: usize) {
        self.head_count = head_count;
        self.body_count = body_count;
        if head_count > 0 {
            self.head_index %= head_count;
        } else {
            self.head_index = 0;
        }
        if body_count > 0 {
            self.body_index %= body_count;
        } else {
            self.body_index = 0;
        }
    }
}

/// Draft state for one household member.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SimDraft {
    pub first_name: String,
    pub bio: String,
    pub kind: MemberKind,
    pub appearance: SimAppearance,
    pub personality: Personality,
    /// Optional 45×45 portrait bitmap rendered by the host / `sims-people`.
    pub portrait: Option<Image>,
}

impl SimDraft {
    pub fn is_adult_human(&self) -> bool {
        self.kind == MemberKind::Sim && self.appearance.age == Age::Adult
    }
}

/// Draft state for the household being created.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FamilyDraft {
    pub last_name: String,
    pub bio: String,
    pub members: Vec<SimDraft>,
}

impl FamilyDraft {
    /// Whether the household can be committed (`Done` enabled): requires a
    /// non-empty family last name, at most [`MAX_MEMBERS`] members, and at
    /// least one adult human Sim.
    pub fn can_finish(&self) -> bool {
        !self.last_name.trim().is_empty()
            && self.members.len() <= MAX_MEMBERS
            && self.members.iter().any(SimDraft::is_adult_human)
    }

    /// Whether another member can be added (`members.len() < MAX_MEMBERS`).
    pub fn can_add_member(&self) -> bool {
        self.members.len() < MAX_MEMBERS
    }

    /// Whether the draft has any user-entered content (used to decide whether
    /// Cancel requires confirmation).
    pub fn has_changes(&self) -> bool {
        !self.last_name.trim().is_empty() || !self.bio.trim().is_empty() || !self.members.is_empty()
    }
}

/// Dynamically loaded UI strings from `GameData/UIText.iff` (`STR# 129`,
/// `130`, `142`, `164`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CafTexts {
    // STR# 129 (Create-a-Family)
    pub tip_add_sim: String,
    pub tip_delete_sim: String,
    pub tip_edit_sim: String,
    pub tip_family_done: String,
    pub tip_family_cancel: String,
    pub confirm_delete_title: String,
    pub confirm_delete_body: String,
    pub confirm_cancel_title: String,
    pub confirm_cancel_body: String,
    pub family_title: String,
    pub family_done: String,
    pub family_cancel: String,
    pub enter_last_name: String,
    pub confirm_finish_title: String,
    pub confirm_finish_body: String,

    // STR# 130 (Create-a-Sim)
    pub tip_adult: String,
    pub tip_child: String,
    pub tip_female: String,
    pub tip_male: String,
    pub tip_dark: String,
    pub tip_medium: String,
    pub tip_light: String,
    pub tip_prev_body: String,
    pub tip_next_body: String,
    pub tip_prev_head: String,
    pub tip_next_head: String,
    pub tip_sim_done: String,
    pub tip_sim_cancel: String,
    pub confirm_personality_title: String,
    pub confirm_personality_body: String,
    pub sim_title: String,
    pub personality_heading: String,
    pub trait_names: [String; 5],
    pub sim_done: String,
    pub sim_cancel: String,
    pub bio_label: String,
    pub bio_for_fmt: String,
    pub enter_first_name: String,
    pub zodiac_tooltip: String,

    // STR# 142 (Standard dialog buttons)
    pub btn_ok: String,
    pub btn_cancel: String,
    pub btn_yes: String,
    pub btn_no: String,

    // STR# 164 (Zodiac signs, Aries..Pisces)
    pub zodiac_names: [String; 12],
}

impl Default for CafTexts {
    fn default() -> Self {
        Self {
            tip_add_sim: "Add".into(),
            tip_delete_sim: "Delete".into(),
            tip_edit_sim: "Edit".into(),
            tip_family_done: "Done".into(),
            tip_family_cancel: "Cancel".into(),
            confirm_delete_title: "Delete".into(),
            confirm_delete_body: "Confirm delete?".into(),
            confirm_cancel_title: "Cancel".into(),
            confirm_cancel_body: "Discard draft?".into(),
            family_title: "Family".into(),
            family_done: "Done".into(),
            family_cancel: "Cancel".into(),
            enter_last_name: "Last Name:".into(),
            confirm_finish_title: "Finish".into(),
            confirm_finish_body: "Commit family?".into(),

            tip_adult: "Adult".into(),
            tip_child: "Child".into(),
            tip_female: "Female".into(),
            tip_male: "Male".into(),
            tip_dark: "Dark".into(),
            tip_medium: "Medium".into(),
            tip_light: "Light".into(),
            tip_prev_body: "Prev Body".into(),
            tip_next_body: "Next Body".into(),
            tip_prev_head: "Prev Head".into(),
            tip_next_head: "Next Head".into(),
            tip_sim_done: "Done".into(),
            tip_sim_cancel: "Cancel".into(),
            confirm_personality_title: "Personality".into(),
            confirm_personality_body: "Points remain. Continue?".into(),
            sim_title: "Sim".into(),
            personality_heading: "Personality".into(),
            trait_names: [
                "Neat".into(),
                "Outgoing".into(),
                "Active".into(),
                "Playful".into(),
                "Nice".into(),
            ],
            sim_done: "Done".into(),
            sim_cancel: "Cancel".into(),
            bio_label: "Bio".into(),
            bio_for_fmt: "Bio: %s".into(),
            enter_first_name: "First Name:".into(),
            zodiac_tooltip: "Zodiac".into(),

            btn_ok: "OK".into(),
            btn_cancel: "Cancel".into(),
            btn_yes: "Yes".into(),
            btn_no: "No".into(),

            zodiac_names: [
                "Aries".into(),
                "Taurus".into(),
                "Gemini".into(),
                "Cancer".into(),
                "Leo".into(),
                "Virgo".into(),
                "Libra".into(),
                "Scorpio".into(),
                "Sagittarius".into(),
                "Capricorn".into(),
                "Aquarius".into(),
                "Pisces".into(),
            ],
        }
    }
}

impl CafTexts {
    /// Loads all CAF strings from `GameData/UIText.iff` in `language`.
    pub fn load(install: &Install, language: u8) -> Result<Self> {
        let path = install
            .file("GameData/UIText.iff")
            .map_err(Error::Content)?;
        let bytes = std::fs::read(&path).map_err(Error::Io)?;
        let iff = Iff::parse(bytes).map_err(Error::Format)?;
        let mut texts = Self::default();

        if let Some(table) = load_table(&iff, FAMILY_STRINGS)? {
            let s = |i: usize, fallback: &str| table_str(&table, i, language, fallback);
            texts.tip_add_sim = s(0, &texts.tip_add_sim);
            texts.tip_delete_sim = s(1, &texts.tip_delete_sim);
            texts.tip_edit_sim = s(2, &texts.tip_edit_sim);
            texts.tip_family_done = s(3, &texts.tip_family_done);
            texts.tip_family_cancel = s(4, &texts.tip_family_cancel);
            texts.confirm_delete_title = s(5, &texts.confirm_delete_title);
            texts.confirm_delete_body = s(6, &texts.confirm_delete_body);
            texts.confirm_cancel_title = s(7, &texts.confirm_cancel_title);
            texts.confirm_cancel_body = s(8, &texts.confirm_cancel_body);
            texts.family_title = s(9, &texts.family_title);
            texts.family_done = s(10, &texts.family_done);
            texts.family_cancel = s(11, &texts.family_cancel);
            texts.enter_last_name = s(12, &texts.enter_last_name);
            texts.confirm_finish_title = s(13, &texts.confirm_finish_title);
            texts.confirm_finish_body = s(14, &texts.confirm_finish_body);
        }

        if let Some(table) = load_table(&iff, SIM_STRINGS)? {
            let s = |i: usize, fallback: &str| table_str(&table, i, language, fallback);
            texts.tip_adult = s(0, &texts.tip_adult);
            texts.tip_child = s(1, &texts.tip_child);
            texts.tip_female = s(2, &texts.tip_female);
            texts.tip_male = s(3, &texts.tip_male);
            texts.tip_dark = s(4, &texts.tip_dark);
            texts.tip_medium = s(5, &texts.tip_medium);
            texts.tip_light = s(6, &texts.tip_light);
            texts.tip_prev_body = s(7, &texts.tip_prev_body);
            texts.tip_next_body = s(8, &texts.tip_next_body);
            texts.tip_prev_head = s(9, &texts.tip_prev_head);
            texts.tip_next_head = s(10, &texts.tip_next_head);
            texts.tip_sim_done = s(11, &texts.tip_sim_done);
            texts.tip_sim_cancel = s(12, &texts.tip_sim_cancel);
            texts.confirm_personality_title = s(13, &texts.confirm_personality_title);
            texts.confirm_personality_body = s(14, &texts.confirm_personality_body);
            texts.sim_title = s(15, &texts.sim_title);
            texts.personality_heading = s(16, &texts.personality_heading);
            for (i, slot) in texts.trait_names.iter_mut().enumerate() {
                *slot = s(17 + i, slot);
            }
            texts.sim_done = s(22, &texts.sim_done);
            texts.sim_cancel = s(23, &texts.sim_cancel);
            texts.bio_label = s(24, &texts.bio_label);
            texts.bio_for_fmt = s(25, &texts.bio_for_fmt);
            texts.enter_first_name = s(26, &texts.enter_first_name);
            texts.zodiac_tooltip = s(27, &texts.zodiac_tooltip);
        }

        if let Some(table) = load_table(&iff, dialog::LABELS)? {
            let s = |i: usize, fallback: &str| table_str(&table, i, language, fallback);
            texts.btn_ok = s(0, &texts.btn_ok);
            texts.btn_cancel = s(1, &texts.btn_cancel);
            texts.btn_yes = s(2, &texts.btn_yes);
            texts.btn_no = s(3, &texts.btn_no);
        }

        if let Some(table) = load_table(&iff, ZODIAC_STRINGS)? {
            for (i, slot) in texts.zodiac_names.iter_mut().enumerate() {
                *slot = table_str(&table, i, language, slot);
            }
        }

        Ok(texts)
    }

    /// Formats the bio heading above the Sim bio field using `bio_label` when
    /// unnamed, or substituting the Sim's name into `bio_for_fmt`.
    pub fn format_bio_heading(&self, first_name: &str, last_name: &str) -> String {
        let first = first_name.trim();
        if first.is_empty() {
            return self.bio_label.clone();
        }
        let last = last_name.trim();
        let full = if last.is_empty() {
            first.to_owned()
        } else {
            format!("{first} {last}")
        };
        if self.bio_for_fmt.contains("%s") {
            self.bio_for_fmt.replacen("%s", &full, 1)
        } else {
            format!("{} {full}", self.bio_for_fmt)
        }
    }

    /// Localized name for zodiac sign `1..=12`.
    pub fn zodiac_name(&self, sign: u8) -> &str {
        &self.zodiac_names[usize::from(sign.clamp(1, 12) - 1)]
    }
}

fn load_table(iff: &Iff, id: u16) -> Result<Option<StringTable>> {
    let Some(chunk) = iff.get(b"STR#", id) else {
        return Ok(None);
    };
    StringTable::parse(iff.data(chunk))
        .map(Some)
        .map_err(Error::Format)
}

fn table_str(table: &StringTable, index: usize, language: u8, fallback: &str) -> String {
    table
        .get(index, language)
        .or_else(|| table.get(index, sims_content::ui_graphics::US_ENGLISH))
        .filter(|s| !s.is_empty())
        .map_or_else(|| fallback.to_owned(), str::to_owned)
}

/// Active modal confirmation dialog inside CAF.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmKind {
    DeleteSim(usize),
    CancelFamily,
    FinishFamily,
    MorePersonality,
}

/// Which text field currently holds keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusField {
    FamilyLastName,
    FamilyBio,
    SimFirstName,
    SimBio,
}

/// Interactive controls for hover/press tracking and tooltips.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlId {
    FamilyAdd,
    FamilyDelete,
    FamilyEdit,
    FamilyDone,
    FamilyCancel,
    MemberSlot(usize),
    SimAdult,
    SimChild,
    SimLight,
    SimMedium,
    SimDark,
    SimMale,
    SimFemale,
    HeadPrev,
    HeadNext,
    BodyPrev,
    BodyNext,
    RotateLeft,
    RotateRight,
    TraitSlider(Trait),
    ZodiacBadge,
    SimDone,
    SimCancel,
    DialogPrimary,
    DialogSecondary,
}

/// Commands emitted by [`CafController`] to the host / game loop.
#[derive(Debug, Clone, PartialEq)]
pub enum CafCommand {
    /// Player confirmed completion on the Family screen with a valid household.
    CommitFamily(FamilyDraft),
    /// Player cancelled Create-a-Family and returned to the Family Bin.
    CancelFamily,
    /// Player opened the Create-a-Sim editor (`index: None` for new Sim,
    /// `Some(i)` to edit member `i`).
    OpenSimEditor { index: Option<usize> },
    /// Player saved a Sim from Create-a-Sim back into the household roster.
    SimSaved { index: usize, is_new: bool },
    /// Player confirmed deletion of household member `index`.
    SimDeleted(usize),
    /// Player cancelled Create-a-Sim without saving changes.
    CancelSim,
    /// Age, gender, skin tone, or head/body selection changed in Create-a-Sim.
    AppearanceChanged(SimAppearance),
    /// Player rotated the 3D turntable preview (new yaw in degrees).
    RotatePreview(f32),
}

/// Caret and blink state for the currently focused `cTSWinTextEdit2` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextFieldState {
    pub caret: usize,
    pub caret_visible: bool,
    pub blink_since_us: u64,
}

impl Default for TextFieldState {
    fn default() -> Self {
        Self {
            caret: 0,
            caret_visible: true,
            blink_since_us: 0,
        }
    }
}

impl TextFieldState {
    fn reset_to_end(&mut self, text: &str, clock_us: u64) {
        self.caret = text.chars().count();
        self.caret_visible = true;
        self.blink_since_us = clock_us;
    }

    fn sync_blink(&mut self, clock_us: u64) -> bool {
        if clock_us.saturating_sub(self.blink_since_us) >= CARET_BLINK_US {
            self.caret_visible = !self.caret_visible;
            self.blink_since_us = clock_us;
            true
        } else {
            false
        }
    }

    fn insert_char(
        &mut self,
        text: &mut String,
        ch: char,
        max_chars: usize,
        multiline: bool,
    ) -> bool {
        if ch.is_control() && !(multiline && ch == '\n') {
            return false;
        }
        let len = text.chars().count();
        if len >= max_chars {
            return false;
        }
        let pos = self.caret.min(len);
        let byte = char_byte_index(text, pos);
        text.insert(byte, ch);
        self.caret = pos + 1;
        self.caret_visible = true;
        true
    }

    fn backspace(&mut self, text: &mut String) -> bool {
        let len = text.chars().count();
        let pos = self.caret.min(len);
        if pos == 0 {
            return false;
        }
        let start = char_byte_index(text, pos - 1);
        let end = char_byte_index(text, pos);
        text.replace_range(start..end, "");
        self.caret = pos - 1;
        self.caret_visible = true;
        true
    }

    fn delete(&mut self, text: &mut String) -> bool {
        let len = text.chars().count();
        let pos = self.caret.min(len);
        if pos >= len {
            return false;
        }
        let start = char_byte_index(text, pos);
        let end = char_byte_index(text, pos + 1);
        text.replace_range(start..end, "");
        self.caret_visible = true;
        true
    }

    fn move_left(&mut self) {
        self.caret = self.caret.saturating_sub(1);
        self.caret_visible = true;
    }

    fn move_home(&mut self) {
        self.caret = 0;
        self.caret_visible = true;
    }
}

fn char_byte_index(text: &str, char_idx: usize) -> usize {
    text.char_indices()
        .nth(char_idx)
        .map_or(text.len(), |(byte, _)| byte)
}

/// Computed layout rectangles for both Create-a-Family screens at a given
/// frame size, including compact reflow below `800×600`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CafLayout {
    pub canvas: Rect,
    pub compact: bool,

    // Family screen
    pub family_title_pos: [i32; 2],
    pub family_name_label_right: [i32; 2],
    pub family_name_field: Rect,
    pub family_roster_well: Rect,
    pub family_slots: [Rect; MAX_MEMBERS],
    pub family_bio_label_pos: [i32; 2],
    pub family_bio_field: Rect,
    pub family_add: Rect,
    pub family_delete: Rect,
    pub family_edit: Rect,
    pub family_done: Rect,
    pub family_cancel: Rect,

    // Create-a-Sim screen
    pub sim_title_pos: [i32; 2],
    pub sim_name_label_right: [i32; 2],
    pub sim_name_field: Rect,
    pub personality_heading_pos: [i32; 2],
    pub personality_well: Rect,
    pub trait_label_right_x: i32,
    pub trait_sliders: [Rect; 5],
    pub zodiac_rect: Rect,
    pub remaining_points_rect: Rect,
    pub sim_child: Rect,
    pub sim_adult: Rect,
    pub sim_light: Rect,
    pub sim_medium: Rect,
    pub sim_dark: Rect,
    pub sim_male: Rect,
    pub sim_female: Rect,
    /// Viewport rectangle where Track 5's `sims-people` 3D turntable renders.
    pub preview_rect: Rect,
    pub head_prev: Rect,
    pub head_next: Rect,
    pub body_prev: Rect,
    pub body_next: Rect,
    pub rotate_left: Rect,
    pub rotate_right: Rect,
    pub sim_bio_label_pos: [i32; 2],
    pub sim_bio_field: Rect,
    pub sim_done: Rect,
    pub sim_cancel: Rect,

    // Modal confirmation dialog
    pub dialog_rect: Rect,
    pub dialog_primary: Rect,
    pub dialog_secondary: Rect,
}

impl CafLayout {
    /// Computes the CAF layout for a frame of `width × height` pixels.
    ///
    /// At `width >= 800 && height >= 600`, all coordinates match Complete
    /// Collection 1.91's native `800×600` layout centered in the frame. Below
    /// `800×600`, rectangles scale cleanly into the available viewport.
    pub fn compute(width: u32, height: u32) -> Self {
        let fw = i32::try_from(width).unwrap_or(BASE_WIDTH).max(1);
        let fh = i32::try_from(height).unwrap_or(BASE_HEIGHT).max(1);
        let compact = fw < BASE_WIDTH || fh < BASE_HEIGHT;

        let (ox, oy, sw, sh) = if compact {
            (0, 0, fw, fh)
        } else {
            (
                (fw - BASE_WIDTH) / 2,
                (fh - BASE_HEIGHT) / 2,
                BASE_WIDTH,
                BASE_HEIGHT,
            )
        };

        let map_x =
            |x: i32| -> i32 { ox + (i64::from(x) * i64::from(sw) / i64::from(BASE_WIDTH)) as i32 };
        let map_y =
            |y: i32| -> i32 { oy + (i64::from(y) * i64::from(sh) / i64::from(BASE_HEIGHT)) as i32 };
        let map_rect = |x: i32, y: i32, w: i32, h: i32| -> Rect {
            let x0 = map_x(x);
            let y0 = map_y(y);
            let x1 = map_x(x + w);
            let y1 = map_y(y + h);
            Rect::new(x0, y0, (x1 - x0).max(1), (y1 - y0).max(1))
        };

        let mut family_slots = [Rect::default(); MAX_MEMBERS];
        for (i, slot) in family_slots.iter_mut().enumerate() {
            let col = (i % 4) as i32;
            let row = (i / 4) as i32;
            *slot = map_rect(108 + col * 118, 126 + row * 130, 82, 86);
        }

        let mut trait_sliders = [Rect::default(); 5];
        for (i, slider) in trait_sliders.iter_mut().enumerate() {
            *slider = map_rect(229, 131 + i as i32 * 32, 60, 25);
        }

        Self {
            canvas: Rect::new(ox, oy, sw, sh),
            compact,

            family_title_pos: [map_x(400), map_y(12)],
            family_name_label_right: [map_x(250), map_y(55)],
            family_name_field: map_rect(263, 47, 274, 33),
            family_roster_well: map_rect(69, 100, 514, 312),
            family_slots,
            family_bio_label_pos: [map_x(72), map_y(418)],
            family_bio_field: map_rect(69, 436, 514, 78),
            family_add: map_rect(593, 103, 139, 103),
            family_delete: map_rect(594, 208, 149, 99),
            family_edit: map_rect(594, 311, 137, 99),
            family_done: map_rect(149, 533, 110, 52),
            family_cancel: map_rect(543, 533, 149, 52),

            sim_title_pos: [map_x(400), map_y(12)],
            sim_name_label_right: [map_x(250), map_y(55)],
            sim_name_field: map_rect(263, 47, 274, 33),
            personality_heading_pos: [map_x(191), map_y(92)],
            personality_well: map_rect(79, 118, 224, 222),
            trait_label_right_x: map_x(221),
            trait_sliders,
            zodiac_rect: map_rect(100, 294, 182, 24),
            remaining_points_rect: map_rect(117, 342, 149, 25),
            sim_child: map_rect(406, 111, 92, 90),
            sim_adult: map_rect(497, 101, 57, 100),
            sim_light: map_rect(396, 202, 55, 71),
            sim_medium: map_rect(452, 202, 46, 71),
            sim_dark: map_rect(499, 202, 57, 71),
            sim_male: map_rect(400, 274, 98, 111),
            sim_female: map_rect(499, 274, 57, 121),
            preview_rect: map_rect(574, 118, 182, 263),
            head_prev: map_rect(597, 160, 18, 34),
            head_next: map_rect(714, 160, 18, 34),
            body_prev: map_rect(580, 291, 18, 34),
            body_next: map_rect(729, 291, 18, 34),
            rotate_left: map_rect(580, 344, 36, 28),
            rotate_right: map_rect(714, 344, 36, 28),
            sim_bio_label_pos: [map_x(28), map_y(393)],
            sim_bio_field: map_rect(16, 422, 764, 97),
            sim_done: map_rect(149, 536, 105, 48),
            sim_cancel: map_rect(543, 536, 137, 48),

            dialog_rect: map_rect(216, 211, 368, 177),
            dialog_primary: map_rect(281, 336, 94, 33),
            dialog_secondary: map_rect(425, 336, 94, 33),
        }
    }
}

/// Interactive controller for Create-a-Family and Create-a-Sim.
#[derive(Debug, Clone)]
pub struct CafController {
    screen: CafScreen,
    family: FamilyDraft,
    selected_member: Option<usize>,
    /// Active Create-a-Sim draft (`None` when on `CafScreen::Family`).
    /// First tuple field is `Some(member_index)` when editing an existing Sim,
    /// or `None` when adding a new Sim.
    editing_sim: Option<(Option<usize>, SimDraft)>,
    confirm: Option<ConfirmKind>,
    focus: FocusField,
    text_state: TextFieldState,
    hovered: Option<ControlId>,
    pressed: Option<ControlId>,
    active_slider: Option<Trait>,
    dragging_preview_x: Option<i32>,
    clock_us: u64,
    texts: CafTexts,
}

impl CafController {
    /// Starts a fresh Create-a-Family session with localized `texts`.
    pub fn new(texts: CafTexts) -> Self {
        Self {
            screen: CafScreen::Family,
            family: FamilyDraft::default(),
            selected_member: None,
            editing_sim: None,
            confirm: None,
            focus: FocusField::FamilyLastName,
            text_state: TextFieldState::default(),
            hovered: None,
            pressed: None,
            active_slider: None,
            dragging_preview_x: None,
            clock_us: 0,
            texts,
        }
    }

    /// Starts Create-a-Family pre-populated with an existing [`FamilyDraft`].
    /// Members past [`MAX_MEMBERS`] are truncated so the roster matches what
    /// the screen renders and commits.
    pub fn with_family(texts: CafTexts, mut family: FamilyDraft) -> Self {
        family.members.truncate(MAX_MEMBERS);
        let mut ctrl = Self::new(texts);
        let name_len = family.last_name.chars().count();
        ctrl.selected_member = (!family.members.is_empty()).then_some(0);
        ctrl.family = family;
        ctrl.text_state.caret = name_len;
        ctrl
    }

    pub const fn screen(&self) -> CafScreen {
        self.screen
    }

    pub const fn family(&self) -> &FamilyDraft {
        &self.family
    }

    /// Updates the rendered portrait bitmap for household member `index`.
    pub fn set_member_portrait(&mut self, index: usize, portrait: Option<Image>) -> bool {
        let Some(member) = self.family.members.get_mut(index) else {
            return false;
        };
        member.portrait = portrait;
        true
    }

    pub const fn selected_member(&self) -> Option<usize> {
        self.selected_member
    }

    pub fn editing_sim(&self) -> Option<&SimDraft> {
        self.editing_sim.as_ref().map(|(_, sim)| sim)
    }

    pub fn editing_sim_mut(&mut self) -> Option<&mut SimDraft> {
        self.editing_sim.as_mut().map(|(_, sim)| sim)
    }

    pub const fn confirm(&self) -> Option<ConfirmKind> {
        self.confirm
    }

    pub const fn focus(&self) -> FocusField {
        self.focus
    }

    pub const fn hovered(&self) -> Option<ControlId> {
        self.hovered
    }

    pub const fn texts(&self) -> &CafTexts {
        &self.texts
    }

    /// Updates available head/body catalog counts for the currently edited Sim.
    pub fn set_skin_catalog_counts(&mut self, head_count: usize, body_count: usize) {
        if let Some((_, sim)) = &mut self.editing_sim {
            sim.appearance.set_catalog_counts(head_count, body_count);
        }
    }

    /// Advances the text-field caret blink timer (`500 ms` half-period).
    pub fn tick(&mut self, clock_us: u64) -> bool {
        self.clock_us = clock_us;
        self.text_state.sync_blink(clock_us)
    }

    /// Current tooltip string for the hovered control, if any.
    pub fn tooltip(&self) -> Option<&str> {
        let control = self.hovered?;
        let t = &self.texts;
        Some(match control {
            ControlId::FamilyAdd => &t.tip_add_sim,
            ControlId::FamilyDelete => &t.tip_delete_sim,
            ControlId::FamilyEdit => &t.tip_edit_sim,
            ControlId::FamilyDone => &t.tip_family_done,
            ControlId::FamilyCancel => &t.tip_family_cancel,
            ControlId::SimAdult => &t.tip_adult,
            ControlId::SimChild => &t.tip_child,
            ControlId::SimFemale => &t.tip_female,
            ControlId::SimMale => &t.tip_male,
            ControlId::SimDark => &t.tip_dark,
            ControlId::SimMedium => &t.tip_medium,
            ControlId::SimLight => &t.tip_light,
            ControlId::HeadPrev => &t.tip_prev_head,
            ControlId::HeadNext => &t.tip_next_head,
            ControlId::BodyPrev => &t.tip_prev_body,
            ControlId::BodyNext => &t.tip_next_body,
            ControlId::ZodiacBadge => &t.zodiac_tooltip,
            ControlId::SimDone => &t.tip_sim_done,
            ControlId::SimCancel => &t.tip_sim_cancel,
            ControlId::MemberSlot(_)
            | ControlId::RotateLeft
            | ControlId::RotateRight
            | ControlId::TraitSlider(_)
            | ControlId::DialogPrimary
            | ControlId::DialogSecondary => return None,
        })
    }

    fn clear_pointer_capture(&mut self) {
        self.pressed = None;
        self.active_slider = None;
        self.dragging_preview_x = None;
    }

    /// Opens the Create-a-Sim screen for a new Sim (if under [`MAX_MEMBERS`]).
    pub fn begin_add_sim(&mut self) -> Option<CafCommand> {
        if !self.family.can_add_member() {
            return None;
        }
        self.clear_pointer_capture();
        self.editing_sim = Some((None, SimDraft::default()));
        self.screen = CafScreen::EditSim;
        self.confirm = None;
        self.focus = FocusField::SimFirstName;
        self.text_state.reset_to_end("", self.clock_us);
        Some(CafCommand::OpenSimEditor { index: None })
    }

    /// Opens the Create-a-Sim screen to edit household member `index`.
    pub fn begin_edit_sim(&mut self, index: usize) -> Option<CafCommand> {
        let member = self.family.members.get(index)?.clone();
        if member.kind != MemberKind::Sim {
            return None;
        }
        self.clear_pointer_capture();
        let name_len = member.first_name.chars().count();
        self.editing_sim = Some((Some(index), member));
        self.screen = CafScreen::EditSim;
        self.confirm = None;
        self.focus = FocusField::SimFirstName;
        self.text_state.caret = name_len;
        self.text_state.caret_visible = true;
        self.text_state.blink_since_us = self.clock_us;
        Some(CafCommand::OpenSimEditor { index: Some(index) })
    }

    /// Handles pointer motion for hover state, active slider drags, and
    /// turntable preview rotation drags.
    pub fn pointer_move(&mut self, x: i32, y: i32, frame_size: [u32; 2]) -> Option<CafCommand> {
        let layout = CafLayout::compute(frame_size[0], frame_size[1]);
        self.hovered = self.hit_test(x, y, &layout);

        if self.confirm.is_some() {
            return None;
        }

        if let Some(trait_) = self.active_slider {
            self.apply_slider_at(trait_, x, &layout);
            return None;
        }

        if let Some(prev_x) = self.dragging_preview_x {
            let dx = x - prev_x;
            self.dragging_preview_x = Some(x);
            if dx != 0
                && let Some((_, sim)) = &mut self.editing_sim
            {
                sim.appearance.rotate_preview(dx as f32 * 1.5);
                return Some(CafCommand::RotatePreview(sim.appearance.preview_yaw_deg));
            }
        }
        None
    }

    /// Handles a left mouse button press at `(x, y)` in frame coordinates.
    pub fn pointer_down(&mut self, x: i32, y: i32, frame_size: [u32; 2]) -> Option<CafCommand> {
        let layout = CafLayout::compute(frame_size[0], frame_size[1]);
        let hit = self.hit_test(x, y, &layout);
        self.hovered = hit;
        self.pressed = hit;

        if let Some(kind) = self.confirm {
            return match hit {
                Some(ControlId::DialogPrimary) => self.accept_confirm(kind),
                Some(ControlId::DialogSecondary) => {
                    self.confirm = None;
                    None
                }
                _ => None,
            };
        }

        match self.screen {
            CafScreen::Family => self.handle_family_click(x, y, hit, &layout),
            CafScreen::EditSim => self.handle_sim_click(x, y, hit, &layout),
        }
    }

    /// Handles left mouse button release.
    pub fn pointer_up(&mut self) {
        self.pressed = None;
        self.active_slider = None;
        self.dragging_preview_x = None;
    }

    /// Handles a typed character into the currently focused text input.
    pub fn key_char(&mut self, ch: char) -> bool {
        if self.confirm.is_some() {
            return false;
        }
        let clock = self.clock_us;
        let focus = self.focus;
        let mut state = self.text_state;
        let changed = match self.active_text_mut() {
            Some((text, max_chars, multiline)) => {
                let ok = state.insert_char(text, ch, max_chars, multiline);
                if ok {
                    state.blink_since_us = clock;
                }
                ok
            }
            None => false,
        };
        if changed {
            self.focus = focus;
            self.text_state = state;
        }
        changed
    }

    pub fn key_backspace(&mut self) -> bool {
        if self.confirm.is_some() {
            return false;
        }
        let mut state = self.text_state;
        let changed = self
            .active_text_mut()
            .is_some_and(|(text, _, _)| state.backspace(text));
        if changed {
            state.blink_since_us = self.clock_us;
            self.text_state = state;
        }
        changed
    }

    pub fn key_delete(&mut self) -> bool {
        if self.confirm.is_some() {
            return false;
        }
        let mut state = self.text_state;
        let changed = self
            .active_text_mut()
            .is_some_and(|(text, _, _)| state.delete(text));
        if changed {
            state.blink_since_us = self.clock_us;
            self.text_state = state;
        }
        changed
    }

    pub fn key_left(&mut self) {
        if self.confirm.is_none() {
            self.text_state.move_left();
            self.text_state.blink_since_us = self.clock_us;
        }
    }

    pub fn key_right(&mut self) {
        if self.confirm.is_none() {
            let len = self.active_text().map_or(0, |t| t.chars().count());
            self.text_state.caret = (self.text_state.caret + 1).min(len);
            self.text_state.caret_visible = true;
            self.text_state.blink_since_us = self.clock_us;
        }
    }

    pub fn key_home(&mut self) {
        if self.confirm.is_none() {
            self.text_state.move_home();
            self.text_state.blink_since_us = self.clock_us;
        }
    }

    pub fn key_end(&mut self) {
        if self.confirm.is_none() {
            let len = self.active_text().map_or(0, |t| t.chars().count());
            self.text_state.caret = len;
            self.text_state.caret_visible = true;
            self.text_state.blink_since_us = self.clock_us;
        }
    }

    /// Cycles focus between the active screen's name and bio fields.
    pub fn key_tab(&mut self) {
        if self.confirm.is_some() {
            return;
        }
        self.focus = match (self.screen, self.focus) {
            (CafScreen::Family, FocusField::FamilyLastName) => FocusField::FamilyBio,
            (CafScreen::Family, _) => FocusField::FamilyLastName,
            (CafScreen::EditSim, FocusField::SimFirstName) => FocusField::SimBio,
            (CafScreen::EditSim, _) => FocusField::SimFirstName,
        };
        let len = self.active_text().map_or(0, |t| t.chars().count());
        self.text_state.caret = len;
        self.text_state.caret_visible = true;
        self.text_state.blink_since_us = self.clock_us;
    }

    /// Handles `Enter`: confirms an open dialog, or inserts a newline in a
    /// focused bio field.
    pub fn key_enter(&mut self) -> Option<CafCommand> {
        if let Some(kind) = self.confirm {
            return self.accept_confirm(kind);
        }
        if matches!(self.focus, FocusField::FamilyBio | FocusField::SimBio) {
            self.key_char('\n');
        }
        None
    }

    /// Handles `Escape`: dismisses an open confirmation dialog, or triggers
    /// Cancel on the active screen.
    pub fn key_escape(&mut self) -> Option<CafCommand> {
        if self.confirm.take().is_some() {
            return None;
        }
        match self.screen {
            CafScreen::Family => self.request_cancel_family(),
            CafScreen::EditSim => self.cancel_sim(),
        }
    }

    fn active_text(&self) -> Option<&str> {
        match (self.screen, self.focus) {
            (CafScreen::Family, FocusField::FamilyLastName) => Some(&self.family.last_name),
            (CafScreen::Family, FocusField::FamilyBio) => Some(&self.family.bio),
            (CafScreen::EditSim, FocusField::SimFirstName) => self
                .editing_sim
                .as_ref()
                .map(|(_, s)| s.first_name.as_str()),
            (CafScreen::EditSim, FocusField::SimBio) => {
                self.editing_sim.as_ref().map(|(_, s)| s.bio.as_str())
            }
            _ => None,
        }
    }

    fn active_text_mut(&mut self) -> Option<(&mut String, usize, bool)> {
        match (self.screen, self.focus) {
            (CafScreen::Family, FocusField::FamilyLastName) => {
                Some((&mut self.family.last_name, NAME_MAX, false))
            }
            (CafScreen::Family, FocusField::FamilyBio) => {
                Some((&mut self.family.bio, BIO_MAX, true))
            }
            (CafScreen::EditSim, FocusField::SimFirstName) => self
                .editing_sim
                .as_mut()
                .map(|(_, s)| (&mut s.first_name, NAME_MAX, false)),
            (CafScreen::EditSim, FocusField::SimBio) => self
                .editing_sim
                .as_mut()
                .map(|(_, s)| (&mut s.bio, BIO_MAX, true)),
            _ => None,
        }
    }

    fn set_focus(&mut self, field: FocusField) {
        self.focus = field;
        let len = self.active_text().map_or(0, |t| t.chars().count());
        self.text_state.caret = len;
        self.text_state.caret_visible = true;
        self.text_state.blink_since_us = self.clock_us;
    }

    fn handle_family_click(
        &mut self,
        x: i32,
        y: i32,
        hit: Option<ControlId>,
        layout: &CafLayout,
    ) -> Option<CafCommand> {
        if layout.family_name_field.contains(x, y) {
            self.set_focus(FocusField::FamilyLastName);
            return None;
        }
        if layout.family_bio_field.contains(x, y) {
            self.set_focus(FocusField::FamilyBio);
            return None;
        }

        match hit {
            Some(ControlId::MemberSlot(i)) => {
                if i < self.family.members.len() {
                    self.selected_member = Some(i);
                }
                None
            }
            Some(ControlId::FamilyAdd) => self.begin_add_sim(),
            Some(ControlId::FamilyEdit) => {
                let idx = self.selected_member?;
                self.begin_edit_sim(idx)
            }
            Some(ControlId::FamilyDelete) => {
                let idx = self.selected_member?;
                if idx < self.family.members.len() {
                    self.confirm = Some(ConfirmKind::DeleteSim(idx));
                }
                None
            }
            Some(ControlId::FamilyDone) => {
                if self.family.can_finish() {
                    self.confirm = Some(ConfirmKind::FinishFamily);
                }
                None
            }
            Some(ControlId::FamilyCancel) => self.request_cancel_family(),
            _ => None,
        }
    }

    fn handle_sim_click(
        &mut self,
        x: i32,
        y: i32,
        hit: Option<ControlId>,
        layout: &CafLayout,
    ) -> Option<CafCommand> {
        if layout.sim_name_field.contains(x, y) {
            self.set_focus(FocusField::SimFirstName);
            return None;
        }
        if layout.sim_bio_field.contains(x, y) {
            self.set_focus(FocusField::SimBio);
            return None;
        }

        match hit {
            Some(ControlId::SimAdult) => self.set_age(Age::Adult),
            Some(ControlId::SimChild) => self.set_age(Age::Child),
            Some(ControlId::SimLight) => self.set_skin(SkinTone::Light),
            Some(ControlId::SimMedium) => self.set_skin(SkinTone::Medium),
            Some(ControlId::SimDark) => self.set_skin(SkinTone::Dark),
            Some(ControlId::SimMale) => self.set_gender(Gender::Male),
            Some(ControlId::SimFemale) => self.set_gender(Gender::Female),
            Some(ControlId::HeadPrev) => {
                let (_, sim) = self.editing_sim.as_mut()?;
                let prev = sim.appearance.head_index;
                sim.appearance.prev_head();
                (sim.appearance.head_index != prev)
                    .then(|| CafCommand::AppearanceChanged(sim.appearance.clone()))
            }
            Some(ControlId::HeadNext) => {
                let (_, sim) = self.editing_sim.as_mut()?;
                let prev = sim.appearance.head_index;
                sim.appearance.next_head();
                (sim.appearance.head_index != prev)
                    .then(|| CafCommand::AppearanceChanged(sim.appearance.clone()))
            }
            Some(ControlId::BodyPrev) => {
                let (_, sim) = self.editing_sim.as_mut()?;
                let prev = sim.appearance.body_index;
                sim.appearance.prev_body();
                (sim.appearance.body_index != prev)
                    .then(|| CafCommand::AppearanceChanged(sim.appearance.clone()))
            }
            Some(ControlId::BodyNext) => {
                let (_, sim) = self.editing_sim.as_mut()?;
                let prev = sim.appearance.body_index;
                sim.appearance.next_body();
                (sim.appearance.body_index != prev)
                    .then(|| CafCommand::AppearanceChanged(sim.appearance.clone()))
            }
            Some(ControlId::RotateLeft) => {
                let (_, sim) = self.editing_sim.as_mut()?;
                sim.appearance.rotate_preview(-30.0);
                Some(CafCommand::RotatePreview(sim.appearance.preview_yaw_deg))
            }
            Some(ControlId::RotateRight) => {
                let (_, sim) = self.editing_sim.as_mut()?;
                sim.appearance.rotate_preview(30.0);
                Some(CafCommand::RotatePreview(sim.appearance.preview_yaw_deg))
            }
            Some(ControlId::TraitSlider(trait_)) => {
                self.active_slider = Some(trait_);
                self.apply_slider_at(trait_, x, layout);
                None
            }
            Some(ControlId::SimDone) => self.request_finish_sim(),
            Some(ControlId::SimCancel) => self.cancel_sim(),
            _ => {
                if layout.preview_rect.contains(x, y) {
                    self.dragging_preview_x = Some(x);
                }
                None
            }
        }
    }

    fn set_age(&mut self, age: Age) -> Option<CafCommand> {
        let (_, sim) = self.editing_sim.as_mut()?;
        if sim.appearance.age != age {
            sim.appearance.age = age;
            sim.appearance.head_index = 0;
            sim.appearance.body_index = 0;
            return Some(CafCommand::AppearanceChanged(sim.appearance.clone()));
        }
        None
    }

    fn set_gender(&mut self, gender: Gender) -> Option<CafCommand> {
        let (_, sim) = self.editing_sim.as_mut()?;
        if sim.appearance.gender != gender {
            sim.appearance.gender = gender;
            sim.appearance.head_index = 0;
            sim.appearance.body_index = 0;
            return Some(CafCommand::AppearanceChanged(sim.appearance.clone()));
        }
        None
    }

    fn set_skin(&mut self, skin: SkinTone) -> Option<CafCommand> {
        let (_, sim) = self.editing_sim.as_mut()?;
        if sim.appearance.skin_tone != skin {
            sim.appearance.skin_tone = skin;
            return Some(CafCommand::AppearanceChanged(sim.appearance.clone()));
        }
        None
    }

    fn apply_slider_at(&mut self, trait_: Trait, x: i32, layout: &CafLayout) {
        let rect = layout.trait_sliders[trait_.index()];
        if rect.width <= 0 {
            return;
        }
        let rel = x - rect.x;
        let target = if rel <= 0 {
            0
        } else {
            let pip = ((i64::from(rel) * i64::from(TRAIT_MAX)) / i64::from(rect.width)) + 1;
            pip.clamp(1, i64::from(TRAIT_MAX)) as u8
        };
        if let Some((_, sim)) = &mut self.editing_sim {
            sim.personality.set(trait_, target);
        }
    }

    fn request_finish_sim(&mut self) -> Option<CafCommand> {
        let (_, sim) = self.editing_sim.as_ref()?;
        if sim.first_name.trim().is_empty() {
            return None;
        }
        if sim.personality.remaining() > 0 {
            self.clear_pointer_capture();
            self.confirm = Some(ConfirmKind::MorePersonality);
            return None;
        }
        self.commit_sim()
    }

    fn commit_sim(&mut self) -> Option<CafCommand> {
        self.clear_pointer_capture();
        let (slot, sim) = self.editing_sim.take()?;
        let (index, is_new) = match slot {
            Some(i) if i < self.family.members.len() => {
                self.family.members[i] = sim;
                (i, false)
            }
            _ => {
                if !self.family.can_add_member() {
                    self.screen = CafScreen::Family;
                    return None;
                }
                self.family.members.push(sim);
                (self.family.members.len() - 1, true)
            }
        };
        self.selected_member = Some(index);
        self.screen = CafScreen::Family;
        self.confirm = None;
        self.set_focus(FocusField::FamilyLastName);
        Some(CafCommand::SimSaved { index, is_new })
    }

    fn cancel_sim(&mut self) -> Option<CafCommand> {
        self.clear_pointer_capture();
        self.editing_sim = None;
        self.screen = CafScreen::Family;
        self.confirm = None;
        self.set_focus(FocusField::FamilyLastName);
        Some(CafCommand::CancelSim)
    }

    fn request_cancel_family(&mut self) -> Option<CafCommand> {
        self.clear_pointer_capture();
        if self.family.has_changes() {
            self.confirm = Some(ConfirmKind::CancelFamily);
            None
        } else {
            Some(CafCommand::CancelFamily)
        }
    }

    fn accept_confirm(&mut self, kind: ConfirmKind) -> Option<CafCommand> {
        self.confirm = None;
        match kind {
            ConfirmKind::DeleteSim(idx) => {
                if idx < self.family.members.len() {
                    self.family.members.remove(idx);
                    self.selected_member = if self.family.members.is_empty() {
                        None
                    } else {
                        Some(idx.min(self.family.members.len() - 1))
                    };
                    return Some(CafCommand::SimDeleted(idx));
                }
                None
            }
            ConfirmKind::CancelFamily => Some(CafCommand::CancelFamily),
            ConfirmKind::FinishFamily => {
                if self.family.can_finish() {
                    Some(CafCommand::CommitFamily(self.family.clone()))
                } else {
                    None
                }
            }
            ConfirmKind::MorePersonality => self.commit_sim(),
        }
    }

    fn hit_test(&self, x: i32, y: i32, layout: &CafLayout) -> Option<ControlId> {
        if self.confirm.is_some() {
            if layout.dialog_primary.contains(x, y) {
                return Some(ControlId::DialogPrimary);
            }
            if layout.dialog_secondary.contains(x, y) {
                return Some(ControlId::DialogSecondary);
            }
            return None;
        }

        match self.screen {
            CafScreen::Family => {
                if layout.family_add.contains(x, y) {
                    return Some(ControlId::FamilyAdd);
                }
                if layout.family_delete.contains(x, y) {
                    return Some(ControlId::FamilyDelete);
                }
                if layout.family_edit.contains(x, y) {
                    return Some(ControlId::FamilyEdit);
                }
                if layout.family_done.contains(x, y) {
                    return Some(ControlId::FamilyDone);
                }
                if layout.family_cancel.contains(x, y) {
                    return Some(ControlId::FamilyCancel);
                }
                for (i, slot) in layout.family_slots.iter().enumerate() {
                    if i < self.family.members.len() && slot.contains(x, y) {
                        return Some(ControlId::MemberSlot(i));
                    }
                }
                None
            }
            CafScreen::EditSim => {
                if layout.sim_child.contains(x, y) {
                    return Some(ControlId::SimChild);
                }
                if layout.sim_adult.contains(x, y) {
                    return Some(ControlId::SimAdult);
                }
                if layout.sim_light.contains(x, y) {
                    return Some(ControlId::SimLight);
                }
                if layout.sim_medium.contains(x, y) {
                    return Some(ControlId::SimMedium);
                }
                if layout.sim_dark.contains(x, y) {
                    return Some(ControlId::SimDark);
                }
                if layout.sim_male.contains(x, y) {
                    return Some(ControlId::SimMale);
                }
                if layout.sim_female.contains(x, y) {
                    return Some(ControlId::SimFemale);
                }
                if layout.head_prev.contains(x, y) {
                    return Some(ControlId::HeadPrev);
                }
                if layout.head_next.contains(x, y) {
                    return Some(ControlId::HeadNext);
                }
                if layout.body_prev.contains(x, y) {
                    return Some(ControlId::BodyPrev);
                }
                if layout.body_next.contains(x, y) {
                    return Some(ControlId::BodyNext);
                }
                if layout.rotate_left.contains(x, y) {
                    return Some(ControlId::RotateLeft);
                }
                if layout.rotate_right.contains(x, y) {
                    return Some(ControlId::RotateRight);
                }
                for trait_ in Trait::ALL {
                    if layout.trait_sliders[trait_.index()].contains(x, y) {
                        return Some(ControlId::TraitSlider(trait_));
                    }
                }
                if layout.zodiac_rect.contains(x, y) {
                    return Some(ControlId::ZodiacBadge);
                }
                if layout.sim_done.contains(x, y) {
                    return Some(ControlId::SimDone);
                }
                if layout.sim_cancel.contains(x, y) {
                    return Some(ControlId::SimCancel);
                }
                None
            }
        }
    }

    /// Renders the active CAF screen into `frame` using loaded `art` and an
    /// optional 3D turntable `preview_image` (for Create-a-Sim).
    pub fn draw(&self, frame: &mut Frame, art: &Art, preview_image: Option<&Image>) {
        let layout = CafLayout::compute(frame.width(), frame.height());
        frame.fill(
            Rect::new(
                0,
                0,
                i32::try_from(frame.width()).unwrap_or(0),
                i32::try_from(frame.height()).unwrap_or(0),
            ),
            FALLBACK_BG,
        );

        match self.screen {
            CafScreen::Family => self.draw_family(frame, art, &layout),
            CafScreen::EditSim => self.draw_sim(frame, art, &layout, preview_image),
        }

        if let Some(kind) = self.confirm {
            self.draw_confirm(frame, art, &layout, kind);
        } else if let Some(tip) = self.tooltip() {
            self.draw_tooltip(frame, art, &layout, tip);
        }
    }

    fn draw_family(&self, frame: &mut Frame, art: &Art, layout: &CafLayout) {
        let has_bkg = art.image(id::FAMILY_BKG).is_some();
        if let Some(bg) = art.image(id::FAMILY_BKG) {
            draw_scaled_or_direct(frame, bg, layout.canvas, layout.compact);
        } else {
            draw_well(frame, layout.family_roster_well, false);
        }

        let title_font = art.font(14).or_else(|| art.font(12));
        let body_font = art.font(12).or_else(|| art.font(11));
        let small_font = art.font(10).or(body_font);

        if let Some(font) = title_font {
            let w = font.width(&self.texts.family_title);
            font.draw(
                frame,
                &self.texts.family_title,
                [
                    layout.family_title_pos[0] - w / 2,
                    layout.family_title_pos[1],
                ],
                LABEL,
            );
        }

        if let Some(font) = body_font {
            let lw = font.width(&self.texts.enter_last_name);
            font.draw(
                frame,
                &self.texts.enter_last_name,
                [
                    layout.family_name_label_right[0] - lw,
                    layout.family_name_label_right[1],
                ],
                LABEL,
            );
        }

        let name_focused = self.confirm.is_none() && self.focus == FocusField::FamilyLastName;
        draw_text_box(
            frame,
            body_font,
            layout.family_name_field,
            &self.family.last_name,
            name_focused,
            &self.text_state,
            false,
            !has_bkg,
        );

        // Family bio box: 1.91 `DsgnFamBkg.bmp` has no separate bio well below the
        // roster, so only paint the optional bio box when focused, non-empty, or
        // in headless/fallback mode without background art.
        let bio_focused = self.confirm.is_none() && self.focus == FocusField::FamilyBio;
        if !has_bkg || bio_focused || !self.family.bio.is_empty() {
            if let Some(font) = small_font {
                font.draw(
                    frame,
                    &self.texts.bio_label,
                    layout.family_bio_label_pos,
                    LABEL,
                );
            }
            draw_text_box(
                frame,
                small_font,
                layout.family_bio_field,
                &self.family.bio,
                bio_focused,
                &self.text_state,
                true,
                true,
            );
        }

        // Up to 8 member portrait slots inside the roster well.
        for (i, member) in self.family.members.iter().take(MAX_MEMBERS).enumerate() {
            let slot = layout.family_slots[i];
            let selected = self.selected_member == Some(i);
            let portrait_side = (slot.width.min(slot.height - 20)).clamp(20, 48);
            let portrait_rect = Rect::new(
                slot.x + (slot.width - portrait_side) / 2,
                slot.y + 4,
                portrait_side,
                portrait_side,
            );
            frame.fill(portrait_rect, PORTRAIT_BG);
            if let Some(img) = &member.portrait {
                draw_scaled_or_direct(frame, img, portrait_rect, true);
            } else {
                // Simple placeholder silhouette + species marker for headless tests.
                let inner = Rect::new(
                    portrait_rect.x + 8,
                    portrait_rect.y + 8,
                    (portrait_rect.width - 16).max(4),
                    (portrait_rect.height - 16).max(4),
                );
                let tone = match member.kind {
                    MemberKind::Sim => [0x74, 0x98, 0xc8],
                    MemberKind::Dog => [0xc8, 0x98, 0x58],
                    MemberKind::Cat => [0xb8, 0x84, 0xc8],
                };
                frame.fill(inner, tone);
            }
            draw_outline(
                frame,
                portrait_rect,
                if selected {
                    PORTRAIT_SELECTED
                } else {
                    WELL_BORDER
                },
            );
            if let Some(font) = small_font {
                let nw = font.width(&member.first_name);
                font.draw(
                    frame,
                    &member.first_name,
                    [
                        slot.x + (slot.width - nw) / 2,
                        portrait_rect.y + portrait_rect.height + 6,
                    ],
                    if selected { PORTRAIT_SELECTED } else { LABEL },
                );
            }
        }

        // Right semicircular action cluster: Add, Delete, Edit.
        let has_sel = self
            .selected_member
            .is_some_and(|i| i < self.family.members.len());
        let can_edit = self
            .selected_member
            .and_then(|i| self.family.members.get(i))
            .is_some_and(|m| m.kind == MemberKind::Sim);

        self.draw_strip_button(
            frame,
            art,
            id::FAMILY_ADD,
            4,
            layout.family_add,
            ControlId::FamilyAdd,
            self.family.can_add_member(),
            false,
            layout.compact,
        );
        self.draw_strip_button(
            frame,
            art,
            id::FAMILY_DELETE,
            4,
            layout.family_delete,
            ControlId::FamilyDelete,
            has_sel,
            false,
            layout.compact,
        );
        self.draw_strip_button(
            frame,
            art,
            id::FAMILY_EDIT,
            4,
            layout.family_edit,
            ControlId::FamilyEdit,
            can_edit,
            false,
            layout.compact,
        );

        // Done & Cancel pill buttons.
        self.draw_labeled_pill_button(
            frame,
            art,
            id::FAMILY_DONE,
            6,
            layout.family_done,
            ControlId::FamilyDone,
            self.family.can_finish(),
            &self.texts.family_done,
            layout.compact,
        );
        self.draw_labeled_pill_button(
            frame,
            art,
            id::FAMILY_CANCEL,
            6,
            layout.family_cancel,
            ControlId::FamilyCancel,
            true,
            &self.texts.family_cancel,
            layout.compact,
        );
    }

    fn draw_sim(
        &self,
        frame: &mut Frame,
        art: &Art,
        layout: &CafLayout,
        preview_image: Option<&Image>,
    ) {
        let Some((_, sim)) = &self.editing_sim else {
            return;
        };

        let has_bkg = art.image(id::CHAR_BKG).is_some();
        if let Some(bg) = art.image(id::CHAR_BKG) {
            draw_scaled_or_direct(frame, bg, layout.canvas, layout.compact);
        } else {
            draw_well(frame, layout.personality_well, false);
            draw_well(frame, layout.preview_rect, false);
        }

        let title_font = art.font(14).or_else(|| art.font(12));
        let body_font = art.font(12).or_else(|| art.font(11));
        let small_font = art.font(10).or(body_font);

        if let Some(font) = title_font {
            let w = font.width(&self.texts.sim_title);
            font.draw(
                frame,
                &self.texts.sim_title,
                [layout.sim_title_pos[0] - w / 2, layout.sim_title_pos[1]],
                LABEL,
            );
        }

        if let Some(font) = body_font {
            let lw = font.width(&self.texts.enter_first_name);
            font.draw(
                frame,
                &self.texts.enter_first_name,
                [
                    layout.sim_name_label_right[0] - lw,
                    layout.sim_name_label_right[1],
                ],
                LABEL,
            );

            let pw = font.width(&self.texts.personality_heading);
            font.draw(
                frame,
                &self.texts.personality_heading,
                [
                    layout.personality_heading_pos[0] - pw / 2,
                    layout.personality_heading_pos[1],
                ],
                LABEL,
            );
        }

        let name_focused = self.confirm.is_none() && self.focus == FocusField::SimFirstName;
        draw_text_box(
            frame,
            body_font,
            layout.sim_name_field,
            &sim.first_name,
            name_focused,
            &self.text_state,
            false,
            !has_bkg,
        );

        // 5 Personality sliders + LEDs.
        let led_art = art.image(id::CHAR_PERS_LED);
        for trait_ in Trait::ALL {
            let idx = trait_.index();
            let slider = layout.trait_sliders[idx];
            if let Some(font) = body_font {
                let label = &self.texts.trait_names[idx];
                let tw = font.width(label);
                font.draw(
                    frame,
                    label,
                    [
                        layout.trait_label_right_x - tw,
                        slider.y + (slider.height - font.line_height) / 2,
                    ],
                    LABEL,
                );
            }
            draw_personality_bar(
                frame,
                led_art,
                slider,
                sim.personality.get(trait_),
                TRAIT_MAX,
            );
        }

        // Live zodiac sign below Nice when at least 1 personality point is spent.
        if let Some(sign) = sim.personality.zodiac()
            && let Some(font) = body_font
        {
            let text = format!("({})", self.texts.zodiac_name(sign));
            let tw = font.width(&text);
            font.draw(
                frame,
                &text,
                [
                    layout.zodiac_rect.x + (layout.zodiac_rect.width - tw) / 2,
                    layout.zodiac_rect.y + (layout.zodiac_rect.height - font.line_height) / 2,
                ],
                LABEL,
            );
        }

        // Remaining 25-point budget bar.
        draw_personality_bar(
            frame,
            art.image(id::CHAR_POINTS_BAR),
            layout.remaining_points_rect,
            sim.personality.remaining(),
            PERSONALITY_BUDGET,
        );

        // Age, skin-tone, and gender toggle cluster.
        self.draw_strip_button(
            frame,
            art,
            id::CHAR_CHILD,
            4,
            layout.sim_child,
            ControlId::SimChild,
            true,
            sim.appearance.age == Age::Child,
            layout.compact,
        );
        self.draw_strip_button(
            frame,
            art,
            id::CHAR_ADULT,
            4,
            layout.sim_adult,
            ControlId::SimAdult,
            true,
            sim.appearance.age == Age::Adult,
            layout.compact,
        );
        self.draw_strip_button(
            frame,
            art,
            id::CHAR_LIGHT,
            4,
            layout.sim_light,
            ControlId::SimLight,
            true,
            sim.appearance.skin_tone == SkinTone::Light,
            layout.compact,
        );
        self.draw_strip_button(
            frame,
            art,
            id::CHAR_MEDIUM,
            4,
            layout.sim_medium,
            ControlId::SimMedium,
            true,
            sim.appearance.skin_tone == SkinTone::Medium,
            layout.compact,
        );
        self.draw_strip_button(
            frame,
            art,
            id::CHAR_DARK,
            4,
            layout.sim_dark,
            ControlId::SimDark,
            true,
            sim.appearance.skin_tone == SkinTone::Dark,
            layout.compact,
        );
        self.draw_strip_button(
            frame,
            art,
            id::CHAR_MALE,
            4,
            layout.sim_male,
            ControlId::SimMale,
            true,
            sim.appearance.gender == Gender::Male,
            layout.compact,
        );
        self.draw_strip_button(
            frame,
            art,
            id::CHAR_FEMALE,
            4,
            layout.sim_female,
            ControlId::SimFemale,
            true,
            sim.appearance.gender == Gender::Female,
            layout.compact,
        );

        // 3D turntable preview image (rendered by Track 5's `sims-people`), if supplied.
        if let Some(img) = preview_image {
            draw_scaled_or_direct(frame, img, layout.preview_rect, true);
        }

        // Head & body prev/next arrows and turntable rotate controls over the preview viewport.
        let heads_enabled = sim.appearance.head_count > 1;
        let bodies_enabled = sim.appearance.body_count > 1;
        self.draw_strip_button(
            frame,
            art,
            id::CHAR_SKINS_LEFT,
            4,
            layout.head_prev,
            ControlId::HeadPrev,
            heads_enabled,
            false,
            layout.compact,
        );
        self.draw_strip_button(
            frame,
            art,
            id::CHAR_SKINS_RIGHT,
            4,
            layout.head_next,
            ControlId::HeadNext,
            heads_enabled,
            false,
            layout.compact,
        );
        self.draw_strip_button(
            frame,
            art,
            id::CHAR_SKINS_LEFT,
            4,
            layout.body_prev,
            ControlId::BodyPrev,
            bodies_enabled,
            false,
            layout.compact,
        );
        self.draw_strip_button(
            frame,
            art,
            id::CHAR_SKINS_RIGHT,
            4,
            layout.body_next,
            ControlId::BodyNext,
            bodies_enabled,
            false,
            layout.compact,
        );
        self.draw_strip_button(
            frame,
            art,
            id::CHAR_SKINS_LEFT,
            4,
            layout.rotate_left,
            ControlId::RotateLeft,
            true,
            false,
            true,
        );
        self.draw_strip_button(
            frame,
            art,
            id::CHAR_SKINS_RIGHT,
            4,
            layout.rotate_right,
            ControlId::RotateRight,
            true,
            false,
            true,
        );

        // Bio heading + multiline bio input.
        if let Some(font) = small_font {
            let heading = self
                .texts
                .format_bio_heading(&sim.first_name, &self.family.last_name);
            font.draw(frame, &heading, layout.sim_bio_label_pos, LABEL);
        }
        let bio_focused = self.confirm.is_none() && self.focus == FocusField::SimBio;
        draw_text_box(
            frame,
            small_font,
            layout.sim_bio_field,
            &sim.bio,
            bio_focused,
            &self.text_state,
            true,
            !has_bkg,
        );

        // Done & Cancel pill buttons.
        let can_done = !sim.first_name.trim().is_empty();
        self.draw_labeled_pill_button(
            frame,
            art,
            id::CHAR_DONE,
            6,
            layout.sim_done,
            ControlId::SimDone,
            can_done,
            &self.texts.sim_done,
            layout.compact,
        );
        self.draw_labeled_pill_button(
            frame,
            art,
            id::CHAR_CANCEL,
            6,
            layout.sim_cancel,
            ControlId::SimCancel,
            true,
            &self.texts.sim_cancel,
            layout.compact,
        );
    }

    fn draw_confirm(&self, frame: &mut Frame, art: &Art, layout: &CafLayout, kind: ConfirmKind) {
        let (title, body, primary_label, secondary_label) = match kind {
            ConfirmKind::DeleteSim(_) => (
                &self.texts.confirm_delete_title,
                &self.texts.confirm_delete_body,
                &self.texts.btn_yes,
                &self.texts.btn_no,
            ),
            ConfirmKind::CancelFamily => (
                &self.texts.confirm_cancel_title,
                &self.texts.confirm_cancel_body,
                &self.texts.btn_yes,
                &self.texts.btn_no,
            ),
            ConfirmKind::FinishFamily => (
                &self.texts.confirm_finish_title,
                &self.texts.confirm_finish_body,
                &self.texts.btn_ok,
                &self.texts.btn_cancel,
            ),
            ConfirmKind::MorePersonality => (
                &self.texts.confirm_personality_title,
                &self.texts.confirm_personality_body,
                &self.texts.btn_yes,
                &self.texts.btn_no,
            ),
        };

        if let Some(sheet) = art.image(dialog::BACKGROUND) {
            dialog::draw_nine_slice(frame, sheet, layout.dialog_rect);
        } else {
            frame.fill(layout.dialog_rect, ENTRY_FILL);
            draw_outline(frame, layout.dialog_rect, WELL_BORDER);
        }

        if let Some(font) = art.font(12).or_else(|| art.font(11)) {
            let pad_x = (layout.dialog_rect.width / 20).clamp(6, 18);
            let mut y = layout.dialog_rect.y + (layout.dialog_rect.height / 10).clamp(6, 18);
            let body_bottom = layout.dialog_primary.y - 4;
            if y + font.line_height <= body_bottom {
                font.draw(frame, title, [layout.dialog_rect.x + pad_x, y], LABEL);
                y += font.line_height + (layout.dialog_rect.height / 18).clamp(4, 10);
            }

            let max_w = (layout.dialog_rect.width - pad_x * 2).max(20);
            for line in font.break_lines(body, max_w) {
                if y + font.line_height > body_bottom {
                    break;
                }
                let lw = font.width(&line);
                let lx = layout.dialog_rect.x + (layout.dialog_rect.width - lw) / 2;
                font.draw(frame, &line, [lx, y], LABEL);
                y += font.line_height + 2;
            }
        }

        self.draw_dialog_button(
            frame,
            art,
            layout.dialog_primary,
            ControlId::DialogPrimary,
            primary_label,
        );
        self.draw_dialog_button(
            frame,
            art,
            layout.dialog_secondary,
            ControlId::DialogSecondary,
            secondary_label,
        );
    }

    fn draw_dialog_button(
        &self,
        frame: &mut Frame,
        art: &Art,
        rect: Rect,
        id_: ControlId,
        label: &str,
    ) {
        let cell = if self.pressed == Some(id_) {
            1
        } else if self.hovered == Some(id_) {
            2
        } else {
            0
        };
        if let Some(img) = art.cell(dialog::BUTTON, 4, cell) {
            draw_scaled_or_direct(frame, &img, rect, true);
        } else {
            frame.fill(rect, FALLBACK_BTN_BG);
            draw_outline(frame, rect, WELL_BORDER);
        }
        if let Some(font) = art.font(12).or_else(|| art.font(11)) {
            let color = match cell {
                1 => BUTTON_PRESSED,
                2 => BUTTON_HOVER,
                _ => LABEL,
            };
            let tw = font.width(label);
            font.draw(
                frame,
                label,
                [
                    rect.x + (rect.width - tw) / 2,
                    rect.y + (rect.height - font.line_height) / 2,
                ],
                color,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_strip_button(
        &self,
        frame: &mut Frame,
        art: &Art,
        res_id: i32,
        cells: u32,
        rect: Rect,
        ctrl: ControlId,
        enabled: bool,
        selected: bool,
        compact: bool,
    ) {
        let cell = if !enabled {
            3.min(cells.saturating_sub(1))
        } else if selected || self.pressed == Some(ctrl) {
            1
        } else if self.hovered == Some(ctrl) {
            2
        } else {
            0
        };
        if let Some(img) = art.cell(res_id, cells, cell) {
            draw_scaled_or_direct(frame, &img, rect, compact);
        } else {
            frame.fill(
                rect,
                if enabled {
                    FALLBACK_BTN_BG
                } else {
                    FALLBACK_BTN_DISABLED
                },
            );
            draw_outline(
                frame,
                rect,
                if selected {
                    PORTRAIT_SELECTED
                } else {
                    WELL_BORDER
                },
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_labeled_pill_button(
        &self,
        frame: &mut Frame,
        art: &Art,
        res_id: i32,
        cells: u32,
        rect: Rect,
        ctrl: ControlId,
        enabled: bool,
        label: &str,
        compact: bool,
    ) {
        let cell = if !enabled {
            3.min(cells.saturating_sub(1))
        } else if self.pressed == Some(ctrl) {
            1
        } else if self.hovered == Some(ctrl) {
            2
        } else {
            0
        };
        if let Some(img) = art.cell(dialog::BUTTON, 4, cell.min(3)) {
            draw_scaled_or_direct(frame, &img, rect, true);
        } else if let Some(img) = art.cell(res_id, cells, cell) {
            draw_scaled_or_direct(frame, &img, rect, compact);
        } else {
            frame.fill(
                rect,
                if enabled {
                    FALLBACK_BTN_BG
                } else {
                    FALLBACK_BTN_DISABLED
                },
            );
            draw_outline(frame, rect, WELL_BORDER);
        }
        if let Some(font) = art.font(12).or_else(|| art.font(11)) {
            let color = if !enabled {
                BUTTON_DISABLED
            } else if self.pressed == Some(ctrl) {
                BUTTON_PRESSED
            } else if self.hovered == Some(ctrl) {
                BUTTON_HOVER
            } else {
                LABEL
            };
            let tw = font.width(label);
            font.draw(
                frame,
                label,
                [
                    rect.x + (rect.width - tw) / 2,
                    rect.y + (rect.height - font.line_height) / 2,
                ],
                color,
            );
        }
    }

    fn draw_tooltip(&self, frame: &mut Frame, art: &Art, layout: &CafLayout, tip: &str) {
        let Some(font) = art.font(10).or_else(|| art.font(11)) else {
            return;
        };
        let tw = font.width(tip);
        let box_w = tw + 12;
        let box_h = font.line_height + 8;
        let bx = (layout.canvas.x + (layout.canvas.width - box_w) / 2).max(layout.canvas.x + 4);
        let by = (layout.canvas.y + layout.canvas.height - box_h - 6).max(layout.canvas.y + 4);
        let rect = Rect::new(bx, by, box_w, box_h);
        frame.fill(rect, TOOLTIP_BG);
        draw_outline(frame, rect, TOOLTIP_BORDER);
        font.draw(frame, tip, [rect.x + 6, rect.y + 4], LABEL);
    }
}

fn draw_well(frame: &mut Frame, rect: Rect, focused: bool) {
    frame.fill(rect, ENTRY_FILL);
    draw_outline(
        frame,
        rect,
        if focused { FOCUS_BORDER } else { WELL_BORDER },
    );
}

fn draw_outline(frame: &mut Frame, rect: Rect, rgb: [u8; 3]) {
    if rect.width <= 0 || rect.height <= 0 {
        return;
    }
    frame.fill(Rect::new(rect.x, rect.y, rect.width, 1), rgb);
    frame.fill(
        Rect::new(rect.x, rect.y + rect.height - 1, rect.width, 1),
        rgb,
    );
    frame.fill(Rect::new(rect.x, rect.y, 1, rect.height), rgb);
    frame.fill(
        Rect::new(rect.x + rect.width - 1, rect.y, 1, rect.height),
        rgb,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_text_box(
    frame: &mut Frame,
    font: Option<&Font>,
    rect: Rect,
    text: &str,
    focused: bool,
    state: &TextFieldState,
    multiline: bool,
    paint_well: bool,
) {
    if paint_well {
        draw_well(frame, rect, focused);
    }
    let Some(font) = font else {
        return;
    };
    let inside = Rect::new(
        rect.x.saturating_add(1),
        rect.y.saturating_add(1),
        (rect.width - 2).max(0),
        (rect.height - 2).max(0),
    );
    let (Ok(w), Ok(h)) = (u32::try_from(inside.width), u32::try_from(inside.height)) else {
        return;
    };
    if w == 0 || h == 0 {
        return;
    }

    let mut scratch = Frame::new(w, h, ENTRY_FILL);
    if !paint_well {
        for dy in 0..h {
            let fy = inside.y.saturating_add(dy as i32);
            for dx in 0..w {
                if let Some(bg) = frame.pixel(inside.x.saturating_add(dx as i32), fy) {
                    scratch.blend(dx as i32, dy as i32, bg, 255);
                }
            }
        }
    }
    let pad = 5.min(inside.width / 4);
    let inner_w = (inside.width - pad * 2).max(1);

    if !multiline {
        let y = (inside.height - font.line_height) / 2;
        let caret_chars = state.caret.min(text.chars().count());
        let prefix_byte = char_byte_index(text, caret_chars);
        let caret_px = font.width(&text[..prefix_byte]);
        let scroll = if focused {
            (caret_px + 2 - inner_w).max(0)
        } else {
            0
        };
        let draw_x = pad - scroll;
        font.draw(&mut scratch, text, [draw_x, y], LABEL);
        if focused && state.caret_visible {
            let cx = (draw_x + caret_px).clamp(0, inside.width - 1);
            scratch.fill(
                Rect::new(cx, y.max(0), 1, font.line_height.clamp(1, inside.height)),
                LABEL,
            );
        }
    } else {
        let lines = font.break_lines(text, inner_w);
        let mut y = 3;
        let max_y = inside.height - font.line_height - 1;
        let mut last_line_w = 0;
        let mut last_y = y;
        for line in &lines {
            if y > max_y {
                break;
            }
            font.draw(&mut scratch, line, [pad, y], LABEL);
            last_line_w = font.width(line);
            last_y = y;
            y += font.line_height + 2;
        }
        if focused && state.caret_visible && inside.width > 0 {
            let cx = (pad + last_line_w).clamp(0, inside.width - 1);
            scratch.fill(
                Rect::new(
                    cx,
                    last_y.clamp(0, inside.height - 1),
                    1,
                    font.line_height.clamp(1, inside.height),
                ),
                LABEL,
            );
        }
    }

    for dy in 0..h {
        let fy = inside.y.saturating_add(dy as i32);
        for dx in 0..w {
            if let Some(px) = scratch.pixel(dx as i32, dy as i32) {
                frame.blend(inside.x.saturating_add(dx as i32), fy, px, 255);
            }
        }
    }
}

fn draw_personality_bar(frame: &mut Frame, art: Option<&Image>, rect: Rect, filled: u8, total: u8) {
    if total == 0 || rect.width <= 0 || rect.height <= 0 {
        return;
    }
    let filled = filled.min(total);
    let lit_w = (i64::from(rect.width) * i64::from(filled) / i64::from(total)) as i32;

    if let Some(img) = art {
        let img_lit_w = (i64::from(img.width) * i64::from(filled) / i64::from(total)) as i32;
        if img_lit_w > 0 && lit_w > 0 {
            draw_scaled_subimage(
                frame,
                img,
                Rect::new(0, 0, img_lit_w, i32::try_from(img.height).unwrap_or(0)),
                Rect::new(rect.x, rect.y, lit_w, rect.height),
            );
        }
    } else {
        draw_pip_fallback(frame, rect, filled, total);
    }
}

fn draw_pip_fallback(frame: &mut Frame, rect: Rect, filled: u8, total: u8) {
    if total == 0 || rect.width <= 0 || rect.height <= 0 {
        return;
    }
    for i in 0..total {
        let x0 = rect.x + (i64::from(rect.width) * i64::from(i) / i64::from(total)) as i32;
        let x1 = rect.x + (i64::from(rect.width) * i64::from(i + 1) / i64::from(total)) as i32;
        let w = (x1 - x0 - 1).max(1);
        let color = if i < filled { LED_LIT } else { LED_DIM };
        frame.fill(
            Rect::new(x0, rect.y + 1, w, (rect.height - 2).max(1)),
            color,
        );
    }
}

fn draw_scaled_or_direct(frame: &mut Frame, img: &Image, dst: Rect, allow_scale: bool) {
    if !allow_scale
        && i32::try_from(img.width).ok() == Some(dst.width)
        && i32::try_from(img.height).ok() == Some(dst.height)
    {
        frame.draw_all(img, [dst.x, dst.y]);
        return;
    }
    draw_scaled_subimage(
        frame,
        img,
        Rect::new(
            0,
            0,
            i32::try_from(img.width).unwrap_or(0),
            i32::try_from(img.height).unwrap_or(0),
        ),
        dst,
    );
}

fn draw_scaled_subimage(frame: &mut Frame, img: &Image, src: Rect, dst: Rect) {
    if src.width <= 0 || src.height <= 0 || dst.width <= 0 || dst.height <= 0 {
        return;
    }
    let fw = i32::try_from(frame.width()).unwrap_or(0);
    let fh = i32::try_from(frame.height()).unwrap_or(0);
    let y_start = 0.max(-dst.y);
    let y_end = dst.height.min(fh - dst.y);
    let x_start = 0.max(-dst.x);
    let x_end = dst.width.min(fw - dst.x);
    for dy in y_start..y_end {
        let sy = src.y + (i64::from(dy) * i64::from(src.height) / i64::from(dst.height)) as i32;
        let Ok(sy_u) = u32::try_from(sy) else {
            continue;
        };
        for dx in x_start..x_end {
            let sx = src.x + (i64::from(dx) * i64::from(src.width) / i64::from(dst.width)) as i32;
            let Ok(sx_u) = u32::try_from(sx) else {
                continue;
            };
            if let Some([r, g, b, a]) = img.pixel(sx_u, sy_u) {
                frame.blend(dst.x + dx, dst.y + dy, [r, g, b], a);
            }
        }
    }
}
