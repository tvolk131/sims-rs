//! Complete Collection 1.91 (`Sims.exe`) neighborhood map screen renderer and
//! hit-tester.
//!
//! Renders the interactive map screens for all six neighborhood types that
//! share the native `cWinLot` (`0x54dd4d`, vtable `0x670204`) widget pipeline:
//!
//! - [`NeighborhoodKind::Unleashed`]: Complete Collection / Unleashed main
//!   neighborhood (`cWinNeighborhood`, `0x556d78..0x55724a`), lots from
//!   `LotZoning.iff` (`STR# 1`) up to the first entry with `id > 80`
//!   (`0x557125..0x557128`), placed by `LotLocations.iff` (`STR# 4`).
//! - [`NeighborhoodKind::Classic`]: Original 1.0 neighborhood
//!   (`cWinNeighborhoodVC`, `0x55eb78..0x55eda3`), lots `1..=10` (`STR# 1`).
//! - [`NeighborhoodKind::Downtown`]: Hot Date Downtown (`cWinDowntown`,
//!   `0x54a7d6..0x54a8e2`), lots `21..=30` in native back-to-front Z-order
//!   `0x54aca8` (`STR# 2`).
//! - [`NeighborhoodKind::Vacation`]: Vacation Island (`cWinVacation`,
//!   `0x5691a5..0x5692bd`), lots `40..=48` in native back-to-front Z-order
//!   `0x569680` (`STR# 3`), plus port (`5442`), trees (`5443`), and post-lot
//!   water overlays (`0x56baeb..0x56bbb7`).
//! - [`NeighborhoodKind::StudioTown`]: Superstar Studio Town
//!   (`cWinStudiotown`, `0x564ce0..0x564db2`), lots `81..=89` (`STR# 5`).
//! - [`NeighborhoodKind::MagicTown`]: Makin' Magic Magic Town
//!   (`cWinMagicland`, `0x551a8a..0x551b5c`), lots `90..=98` (`STR# 7`), with
//!   post-lot water waves at `(0, 62)` (`0x542560`, `0x554ff6`).
//!
//! All lot compositing, 0.5× Hermite thumbnail downsampling (`0x4ba778` ->
//! `0x4ba79c`), YIQ brighten + horizontal edge highlight (`0x54f28f` +
//! `0x49bbfe`), `800×600` / `1024×768` viewport framing (`0x556db7..0x556ddc`),
//! and color-key silhouette hit-testing (`0x5e4071` -> `0x5dd79a`) operate on
//! CPU surfaces and write directly into [`Canvas`].

use crate::canvas::{Canvas, quantize565};
use crate::{Error, Result};
use sims_content::Install;
use sims_content::ui_graphics::{US_ENGLISH, UiGraphics};
use sims_formats::bitmap::{self, Image as BitmapImage};
use sims_formats::iff::Iff;
use sims_formats::lot::Simulator;
use sims_formats::neighborhood::Family;
use sims_formats::strings::StringTable;
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// Native map backdrop width in pixels (`0x320`).
pub const MAP_WIDTH: u32 = 800;
/// Native map backdrop height in pixels (`0x258`).
pub const MAP_HEIGHT: u32 = 600;

/// Maximum pixel count accepted for a single [`Surface565`] or rendered
/// viewport canvas (`4096 × 4096`), matching `sims_formats::bitmap::MAX_PIXELS`.
pub const MAX_SURFACE_PIXELS: usize = 4096 * 4096;

/// Maximum width or height for a composed lot thumbnail cell, guarding against
/// hostile `THMB` offsets causing excessive memory retention.
const MAX_LOT_CELL_DIM: usize = 1024;

/// Native 16-bit 5-6-5 color key: magenta `RGB(248, 0, 248)` / `(255, 0, 255)`.
pub const COLOR_KEY_565: u16 = 0xf81f;

/// Native lot highlight RGB (`0x54e91c`, `0x54f1f6`: `push 0x1d; push 0xff; push 0xf2`).
pub const DEFAULT_HIGHLIGHT_RGB: [u8; 3] = [0xf2, 0xff, 0x1d];

/// Native blue community-filter highlight RGB (`0x54f520`: `push 0xff; push 0; push 0`).
pub const COMMUNITY_FILTER_HIGHLIGHT_RGB: [u8; 3] = [0x00, 0x00, 0xff];

/// Native YIQ luminance offset added to highlighted lot pixels (`[0x6703bc]`).
const HIGHLIGHT_Y_OFFSET: f32 = f32::from_bits(0x3db8_51ec); // 0.09f

/// Native color-key surviving-weight cutoff in the 0.5× resampler (`[0x668820]`).
const RESAMPLE_WEIGHT_CUTOFF: f32 = 0.46875;

/// Native Magic Town water wave vertical offset in map coordinates (`0x542560`).
const MAGICLAND_WAVE_OFFSET_Y: i32 = 62;

/// Native sentinel stored in `THMB` when a house lacks a `THMB` chunk
/// (`0x4e27da`, `0x4e2952`), instructing `0x54ea0c..0x54ea79` to center the
/// thumbnail on the lot's map anchor.
pub const THMB_CENTER_SENTINEL: i32 = 0x7fff_ffff;

/// `UIGraphics` resource IDs from `Res_Nbhd.h` and `Res_CPanel.h`.
pub mod res {
    pub const LARGE_MASK: i32 = 5001; // Downtown/LargeBack.bmp
    pub const LARGE_MASK_SS: i32 = 5002; // Studiotown/dlgframe_1024x768.bmp

    pub const NBHD_BCK_CLASSIC: i32 = 5000;
    pub const NBHD_BCK_CLASSIC_NON_US: i32 = 5308;
    pub const NBHD_WAVES_CLASSIC: [i32; 3] = [5302, 5303, 5304];
    pub const NBHD_WAVES_CLASSIC_NON_US: [i32; 3] = [5309, 5310, 5311];

    pub const NBHD_BCK_UNLEASHED: i32 = 6000;
    pub const NBHD_BCK_UNLEASHED_NON_US: i32 = 6308;
    pub const NBHD_WAVES_UNLEASHED: [i32; 6] = [6302, 6303, 6304, 6330, 6331, 6332];
    pub const NBHD_WAVES_UNLEASHED_NON_US: [i32; 6] = [6309, 6310, 6311, 6336, 6337, 6338];

    pub const DOWNTOWN_BCK: i32 = 5320;
    pub const DOWNTOWN_WAVES: [i32; 6] = [5400, 5401, 5402, 5403, 5404, 5405];

    pub const VACATION_BCK: i32 = 5440;
    pub const VACATION_WAVES: [i32; 5] = [5410, 5411, 5412, 5413, 5414];
    pub const VACATION_LOGO: i32 = 5441;
    pub const VACATION_PORT: i32 = 5442;
    pub const VACATION_TREES: i32 = 5443;
    pub const VACATION_NO_VACANCY: i32 = 5510;

    pub const STUDIOTOWN_BCK: i32 = 5321;

    pub const MAGICLAND_BCK: i32 = 5322;
    pub const MAGICLAND_WAVES: [i32; 6] = [5720, 5721, 5722, 5723, 5724, 5725];

    // Community / sub-neighborhood filter 3-frame map badges (0x555216..0x555320).
    pub const FILTER_LODGING: i32 = 4930;
    pub const FILTER_FOOD: i32 = 4931;
    pub const FILTER_GARDENING: i32 = 4932;
    pub const FILTER_SHOPPING: i32 = 4933;
    pub const FILTER_PET: i32 = 4934;
    pub const FILTER_SMALL_ANIMAL: i32 = 4935;
    pub const FILTER_PARK: i32 = 4936;
    pub const FILTER_SPA: i32 = 4937;
    pub const FILTER_MUSIC: i32 = 4938;
    pub const FILTER_MOVIE: i32 = 4939;
    pub const FILTER_MODEL: i32 = 4940;
    pub const FILTER_RIDES: i32 = 4941;
    pub const FILTER_MAGIC: i32 = 4942;
}

/// Which neighborhood or expansion sub-neighborhood map screen to render.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NeighborhoodKind {
    /// Complete Collection / Unleashed main map (`cWinNeighborhood`, lots `1..=10, 50..=80`).
    Unleashed,
    /// Original 1.0 main map (`cWinNeighborhoodVC`, lots `1..=10`).
    Classic,
    /// Hot Date Downtown (`cWinDowntown`, lots `21..=30`).
    Downtown,
    /// Vacation Island (`cWinVacation`, lots `40..=48`).
    Vacation,
    /// Superstar Studio Town (`cWinStudiotown`, lots `81..=89`).
    StudioTown,
    /// Makin' Magic Magic Town (`cWinMagicland`, lots `90..=98`).
    MagicTown,
}

impl NeighborhoodKind {
    /// Whether this map uses half-scale `THMB 1024` / `BMP_ 1024..=1025`
    /// thumbnails (falling back to 0.5× downsampling of `512..=513`).
    pub fn uses_half_scale(self) -> bool {
        matches!(self, Self::Unleashed)
    }

    /// `STR#` chunk ID in `LotLocations.iff` for this map.
    pub fn locations_str_id(self) -> u16 {
        match self {
            Self::Classic => 1,
            Self::Downtown => 2,
            Self::Vacation => 3,
            Self::Unleashed => 4,
            Self::StudioTown => 5,
            Self::MagicTown => 7,
        }
    }

    /// Backdrop `UIGraphics` resource ID for US or non-US locales.
    pub fn backdrop_id(self, non_us: bool) -> i32 {
        match (self, non_us) {
            (Self::Unleashed, false) => res::NBHD_BCK_UNLEASHED,
            (Self::Unleashed, true) => res::NBHD_BCK_UNLEASHED_NON_US,
            (Self::Classic, false) => res::NBHD_BCK_CLASSIC,
            (Self::Classic, true) => res::NBHD_BCK_CLASSIC_NON_US,
            (Self::Downtown, _) => res::DOWNTOWN_BCK,
            (Self::Vacation, _) => res::VACATION_BCK,
            (Self::StudioTown, _) => res::STUDIOTOWN_BCK,
            (Self::MagicTown, _) => res::MAGICLAND_BCK,
        }
    }

    /// Water wave animation `UIGraphics` resource IDs for this map.
    pub fn wave_ids(self, non_us: bool) -> &'static [i32] {
        match (self, non_us) {
            (Self::Unleashed, false) => &res::NBHD_WAVES_UNLEASHED,
            (Self::Unleashed, true) => &res::NBHD_WAVES_UNLEASHED_NON_US,
            (Self::Classic, false) => &res::NBHD_WAVES_CLASSIC,
            (Self::Classic, true) => &res::NBHD_WAVES_CLASSIC_NON_US,
            (Self::Downtown, _) => &res::DOWNTOWN_WAVES,
            (Self::Vacation, _) => &res::VACATION_WAVES,
            (Self::StudioTown, _) => &[],
            (Self::MagicTown, _) => &res::MAGICLAND_WAVES,
        }
    }

    /// Whether water waves draw in the post-child pass (after lots), as in
    /// Downtown (`0x54cb21`), Vacation (`0x56bb7a`), and Magic Town (`0x554ff6`),
    /// rather than beneath the lots as in Unleashed (`0x5570c1`) and Classic.
    pub fn waves_draw_after_lots(self) -> bool {
        matches!(self, Self::Downtown | Self::Vacation | Self::MagicTown)
    }

    /// Offset within the `800×600` map at which water waves are blitted
    /// (`(0, 62)` for Magic Town at `0x542560`, `(0, 0)` elsewhere).
    pub fn wave_offset(self) -> [i32; 2] {
        match self {
            Self::MagicTown => [0, MAGICLAND_WAVE_OFFSET_Y],
            _ => [0, 0],
        }
    }

    /// `1024×768` outer border frame `UIGraphics` resource ID (`0x556da0`,
    /// `0x564b30`, `0x5518e0`).
    pub fn large_frame_id(self) -> i32 {
        match self {
            Self::StudioTown | Self::MagicTown => res::LARGE_MASK_SS,
            _ => res::LARGE_MASK,
        }
    }

    /// Fixed back-to-front lot Z-order for sub-neighborhood maps (from native
    /// loops and jump tables `0x54aca8`, `0x569680`, `0x564d1f`, `0x551ac9`,
    /// `0x55ecb5`). Returns `None` for [`NeighborhoodKind::Unleashed`], which
    /// enumerates `LotZoning.iff` in file order until the first entry with
    /// `lot_id > 80` (`0x557125..0x557128`).
    pub fn fixed_lot_order(self) -> Option<&'static [u16]> {
        match self {
            Self::Unleashed => None,
            Self::Classic => Some(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]),
            Self::Downtown => Some(&[23, 22, 21, 27, 26, 25, 24, 30, 29, 28]),
            Self::Vacation => Some(&[42, 41, 40, 46, 45, 44, 43, 48, 47]),
            Self::StudioTown => Some(&[81, 82, 83, 84, 85, 86, 87, 88, 89]),
            Self::MagicTown => Some(&[90, 91, 92, 93, 94, 95, 96, 97, 98]),
        }
    }

    /// Whether `filter` is supported by this map's native filter switch
    /// (`0x55d737`, `0x56bf1b`, `0x5670f4`, `0x555243`).
    pub fn supports_filter(self, filter: CommunityFilter) -> bool {
        let bit = filter.bit();
        match self {
            Self::Classic => false,
            Self::Unleashed | Self::Downtown | Self::Vacation => bit <= 0x0040,
            Self::StudioTown => bit <= 0x0400,
            Self::MagicTown => bit <= 0x1000,
        }
    }

    /// Whether `lot` is eligible to show an active filter badge on this map.
    pub fn lot_accepts_filter_badge(self, lot: &MapLot) -> bool {
        match self {
            Self::Classic => false,
            // 0x55d5d5..0x55d5e7: Unleashed tests LotZoning == community (2).
            Self::Unleashed => lot.zoning == LotZoning::Community,
            // 0x54cd4c, 0x56bdad, 0x566f93: all map lots are destination lots.
            Self::Downtown | Self::Vacation | Self::StudioTown => true,
            // 0x5550da..0x5550ea: Magic Town explicitly skips residential lots 90..=92.
            Self::MagicTown => !(90..=92).contains(&lot.id),
        }
    }

    /// Native map-space badge offset `(dx, dy)` relative to `lot.anchor`
    /// (`0x55d61f`, `0x54cd60..0x54cd74`, `0x56bdc7..0x56bde2`,
    /// `0x566fa7..0x566fbb`, `0x55510a`).
    pub fn filter_badge_offset(self, lot_id: u16) -> [i32; 2] {
        let dy = match (self, lot_id) {
            (Self::Downtown, 23) | (Self::Vacation, 40 | 42) | (Self::StudioTown, 83) => -0x0a,
            _ => -0x46,
        };
        [-0x18, dy]
    }
}

