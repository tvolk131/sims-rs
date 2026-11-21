//! The help balloons the live-mode panels open over the lot: a title,
//! a body of text, and up to two picture slots on a bevelled tile
//! background (`0x50f6f2`, `0x50fc30`, `0x510547`).
//!
//! The original keeps one balloon for the control panel (made by
//! `0x50f6f2`, set up by `0x50f7cd`) and gives it its text and pictures
//! when a control asks for help.
//!
//! The texts are the game's own and are read from the install
//! ([`Texts::load`]); none is kept here.

use crate::art::Art;
use crate::frame::{Frame, Rect};
use crate::hud::{LABEL, id};
use crate::thumbnail::Thumbnails;
use crate::{Error, Result};
use sims_content::Install;
use sims_formats::iff::Iff;
use sims_formats::strings::StringTable;

/// The balloon's background (UIGraphics 3008, `kCatalogPopupBackTiles`):
/// 36 by 36, cut in three each way into [`TILE`]-sized corners, edges and
/// a middle (`0x50fe18`).
pub const BACKGROUND: i32 = 3008;
/// How wide the balloon is (`0x50f7cd`).
pub const WIDTH: i32 = 559;
/// A third of the background's side.
const TILE: i32 = 12;
/// The title's font (`0x6f2aec`) and the body's (`0x6f2ae8`).
const TITLE_FONT: u8 = 10;
const BODY_FONT: u8 = 9;
/// The text's left and right edges, inside the balloon, with no picture
/// (`0x50fc30`): the right one is 530 in language 0x13, which is not
/// ported.
const TEXT_LEFT: i32 = 5;
const TEXT_RIGHT: i32 = 535;
/// Picture slot geometry (`0x50fc5d..0x50fc7a`): left slot `(0, 0, 168, 127)`
/// pushes `text_left` to `168`; right slot `(372, 0, 168, 127)` pulls
/// `text_right` to `372`.
const SLOT_WIDTH: i32 = 168;
const SLOT_HEIGHT: i32 = 127;
const RIGHT_SLOT_X: i32 = 372;
/// How far the text is drawn right of its edges, and where the title and
/// the body start (`0x50fea6..`).
const TEXT_INDENT: i32 = 10;
const TITLE_TOP: i32 = 12;
/// What the balloon adds to its text's height, and its least height
/// (`0x50fd24`): text shorter than this less [`PADDING`] takes 103.
const PADDING: i32 = 24;
const MIN_HEIGHT: i32 = 127;

/// What a balloon picture slot draws (`0x510547`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Picture {
    /// An object definition's picture (`0x51035b`, `0x510547` type 1).
    /// The original renders the object's isometric sprite via `0x4e5306`;
    /// this port draws its catalog thumbnail icon via [`Thumbnails::icon`].
    Object(u32),
    /// A whole UIGraphics bitmap (`0x5103fe`, `0x510547` type 2 without subrect).
    Image(i32),
    /// One row of a vertically stacked UIGraphics strip (`0x530a7b` ->
    /// `0x5104a1`, `0x510547` type 2 with subrect): height divided evenly
    /// into `rows`, taking `row` (0-based).
    Row { id: i32, rows: u32, row: u32 },
}

/// What a balloon explains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Help {
    /// The sign of the zodiac, 1 (Aries) to 12, as the personality panel
    /// read it when it was pressed (`0x51bfb3`).
    Sign(u8),
    /// One of the 8 Needs (`Live.iff STR# 130`, `pet = false`) or Pet Needs
    /// (`Live.iff STR# 180`, `pet = true`) meters, index `0..8` in panel order
    /// (`0x519603`, `0x5197f3`, `0x51be80..0x51bf41`).
    Need { pet: bool, index: u8 },
    /// One of the 3 Dog (`cat = false`) or Cat (`cat = true`) Pet Skills
    /// (`Live.iff STR# 172`, `0x531895`, `0x532300`, `0x531bc6`, `0x5326e0`).
    PetSkill { cat: bool, index: u8 },
    /// The Child Report Card grade button (`Live.iff STR# 139`, `0x532c83`,
    /// `0x532e66`).
    ReportCard,
    /// One of the 6 Superstar skill labels (`Live.iff STR# 137`, `0x52c1de`,
    /// `0x52cdc3`), index `0..6` in `SKILLS` order.
    FameSkill(u8),
    /// The Superstar fame rank title button (`fame.iff STR# 1[13..=34]`,
    /// `0x52d446`, `0x52cdc3` index 6), level `0..=10` (0.0 to 5.0 stars).
    FameLevel(u8),
    /// The Superstar friends total starpower button (`fame.iff STR# 1[37..=38]`,
    /// `0x52d068`, `0x52d134`).
    FameFriends,
}

