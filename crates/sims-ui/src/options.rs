//! The control panel's Options mode (`cWinOptions`, vtable `0x66c428`,
//! `0x513dc4..0x5179e5`): the action buttons (Save, Neighborhood, Quit),
//! the three subpanel tabs (Graphics, Sound, Play), and their checkboxes,
//! 3-way detail radio groups, and 11-step volume sliders.
//!
//! Coordinates and labels come from `GameData/UIText.iff STR# 145`
//! (`0x5140d4..0x5140e0`, parsed by `0x5001cc` as `"(x;y)"`). Confirmation
//! dialog strings for Save, Quit, and Reset Tutorial come from `STR# 153`.

use crate::art::Art;
use crate::command::{
    Command, DetailLevel, OptionsAction, OptionsHelp, OptionsRadio, OptionsTab, OptionsToggle,
    VolumeChannel,
};
use crate::dialog;
use crate::frame::{Frame, Rect};
use crate::hud::{Align, Item, Kind, LABEL, rgb565};
use crate::{Error, Result};
use sims_content::Install;
use sims_formats::iff::Iff;
use sims_formats::strings::StringTable;

/// Resources the options panel draws (`UIGraphics/Res_CPanel.h`, loaded in
/// `cWinOptions::Init` at `0x513ef0..0x513f5a`, `0x514145`, `0x514280`, and
/// `0x5159a2`).
pub mod id {
    /// Horizontal divider (`OptionsToothpkHoriz.bmp`, 201×2, `0x513f43`).
    pub const TOOTHPICK_HORIZ: i32 = 4916;
    /// Vertical divider (`OptionsToothpkVert.bmp`, 2×89, `0x513f55`).
    pub const TOOTHPICK_VERT: i32 = 4917;
    /// Save action button (`OptSave.BMP`, 156×39, 4 cells of 39×39, `0x66c368`).
    pub const SAVE: i32 = 4950;
    /// Neighborhood action button (`OptNbhd.bmp`, 196×37, 4 cells of 49×37, `0x66c364`).
    pub const NEIGHBORHOOD: i32 = 4951;
    /// Quit action button (`OptExit.bmp`, 200×40, 4 cells of 50×40, `0x66c36c`).
    pub const QUIT: i32 = 4952;
    /// 6-cell checkbox strip (`optcheckbox.bmp`, 120×21, 6 cells of 20×21, `0x513f0d`, `0x5e4dbb`).
    pub const CHECKBOX: i32 = 4953;
    /// 6-cell radio strip (`optradio.bmp`, 120×20, 6 cells of 20×20, `0x513efb`, `0x5376cd`).
    pub const RADIO: i32 = 4954;
    /// Active volume slider strip (`optslideron.bmp`, 44×14, 4 cells of 11×14, `0x513f1f`).
    pub const SLIDER_ON: i32 = 4955;
    /// Muted (step 0) volume slider strip (`optslideroff.bmp`, 44×14, 4 cells of 11×14, `0x513f31`).
    pub const SLIDER_OFF: i32 = 4956;
    /// Graphics Options tab (`optgraphics.bmp`, 152×40, 4 cells of 38×40, `0x66c370`).
    pub const GRAPHICS: i32 = 4957;
    /// Sound Options tab (`optsound.bmp`, 188×40, 4 cells of 47×40, `0x66c374`).
    pub const SOUND: i32 = 4958;
    /// Play Options tab (`optplay.bmp`, 220×40, 4 cells of 55×40, `0x66c378`).
    pub const PLAY: i32 = 4959;
    /// Reset Tutorial button (`opttutreset.BMP`, 88×30, 4 cells of 22×30, `0x51599d`).
    pub const TUTORIAL_RESET: i32 = 4960;
    /// Save-house confirmation card bitmap (`0x1382`, `0x50d8b4`).
    pub const SAVE_CARD: i32 = 4994;
}

/// Every resource the options panel needs, for [`Art::load`].
pub const RESOURCES: [i32; 13] = [
    id::TOOTHPICK_HORIZ,
    id::TOOTHPICK_VERT,
    id::SAVE,
    id::NEIGHBORHOOD,
    id::QUIT,
    id::CHECKBOX,
    id::RADIO,
    id::SLIDER_ON,
    id::SLIDER_OFF,
    id::GRAPHICS,
    id::SOUND,
    id::PLAY,
    id::TUTORIAL_RESET,
];

/// Optional `kPopupOpt*` help-balloon illustrations (`0x66c394..0x66c410`,
/// UIGraphics IDs `3700..=3717`), loaded via [`Art::load_optional`].
pub const HELP_RESOURCES: [i32; 18] = [
    3700, 3701, 3702, 3703, 3704, 3705, 3706, 3707, 3708, 3709, 3710, 3711, 3712, 3713, 3714, 3715,
    3716, 3717,
];

/// Maps an [`OptionsHelp`] setting to its `kPopupOpt*` UIGraphics bitmap ID
/// from the native table at `0x66c394..0x66c410`.
pub const fn help_bitmap(help: OptionsHelp) -> i32 {
    match help {
        OptionsHelp::AntiAlias => 3700,
        OptionsHelp::Shadows => 3711,
        OptionsHelp::Lighting => 3706,
        OptionsHelp::InterfaceEffects => 3714,
        OptionsHelp::TerrainDetail => 3713,
        OptionsHelp::CharacterDetail => 3703,
        OptionsHelp::Fx => 3710,
        OptionsHelp::Music => 3708,
        OptionsHelp::Vox => 3715,
        OptionsHelp::AutoCentering => 3701,
        OptionsHelp::FreeWill => 3705,
        OptionsHelp::EdgeScrolling => 3704,
        OptionsHelp::SimInBackground => 3712,
        OptionsHelp::ResetTutorial => 3717,
        OptionsHelp::QuickTips => 3709,
        OptionsHelp::AutoSnapshot => 3702,
        OptionsHelp::LivePip => 3707,
        OptionsHelp::ExportHtml => 3716,
    }
}

/// Index of `help` in [`OptionsTexts::help_entries`] (`0..18`).
const fn help_index(help: OptionsHelp) -> usize {
    match help {
        OptionsHelp::AntiAlias => 0,
        OptionsHelp::Shadows => 1,
        OptionsHelp::Lighting => 2,
        OptionsHelp::InterfaceEffects => 3,
        OptionsHelp::TerrainDetail => 4,
        OptionsHelp::CharacterDetail => 5,
        OptionsHelp::Fx => 6,
        OptionsHelp::Music => 7,
        OptionsHelp::Vox => 8,
        OptionsHelp::AutoCentering => 9,
        OptionsHelp::FreeWill => 10,
        OptionsHelp::EdgeScrolling => 11,
        OptionsHelp::SimInBackground => 12,
        OptionsHelp::ResetTutorial => 13,
        OptionsHelp::QuickTips => 14,
        OptionsHelp::AutoSnapshot => 15,
        OptionsHelp::LivePip => 16,
        OptionsHelp::ExportHtml => 17,
    }
}

/// Per-lot Options control visibility rules from Complete Collection 1.91
/// (`0x4f2490..0x4f24e7`, `0x5174ac..0x5174c5`, `0x5177aa..0x517833`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LotVisibility {
    /// Whether the Save action button is visible (`!no_save`, `0x5174ac..0x5174c5`).
    pub can_save: bool,
    /// Whether the Play subpanel's Free Will label and checkbox are visible
    /// (`0x5177aa..0x51788e`: hidden on Downtown `21..=30` (`0x40749a`),
    /// Studio Town `81..=89` (`0x4074b1`), and Magic Town `93..=98` (`0x428825`);
    /// explicitly shown on Vacation Island `40..=49` (`0x4074c8`),
    /// Old Town Community (`zoning == 2`, `0x40747c`), and Residential lots).
    pub show_free_will: bool,
    /// Whether the Play subpanel's Reset Tutorial button is visible
    /// (`0x5175d5..0x5175f4`, `0x40745f`: residential lots only).
    pub show_reset_tutorial: bool,
}

impl LotVisibility {
    /// Computes native Options visibility from the lot's ID (`globals[10]`,
    /// house `+0x24`), neighborhood zoning (`0x42b9e3`: 1 residential, 2
    /// community/Studio Town), whether a family occupies the lot
    /// (`[lot + 0x10c] != 0`), and whether the lot is open in edit/build mode
    /// (`[house + 0x50] != 0`).
    pub const fn for_lot(
        lot_id: i16,
        zoning: Option<i32>,
        has_family: bool,
        edit_mode: bool,
    ) -> Self {
        let is_residential = matches!(zoning, Some(1));
        let is_downtown = lot_id >= 21 && lot_id <= 30;
        let is_studio_town = lot_id >= 81 && lot_id <= 89;
        let is_magic_town = lot_id >= 93 && lot_id <= 98;
        // 0x4f2490..0x4f24e7: community/Studio Town (zoning == 2) or Magic Town
        // lots 93..=99 (`0x5d..=0x63`) cannot save while visiting (`!edit_mode`);
        // other lots cannot save while visiting without an active family.
        let no_save = if matches!(zoning, Some(2)) || (lot_id >= 93 && lot_id <= 99) {
            !edit_mode
        } else {
            !edit_mode && !has_family
        };
        Self {
            can_save: !no_save,
            // 0x5177b2 (`0x40749a` Downtown 21..=30), 0x51780a (`0x4074b1`
            // Studio Town 81..=89), and 0x51781b (`0x428825` Magic Town 93..=98)
            // hide Free Will; 0x51782c (`0x4074c8` Vacation Island 40..=49),
            // 0x51783d (`0x40747c` zoning == 2 Community), and Residential keep it visible.
            show_free_will: !(is_downtown || is_studio_town || is_magic_town),
            show_reset_tutorial: is_residential,
        }
    }

    /// Fallback visibility when only the residential boolean is supplied.
    pub const fn from_residential(residential: bool) -> Self {
        Self {
            can_save: residential,
            show_free_will: true,
            show_reset_tutorial: residential,
        }
    }
}

impl Default for LotVisibility {
    fn default() -> Self {
        Self::from_residential(true)
    }
}

/// String table ID for options layout coordinates and labels in
/// `GameData/UIText.iff` (`0x5140db`: `0x91 = 145`).
pub const OPTIONS_STRINGS: u16 = 145;