/// Community / destination amenity filter bit (`SIMI` `globals[37]`,
/// `0x427f46` -> `cWinLot + 0x1b4`, mapped to badge sprites at `0x555216`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u16)]
pub enum CommunityFilter {
    Lodging = 0x0001,
    Food = 0x0002,
    Gardening = 0x0004,
    Shopping = 0x0008,
    Pets = 0x0010,
    SmallAnimals = 0x0020,
    Park = 0x0040,
    Spa = 0x0080,
    Movie = 0x0100,
    Model = 0x0200,
    Music = 0x0400,
    Magic = 0x0800,
    Rides = 0x1000,
}

impl CommunityFilter {
    pub const ALL: [Self; 13] = [
        Self::Lodging,
        Self::Food,
        Self::Gardening,
        Self::Shopping,
        Self::Pets,
        Self::SmallAnimals,
        Self::Park,
        Self::Spa,
        Self::Movie,
        Self::Model,
        Self::Music,
        Self::Magic,
        Self::Rides,
    ];

    /// Bitmask bit in `SIMI` `globals[37]`.
    pub fn bit(self) -> u16 {
        self as u16
    }

    /// 3-frame animated map badge `UIGraphics` resource ID (`0x555216..0x555320`).
    pub fn badge_id(self) -> i32 {
        match self {
            Self::Lodging => res::FILTER_LODGING,
            Self::Food => res::FILTER_FOOD,
            Self::Gardening => res::FILTER_GARDENING,
            Self::Shopping => res::FILTER_SHOPPING,
            Self::Pets => res::FILTER_PET,
            Self::SmallAnimals => res::FILTER_SMALL_ANIMAL,
            Self::Park => res::FILTER_PARK,
            Self::Spa => res::FILTER_SPA,
            Self::Movie => res::FILTER_MOVIE,
            Self::Model => res::FILTER_MODEL,
            Self::Music => res::FILTER_MUSIC,
            Self::Magic => res::FILTER_MAGIC,
            Self::Rides => res::FILTER_RIDES,
        }
    }
}

/// Zoning classification from `LotZoning.iff` (`STR# 1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LotZoning {
    Residential,
    Community,
    Unknown,
}

/// Occupancy / destination status of a lot on the neighborhood map.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LotStatus {
    /// Occupied by a family in `Neighborhood.iff` (`FAMI.house == lot_id`).
    Occupied {
        /// `FAMI` IFF chunk ID (`0x410673`, read at `0x4e1e73`).
        family_id: i32,
        /// Non-zero when the family is currently checked into a Vacation lot
        /// (`FAMI` version `>= 8` secondary-house field `[edx + 0x13c]`,
        /// `0x410602..0x410613`, `0x54f237`).
        vacation_lot: Option<u16>,
    },
    /// Unoccupied residential lot.
    Vacant,
    /// Community / Downtown / Vacation / Studio / Magic destination lot.
    Community {
        /// Family occupying this Vacation Island lot (`0x54f270..0x54f27e`:
        /// matches either primary or secondary house).
        occupied_by_family: Option<i32>,
    },
}

/// Five signed 32-bit integers stored in a house file's `THMB` chunk (`0x4e279f`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThmbMetrics {
    pub anchor_x: i32,
    pub anchor_y: i32,
    pub anchor_y_extra: i32,
    pub dx_extra: i32,
    pub dy_extra: i32,
}

impl ThmbMetrics {
    /// Fallback sentinel when a `HouseXX.iff` lacks a `THMB` chunk (`0x4e27da`).
    pub const CENTERED: Self = Self {
        anchor_x: THMB_CENTER_SENTINEL,
        anchor_y: THMB_CENTER_SENTINEL,
        anchor_y_extra: THMB_CENTER_SENTINEL,
        dx_extra: THMB_CENTER_SENTINEL,
        dy_extra: THMB_CENTER_SENTINEL,
    };

    /// Parses a `THMB` payload (`5 × i32` little-endian), zero-filling any
    /// trailing bytes when fewer than 20 bytes are present (`0x4e279f..0x4e27c6`).
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut buf = [0u8; 20];
        let len = bytes.len().min(buf.len());
        buf[..len].copy_from_slice(&bytes[..len]);
        let read = |i: usize| i32::from_le_bytes(buf[i..i + 4].try_into().unwrap());
        Ok(Self {
            anchor_x: read(0),
            anchor_y: read(4),
            anchor_y_extra: read(8),
            dx_extra: read(12),
            dy_extra: read(16),
        })
    }

    /// Halves all five fields toward zero (`cdq; sub eax, edx; sar eax, 1` at
    /// `0x4e285a..0x4e288e`) when a lot falls back from a present `THMB 512`
    /// chunk to half-scale. Missing chunks take the separate sentinel path at
    /// `0x4e2952` and do not call `halved`.
    pub fn halved(self) -> Self {
        Self {
            anchor_x: self.anchor_x / 2,
            anchor_y: self.anchor_y / 2,
            anchor_y_extra: self.anchor_y_extra / 2,
            dx_extra: self.dx_extra / 2,
            dy_extra: self.dy_extra / 2,
        }
    }
}

/// A 16-bit 5-6-5 color-keyed bitmap surface, matching the native surface
/// representation used by `cWinLot` (`0x54e35b`), the 0.5× resampler
/// (`0x4ba79c`), and the highlight pass (`0x54f28f`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Surface565 {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u16>,
}

impl Surface565 {
    /// Allocates a `width × height` surface filled with `color`.
    /// Returns an empty `0×0` surface if `width * height` overflows or exceeds
    /// [`MAX_SURFACE_PIXELS`].
    pub fn filled(width: usize, height: usize, color: u16) -> Self {
        let Some(len) = width
            .checked_mul(height)
            .filter(|&n| n <= MAX_SURFACE_PIXELS)
        else {
            return Self {
                width: 0,
                height: 0,
                pixels: Vec::new(),
            };
        };
        Self {
            width,
            height,
            pixels: vec![color; len],
        }
    }

    /// Converts a decoded `sims_formats::bitmap::Image` to 16-bit 5-6-5.
    /// Returns an empty `0×0` surface if the image's dimensions overflow,
    /// exceed [`MAX_SURFACE_PIXELS`], or disagree with its `rgba` buffer.
    pub fn from_bitmap(image: &BitmapImage) -> Self {
        let width = image.width as usize;
        let height = image.height as usize;
        let Some(len) = width
            .checked_mul(height)
            .filter(|&n| n <= MAX_SURFACE_PIXELS)
        else {
            return Self {
                width: 0,
                height: 0,
                pixels: Vec::new(),
            };
        };
        if len.checked_mul(4) != Some(image.rgba.len()) {
            return Self {
                width: 0,
                height: 0,
                pixels: Vec::new(),
            };
        }
        let mut pixels = Vec::with_capacity(len);
        for px in image.rgba.as_chunks::<4>().0 {
            if px[3] == 0 {
                pixels.push(COLOR_KEY_565);
            } else {
                pixels.push(pack_565([px[0], px[1], px[2]]));
            }
        }
        Self {
            width,
            height,
            pixels,
        }
    }

    /// Returns the 5-6-5 pixel at `(x, y)` if in bounds and valid.
    pub fn get(&self, x: usize, y: usize) -> Option<u16> {
        if !self.is_valid() || x >= self.width || y >= self.height {
            return None;
        }
        let idx = y.checked_mul(self.width)?.checked_add(x)?;
        self.pixels.get(idx).copied()
    }

    /// Blits `src` onto `self` at `(dx, dy)` skipping [`COLOR_KEY_565`].
    fn blit_keyed(&mut self, src: &Surface565, dx: i64, dy: i64) {
        if !self.is_valid() || !src.is_valid() {
            return;
        }
        let (sw, sh) = (src.width as i64, src.height as i64);
        let (dw, dh) = (self.width as i64, self.height as i64);
        let x0 = dx.clamp(0, dw);
        let y0 = dy.clamp(0, dh);
        let x1 = dx.saturating_add(sw).clamp(x0, dw);
        let y1 = dy.saturating_add(sh).clamp(y0, dh);
        for py in y0..y1 {
            let sy = (py - dy) as usize;
            let dst_row = py as usize * self.width;
            let src_row = sy * src.width;
            for px in x0..x1 {
                let sx = (px - dx) as usize;
                let pixel = src.pixels[src_row + sx];
                if pixel != COLOR_KEY_565 {
                    self.pixels[dst_row + px as usize] = pixel;
                }
            }
        }
    }

    /// Whether `pixels` has at least `width * height` entries without overflow
    /// and within [`MAX_SURFACE_PIXELS`].
    pub fn is_valid(&self) -> bool {
        self.width
            .checked_mul(self.height)
            .is_some_and(|n| n <= MAX_SURFACE_PIXELS && self.pixels.len() >= n)
    }
}

/// Prepared thumbnail and metadata for one lot on a neighborhood map.
#[derive(Debug, Clone)]
pub struct MapLot {
    /// Lot number (`1..=98`).
    pub id: u16,
    /// Map anchor point from `LotLocations.iff` in `800×600` map coordinates.
    pub anchor: [i32; 2],
    /// `THMB` metrics used for positioning (already halved for 0.5× fallback lots).
    pub metrics: ThmbMetrics,
    /// Top-left corner of the lot's sprite cell in `800×600` map coordinates
    /// (`0x5571ab..0x5571d9`, `0x54e9ec..0x54ea79`).
    pub cell_origin: [i32; 2],
    /// Zoning from `LotZoning.iff` (or inferred for expansion sub-neighborhoods).
    pub zoning: LotZoning,
    /// Occupancy / destination status from `Neighborhood.iff` and `LotZoning.iff`.
    pub status: LotStatus,
    /// Whether any family in `Neighborhood.iff` has `primary_house == id` and
    /// any nonzero 32-bit secondary house (`0x54f20e`), tracked across all
    /// `FAMI` records independently of [`Self::zoning`] / [`Self::status`].
    pub has_primary_vacation_stay: bool,
    /// Whether any family in `Neighborhood.iff` references `id` as its primary
    /// or secondary house (`0x54f24f`), used for Vacation Island badges.
    pub has_vacation_occupancy: bool,
    /// Community amenity filter bitmask from `SIMI` `globals[37]` (`0x427f46`).
    pub filter_mask: u16,
    /// Normal idle thumbnail cell (`BMP_ 512/1024` composited under `BMP_ 513/1025`).
    pub normal: Surface565,
    /// Hovered cutaway (roofs-off) thumbnail cell (`BMP_ 512/1024` only), when
    /// `BMP_ 513/1025` was present so two distinct cells exist (`0x54e6b2`).
    pub roofs_off: Option<Surface565>,
    /// Separate roofs-on + centered badge surface (`[esi + 0x264]`,
    /// `0x54e58b..0x54e680`), built whenever `BMP_ 513/1025` and the native
    /// Vacation / No-Vacancy badge are present, centered on `BMP_ 512/1024`'s
    /// dimensions. Selected on hover when [`RenderOptions::show_vacation_badges`]
    /// is enabled and the lot has a vacation stay (`0x54ec48..0x54ec75`).
    pub vacation_badged: Option<Surface565>,
}

impl MapLot {
    /// Width and height of the lot's sprite cell.
    pub fn size(&self) -> [u32; 2] {
        [self.normal.width as u32, self.normal.height as u32]
    }

    /// Screen-space bounding rectangle `[left, top, right, bottom]` inside a
    /// viewport of size `viewport`.
    pub fn screen_rect(&self, viewport: [u32; 2]) -> [i32; 4] {
        let origin = map_origin(viewport);
        let left = origin[0].saturating_add(self.cell_origin[0]);
        let top = origin[1].saturating_add(self.cell_origin[1]);
        let right = left.saturating_add(i32::try_from(self.normal.width).unwrap_or(i32::MAX));
        let bottom = top.saturating_add(i32::try_from(self.normal.height).unwrap_or(i32::MAX));
        [left, top, right, bottom]
    }

    /// Selects the unhighlighted cell surface for hit-testing or drawing.
    pub fn active_surface(&self, roofs_off: bool) -> &Surface565 {
        if roofs_off {
            self.roofs_off.as_ref().unwrap_or(&self.normal)
        } else {
            &self.normal
        }
    }
}