impl Help {
    /// The balloon / trigger identity the original compares to decide
    /// whether pressing a control closes the open balloon (`0x50fb7f`) or
    /// replaces its text in place (`0x50fbdf`):
    /// - Sign (`0x51c025`): single balloon `+0x1c8`, whatever the sign.
    /// - Needs / Pet Needs (`0x51bec2`, `0x51bf1c`): one balloon per meter.
    /// - Pet Skills (`0x531cbe`, `0x5327d8`): compares clicked button pointer.
    /// - Report Card (`0x532ec0`): single grade button `[esi + 0xf8]`.
    /// - Superstar (`0x52ce92`, `0x52d0c5`): compares clicked button pointer
    ///   (6 skill buttons, 1 fame-level button regardless of level, 1 friends
    ///   starpower button).
    pub fn balloon(self) -> u8 {
        match self {
            Self::Sign(_) => 0,
            Self::Need { pet: false, index } => 1 + index.min(7),
            Self::Need { pet: true, index } => 9 + index.min(7),
            Self::PetSkill { cat: false, index } => 17 + index.min(2),
            Self::PetSkill { cat: true, index } => 20 + index.min(2),
            Self::ReportCard => 23,
            Self::FameSkill(index) => 24 + index.min(5),
            Self::FameLevel(_) => 30,
            Self::FameFriends => 31,
        }
    }

    /// Whether `self` names a valid index for its variant.
    pub fn is_valid(self) -> bool {
        match self {
            Self::Sign(sign) => (1..=12).contains(&sign),
            Self::Need { index, .. } => index < 8,
            Self::PetSkill { index, .. } => index < 3,
            Self::ReportCard | Self::FameFriends => true,
            Self::FameSkill(index) => index < 6,
            Self::FameLevel(level) => level <= 10,
        }
    }
}

/// A balloon's text and optional left/right picture slots.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Text {
    pub title: String,
    pub body: String,
    pub pictures: [Option<Picture>; 2],
}

/// The balloons' texts, read from the install.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Texts {
    /// The signs', Aries first (`UIText.iff STR# 165/164/154`).
    signs: Vec<Text>,
    /// Human Needs (`Live.iff STR# 130`, 8 entries).
    needs: Vec<Text>,
    /// Pet Needs (`Live.iff STR# 180`, 8 entries).
    pet_needs: Vec<Text>,
    /// Dog Pet Skills (`Live.iff STR# 172` pairs 0..3).
    dog_skills: Vec<Text>,
    /// Cat Pet Skills (`Live.iff STR# 172` pair 3 then pairs 1..3).
    cat_skills: Vec<Text>,
    /// Child Report Card (`Live.iff STR# 139` strings 1 and 2).
    report_card: Option<Text>,
    /// Superstar skill popups (`Live.iff STR# 137` pairs 0..6).
    fame_skills: Vec<Text>,
    /// Superstar fame level popups (`fame.iff STR# 1` strings 13..35, 11 levels).
    fame_levels: Vec<Text>,
    /// Superstar friends starpower popup (`fame.iff STR# 1` strings 37 and 38).
    fame_friends: Option<Text>,
}

/// UIText.iff's tables the sign balloons read (`0x519e68..0x51a00f`): the
/// signs' titles and bodies, two entries a sign; the signs' names; and
/// the panel's labels, of which the compatibility lines' are entries 6
/// and 7.
const SIGN_TEXTS: u16 = 165;
const SIGN_NAMES: u16 = 164;
const PANEL_LABELS: u16 = 154;
const MOST_LABEL: usize = 6;
const LEAST_LABEL: usize = 7;

/// `Live.iff` string tables for Needs, Pet Needs, Pet Skills, Report Card,
/// and Skill popups (`0x519621`, `0x51980d`, `0x5318a5`, `0x532310`,
/// `0x532aae`, `0x52c1de`).
const LIVE_NEEDS: u16 = 130;
const LIVE_SKILL_POPUPS: u16 = 137;
const LIVE_REPORT_CARD: u16 = 139;
const LIVE_PET_SKILLS: u16 = 172;
const LIVE_PET_NEEDS: u16 = 180;

/// `fame.iff` panel & popup text table (`0x52bd10`).
const FAME_PANEL_TEXT: u16 = 1;

/// Human Needs picture slots (`0x6f39d0`/`0x6f39d4`, plus `0xea6` Social
/// bitmap attached at `0x51974c`).
const NEED_PICTURES: [[Option<Picture>; 2]; 8] = [
    [
        Some(Picture::Object(0xb947_55df)),
        Some(Picture::Object(0xff9f_18e0)),
    ],
    [
        Some(Picture::Object(0x82e0_4c5b)),
        Some(Picture::Object(0xbf13_7195)),
    ],
    [
        Some(Picture::Object(0xd976_81db)),
        Some(Picture::Object(0x7cb1_1019)),
    ],
    [
        Some(Picture::Object(0x5cdc_712f)),
        Some(Picture::Object(0x481a_74ec)),
    ],
    [
        Some(Picture::Object(0x85e0_0942)),
        Some(Picture::Object(0x85e0_2a40)),
    ],
    [Some(Picture::Image(id::SOCIAL_POPUP)), None],
    [Some(Picture::Object(0x84e0_774c)), None],
    [
        Some(Picture::Object(0x7f90_7075)),
        Some(Picture::Object(0xbc3b_a088)),
    ],
];