/// String table ID for Save / Quit / Reset Tutorial confirmation dialogs
/// in `GameData/UIText.iff` (`0x517c03`, `0x5168b0`).
pub const CONFIRM_STRINGS: u16 = 153;

/// Maximum volume slider step (`0..=10`, 11 steps matching `0x5153a8` and
/// `sims_audio::player::VOLUME_STEPS` at `0x67156c`).
pub const VOLUME_MAX: u8 = 10;

/// Width and height of one cell of `optslideron.bmp` / `optslideroff.bmp`
/// (44×14 split into 4 cells: left cap, middle tile, right cap, thumb).
pub const SLIDER_CELL_WIDTH: i32 = 11;
pub const SLIDER_HEIGHT: i32 = 14;

/// Default label font size (`[0x6f2ae8]`, 9 pt, `0x5145b0`, `0x51509e`).
const LABEL_FONT: u8 = 9;
/// Radio column header font size (`[0x6f2ae4]`, 8 pt, `0x514e12`).
const HEADER_FONT: u8 = 8;

/// Divider positions relative to the options strip (`cWinOptions::OnDraw`,
/// `0x516160..0x5161bd`).
const HORIZ_DIVIDER_POS: [i32; 2] = [16, 51];
const VERT_DIVIDER_POS: [i32; 2] = [222, 6];

/// Checkbox and radio cell dimensions (`optcheckbox.bmp` 120×21 / 6,
/// `optradio.bmp` 120×20 / 6, `0x5e4dbb`, `0x5376cd`).
const CHECKBOX_SIZE: [i32; 2] = [20, 21];
const RADIO_CELL_SIZE: [i32; 2] = [20, 20];
/// Play subpanel checkbox x-offset from label (`120 / -4 = -30`, `0x515de7`).
const PLAY_CHECKBOX_OFFSET_X: i32 = 30;

/// Graphics subpanel settings (`cTSOptions`, `0x4e5f5d..0x4e6090`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphicsOptions {
    /// Anti-aliasing (`[0x6f0189]`). Hidden in Complete Collection 1.91
    /// (`0x517485..0x51749b`), kept in state for parity.
    pub anti_alias: bool,
    /// Object shadows (`[0x6f0188]`, `0x4e5fac`).
    pub shadows: bool,
    /// Object lighting (`[0x6f018a]`, `0x4e5fcf`).
    pub lighting: bool,
    /// Interface effects / UI translucency (`0x4e5ff2`).
    pub interface_effects: bool,
    /// Terrain detail (`0x4e6000` / `0x4e6025`).
    pub terrain_detail: DetailLevel,
    /// Character detail (`0x4e606a` / `0x4e6085`).
    pub character_detail: DetailLevel,
}

impl Default for GraphicsOptions {
    fn default() -> Self {
        Self {
            anti_alias: false,
            shadows: true,
            lighting: true,
            interface_effects: true,
            terrain_detail: DetailLevel::High,
            character_detail: DetailLevel::High,
        }
    }
}

/// Sound subpanel settings (`0x4e6166..0x4e620e`): three 11-step (`0..=10`)
/// volume sliders corresponding to audio kinds 37, 38, and 39.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoundOptions {
    /// Sound effects volume step `0..=10` (kind 37, `0x4e6182 -> 0x57ad03`).
    pub fx: u8,
    /// Music volume step `0..=10` (kind 38, `0x4e61bb -> 0x57ad6a`).
    pub music: u8,
    /// Voice volume step `0..=10` (kind 39, `0x4e61f4 -> 0x57add1`).
    pub vox: u8,
}

impl Default for SoundOptions {
    fn default() -> Self {
        Self {
            fx: VOLUME_MAX,
            music: VOLUME_MAX,
            vox: VOLUME_MAX,
        }
    }
}

impl SoundOptions {
    /// Reads the current step (`0..=10`) for `channel`.
    pub fn get(self, channel: VolumeChannel) -> u8 {
        match channel {
            VolumeChannel::Fx => self.fx,
            VolumeChannel::Music => self.music,
            VolumeChannel::Vox => self.vox,
        }
    }

    /// Sets `channel` to `step` (clamped to `0..=10`), returning whether the
    /// stored step changed.
    pub fn set(&mut self, channel: VolumeChannel, step: u8) -> bool {
        let clamped = step.min(VOLUME_MAX);
        let slot = match channel {
            VolumeChannel::Fx => &mut self.fx,
            VolumeChannel::Music => &mut self.music,
            VolumeChannel::Vox => &mut self.vox,
        };
        if *slot == clamped {
            false
        } else {
            *slot = clamped;
            true
        }
    }

    /// Returns `[fx, music, vox]`.
    pub fn steps(self) -> [u8; 3] {
        [self.fx, self.music, self.vox]
    }
}

/// Play subpanel settings (`0x4e6211..0x4e6310`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayOptions {
    /// Auto-centering (`[0x6c2981]`, `0x4e6211`).
    pub auto_centering: bool,
    /// Free will (`[0x6c2980]`, `0x4e6223`).
    pub free_will: bool,
    /// Edge scrolling (`[options + 0xc]`, `0x4e6242`).
    pub edge_scrolling: bool,
    /// Simulate in background (`![0x6c9fa1]`, `0x4e6229`).
    pub sim_in_background: bool,
    /// Quick tips (`[options + 2]`, `0x4e629d`).
    pub quick_tips: bool,
    /// Auto snapshot (`0x4e62ec`).
    pub auto_snapshot: bool,
    /// Live picture-in-picture (`0x4cfe18`).
    pub live_pip: bool,
    /// Export HTML (`0x4e6304`).
    pub export_html: bool,
}

impl Default for PlayOptions {
    fn default() -> Self {
        Self {
            auto_centering: true,
            free_will: true,
            edge_scrolling: true,
            sim_in_background: false,
            quick_tips: false,
            auto_snapshot: true,
            live_pip: true,
            export_html: true,
        }
    }
}

/// Full state of the Options panel (`cWinOptions`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OptionsState {
    /// Which subpanel is open (`[esi + 0xdc]`): `None` on initial open
    /// (`0x514083` / `0x5179eb` calls `ShowSubpanel(0)`), or one of Graphics,
    /// Sound, Play.
    pub tab: Option<OptionsTab>,
    /// Which Options setting's help balloon is currently open (`[esi + 0x194]`,
    /// `0x516719..0x51679a`).
    pub help: Option<OptionsHelp>,
    pub graphics: GraphicsOptions,
    pub sound: SoundOptions,
    pub play: PlayOptions,
    /// Native `cTSOptions` header flag `[esi + 0]` serialized at `Optn` byte 14
    /// (`0x4e6541..0x4e6554`), preserved across round-trips.
    pub opt_byte0: u8,
    /// Native `cTSOptions` field `[esi + 8]` serialized at `Optn` bytes 28..32
    /// (`0x4e6772..0x4e6780`), preserved across round-trips.
    pub opt_dword8: u32,
}

/// IFF chunk ID of the `Optn` preferences record inside `UserData/Options.iff`
/// (`0x4e64a6..0x4e64cd`: tag `Optn`, id `0x80` = 128).
pub const OPTN_CHUNK_ID: u16 = 128;

impl OptionsState {
    /// Decodes a native `Optn` chunk body (`cTSOptions::Serialize`,
    /// `0x4e6505..0x4e685e`, `UserData/Options.iff` chunk `Optn` id `128`).
    ///
    /// Exact 1.91 byte layout:
    /// - `12..14`: `u16` version
    /// - `14`: `[esi + 0]` (`opt_byte0`, `0x4e6545`)
    /// - `15`: `anti_alias` (`[0x6f0189]`, `0x4e6556..0x4e6579`)
    /// - `16`: `shadows` (`[0x6f0188]`, `0x4e658d`)
    /// - `17`: `lighting` (`[0x6f018a]`, `0x4e65be`)
    /// - `18`: `terrain_detail` (`0x4e65f1`)
    /// - `19`: `character_detail` (`0x4e6627`)
    /// - `20..=22`: `sound.{fx, music, vox}` (`0x4e6649..0x4e66b5`)
    /// - `23..=26`: `play.{auto_centering, free_will, edge_scrolling, sim_in_background}`
    ///   (`0x4e66c8..0x4e674e`)
    /// - `27`: `graphics.interface_effects` (`[esi + 1]`, `0x4e6753..0x4e676d`)
    /// - `28..32`: `[esi + 8]` (`opt_dword8`, version > 1, `0x4e6772`)
    /// - `32..=34`: `play.{quick_tips, auto_snapshot, live_pip}` (version > 2)
    /// - `35`: `play.export_html` (version > 3)
    ///
    /// Returns `None` if `body` is shorter than the 28-byte v1 header+payload
    /// or lacks the `'Optn'` LE signature (`6e 74 70 4f` at bytes `8..12`).
    pub fn from_optn(body: &[u8]) -> Option<Self> {
        if body.len() < 28 || &body[8..12] != b"ntpO" {
            return None;
        }
        let version = u16::from_le_bytes([body[12], body[13]]);
        let detail = |b: u8| match b {
            0 => DetailLevel::Low,
            1 => DetailLevel::Med,
            _ => DetailLevel::High,
        };
        let opt_dword8 = if version > 1 && body.len() >= 32 {
            u32::from_le_bytes([body[28], body[29], body[30], body[31]])
        } else {
            0
        };
        // `0x4e650f..0x4e6513` (`bl = 1`), `0x4e6814..0x4e681a` (`version <= 2`
        // defaults Quick Tips, Auto Snapshot, and Live PIP to `true`), and
        // `0x4e6842` (`version <= 3` defaults Export HTML to `true`).
        let (quick_tips, auto_snapshot, live_pip) = if version > 2 && body.len() >= 35 {
            (body[32] != 0, body[33] != 0, body[34] != 0)
        } else {
            (true, true, true)
        };
        let export_html = if version > 3 && body.len() >= 36 {
            body[35] != 0
        } else {
            true
        };
        Some(Self {
            tab: None,
            help: None,
            graphics: GraphicsOptions {
                // `0x4e6541..0x4e662c`, `0x4e6753..0x4e676d`:
                anti_alias: body[15] != 0,
                shadows: body[16] != 0,
                lighting: body[17] != 0,
                interface_effects: body[27] != 0,
                terrain_detail: detail(body[18]),
                character_detail: detail(body[19]),
            },
            sound: SoundOptions {
                // `0x4e6638..0x4e66c3`: FX, Music, Vox volume steps `0..=10`.
                fx: body[20].min(VOLUME_MAX),
                music: body[21].min(VOLUME_MAX),
                vox: body[22].min(VOLUME_MAX),
            },
            play: PlayOptions {
                // `0x4e66c8..0x4e674e`, `0x4e6784..0x4e6842`:
                auto_centering: body[23] != 0,
                free_will: body[24] != 0,
                edge_scrolling: body[25] != 0,
                sim_in_background: body[26] != 0,
                quick_tips,
                auto_snapshot,
                live_pip,
                export_html,
            },
            opt_byte0: u8::from(body[14] != 0),
            opt_dword8,
        })
    }