/// Options controlling a single [`NeighborhoodMap::render`] or
/// [`NeighborhoodMap::hit_test_with_options`] call.
#[derive(Debug, Clone)]
pub struct RenderOptions {
    /// Output canvas dimensions (`[800, 600]` or `[1024, 768]` natively).
    pub viewport: [u32; 2],
    /// Lot currently under the cursor, if any.
    pub hovered_lot: Option<u16>,
    /// When `true` and [`Self::hovered_lot`] has a roofs-off cell (and is not
    /// displaying its vacation-badged surface), render its cutaway interior
    /// cell (`0x54ebcb`, `BMP_ 512/1024`).
    pub hover_roofs_off: bool,
    /// When `true`, apply the native `0x54f28f` brighten + edge outline to
    /// [`Self::hovered_lot`] as well as [`Self::highlighted_lots`].
    pub highlight_hovered: bool,
    /// Additional selected or filtered lots to render with the native
    /// `0x54f28f` brighten + horizontal edge outline.
    pub highlighted_lots: BTreeSet<u16>,
    /// Outline RGB for highlighted lots (defaults to [`DEFAULT_HIGHLIGHT_RGB`]).
    pub highlight_rgb: [u8; 3],
    /// Optional water animation frame index (`0..wave_count`). `None` renders
    /// the static backdrop without a wave overlay.
    pub water_frame: Option<usize>,
    /// Active community amenity filter (`0x55d5ac..0x55d735`). On Unleashed,
    /// community lots receive the native blue highlight (`0x54f520`) and
    /// matching lots draw the animated 3-frame filter badge.
    pub active_filter: Option<CommunityFilter>,
    /// Animation step (`0..6`) for community filter badges (`0x55d6c1..0x55d735`:
    /// `0,5 -> cell 0`, `1,4 -> cell 1`, `2,3 -> cell 2`).
    pub filter_frame: usize,
    /// Draw Vacation Island port & trees overlays (`5442`, `5443`) on
    /// [`NeighborhoodKind::Vacation`].
    pub show_vacation_overlays: bool,
    /// Use the native Vacation / No-Vacancy badged surface (`[esi + 0x264]`,
    /// `5441` / `5510`) when hovering a lot with a vacation stay
    /// (`0x54ec48..0x54ec91`).
    pub show_vacation_badges: bool,
    /// Draw compact occupied/vacant status markers on residential lots.
    pub show_occupancy_markers: bool,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            viewport: [MAP_WIDTH, MAP_HEIGHT],
            hovered_lot: None,
            hover_roofs_off: true,
            highlight_hovered: true,
            highlighted_lots: BTreeSet::new(),
            highlight_rgb: DEFAULT_HIGHLIGHT_RGB,
            water_frame: None,
            active_filter: None,
            filter_frame: 0,
            show_vacation_overlays: true,
            show_vacation_badges: true,
            show_occupancy_markers: false,
        }
    }
}

/// Loaded neighborhood map screen ready to render and hit-test.
#[derive(Debug, Clone)]
pub struct NeighborhoodMap {
    pub kind: NeighborhoodKind,
    pub neighborhood: u8,
    pub non_us: bool,
    pub backdrop: Surface565,
    pub large_frame: Option<Surface565>,
    pub waves: Vec<Surface565>,
    pub vacation_port: Option<Surface565>,
    pub vacation_trees: Option<Surface565>,
    pub vacation_logo: Option<Surface565>,
    pub vacation_no_vacancy: Option<Surface565>,
    pub filter_badges: BTreeMap<CommunityFilter, Surface565>,
    /// Lots in exact back-to-front draw order (`hit_test` scans in reverse).
    pub lots: Vec<MapLot>,
}

impl NeighborhoodMap {
    /// Loads a neighborhood map screen for neighborhood `1..=8` (`UserData` or
    /// `UserData2`..=`UserData8`) from a Complete Collection install.
    pub fn load(
        install: &Install,
        neighborhood: u8,
        kind: NeighborhoodKind,
        non_us: bool,
    ) -> Result<Self> {
        if !(1..=8).contains(&neighborhood) {
            return Err(Error::new(format!(
                "neighborhood index {neighborhood} out of range 1..=8"
            )));
        }
        let user_dir_name = if neighborhood == 1 {
            "UserData".to_string()
        } else {
            format!("UserData{neighborhood}")
        };
        let user_dir = install.file(&user_dir_name)?;
        Self::load_from_dir(install, &user_dir, neighborhood, kind, non_us)
    }

    /// Loads a neighborhood map screen from an explicit `UserData`-style
    /// directory (useful for tests against `TemplateUserData` or custom saves).
    pub fn load_from_dir(
        install: &Install,
        user_dir: &Path,
        neighborhood: u8,
        kind: NeighborhoodKind,
        non_us: bool,
    ) -> Result<Self> {
        let ui = UiGraphics::open(install)?;
        let load_ui = |id: i32| -> Result<Option<Surface565>> {
            Ok(ui
                .bitmap(id, US_ENGLISH)?
                .map(|img| Surface565::from_bitmap(&img)))
        };

        let backdrop = load_ui(kind.backdrop_id(non_us))?.ok_or_else(|| {
            Error::new(format!(
                "missing neighborhood backdrop resource {}",
                kind.backdrop_id(non_us)
            ))
        })?;
        let large_frame = load_ui(kind.large_frame_id())?;
        let mut waves = Vec::new();
        for &wave_id in kind.wave_ids(non_us) {
            if let Some(surf) = load_ui(wave_id)? {
                waves.push(surf);
            }
        }
        let (vacation_port, vacation_trees, vacation_logo, vacation_no_vacancy) =
            if kind == NeighborhoodKind::Vacation {
                (
                    load_ui(res::VACATION_PORT)?,
                    load_ui(res::VACATION_TREES)?,
                    load_ui(res::VACATION_LOGO)?,
                    load_ui(res::VACATION_NO_VACANCY)?,
                )
            } else {
                (None, None, load_ui(res::VACATION_LOGO)?, None)
            };

        let mut filter_badges = BTreeMap::new();
        for filter in CommunityFilter::ALL {
            if kind.supports_filter(filter)
                && let Some(surf) = load_ui(filter.badge_id())?
            {
                filter_badges.insert(filter, surf);
            }
        }

        let locations = load_lot_locations(install, user_dir, kind.locations_str_id())?;
        let zoning_table = load_lot_zoning(install, user_dir)?;
        let occupancy = load_family_occupancy(install, user_dir)?;

        // 0x557122..0x557128: Unleashed walks LotZoning in file order and stops
        // at the first entry whose signed 32-bit lot ID exceeds 80 (0x50).
        let lot_ids: Vec<u16> = match kind.fixed_lot_order() {
            Some(fixed) => fixed.to_vec(),
            None => zoning_table
                .rows
                .iter()
                .take_while(|&&(id, _)| id <= 80)
                .filter_map(|&(id, _)| u16::try_from(id).ok())
                .collect(),
        };

        let houses_dir = resolve_child(install, user_dir, "Houses").ok_or_else(|| {
            Error::new(format!(
                "missing Houses directory inside {}",
                user_dir.display()
            ))
        })?;

        let badge_sprite = if kind == NeighborhoodKind::Vacation {
            vacation_no_vacancy.as_ref()
        } else {
            vacation_logo.as_ref()
        };

        let mut lots = Vec::with_capacity(lot_ids.len());
        for lot_id in lot_ids {
            // 0x4e2501..0x4e257c: missing LotLocations row retains [0, 0] anchor.
            let anchor = locations.get(&lot_id).copied().unwrap_or([0, 0]);
            let house_name = format!("House{lot_id:02}.iff");
            let Some(house_path) = resolve_child(install, &houses_dir, &house_name) else {
                continue;
            };
            let iff_bytes = fs::read(&house_path)
                .map_err(|e| Error::new(format!("reading {}: {e}", house_path.display())))?;
            let iff = Iff::parse(iff_bytes)?;

            let Some(BuiltLotCells {
                metrics,
                normal,
                roofs_off,
                vacation_badged,
            }) = build_lot_cells(&iff, kind.uses_half_scale(), badge_sprite)?
            else {
                continue;
            };

            let filter_mask = read_lot_filter_mask(&iff);
            let zoning = zoning_table
                .first_zone
                .get(&i32::from(lot_id))
                .copied()
                .unwrap_or(match kind {
                    NeighborhoodKind::Classic => LotZoning::Residential,
                    NeighborhoodKind::Downtown
                    | NeighborhoodKind::Vacation
                    | NeighborhoodKind::StudioTown => LotZoning::Community,
                    NeighborhoodKind::MagicTown => {
                        if lot_id <= 92 {
                            LotZoning::Residential
                        } else {
                            LotZoning::Community
                        }
                    }
                    NeighborhoodKind::Unleashed => LotZoning::Unknown,
                });
            let status = resolve_lot_status(lot_id, kind, zoning, &occupancy);
            let lot_i32 = i32::from(lot_id);
            let has_primary_vacation_stay = occupancy.primary_vacation_stays.contains(&lot_i32);
            let has_vacation_occupancy = occupancy.any_house_matches.contains(&lot_i32);
            let cell_origin = compute_cell_origin(anchor, metrics, [normal.width, normal.height]);

            lots.push(MapLot {
                id: lot_id,
                anchor,
                metrics,
                cell_origin,
                zoning,
                status,
                has_primary_vacation_stay,
                has_vacation_occupancy,
                filter_mask,
                normal,
                roofs_off,
                vacation_badged,
            });
        }

        Ok(Self {
            kind,
            neighborhood,
            non_us,
            backdrop,
            large_frame,
            waves,
            vacation_port,
            vacation_trees,
            vacation_logo,
            vacation_no_vacancy,
            filter_badges,
            lots,
        })
    }

    /// Returns metadata for a specific lot by ID.
    pub fn lot(&self, id: u16) -> Option<&MapLot> {
        self.lots.iter().find(|lot| lot.id == id)
    }

    /// Renders the neighborhood map screen into a freshly allocated [`Canvas`].
    ///
    /// Returns an empty `0×0` canvas if `options.viewport` overflows or exceeds
    /// [`MAX_SURFACE_PIXELS`]; use [`Self::try_render`] to receive an error.
    pub fn render(&self, options: &RenderOptions) -> Canvas {
        self.try_render(options)
            .unwrap_or_else(|_| Canvas::new(0, 0, [0, 0, 0]))
    }

    /// Fallible counterpart to [`Self::render`] that validates viewport size
    /// against [`MAX_SURFACE_PIXELS`] before allocating a [`Canvas`].
    pub fn try_render(&self, options: &RenderOptions) -> Result<Canvas> {
        let w = options.viewport[0] as usize;
        let h = options.viewport[1] as usize;
        if w.checked_mul(h).is_none_or(|n| n > MAX_SURFACE_PIXELS) {
            return Err(Error::new(format!(
                "viewport {}×{} exceeds maximum {} pixels",
                options.viewport[0], options.viewport[1], MAX_SURFACE_PIXELS
            )));
        }
        let mut canvas = Canvas::new(w, h, [0, 0, 0]);
        self.render_into(&mut canvas, options);
        Ok(canvas)
    }

    /// Renders the neighborhood map screen into an existing [`Canvas`].
    ///
    /// Sets `canvas.set_id(Some(lot.id))` for each lot's opaque pixels so
    /// [`Canvas::id_at`] agrees with [`Self::hit_test_with_options`] inside
    /// the clipped `800×600` map rectangle.
    pub fn render_into(&self, canvas: &mut Canvas, options: &RenderOptions) {
        let viewport = [canvas.width as u32, canvas.height as u32];
        let origin = map_origin(viewport);
        let map_clip = map_clip_rect(viewport);

        canvas.clear([0, 0, 0]);
        canvas.set_id(None);

        // 1. 1024×768 outer frame when viewport exceeds 800×600 (0x556d8c..0x557031).
        if (viewport[0] > MAP_WIDTH || viewport[1] > MAP_HEIGHT)
            && let Some(frame) = &self.large_frame
        {
            blit_surface_to_canvas(
                canvas,
                frame,
                [0, 0],
                [0, 0, canvas.width as i32, canvas.height as i32],
                false,
                None,
            );
        }

        // 2. 800×600 map backdrop at centered origin (0x556db7..0x556ddc).
        blit_surface_to_canvas(canvas, &self.backdrop, origin, map_clip, false, None);

        // 3. Pre-lot water wave overlay for Unleashed (0x5570c1) and Classic.
        if !self.kind.waves_draw_after_lots()
            && let Some(frame_idx) = options.water_frame
            && !self.waves.is_empty()
        {
            let wave = &self.waves[frame_idx % self.waves.len()];
            let [wx, wy] = self.kind.wave_offset();
            let at = [origin[0].saturating_add(wx), origin[1].saturating_add(wy)];
            blit_surface_to_canvas(canvas, wave, at, map_clip, true, None);
        }

        // 4. Lots in back-to-front Z-order, clipped strictly to the 800×600 map rect
        //    (cWinLot + 0x230..0x240, 0x5571e4..0x557201).
        for lot in &self.lots {
            let draw_at = [
                origin[0].saturating_add(lot.cell_origin[0]),
                origin[1].saturating_add(lot.cell_origin[1]),
            ];
            if let Some(badged) = self.vacation_badged_for_draw(lot, options) {
                // 0x54ec5a: native draws [esi + 0x264] directly without
                // applying highlight; picking (0x5e40b4) still tests the
                // regular thumbnail [esi + 0xd8], so record pick IDs from the
                // regular pick surface.
                blit_color_only(canvas, badged, draw_at, map_clip);
                let pick_surf = self.resolved_pick_surface(lot, options);
                mark_surface_ids(canvas, &pick_surf, draw_at, map_clip, lot.id);
            } else {
                let surface = self.resolved_pick_surface(lot, options);
                blit_surface_to_canvas(canvas, &surface, draw_at, map_clip, true, Some(lot.id));
            }
        }

        // 5. Vacation Island foreground port & trees overlays (5442, 5443)
        //    at 0x56baeb..0x56bb5d (drawn after lots, before Vacation waves).
        if options.show_vacation_overlays && self.kind == NeighborhoodKind::Vacation {
            if let Some(port) = &self.vacation_port {
                blit_color_only(canvas, port, origin, map_clip);
            }
            if let Some(trees) = &self.vacation_trees {
                blit_color_only(canvas, trees, origin, map_clip);
            }
        }

        // 6. Post-lot water wave overlay for Downtown (0x54cb21), Vacation
        //    (0x56bb7a, after port & trees), and Magic Town (0x554ff6 at (0, 62)).
        if self.kind.waves_draw_after_lots()
            && let Some(frame_idx) = options.water_frame
            && !self.waves.is_empty()
        {
            let wave = &self.waves[frame_idx % self.waves.len()];
            let [wx, wy] = self.kind.wave_offset();
            let at = [origin[0].saturating_add(wx), origin[1].saturating_add(wy)];
            blit_color_only(canvas, wave, at, map_clip);
        }

        // 7. Optional residential occupied/vacant status markers.
        if options.show_occupancy_markers {
            for lot in &self.lots {
                let marker_rgb = match lot.status {
                    LotStatus::Occupied { .. } => [0x38, 0xd8, 0x48],
                    LotStatus::Vacant => [0xe0, 0xb8, 0x30],
                    LotStatus::Community { .. } => continue,
                };
                draw_status_pip(
                    canvas,
                    [
                        origin[0].saturating_add(lot.anchor[0]),
                        origin[1].saturating_add(lot.anchor[1]).saturating_sub(4),
                    ],
                    map_clip,
                    marker_rgb,
                );
            }
        }

        // 8. Active community filter animated 3-frame badges (0x55d5ac..0x55d735,
        //    0x54cd4c..0x54cd89, 0x56bdad..0x56bdf7, 0x566f93..0x566fd0,
        //    0x5550da..0x555119).
        if let Some(filter) = options.active_filter
            && self.kind.supports_filter(filter)
            && let Some(strip) = self.filter_badges.get(&filter)
        {
            let cell_w = strip.width / 3;
            let cell_idx = filter_badge_cell(options.filter_frame);
            if cell_w > 0 && strip.height > 0 {
                let src_x = cell_idx * cell_w;
                for lot in &self.lots {
                    if !self.kind.lot_accepts_filter_badge(lot) {
                        continue;
                    }
                    if lot.filter_mask & filter.bit() == 0 {
                        continue;
                    }
                    let [dx, dy] = self.kind.filter_badge_offset(lot.id);
                    let bx = origin[0].saturating_add(lot.anchor[0]).saturating_add(dx);
                    let by = origin[1].saturating_add(lot.anchor[1]).saturating_add(dy);
                    blit_subrect_to_canvas(
                        canvas,
                        strip,
                        [src_x, 0, cell_w, strip.height],
                        [bx, by],
                        map_clip,
                    );
                }
            }
        }

        canvas.set_id(None);
    }