/// Pet Needs picture slots (`0x6f3950`/`0x6f3954`, plus `0xea6` Social
/// bitmap attached at `0x51995d`).
const PET_NEED_PICTURES: [[Option<Picture>; 2]; 8] = [
    [
        Some(Picture::Object(0x8eec_19f7)),
        Some(Picture::Object(0xffc0_ea58)),
    ],
    [
        Some(Picture::Object(0xbc53_7c52)),
        Some(Picture::Object(0x96cb_67ae)),
    ],
    [
        Some(Picture::Object(0xe23f_9020)),
        Some(Picture::Object(0x2b45_02a2)),
    ],
    [
        Some(Picture::Object(0x914e_5a74)),
        Some(Picture::Object(0xb26a_b256)),
    ],
    [Some(Picture::Object(0x2ecd_b068)), None],
    [Some(Picture::Image(id::SOCIAL_POPUP)), None],
    [
        Some(Picture::Object(0x39cc_f441)),
        Some(Picture::Object(0xbedd_7b26)),
    ],
    [
        Some(Picture::Object(0x7f90_7075)),
        Some(Picture::Object(0xbc3b_a088)),
    ],
];

/// Child Report Card picture slot (`0x532caf -> 0x530a7b(0xb)`): row 11 of
/// `kJobPopupComposite` (4905, 22 rows of 84x63).
const REPORT_CARD_PICTURES: [Option<Picture>; 2] = [
    Some(Picture::Row {
        id: id::JOB_POPUP,
        rows: 22,
        row: 11,
    }),
    None,
];

/// Superstar skill popup picture slots (`0x6f3cb8[0..6]`).
const FAME_SKILL_PICTURES: [[Option<Picture>; 2]; 6] = [
    [Some(Picture::Object(0x5cdc_712f)), None],
    [Some(Picture::Object(0x5cdc_712f)), None],
    [Some(Picture::Object(0xc041_ed5b)), None],
    [Some(Picture::Object(0xddec_d497)), None],
    [Some(Picture::Object(0x94aa_32f4)), None],
    [
        Some(Picture::Object(0x9fb2_23ce)),
        Some(Picture::Object(0x1120_39a0)),
    ],
];

/// Superstar fame-level popup picture slots (`0x6f3c60[0..11]`, with sentinels
/// `0x11 -> kFameRunway (278)`, `0x12 -> kFameMusicVideo (280)`,
/// `0x13 -> kFameMovieSet (279)` dispatched at `0x52cf01..0x52cf40`).
const FAME_LEVEL_PICTURES: [[Option<Picture>; 2]; 11] = [
    [
        Some(Picture::Object(0x6a85_94ad)),
        Some(Picture::Object(0x5a91_cee0)),
    ],
    [Some(Picture::Object(0x4450_e4e0)), None],
    [Some(Picture::Object(0x1d77_2052)), None],
    [Some(Picture::Object(0xf412_6cc4)), None],
    [Some(Picture::Object(0x4450_e4e0)), None],
    [Some(Picture::Object(0xf412_6cc4)), None],
    [Some(Picture::Object(0x1d77_2052)), None],
    [Some(Picture::Image(id::FAME_RUNWAY)), None],
    [Some(Picture::Image(id::FAME_MUSIC_VIDEO)), None],
    [Some(Picture::Image(id::FAME_MOVIE_SET)), None],
    [Some(Picture::Object(0x4b61_d53e)), None],
];

/// The signs each sign gets on best and worst with, 1 to 12 (the tables
/// at `0x6c5ce0` and `0x6c5d40`, two a sign).
const MOST: [[u8; 2]; 12] = [
    [3, 2],
    [1, 7],
    [12, 6],
    [2, 8],
    [9, 4],
    [11, 9],
    [6, 4],
    [12, 5],
    [12, 10],
    [11, 2],
    [10, 9],
    [8, 3],
];
const LEAST: [[u8; 2]; 12] = [
    [4, 7],
    [6, 4],
    [10, 1],
    [3, 1],
    [10, 3],
    [5, 2],
    [12, 8],
    [7, 11],
    [7, 8],
    [5, 3],
    [8, 6],
    [5, 1],
];

fn read_table(iff: &Iff, id: u16) -> Vec<String> {
    let Some(chunk) = iff.get(b"STR#", id) else {
        return Vec::new();
    };
    let Ok(table) = StringTable::parse(iff.data(chunk)) else {
        return Vec::new();
    };
    (0..table.entries.len())
        .map_while(|i| table.get(i, 1).map(str::to_owned))
        .collect()
}