    /// Encodes this [`OptionsState`] as a 36-byte v4 native `Optn` chunk body
    /// (`cTSOptions::Serialize`, `0x4e6505..0x4e685e`).
    pub fn to_optn(&self) -> Vec<u8> {
        let mut body = Vec::with_capacity(36);
        // 12-byte native cRZSerialize header matching `UserData/Options.iff`.
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(&62u32.to_le_bytes());
        body.extend_from_slice(b"ntpO");
        // Version 4 (matching installed Complete Collection `Options.iff`).
        body.extend_from_slice(&4u16.to_le_bytes());
        body.push(u8::from(self.opt_byte0 != 0));
        body.push(u8::from(self.graphics.anti_alias));
        body.push(u8::from(self.graphics.shadows));
        body.push(u8::from(self.graphics.lighting));
        body.push(self.graphics.terrain_detail as u8);
        body.push(self.graphics.character_detail as u8);
        body.push(self.sound.fx.min(VOLUME_MAX));
        body.push(self.sound.music.min(VOLUME_MAX));
        body.push(self.sound.vox.min(VOLUME_MAX));
        body.push(u8::from(self.play.auto_centering));
        body.push(u8::from(self.play.free_will));
        body.push(u8::from(self.play.edge_scrolling));
        body.push(u8::from(self.play.sim_in_background));
        body.push(u8::from(self.graphics.interface_effects));
        body.extend_from_slice(&self.opt_dword8.to_le_bytes());
        body.push(u8::from(self.play.quick_tips));
        body.push(u8::from(self.play.auto_snapshot));
        body.push(u8::from(self.play.live_pip));
        body.push(u8::from(self.play.export_html));
        body
    }

    /// Loads an `Options.iff` file from `path` (chunk `Optn` id `128`).
    pub fn load_iff(path: &std::path::Path) -> Result<Self> {
        let bytes = std::fs::read(path).map_err(Error::Io)?;
        let iff = Iff::parse(bytes).map_err(Error::Format)?;
        let chunk = iff.get(b"Optn", OPTN_CHUNK_ID).ok_or_else(|| {
            Error::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Options.iff has no Optn 128 chunk",
            ))
        })?;
        Self::from_optn(iff.data(chunk)).ok_or_else(|| {
            Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid Optn chunk body",
            ))
        })
    }

    /// Loads preferences from the install's `UserData/Options.iff` (`Optn` 128).
    pub fn load_install(install: &Install) -> Result<Self> {
        let path = install
            .file("UserData/Options.iff")
            .map_err(Error::Content)?;
        Self::load_iff(&path)
    }

    /// Writes this [`OptionsState`] as a valid IFF 2.5 file containing chunk
    /// `Optn` id `128`, readable by [`OptionsState::load_iff`] and the original.
    pub fn save_iff(&self, path: &std::path::Path) -> Result<()> {
        let body = self.to_optn();
        let mut out = Vec::with_capacity(64 + 76 + body.len());
        let mut header = [0u8; 64];
        let sig = b"IFF FILE 2.5:TYPE FOLLOWED BY SIZE\0 JAMIE DOORNBOS & MAXIS 1\0\0\0\0";
        header[..60].copy_from_slice(&sig[..60]);
        out.extend_from_slice(&header);
        out.extend_from_slice(b"Optn");
        let chunk_size = u32::try_from(76 + body.len()).unwrap_or(u32::MAX);
        out.extend_from_slice(&chunk_size.to_be_bytes());
        out.extend_from_slice(&OPTN_CHUNK_ID.to_be_bytes());
        out.extend_from_slice(&0u16.to_be_bytes());
        out.extend_from_slice(&[0u8; 64]);
        out.extend_from_slice(&body);
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(Error::Io)?;
        }
        std::fs::write(path, out).map_err(Error::Io)
    }

    /// Reads whether `toggle` is currently checked.
    pub fn is_checked(self, toggle: OptionsToggle) -> bool {
        match toggle {
            OptionsToggle::AntiAlias => self.graphics.anti_alias,
            OptionsToggle::Shadows => self.graphics.shadows,
            OptionsToggle::Lighting => self.graphics.lighting,
            OptionsToggle::InterfaceEffects => self.graphics.interface_effects,
            OptionsToggle::AutoCentering => self.play.auto_centering,
            OptionsToggle::FreeWill => self.play.free_will,
            OptionsToggle::EdgeScrolling => self.play.edge_scrolling,
            OptionsToggle::SimInBackground => self.play.sim_in_background,
            OptionsToggle::QuickTips => self.play.quick_tips,
            OptionsToggle::AutoSnapshot => self.play.auto_snapshot,
            OptionsToggle::LivePip => self.play.live_pip,
            OptionsToggle::ExportHtml => self.play.export_html,
        }
    }

    /// Reads the selected level of `radio`.
    pub fn detail(self, radio: OptionsRadio) -> DetailLevel {
        match radio {
            OptionsRadio::TerrainDetail => self.graphics.terrain_detail,
            OptionsRadio::CharacterDetail => self.graphics.character_detail,
        }
    }

    /// Applies an options-panel command, returning whether state changed.
    pub fn apply(&mut self, command: &Command) -> bool {
        match *command {
            Command::OpenOptionsTab(tab) => {
                // 0x5167d5..0x5167fb: clicking the active tab closes it to 0 (`None`).
                self.tab = if self.tab == Some(tab) {
                    None
                } else {
                    Some(tab)
                };
                self.help = None;
                true
            }
            Command::OptionsHelp(help) => {
                // 0x516719..0x51679a: clicking the same setting closes its
                // balloon; clicking another switches to that setting's balloon.
                self.help = if self.help == Some(help) {
                    None
                } else {
                    Some(help)
                };
                true
            }
            Command::ToggleOption(toggle) => {
                let slot = match toggle {
                    OptionsToggle::AntiAlias => &mut self.graphics.anti_alias,
                    OptionsToggle::Shadows => &mut self.graphics.shadows,
                    OptionsToggle::Lighting => &mut self.graphics.lighting,
                    OptionsToggle::InterfaceEffects => &mut self.graphics.interface_effects,
                    OptionsToggle::AutoCentering => &mut self.play.auto_centering,
                    OptionsToggle::FreeWill => &mut self.play.free_will,
                    OptionsToggle::EdgeScrolling => &mut self.play.edge_scrolling,
                    OptionsToggle::SimInBackground => &mut self.play.sim_in_background,
                    OptionsToggle::QuickTips => &mut self.play.quick_tips,
                    OptionsToggle::AutoSnapshot => &mut self.play.auto_snapshot,
                    OptionsToggle::LivePip => &mut self.play.live_pip,
                    OptionsToggle::ExportHtml => &mut self.play.export_html,
                };
                *slot = !*slot;
                true
            }
            Command::SetDetail { radio, level } => {
                let slot = match radio {
                    OptionsRadio::TerrainDetail => &mut self.graphics.terrain_detail,
                    OptionsRadio::CharacterDetail => &mut self.graphics.character_detail,
                };
                if *slot == level {
                    false
                } else {
                    *slot = level;
                    true
                }
            }
            Command::SetVolume { channel, step } => self.sound.set(channel, step),
            _ => false,
        }
    }
}

/// Parsed coordinates and labels from `GameData/UIText.iff STR# 145` and
/// confirmation strings from `STR# 153`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionsTexts {
    pub nbhd_pos: [i32; 2],
    pub save_pos: [i32; 2],
    pub quit_pos: [i32; 2],
    pub graphics_pos: [i32; 2],
    pub sound_pos: [i32; 2],
    pub play_pos: [i32; 2],

    pub graphics_label_origin: [i32; 2],
    pub graphics_label_step: [i32; 2],
    pub graphics_checkbox_origin: [i32; 2],
    pub graphics_checkbox_step: [i32; 2],
    pub graphics_labels: [String; 4],

    pub detail_label_origin: [i32; 2],
    pub detail_label_step: [i32; 2],
    pub detail_labels: [String; 2],
    pub detail_radio_size: [i32; 2],
    pub detail_radio_origin: [i32; 2],
    pub detail_radio_step: [i32; 2],
    pub detail_headers: [String; 3],

    pub sound_origin: [i32; 2],
    pub sound_step: [i32; 2],
    pub sound_labels: [String; 3],
    pub slider_width: i32,

    pub play_origin: [i32; 2],
    pub play_step: [i32; 2],
    pub play_left_labels: [String; 4],
    pub play_right_labels: [String; 4],

    /// Action, tab, and Reset Tutorial button tooltips from `STR# 145`
    /// (`[1, 3, 5, 7, 9, 11, 69]`), shown on hover when Quick Tips is enabled
    /// (`0x5164ef..0x5164ff -> 0x517a6f`).
    pub action_tooltips: [String; 7],
    /// Per-setting help balloon `(title, body)` pairs loaded from `STR# 145`
    /// (`0x516719..0x51679a`), indexed by [`help_index`].
    pub help_entries: [(String, String); 18],

    /// Confirmation dialog `(title, body)` pairs loaded from `STR# 153`.
    pub confirm_quit: (String, String),
    pub confirm_save: (String, String),
    pub confirm_save_before_quit: (String, String),
    pub confirm_reset_tutorial: (String, String),
}