    /// Exact native silhouette hit-test (`0x5e4071` -> `0x5dd79a`) at screen
    /// `point` in a viewport of size `viewport`.
    ///
    /// Tests lots in reverse draw order (front-most wins) against each lot's
    /// widget bounds and active bitmap silhouette, without applying the
    /// `+0x230` draw clip (matching `0x5dc83c` / `0x5e4071`).
    pub fn hit_test(&self, viewport: [u32; 2], point: [i32; 2]) -> Option<u16> {
        let options = RenderOptions {
            viewport,
            hover_roofs_off: false,
            highlight_hovered: false,
            ..RenderOptions::default()
        };
        self.hit_test_with_options(&options, point)
    }

    /// Like [`Self::hit_test`], accounting for a hovered lot currently showing
    /// its roofs-off cutaway cell.
    pub fn hit_test_with_state(
        &self,
        viewport: [u32; 2],
        point: [i32; 2],
        hovered_lot: Option<u16>,
        hover_roofs_off: bool,
    ) -> Option<u16> {
        let options = RenderOptions {
            viewport,
            hovered_lot,
            hover_roofs_off,
            highlight_hovered: false,
            ..RenderOptions::default()
        };
        self.hit_test_with_options(&options, point)
    }

    /// Like [`Self::hit_test`], testing the exact active regular thumbnail
    /// (`[esi + 0xd8]`, possibly highlighted across its full two-cell strip
    /// width per `0x54f347..0x54f3e1`) selected by `options`, matching
    /// `0x54f2af` + `0x5e4071` + `0x5e40b4`. Bounds are checked before any
    /// highlight evaluation, and only the single sampled scanline is
    /// highlighted on hit candidates.
    pub fn hit_test_with_options(&self, options: &RenderOptions, point: [i32; 2]) -> Option<u16> {
        let origin = map_origin(options.viewport);
        for lot in self.lots.iter().rev() {
            let is_hovered = options.hovered_lot == Some(lot.id);
            let show_cutaway = is_hovered && options.hover_roofs_off;
            let base = lot.active_surface(show_cutaway);
            if !base.is_valid() {
                continue;
            }
            let draw_x = i64::from(origin[0]) + i64::from(lot.cell_origin[0]);
            let draw_y = i64::from(origin[1]) + i64::from(lot.cell_origin[1]);
            let lx = i64::from(point[0]) - draw_x;
            let ly = i64::from(point[1]) - draw_y;
            if lx < 0 || ly < 0 || lx >= base.width as i64 || ly >= base.height as i64 {
                continue;
            }
            let (ux, uy) = (lx as usize, ly as usize);
            let pixel = match self.highlight_rgb_for(lot, options) {
                Some(rgb) => highlight_lot_row_pixel(lot, uy, ux, show_cutaway, pack_565(rgb)),
                None => base.pixels[uy * base.width + ux],
            };
            if pixel != COLOR_KEY_565 {
                return Some(lot.id);
            }
        }
        None
    }

    /// Whether `lot` satisfies the native Vacation badge predicate: `0x54eb49`
    /// (`0x54f20e`: primary house has a secondary vacation stay, independent
    /// of zoning and checked on every map including Magic Town homes) or
    /// `0x54eb52..0x54eb64` (`0x54f24f`: Vacation Island lot occupied by a
    /// family's primary or secondary house).
    fn has_vacation_badge(&self, lot: &MapLot) -> bool {
        lot.has_primary_vacation_stay
            || matches!(
                lot.status,
                LotStatus::Occupied {
                    vacation_lot: Some(_),
                    ..
                }
            )
            || (self.kind == NeighborhoodKind::Vacation
                && (lot.has_vacation_occupancy
                    || matches!(
                        lot.status,
                        LotStatus::Community {
                            occupied_by_family: Some(_),
                        } | LotStatus::Occupied { .. }
                    )))
    }

    /// Returns the unhighlighted `[esi + 0x264]` vacation-badged surface when
    /// it replaces `[esi + 0xd8]` for drawing (`0x54ec48..0x54ec75`).
    fn vacation_badged_for_draw<'a>(
        &'a self,
        lot: &'a MapLot,
        options: &RenderOptions,
    ) -> Option<&'a Surface565> {
        let is_hovered = options.hovered_lot == Some(lot.id);
        if is_hovered && options.show_vacation_badges && self.has_vacation_badge(lot) {
            lot.vacation_badged.as_ref()
        } else {
            None
        }
    }

    /// Returns the highlight outline RGB applied to `lot`'s regular thumbnail
    /// `[esi + 0xd8]` under `options`, if any (`0x54f28f`, `0x55d5e9..0x55d5ff`).
    fn highlight_rgb_for(&self, lot: &MapLot, options: &RenderOptions) -> Option<[u8; 3]> {
        let is_hovered = options.hovered_lot == Some(lot.id);
        let explicit_highlight =
            options.highlighted_lots.contains(&lot.id) || (is_hovered && options.highlight_hovered);
        if explicit_highlight {
            Some(options.highlight_rgb)
        } else if self.kind == NeighborhoodKind::Unleashed
            && options.active_filter.is_some()
            && lot.zoning == LotZoning::Community
        {
            Some(COMMUNITY_FILTER_HIGHLIGHT_RGB)
        } else {
            None
        }
    }

    /// Resolves `lot`'s regular thumbnail surface `[esi + 0xd8]` (possibly
    /// highlighted across its full native two-cell strip width per
    /// `0x54f347..0x54f3e1`) under `options`, matching `0x54f2af` + `0x5e40b4`.
    fn resolved_pick_surface<'a>(
        &'a self,
        lot: &'a MapLot,
        options: &RenderOptions,
    ) -> Cow<'a, Surface565> {
        let is_hovered = options.hovered_lot == Some(lot.id);
        let show_cutaway = is_hovered && options.hover_roofs_off;
        match self.highlight_rgb_for(lot, options) {
            Some(rgb) => Cow::Owned(highlight_lot_cell(lot, show_cutaway, rgb)),
            None => Cow::Borrowed(lot.active_surface(show_cutaway)),
        }
    }
}

/// Computes the top-left origin of the `800×600` map inside `viewport`
/// (`0x556db7..0x556ddc`): `[0, 0]` at `800×600`, `[112, 84]` at `1024×768`.
pub fn map_origin(viewport: [u32; 2]) -> [i32; 2] {
    let ox = if viewport[0] > MAP_WIDTH {
        ((viewport[0] - MAP_WIDTH) / 2) as i32
    } else {
        0
    };
    let oy = if viewport[1] > MAP_HEIGHT {
        ((viewport[1] - MAP_HEIGHT) / 2) as i32
    } else {
        0
    };
    [ox, oy]
}

/// Computes the map clip rectangle `[left, top, right, bottom]` inside
/// `viewport`, matching `cWinLot + 0x230..0x240` (`0x5571e4..0x557201`).
pub fn map_clip_rect(viewport: [u32; 2]) -> [i32; 4] {
    let [ox, oy] = map_origin(viewport);
    let vw = i32::try_from(viewport[0]).unwrap_or(i32::MAX);
    let vh = i32::try_from(viewport[1]).unwrap_or(i32::MAX);
    [
        ox.clamp(0, vw),
        oy.clamp(0, vh),
        ox.saturating_add(MAP_WIDTH as i32).clamp(0, vw),
        oy.saturating_add(MAP_HEIGHT as i32).clamp(0, vh),
    ]
}

/// Maps the 6-step badge animation counter to one of the 3 horizontal sprite
/// cells (`0x55d6c1..0x55d735`): `0,5 -> 0`, `1,4 -> 1`, `2,3 -> 2`.
pub fn filter_badge_cell(step: usize) -> usize {
    match step % 6 {
        0 | 5 => 0,
        1 | 4 => 1,
        _ => 2,
    }
}

/// Computes the lot cell's top-left corner in `800×600` map coordinates
/// (`0x5571ab..0x5571d9`, `0x54e9ec..0x54ea79`).
pub fn compute_cell_origin(
    anchor: [i32; 2],
    metrics: ThmbMetrics,
    cell_size: [usize; 2],
) -> [i32; 2] {
    let pos_x = anchor[0].saturating_add(metrics.dx_extra.min(0));
    let pos_y = anchor[1].saturating_add(metrics.dy_extra.min(0));
    if metrics.anchor_x == THMB_CENTER_SENTINEL {
        let half_w = i32::try_from(cell_size[0]).unwrap_or(i32::MAX) / 2;
        let half_h = i32::try_from(cell_size[1]).unwrap_or(i32::MAX) / 2;
        [pos_x.saturating_sub(half_w), pos_y.saturating_sub(half_h)]
    } else {
        [
            pos_x.saturating_sub(metrics.anchor_x),
            pos_y
                .saturating_sub(metrics.anchor_y)
                .saturating_sub(metrics.anchor_y_extra),
        ]
    }
}

/// Packs 8-bit RGB into native 16-bit 5-6-5 (`0x49f0ae`, `0x4bb479`).
#[inline]
pub fn pack_565([r, g, b]: [u8; 3]) -> u16 {
    ((u16::from(r & 0xf8)) << 8) | ((u16::from(g & 0xfc)) << 3) | (u16::from(b) >> 3)
}

/// Unpacks 16-bit 5-6-5 to top-aligned 8-bit RGB (`0x438ef6`: low bits zero).
/// Used internally by `0x4ba79c` (resampler) and `0x49bbfe` (YIQ brighten).
#[inline]
pub fn unpack_565_top(p: u16) -> [u8; 3] {
    [
        ((p >> 8) & 0xf8) as u8,
        ((p >> 3) & 0xfc) as u8,
        ((p << 3) & 0xf8) as u8,
    ]
}

/// Widens 16-bit 5-6-5 to full-range 8-bit RGB matching [`quantize565`].
#[inline]
pub fn widen_565(p: u16) -> [u8; 3] {
    let r5 = ((p >> 11) & 0x1f) as u8;
    let g6 = ((p >> 5) & 0x3f) as u8;
    let b5 = (p & 0x1f) as u8;
    [
        (r5 << 3) | (r5 >> 2),
        (g6 << 2) | (g6 >> 4),
        (b5 << 3) | (b5 >> 2),
    ]
}