fn read_optional_iff(install: &Install, relative: &str) -> Option<Iff> {
    let path = install.file(relative).ok()?;
    let bytes = std::fs::read(&path).ok()?;
    Iff::parse(bytes).ok()
}

impl Texts {
    /// Reads balloon texts from `GameData/UIText.iff`, `GameData/Live.iff`,
    /// and (when present) `GameData/fame.iff`, in English (language 1).
    pub fn load(install: &Install) -> Result<Self> {
        let path = install
            .file("GameData/UIText.iff")
            .map_err(Error::Content)?;
        let bytes = std::fs::read(&path).map_err(Error::Io)?;
        let ui_iff = Iff::parse(bytes).map_err(Error::Format)?;
        // A table missing, or one that does not parse, leaves its strings
        // empty: the balloons lose that text rather than the HUD failing
        // to open (UIText.iff itself the dialogs' labels need anyway).
        let mut out = Self::signs(
            &read_table(&ui_iff, SIGN_TEXTS),
            &read_table(&ui_iff, SIGN_NAMES),
            &read_table(&ui_iff, PANEL_LABELS),
        );
        if let Some(live) = read_optional_iff(install, "GameData/Live.iff") {
            out.set_live(
                &read_table(&live, LIVE_NEEDS),
                &read_table(&live, LIVE_PET_NEEDS),
                &read_table(&live, LIVE_PET_SKILLS),
                &read_table(&live, LIVE_REPORT_CARD),
                &read_table(&live, LIVE_SKILL_POPUPS),
            );
        }
        if let Some(fame) = read_optional_iff(install, "GameData/fame.iff") {
            out.set_fame(&read_table(&fame, FAME_PANEL_TEXT));
        }
        Ok(out)
    }

    /// The signs' balloons from the three tables' strings, as the
    /// personality panel puts them together (`0x519e68..0x51a00f`): each
    /// sign's title, and its body followed by a line of the signs it gets
    /// on best with and one of those it gets on worst with. A missing
    /// string is left empty.
    pub fn signs(texts: &[String], names: &[String], labels: &[String]) -> Self {
        let get = |list: &[String], n: usize| list.get(n).cloned().unwrap_or_default();
        let name = |sign: u8| get(names, usize::from(sign) - 1);
        let signs = (0..12)
            .map(|n| {
                let [a, b] = MOST[n];
                let [c, d] = LEAST[n];
                Text {
                    title: get(texts, 2 * n),
                    body: format!(
                        "{}\n{}{}, {}\n{}{}, {}",
                        get(texts, 2 * n + 1),
                        get(labels, MOST_LABEL),
                        name(a),
                        name(b),
                        get(labels, LEAST_LABEL),
                        name(c),
                        name(d),
                    ),
                    pictures: [None, None],
                }
            })
            .collect();
        Self {
            signs,
            ..Self::default()
        }
    }

    /// Populates the `Live.iff` balloon tables (Needs `STR# 130`, Pet Needs
    /// `STR# 180`, Pet Skills `STR# 172`, Report Card `STR# 139`, and
    /// Skill popups `STR# 137`).
    pub fn set_live(
        &mut self,
        needs: &[String],
        pet_needs: &[String],
        pet_skills: &[String],
        report_card: &[String],
        skill_popups: &[String],
    ) {
        let get = |list: &[String], n: usize| list.get(n).cloned().unwrap_or_default();
        if !needs.is_empty() {
            // STR# 130[0] is the panel header "Needs"; meters read pairs
            // starting at index 1 (0x519654..0x519723, 0x5130a7..0x51311e).
            self.needs = (0..8)
                .map(|i| Text {
                    title: get(needs, 1 + 2 * i),
                    body: get(needs, 2 + 2 * i),
                    pictures: NEED_PICTURES[i],
                })
                .collect();
        }
        if !pet_needs.is_empty() {
            // STR# 180[0] is "Needs"; pet meters read pairs from index 1
            // (0x519823..0x51995d).
            self.pet_needs = (0..8)
                .map(|i| Text {
                    title: get(pet_needs, 1 + 2 * i),
                    body: get(pet_needs, 2 + 2 * i),
                    pictures: PET_NEED_PICTURES[i],
                })
                .collect();
        }
        if !pet_skills.is_empty() {
            // Dog reads pairs 0, 1, 2 (0x531895..0x5319a0). Cat reads 0, 1, 2
            // then overwrites slot 0 with pair 3 (Hunting, 0x532300..0x532435).
            // Both panels' GUID table at 0x6f4458..0x6f446c is all zero.
            self.dog_skills = (0..3)
                .map(|i| Text {
                    title: get(pet_skills, 2 * i),
                    body: get(pet_skills, 2 * i + 1),
                    pictures: [None, None],
                })
                .collect();
            self.cat_skills = [3, 1, 2]
                .into_iter()
                .map(|pair| Text {
                    title: get(pet_skills, 2 * pair),
                    body: get(pet_skills, 2 * pair + 1),
                    pictures: [None, None],
                })
                .collect();
        }
        if !report_card.is_empty() {
            // STR# 139[0] is "Report Card"; [1] title, [2] body (0x532aae..0x532c83).
            self.report_card = Some(Text {
                title: get(report_card, 1),
                body: get(report_card, 2),
                pictures: REPORT_CARD_PICTURES,
            });
        }
        if !skill_popups.is_empty() {
            // STR# 137 first 6 pairs for Superstar skill buttons (0x52c1de..0x52c2b5).
            self.fame_skills = (0..6)
                .map(|i| Text {
                    title: get(skill_popups, 2 * i),
                    body: get(skill_popups, 2 * i + 1),
                    pictures: FAME_SKILL_PICTURES[i],
                })
                .collect();
        }
    }