impl Default for OptionsTexts {
    fn default() -> Self {
        Self {
            nbhd_pos: [88, 6],
            save_pos: [29, 6],
            quit_pos: [159, 6],
            graphics_pos: [29, 58],
            sound_pos: [89, 58],
            play_pos: [151, 58],

            graphics_label_origin: [275, 9],
            graphics_label_step: [200, 20],
            graphics_checkbox_origin: [245, 7],
            graphics_checkbox_step: [200, 20],
            graphics_labels: [
                "Anti-alias".into(),
                "Shadows".into(),
                "Lighting".into(),
                "Interface Effects".into(),
            ],

            detail_label_origin: [480, 30],
            detail_label_step: [200, 26],
            detail_labels: ["Terrain Detail".into(), "Character Detail".into()],
            detail_radio_size: [80, 20],
            detail_radio_origin: [485, 30],
            detail_radio_step: [200, 26],
            detail_headers: ["Low".into(), "Med".into(), "High".into()],

            sound_origin: [305, 13],
            sound_step: [200, 28],
            sound_labels: ["F/X".into(), "Music".into(), "Vox".into()],
            slider_width: 130,

            play_origin: [275, 9],
            play_step: [200, 20],
            play_left_labels: [
                "Auto-Centering".into(),
                "Free Will".into(),
                "Edge Scrolling".into(),
                "Sim In Background".into(),
            ],
            play_right_labels: [
                "Quick Tips".into(),
                "Auto Snapshot".into(),
                "Live PIP".into(),
                "Export HTML".into(),
            ],

            action_tooltips: [
                "Neighborhood".into(),
                "Save".into(),
                "Quit".into(),
                "Graphics Options".into(),
                "Sound Options".into(),
                "Play Options".into(),
                "Reset Tutorial".into(),
            ],
            help_entries: std::array::from_fn(|_| (String::new(), String::new())),

            confirm_quit: ("Quit".into(), "Quit?".into()),
            confirm_save: ("Save".into(), "Save?".into()),
            confirm_save_before_quit: ("Quit".into(), "Save before quitting?".into()),
            confirm_reset_tutorial: ("Reset Tutorial".into(), "Reset tutorial?".into()),
        }
    }
}

/// Parses a `"(x;y)"` coordinate pair as `0x5001cc` (`sscanf(s, "(%d;%d)")`)
/// does in `cWinOptions::Init`.
pub fn parse_pair(text: &str) -> Option<[i32; 2]> {
    let inner = text.trim().strip_prefix('(')?.strip_suffix(')')?;
    let (a, b) = inner.split_once(';')?;
    Some([a.trim().parse().ok()?, b.trim().parse().ok()?])
}

impl OptionsTexts {
    /// Reads `STR# 145` and `STR# 153` from `GameData/UIText.iff`. Any missing
    /// or malformed entry keeps its default value.
    pub fn load(install: &Install) -> Result<Self> {
        let path = install
            .file("GameData/UIText.iff")
            .map_err(Error::Content)?;
        let bytes = std::fs::read(&path).map_err(Error::Io)?;
        let iff = Iff::parse(bytes).map_err(Error::Format)?;
        let mut texts = Self::default();
        if let Some(chunk) = iff.get(b"STR#", OPTIONS_STRINGS) {
            let table = StringTable::parse(iff.data(chunk)).map_err(Error::Format)?;
            let get = |i: usize| table.get(i, 1);
            let pair = |i: usize| get(i).and_then(parse_pair);
            if let Some(p) = pair(0) {
                texts.nbhd_pos = p;
            }
            if let Some(p) = pair(2) {
                texts.save_pos = p;
            }
            if let Some(p) = pair(4) {
                texts.quit_pos = p;
            }
            if let Some(p) = pair(6) {
                texts.graphics_pos = p;
            }
            if let Some(p) = pair(8) {
                texts.sound_pos = p;
            }
            if let Some(p) = pair(10) {
                texts.play_pos = p;
            }
            if let Some(p) = pair(12) {
                texts.graphics_label_origin = p;
            }
            if let Some(p) = pair(13) {
                texts.graphics_label_step = p;
            }
            for (slot, idx) in texts.graphics_labels.iter_mut().zip([14, 17, 20, 23]) {
                if let Some(s) = get(idx) {
                    *slot = s.to_owned();
                }
            }
            if let Some(p) = pair(26) {
                texts.detail_label_origin = p;
            }
            if let Some(p) = pair(27) {
                texts.detail_label_step = p;
            }
            for (slot, idx) in texts.detail_labels.iter_mut().zip([28, 31]) {
                if let Some(s) = get(idx) {
                    *slot = s.to_owned();
                }
            }
            if let Some(p) = pair(34) {
                texts.graphics_checkbox_origin = p;
            }
            if let Some(p) = pair(35) {
                texts.graphics_checkbox_step = p;
            }
            if let Some(p) = pair(36) {
                texts.detail_radio_size = p;
            }
            if let Some(p) = pair(37) {
                texts.detail_radio_origin = p;
            }
            if let Some(p) = pair(38) {
                texts.detail_radio_step = p;
            }
            for (slot, idx) in texts.detail_headers.iter_mut().zip([39, 40, 41]) {
                if let Some(s) = get(idx) {
                    *slot = s.to_owned();
                }
            }
            if let Some(p) = pair(42) {
                texts.sound_origin = p;
            }
            if let Some(p) = pair(43) {
                texts.sound_step = p;
            }
            for (slot, idx) in texts.sound_labels.iter_mut().zip([44, 47, 50]) {
                if let Some(s) = get(idx) {
                    *slot = s.to_owned();
                }
            }
            if let Some(p) = pair(53)
                && p[0] > 0
            {
                texts.slider_width = p[0];
            }
            if let Some(p) = pair(54) {
                texts.play_origin = p;
            }
            if let Some(p) = pair(55) {
                texts.play_step = p;
            }
            for (slot, idx) in texts.play_left_labels.iter_mut().zip([56, 59, 62, 65]) {
                if let Some(s) = get(idx) {
                    *slot = s.to_owned();
                }
            }
            for (slot, idx) in texts.play_right_labels.iter_mut().zip([72, 75, 78, 81]) {
                if let Some(s) = get(idx) {
                    *slot = s.to_owned();
                }
            }
            for (slot, idx) in texts
                .action_tooltips
                .iter_mut()
                .zip([1, 3, 5, 7, 9, 11, 69])
            {
                if let Some(s) = get(idx) {
                    *slot = s.to_owned();
                }
            }
            const HELP_STR_INDICES: [(usize, usize); 18] = [
                (15, 16), // Anti-alias
                (18, 19), // Shadows
                (21, 22), // Lighting
                (24, 25), // Interface Effects
                (29, 30), // Terrain Detail
                (32, 33), // Character Detail
                (45, 46), // F/X
                (48, 49), // Music
                (51, 52), // Vox
                (57, 58), // Auto-Centering
                (60, 61), // Free Will
                (63, 64), // Edge Scrolling
                (66, 67), // Sim In Background
                (70, 71), // Reset Tutorial
                (73, 74), // Quick Tips
                (76, 77), // Auto Snapshot
                (79, 80), // Live PIP
                (82, 83), // Export HTML
            ];
            for (slot, (title_i, body_i)) in texts.help_entries.iter_mut().zip(HELP_STR_INDICES) {
                if let Some(t) = get(title_i) {
                    slot.0 = t.trim().to_owned();
                }
                if let Some(b) = get(body_i) {
                    slot.1 = b.to_owned();
                }
            }
        }
        if let Some(chunk) = iff.get(b"STR#", CONFIRM_STRINGS) {
            let table = StringTable::parse(iff.data(chunk)).map_err(Error::Format)?;
            let get = |i: usize| table.get(i, 1);
            if let (Some(title), Some(body)) = (get(1), get(2)) {
                texts.confirm_quit = (title.to_owned(), body.to_owned());
            }
            if let (Some(title), Some(body)) = (get(3), get(4)) {
                texts.confirm_save = (title.to_owned(), body.to_owned());
            }
            if let (Some(title), Some(body)) = (get(5), get(6)) {
                texts.confirm_save_before_quit = (title.to_owned(), body.to_owned());
            }
            if let (Some(title), Some(body)) = (get(7), get(8)) {
                texts.confirm_reset_tutorial = (title.to_owned(), body.to_owned());
            }
        }
        Ok(texts)
    }

    /// Returns the `(title, body)` help strings for `help` (from `STR# 145`).
    pub fn help_text(&self, help: OptionsHelp) -> (&str, &str) {
        let (title, body) = &self.help_entries[help_index(help)];
        (title.as_str(), body.as_str())
    }
}

/// A laid-out Options help balloon (`cWinOptions` -> `cWinBalloon`,
/// `0x516719..0x51679a`, `0x50fc30..0x50fea6`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionsBalloon {
    pub rect: Rect,
    pub help: OptionsHelp,
    pub title: String,
    pub lines: Vec<String>,
    pub bitmap: i32,
    pub text_left: i32,
}

const BALLOON_WIDTH: i32 = 559;
const BALLOON_MIN_HEIGHT: i32 = 127;
const BALLOON_PADDING: i32 = 24;
const BALLOON_SLOT_WIDTH: i32 = 168;
const BALLOON_SLOT_HEIGHT: i32 = 127;
const BALLOON_TEXT_RIGHT: i32 = 535;
const BALLOON_TEXT_INDENT: i32 = 10;
const BALLOON_TITLE_TOP: i32 = 12;
const BALLOON_TITLE_FONT: u8 = 10;
const BALLOON_BODY_FONT: u8 = 9;
const TOOLTIP_BACK: [u8; 3] = rgb565(0x000a);