/// Native 0.5× separable Hermite image resampler (`0x4ba778` -> `0x4ba79c`).
///
/// Reproduces `Sims.exe` instruction-for-instruction:
/// - `dst_w = ceil(src_w * 0.5)`, `dst_h = ceil(src_h * 0.5)` (`0x636b98`)
/// - Hermite kernel `(2t - 3)t^2 + 1` for `|t| < 1`, reflection boundary
/// - Two passes (horizontal then vertical) in 16-bit 5-6-5, skipping
///   [`COLOR_KEY_565`] and emitting [`COLOR_KEY_565`] whenever surviving weight
///   is below `0.46875` (`0x668820`).
pub fn resample_half_565(src: &Surface565) -> Surface565 {
    if src.width == 0 || src.height == 0 || !src.is_valid() {
        return Surface565::filled(0, 0, COLOR_KEY_565);
    }

    let round_ftol = |v: f64| -> i32 {
        if v >= 0.0 {
            (v + 0.5) as i32
        } else {
            (v - 0.5) as i32
        }
    };

    let dst_w = round_ftol(libm::ceil(src.width as f64 * 0.5)).max(1) as usize;
    let dst_h = round_ftol(libm::ceil(src.height as f64 * 0.5)).max(1) as usize;
    let sx = dst_w as f32 / src.width as f32;
    let sy = dst_h as f32 / src.height as f32;

    let build_contrib = |src_len: usize, dst_len: usize, scale: f32| -> Vec<Vec<(usize, f32)>> {
        let width = if scale < 1.0 { 1.0 / scale } else { 1.0 };
        let fscale = 1.0 / width;
        let mut table = Vec::with_capacity(dst_len);
        for i in 0..dst_len {
            let center = i as f32 / scale;
            let left = round_ftol(libm::ceil(f64::from(center - width)));
            let right = round_ftol(libm::floor(f64::from(center + width)));
            let mut items = Vec::new();
            let mut wsum = 0.0f64;
            for j in left..=right {
                let t = ((center - j as f32) * fscale).abs();
                let wt = if t < 1.0 {
                    ((2.0 * t - 3.0) * t * t + 1.0) * fscale
                } else {
                    0.0
                };
                wsum += f64::from(wt);
                let idx = reflect_index(j, src_len);
                items.push((idx, wt));
            }
            if wsum > 1e-12 {
                let inv = (1.0 / wsum) as f32;
                for (_, wt) in &mut items {
                    *wt *= inv;
                }
            }
            table.push(items);
        }
        table
    };

    let cx = build_contrib(src.width, dst_w, sx);
    let cy = build_contrib(src.height, dst_h, sy);

    // Horizontal pass: src (w × h) -> tmp (dst_w × h)
    let mut tmp = vec![COLOR_KEY_565; dst_w * src.height];
    for y in 0..src.height {
        let src_row = &src.pixels[y * src.width..(y + 1) * src.width];
        let tmp_row = &mut tmp[y * dst_w..(y + 1) * dst_w];
        for (x, taps) in cx.iter().enumerate() {
            let (mut r, mut g, mut b, mut w) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
            for &(k, wt) in taps {
                let p = src_row[k];
                if p == COLOR_KEY_565 {
                    continue;
                }
                let [pr, pg, pb] = unpack_565_top(p);
                r += f32::from(pr) * wt;
                g += f32::from(pg) * wt;
                b += f32::from(pb) * wt;
                w += wt;
            }
            if w >= RESAMPLE_WEIGHT_CUTOFF {
                let inv = 1.0 / w;
                tmp_row[x] = pack_565([
                    (r * inv + 0.5).clamp(0.0, 255.0) as u8,
                    (g * inv + 0.5).clamp(0.0, 255.0) as u8,
                    (b * inv + 0.5).clamp(0.0, 255.0) as u8,
                ]);
            }
        }
    }

    // Vertical pass: tmp (dst_w × h) -> out (dst_w × dst_h)
    let mut out = vec![COLOR_KEY_565; dst_w * dst_h];
    for (y, taps) in cy.iter().enumerate() {
        let out_row = &mut out[y * dst_w..(y + 1) * dst_w];
        for x in 0..dst_w {
            let (mut r, mut g, mut b, mut w) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
            for &(k, wt) in taps {
                let p = tmp[k * dst_w + x];
                if p == COLOR_KEY_565 {
                    continue;
                }
                let [pr, pg, pb] = unpack_565_top(p);
                r += f32::from(pr) * wt;
                g += f32::from(pg) * wt;
                b += f32::from(pb) * wt;
                w += wt;
            }
            if w >= RESAMPLE_WEIGHT_CUTOFF {
                let inv = 1.0 / w;
                out_row[x] = pack_565([
                    (r * inv + 0.5).clamp(0.0, 255.0) as u8,
                    (g * inv + 0.5).clamp(0.0, 255.0) as u8,
                    (b * inv + 0.5).clamp(0.0, 255.0) as u8,
                ]);
            }
        }
    }

    Surface565 {
        width: dst_w,
        height: dst_h,
        pixels: out,
    }
}

fn reflect_index(mut k: i32, len: usize) -> usize {
    if len <= 1 || len > (i32::MAX / 2) as usize {
        return 0;
    }
    let n = len as i32;
    while k < 0 || k >= n {
        if k < 0 {
            k = k.saturating_neg();
        }
        if k >= n {
            k = 2 * n - k - 2;
        }
    }
    k as usize
}

/// Native YIQ luminance adjustment (`0x49bbfe`) used by `cWinLot::Highlight`
/// (`0x54f42d..0x54f456`).
pub fn brighten_565(pixel: u16, y_scale: f32, y_offset: f32) -> u16 {
    let [r8, g8, b8] = unpack_565_top(pixel);
    let inv255 = f32::from_bits(0x3b80_8081); // [0x667664] = 1.0 / 255.0
    let rf = f64::from(f32::from(r8) * inv255);
    let gf = f64::from(f32::from(g8) * inv255);
    let bf = f64::from(f32::from(b8) * inv255);

    // 0x49bc4a..0x49bcaa (Y, U/Q, V/I transform with native f64 constants)
    let y = (gf * 0.587 + rf * 0.299 + bf * 0.114) as f32;
    let c1 = (bf * 0.434 - (rf * 0.146 + gf * 0.288)) as f32;
    let c2 = (rf * 0.617 - gf * 0.517 - bf * 0.1) as f32;

    let y2 = f64::from(y * y_scale + y_offset);
    let c1 = f64::from(c1);
    let c2 = f64::from(c2);

    // 0x49bcb5..0x49bd00
    let mut ro = (y2 - c1 * 0.001 + c2 * 1.136) as f32;
    let mut go = (y2 - c1 * 0.396 - c2 * 0.578) as f32;
    let mut bo = (y2 + c1 * 2.041 - c2 * 0.002) as f32;

    ro = ro.clamp(0.0, 1.0);
    go = go.clamp(0.0, 1.0);
    bo = bo.clamp(0.0, 1.0);

    pack_565([
        (ro * 255.0 + 0.5) as u8,
        (go * 255.0 + 0.5) as u8,
        (bo * 255.0 + 0.5) as u8,
    ])
}

fn highlight_row_in_place(row: &mut [u16], outline_565: u16) {
    let w = row.len();
    for x in 0..w {
        let p = row[x];
        if p != COLOR_KEY_565 {
            row[x] = brighten_565(p, 1.0, HIGHLIGHT_Y_OFFSET);
        } else {
            if x + 1 < w && row[x + 1] != COLOR_KEY_565 {
                row[x + 1] = outline_565;
            }
            if x > 0 && row[x - 1] != COLOR_KEY_565 {
                row[x - 1] = outline_565;
            }
        }
    }
}

/// Highlights a single scanline `uy` of `lot`'s native `[esi + 0xd8]` strip
/// (`[normal_row | roofs_off_row]` when both cells exist with matching
/// dimensions, per `0x54e844..0x54e8d6` and `0x54f347..0x54f3e1`) and returns
/// pixel `ux` in the selected cell.
fn highlight_lot_row_pixel(
    lot: &MapLot,
    uy: usize,
    ux: usize,
    show_cutaway: bool,
    outline_565: u16,
) -> u16 {
    let normal = &lot.normal;
    if let Some(cutaway) = lot.roofs_off.as_ref().filter(|c| {
        c.is_valid() && normal.is_valid() && c.width == normal.width && c.height == normal.height
    }) {
        let w = normal.width;
        let start = uy * w;
        let mut strip_row = Vec::with_capacity(w * 2);
        strip_row.extend_from_slice(&normal.pixels[start..start + w]);
        strip_row.extend_from_slice(&cutaway.pixels[start..start + w]);
        highlight_row_in_place(&mut strip_row, outline_565);
        let offset = if show_cutaway { w } else { 0 };
        strip_row[offset + ux]
    } else {
        let base = lot.active_surface(show_cutaway);
        let w = base.width;
        let start = uy * w;
        let mut row = base.pixels[start..start + w].to_vec();
        highlight_row_in_place(&mut row, outline_565);
        row[ux]
    }
}

/// Highlights `lot`'s active cell using the full native `[esi + 0xd8]`
/// two-cell strip row width (`0x54f347..0x54f3e1`).
fn highlight_lot_cell(lot: &MapLot, show_cutaway: bool, highlight_rgb: [u8; 3]) -> Surface565 {
    let normal = &lot.normal;
    let outline_565 = pack_565(highlight_rgb);
    if let Some(cutaway) = lot.roofs_off.as_ref().filter(|c| {
        c.is_valid() && normal.is_valid() && c.width == normal.width && c.height == normal.height
    }) {
        let w = normal.width;
        let h = normal.height;
        if w == 0 || h == 0 {
            return Surface565::filled(0, 0, COLOR_KEY_565);
        }
        let mut out = Vec::with_capacity(w * h);
        let mut strip_row = vec![0u16; w * 2];
        let slice_range = if show_cutaway { w..w * 2 } else { 0..w };
        for uy in 0..h {
            let start = uy * w;
            strip_row[..w].copy_from_slice(&normal.pixels[start..start + w]);
            strip_row[w..].copy_from_slice(&cutaway.pixels[start..start + w]);
            highlight_row_in_place(&mut strip_row, outline_565);
            out.extend_from_slice(&strip_row[slice_range.clone()]);
        }
        Surface565 {
            width: w,
            height: h,
            pixels: out,
        }
    } else {
        highlight_surface(lot.active_surface(show_cutaway), highlight_rgb)
    }
}

/// Applies the native `cWinLot` highlight transform (`0x54f28f..0x54f4b4`):
/// left-to-right scanline pass in 5-6-5 that brightens every non-colorkey pixel
/// via [`brighten_565`] (`y_scale = 1.0`, `y_offset = 0.09`) and outlines
/// horizontal silhouette boundary pixels adjacent to color-key pixels in
/// `highlight_rgb`.
pub fn highlight_surface(src: &Surface565, highlight_rgb: [u8; 3]) -> Surface565 {
    let mut out = src.clone();
    let outline_565 = pack_565(highlight_rgb);
    let w = out.width;
    if w == 0 || out.height == 0 || !out.is_valid() {
        return out;
    }
    for row in out.pixels.chunks_exact_mut(w).take(out.height) {
        highlight_row_in_place(row, outline_565);
    }
    out
}

/// Computes bounded combined cell dimensions and `i64` blit offsets for
/// `roofs_off` (`BMP_ 512/1024`) and `roofs_on` (`BMP_ 513/1025`) matching
/// `0x54e42e..0x54e504` and `0x54e813..0x54e8d6`.
///
/// Uses `i64` throughout so hostile offsets (`i32::MIN`, `i32::MAX` /
/// [`THMB_CENTER_SENTINEL`]) never overflow in debug or release builds, and
/// falls back to `roofs_off`'s bounded dimensions whenever raw dimensions would
/// exceed [`MAX_LOT_CELL_DIM`].
fn lot_cell_layout(
    roofs_off: &Surface565,
    roofs_on: &Surface565,
    metrics: ThmbMetrics,
) -> (usize, usize, i64, i64, i64, i64) {
    let dx = i64::from(metrics.dx_extra);
    let dy = i64::from(metrics.dy_extra);

    let off_w = roofs_off.width as i64;
    let off_h = roofs_off.height as i64;
    let on_w = roofs_on.width as i64;
    let on_h = roofs_on.height as i64;

    let raw_w = if dx >= 0 {
        off_w.max(on_w.saturating_add(dx))
    } else {
        on_w.max(off_w.saturating_sub(dx))
    };
    let raw_h = if dy >= 0 {
        off_h.max(on_h.saturating_add(dy))
    } else {
        on_h.max(off_h.saturating_sub(dy))
    };

    let limit = MAX_LOT_CELL_DIM as i64;
    let (cell_w, cell_h) = if raw_w > limit || raw_h > limit {
        // Guard memory against hostile or sentinel THMB offsets while preserving
        // the exact raw blit offsets (so e.g. THMB_CENTER_SENTINEL starts
        // roofs_on at 0x7fffffff, off-surface, matching 0x54e88a..0x54e8d6).
        (
            roofs_off.width.min(MAX_LOT_CELL_DIM),
            roofs_off.height.min(MAX_LOT_CELL_DIM),
        )
    } else {
        (raw_w.max(0) as usize, raw_h.max(0) as usize)
    };

    let off_x = (-dx).max(0);
    let off_y = (-dy).max(0);
    let on_x = dx.max(0);
    let on_y = dy.max(0);

    (cell_w, cell_h, off_x, off_y, on_x, on_y)
}

/// Combines a lot's roofs-off (`BMP_ 512/1024`) and roofs-on (`BMP_ 513/1025`)
/// thumbnails into the native two-cell layout (`0x54e42e..0x54e8fb`).
pub fn compose_lot_cells(
    roofs_off: &Surface565,
    roofs_on: Option<&Surface565>,
    metrics: ThmbMetrics,
) -> (Surface565, Option<Surface565>) {
    let Some(roofs_on) = roofs_on else {
        return (roofs_off.clone(), None);
    };

    let (cell_w, cell_h, off_x, off_y, on_x, on_y) = lot_cell_layout(roofs_off, roofs_on, metrics);

    // Cell 0 (normal): roofs_off base + roofs_on overlay (0x54e844..0x54e8d6).
    let mut normal = Surface565::filled(cell_w, cell_h, COLOR_KEY_565);
    normal.blit_keyed(roofs_off, off_x, off_y);
    normal.blit_keyed(roofs_on, on_x, on_y);

    // Cell 1 (hovered cutaway): roofs_off only (0x54e85a..0x54e875).
    let mut cutaway = Surface565::filled(cell_w, cell_h, COLOR_KEY_565);
    cutaway.blit_keyed(roofs_off, off_x, off_y);

    (normal, Some(cutaway))
}