    /// Populates the Superstar `fame.iff STR# 1` balloon texts
    /// (`0x52bd10..0x52bf72`): 11 fame-level pairs at indices `13..35` and
    /// the friends total starpower pair at indices `37..=38`.
    pub fn set_fame(&mut self, fame_panel: &[String]) {
        if fame_panel.is_empty() {
            return;
        }
        let get = |n: usize| fame_panel.get(n).cloned().unwrap_or_default();
        self.fame_levels = (0..11)
            .map(|level| Text {
                title: get(13 + 2 * level),
                body: get(14 + 2 * level),
                pictures: FAME_LEVEL_PICTURES[level],
            })
            .collect();
        self.fame_friends = Some(Text {
            title: get(37),
            body: get(38),
            pictures: [None, None],
        });
    }

    /// What `help` shows; `None` if out of range or its table was not loaded.
    pub fn get(&self, help: Help) -> Option<&Text> {
        match help {
            Help::Sign(sign) => self.signs.get(usize::from(sign).checked_sub(1)?),
            Help::Need { pet: false, index } => self.needs.get(usize::from(index)),
            Help::Need { pet: true, index } => self.pet_needs.get(usize::from(index)),
            Help::PetSkill { cat: false, index } => self.dog_skills.get(usize::from(index)),
            Help::PetSkill { cat: true, index } => self.cat_skills.get(usize::from(index)),
            Help::ReportCard => self.report_card.as_ref(),
            Help::FameSkill(index) => self.fame_skills.get(usize::from(index)),
            Help::FameLevel(level) => self.fame_levels.get(usize::from(level)),
            Help::FameFriends => self.fame_friends.as_ref(),
        }
    }
}

/// A balloon laid out: where it goes, its text broken into lines, and its
/// left/right picture slots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Balloon {
    pub rect: Rect,
    pub title: String,
    pub lines: Vec<String>,
    /// Left text edge inside `rect` before [`TEXT_INDENT`] (`5` without a left
    /// picture, `168` with one, `0x50fc83..0x50fc9c`).
    pub text_left: i32,
    pub pictures: [Option<Picture>; 2],
}

impl Balloon {
    /// Object GUIDs referenced by this balloon's picture slots, for
    /// pre-loading via [`Thumbnails::read`].
    pub fn objects(&self) -> impl Iterator<Item = u32> + '_ {
        self.pictures.iter().filter_map(|slot| match slot {
            Some(Picture::Object(guid)) => Some(*guid),
            _ => None,
        })
    }
}

/// Lays `text` out in a balloon over `owner`, the subpanel that asked
/// for it (`0x50fc30`). The body is broken to the text's width (narrowed
/// by any left or right picture slot); the balloon is as tall as the
/// title's line and the body's lines, plus [`PADDING`], and at least
/// [`MIN_HEIGHT`]. Its right edge is the owner's and its bottom the
/// owner's top.
///
/// **Not ported:** the 50 pixels more an East Asian language takes, and
/// the owner 50 lower when it is 150 tall (no live-mode panel is).
pub fn lay_out(art: &Art, text: &Text, owner: Rect) -> Balloon {
    let text_left = if text.pictures[0].is_some() {
        SLOT_WIDTH
    } else {
        TEXT_LEFT
    };
    let text_right = if text.pictures[1].is_some() {
        RIGHT_SLOT_X
    } else {
        TEXT_RIGHT
    };
    let title_line = art.font(TITLE_FONT).map_or(0, |f| f.line_height);
    let (body_line, lines) = art.font(BODY_FONT).map_or((0, Vec::new()), |f| {
        (
            f.line_height,
            f.break_lines(&text.body, text_right - text_left),
        )
    });
    let count = i32::try_from(lines.len()).unwrap_or(i32::MAX);
    let mut inner = title_line.saturating_add(body_line.saturating_mul(count));
    if inner.saturating_add(PADDING) < MIN_HEIGHT {
        inner = MIN_HEIGHT - PADDING;
    }
    let height = inner.saturating_add(PADDING);
    let right = owner.x.saturating_add(owner.width);
    // The original runs at 800 by 600 or more, where the balloon always
    // fits. In a smaller window it goes where the same sum puts it, partly
    // off the screen, and never over the panel whose control closes it.
    Balloon {
        rect: Rect::new(
            right.saturating_sub(WIDTH),
            owner.y.saturating_sub(height),
            WIDTH,
            height,
        ),
        title: text.title.clone(),
        lines,
        text_left,
        pictures: text.pictures,
    }
}