/// Lays out an Options help balloon anchored above `strip` (`0x50fc30..0x50fd40`).
pub fn layout_balloon(
    strip: Rect,
    help: OptionsHelp,
    texts: &OptionsTexts,
    art: Option<&Art>,
) -> OptionsBalloon {
    let (title, body) = texts.help_text(help);
    let title = if title.is_empty() {
        format!("{help:?}")
    } else {
        title.to_owned()
    };
    let bitmap = help_bitmap(help);
    let has_picture = art.is_some_and(|a| a.image(bitmap).is_some());
    let text_left = if has_picture { BALLOON_SLOT_WIDTH } else { 5 };
    let window_w = strip.x.saturating_add(strip.width);
    let width = BALLOON_WIDTH.min(window_w.max(1));
    let right_margin = BALLOON_WIDTH - BALLOON_TEXT_RIGHT;
    let text_w = (width - right_margin - text_left - BALLOON_TEXT_INDENT).max(60);
    let title_h = art
        .and_then(|a| a.font(BALLOON_TITLE_FONT))
        .map_or(16, |f| f.line_height.max(1));
    let (lines, body_line_h) = match art.and_then(|a| a.font(BALLOON_BODY_FONT)) {
        Some(font) => (font.wrap(body, text_w), font.line_height.max(1)),
        None => (
            if body.is_empty() {
                Vec::new()
            } else {
                vec![body.to_owned()]
            },
            14,
        ),
    };
    let text_h = title_h + 4 + body_line_h * i32::try_from(lines.len()).unwrap_or(0);
    let height = (text_h + BALLOON_PADDING).max(BALLOON_MIN_HEIGHT);
    // `0x5100b2..0x5100c7`: native Options help balloons anchor to the right
    // edge of the window (`window_w - width`), clamping at `0` on narrow widths.
    let x = (window_w - width).max(0);
    let y = (strip.y - height).max(0);
    OptionsBalloon {
        rect: Rect::new(x, y, width, height),
        help,
        title,
        lines,
        bitmap,
        text_left,
    }
}

/// Draws an [`OptionsBalloon`] using 9-slice `PopupInfoTiles.bmp` (`3008`),
/// its `kPopupOpt*` illustration (`3700..3717`), title (font 10), and body
/// lines (font 9).
pub fn draw_balloon(frame: &mut Frame, art: &Art, balloon: &OptionsBalloon) {
    let r = balloon.rect;
    if let Some(sheet) = art.image(dialog::BACKGROUND) {
        dialog::draw_nine_slice(frame, sheet, r);
    } else {
        frame.fill(r, TOOLTIP_BACK);
    }
    if let Some(pic) = art.image(balloon.bitmap) {
        let px = r.x + (BALLOON_SLOT_WIDTH - i32::try_from(pic.width).unwrap_or(0)) / 2;
        let py = r.y + (BALLOON_SLOT_HEIGHT - i32::try_from(pic.height).unwrap_or(0)) / 2;
        frame.draw_all(pic, [px, py]);
    }
    let tx = r.x.saturating_add(balloon.text_left + BALLOON_TEXT_INDENT);
    let mut ty = r.y.saturating_add(BALLOON_TITLE_TOP);
    if let Some(font) = art.font(BALLOON_TITLE_FONT) {
        font.draw(frame, &balloon.title, [tx, ty], LABEL);
        ty = ty.saturating_add(font.line_height.max(1) + 4);
    }
    if let Some(font) = art.font(BALLOON_BODY_FONT) {
        let step = font.line_height.max(1);
        for line in &balloon.lines {
            font.draw(frame, line, [tx, ty], LABEL);
            ty = ty.saturating_add(step);
        }
    }
}

/// Draws `balloon` at `alpha`, for its 250 ms fade (the Options help
/// balloon opens and closes through the same `cWinBalloon` code as the
/// live panel's, `0x516615`/`0x51679a`, `0x5165ac`/`0x516734`).
pub fn draw_balloon_with_alpha(frame: &mut Frame, art: &Art, balloon: &OptionsBalloon, alpha: u8) {
    if alpha == 255 {
        draw_balloon(frame, art, balloon);
        return;
    }
    crate::balloon::composite(frame, balloon.rect, alpha, |scratch, [x0, y0]| {
        let mut shifted = balloon.clone();
        shifted.rect.x -= x0;
        shifted.rect.y -= y0;
        draw_balloon(scratch, art, &shifted);
    });
}

/// Computes the screen rectangle of a Quick Tip tooltip box above `anchor`
/// (`0x517a6f`), matching [`draw_tooltip`] byte-for-byte.
pub fn tooltip_rect(size: [i32; 2], art: &Art, anchor: Rect, text: &str) -> Option<Rect> {
    let font = art.font(LABEL_FONT)?;
    if text.is_empty() {
        return None;
    }
    let pad_x = 5;
    let pad_y = 2;
    let w = font.width(text) + pad_x * 2;
    let h = font.line_height.max(1) + pad_y * 2;
    let x = (anchor.x + (anchor.width - w) / 2).clamp(0, (size[0] - w).max(0));
    let y = (anchor.y - h - 2).clamp(0, (size[1] - h).max(0));
    Some(Rect::new(x, y, w, h))
}

/// Draws a Quick Tip tooltip box above an Options button (`0x517a6f`).
pub fn draw_tooltip(frame: &mut Frame, art: &Art, anchor: Rect, text: &str) {
    let fw = i32::try_from(frame.width()).unwrap_or(i32::MAX);
    let fh = i32::try_from(frame.height()).unwrap_or(i32::MAX);
    let Some(r) = tooltip_rect([fw, fh], art, anchor, text) else {
        return;
    };
    let Some(font) = art.font(LABEL_FONT) else {
        return;
    };
    let pad_x = 5;
    let pad_y = 2;
    frame.fill(r, LABEL);
    frame.fill(
        Rect::new(
            r.x + 1,
            r.y + 1,
            (r.width - 2).max(0),
            (r.height - 2).max(0),
        ),
        TOOLTIP_BACK,
    );
    font.draw(frame, text, [r.x + pad_x, r.y + pad_y], LABEL);
}

/// Computes the x coordinate of a horizontal `cTSWinSlider`'s thumb
/// (`0x5f640e..0x5f64fe`, with margin `[esi + 0xf0] == 0` from `0x5f5dc7`,
/// thumb width `11`, and range `0..=10` from `0x51539c..0x5153ac`).
pub fn slider_thumb_x(rect: Rect, value: u8) -> i32 {
    let travel = (rect.width - SLIDER_CELL_WIDTH).max(0);
    let step = i32::from(value.min(VOLUME_MAX));
    // 0x5f64c6..0x5f64dd: trunc((step * travel) / 10) + start.
    rect.x
        .saturating_add((step * travel) / i32::from(VOLUME_MAX))
}

/// Maps a canvas x coordinate `point_x` on a horizontal `cTSWinSlider` to its
/// discrete volume step `0..=10` (`0x5f633d..0x5f640b`, clamped to `0..=10` by
/// `SetValue` at `0x5f5e50`).
pub fn slider_value_at(rect: Rect, point_x: i32) -> u8 {
    let travel = rect.width - SLIDER_CELL_WIDTH;
    if travel <= 0 {
        return 0;
    }
    let local = point_x.saturating_sub(rect.x);
    // 0x5f6370..0x5f63a4: center on thumb (`local - thumb_w / 2`), clamp to [0, width].
    let clamped = (local - SLIDER_CELL_WIDTH / 2).clamp(0, rect.width);
    // 0x5f63e7..0x5f6408: trunc(clamped * (max - min + 1) / travel) + min.
    let raw = (i64::from(clamped) * i64::from(VOLUME_MAX + 1)) / i64::from(travel);
    raw.clamp(0, i64::from(VOLUME_MAX)) as u8
}

/// Draws a 6-cell toggle (`optcheckbox.bmp` 4953 or `optradio.bmp` 4954)
/// following `cTSWinBtn::GetCell` (`0x5e3ba1..0x5e3c3e`):
/// - enabled: 0 unchecked normal, 1 checked normal, 2 unchecked hover, 3 checked hover;
/// - disabled: 4 unchecked (`0x5e3ba1: push 4; pop ebx` -> `0x5e3ba8: cmp edx, 6; je 0x5e3be6`
///   -> `0x5e3be6: test edi, edi; je 0x5e3c13` -> `0x5e3c15: jne 0x5e3c23` ->
///   `0x5e3c25: je 0x5e3b96` -> `mov [ecx + 0xe8], ebx` storing `4`), 5 checked (`0x5e3c2b`).
pub fn draw_toggle(
    frame: &mut Frame,
    art: &Art,
    rect: Rect,
    id: i32,
    checked: bool,
    enabled: bool,
    hover: bool,
) {
    let cell = if !enabled {
        if checked { 5 } else { 4 }
    } else if checked {
        if hover { 3 } else { 1 }
    } else if hover {
        2
    } else {
        0
    };
    if let Some(image) = art.cell(id, 6, cell) {
        frame.draw_all(&image, [rect.x, rect.y]);
    }
}

/// Draws a horizontal `cTSWinSlider` (`0x5f6091..0x5f6232`):
/// selects `kOptSliderOn` (`4955`) when `value > 0` and `kOptSliderOff`
/// (`4956`) when `value == 0` (`0x516685..0x5166ab`), then draws cell 0 (left
/// cap), tiles cell 1 across the middle track, draws cell 2 (right cap), and
/// draws cell 3 (thumb) at [`slider_thumb_x`].
pub fn draw_slider(frame: &mut Frame, art: &Art, rect: Rect, value: u8) {
    let id = if value == 0 {
        id::SLIDER_OFF
    } else {
        id::SLIDER_ON
    };
    let Some(sheet) = art.image(id) else {
        return;
    };
    let cell_w = i32::try_from(sheet.width / 4).unwrap_or(0);
    let cell_h = i32::try_from(sheet.height).unwrap_or(0);
    if cell_w <= 0 || cell_h <= 0 || rect.width <= 0 {
        return;
    }
    let left_w = cell_w.min(rect.width);
    frame.draw(sheet, Rect::new(0, 0, left_w, cell_h), [rect.x, rect.y]);

    let mid_start = rect.x.saturating_add(cell_w);
    let right_x = rect.x.saturating_add(rect.width).saturating_sub(cell_w);
    let mut x = mid_start;
    while x < right_x {
        let w = cell_w.min(right_x - x);
        frame.draw(sheet, Rect::new(cell_w, 0, w, cell_h), [x, rect.y]);
        x = x.saturating_add(cell_w);
    }
    if right_x >= rect.x {
        let right_w = (rect.x.saturating_add(rect.width) - right_x).min(cell_w);
        frame.draw(
            sheet,
            Rect::new(cell_w * 2, 0, right_w, cell_h),
            [right_x, rect.y],
        );
    }
    let thumb_x = slider_thumb_x(rect, value);
    frame.draw(
        sheet,
        Rect::new(cell_w * 3, 0, cell_w, cell_h),
        [thumb_x, rect.y],
    );
}