/// Builds the native `[esi + 0x264]` badged surface (`0x54e58b..0x54e680`):
/// `roofs_on` (`BMP_ 513/1025`) at `(max(0, dx_extra), max(0, dy_extra))` with
/// `badge` centered inside `roofs_off`'s (`BMP_ 512/1024`) dimensions.
fn compose_vacation_badged_surface(
    roofs_off: &Surface565,
    roofs_on: &Surface565,
    metrics: ThmbMetrics,
    badge: &Surface565,
) -> Surface565 {
    let (cell_w, cell_h, _off_x, _off_y, on_x, on_y) =
        lot_cell_layout(roofs_off, roofs_on, metrics);
    let mut surface = Surface565::filled(cell_w, cell_h, COLOR_KEY_565);
    surface.blit_keyed(roofs_on, on_x, on_y);
    // 0x54e630..0x54e669: signed division by 2 of (roofs_off_dim - badge_dim).
    let bx = (roofs_off.width as i64 - badge.width as i64) / 2;
    let by = (roofs_off.height as i64 - badge.height as i64) / 2;
    surface.blit_keyed(badge, bx, by);
    surface
}

struct BuiltLotCells {
    metrics: ThmbMetrics,
    normal: Surface565,
    roofs_off: Option<Surface565>,
    vacation_badged: Option<Surface565>,
}

fn build_lot_cells(
    iff: &Iff,
    half_scale: bool,
    badge_sprite: Option<&Surface565>,
) -> Result<Option<BuiltLotCells>> {
    let decode_bmp_chunk = |id: u16| -> Result<Option<Surface565>> {
        let Some(chunk) = iff.get(b"BMP_", id) else {
            return Ok(None);
        };
        let img = bitmap::bmp(iff.data(chunk))?;
        Ok(Some(Surface565::from_bitmap(&img)))
    };

    let (metrics, roofs_off, roofs_on) = if half_scale {
        // 0x4e2676..0x4e288e: try 1024/1025 + THMB 1024 first; if absent, load
        // 512/513 + THMB 512, downsample by 0.5×, and halve THMB toward zero.
        if let Some(bmp_1024) = decode_bmp_chunk(1024)? {
            let bmp_1025 = decode_bmp_chunk(1025)?;
            let metrics = match iff.get(b"THMB", 1024) {
                Some(chunk) => ThmbMetrics::parse(iff.data(chunk))?,
                None => ThmbMetrics::CENTERED,
            };
            (metrics, bmp_1024, bmp_1025)
        } else {
            let Some(bmp_512) = decode_bmp_chunk(512)? else {
                return Ok(None);
            };
            let bmp_513 = decode_bmp_chunk(513)?;
            let half_512 = resample_half_565(&bmp_512);
            let half_513 = bmp_513.as_ref().map(resample_half_565);
            let metrics = match iff.get(b"THMB", 512) {
                Some(chunk) => ThmbMetrics::parse(iff.data(chunk))?.halved(),
                None => ThmbMetrics::CENTERED,
            };
            (metrics, half_512, half_513)
        }
    } else {
        let Some(bmp_512) = decode_bmp_chunk(512)? else {
            return Ok(None);
        };
        let bmp_513 = decode_bmp_chunk(513)?;
        let metrics = match iff.get(b"THMB", 512) {
            Some(chunk) => ThmbMetrics::parse(iff.data(chunk))?,
            None => ThmbMetrics::CENTERED,
        };
        (metrics, bmp_512, bmp_513)
    };

    let (normal, cutaway) = compose_lot_cells(&roofs_off, roofs_on.as_ref(), metrics);
    let vacation_badged = match (roofs_on.as_ref(), badge_sprite) {
        (Some(on), Some(badge)) => Some(compose_vacation_badged_surface(
            &roofs_off, on, metrics, badge,
        )),
        _ => None,
    };
    Ok(Some(BuiltLotCells {
        metrics,
        normal,
        roofs_off: cutaway,
        vacation_badged,
    }))
}

fn read_lot_filter_mask(iff: &Iff) -> u16 {
    let Some(chunk) = iff.get(b"SIMI", 1).or_else(|| iff.all(b"SIMI").next()) else {
        return 0;
    };
    let Ok(sim) = Simulator::parse(iff.data(chunk)) else {
        return 0;
    };
    // 0x427f46: SIMI globals[37] holds the lot's community amenity bitmask
    // (present on SIMI version >= 0x41 which stores 64 globals).
    sim.globals.get(37).copied().unwrap_or(0) as u16
}

#[derive(Default)]
struct FamilyOccupancy {
    primary_lot: BTreeMap<u16, (i32, Option<u16>)>,
    vacation_lot: BTreeMap<u16, i32>,
    /// Primary houses with at least one `FAMI` record whose 32-bit secondary
    /// house is nonzero (`0x54f20e`), accumulated across every `FAMI` chunk.
    primary_vacation_stays: BTreeSet<i32>,
    /// Every house ID referenced as primary or secondary by any `FAMI` record
    /// (`0x54f24f`), accumulated across every `FAMI` chunk.
    any_house_matches: BTreeSet<i32>,
}

impl FamilyOccupancy {
    fn record_family(&mut self, family_id: i32, family: &Family) {
        // 0x410602..0x410613: secondary/vacation house is serialized starting at version 8.
        let secondary_raw = if family.version >= 8 && family.tail.len() >= 4 {
            i32::from_le_bytes(family.tail[0..4].try_into().unwrap())
        } else {
            0
        };
        // 0x54f20e & 0x54f24f compare family.house and secondary_raw directly
        // against the lot ID without excluding lot 0.
        self.any_house_matches.insert(family.house);
        self.any_house_matches.insert(secondary_raw);
        if secondary_raw != 0 {
            self.primary_vacation_stays.insert(family.house);
        }
        let secondary_u16 = u16::try_from(secondary_raw).ok().filter(|&lot| lot > 0);
        if let Some(house) = u16::try_from(family.house).ok().filter(|&lot| lot > 0) {
            self.primary_lot.insert(house, (family_id, secondary_u16));
        }
        if let Some(vac_lot) = secondary_u16 {
            self.vacation_lot.insert(vac_lot, family_id);
        }
    }
}

fn load_family_occupancy(install: &Install, user_dir: &Path) -> Result<FamilyOccupancy> {
    let mut occ = FamilyOccupancy::default();
    let Some(path) = resolve_child(install, user_dir, "Neighborhood.iff") else {
        return Ok(occ);
    };
    let bytes =
        fs::read(&path).map_err(|e| Error::new(format!("reading {}: {e}", path.display())))?;
    let iff = Iff::parse(bytes)?;
    for chunk in iff.all(b"FAMI") {
        let Ok(family) = Family::parse(iff.data(chunk)) else {
            continue;
        };
        // 0x410673 / 0x4e1e73: native family ID is the FAMI chunk ID, not payload `number`.
        occ.record_family(i32::from(chunk.id), &family);
    }
    Ok(occ)
}

fn resolve_lot_status(
    lot_id: u16,
    kind: NeighborhoodKind,
    zoning: LotZoning,
    occ: &FamilyOccupancy,
) -> LotStatus {
    if kind == NeighborhoodKind::Vacation || zoning == LotZoning::Community {
        // 0x54f270..0x54f27e: Vacation occupancy matches primary OR secondary house.
        let occupied_by_family = occ
            .vacation_lot
            .get(&lot_id)
            .copied()
            .or_else(|| occ.primary_lot.get(&lot_id).map(|&(fid, _)| fid));
        LotStatus::Community { occupied_by_family }
    } else if let Some(&(family_id, vacation_lot)) = occ.primary_lot.get(&lot_id) {
        LotStatus::Occupied {
            family_id,
            vacation_lot,
        }
    } else {
        LotStatus::Vacant
    }
}

fn load_lot_locations(
    install: &Install,
    user_dir: &Path,
    str_id: u16,
) -> Result<BTreeMap<u16, [i32; 2]>> {
    let path = resolve_child(install, user_dir, "LotLocations.iff")
        .or_else(|| install.file("GameData/LotLocations.iff").ok())
        .ok_or_else(|| {
            Error::new(format!(
                "missing LotLocations.iff in {} or GameData",
                user_dir.display()
            ))
        })?;
    let bytes =
        fs::read(&path).map_err(|e| Error::new(format!("reading {}: {e}", path.display())))?;
    let iff = Iff::parse(bytes)?;
    let chunk = iff
        .get(b"STR#", str_id)
        .ok_or_else(|| Error::new(format!("{} missing STR# {str_id}", path.display())))?;
    let table = StringTable::parse_chunk(iff.data(chunk), iff.resource_flags(chunk))?;
    Ok(parse_lot_locations_table(&table))
}

/// Parses a `LotLocations.iff` `STR#` table (`"lot_id, x, y"` rows in US English),
/// matching `0x4e2501..0x4e257c`:
/// - stops each row at its first NUL byte,
/// - splits on the first two commas (rows with fewer than two commas are skipped),
/// - parses signed 32-bit `id`, `x`, and `y` via C `atoi` (`0x636029`),
/// - keeps the first matching row for each lot ID.
pub fn parse_lot_locations_table(table: &StringTable) -> BTreeMap<u16, [i32; 2]> {
    let mut map = BTreeMap::new();
    for entry in &table.entries {
        if entry.language != US_ENGLISH {
            continue;
        }
        let raw = if !entry.raw_text.is_empty() {
            entry.raw_text.as_slice()
        } else {
            entry.text.as_bytes()
        };
        let nul_end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
        let cstr = &raw[..nul_end];
        let Some(c1) = cstr.iter().position(|&b| b == b',') else {
            continue;
        };
        let rest1 = &cstr[c1 + 1..];
        let Some(c2) = rest1.iter().position(|&b| b == b',') else {
            continue;
        };
        let id = sims_content::atoi(&cstr[..c1]);
        let Ok(lot_id) = u16::try_from(id) else {
            continue;
        };
        let x = sims_content::atoi(&rest1[..c2]);
        let y = sims_content::atoi(&rest1[c2 + 1..]);
        map.entry(lot_id).or_insert([x, y]);
    }
    map
}

/// Parsed `LotZoning.iff` `STR# 1` table preserving every row in file order
/// with signed 32-bit lot IDs (`0x425016..0x425064`) and recording the first
/// matching zone per lot ID (`0x42b9fc`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LotZoningTable {
    pub rows: Vec<(i32, LotZoning)>,
    pub first_zone: BTreeMap<i32, LotZoning>,
}

fn load_lot_zoning(install: &Install, user_dir: &Path) -> Result<LotZoningTable> {
    // 0x424f1c..0x424f27: native reload opens the neighborhood's local
    // LotZoning.iff and fails rather than substituting GameData/MasterLotZoning.iff.
    let path = resolve_child(install, user_dir, "LotZoning.iff")
        .ok_or_else(|| Error::new(format!("missing LotZoning.iff in {}", user_dir.display())))?;
    let bytes =
        fs::read(&path).map_err(|e| Error::new(format!("reading {}: {e}", path.display())))?;
    let iff = Iff::parse(bytes)?;
    let chunk = iff
        .get(b"STR#", 1)
        .ok_or_else(|| Error::new(format!("{} missing STR# 1", path.display())))?;
    let table = StringTable::parse_chunk(iff.data(chunk), iff.resource_flags(chunk))?;
    Ok(parse_lot_zoning_table(&table))
}

/// Parses a `LotZoning.iff` `STR# 1` table (`"lot_id, residential|community"`),
/// following `0x424fb9..0x425064`:
/// - truncates each row at the first NUL byte and deterministically caps it at
///   99 bytes (native `strncpy(buf, s, 0x63)` at `0x424fb9..0x424fcb` copies at
///   most 99 bytes and may leave a full 99-byte copy unterminated; Rust always
///   bounds the slice at 99 bytes rather than reading past the buffer),
/// - splits at the first `,` (rows without a comma are skipped),
/// - parses the signed 32-bit decimal prefix before the comma via C `atoi`
///   (`0x636029`),
/// - searches the ASCII-lowercased bytes after the comma for `"residential"`
///   first (`0x6c25a4`), then `"community"` (`0x6c2598`), defaulting to
///   [`LotZoning::Residential`] (`0x425052..0x42505e`),
/// - preserves every row in file order (`0x425064`) and records the first
///   matching zone per lot ID (`0x42b9fc..0x42ba09`).
pub fn parse_lot_zoning_table(table: &StringTable) -> LotZoningTable {
    const NATIVE_BUF_LIMIT: usize = 99;
    let mut rows = Vec::new();
    let mut first_zone = BTreeMap::new();
    for entry in &table.entries {
        if entry.language != US_ENGLISH {
            continue;
        }
        let raw = if !entry.raw_text.is_empty() {
            entry.raw_text.as_slice()
        } else {
            entry.text.as_bytes()
        };
        // Stop at the first NUL byte and at 99 bytes (0x424fb9: push 0x63).
        let nul_end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
        let cstr = &raw[..nul_end.min(NATIVE_BUF_LIMIT)];
        let Some(comma) = cstr.iter().position(|&b| b == b',') else {
            continue;
        };
        let id = sims_content::atoi(&cstr[..comma]);
        let after_lower: Vec<u8> = cstr[comma + 1..]
            .iter()
            .map(u8::to_ascii_lowercase)
            .collect();
        let zoning = if after_lower.windows(11).any(|w| w == b"residential") {
            LotZoning::Residential
        } else if after_lower.windows(9).any(|w| w == b"community") {
            LotZoning::Community
        } else {
            LotZoning::Residential
        };
        rows.push((id, zoning));
        first_zone.entry(id).or_insert(zoning);
    }
    LotZoningTable { rows, first_zone }
}

fn resolve_child(install: &Install, dir: &Path, name: &str) -> Option<PathBuf> {
    if let Ok(rel_dir) = dir.strip_prefix(&install.root) {
        let mut rel = rel_dir.to_string_lossy().replace('\\', "/");
        if !rel.is_empty() && !rel.ends_with('/') {
            rel.push('/');
        }
        rel.push_str(name);
        if let Ok(found) = install.file(&rel) {
            return Some(found);
        }
    }
    let direct = dir.join(name);
    if direct.exists() {
        return Some(direct);
    }
    let lower = dir.join(name.to_ascii_lowercase());
    if lower.exists() {
        return Some(lower);
    }
    let upper = dir.join(name.to_ascii_uppercase());
    if upper.exists() {
        return Some(upper);
    }
    None
}