/// Computes the vertical span `(top, height)` of a picture slot inside a
/// balloon of `height` (`0x50ff36..0x50ff91`).
fn slot_vertical(balloon_height: i32) -> (i32, i32) {
    let mut top = 0;
    let mut bottom = SLOT_HEIGHT;
    if balloon_height > bottom - top {
        let dy = (balloon_height - (bottom - top)) / 2 - top;
        top += dy;
        bottom += dy;
        if bottom > balloon_height {
            let shift = balloon_height - bottom;
            top += shift;
            bottom += shift;
        }
        let expand = (balloon_height - (bottom - top)) / 2 - 5;
        top -= expand;
        bottom += expand;
    }
    (top, (bottom - top).max(0))
}

/// Draws `balloon` without object catalog thumbnails.
pub fn draw(frame: &mut Frame, art: &Art, balloon: &Balloon) {
    draw_with_thumbnails(frame, art, None, balloon);
}

/// Draws `balloon`: the background's corners, its edges and middle
/// repeated from their top left to fill the rest (`0x5df45d`), the left
/// and right picture slots (`0x50fef5..0x50fff9`, `0x510547`), and then
/// the title and as many of the body's lines as start above the body's
/// bottom, in the labels' colour, left aligned.
pub fn draw_with_thumbnails(
    frame: &mut Frame,
    art: &Art,
    thumbnails: Option<&Thumbnails>,
    balloon: &Balloon,
) {
    let r = balloon.rect;
    if let Some(back) = art.image(BACKGROUND) {
        let (right, bottom) = (r.x.saturating_add(r.width), r.y.saturating_add(r.height));
        let columns = [
            (r.x, TILE, 0),
            (r.x + TILE, r.width - 2 * TILE, TILE),
            (right - TILE, TILE, 2 * TILE),
        ];
        let rows = [
            (r.y, TILE, 0),
            (r.y + TILE, r.height - 2 * TILE, TILE),
            (bottom - TILE, TILE, 2 * TILE),
        ];
        for &(y, height, sy) in &rows {
            for &(x, width, sx) in &columns {
                tile(
                    frame,
                    back,
                    Rect::new(sx, sy, TILE, TILE),
                    Rect::new(x, y, width, height),
                );
            }
        }
    }
    let (slot_y, slot_h) = slot_vertical(r.height);
    for (slot_index, picture) in balloon.pictures.iter().enumerate() {
        let Some(picture) = picture else { continue };
        let slot_x = if slot_index == 0 { 0 } else { RIGHT_SLOT_X };
        let slot = Rect::new(
            r.x.saturating_add(slot_x),
            r.y.saturating_add(slot_y),
            SLOT_WIDTH,
            slot_h,
        );
        draw_picture(frame, art, thumbnails, *picture, slot);
    }
    let left = r.x.saturating_add(balloon.text_left + TEXT_INDENT);
    if let Some(font) = art.font(TITLE_FONT) {
        font.draw(
            frame,
            &balloon.title,
            [left, r.y.saturating_add(TITLE_TOP)],
            LABEL,
        );
    }
    if let (Some(title), Some(font)) = (art.font(TITLE_FONT), art.font(BODY_FONT)) {
        let top = r.y.saturating_add(TITLE_TOP + title.line_height);
        // The body's box ends 12 above the balloon's bottom.
        let bottom = r.y.saturating_add(r.height - PADDING + TITLE_TOP);
        let mut y = top;
        for line in &balloon.lines {
            if y >= bottom {
                break;
            }
            font.draw(frame, line, [left, y], LABEL);
            y = y.saturating_add(font.line_height.max(1));
        }
    }
}