/// Lays out the Options panel strip (`cWinOptions`, `0x51409b..0x515ef6`) inside
/// `strip` (`(220, height - 100, width - 220, 100)`), using default English
/// coordinates and labels. Use [`layout_strip_with`] when `OptionsTexts` was
/// loaded from `GameData/UIText.iff`.
pub fn layout_strip(items: &mut Vec<Item>, strip: Rect, residential: bool, state: &OptionsState) {
    layout_strip_with(
        items,
        strip,
        residential,
        state,
        &OptionsTexts::default(),
        None,
    );
}

/// Lays out the Options panel strip with loaded [`OptionsTexts`] and optional
/// [`Art`] (for exact 9 pt / 8 pt font measurements matching `0x514636`,
/// `0x514a8a`, `0x5152d7`, `0x515b38`).
pub fn layout_strip_with(
    items: &mut Vec<Item>,
    strip: Rect,
    residential: bool,
    state: &OptionsState,
    texts: &OptionsTexts,
    art: Option<&Art>,
) {
    layout_strip_for_lot(
        items,
        strip,
        LotVisibility::from_residential(residential),
        state,
        texts,
        art,
        None,
    );
}

/// Lays out the Options panel strip with exact per-lot [`LotVisibility`]
/// (`0x4f2490..0x4f24e7`, `0x5177aa..0x517833`), clickable setting help labels
/// (`0x516719..0x51679a`), open Options help balloon, and Quick Tips hover
/// tooltips (`0x5164ef..0x5164ff -> 0x517a6f`).
pub fn layout_strip_for_lot(
    items: &mut Vec<Item>,
    strip: Rect,
    visibility: LotVisibility,
    state: &OptionsState,
    texts: &OptionsTexts,
    art: Option<&Art>,
    pointer: Option<[i32; 2]>,
) {
    let at = |x: i32, y: i32, w: i32, h: i32| Rect::new(strip.x + x, strip.y + y, w, h);
    let label_font = art.and_then(|a| a.font(LABEL_FONT));
    let header_font = art.and_then(|a| a.font(HEADER_FONT));
    let measure = |text: &str| {
        label_font.map_or_else(
            || i32::try_from(text.len()).unwrap_or(0) * 6,
            |font| font.width(text),
        )
    };
    let fit_label = |text: &str, max_w: i32| -> String {
        if max_w <= 0 {
            return String::new();
        }
        if measure(text) <= max_w {
            return text.to_owned();
        }
        let fit_prefix = |budget: i32| -> &str {
            let mut end = 0usize;
            for (idx, ch) in text.char_indices() {
                let next = idx + ch.len_utf8();
                if measure(&text[..next]) <= budget {
                    end = next;
                } else {
                    break;
                }
            }
            text[..end].trim_end()
        };
        let ellipsis = "...";
        let ell_w = measure(ellipsis);
        if ell_w < max_w {
            let prefix = fit_prefix(max_w - ell_w);
            if !prefix.is_empty() {
                return format!("{prefix}{ellipsis}");
            }
        }
        fit_prefix(max_w).to_owned()
    };
    let label_h = label_font.map_or(14, |f| f.line_height.max(1));
    let header_h = header_font.map_or(12, |f| f.line_height.max(1));

    // Compact left action/tab column on very narrow strips (< 360 px, e.g.
    // 500x480) so the subpanel still has room for disjoint controls and all
    // setting labels, while keeping native 1.91 action/tab coordinates at >= 600 px wide.
    let compact_left = strip.width < 360;
    let (horiz_x, horiz_w, vert_x, nbhd_pos, save_pos, quit_pos, gfx_pos, snd_pos, play_pos) =
        if compact_left {
            (
                2,
                142,
                146,
                [2, texts.nbhd_pos[1]],
                [52, texts.save_pos[1]],
                [92, texts.quit_pos[1]],
                [2, texts.graphics_pos[1]],
                [41, texts.sound_pos[1]],
                [89, texts.play_pos[1]],
            )
        } else {
            (
                HORIZ_DIVIDER_POS[0],
                201,
                VERT_DIVIDER_POS[0],
                texts.nbhd_pos,
                texts.save_pos,
                texts.quit_pos,
                texts.graphics_pos,
                texts.sound_pos,
                texts.play_pos,
            )
        };
    let subpanel_left = vert_x + 4;

    // Dividers (cWinOptions::OnDraw, 0x516153..0x5161fa). `Kind::Stretched`
    // clips the 201 px horizontal divider cleanly to `horiz_w` on very narrow strips.
    items.push(Item {
        rect: at(horiz_x, HORIZ_DIVIDER_POS[1], horiz_w, 2),
        kind: Kind::Stretched(id::TOOTHPICK_HORIZ),
        command: None,
    });
    items.push(Item {
        rect: at(vert_x, VERT_DIVIDER_POS[1], 2, 89),
        kind: Kind::Image(id::TOOTHPICK_VERT),
        command: None,
    });

    // Track hovered action/tab/Reset Tutorial button for Quick Tips (`0x517a6f`).
    let mut hovered_tip: Option<(Rect, &str)> = None;
    let mut check_tip = |rect: Rect, tip_idx: usize| {
        if state.play.quick_tips
            && pointer.is_some_and(|[px, py]| rect.contains(px, py))
            && !texts.action_tooltips[tip_idx].is_empty()
        {
            hovered_tip = Some((rect, texts.action_tooltips[tip_idx].as_str()));
        }
    };

    // Top row: Neighborhood, Save (`!no_save`, `0x5174ac..0x5174c5`), Quit.
    let nbhd_rect = at(nbhd_pos[0], nbhd_pos[1], 49, 37);
    check_tip(nbhd_rect, 0);
    items.push(Item {
        rect: nbhd_rect,
        kind: Kind::Button {
            id: id::NEIGHBORHOOD,
            cells: 4,
            selected: false,
        },
        command: Some(Command::OptionsAction(OptionsAction::Neighborhood)),
    });
    if visibility.can_save {
        let save_rect = at(save_pos[0], save_pos[1], 39, 39);
        check_tip(save_rect, 1);
        items.push(Item {
            rect: save_rect,
            kind: Kind::Button {
                id: id::SAVE,
                cells: 4,
                selected: false,
            },
            command: Some(Command::OptionsAction(OptionsAction::Save)),
        });
    }
    let quit_rect = at(quit_pos[0], quit_pos[1], 50, 40);
    check_tip(quit_rect, 2);
    items.push(Item {
        rect: quit_rect,
        kind: Kind::Button {
            id: id::QUIT,
            cells: 4,
            selected: false,
        },
        command: Some(Command::OptionsAction(OptionsAction::Quit)),
    });

    // Bottom row: Graphics, Sound, Play tab buttons (0x514236..0x514374).
    for (tip_idx, (bmp, pos, size, tab)) in [
        (id::GRAPHICS, gfx_pos, [38, 40], OptionsTab::Graphics),
        (id::SOUND, snd_pos, [47, 40], OptionsTab::Sound),
        (id::PLAY, play_pos, [55, 40], OptionsTab::Play),
    ]
    .into_iter()
    .enumerate()
    {
        let rect = at(pos[0], pos[1], size[0], size[1]);
        check_tip(rect, 3 + tip_idx);
        items.push(Item {
            rect,
            kind: Kind::Button {
                id: bmp,
                cells: 4,
                selected: state.tab == Some(tab),
            },
            command: Some(Command::OpenOptionsTab(tab)),
        });
    }

    let push_label = |items: &mut Vec<Item>,
                      rect: Rect,
                      text: &str,
                      size: u8,
                      align: Align,
                      help: Option<OptionsHelp>| {
        items.push(Item {
            rect,
            kind: Kind::Text {
                text: text.to_owned(),
                size,
                rgb: LABEL,
                align,
            },
            command: help.map(Command::OptionsHelp),
        });
    };

    match state.tab {
        None => {}
        Some(OptionsTab::Graphics) => {
            // Left column: 4 rows laid out at (275, 9) step (200, 20) and checkboxes at
            // (245, 7) step (200, 20); row 0 (Anti-alias) is hidden after layout
            // (0x517485..0x51749b), so visible rows occupy indices 1, 2, 3.
            // On narrow windows (< 800 px wide), split the subpanel into
            // disjoint left/right bands and fit label strings to their column
            // width so no hit rects or rendered glyphs overlap, while preserving
            // exact 1.91 coordinates at >= 800 px.
            let right_limit = if compact_left {
                strip.width - 2
            } else {
                strip.width - 6
            };
            let native_fits = strip.width >= 580
                && texts.detail_radio_origin[0] + texts.detail_radio_size[0] + 5 <= strip.width;
            let (gfx_cb_x, gfx_label_x, radio_x, radio_w, left_label_max, right_label_max) =
                if native_fits {
                    (
                        texts.graphics_checkbox_origin[0],
                        texts.graphics_label_origin[0],
                        texts.detail_radio_origin[0],
                        texts.detail_radio_size[0],
                        i32::MAX,
                        i32::MAX,
                    )
                } else {
                    let compact_w = RADIO_CELL_SIZE[0] * 3;
                    let compact_radio_x =
                        (right_limit - compact_w).max(subpanel_left + CHECKBOX_SIZE[0] + 4);
                    let l_label_x = subpanel_left + CHECKBOX_SIZE[0] + 2;
                    let avail_labels = (compact_radio_x - 2 - l_label_x).max(0);
                    let l_max = (avail_labels - 2).max(0) / 2;
                    let r_max = (avail_labels - 2 - l_max).max(0);
                    (
                        subpanel_left,
                        l_label_x,
                        compact_radio_x,
                        compact_w,
                        l_max,
                        r_max,
                    )
                };
            let gfx_cb_step_y = if native_fits {
                texts.graphics_checkbox_step[1]
            } else {
                texts.graphics_checkbox_step[1].max(CHECKBOX_SIZE[1])
            };
            let gfx_lb_step_y = if native_fits {
                texts.graphics_label_step[1]
            } else {
                texts.graphics_label_step[1].max(CHECKBOX_SIZE[1])
            };
            let toggles = [
                (1, OptionsToggle::Shadows, OptionsHelp::Shadows),
                (2, OptionsToggle::Lighting, OptionsHelp::Lighting),
                (
                    3,
                    OptionsToggle::InterfaceEffects,
                    OptionsHelp::InterfaceEffects,
                ),
            ];
            for (row, toggle, help) in toggles {
                let cb_y = texts.graphics_checkbox_origin[1] + gfx_cb_step_y * row as i32;
                let lb_y = texts.graphics_label_origin[1] + gfx_lb_step_y * row as i32;
                items.push(Item {
                    rect: at(gfx_cb_x, cb_y, CHECKBOX_SIZE[0], CHECKBOX_SIZE[1]),
                    kind: Kind::Toggle {
                        id: id::CHECKBOX,
                        checked: state.is_checked(toggle),
                    },
                    command: Some(Command::ToggleOption(toggle)),
                });
                let fitted = if native_fits {
                    texts.graphics_labels[row].clone()
                } else {
                    fit_label(&texts.graphics_labels[row], left_label_max)
                };
                if !fitted.is_empty() {
                    let w = measure(&fitted);
                    push_label(
                        items,
                        at(gfx_label_x, lb_y, w, label_h),
                        &fitted,
                        LABEL_FONT,
                        Align::Left,
                        Some(help),
                    );
                }
            }

            // Right column labels: right-aligned to `detail_label_origin[0]` (480, 0x514a77..0x514af0).
            let max_detail_w = texts
                .detail_labels
                .iter()
                .map(|s| measure(s))
                .max()
                .unwrap_or(0);
            for (row, (text, help)) in texts
                .detail_labels
                .iter()
                .zip([OptionsHelp::TerrainDetail, OptionsHelp::CharacterDetail])
                .enumerate()
            {
                let y = texts.detail_label_origin[1] + texts.detail_label_step[1] * row as i32;
                if native_fits {
                    let detail_label_x = texts.detail_label_origin[0] - max_detail_w;
                    push_label(
                        items,
                        at(detail_label_x, y, max_detail_w, label_h),
                        text,
                        LABEL_FONT,
                        Align::Right,
                        Some(help),
                    );
                } else {
                    let fitted = fit_label(text, right_label_max);
                    if !fitted.is_empty() {
                        let w = measure(&fitted);
                        let x = (radio_x - 2 - w).max(gfx_label_x + left_label_max + 2);
                        push_label(
                            items,
                            at(x, y, w, label_h),
                            &fitted,
                            LABEL_FONT,
                            Align::Right,
                            Some(help),
                        );
                    }
                }
            }

            // Column headers "Low", "Med", "High" above the radio groups (`0x514efa..0x514f0a`:
            // span `[radio_x - 8, radio_x + radio_w + 5)`, divided into exact thirds at
            // `0x537b3c..0x537b96`).
            let radio_y = texts.detail_radio_origin[1];
            let header_y = radio_y - 7 - header_h;
            let cell_offsets = [
                0,
                (radio_w - RADIO_CELL_SIZE[0]) / 2,
                radio_w - RADIO_CELL_SIZE[0],
            ];
            let max_hdr_w = texts
                .detail_headers
                .iter()
                .map(|h| header_font.map_or(24, |f| f.width(h)))
                .max()
                .unwrap_or(24);
            let min_hdr_total = max_hdr_w * 3;
            let hdr_right = (radio_x + radio_w + 5).min(strip.width);
            let hdr_left = (radio_x - 8).min(hdr_right - min_hdr_total).max(0);
            let hdr_total = (hdr_right - hdr_left).max(0);
            let third = hdr_total / 3;
            for (col, hdr) in texts.detail_headers.iter().enumerate() {
                let align = match col {
                    0 => Align::Left,
                    1 => Align::Center,
                    _ => Align::Right,
                };
                let (hx, hw) = match col {
                    0 => (hdr_left, third),
                    1 => (hdr_left + third, third),
                    _ => (hdr_left + third * 2, hdr_total - third * 2),
                };
                push_label(
                    items,
                    at(hx, header_y, hw, header_h),
                    hdr,
                    HEADER_FONT,
                    align,
                    None,
                );
            }

            // 3-way radio groups for Terrain Detail and Character Detail (0x53774d..0x5377ac).
            for (row, radio) in [OptionsRadio::TerrainDetail, OptionsRadio::CharacterDetail]
                .into_iter()
                .enumerate()
            {
                let y = radio_y + texts.detail_radio_step[1] * row as i32;
                let current = state.detail(radio);
                for (level, ox) in [
                    (DetailLevel::Low, cell_offsets[0]),
                    (DetailLevel::Med, cell_offsets[1]),
                    (DetailLevel::High, cell_offsets[2]),
                ] {
                    items.push(Item {
                        rect: at(radio_x + ox, y, RADIO_CELL_SIZE[0], RADIO_CELL_SIZE[1]),
                        kind: Kind::Toggle {
                            id: id::RADIO,
                            checked: current == level,
                        },
                        command: Some(Command::SetDetail { radio, level }),
                    });
                }
            }
        }
        Some(OptionsTab::Sound) => {
            // 3 volume labels (F/X, Music, Vox) + 3 horizontal 0..=10 sliders
            // (0x514fa3..0x515416). On narrow windows (< 800 px wide), compact
            // the label margin and slider width so all 3 sliders fit inside `strip`.
            let max_label_w = texts
                .sound_labels
                .iter()
                .map(|s| measure(s))
                .max()
                .unwrap_or(0);
            let native_slider_x = texts.sound_origin[0] + max_label_w + 15;
            let right_limit = strip.width - 4;
            let (sound_label_x, slider_x, slider_w) =
                if strip.width >= 580 && native_slider_x + texts.slider_width <= right_limit {
                    (texts.sound_origin[0], native_slider_x, texts.slider_width)
                } else {
                    let lx = subpanel_left.min(texts.sound_origin[0]);
                    let sx = (lx + max_label_w + 6).min((right_limit - SLIDER_CELL_WIDTH).max(lx));
                    let sw = (right_limit - sx).clamp(SLIDER_CELL_WIDTH, texts.slider_width);
                    (lx, sx, sw)
                };
            let slider_y_offset = (label_h - SLIDER_HEIGHT) / 2;
            for (row, ((label, channel), help)) in texts
                .sound_labels
                .iter()
                .zip([VolumeChannel::Fx, VolumeChannel::Music, VolumeChannel::Vox])
                .zip([OptionsHelp::Fx, OptionsHelp::Music, OptionsHelp::Vox])
                .enumerate()
            {
                let y = texts.sound_origin[1] + texts.sound_step[1] * row as i32;
                let w = measure(label);
                push_label(
                    items,
                    at(sound_label_x, y, w, label_h),
                    label,
                    LABEL_FONT,
                    Align::Left,
                    Some(help),
                );
                let step = state.sound.get(channel);
                items.push(Item {
                    rect: at(slider_x, y + slider_y_offset, slider_w, SLIDER_HEIGHT),
                    kind: Kind::Slider {
                        channel,
                        value: step,
                    },
                    command: Some(Command::SetVolume { channel, step }),
                });
            }
        }
        Some(OptionsTab::Play) => {
            // Left column: Auto-Centering, Free Will (hidden on Downtown/Vacation/
            // Magic Town at `0x5177aa..0x517822`), Edge Scrolling, Sim In Background
            // (0x51542e..0x515935).
            let cb_y_offset = (label_h - CHECKBOX_SIZE[1]) / 2;
            let native_left_cb_x = texts.play_origin[0] - PLAY_CHECKBOX_OFFSET_X;
            let max_left_w = texts
                .play_left_labels
                .iter()
                .map(|s| measure(s))
                .max()
                .unwrap_or(0);
            let max_right_w = texts
                .play_right_labels
                .iter()
                .map(|s| measure(s))
                .max()
                .unwrap_or(0);
            let play_right_limit = if visibility.show_reset_tutorial {
                strip.width - 28
            } else {
                strip.width - 4
            };
            let native_right_label_x = native_left_cb_x + max_left_w + 70;
            let native_fits =
                strip.width >= 580 && native_right_label_x + max_right_w <= play_right_limit;
            let (
                left_cb_x,
                left_label_x,
                left_label_max,
                right_cb_x,
                right_label_x,
                right_label_max,
            ) = if native_fits {
                (
                    native_left_cb_x,
                    texts.play_origin[0],
                    i32::MAX,
                    native_right_label_x - PLAY_CHECKBOX_OFFSET_X,
                    native_right_label_x,
                    i32::MAX,
                )
            } else {
                let col_w = ((play_right_limit - subpanel_left - 4) / 2).max(CHECKBOX_SIZE[0]);
                let lcb = subpanel_left;
                let llx = lcb + CHECKBOX_SIZE[0] + 3;
                let l_max = (subpanel_left + col_w - llx).max(0);
                let rcb = subpanel_left + col_w + 4;
                let rlx = rcb + CHECKBOX_SIZE[0] + 3;
                let r_max = (play_right_limit - rlx).max(0);
                (lcb, llx, l_max, rcb, rlx, r_max)
            };
            let play_step_y = if native_fits {
                texts.play_step[1]
            } else {
                texts.play_step[1].max(CHECKBOX_SIZE[1])
            };
            for (row, ((label, toggle), help)) in texts
                .play_left_labels
                .iter()
                .zip([
                    OptionsToggle::AutoCentering,
                    OptionsToggle::FreeWill,
                    OptionsToggle::EdgeScrolling,
                    OptionsToggle::SimInBackground,
                ])
                .zip([
                    OptionsHelp::AutoCentering,
                    OptionsHelp::FreeWill,
                    OptionsHelp::EdgeScrolling,
                    OptionsHelp::SimInBackground,
                ])
                .enumerate()
            {
                if toggle == OptionsToggle::FreeWill && !visibility.show_free_will {
                    continue;
                }
                let y = texts.play_origin[1] + play_step_y * row as i32;
                items.push(Item {
                    rect: at(
                        left_cb_x,
                        y + cb_y_offset,
                        CHECKBOX_SIZE[0],
                        CHECKBOX_SIZE[1],
                    ),
                    kind: Kind::Toggle {
                        id: id::CHECKBOX,
                        checked: state.is_checked(toggle),
                    },
                    command: Some(Command::ToggleOption(toggle)),
                });
                let fitted = if native_fits {
                    label.clone()
                } else {
                    fit_label(label, left_label_max)
                };
                if !fitted.is_empty() {
                    push_label(
                        items,
                        at(left_label_x, y, measure(&fitted), label_h),
                        &fitted,
                        LABEL_FONT,
                        Align::Left,
                        Some(help),
                    );
                }
            }

            // Right column: Quick Tips, Auto Snapshot, Live PIP, Export HTML.
            // Native `0x51584c..0x515852` mutates the running Y by `cb_y_offset`,
            // then `0x515b44..0x515b4a` adds +4 in English (`[ebp - 0x25] == 0`),
            // placing right-column labels at `play_origin[1] + cb_y_offset + 4`
            // (2 px below the left column with the installed 17 px 9 pt font)
            // and checkboxes at `right_label_y + cb_y_offset` (`0x515e14..0x515e17`).
            let right_label_y0 = texts.play_origin[1] + cb_y_offset + 4;
            for (row, ((label, toggle), help)) in texts
                .play_right_labels
                .iter()
                .zip([
                    OptionsToggle::QuickTips,
                    OptionsToggle::AutoSnapshot,
                    OptionsToggle::LivePip,
                    OptionsToggle::ExportHtml,
                ])
                .zip([
                    OptionsHelp::QuickTips,
                    OptionsHelp::AutoSnapshot,
                    OptionsHelp::LivePip,
                    OptionsHelp::ExportHtml,
                ])
                .enumerate()
            {
                let y = right_label_y0 + play_step_y * row as i32;
                items.push(Item {
                    rect: at(
                        right_cb_x,
                        y + cb_y_offset,
                        CHECKBOX_SIZE[0],
                        CHECKBOX_SIZE[1],
                    ),
                    kind: Kind::Toggle {
                        id: id::CHECKBOX,
                        checked: state.is_checked(toggle),
                    },
                    command: Some(Command::ToggleOption(toggle)),
                });
                let fitted = if native_fits {
                    label.clone()
                } else {
                    fit_label(label, right_label_max)
                };
                if !fitted.is_empty() {
                    push_label(
                        items,
                        at(right_label_x, y, measure(&fitted), label_h),
                        &fitted,
                        LABEL_FONT,
                        Align::Left,
                        Some(help),
                    );
                }
            }

            // Reset Tutorial button at top-right of strip (`strip.width - 22 - 3`, y = 7,
            // `0x515a33..0x515a52`), shown only on residential lots (`0x5175d5..0x5175f4`).
            if visibility.show_reset_tutorial {
                let reset_x = (strip.width - 22 - 3).max(0);
                let reset_rect = at(reset_x, 7, 22, 30);
                check_tip(reset_rect, 6);
                items.push(Item {
                    rect: reset_rect,
                    kind: Kind::Button {
                        id: id::TUTORIAL_RESET,
                        cells: 4,
                        selected: false,
                    },
                    command: Some(Command::OptionsAction(OptionsAction::TutorialReset)),
                });
            }
        }
    }

    if let Some(help) = state.help {
        let balloon = layout_balloon(strip, help, texts, art);
        items.push(Item {
            rect: balloon.rect,
            kind: Kind::OptionsBalloon(balloon),
            command: None,
        });
    }
    if let Some((anchor, tip)) = hovered_tip {
        items.push(Item {
            rect: anchor,
            kind: Kind::OptionsTooltip(tip.to_owned()),
            command: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sims_formats::bitmap::Image;

    #[test]
    fn parses_native_coordinate_pairs() {
        assert_eq!(parse_pair("(88;6)"), Some([88, 6]));
        assert_eq!(parse_pair(" ( 275 ; 9 ) "), Some([275, 9]));
        assert_eq!(parse_pair("(130;0)"), Some([130, 0]));
        assert_eq!(parse_pair("bad"), None);
    }

    #[test]
    fn slider_thumb_and_hit_math_match_original_range() {
        let rect = Rect::new(350, 513, 130, 14);
        assert_eq!(slider_thumb_x(rect, 0), 350);
        assert_eq!(slider_thumb_x(rect, 5), 350 + 59);
        assert_eq!(slider_thumb_x(rect, 10), 350 + 119);
        assert_eq!(slider_thumb_x(rect, 99), 350 + 119);

        assert_eq!(slider_value_at(rect, 350), 0);
        assert_eq!(slider_value_at(rect, 300), 0);
        assert_eq!(slider_value_at(rect, 350 + 65), 5);
        assert_eq!(slider_value_at(rect, 350 + 129), 10);
        assert_eq!(slider_value_at(rect, 500), 10);
    }

    #[test]
    fn options_state_toggles_tabs_checkboxes_radios_and_volumes() {
        let mut state = OptionsState::default();
        assert_eq!(state.tab, None);
        assert!(state.apply(&Command::OpenOptionsTab(OptionsTab::Sound)));
        assert_eq!(state.tab, Some(OptionsTab::Sound));
        // Clicking the active tab closes the subpanel (`0x5167d5..0x5167fb`).
        assert!(state.apply(&Command::OpenOptionsTab(OptionsTab::Sound)));
        assert_eq!(state.tab, None);

        assert!(state.graphics.shadows);
        assert!(state.apply(&Command::ToggleOption(OptionsToggle::Shadows)));
        assert!(!state.graphics.shadows);

        assert_eq!(state.graphics.terrain_detail, DetailLevel::High);
        assert!(state.apply(&Command::SetDetail {
            radio: OptionsRadio::TerrainDetail,
            level: DetailLevel::Low,
        }));
        assert_eq!(state.graphics.terrain_detail, DetailLevel::Low);
        assert!(!state.apply(&Command::SetDetail {
            radio: OptionsRadio::TerrainDetail,
            level: DetailLevel::Low,
        }));

        assert!(state.apply(&Command::SetVolume {
            channel: VolumeChannel::Music,
            step: 4,
        }));
        assert_eq!(state.sound.music, 4);
        assert!(state.apply(&Command::SetVolume {
            channel: VolumeChannel::Fx,
            step: 3,
        }));
        assert_eq!(state.sound.fx, 3);
        assert!(state.apply(&Command::SetVolume {
            channel: VolumeChannel::Fx,
            step: 99,
        }));
        assert_eq!(state.sound.fx, 10);
        assert!(!state.apply(&Command::SetVolume {
            channel: VolumeChannel::Fx,
            step: 10,
        }));
    }

    #[test]
    fn six_cell_toggle_selects_native_cells() {
        let mut art = Art::default();
        // Six 1x1 cells colored [10..60].
        art.insert(
            id::CHECKBOX,
            Image {
                width: 6,
                height: 1,
                rgba: [
                    [10, 0, 0, 255],
                    [20, 0, 0, 255],
                    [30, 0, 0, 255],
                    [40, 0, 0, 255],
                    [50, 0, 0, 255],
                    [60, 0, 0, 255],
                ]
                .concat(),
            },
        );
        let sample = |checked, enabled, hover| {
            let mut frame = Frame::new(1, 1, [0; 3]);
            draw_toggle(
                &mut frame,
                &art,
                Rect::new(0, 0, 1, 1),
                id::CHECKBOX,
                checked,
                enabled,
                hover,
            );
            frame.pixel(0, 0).unwrap()[0]
        };
        assert_eq!(sample(false, true, false), 10); // cell 0
        assert_eq!(sample(true, true, false), 20); // cell 1
        assert_eq!(sample(false, true, true), 30); // cell 2
        assert_eq!(sample(true, true, true), 40); // cell 3
        assert_eq!(sample(false, false, false), 50); // cell 4
        assert_eq!(sample(true, false, false), 60); // cell 5
    }

    #[test]
    fn slider_draw_switches_between_on_and_off_bitmaps() {
        let make_strip = |tag: u8| Image {
            width: 44,
            height: 14,
            rgba: (0..44 * 14)
                .flat_map(|i| {
                    let cell = ((i % 44) / 11) as u8;
                    [tag, cell, 0, 255]
                })
                .collect(),
        };
        let mut art = Art::default();
        art.insert(id::SLIDER_ON, make_strip(100));
        art.insert(id::SLIDER_OFF, make_strip(200));

        let rect = Rect::new(0, 0, 130, 14);
        let mut off_frame = Frame::new(130, 14, [0; 3]);
        draw_slider(&mut off_frame, &art, rect, 0);
        // Step 0 uses SLIDER_OFF (tag 200); thumb (cell 3) sits at x=0..11.
        assert_eq!(off_frame.pixel(5, 7), Some([200, 3, 0]));
        assert_eq!(off_frame.pixel(50, 7), Some([200, 1, 0]));
        assert_eq!(off_frame.pixel(125, 7), Some([200, 2, 0]));

        let mut on_frame = Frame::new(130, 14, [0; 3]);
        draw_slider(&mut on_frame, &art, rect, 10);
        // Step 10 uses SLIDER_ON (tag 100); left cap (cell 0) at x=0..11,
        // thumb (cell 3) at x=119..130.
        assert_eq!(on_frame.pixel(5, 7), Some([100, 0, 0]));
        assert_eq!(on_frame.pixel(50, 7), Some([100, 1, 0]));
        assert_eq!(on_frame.pixel(125, 7), Some([100, 3, 0]));
    }

    #[test]
    fn layout_hides_save_and_tutorial_reset_on_community_lots() {
        let strip = Rect::new(220, 500, 580, 100);
        let state = OptionsState {
            tab: Some(OptionsTab::Play),
            ..OptionsState::default()
        };
        let mut res_items = Vec::new();
        layout_strip(&mut res_items, strip, true, &state);
        assert!(
            res_items
                .iter()
                .any(|i| i.command == Some(Command::OptionsAction(OptionsAction::Save)))
        );
        assert!(
            res_items
                .iter()
                .any(|i| i.command == Some(Command::OptionsAction(OptionsAction::TutorialReset)))
        );

        let mut comm_items = Vec::new();
        layout_strip(&mut comm_items, strip, false, &state);
        assert!(
            !comm_items
                .iter()
                .any(|i| i.command == Some(Command::OptionsAction(OptionsAction::Save)))
        );
        assert!(
            !comm_items
                .iter()
                .any(|i| i.command == Some(Command::OptionsAction(OptionsAction::TutorialReset)))
        );
    }
}