fn blit_surface_to_canvas(
    canvas: &mut Canvas,
    surface: &Surface565,
    at: [i32; 2],
    clip: [i32; 4],
    keyed: bool,
    lot_id: Option<u16>,
) {
    if !surface.is_valid() {
        return;
    }
    let cw = canvas.width as i64;
    let ch = canvas.height as i64;
    let clip_x0 = i64::from(clip[0]).clamp(0, cw);
    let clip_y0 = i64::from(clip[1]).clamp(0, ch);
    let clip_x1 = i64::from(clip[2]).clamp(clip_x0, cw);
    let clip_y1 = i64::from(clip[3]).clamp(clip_y0, ch);

    let ax = i64::from(at[0]);
    let ay = i64::from(at[1]);
    let x0 = ax.clamp(clip_x0, clip_x1);
    let y0 = ay.clamp(clip_y0, clip_y1);
    let x1 = ax.saturating_add(surface.width as i64).clamp(x0, clip_x1);
    let y1 = ay.saturating_add(surface.height as i64).clamp(y0, clip_y1);

    canvas.set_id(lot_id);
    for py in y0..y1 {
        let sy = (py - ay) as usize;
        let row = &surface.pixels[sy * surface.width..(sy + 1) * surface.width];
        for px in x0..x1 {
            let sx = (px - ax) as usize;
            let pixel = row[sx];
            if keyed && pixel == COLOR_KEY_565 {
                continue;
            }
            let rgb = widen_565(pixel);
            canvas.plot(px as i32, py as i32, 0, rgb);
        }
    }
}

fn mark_surface_ids(
    canvas: &mut Canvas,
    surface: &Surface565,
    at: [i32; 2],
    clip: [i32; 4],
    lot_id: u16,
) {
    if !surface.is_valid() {
        return;
    }
    let cw = canvas.width as i64;
    let ch = canvas.height as i64;
    let clip_x0 = i64::from(clip[0]).clamp(0, cw);
    let clip_y0 = i64::from(clip[1]).clamp(0, ch);
    let clip_x1 = i64::from(clip[2]).clamp(clip_x0, cw);
    let clip_y1 = i64::from(clip[3]).clamp(clip_y0, ch);

    let ax = i64::from(at[0]);
    let ay = i64::from(at[1]);
    let x0 = ax.clamp(clip_x0, clip_x1);
    let y0 = ay.clamp(clip_y0, clip_y1);
    let x1 = ax.saturating_add(surface.width as i64).clamp(x0, clip_x1);
    let y1 = ay.saturating_add(surface.height as i64).clamp(y0, clip_y1);

    canvas.set_id(Some(lot_id));
    for py in y0..y1 {
        let sy = (py - ay) as usize;
        let row = &surface.pixels[sy * surface.width..(sy + 1) * surface.width];
        let canvas_row = py as usize * canvas.width;
        for px in x0..x1 {
            let sx = (px - ax) as usize;
            if row[sx] == COLOR_KEY_565 {
                continue;
            }
            let existing = canvas.color[canvas_row + px as usize];
            canvas.plot(px as i32, py as i32, 0, existing);
        }
    }
}

fn blit_color_only(canvas: &mut Canvas, surface: &Surface565, at: [i32; 2], clip: [i32; 4]) {
    if !surface.is_valid() {
        return;
    }
    blit_subrect_to_canvas(
        canvas,
        surface,
        [0, 0, surface.width, surface.height],
        at,
        clip,
    );
}

fn blit_subrect_to_canvas(
    canvas: &mut Canvas,
    surface: &Surface565,
    src_rect: [usize; 4],
    at: [i32; 2],
    clip: [i32; 4],
) {
    let [rx, ry, rw, rh] = src_rect;
    if !surface.is_valid()
        || rx.saturating_add(rw) > surface.width
        || ry.saturating_add(rh) > surface.height
    {
        return;
    }
    let cw = canvas.width as i64;
    let ch = canvas.height as i64;
    let clip_x0 = i64::from(clip[0]).clamp(0, cw);
    let clip_y0 = i64::from(clip[1]).clamp(0, ch);
    let clip_x1 = i64::from(clip[2]).clamp(clip_x0, cw);
    let clip_y1 = i64::from(clip[3]).clamp(clip_y0, ch);

    let ax = i64::from(at[0]);
    let ay = i64::from(at[1]);
    let x0 = ax.clamp(clip_x0, clip_x1);
    let y0 = ay.clamp(clip_y0, clip_y1);
    let x1 = ax.saturating_add(rw as i64).clamp(x0, clip_x1);
    let y1 = ay.saturating_add(rh as i64).clamp(y0, clip_y1);

    for py in y0..y1 {
        let sy = ry + (py - ay) as usize;
        let row = &surface.pixels[sy * surface.width..(sy + 1) * surface.width];
        for px in x0..x1 {
            let sx = rx + (px - ax) as usize;
            let pixel = row[sx];
            if pixel == COLOR_KEY_565 {
                continue;
            }
            canvas.plot_color(px as i32, py as i32, widen_565(pixel));
        }
    }
}