/// Draws one picture slot centered inside `slot` (`0x510547`):
/// - `Picture::Image` and `Picture::Row` follow `0x5105cb..0x51065d` (center
///   using truncation toward zero whenever the slot is larger than the image).
/// - `Picture::Object` draws the object's catalog icon via [`Thumbnails::icon`]
///   centered in `slot` (the original renders the isometric sprite via
///   `0x4e5306`, which is not ported).
fn draw_picture(
    frame: &mut Frame,
    art: &Art,
    thumbnails: Option<&Thumbnails>,
    picture: Picture,
    slot: Rect,
) {
    let center = |w: i32, h: i32| {
        let x = if slot.width > w {
            slot.x.saturating_add((slot.width - w) / 2)
        } else {
            slot.x
        };
        let y = if slot.height > h {
            slot.y.saturating_add((slot.height - h) / 2)
        } else {
            slot.y
        };
        [x, y]
    };
    match picture {
        Picture::Image(id) => {
            let Some(image) = art.image(id) else { return };
            let w = i32::try_from(image.width).unwrap_or(0);
            let h = i32::try_from(image.height).unwrap_or(0);
            frame.draw(image, Rect::new(0, 0, w, h), center(w, h));
        }
        Picture::Row { id, rows, row } => {
            let Some(image) = art.image(id) else { return };
            let rows = rows.max(1);
            if row >= rows {
                return;
            }
            let w = i32::try_from(image.width).unwrap_or(0);
            let h = i32::try_from(image.height / rows).unwrap_or(0);
            let Some(sy) = h.checked_mul(i32::try_from(row).unwrap_or(i32::MAX)) else {
                return;
            };
            frame.draw(image, Rect::new(0, sy, w, h), center(w, h));
        }
        Picture::Object(guid) => {
            let Some(icon) = thumbnails.and_then(|t| t.icon(art, guid)) else {
                return;
            };
            let w = i32::try_from(icon.width).unwrap_or(0);
            let h = i32::try_from(icon.height).unwrap_or(0);
            frame.draw_all(&icon, center(w, h));
        }
    }
}

/// Fills `target` with copies of `source`, the first at its top left.
fn tile(frame: &mut Frame, image: &sims_formats::bitmap::Image, source: Rect, target: Rect) {
    if source.width <= 0 || source.height <= 0 {
        return;
    }
    let mut y = 0;
    while y < target.height {
        let mut x = 0;
        while x < target.width {
            let part = Rect::new(
                source.x,
                source.y,
                source.width.min(target.width - x),
                source.height.min(target.height - y),
            );
            frame.draw(image, part, [target.x + x, target.y + y]);
            x += source.width;
        }
        y += source.height;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sign_balloon_names_the_signs_it_gets_on_with() {
        let texts: Vec<String> = (0..24).map(|n| format!("t{n}")).collect();
        let names: Vec<String> = (1..=12).map(|n| format!("s{n}")).collect();
        let labels: Vec<String> = (0..8).map(|n| format!("l{n}: ")).collect();
        let t = Texts::signs(&texts, &names, &labels);
        // Taurus: Aries and Libra, then Virgo and Cancer.
        let taurus = t.get(Help::Sign(2)).unwrap();
        assert_eq!(taurus.title, "t2");
        assert_eq!(taurus.body, "t3\nl6: s1, s7\nl7: s6, s4");
        assert!(t.get(Help::Sign(0)).is_none());
        assert!(t.get(Help::Sign(13)).is_none());
        assert_eq!(t.get(Help::Sign(12)).unwrap().title, "t22");
    }

    #[test]
    fn missing_strings_leave_the_balloon_empty_where_they_go() {
        let t = Texts::signs(&[], &[], &[]);
        assert_eq!(t.get(Help::Sign(1)).unwrap().body, "\n, \n, ");
    }

    #[test]
    fn live_and_fame_texts_map_all_variants_and_pictures() {
        let mut t = Texts::default();
        let needs: Vec<String> = (0..17).map(|n| format!("n{n}")).collect();
        let pet_needs: Vec<String> = (0..17).map(|n| format!("pn{n}")).collect();
        let pet_skills: Vec<String> = (0..8).map(|n| format!("ps{n}")).collect();
        let report: Vec<String> = vec!["Report Card".into(), "Grades".into(), "Study hard".into()];
        let skill_popups: Vec<String> = (0..12).map(|n| format!("sk{n}")).collect();
        let fame: Vec<String> = (0..39).map(|n| format!("f{n}")).collect();
        t.set_live(&needs, &pet_needs, &pet_skills, &report, &skill_popups);
        t.set_fame(&fame);

        // Hunger (index 0) skips header n0 -> n1 / n2 with two object GUIDs.
        let hunger = t
            .get(Help::Need {
                pet: false,
                index: 0,
            })
            .unwrap();
        assert_eq!((hunger.title.as_str(), hunger.body.as_str()), ("n1", "n2"));
        assert_eq!(
            hunger.pictures,
            [
                Some(Picture::Object(0xb947_55df)),
                Some(Picture::Object(0xff9f_18e0))
            ]
        );
        // Social (index 5) uses UIGraphics bitmap 3750 in slot 0 and none in slot 1.
        let social = t
            .get(Help::Need {
                pet: true,
                index: 5,
            })
            .unwrap();
        assert_eq!(
            (social.title.as_str(), social.body.as_str()),
            ("pn11", "pn12")
        );
        assert_eq!(social.pictures, [Some(Picture::Image(3750)), None]);
        // Dog skill 0 is pair 0 (ps0/ps1); Cat skill 0 is pair 3 (ps6/ps7, Hunting).
        let dog0 = t
            .get(Help::PetSkill {
                cat: false,
                index: 0,
            })
            .unwrap();
        let cat0 = t
            .get(Help::PetSkill {
                cat: true,
                index: 0,
            })
            .unwrap();
        let cat1 = t
            .get(Help::PetSkill {
                cat: true,
                index: 1,
            })
            .unwrap();
        assert_eq!((dog0.title.as_str(), dog0.body.as_str()), ("ps0", "ps1"));
        assert_eq!((cat0.title.as_str(), cat0.body.as_str()), ("ps6", "ps7"));
        assert_eq!((cat1.title.as_str(), cat1.body.as_str()), ("ps2", "ps3"));
        assert_eq!(cat0.pictures, [None, None]);
        // Report Card uses row 11 of JOB_POPUP (4905).
        let rc = t.get(Help::ReportCard).unwrap();
        assert_eq!(
            (rc.title.as_str(), rc.body.as_str()),
            ("Grades", "Study hard")
        );
        assert_eq!(rc.pictures, REPORT_CARD_PICTURES);
        // Superstar fame levels 7/8/9 map to bitmaps 278/280/279.
        assert_eq!(
            t.get(Help::FameLevel(7)).unwrap().pictures,
            [Some(Picture::Image(278)), None]
        );
        assert_eq!(
            t.get(Help::FameLevel(8)).unwrap().pictures,
            [Some(Picture::Image(280)), None]
        );
        assert_eq!(
            t.get(Help::FameLevel(9)).unwrap().pictures,
            [Some(Picture::Image(279)), None]
        );
        assert_eq!(t.get(Help::FameFriends).unwrap().title, "f37");
        assert_eq!(t.get(Help::FameFriends).unwrap().body, "f38");
        assert_eq!(t.get(Help::FameSkill(5)).unwrap().title, "sk10");
        assert_eq!(
            t.get(Help::FameSkill(5)).unwrap().pictures,
            [
                Some(Picture::Object(0x9fb2_23ce)),
                Some(Picture::Object(0x1120_39a0))
            ]
        );
    }

    #[test]
    fn a_balloon_sits_on_its_owner_flush_right() {
        let art = Art::default();
        let b = lay_out(&art, &Text::default(), Rect::new(413, 500, 387, 100));
        assert_eq!(b.rect, Rect::new(241, 373, WIDTH, MIN_HEIGHT));
        assert_eq!(b.text_left, TEXT_LEFT);
        // In a window smaller than the original's least, it goes where
        // the same sum puts it, off the top left, clear of the panel.
        let b = lay_out(&art, &Text::default(), Rect::new(113, 100, 387, 100));
        assert_eq!(b.rect, Rect::new(-59, -27, WIDTH, MIN_HEIGHT));
    }

    #[test]
    fn picture_slots_narrow_body_wrapping_as_the_original_does() {
        let mut art = Art::default();
        let font = crate::font::tests::font();
        art.insert_font(TITLE_FONT, font.clone());
        art.insert_font(BODY_FONT, font);
        let body = "AAAA ".repeat(40);
        let owner = Rect::new(413, 500, 387, 100);
        let plain = lay_out(
            &art,
            &Text {
                title: "T".into(),
                body: body.clone(),
                pictures: [None, None],
            },
            owner,
        );
        let one_pic = lay_out(
            &art,
            &Text {
                title: "T".into(),
                body: body.clone(),
                pictures: [Some(Picture::Image(id::SOCIAL_POPUP)), None],
            },
            owner,
        );
        let two_pics = lay_out(
            &art,
            &Text {
                title: "T".into(),
                body,
                pictures: [
                    Some(Picture::Object(0xb947_55df)),
                    Some(Picture::Object(0xff9f_18e0)),
                ],
            },
            owner,
        );
        assert_eq!(plain.text_left, 5);
        assert_eq!(one_pic.text_left, 168);
        assert_eq!(two_pics.text_left, 168);
        // Narrower text column wraps into more lines and grows taller.
        assert!(plain.lines.len() < one_pic.lines.len());
        assert!(one_pic.lines.len() < two_pics.lines.len());
        assert!(two_pics.rect.height >= one_pic.rect.height);
    }

    #[test]
    fn a_balloon_grows_with_its_body() {
        let mut art = Art::default();
        let font = crate::font::tests::font();
        let line = font.line_height;
        art.insert_font(TITLE_FONT, font.clone());
        art.insert_font(BODY_FONT, font);
        let text = |lines: usize| Text {
            title: "A".into(),
            body: vec!["A"; lines].join("\n"),
            pictures: [None, None],
        };
        let owner = Rect::new(413, 500, 387, 100);
        // Short text takes the least height.
        assert_eq!(lay_out(&art, &text(1), owner).rect.height, MIN_HEIGHT);
        // Enough lines to pass it: the title's, the body's and 24.
        let n = usize::try_from((MIN_HEIGHT / line) + 1).unwrap();
        let b = lay_out(&art, &text(n), owner);
        assert_eq!(b.lines.len(), n);
        let height = line * (1 + i32::try_from(n).unwrap()) + PADDING;
        assert_eq!(b.rect, Rect::new(241, 500 - height, WIDTH, height));
    }
}