fn draw_status_pip(canvas: &mut Canvas, center: [i32; 2], clip: [i32; 4], rgb: [u8; 3]) {
    let fill = quantize565(rgb);
    let border = [0x18, 0x18, 0x18];
    for dy in -3..=3 {
        let py = center[1].saturating_add(dy);
        if py < clip[1] || py >= clip[3] {
            continue;
        }
        for dx in -3..=3 {
            let dist2 = dx * dx + dy * dy;
            if dist2 > 9 {
                continue;
            }
            let px = center[0].saturating_add(dx);
            if px < clip[0] || px >= clip[2] {
                continue;
            }
            canvas.plot_color(px, py, if dist2 >= 5 { border } else { fill });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sims_formats::strings::{Format, TextEntry};

    fn make_lot(id: u16, origin: [i32; 2], surface: Surface565) -> MapLot {
        MapLot {
            id,
            anchor: origin,
            metrics: ThmbMetrics {
                anchor_x: 0,
                anchor_y: 0,
                anchor_y_extra: 0,
                dx_extra: 0,
                dy_extra: 0,
            },
            cell_origin: origin,
            zoning: LotZoning::Residential,
            status: LotStatus::Vacant,
            has_primary_vacation_stay: false,
            has_vacation_occupancy: false,
            filter_mask: 0,
            normal: surface,
            roofs_off: None,
            vacation_badged: None,
        }
    }

    fn make_test_map(kind: NeighborhoodKind, lots: Vec<MapLot>) -> NeighborhoodMap {
        NeighborhoodMap {
            kind,
            neighborhood: 1,
            non_us: false,
            backdrop: Surface565::filled(
                MAP_WIDTH as usize,
                MAP_HEIGHT as usize,
                pack_565([0, 64, 0]),
            ),
            large_frame: None,
            waves: Vec::new(),
            vacation_port: None,
            vacation_trees: None,
            vacation_logo: None,
            vacation_no_vacancy: None,
            filter_badges: BTreeMap::new(),
            lots,
        }
    }

    #[test]
    fn map_origin_centers_800x600_inside_1024x768() {
        assert_eq!(map_origin([800, 600]), [0, 0]);
        assert_eq!(map_origin([1024, 768]), [112, 84]);
        assert_eq!(map_origin([640, 480]), [0, 0]);
        assert_eq!(map_clip_rect([800, 600]), [0, 0, 800, 600]);
        assert_eq!(map_clip_rect([1024, 768]), [112, 84, 912, 684]);
    }

    #[test]
    fn thmb_parse_zero_pads_short_chunks_and_halves_toward_zero() {
        // 16-byte THMB leaves dy_extra = 0 (0x4e279f..0x4e27c6).
        let mut short = Vec::new();
        for v in [136i32, 131, -39, -3] {
            short.extend(v.to_le_bytes());
        }
        let parsed = ThmbMetrics::parse(&short).unwrap();
        assert_eq!(
            parsed,
            ThmbMetrics {
                anchor_x: 136,
                anchor_y: 131,
                anchor_y_extra: -39,
                dx_extra: -3,
                dy_extra: 0,
            }
        );

        let h = ThmbMetrics {
            dy_extra: 5,
            ..parsed
        }
        .halved();
        assert_eq!(h.anchor_x, 68);
        assert_eq!(h.anchor_y, 65);
        assert_eq!(h.anchor_y_extra, -19);
        assert_eq!(h.dx_extra, -1);
        assert_eq!(h.dy_extra, 2);
        // Present fallback THMB with sentinel X still halves all five fields (0x4e285a).
        assert_eq!(
            ThmbMetrics::CENTERED.halved().anchor_x,
            THMB_CENTER_SENTINEL / 2
        );
    }

    #[test]
    fn cell_origin_handles_negative_extra_and_center_sentinel() {
        let m = ThmbMetrics {
            anchor_x: 57,
            anchor_y: 37,
            anchor_y_extra: 1,
            dx_extra: 0,
            dy_extra: -1,
        };
        assert_eq!(compute_cell_origin([699, 413], m, [114, 74]), [642, 374]);
        assert_eq!(
            compute_cell_origin([400, 300], ThmbMetrics::CENTERED, [101, 51]),
            [350, 275]
        );
    }

    #[test]
    fn hostile_thmb_offsets_and_missing_thmb_sentinel_never_panic_or_bloat() {
        let off = Surface565::filled(4, 3, pack_565([10, 20, 30]));
        let on = Surface565::filled(6, 5, pack_565([40, 50, 60]));

        // dx_extra = i32::MIN previously panicked on `-dx` in debug builds.
        let min_metrics = ThmbMetrics {
            anchor_x: 0,
            anchor_y: 0,
            anchor_y_extra: 0,
            dx_extra: i32::MIN,
            dy_extra: i32::MIN,
        };
        let (normal, cutaway) = compose_lot_cells(&off, Some(&on), min_metrics);
        assert_eq!((normal.width, normal.height), (4, 3));
        assert!(cutaway.is_some());

        // Missing THMB installs THMB_CENTER_SENTINEL (0x7fffffff): roofs_on starts
        // off-surface (at x = 0x7fffffff) and roofs_off sits at (0, 0), bounded to
        // roofs_off's dimensions (matching 0x54e42e..0x54e8d6 without OOM).
        let (sent_normal, sent_cutaway) = compose_lot_cells(&off, Some(&on), ThmbMetrics::CENTERED);
        assert_eq!((sent_normal.width, sent_normal.height), (4, 3));
        assert_eq!(sent_normal.pixels, off.pixels);
        assert_eq!(sent_cutaway.unwrap().pixels, off.pixels);

        // Large positive offsets cannot allocate huge 2048×2048 buffers.
        let huge_metrics = ThmbMetrics {
            anchor_x: 0,
            anchor_y: 0,
            anchor_y_extra: 0,
            dx_extra: 50_000,
            dy_extra: 50_000,
        };
        let (bounded, _) = compose_lot_cells(&off, Some(&on), huge_metrics);
        assert_eq!((bounded.width, bounded.height), (4, 3));
    }

    #[test]
    fn surface565_and_oversized_viewport_never_overflow_or_panic() {
        // Overflowing Surface565::filled / from_bitmap / get
        let overflow_filled = Surface565::filled(usize::MAX, 2, 0);
        assert_eq!((overflow_filled.width, overflow_filled.height), (0, 0));

        let bad_bmp = BitmapImage {
            width: u32::MAX,
            height: u32::MAX,
            rgba: Vec::new(),
        };
        let from_bad = Surface565::from_bitmap(&bad_bmp);
        assert_eq!((from_bad.width, from_bad.height), (0, 0));

        let bad_surf = Surface565 {
            width: usize::MAX,
            height: 2,
            pixels: vec![0x1234],
        };
        assert!(!bad_surf.is_valid());
        assert_eq!(bad_surf.get(1, 1), None);
        assert_eq!(resample_half_565(&bad_surf).pixels.len(), 0);
        let _ = highlight_surface(&bad_surf, DEFAULT_HIGHLIGHT_RGB);

        // Oversized viewport never panics in render() and errors in try_render().
        let map = make_test_map(NeighborhoodKind::Classic, Vec::new());
        let huge_opts = RenderOptions {
            viewport: [u32::MAX, u32::MAX],
            ..RenderOptions::default()
        };
        assert!(map.try_render(&huge_opts).is_err());
        let empty = map.render(&huge_opts);
        assert_eq!((empty.width, empty.height), (0, 0));
    }

    #[test]
    fn resample_half_uses_ceil_and_preserves_colorkey_holes() {
        let mut src = Surface565::filled(5, 3, pack_565([120, 180, 60]));
        src.pixels[0] = COLOR_KEY_565;
        let half = resample_half_565(&src);
        assert_eq!((half.width, half.height), (3, 2));
        assert_ne!(half.pixels[half.width + 1], COLOR_KEY_565);

        let keyed = Surface565::filled(4, 4, COLOR_KEY_565);
        let half_keyed = resample_half_565(&keyed);
        assert!(half_keyed.pixels.iter().all(|&p| p == COLOR_KEY_565));

        let tiny = Surface565::filled(1, 1, pack_565([64, 128, 192]));
        let half_tiny = resample_half_565(&tiny);
        assert_eq!((half_tiny.width, half_tiny.height), (1, 1));
        assert_ne!(half_tiny.pixels[0], COLOR_KEY_565);
    }

    #[test]
    fn highlight_brightens_interior_and_outlines_horizontal_edges() {
        let mid = pack_565([80, 100, 120]);
        let mut surf = Surface565::filled(5, 1, COLOR_KEY_565);
        surf.pixels[1] = mid;
        surf.pixels[2] = mid;
        surf.pixels[3] = mid;

        let hi = highlight_surface(&surf, DEFAULT_HIGHLIGHT_RGB);
        assert_eq!(hi.pixels[0], COLOR_KEY_565);
        assert_eq!(hi.pixels[4], COLOR_KEY_565);
        assert_eq!(hi.pixels[3], pack_565(DEFAULT_HIGHLIGHT_RGB));
        assert_eq!(
            hi.pixels[1],
            brighten_565(pack_565(DEFAULT_HIGHLIGHT_RGB), 1.0, HIGHLIGHT_Y_OFFSET)
        );
        assert_eq!(hi.pixels[2], brighten_565(mid, 1.0, HIGHLIGHT_Y_OFFSET));
        assert!(widen_565(hi.pixels[2])[0] > widen_565(mid)[0]);
    }

    #[test]
    fn hit_test_matches_active_surface_and_allows_unclipped_silhouette() {
        let mut back = Surface565::filled(10, 10, pack_565([40, 80, 120]));
        back.pixels[0] = COLOR_KEY_565;

        let mut front = Surface565::filled(10, 10, COLOR_KEY_565);
        for i in 0..10 {
            front.pixels[5 * 10 + i] = pack_565([200, 60, 40]);
            front.pixels[i * 10 + 5] = pack_565([200, 60, 40]);
        }

        // Lot 3 overhangs past x = 800 (map right edge) to verify native
        // unclipped widget hit predicate (0x5e4071..0x5e40da).
        let overhang = Surface565::filled(10, 10, pack_565([90, 140, 180]));

        let map = make_test_map(
            NeighborhoodKind::Classic,
            vec![
                make_lot(1, [20, 20], back),
                make_lot(2, [20, 20], front),
                make_lot(3, [795, 50], overhang),
            ],
        );

        assert_eq!(map.hit_test([800, 600], [20, 20]), None);
        assert_eq!(map.hit_test([800, 600], [21, 21]), Some(1));
        assert_eq!(map.hit_test([800, 600], [25, 25]), Some(2));
        assert_eq!(map.hit_test([1024, 768], [25, 25]), None);
        assert_eq!(map.hit_test([1024, 768], [137, 109]), Some(2));
        // Overhanging pixel at 1024×768: 112 + 795 + 7 = 914 (> 912 map right edge).
        assert_eq!(map.hit_test([1024, 768], [914, 136]), Some(3));

        // Custom highlight RGB [255, 0, 255] turns a 1px isolated pixel into
        // COLOR_KEY_565: both Canvas::id_at and hit_test_with_options agree on None.
        let mut isolated = Surface565::filled(3, 1, COLOR_KEY_565);
        isolated.pixels[1] = 0x1234;
        let hi_map = make_test_map(
            NeighborhoodKind::Classic,
            vec![make_lot(1, [10, 10], isolated)],
        );
        let mut set = BTreeSet::new();
        set.insert(1);
        let opts = RenderOptions {
            viewport: [800, 600],
            highlighted_lots: set,
            highlight_rgb: [255, 0, 255],
            ..RenderOptions::default()
        };
        let canvas = hi_map.render(&opts);
        assert_eq!(canvas.id_at(11, 10), None);
        assert_eq!(hi_map.hit_test_with_options(&opts, [11, 10]), None);
        // Two-cell strip highlight boundary (0x54f347..0x54f3e1): when normal's
        // last pixel is opaque and cutaway's first pixel is COLOR_KEY_565, the
        // full-width scan turns normal's last pixel into outline_565 (which is
        // COLOR_KEY_565 for magenta highlight), so BOTH hit_test & Canvas::id_at
        // return None there.
        let mut two_cell_normal = Surface565::filled(2, 1, pack_565([40, 80, 120]));
        two_cell_normal.pixels[0] = pack_565([40, 80, 120]);
        let mut two_cell_cutaway = Surface565::filled(2, 1, pack_565([40, 80, 120]));
        two_cell_cutaway.pixels[0] = COLOR_KEY_565;
        let mut two_lot = make_lot(1, [10, 10], two_cell_normal);
        two_lot.roofs_off = Some(two_cell_cutaway);
        let two_map = make_test_map(NeighborhoodKind::Classic, vec![two_lot]);
        let two_canvas = two_map.render(&opts);
        assert_eq!(two_canvas.id_at(11, 10), None);
        assert_eq!(two_map.hit_test_with_options(&opts, [11, 10]), None);
    }

    #[test]
    fn zoning_parser_preserves_duplicates_first_zone_and_stop_order() {
        let entry = |text: &str| TextEntry {
            language: US_ENGLISH,
            text: text.to_string(),
            comment: String::new(),
            raw_text: text.as_bytes().to_vec(),
            raw_comment: Vec::new(),
        };
        let table = StringTable {
            format: Format::Localized,
            entries: vec![
                entry("1, residential"),
                entry("1, community"),
                entry("3, bogus_zone"),
                entry("4, community (park)"),
                entry("5, residential community"),
                entry("65536 # stop, residential"),
                entry("2, residential"),
            ],
        };
        let parsed = parse_lot_zoning_table(&table);
        assert_eq!(parsed.rows.len(), 7);
        assert_eq!(parsed.first_zone.get(&1), Some(&LotZoning::Residential));
        // Unrecognized zoning text defaults to Residential (0x425052..0x42505e).
        assert_eq!(parsed.first_zone.get(&3), Some(&LotZoning::Residential));
        // Substring match after first comma (0x425004, 0x42502f).
        assert_eq!(parsed.first_zone.get(&4), Some(&LotZoning::Community));
        assert_eq!(parsed.first_zone.get(&5), Some(&LotZoning::Residential));
        let unleashed: Vec<u16> = parsed
            .rows
            .iter()
            .take_while(|&&(id, _)| id <= 80)
            .filter_map(|&(id, _)| u16::try_from(id).ok())
            .collect();
        // C atoi parses "65536 # stop" as 65536 and stops before lot 2 (0x557125).
        assert_eq!(unleashed, vec![1, 1, 3, 4, 5]);

        // NUL termination and 99-byte native buffer limit (0x424fb9..0x424fcb).
        let nul_entry = entry("6, neutral\0community");
        let mut long_text = String::from("7, ");
        long_text.push_str(&"x".repeat(96)); // 3 + 96 = 99 bytes before "community"
        long_text.push_str("community");
        let long_entry = entry(&long_text);
        let cstr_table = StringTable {
            format: Format::Pascal,
            entries: vec![nul_entry, long_entry],
        };
        let cstr_parsed = parse_lot_zoning_table(&cstr_table);
        assert_eq!(
            cstr_parsed.first_zone.get(&6),
            Some(&LotZoning::Residential)
        );
        assert_eq!(
            cstr_parsed.first_zone.get(&7),
            Some(&LotZoning::Residential)
        );

        // LotLocations parser accepts C atoi prefixes, NUL termination, and keeps
        // the first matching row for each lot ID (0x4e2501..0x4e257c).
        let loc_table = StringTable {
            format: Format::Pascal,
            entries: vec![
                entry("5 # lot, 300 # x, 120 # y"),
                entry("7, 100, 200"),
                entry("7, 999, 888"),
                entry("0, 50, 60\0, 999"),
                entry("8, only_one_comma"),
            ],
        };
        let locs = parse_lot_locations_table(&loc_table);
        assert_eq!(locs.get(&5), Some(&[300, 120]));
        assert_eq!(locs.get(&7), Some(&[100, 200]));
        assert_eq!(locs.get(&0), Some(&[50, 60]));
        assert_eq!(locs.get(&8), None);
    }

    #[test]
    fn filter_badge_animation_offsets_and_vacation_surface_match_native() {
        assert_eq!(
            (0..6).map(filter_badge_cell).collect::<Vec<_>>(),
            vec![0, 1, 2, 2, 1, 0]
        );
        assert_eq!(
            NeighborhoodKind::Downtown.filter_badge_offset(23),
            [-24, -10]
        );
        assert_eq!(
            NeighborhoodKind::Downtown.filter_badge_offset(24),
            [-24, -70]
        );
        assert_eq!(
            NeighborhoodKind::Vacation.filter_badge_offset(40),
            [-24, -10]
        );
        assert_eq!(
            NeighborhoodKind::Vacation.filter_badge_offset(42),
            [-24, -10]
        );
        assert_eq!(
            NeighborhoodKind::Vacation.filter_badge_offset(43),
            [-24, -70]
        );
        assert_eq!(
            NeighborhoodKind::StudioTown.filter_badge_offset(83),
            [-24, -10]
        );
        assert_eq!(
            NeighborhoodKind::StudioTown.filter_badge_offset(84),
            [-24, -70]
        );

        assert!(NeighborhoodKind::Unleashed.supports_filter(CommunityFilter::Park));
        assert!(!NeighborhoodKind::Unleashed.supports_filter(CommunityFilter::Spa));
        assert!(NeighborhoodKind::StudioTown.supports_filter(CommunityFilter::Music));
        assert!(!NeighborhoodKind::StudioTown.supports_filter(CommunityFilter::Magic));
        assert!(NeighborhoodKind::MagicTown.supports_filter(CommunityFilter::Rides));
    }

    #[test]
    fn vacation_badge_surface_centers_on_roofs_off_and_fami_v8_occupancy_matches() {
        // roofs_off = 20×10, roofs_on = 30×10, dx_extra = 4 => combined 34×10.
        // Badge (4×2) centers on roofs_off (20×10): bx = (20-4)/2 = 8, by = (10-2)/2 = 4.
        let off = Surface565::filled(20, 10, pack_565([10, 20, 30]));
        let on = Surface565::filled(30, 10, pack_565([40, 80, 120]));
        let badge_c = pack_565([240, 16, 16]);
        let badge = Surface565::filled(4, 2, badge_c);
        let metrics = ThmbMetrics {
            anchor_x: 0,
            anchor_y: 0,
            anchor_y_extra: 0,
            dx_extra: 4,
            dy_extra: 0,
        };
        let badged = compose_vacation_badged_surface(&off, &on, metrics, &badge);
        assert_eq!((badged.width, badged.height), (34, 10));
        assert_eq!(badged.get(8, 4), Some(badge_c));
        assert_eq!(badged.get(11, 5), Some(badge_c));
        assert_ne!(badged.get(7, 4), Some(badge_c));

        // Occupancy: primary house 40 matches Vacation lot 40 (0x54f270..0x54f27e),
        // and secondary house 41 matches Vacation lot 41.
        let mut occ = FamilyOccupancy::default();
        occ.primary_lot.insert(40, (9, None));
        occ.primary_lot.insert(5, (5, Some(41)));
        occ.vacation_lot.insert(41, 5);
        assert_eq!(
            resolve_lot_status(40, NeighborhoodKind::Vacation, LotZoning::Community, &occ),
            LotStatus::Community {
                occupied_by_family: Some(9)
            }
        );
        assert_eq!(
            resolve_lot_status(41, NeighborhoodKind::Vacation, LotZoning::Community, &occ),
            LotStatus::Community {
                occupied_by_family: Some(5)
            }
        );
        assert_eq!(
            resolve_lot_status(5, NeighborhoodKind::Unleashed, LotZoning::Residential, &occ),
            LotStatus::Occupied {
                family_id: 5,
                vacation_lot: Some(41),
            }
        );

        // Verify vacation badge draws unhighlighted (0x54ec5a) while picking
        // tests the regular thumbnail (0x5e40b4), and works on Magic Town homes (0x54eb49).
        let mut cutaway = Surface565::filled(34, 10, COLOR_KEY_565);
        cutaway.pixels[0] = pack_565([20, 40, 60]); // only (0,0) opaque in cutaway
        let mut lot = make_lot(90, [10, 10], on.clone());
        lot.roofs_off = Some(cutaway);
        lot.vacation_badged = Some(badged.clone());
        lot.zoning = LotZoning::Community;
        lot.status = LotStatus::Community {
            occupied_by_family: None,
        };
        // Even under Community zoning, primary vacation stay triggers the badge (0x54f20e).
        lot.has_primary_vacation_stay = true;
        let magic_map = make_test_map(NeighborhoodKind::MagicTown, vec![lot]);
        let hover_opts = RenderOptions {
            hovered_lot: Some(90),
            hover_roofs_off: true,
            highlight_hovered: true,
            show_vacation_badges: true,
            ..RenderOptions::default()
        };
        let canvas = magic_map.render(&hover_opts);
        // Badge pixel at (10+8, 10+4) renders at its exact unhighlighted color.
        assert_eq!(canvas.color[14 * 800 + 18], widen_565(badge_c));
        // Picking tests the regular cutaway thumbnail: (10+0, 10+0) hits,
        // while (10+8, 10+4) (transparent in cutaway) misses on BOTH hit_test & Canvas::id_at.
        assert_eq!(
            magic_map.hit_test_with_options(&hover_opts, [10, 10]),
            Some(90)
        );
        assert_eq!(canvas.id_at(10, 10), Some(90));
        assert_eq!(magic_map.hit_test_with_options(&hover_opts, [18, 14]), None);
        assert_eq!(canvas.id_at(18, 14), None);

        // Multiple FAMI records sharing lot 5 (secondary 41 followed by 0) and
        // a single record with negative nonzero secondary (-1) both preserve
        // primary vacation stay eligibility (0x54f20e).
        let make_fam = |house: i32, secondary: i32| Family {
            version: 8,
            house,
            number: 1,
            budget: 0,
            architecture_value: 0,
            friends: 0,
            flags: 0,
            members: Vec::new(),
            tail: secondary.to_le_bytes().to_vec(),
        };
        let mut multi_occ = FamilyOccupancy::default();
        multi_occ.record_family(1, &make_fam(5, 41));
        multi_occ.record_family(2, &make_fam(5, 0));
        multi_occ.record_family(3, &make_fam(6, -1));
        multi_occ.record_family(4, &make_fam(0, 41));
        assert!(multi_occ.primary_vacation_stays.contains(&5));
        assert!(multi_occ.primary_vacation_stays.contains(&6));
        assert!(multi_occ.primary_vacation_stays.contains(&0));
        assert!(multi_occ.any_house_matches.contains(&41));
    }
}
