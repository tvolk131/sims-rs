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
use sims_formats::iff::DELETED as IFF_DELETED;
use sims_formats::strings::{Format as StringTableFormat, StringTable, TextEntry};
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
    /// Lot number (stock maps use `1..=98`; custom Unleashed `LotZoning.iff`
    /// rows can include `0`).
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
    backdrop_size: [u32; 2],
}

impl MapLot {
    /// Width and height of the lot's sprite cell.
    pub fn size(&self) -> [u32; 2] {
        [self.normal.width as u32, self.normal.height as u32]
    }

    /// Screen-space bounding rectangle `[left, top, right, bottom]` inside a
    /// viewport of size `viewport`, using the backdrop dimensions captured when
    /// the map was loaded (`0x568fa7..0x568fe6`). If [`NeighborhoodMap::backdrop`]
    /// is replaced after loading, use [`NeighborhoodMap::lot_screen_rect`].
    pub fn screen_rect(&self, viewport: [u32; 2]) -> [i32; 4] {
        self.screen_rect_at(map_origin_for_backdrop(viewport, self.backdrop_size))
    }

    fn screen_rect_at(&self, origin: [i32; 2]) -> [i32; 4] {
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
    /// Numbered water wave animation slots (`None` when a slot is absent, `0x56bb7a..0x56bb8d`).
    pub waves: Vec<Option<Surface565>>,
    /// True when the backdrop and companion wave/port/tree overlays were loaded
    /// from neighborhood-local JPEGs (`0x4b8285`), which leave source color-keying
    /// disabled rather than treating `0xf81f` magenta as transparent.
    pub opaque_overlays: bool,
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
        let load_ui_opt = |id: i32| -> Option<Surface565> {
            ui.bitmap(id, US_ENGLISH)
                .ok()
                .flatten()
                .map(|img| Surface565::from_bitmap(&img))
        };

        let local_art = load_local_map_artwork(install, user_dir, kind);
        let (backdrop, waves, vacation_port, vacation_trees, opaque_overlays) =
            if let Some(local) = local_art {
                (
                    local.backdrop,
                    local.waves,
                    local.vacation_port,
                    local.vacation_trees,
                    true,
                )
            } else {
                let backdrop = ui
                    .bitmap(kind.backdrop_id(non_us), US_ENGLISH)?
                    .map(|img| Surface565::from_bitmap(&img))
                    .ok_or_else(|| {
                        Error::new(format!(
                            "missing neighborhood backdrop resource {}",
                            kind.backdrop_id(non_us)
                        ))
                    })?;
                // Missing or unreadable bundled wave slots stay `None` and are
                // skipped at draw time rather than aborting map load (0x56bb7a).
                let waves = kind
                    .wave_ids(non_us)
                    .iter()
                    .map(|&wave_id| load_ui_opt(wave_id))
                    .collect();
                let (port, trees) = if kind == NeighborhoodKind::Vacation {
                    (
                        load_ui_opt(res::VACATION_PORT),
                        load_ui_opt(res::VACATION_TREES),
                    )
                } else {
                    (None, None)
                };
                (backdrop, waves, port, trees, false)
            };
        let large_frame = load_ui_opt(kind.large_frame_id());
        let vacation_logo = load_ui_opt(res::VACATION_LOGO);
        let vacation_no_vacancy = if kind == NeighborhoodKind::Vacation {
            load_ui_opt(res::VACATION_NO_VACANCY)
        } else {
            None
        };

        let mut filter_badges = BTreeMap::new();
        for filter in CommunityFilter::ALL {
            if kind.supports_filter(filter)
                && let Some(surf) = load_ui_opt(filter.badge_id())
            {
                filter_badges.insert(filter, surf);
            }
        }

        let locations = load_lot_locations(install, user_dir);
        let zoning_table = load_lot_zoning(install, user_dir);
        let occupancy = load_family_occupancy(install, user_dir);

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

        let houses_dir = resolve_child(install, user_dir, "Houses");

        let badge_sprite = if kind == NeighborhoodKind::Vacation {
            vacation_no_vacancy.as_ref()
        } else {
            vacation_logo.as_ref()
        };

        let mut lots = Vec::with_capacity(lot_ids.len());
        if let Some(houses_dir) = houses_dir {
            for lot_id in lot_ids {
                // 0x4e20a9..0x4e257c: dispatch by lot ID across STR# 1/2/3/4/5/7,
                // defaulting to [0, 0] when the table or row is absent.
                let anchor = locations.anchor_for(lot_id, kind.uses_half_scale());
                let house_name = format!("House{lot_id:02}.iff");
                let Some(house_path) = resolve_child(install, &houses_dir, &house_name) else {
                    continue;
                };
                // 0x4e1e04..0x4e1e0f: an unreadable or invalid HouseNN.iff exits
                // only this lot's load and lets the map continue with remaining lots.
                let Some(iff) = open_iff(&house_path) else {
                    continue;
                };

                // 0x427f21 / 0x464e6d / 0x4e1e58..0x4e1e62: SIMI 1 is required
                // for a lot to load; missing or unreadable SIMI 1 skips the lot.
                let Some(filter_mask) = read_lot_filter_mask(&iff) else {
                    continue;
                };
                let Ok(Some(BuiltLotCells {
                    metrics,
                    normal,
                    roofs_off,
                    vacation_badged,
                })) = build_lot_cells(&iff, kind.uses_half_scale(), badge_sprite)
                else {
                    continue;
                };
                let zoning = if lot_id == 0 {
                    // 0x42b9e3..0x42b9ee: GetLotZoning(0) always returns
                    // Residential (1), overriding any `0, community` row.
                    LotZoning::Residential
                } else {
                    zoning_table
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
                        })
                };
                let status = resolve_lot_status(lot_id, kind, zoning, &occupancy);
                let has_primary_vacation_stay = occupancy.has_primary_vacation_stay(lot_id);
                let has_vacation_occupancy = occupancy.has_vacation_occupancy(lot_id);
                let cell_origin =
                    compute_cell_origin(anchor, metrics, [normal.width, normal.height]);

                let backdrop_size = [backdrop.width as u32, backdrop.height as u32];
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
                    backdrop_size,
                });
            }
        }

        Ok(Self {
            kind,
            neighborhood,
            non_us,
            backdrop,
            large_frame,
            waves,
            opaque_overlays,
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

    /// Top-left map origin in `viewport` using this map's decoded backdrop
    /// dimensions (`0x568fa7..0x568fe6`). Matches [`map_origin`] for standard
    /// `800×600` backdrops.
    pub fn map_origin_for(&self, viewport: [u32; 2]) -> [i32; 2] {
        map_origin_for_backdrop(
            viewport,
            [self.backdrop.width as u32, self.backdrop.height as u32],
        )
    }

    /// Screen-space bounding rectangle `[left, top, right, bottom]` of `lot`
    /// inside `viewport`, using this map's backdrop-derived origin.
    pub fn lot_screen_rect(&self, lot: &MapLot, viewport: [u32; 2]) -> [i32; 4] {
        lot.screen_rect_at(self.map_origin_for(viewport))
    }

    /// Lot clip rectangle (`[ox, oy, ox + 800, oy + 600]` clamped to `viewport`),
    /// matching native `cWinLot` clip initialization at `0x5571e4..0x557201`
    /// and `0x56928b..0x56929a`: always `800×600` from the centered backdrop origin.
    fn lot_clip_for(&self, viewport: [u32; 2]) -> [i32; 4] {
        let [ox, oy] = self.map_origin_for(viewport);
        let vw = viewport[0].min(i32::MAX as u32) as i32;
        let vh = viewport[1].min(i32::MAX as u32) as i32;
        [
            ox,
            oy,
            ox.saturating_add(MAP_WIDTH as i32).min(vw),
            oy.saturating_add(MAP_HEIGHT as i32).min(vh),
        ]
    }

    /// Centered backdrop clip rectangle (`[ox, oy, ox + bw, oy + bh]` clamped
    /// to `viewport`), matching native backdrop/wave destination bounds
    /// (`0x55f95f..0x55f98a`).
    fn backdrop_clip_for(&self, viewport: [u32; 2]) -> [i32; 4] {
        let [ox, oy] = self.map_origin_for(viewport);
        let bw = self.backdrop.width.min(i32::MAX as usize) as i32;
        let bh = self.backdrop.height.min(i32::MAX as usize) as i32;
        let vw = viewport[0].min(i32::MAX as u32) as i32;
        let vh = viewport[1].min(i32::MAX as u32) as i32;
        [
            ox,
            oy,
            ox.saturating_add(bw).min(vw),
            oy.saturating_add(bh).min(vh),
        ]
    }

    /// Resolves the active water wave surface for `frame_idx`, matching each
    /// neighborhood's native wave playback logic:
    /// - **Downtown (`0x54caf5`) & Vacation (`0x56baa5`)**: 10-step ping-pong
    ///   over slots `0..=4` (`0,1,2,3,4,4,3,2,1,0`); slot `5` is never selected.
    /// - **Unleashed (`0x557066`, `0x557e87`)**: cycles through slots `0..len`
    ///   and skips widget updates when a slot is missing, keeping the most
    ///   recently loaded wave visible.
    /// - **Classic (`0x55f91a..0x55f92f`) & Magic Town (`0x554ff6`)**: direct
    ///   modulo indexing over `self.waves`, drawing nothing if the slot is empty.
    fn wave_for_frame(&self, frame_idx: usize) -> Option<&Surface565> {
        let len = self.waves.len();
        if len == 0 {
            return None;
        }
        match self.kind {
            NeighborhoodKind::Downtown | NeighborhoodKind::Vacation => {
                let step = frame_idx % 10;
                let slot = if step < 5 { step } else { 9 - step };
                self.waves.get(slot).and_then(Option::as_ref)
            }
            NeighborhoodKind::Unleashed => {
                let start = frame_idx.saturating_sub(len - 1);
                for step in (start..=frame_idx).rev() {
                    if let Some(wave) = &self.waves[step % len] {
                        return Some(wave);
                    }
                }
                None
            }
            _ => self.waves[frame_idx % len].as_ref(),
        }
    }

    /// Renders the neighborhood map screen into an existing [`Canvas`].
    ///
    /// Sets `canvas.set_id(Some(lot.id))` for each lot's opaque pixels so
    /// [`Canvas::id_at`] agrees with [`Self::hit_test_with_options`] inside
    /// the clipped `800×600` lot rectangle from the centered backdrop origin.
    pub fn render_into(&self, canvas: &mut Canvas, options: &RenderOptions) {
        let viewport = [canvas.width as u32, canvas.height as u32];
        let origin = self.map_origin_for(viewport);
        let full_clip = [0, 0, canvas.width as i32, canvas.height as i32];
        let backdrop_clip = self.backdrop_clip_for(viewport);
        let map_clip = self.lot_clip_for(viewport);
        let overlay_keyed = !self.opaque_overlays;

        canvas.clear([0, 0, 0]);
        canvas.set_id(None);

        // 1. 1024×768 outer frame only when viewport width exceeds 800
        //    (`cmp eax, 0x320; jle` at 0x556d89, 0x569025).
        if viewport[0] > MAP_WIDTH
            && let Some(frame) = &self.large_frame
        {
            blit_surface_to_canvas(canvas, frame, [0, 0], full_clip, false, None);
        }

        // 2. Map backdrop at centered origin (0x556db7..0x556ddc, 0x568fa7..0x568fe6).
        blit_surface_to_canvas(canvas, &self.backdrop, origin, backdrop_clip, false, None);

        // 3. Pre-lot water wave overlay:
        //    - Classic (0x55f95f..0x55f98a -> cRZGraphic::Blt 0x5def36 ->
        //      DirectDraw Blt 0x605091) crops dst & src 1:1 against the canvas
        //      and stretches the remaining source rect into the clipped backdrop rect.
        //    - Unleashed (0x5570c1, 0x557e87) blits 1:1 at origin with a null clip,
        //      retaining the last valid frame if a custom slot is absent.
        if !self.kind.waves_draw_after_lots()
            && let Some(frame_idx) = options.water_frame
            && let Some(wave) = self.wave_for_frame(frame_idx)
        {
            if self.kind == NeighborhoodKind::Classic {
                blit_surface_stretched_to_canvas(
                    canvas,
                    wave,
                    origin,
                    [self.backdrop.width, self.backdrop.height],
                    full_clip,
                    overlay_keyed,
                );
            } else {
                let [wx, wy] = self.kind.wave_offset();
                let at = [origin[0].saturating_add(wx), origin[1].saturating_add(wy)];
                blit_surface_to_canvas(canvas, wave, at, full_clip, overlay_keyed, None);
            }
        }

        // 4. Lots in back-to-front Z-order, clipped strictly to the 800×600 rect
        //    at `origin` (cWinLot + 0x230..0x240, 0x5571e4..0x557201, 0x56928b..0x56929a).
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
        //    at 0x56bac8..0x56bb5d: drawn only when BOTH port and trees are
        //    present, using each overlay's own dimensions with a null clip
        //    (0x56baf4, 0x56bb29).
        if options.show_vacation_overlays
            && self.kind == NeighborhoodKind::Vacation
            && let (Some(port), Some(trees)) = (&self.vacation_port, &self.vacation_trees)
        {
            blit_color_only_mode(canvas, port, origin, full_clip, overlay_keyed);
            blit_color_only_mode(canvas, trees, origin, full_clip, overlay_keyed);
        }

        // 6. Post-lot water wave overlay for Downtown (0x54caf5..0x54cb3e,
        //    ping-pong over slots 0..=4, null clip), Vacation (0x56baa5..0x56bb92,
        //    ping-pong over slots 0..=4, null clip), and Magic Town (0x554ff6 at (0, 62)).
        if self.kind.waves_draw_after_lots()
            && let Some(frame_idx) = options.water_frame
            && let Some(wave) = self.wave_for_frame(frame_idx)
        {
            let [wx, wy] = self.kind.wave_offset();
            let at = [origin[0].saturating_add(wx), origin[1].saturating_add(wy)];
            blit_color_only_mode(canvas, wave, at, full_clip, overlay_keyed);
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
                    // Native filter badge blits pass a null clip (e.g. 0x56bec5),
                    // allowing badges on edge lots to extend into the outer frame.
                    blit_subrect_to_canvas(
                        canvas,
                        strip,
                        [src_x, 0, cell_w, strip.height],
                        [bx, by],
                        full_clip,
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
        let origin = self.map_origin_for(options.viewport);
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
        // 0x54eb49..0x54eb64: primary home with vacation stay (0x54f20e) on any
        // map, or secondary/primary occupancy (0x54f24f) only on Vacation Island.
        lot.has_primary_vacation_stay
            || (self.kind == NeighborhoodKind::Vacation && lot.has_vacation_occupancy)
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

/// Computes the centered top-left origin for a backdrop of size `backdrop_size`
/// inside `viewport` (`0x568fa7..0x568fe6`).
pub fn map_origin_for_backdrop(viewport: [u32; 2], backdrop_size: [u32; 2]) -> [i32; 2] {
    let ox = if viewport[0] > backdrop_size[0] {
        ((viewport[0] - backdrop_size[0]) / 2) as i32
    } else {
        0
    };
    let oy = if viewport[1] > backdrop_size[1] {
        ((viewport[1] - backdrop_size[1]) / 2) as i32
    } else {
        0
    };
    [ox, oy]
}

/// Computes the top-left origin of the stock `800×600` map inside `viewport`
/// (`0x556db7..0x556ddc`): `[0, 0]` at `800×600`, `[112, 84]` at `1024×768`.
pub fn map_origin(viewport: [u32; 2]) -> [i32; 2] {
    map_origin_for_backdrop(viewport, [MAP_WIDTH, MAP_HEIGHT])
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
    iff: &NativeIff,
    half_scale: bool,
    badge_sprite: Option<&Surface565>,
) -> Result<Option<BuiltLotCells>> {
    // 0x4ded62..0x4ded72: a missing or corrupt BMP_ resource returns null rather
    // than aborting the lot, so optional roofs-on bitmaps (513/1025) degrade to
    // a single roofs-off cell (0x54e685..0x54e6a7) and corrupt 1024 falls back
    // to 512 (0x4e2692..0x4e26de).
    let decode_bmp_chunk = |id: u16| -> Option<Surface565> {
        let (data, _flags) = iff.get(b"BMP_", id)?;
        let img = bitmap::bmp(data).ok()?;
        Some(Surface565::from_bitmap(&img))
    };

    let (metrics, roofs_off, roofs_on) = if half_scale {
        // 0x4e2676..0x4e288e: try 1024/1025 + THMB 1024 first; if absent, load
        // 512/513 + THMB 512, downsample by 0.5×, and halve THMB toward zero.
        if let Some(bmp_1024) = decode_bmp_chunk(1024) {
            let bmp_1025 = decode_bmp_chunk(1025);
            let metrics = match iff.get(b"THMB", 1024) {
                Some((data, _)) => ThmbMetrics::parse(data)?,
                None => ThmbMetrics::CENTERED,
            };
            (metrics, bmp_1024, bmp_1025)
        } else {
            let Some(bmp_512) = decode_bmp_chunk(512) else {
                return Ok(None);
            };
            let bmp_513 = decode_bmp_chunk(513);
            let half_512 = resample_half_565(&bmp_512);
            let half_513 = bmp_513.as_ref().map(resample_half_565);
            let metrics = match iff.get(b"THMB", 512) {
                Some((data, _)) => ThmbMetrics::parse(data)?.halved(),
                None => ThmbMetrics::CENTERED,
            };
            (metrics, half_512, half_513)
        }
    } else {
        let Some(bmp_512) = decode_bmp_chunk(512) else {
            return Ok(None);
        };
        let bmp_513 = decode_bmp_chunk(513);
        let metrics = match iff.get(b"THMB", 512) {
            Some((data, _)) => ThmbMetrics::parse(data)?,
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

/// Native serialized chunk stream matching `0x464f49` + `0x464ec1` + `0x464885`:
/// - when `flags & 0x10 == 0` (and `run_swap_callback` is true, as in `0x464e7a`),
///   native `0x464f49` sets `byte[0] = 1` and byte-swaps `version` (`bytes[4..8]`),
/// - otherwise `version` is little-endian and `big_endian = bytes[0] != 0`,
/// - stream reads start at byte 12 without inspecting `bytes[8..12]`.
struct NativeStream<'a> {
    bytes: &'a [u8],
    pos: usize,
    big_endian: bool,
}

impl<'a> NativeStream<'a> {
    fn open(bytes: &'a [u8], flags: u16, run_swap_callback: bool) -> Option<(Self, i32)> {
        if bytes.len() < 12 {
            return None;
        }
        let swapped_header = run_swap_callback && (flags & 0x10) == 0;
        let version = if swapped_header {
            i32::from_be_bytes(bytes[4..8].try_into().unwrap())
        } else {
            i32::from_le_bytes(bytes[4..8].try_into().unwrap())
        };
        let big_endian = swapped_header || bytes[0] != 0;
        Some((
            Self {
                bytes,
                pos: 12,
                big_endian,
            },
            version,
        ))
    }

    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.pos)
    }

    fn u8(&mut self) -> Option<u8> {
        let &b = self.bytes.get(self.pos)?;
        self.pos += 1;
        Some(b)
    }

    fn i16(&mut self) -> Option<i16> {
        let b = self.bytes.get(self.pos..self.pos + 2)?;
        self.pos += 2;
        Some(if self.big_endian {
            i16::from_be_bytes([b[0], b[1]])
        } else {
            i16::from_le_bytes([b[0], b[1]])
        })
    }

    fn i32(&mut self) -> Option<i32> {
        let b = self.bytes.get(self.pos..self.pos + 4)?;
        self.pos += 4;
        Some(if self.big_endian {
            i32::from_be_bytes([b[0], b[1], b[2], b[3]])
        } else {
            i32::from_le_bytes([b[0], b[1], b[2], b[3]])
        })
    }

    fn skip_cstr_padded(&mut self) -> Option<()> {
        let rel_nul = self.bytes[self.pos..].iter().position(|&b| b == 0)?;
        self.pos += rel_nul + 1;
        if (self.pos & 1) != 0 && self.pos < self.bytes.len() {
            self.pos += 1;
        }
        Some(())
    }
}

fn read_lot_filter_mask(iff: &NativeIff) -> Option<u16> {
    // 0x427f21 / 0x464e6d / 0x4e1e58..0x4e1e62: requires SIMI 1 to be present;
    // once found, 0x464ec1 opens the stream at byte 12 using byte[0] (or
    // 0x464f49 when flags & 0x10 == 0) for byte order without checking `IMIS`,
    // and 0x46cfb3 reads 64 globals for any version >= 0x41 (globals[37] at 0x427f46).
    let (data, flags) = iff.get(b"SIMI", 1)?;
    let Some((mut s, version)) = NativeStream::open(data, flags, true) else {
        return Some(0);
    };
    if version < 0x41 {
        return Some(0);
    }
    s.pos = s.pos.saturating_add(37 * 2);
    Some(s.i16().unwrap_or(0) as u16)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FamilyRecord {
    id: i32,
    number: i32,
    house: i32,
    secondary_house: i32,
}

#[derive(Default)]
struct FamilyOccupancy {
    /// Variant-0 `FAMI` records plus default family `0` when absent (`0x424c65..0x424cb5`),
    /// sorted descending by `(number, id)` matching `0x425209` and scanned by `0x425d95`.
    families: Vec<FamilyRecord>,
}

impl FamilyOccupancy {
    fn record_family_houses(
        &mut self,
        family_id: i32,
        number: i32,
        house: i32,
        secondary_house: i32,
    ) {
        self.families.push(FamilyRecord {
            id: family_id,
            number,
            house,
            secondary_house,
        });
    }

    /// Synthesizes default family `0` when absent (`0x424c65..0x424cb5`, with
    /// default `number = -1` from `0x410083`) and sorts the list descending by
    /// `(number, id)` (`0x425209`).
    fn finalize_native(&mut self) {
        if !self.families.iter().any(|f| f.id == 0) {
            self.families.push(FamilyRecord {
                id: 0,
                number: -1,
                house: 0,
                secondary_house: 0,
            });
        }
        self.families
            .sort_by(|a, b| b.number.cmp(&a.number).then_with(|| b.id.cmp(&a.id)));
    }

    /// Native `GetFamilyByLot` (`0x425d95..0x425de0`):
    /// - for lot `0`, selects the first family whose `id == 0`,
    /// - for any other lot, selects the first family whose primary or secondary
    ///   house equals `lot_id`.
    fn occupant_for_lot(&self, lot_id: u16) -> Option<&FamilyRecord> {
        let lot = i32::from(lot_id);
        if lot == 0 {
            self.families.iter().find(|f| f.id == 0)
        } else {
            self.families
                .iter()
                .find(|f| f.house == lot || f.secondary_house == lot)
        }
    }

    /// Native `0x54f20e`: true if any family has `primary_house == lot_id` and
    /// nonzero 32-bit `secondary_house`.
    fn has_primary_vacation_stay(&self, lot_id: u16) -> bool {
        let lot = i32::from(lot_id);
        self.families
            .iter()
            .any(|f| f.house == lot && f.secondary_house != 0)
    }

    /// Native `0x54f24f`: true if any family references `lot_id` as its primary
    /// or secondary house.
    fn has_vacation_occupancy(&self, lot_id: u16) -> bool {
        let lot = i32::from(lot_id);
        self.families
            .iter()
            .any(|f| f.house == lot || f.secondary_house == lot)
    }

    /// Parses a raw `FAMI` chunk payload matching `0x4104ce..0x41064e` (with no
    /// upper version ceiling and native `0x464f49` / `0x464ec1` byte-order handling).
    fn record_fami_chunk(&mut self, family_id: i32, data: &[u8], flags: u16) {
        let Some((mut s, version)) = NativeStream::open(data, flags, true) else {
            return;
        };
        let house = s.i32().unwrap_or(0);
        // 0x410083 initializes family `number` to -1; version 0 leaves it at -1 (0x4104f9).
        if version < 5 && s.skip_cstr_padded().is_none() {
            self.record_family_houses(family_id, -1, house, 0);
            return;
        }
        let number = if version >= 1 {
            s.i32().unwrap_or(-1)
        } else {
            -1
        };
        if version >= 2 {
            let _ = s.i32(); // budget (+0x118)
            let _ = s.i32(); // architecture_value (+0x11c)
            let _ = s.i32(); // friends (+0x120)
        }
        if version >= 7 {
            let _ = s.i32(); // flags (+0x138)
        } else {
            if version >= 3 {
                let _ = s.u8();
            }
            if version >= 4 {
                let _ = s.u8();
            }
            if version >= 6 {
                let _ = s.u8();
            }
        }
        // 0x4105e7..0x4105ff: member list is serialized when family_id != 0.
        if family_id != 0 {
            let Some(member_count) = s.i32() else {
                self.record_family_houses(family_id, number, house, 0);
                return;
            };
            if member_count > 0 {
                let skip = (member_count as usize).saturating_mul(4);
                if skip > s.remaining() {
                    self.record_family_houses(family_id, number, house, 0);
                    return;
                }
                s.pos += skip;
            }
        }
        // 0x410602..0x410613: secondary/vacation house exists for all versions >= 8.
        let secondary_raw = if version >= 8 {
            s.i32().unwrap_or(0)
        } else {
            0
        };
        self.record_family_houses(family_id, number, house, secondary_raw);
    }
}

fn load_family_occupancy(install: &Install, user_dir: &Path) -> FamilyOccupancy {
    let mut occ = FamilyOccupancy::default();
    if let Some(iff) =
        resolve_child(install, user_dir, "Neighborhood.iff").and_then(|p| open_iff(&p))
    {
        // 0x424b75..0x424c0b + 0x41031b + 0x41bad8: native family loading resolves
        // each non-negative signed 16-bit FAMI ID through variant 0.
        for (id, data, flags) in iff.all_variant_zero_sorted(b"FAMI") {
            occ.record_fami_chunk(i32::from(id), data, flags);
        }
    }
    // 0x424c65..0x424d38 + 0x425209: add default family 0 if absent, then sort
    // descending by (number, id).
    occ.finalize_native();
    occ
}

fn resolve_lot_status(
    lot_id: u16,
    kind: NeighborhoodKind,
    zoning: LotZoning,
    occ: &FamilyOccupancy,
) -> LotStatus {
    let occupant = occ.occupant_for_lot(lot_id);
    if kind == NeighborhoodKind::Vacation || zoning == LotZoning::Community {
        LotStatus::Community {
            occupied_by_family: occupant.map(|f| f.id),
        }
    } else if let Some(fam) = occupant {
        LotStatus::Occupied {
            family_id: fam.id,
            vacation_lot: u16::try_from(fam.secondary_house).ok().filter(|&v| v > 0),
        }
    } else {
        LotStatus::Vacant
    }
}

/// All six `LotLocations.iff` `STR#` tables loaded by `0x4e1f5e..0x4e2098`.
///
/// Missing `LotLocations.iff` files or missing `STR#` chunks leave their table
/// empty rather than aborting map load, so lookups retain `[0, 0]` (`0x4e20a9`).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct LotLocations {
    /// `STR# 1` — Classic neighborhood (`0..=11`, `50..=80` when not Unleashed; first match wins).
    classic: BTreeMap<u16, [i32; 2]>,
    /// `STR# 2` — Downtown (`21..=30`; last match wins, 49-byte `strncpy` cap at `0x4e2104`).
    downtown: BTreeMap<u16, [i32; 2]>,
    /// `STR# 3` — Vacation Island (`40..=48`; last match wins at `0x4e218f..0x4e2262`).
    vacation: BTreeMap<u16, [i32; 2]>,
    /// `STR# 4` — Unleashed (`0..=11`, `50..=80` when Unleashed; first match wins).
    unleashed: BTreeMap<u16, [i32; 2]>,
    /// `STR# 5` — Studio Town (`81..=89`; last match wins at `0x4e2273..0x4e233e`).
    studio_town: BTreeMap<u16, [i32; 2]>,
    /// `STR# 7` — Magic Town (`90..=99`; last match wins at `0x4e234f..0x4e241a`).
    magic_town: BTreeMap<u16, [i32; 2]>,
}

impl LotLocations {
    /// Dispatches a lot anchor lookup by lot ID, matching `0x4e20a9..0x4e2585`.
    fn anchor_for(&self, lot_id: u16, use_unleashed_table: bool) -> [i32; 2] {
        let table = match lot_id {
            21..=30 => Some(&self.downtown),
            40..=48 => Some(&self.vacation),
            81..=89 => Some(&self.studio_town),
            90..=99 => Some(&self.magic_town),
            0..=11 | 50..=80 => Some(if use_unleashed_table {
                &self.unleashed
            } else {
                &self.classic
            }),
            _ => None,
        };
        table
            .and_then(|map| map.get(&lot_id).copied())
            .unwrap_or([0, 0])
    }
}

#[derive(Debug, Clone, Copy)]
struct NativeResource {
    offset: usize,
    tag: [u8; 4],
    id: u16,
    flags: u16,
}

/// Native IFF 2.0/2.5 reader matching `0x41c2d6..0x41c601` and `0x41d316..0x41d36e`:
/// - checks all 64 header bytes against the wildcard template at `0x6c2128` and
///   requires version `2.0` or `2.5`,
/// - for 2.5 with a nonzero resource-map offset, reads the `rsmp` chunk at that
///   offset and skips the physical scan (`0x41c500`), rejecting invalid offsets,
/// - on resource lookup, seeks to the entry's offset and checks only the chunk's
///   4-byte type tag and bounds (`0x41d32a..0x41d341`), matching native lookup.
struct NativeIff {
    bytes: Vec<u8>,
    by_key: BTreeMap<([u8; 4], u16, u8), NativeResource>,
}

impl NativeIff {
    fn parse(bytes: Vec<u8>) -> Option<Self> {
        const HEADER_TEMPLATE: &[u8; 64] =
            b"IFF FILE *.*:TYPE FOLLOWED BY SIZE\0 JAMIE DOORNBOS & MAXIS 1****";
        const CHUNK_HEADER_SIZE: usize = 76;
        const MAX_PAYLOAD_SIZE: usize = 0x4000_0000;

        let header = bytes.get(..64)?;
        for (&expected, &actual) in HEADER_TEMPLATE.iter().zip(header.iter()) {
            if expected != b'*' && expected != actual {
                return None;
            }
        }
        let is_25 = match (header[9], header[11]) {
            (b'2', b'0') => false,
            (b'2', b'5') => true,
            _ => return None,
        };

        let mut by_key: BTreeMap<([u8; 4], u16, u8), NativeResource> = BTreeMap::new();
        let mut used_map = false;

        if is_25 {
            let rsmp_offset = u32::from_be_bytes(header[60..64].try_into().unwrap()) as usize;
            if rsmp_offset != 0 {
                // 0x41c3c3..0x41c3dd: rsmp chunk header must exist at rsmp_offset,
                // be live (not deleted), and carry tag `rsmp`.
                let hdr = bytes.get(rsmp_offset..rsmp_offset.checked_add(CHUNK_HEADER_SIZE)?)?;
                let tag: [u8; 4] = hdr[0..4].try_into().unwrap();
                let length = u32::from_be_bytes(hdr[4..8].try_into().unwrap()) as usize;
                let flags = u16::from_be_bytes(hdr[10..12].try_into().unwrap());
                if tag != *b"rsmp" || (flags & IFF_DELETED) != 0 {
                    return None;
                }
                // 0x41c468..0x41c4dd: reading or parsing the rsmp payload may
                // succeed (skipping physical scan) or fall back to physical scan
                // when the map payload is unusable or lists `rsmp` ID 0.
                let payload_len = length.checked_sub(CHUNK_HEADER_SIZE);
                let payload = payload_len
                    .filter(|&n| n <= MAX_PAYLOAD_SIZE)
                    .and_then(|_| {
                        bytes.get(rsmp_offset + CHUNK_HEADER_SIZE..rsmp_offset.checked_add(length)?)
                    });
                if let Some(payload) = payload
                    && let Some((parsed_by_key, lists_self)) = parse_native_rsmp(payload)
                    && !lists_self
                {
                    by_key = parsed_by_key;
                    used_map = true;
                }
            }
        }

        if !used_map {
            // 0x41c510..0x41c5f8: physical chunk scan from offset 64 until EOF
            // or invalid chunk size; deleted chunks are skipped and `rsmp` is dropped.
            let mut pos = 64usize;
            while let Some(hdr) = pos
                .checked_add(CHUNK_HEADER_SIZE)
                .and_then(|end| bytes.get(pos..end))
            {
                let tag: [u8; 4] = hdr[0..4].try_into().unwrap();
                let length = u32::from_be_bytes(hdr[4..8].try_into().unwrap()) as usize;
                let id = u16::from_be_bytes(hdr[8..10].try_into().unwrap());
                let flags = u16::from_be_bytes(hdr[10..12].try_into().unwrap());
                if !(CHUNK_HEADER_SIZE..=MAX_PAYLOAD_SIZE).contains(&length) {
                    break;
                }
                if (flags & IFF_DELETED) == 0 && tag != *b"rsmp" {
                    let key = (tag, id, (flags >> 8) as u8);
                    let res = NativeResource {
                        offset: pos,
                        tag,
                        id,
                        flags,
                    };
                    by_key.insert(key, res);
                }
                let Some(next) = pos.checked_add(length) else {
                    break;
                };
                pos = next;
            }
        }

        Some(Self { bytes, by_key })
    }

    /// Reads the payload for `res`, verifying only bounds and matching chunk
    /// tag (`0x41d316..0x41d36e`), without requiring physical chunk ID parity.
    fn payload(&self, res: NativeResource) -> Option<&[u8]> {
        const CHUNK_HEADER_SIZE: usize = 76;
        const MAX_PAYLOAD_SIZE: usize = 0x4000_0000;

        let hdr_end = res.offset.checked_add(CHUNK_HEADER_SIZE)?;
        let hdr = self.bytes.get(res.offset..hdr_end)?;
        if &hdr[0..4] != res.tag.as_slice() {
            return None;
        }
        let length = u32::from_be_bytes(hdr[4..8].try_into().unwrap()) as usize;
        let payload_len = length.checked_sub(CHUNK_HEADER_SIZE)?;
        if payload_len > MAX_PAYLOAD_SIZE {
            return None;
        }
        let end = res.offset.checked_add(length)?;
        self.bytes.get(hdr_end..end)
    }

    fn get(&self, tag: &[u8; 4], id: u16) -> Option<(&[u8], u16)> {
        let &res = self.by_key.get(&(*tag, id, 0))?;
        Some((self.payload(res)?, res.flags))
    }

    /// Returns every variant-0 (`(flags >> 8) == 0`) resource of `tag` with a
    /// non-negative signed 16-bit ID (`0..=32767`), sorted by ascending ID
    /// (`0x424b75..0x424c0b`, `0x41031b`, `0x41bad8`).
    fn all_variant_zero_sorted(&self, tag: &[u8; 4]) -> Vec<(u16, &[u8], u16)> {
        self.by_key
            .range((*tag, 0, 0)..=(*tag, i16::MAX as u16, 0))
            .filter(|&(&(_, _, variant), _)| variant == 0)
            .filter_map(|(_, &res)| Some((res.id, self.payload(res)?, res.flags)))
            .collect()
    }
}

type ParsedRsmp = (BTreeMap<([u8; 4], u16, u8), NativeResource>, bool);

/// Parses a native `rsmp` stream (`0x41be90..0x41c04c`, `0x41b7cb..0x41b826`),
/// stopping immediately after the declared type/entry counts so trailing
/// padding bytes inside the `rsmp` chunk do not invalidate the index.
fn parse_native_rsmp(payload: &[u8]) -> Option<ParsedRsmp> {
    // 0x41c465 pushes 0 for the load callback, so rsmp uses payload[0] for byte order.
    let (mut s, _version) = NativeStream::open(payload, 0x10, false)?;
    let _unused_bytes = s.i32()?;
    let type_count = s.i32()?;
    let mut by_key: BTreeMap<([u8; 4], u16, u8), NativeResource> = BTreeMap::new();
    let mut lists_self = false;
    if type_count <= 0 {
        return Some((by_key, false));
    }
    let type_count = type_count as usize;
    if type_count.saturating_mul(8) > s.remaining() {
        return None;
    }
    for _ in 0..type_count {
        let entry_tag = (s.i32()? as u32).to_be_bytes();
        let entry_count = s.i32()?;
        if entry_count <= 0 {
            continue;
        }
        let entry_count = entry_count as usize;
        // Each entry holds 8 fixed bytes + NUL-terminated label (at least 1 byte;
        // 0x464c3e advances past the optional alignment byte without reading past EOF).
        if entry_count.saturating_mul(9) > s.remaining() {
            return None;
        }
        for _ in 0..entry_count {
            let offset = s.i32()? as u32 as usize;
            let id = s.i16()? as u16;
            let flags = s.i16()? as u16;
            s.skip_cstr_padded()?;
            if entry_tag == *b"rsmp" {
                if id == 0 && (flags >> 8) == 0 {
                    lists_self = true;
                }
                continue;
            }
            let key = (entry_tag, id, (flags >> 8) as u8);
            by_key.entry(key).or_insert(NativeResource {
                offset,
                tag: entry_tag,
                id,
                flags,
            });
        }
    }
    Some((by_key, lists_self))
}

fn open_iff(path: &Path) -> Option<NativeIff> {
    NativeIff::parse(fs::read(path).ok()?)
}

/// Parses a legacy `\0CST` string-table chunk (`0x470178..0x470209`),
/// extracting `^...^`-delimited strings in file order.
fn parse_cst_chunk(data: &[u8]) -> StringTable {
    let mut entries = Vec::new();
    let mut start: Option<usize> = None;
    for (i, &b) in data.iter().enumerate() {
        if b == b'^' {
            if let Some(s) = start.take() {
                let raw = &data[s..i];
                entries.push(TextEntry {
                    language: US_ENGLISH,
                    text: sims_formats::strings::decode(raw, US_ENGLISH),
                    comment: String::new(),
                    raw_text: raw.to_vec(),
                    raw_comment: Vec::new(),
                });
            } else {
                start = Some(i + 1);
            }
        }
    }
    StringTable {
        format: StringTableFormat::Terminated,
        entries,
    }
}

/// Loads a string table chunk matching `0x4702f9..0x47034b`: tries `STR#` first,
/// then falls back to legacy `\0CST` (`0x00435354`) with the same resource ID.
fn load_native_string_table(iff: &NativeIff, id: u16) -> Option<StringTable> {
    if let Some((data, flags)) = iff.get(b"STR#", id)
        && let Ok(table) = StringTable::parse_chunk(data, flags)
    {
        return Some(table);
    }
    iff.get(b"\0CST", id).map(|(data, _)| parse_cst_chunk(data))
}

/// Reproduces the native `STR#` US English entry vector order (`0x46ffd5..0x470069`,
/// `0x470482`): for version `-3` (`Format::Localized`) tables, each language
/// transition resets the insertion cursor to 0 and inserts subsequent entries
/// ahead of earlier blocks of the same language.
fn native_us_entries(table: &StringTable) -> Vec<&TextEntry> {
    let mut out = Vec::new();
    if table.format != StringTableFormat::Localized {
        out.extend(table.entries.iter().filter(|e| e.language == US_ENGLISH));
        return out;
    }
    let mut prev_lang: Option<u8> = None;
    let mut idx: usize = 0;
    for entry in &table.entries {
        if prev_lang != Some(entry.language) {
            idx = 0;
            prev_lang = Some(entry.language);
        }
        if entry.language != US_ENGLISH {
            continue;
        }
        let pos = idx.min(out.len());
        out.insert(pos, entry);
        idx += 1;
    }
    out
}

fn load_lot_locations(install: &Install, user_dir: &Path) -> LotLocations {
    // 0x4e1f7c..0x4e2098: attempt to open UserDataN/LotLocations.iff first; if
    // missing or rejected by the IFF opener, fall back to GameData/LotLocations.iff.
    // Missing files or missing STR#/\0CST tables leave empty maps ([0, 0] anchors).
    let Some(iff) = resolve_child(install, user_dir, "LotLocations.iff")
        .and_then(|p| open_iff(&p))
        .or_else(|| {
            install
                .file("GameData/LotLocations.iff")
                .ok()
                .and_then(|p| open_iff(&p))
        })
    else {
        return LotLocations::default();
    };
    let read_table = |str_id: u16, last_match_wins: bool, max_bytes: Option<usize>| {
        load_native_string_table(&iff, str_id)
            .map(|table| parse_lot_locations_table_with(&table, last_match_wins, max_bytes))
            .unwrap_or_default()
    };
    LotLocations {
        classic: read_table(1, false, None),
        downtown: read_table(2, true, Some(49)),
        vacation: read_table(3, true, None),
        unleashed: read_table(4, false, None),
        studio_town: read_table(5, true, None),
        magic_town: read_table(7, true, None),
    }
}

/// Parses a main-map `LotLocations.iff` `STR#` table (`STR# 1` or `STR# 4`,
/// `"lot_id, x, y"` rows in US English), matching `0x4e2501..0x4e257c`:
/// - iterates entries in native `STR#` insertion order (`0x470002`, `0x470482`),
/// - stops each row at its first NUL byte,
/// - splits on the first two commas (rows with fewer than two commas are skipped),
/// - parses signed 32-bit `id`, `x`, and `y` via C `atoi` (`0x636029`),
/// - keeps the first matching row for each lot ID.
pub fn parse_lot_locations_table(table: &StringTable) -> BTreeMap<u16, [i32; 2]> {
    parse_lot_locations_table_with(table, false, None)
}

fn parse_lot_locations_table_with(
    table: &StringTable,
    last_match_wins: bool,
    max_bytes: Option<usize>,
) -> BTreeMap<u16, [i32; 2]> {
    let mut map = BTreeMap::new();
    for entry in native_us_entries(table) {
        let raw = if !entry.raw_text.is_empty() {
            entry.raw_text.as_slice()
        } else {
            entry.text.as_bytes()
        };
        let nul_end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
        let cap_end = max_bytes.map_or(nul_end, |cap| nul_end.min(cap));
        let cstr = &raw[..cap_end];
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
        if last_match_wins {
            map.insert(lot_id, [x, y]);
        } else {
            map.entry(lot_id).or_insert([x, y]);
        }
    }
    map
}

/// Parsed `LotZoning.iff` `STR# 1` table preserving every row in file order
/// with signed 32-bit lot IDs (`0x425016..0x425064`) and recording the first
/// matching zone per lot ID (`0x42b9fc`).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LotZoningTable {
    pub rows: Vec<(i32, LotZoning)>,
    pub first_zone: BTreeMap<i32, LotZoning>,
}

fn load_lot_zoning(install: &Install, user_dir: &Path) -> LotZoningTable {
    // 0x424f1c..0x424f42: native reload opens only the neighborhood's local
    // LotZoning.iff (never GameData/MasterLotZoning.iff) and tries STR# 1 then
    // \0CST 1; failure leaves the zoning table empty rather than aborting maps.
    let Some(iff) = resolve_child(install, user_dir, "LotZoning.iff").and_then(|p| open_iff(&p))
    else {
        return LotZoningTable::default();
    };
    load_native_string_table(&iff, 1)
        .map(|table| parse_lot_zoning_table(&table))
        .unwrap_or_default()
}

/// Parses a `LotZoning.iff` `STR# 1` table (`"lot_id, residential|community"`),
/// following `0x424fb9..0x425064`:
/// - iterates entries in native `STR#` insertion order (`0x470002`, `0x470482`),
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
/// - preserves every row in native table order (`0x425064`) and records the
///   first matching zone per lot ID (`0x42b9fc..0x42ba09`).
pub fn parse_lot_zoning_table(table: &StringTable) -> LotZoningTable {
    const NATIVE_BUF_LIMIT: usize = 99;
    let mut rows = Vec::new();
    let mut first_zone = BTreeMap::new();
    for entry in native_us_entries(table) {
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

struct LocalMapArtwork {
    backdrop: Surface565,
    waves: Vec<Option<Surface565>>,
    vacation_port: Option<Surface565>,
    vacation_trees: Option<Surface565>,
}

/// Attempts to load neighborhood-local custom JPEG map artwork before falling
/// back to bundled `UIGraphics`, matching native `0x55e976` (Classic),
/// `0x556a18` (Unleashed), `0x54a2ad` (Downtown), and `0x568bfd` (Vacation).
fn load_local_map_artwork(
    install: &Install,
    user_dir: &Path,
    kind: NeighborhoodKind,
) -> Option<LocalMapArtwork> {
    let (backdrop_name, wave_names, port_name, trees_name): (
        &str,
        &[&str],
        Option<&str>,
        Option<&str>,
    ) = match kind {
        NeighborhoodKind::Classic => (
            "neighborhood.jpg",
            &[
                "neighborhood0.jpg",
                "neighborhood1.jpg",
                "neighborhood2.jpg",
            ],
            None,
            None,
        ),
        NeighborhoodKind::Unleashed => (
            "neighborhoodul.jpg",
            &[
                "neighborhoodul0.jpg",
                "neighborhoodul1.jpg",
                "neighborhoodul2.jpg",
                "neighborhoodul3.jpg",
                "neighborhoodul4.jpg",
                "neighborhoodul5.jpg",
            ],
            None,
            None,
        ),
        NeighborhoodKind::Downtown => (
            "downtown.jpg",
            &[
                "downtown0.jpg",
                "downtown1.jpg",
                "downtown2.jpg",
                "downtown3.jpg",
                "downtown4.jpg",
                "downtown5.jpg",
            ],
            None,
            None,
        ),
        NeighborhoodKind::Vacation => (
            "island.jpg",
            &[
                "island0.jpg",
                "island1.jpg",
                "island2.jpg",
                "island3.jpg",
                "island4.jpg",
                "island5.jpg",
            ],
            Some("islandPort.jpg"),
            Some("islandTrees.jpg"),
        ),
        NeighborhoodKind::StudioTown | NeighborhoodKind::MagicTown => return None,
    };

    let load_jpg = |name: &str| -> Option<Surface565> {
        let path = resolve_child(install, user_dir, name)?;
        let bytes = fs::read(&path).ok()?;
        decode_jpeg_to_surface565(&bytes)
    };

    let backdrop = load_jpg(backdrop_name)?;
    // Preserve numbered wave slots so missing slots are skipped rather than
    // shifting subsequent frames down (0x56bb7a..0x56bb8d).
    let waves = wave_names.iter().map(|&name| load_jpg(name)).collect();
    let vacation_port = port_name.and_then(load_jpg);
    let vacation_trees = trees_name.and_then(load_jpg);
    Some(LocalMapArtwork {
        backdrop,
        waves,
        vacation_port,
        vacation_trees,
    })
}

const JPEG_ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20,
    13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59,
    52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

#[derive(Clone, Default)]
struct JpegHuffTable {
    values: Vec<u8>,
    max_code: [i32; 17],
    val_offset: [i32; 17],
}

impl JpegHuffTable {
    fn build(counts: &[u8; 16], symbols: &[u8]) -> Option<Self> {
        let total: usize = counts.iter().map(|&c| usize::from(c)).sum();
        if total != symbols.len() || total > 256 {
            return None;
        }
        let mut code: u32 = 0;
        let mut max_code = [-1i32; 17];
        let mut val_offset = [0i32; 17];
        let mut k = 0i32;
        for (len_idx, &count) in counts.iter().enumerate() {
            let bits = len_idx + 1;
            if count > 0 {
                val_offset[bits] = k - code as i32;
                code += u32::from(count);
                k += i32::from(count);
                max_code[bits] = (code - 1) as i32;
            }
            code <<= 1;
        }
        Some(Self {
            values: symbols.to_vec(),
            max_code,
            val_offset,
        })
    }
}

struct JpegBitReader<'a> {
    data: &'a [u8],
    pos: usize,
    bit_buf: u32,
    bit_count: u8,
    pending_marker: Option<u8>,
}

impl<'a> JpegBitReader<'a> {
    fn new(data: &'a [u8], pos: usize) -> Self {
        Self {
            data,
            pos,
            bit_buf: 0,
            bit_count: 0,
            pending_marker: None,
        }
    }

    fn next_byte(&mut self) -> Option<u8> {
        if self.pending_marker.is_some() {
            return Some(0);
        }
        let &b = self.data.get(self.pos)?;
        self.pos += 1;
        if b == 0xff {
            while self.data.get(self.pos) == Some(&0xff) {
                self.pos += 1;
            }
            let &next = self.data.get(self.pos)?;
            self.pos += 1;
            if next == 0x00 {
                return Some(0xff);
            }
            self.pending_marker = Some(next);
            return Some(0);
        }
        Some(b)
    }

    fn read_bit(&mut self) -> Option<u32> {
        if self.bit_count == 0 {
            let b = self.next_byte()?;
            self.bit_buf = u32::from(b);
            self.bit_count = 8;
        }
        self.bit_count -= 1;
        Some((self.bit_buf >> self.bit_count) & 1)
    }

    fn read_bits(&mut self, n: u8) -> Option<u32> {
        let mut v = 0u32;
        for _ in 0..n {
            v = (v << 1) | self.read_bit()?;
        }
        Some(v)
    }

    fn decode_huff(&mut self, table: &JpegHuffTable) -> Option<u8> {
        let mut code = 0i32;
        for bits in 1..=16usize {
            code = (code << 1) | (self.read_bit()? as i32);
            if code <= table.max_code[bits] {
                let idx = (code + table.val_offset[bits]) as usize;
                return table.values.get(idx).copied();
            }
        }
        None
    }

    fn receive_extend(&mut self, n: u8) -> Option<i32> {
        if n == 0 {
            return Some(0);
        }
        if n > 15 {
            return None;
        }
        let v = self.read_bits(n)? as i32;
        let vt = 1i32 << (n - 1);
        Some(if v < vt { v + (-1 << n) + 1 } else { v })
    }

    fn consume_restart(&mut self) {
        self.bit_buf = 0;
        self.bit_count = 0;
        if let Some(m) = self.pending_marker {
            if (0xd0..=0xd7).contains(&m) {
                self.pending_marker = None;
            }
            return;
        }
        while self.pos + 1 < self.data.len() && self.data[self.pos] == 0xff {
            let m = self.data[self.pos + 1];
            if m == 0xff {
                self.pos += 1;
            } else if (0xd0..=0xd7).contains(&m) {
                self.pos += 2;
                break;
            } else {
                break;
            }
        }
    }

    fn finish_scan_pos(&self) -> usize {
        if self.pending_marker.is_some() {
            self.pos.saturating_sub(2)
        } else {
            self.pos
        }
    }
}

#[derive(Clone, Default)]
struct JpegComponent {
    id: u8,
    h_samp: usize,
    v_samp: usize,
    qt_id: usize,
    blocks_x: usize,
    blocks_y: usize,
    coeffs: Vec<[i32; 64]>,
}

/// Decodes a baseline (`SOF0`), extended sequential (`SOF1`), or progressive
/// (`SOF2`) 8-bit JPEG image directly into a 5-6-5 [`Surface565`] using
/// IJG-compatible integer 8×8 IDCT and YCbCr-to-RGB conversion (`0x4b8285`).
fn decode_jpeg_to_surface565(data: &[u8]) -> Option<Surface565> {
    if data.len() < 4 || data[0] != 0xff || data[1] != 0xd8 {
        return None;
    }
    let mut pos = 2usize;
    let mut qtables: [Option<[i32; 64]>; 4] = [None, None, None, None];
    let mut dc_tables: [Option<JpegHuffTable>; 4] = [None, None, None, None];
    let mut ac_tables: [Option<JpegHuffTable>; 4] = [None, None, None, None];
    let mut width = 0usize;
    let mut height = 0usize;
    let mut progressive = false;
    let mut comps: Vec<JpegComponent> = Vec::new();
    let mut comp_has_scan: Vec<bool> = Vec::new();
    let mut h_max = 1usize;
    let mut v_max = 1usize;
    let mut mcu_cols = 0usize;
    let mut mcu_rows = 0usize;
    let mut restart_interval = 0usize;
    let mut adobe_transform: Option<u8> = None;

    while pos + 1 < data.len() {
        while pos < data.len() && data[pos] != 0xff {
            pos += 1;
        }
        while pos < data.len() && data[pos] == 0xff {
            pos += 1;
        }
        if pos >= data.len() {
            break;
        }
        let marker = data[pos];
        pos += 1;
        if marker == 0xd9 {
            break;
        }
        if marker == 0x00 || marker == 0x01 || (0xd0..=0xd8).contains(&marker) {
            continue;
        }
        if pos + 2 > data.len() {
            return None;
        }
        let seg_len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
        if seg_len < 2 || pos + seg_len > data.len() {
            return None;
        }
        let seg = &data[pos + 2..pos + seg_len];
        pos += seg_len;

        match marker {
            0xdb => {
                // DQT
                let mut i = 0;
                while i < seg.len() {
                    let pq_tq = seg[i];
                    i += 1;
                    let pq = pq_tq >> 4;
                    let tq = (pq_tq & 0x0f) as usize;
                    if tq >= 4 {
                        return None;
                    }
                    let mut qt = [0i32; 64];
                    if pq == 0 {
                        if i + 64 > seg.len() {
                            return None;
                        }
                        for k in 0..64 {
                            qt[k] = i32::from(seg[i + k]);
                        }
                        i += 64;
                    } else if pq == 1 {
                        if i + 128 > seg.len() {
                            return None;
                        }
                        for k in 0..64 {
                            qt[k] =
                                i32::from(u16::from_be_bytes([seg[i + k * 2], seg[i + k * 2 + 1]]));
                        }
                        i += 128;
                    } else {
                        return None;
                    }
                    qtables[tq] = Some(qt);
                }
            }
            0xc4 => {
                // DHT
                let mut i = 0;
                while i < seg.len() {
                    if i + 17 > seg.len() {
                        return None;
                    }
                    let tc_th = seg[i];
                    let tc = tc_th >> 4;
                    let th = (tc_th & 0x0f) as usize;
                    if th >= 4 || tc > 1 {
                        return None;
                    }
                    let counts: [u8; 16] = seg[i + 1..i + 17].try_into().ok()?;
                    let total: usize = counts.iter().map(|&c| usize::from(c)).sum();
                    if i + 17 + total > seg.len() {
                        return None;
                    }
                    let table = JpegHuffTable::build(&counts, &seg[i + 17..i + 17 + total])?;
                    if tc == 0 {
                        dc_tables[th] = Some(table);
                    } else {
                        ac_tables[th] = Some(table);
                    }
                    i += 17 + total;
                }
            }
            0xdd => {
                // DRI
                if seg.len() < 2 {
                    return None;
                }
                restart_interval = usize::from(u16::from_be_bytes([seg[0], seg[1]]));
            }
            0xee => {
                // Adobe APP14
                if seg.len() >= 12 && &seg[0..5] == b"Adobe" {
                    adobe_transform = Some(seg[11]);
                }
            }
            0xc0..=0xc2 => {
                // SOF0 / SOF1 / SOF2
                if seg.len() < 6 || seg[0] != 8 {
                    return None;
                }
                progressive = marker == 0xc2;
                height = usize::from(u16::from_be_bytes([seg[1], seg[2]]));
                width = usize::from(u16::from_be_bytes([seg[3], seg[4]]));
                let ncomp = usize::from(seg[5]);
                if width == 0
                    || height == 0
                    || width
                        .checked_mul(height)
                        .is_none_or(|p| p > MAX_SURFACE_PIXELS)
                    || !matches!(ncomp, 1 | 3 | 4)
                    || seg.len() < 6 + ncomp * 3
                {
                    return None;
                }
                comps.clear();
                comp_has_scan.clear();
                h_max = 1;
                v_max = 1;
                for c in 0..ncomp {
                    let id = seg[6 + c * 3];
                    let hv = seg[6 + c * 3 + 1];
                    let h = usize::from(hv >> 4);
                    let v = usize::from(hv & 0x0f);
                    let qt_id = usize::from(seg[6 + c * 3 + 2]);
                    if !(1..=4).contains(&h) || !(1..=4).contains(&v) || qt_id >= 4 {
                        return None;
                    }
                    h_max = h_max.max(h);
                    v_max = v_max.max(v);
                    comps.push(JpegComponent {
                        id,
                        h_samp: h,
                        v_samp: v,
                        qt_id,
                        blocks_x: 0,
                        blocks_y: 0,
                        coeffs: Vec::new(),
                    });
                }
                mcu_cols = width.div_ceil(h_max * 8);
                mcu_rows = height.div_ceil(v_max * 8);
                for comp in &mut comps {
                    comp.blocks_x = mcu_cols.checked_mul(comp.h_samp)?;
                    comp.blocks_y = mcu_rows.checked_mul(comp.v_samp)?;
                    let total_blocks = comp.blocks_x.checked_mul(comp.blocks_y)?;
                    if total_blocks > MAX_SURFACE_PIXELS / 16 + 4096 {
                        return None;
                    }
                    comp.coeffs = vec![[0i32; 64]; total_blocks];
                }
                comp_has_scan = vec![false; comps.len()];
            }
            0xda => {
                // SOS
                if comps.is_empty() || seg.is_empty() {
                    return None;
                }
                let ns = usize::from(seg[0]);
                if ns == 0 || ns > comps.len() || seg.len() < 1 + ns * 2 + 3 {
                    return None;
                }
                let mut scan_comps = Vec::with_capacity(ns);
                for i in 0..ns {
                    let cs = seg[1 + i * 2];
                    let td_ta = seg[1 + i * 2 + 1];
                    let comp_idx = comps.iter().position(|c| c.id == cs)?;
                    scan_comps.push((comp_idx, usize::from(td_ta >> 4), usize::from(td_ta & 0x0f)));
                }
                let ss = usize::from(seg[1 + ns * 2]);
                let se = usize::from(seg[1 + ns * 2 + 1]);
                let approx = seg[1 + ns * 2 + 2];
                let ah = approx >> 4;
                let al = approx & 0x0f;
                if ss > se || se >= 64 || ah > 15 || al > 15 {
                    return None;
                }

                let mut br = JpegBitReader::new(data, pos);
                let mut dc_pred = [0i32; 4];
                let mut eobrun = 0u32;
                let mut mcus_done = 0usize;

                let decode_block = |br: &mut JpegBitReader<'_>,
                                    block: &mut [i32; 64],
                                    dc_pred: &mut [i32; 4],
                                    eobrun: &mut u32,
                                    comp_idx: usize,
                                    td: usize,
                                    ta: usize|
                 -> Option<()> {
                    if !progressive {
                        let dc_t = dc_tables.get(td)?.as_ref()?;
                        let ac_t = ac_tables.get(ta)?.as_ref()?;
                        let s = br.decode_huff(dc_t)?;
                        let diff = br.receive_extend(s)?;
                        dc_pred[comp_idx] += diff;
                        block[0] = dc_pred[comp_idx];
                        let mut k = 1usize;
                        while k < 64 {
                            let rs = br.decode_huff(ac_t)?;
                            let r = usize::from(rs >> 4);
                            let s = rs & 0x0f;
                            if s == 0 {
                                if r == 15 {
                                    k += 16;
                                } else {
                                    break;
                                }
                            } else {
                                k += r;
                                if k >= 64 {
                                    return None;
                                }
                                block[k] = br.receive_extend(s)?;
                                k += 1;
                            }
                        }
                    } else if ss == 0 && se == 0 {
                        if ah == 0 {
                            let dc_t = dc_tables.get(td)?.as_ref()?;
                            let s = br.decode_huff(dc_t)?;
                            let diff = br.receive_extend(s)?;
                            dc_pred[comp_idx] += diff;
                            block[0] = dc_pred[comp_idx] << al;
                        } else {
                            let bit = br.read_bit()? as i32;
                            block[0] |= bit << al;
                        }
                    } else {
                        let ac_t = ac_tables.get(ta)?.as_ref()?;
                        let p1 = 1i32 << al;
                        let m1 = -1i32 << al;
                        if ah == 0 {
                            if *eobrun > 0 {
                                *eobrun -= 1;
                            } else {
                                let mut k = ss;
                                while k <= se {
                                    let rs = br.decode_huff(ac_t)?;
                                    let r = usize::from(rs >> 4);
                                    let s = rs & 0x0f;
                                    if s == 0 {
                                        if r == 15 {
                                            k += 16;
                                        } else {
                                            *eobrun = (1u32 << r) + br.read_bits(r as u8)? - 1;
                                            break;
                                        }
                                    } else {
                                        k += r;
                                        if k > se {
                                            return None;
                                        }
                                        block[k] = br.receive_extend(s)? << al;
                                        k += 1;
                                    }
                                }
                            }
                        } else {
                            let mut k = ss;
                            if *eobrun == 0 {
                                while k <= se {
                                    let rs = br.decode_huff(ac_t)?;
                                    let mut r = usize::from(rs >> 4);
                                    let s = rs & 0x0f;
                                    let new_val = if s == 0 {
                                        if r != 15 {
                                            *eobrun = (1u32 << r) + br.read_bits(r as u8)?;
                                            break;
                                        }
                                        0
                                    } else if s == 1 {
                                        if br.read_bit()? == 1 { p1 } else { m1 }
                                    } else {
                                        return None;
                                    };
                                    while k <= se {
                                        if block[k] != 0 {
                                            if br.read_bit()? == 1
                                                && (block[k].unsigned_abs() & (p1 as u32)) == 0
                                            {
                                                block[k] += if block[k] > 0 { p1 } else { m1 };
                                            }
                                        } else {
                                            if r == 0 {
                                                break;
                                            }
                                            r -= 1;
                                        }
                                        k += 1;
                                    }
                                    if s != 0 {
                                        if k > se {
                                            return None;
                                        }
                                        block[k] = new_val;
                                        k += 1;
                                    } else if k <= se {
                                        // ZRL (s == 0, r == 15): advance past the 16th zero.
                                        k += 1;
                                    }
                                }
                            }
                            if *eobrun > 0 {
                                while k <= se {
                                    if block[k] != 0
                                        && br.read_bit()? == 1
                                        && (block[k].unsigned_abs() & (p1 as u32)) == 0
                                    {
                                        block[k] += if block[k] > 0 { p1 } else { m1 };
                                    }
                                    k += 1;
                                }
                                *eobrun -= 1;
                            }
                        }
                    }
                    Some(())
                };

                if ns == 1 {
                    let (ci, td, ta) = scan_comps[0];
                    let comp_w = (width * comps[ci].h_samp).div_ceil(h_max);
                    let comp_h = (height * comps[ci].v_samp).div_ceil(v_max);
                    let bw = comp_w.div_ceil(8).min(comps[ci].blocks_x);
                    let bh = comp_h.div_ceil(8).min(comps[ci].blocks_y);
                    let stride = comps[ci].blocks_x;
                    let total_mcus = bw * bh;
                    for by in 0..bh {
                        for bx in 0..bw {
                            if restart_interval > 0
                                && mcus_done > 0
                                && mcus_done.is_multiple_of(restart_interval)
                            {
                                br.consume_restart();
                                dc_pred = [0; 4];
                                eobrun = 0;
                            }
                            let idx = by * stride + bx;
                            decode_block(
                                &mut br,
                                &mut comps[ci].coeffs[idx],
                                &mut dc_pred,
                                &mut eobrun,
                                ci,
                                td,
                                ta,
                            )?;
                            mcus_done += 1;
                            if mcus_done >= total_mcus {
                                break;
                            }
                        }
                    }
                } else {
                    for my in 0..mcu_rows {
                        for mx in 0..mcu_cols {
                            if restart_interval > 0
                                && mcus_done > 0
                                && mcus_done.is_multiple_of(restart_interval)
                            {
                                br.consume_restart();
                                dc_pred = [0; 4];
                                eobrun = 0;
                            }
                            for &(ci, td, ta) in &scan_comps {
                                let hs = comps[ci].h_samp;
                                let vs = comps[ci].v_samp;
                                let stride = comps[ci].blocks_x;
                                for dy in 0..vs {
                                    for dx in 0..hs {
                                        let idx = (my * vs + dy) * stride + (mx * hs + dx);
                                        decode_block(
                                            &mut br,
                                            &mut comps[ci].coeffs[idx],
                                            &mut dc_pred,
                                            &mut eobrun,
                                            ci,
                                            td,
                                            ta,
                                        )?;
                                    }
                                }
                            }
                            mcus_done += 1;
                        }
                    }
                }
                for &(ci, _, _) in &scan_comps {
                    comp_has_scan[ci] = true;
                }
                pos = br.finish_scan_pos();
            }
            _ => {}
        }
    }

    if comps.is_empty()
        || width == 0
        || height == 0
        || !comp_has_scan.iter().all(|&has_scan| has_scan)
    {
        return None;
    }

    // Dequantize + integer 8×8 IDCT for every component plane.
    let mut planes: Vec<(usize, usize, Vec<u8>)> = Vec::with_capacity(comps.len());
    for comp in &comps {
        let qt = qtables[comp.qt_id]?;
        let pw = comp.blocks_x * 8;
        let ph = comp.blocks_y * 8;
        let mut plane = vec![0u8; pw * ph];
        for by in 0..comp.blocks_y {
            for bx in 0..comp.blocks_x {
                let mut deq = [0i32; 64];
                let raw = &comp.coeffs[by * comp.blocks_x + bx];
                for k in 0..64 {
                    deq[JPEG_ZIGZAG[k]] = raw[k].saturating_mul(qt[k]);
                }
                let out = jpeg_idct_8x8(&deq);
                for r in 0..8 {
                    let dst = (by * 8 + r) * pw + bx * 8;
                    plane[dst..dst + 8].copy_from_slice(&out[r * 8..r * 8 + 8]);
                }
            }
        }
        planes.push((pw, ph, plane));
    }

    let is_rgb = comps.len() == 3
        && (adobe_transform == Some(0)
            || (comps[0].id == b'R' && comps[1].id == b'G' && comps[2].id == b'B'));

    let mut pixels = vec![0u16; width * height];
    for y in 0..height {
        for x in 0..width {
            let sample = |ci: usize| -> i32 {
                let (pw, ph, ref p) = planes[ci];
                let sx = (x * comps[ci].h_samp / h_max).min(pw - 1);
                let sy = (y * comps[ci].v_samp / v_max).min(ph - 1);
                i32::from(p[sy * pw + sx])
            };
            let rgb = match comps.len() {
                1 => {
                    let v = sample(0) as u8;
                    [v, v, v]
                }
                3 => {
                    let c0 = sample(0);
                    let c1 = sample(1);
                    let c2 = sample(2);
                    if is_rgb {
                        [c0 as u8, c1 as u8, c2 as u8]
                    } else {
                        let cb = c1 - 128;
                        let cr = c2 - 128;
                        let r = (c0 + ((91_881 * cr + 32_768) >> 16)).clamp(0, 255) as u8;
                        let g = (c0 + ((-22_554 * cb - 46_802 * cr + 32_768) >> 16)).clamp(0, 255)
                            as u8;
                        let b = (c0 + ((116_130 * cb + 32_768) >> 16)).clamp(0, 255) as u8;
                        [r, g, b]
                    }
                }
                4 => {
                    // Shipped `gimex.dll` (`0x600185b5..0x600185dc`) calls
                    // `ijl10.dll` (`ijlInit` sets `JPGColor = IJL_YCBCR (3)`,
                    // `DIBColor = IJL_BGR (2)`, `DIBChannels = 3`) with
                    // `cconversion_reqd = 1` without overriding `JPGColor`, so
                    // 4-component CMYK and YCCK JPEGs are converted from the
                    // first three channels as YCbCr -> RGB and channel 4 is dropped.
                    let c0 = sample(0);
                    let cb = sample(1) - 128;
                    let cr = sample(2) - 128;
                    let r = (c0 + ((91_881 * cr + 32_768) >> 16)).clamp(0, 255) as u8;
                    let g =
                        (c0 + ((-22_554 * cb - 46_802 * cr + 32_768) >> 16)).clamp(0, 255) as u8;
                    let b = (c0 + ((116_130 * cb + 32_768) >> 16)).clamp(0, 255) as u8;
                    [r, g, b]
                }
                _ => return None,
            };
            pixels[y * width + x] = pack_565(rgb);
        }
    }

    Some(Surface565 {
        width,
        height,
        pixels,
    })
}

/// IJG 13-bit fixed-point separable 8×8 inverse DCT (`jidctint.c`), returning
/// level-shifted (`+128`) samples clamped to `0..=255`. Uses `i64` intermediates
/// so adversarial quantization/coefficient combinations never overflow in debug builds.
fn jpeg_idct_8x8(block: &[i32; 64]) -> [u8; 64] {
    const CONST_BITS: u32 = 13;
    const PASS1_BITS: u32 = 2;
    const FIX_0_298631336: i64 = 2446;
    const FIX_0_390180644: i64 = 3196;
    const FIX_0_541196100: i64 = 4433;
    const FIX_0_765366865: i64 = 6270;
    const FIX_0_899976223: i64 = 7373;
    const FIX_1_175875602: i64 = 9633;
    const FIX_1_501321110: i64 = 12299;
    const FIX_1_847759065: i64 = 15137;
    const FIX_1_961570560: i64 = 16069;
    const FIX_2_053119869: i64 = 16819;
    const FIX_2_562915447: i64 = 20995;
    const FIX_3_072711026: i64 = 25172;
    let descale = |x: i64, n: u32| x.saturating_add(1i64 << (n - 1)) >> n;

    let mut ws = [0i64; 64];
    for c in 0..8 {
        let d0 = i64::from(block[c]);
        let d1 = i64::from(block[8 + c]);
        let d2 = i64::from(block[16 + c]);
        let d3 = i64::from(block[24 + c]);
        let d4 = i64::from(block[32 + c]);
        let d5 = i64::from(block[40 + c]);
        let d6 = i64::from(block[48 + c]);
        let d7 = i64::from(block[56 + c]);
        if (d1 | d2 | d3 | d4 | d5 | d6 | d7) == 0 {
            let dc = d0.saturating_mul(1 << PASS1_BITS);
            for r in 0..8 {
                ws[r * 8 + c] = dc;
            }
            continue;
        }
        let z1 = (d2 + d6) * FIX_0_541196100;
        let tmp2 = z1 + d6 * -FIX_1_847759065;
        let tmp3 = z1 + d2 * FIX_0_765366865;
        let tmp0 = (d0 + d4) << CONST_BITS;
        let tmp1 = (d0 - d4) << CONST_BITS;
        let tmp10 = tmp0 + tmp3;
        let tmp13 = tmp0 - tmp3;
        let tmp11 = tmp1 + tmp2;
        let tmp12 = tmp1 - tmp2;

        let mut tmp0 = d7;
        let mut tmp1 = d5;
        let mut tmp2 = d3;
        let mut tmp3 = d1;
        let z1 = tmp0 + tmp3;
        let z2 = tmp1 + tmp2;
        let mut z3 = tmp0 + tmp2;
        let mut z4 = tmp1 + tmp3;
        let z5 = (z3 + z4) * FIX_1_175875602;
        tmp0 *= FIX_0_298631336;
        tmp1 *= FIX_2_053119869;
        tmp2 *= FIX_3_072711026;
        tmp3 *= FIX_1_501321110;
        let z1 = z1 * -FIX_0_899976223;
        let z2 = z2 * -FIX_2_562915447;
        z3 = z3 * -FIX_1_961570560 + z5;
        z4 = z4 * -FIX_0_390180644 + z5;
        tmp0 += z1 + z3;
        tmp1 += z2 + z4;
        tmp2 += z2 + z3;
        tmp3 += z1 + z4;

        let shift = CONST_BITS - PASS1_BITS;
        ws[c] = descale(tmp10 + tmp3, shift);
        ws[56 + c] = descale(tmp10 - tmp3, shift);
        ws[8 + c] = descale(tmp11 + tmp2, shift);
        ws[48 + c] = descale(tmp11 - tmp2, shift);
        ws[16 + c] = descale(tmp12 + tmp1, shift);
        ws[40 + c] = descale(tmp12 - tmp1, shift);
        ws[24 + c] = descale(tmp13 + tmp0, shift);
        ws[32 + c] = descale(tmp13 - tmp0, shift);
    }

    let mut out = [0u8; 64];
    let shift = CONST_BITS + PASS1_BITS + 3;
    let clamp = |v: i64| (descale(v, shift).saturating_add(128)).clamp(0, 255) as u8;
    for r in 0..8 {
        let base = r * 8;
        let w0 = ws[base];
        let w1 = ws[base + 1];
        let w2 = ws[base + 2];
        let w3 = ws[base + 3];
        let w4 = ws[base + 4];
        let w5 = ws[base + 5];
        let w6 = ws[base + 6];
        let w7 = ws[base + 7];
        if (w1 | w2 | w3 | w4 | w5 | w6 | w7) == 0 {
            let dc = (descale(w0, PASS1_BITS + 3).saturating_add(128)).clamp(0, 255) as u8;
            out[base..base + 8].fill(dc);
            continue;
        }
        let z1 = (w2 + w6) * FIX_0_541196100;
        let tmp2 = z1 + w6 * -FIX_1_847759065;
        let tmp3 = z1 + w2 * FIX_0_765366865;
        let tmp10 = (w0 + w4) << CONST_BITS;
        let tmp11 = (w0 - w4) << CONST_BITS;
        let t10 = tmp10 + tmp3;
        let t13 = tmp10 - tmp3;
        let t11 = tmp11 + tmp2;
        let t12 = tmp11 - tmp2;

        let mut tmp0 = w7;
        let mut tmp1 = w5;
        let mut tmp2 = w3;
        let mut tmp3 = w1;
        let z1 = tmp0 + tmp3;
        let z2 = tmp1 + tmp2;
        let mut z3 = tmp0 + tmp2;
        let mut z4 = tmp1 + tmp3;
        let z5 = (z3 + z4) * FIX_1_175875602;
        tmp0 *= FIX_0_298631336;
        tmp1 *= FIX_2_053119869;
        tmp2 *= FIX_3_072711026;
        tmp3 *= FIX_1_501321110;
        let z1 = z1 * -FIX_0_899976223;
        let z2 = z2 * -FIX_2_562915447;
        z3 = z3 * -FIX_1_961570560 + z5;
        z4 = z4 * -FIX_0_390180644 + z5;
        tmp0 += z1 + z3;
        tmp1 += z2 + z4;
        tmp2 += z2 + z3;
        tmp3 += z1 + z4;

        out[base] = clamp(t10 + tmp3);
        out[base + 7] = clamp(t10 - tmp3);
        out[base + 1] = clamp(t11 + tmp2);
        out[base + 6] = clamp(t11 - tmp2);
        out[base + 2] = clamp(t12 + tmp1);
        out[base + 5] = clamp(t12 - tmp1);
        out[base + 3] = clamp(t13 + tmp0);
        out[base + 4] = clamp(t13 - tmp0);
    }
    out
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

fn blit_surface_stretched_to_canvas(
    canvas: &mut Canvas,
    surface: &Surface565,
    dst_rect: [i32; 2],
    dst_size: [usize; 2],
    clip: [i32; 4],
    keyed: bool,
) {
    let [dw, dh] = dst_size;
    if !surface.is_valid() || surface.width == 0 || surface.height == 0 || dw == 0 || dh == 0 {
        return;
    }
    let cw = canvas.width as i64;
    let ch = canvas.height as i64;
    let clip_x0 = i64::from(clip[0]).clamp(0, cw);
    let clip_y0 = i64::from(clip[1]).clamp(0, ch);
    let clip_x1 = i64::from(clip[2]).clamp(clip_x0, cw);
    let clip_y1 = i64::from(clip[3]).clamp(clip_y0, ch);

    // Match `cRZGraphic::Blt` (`0x5def36..0x5df028`): subtract any destination
    // overhang outside the target surface and clip rect 1:1 from `src` before
    // calling `IDirectDrawSurface::Blt` (`0x605091`) to stretch the remaining
    // source sub-rectangle into the clipped destination rectangle.
    let mut sx0 = 0i64;
    let mut sy0 = 0i64;
    let mut sx1 = surface.width as i64;
    let mut sy1 = surface.height as i64;
    let mut dx0 = i64::from(dst_rect[0]);
    let mut dy0 = i64::from(dst_rect[1]);
    let mut dx1 = dx0.saturating_add(dw as i64);
    let mut dy1 = dy0.saturating_add(dh as i64);

    if dx1 > clip_x1 {
        let over = dx1 - clip_x1;
        sx1 -= over;
        dx1 -= over;
    }
    if dx0 < clip_x0 {
        let over = clip_x0 - dx0;
        sx0 += over;
        dx0 += over;
    }
    if dy1 > clip_y1 {
        let over = dy1 - clip_y1;
        sy1 -= over;
        dy1 -= over;
    }
    if dy0 < clip_y0 {
        let over = clip_y0 - dy0;
        sy0 += over;
        dy0 += over;
    }

    if dx0 >= dx1
        || dy0 >= dy1
        || sx0 >= sx1
        || sy0 >= sy1
        || sx0 < 0
        || sy0 < 0
        || sx1 > surface.width as i64
        || sy1 > surface.height as i64
    {
        return;
    }

    let sw = (sx1 - sx0) as usize;
    let sh = (sy1 - sy0) as usize;
    let out_w = (dx1 - dx0) as usize;
    let out_h = (dy1 - dy0) as usize;
    let src_x0 = sx0 as usize;
    let src_y0 = sy0 as usize;

    for py in dy0..dy1 {
        let sy = src_y0 + (((py - dy0) as usize) * sh / out_h).min(sh - 1);
        let row = &surface.pixels[sy * surface.width..(sy + 1) * surface.width];
        for px in dx0..dx1 {
            let sx = src_x0 + (((px - dx0) as usize) * sw / out_w).min(sw - 1);
            let pixel = row[sx];
            if keyed && pixel == COLOR_KEY_565 {
                continue;
            }
            canvas.plot_color(px as i32, py as i32, widen_565(pixel));
        }
    }
}

fn blit_color_only(canvas: &mut Canvas, surface: &Surface565, at: [i32; 2], clip: [i32; 4]) {
    blit_color_only_mode(canvas, surface, at, clip, true);
}

fn blit_color_only_mode(
    canvas: &mut Canvas,
    surface: &Surface565,
    at: [i32; 2],
    clip: [i32; 4],
    keyed: bool,
) {
    if !surface.is_valid() {
        return;
    }
    blit_subrect_to_canvas_mode(
        canvas,
        surface,
        [0, 0, surface.width, surface.height],
        at,
        clip,
        keyed,
    );
}

fn blit_subrect_to_canvas(
    canvas: &mut Canvas,
    surface: &Surface565,
    src_rect: [usize; 4],
    at: [i32; 2],
    clip: [i32; 4],
) {
    blit_subrect_to_canvas_mode(canvas, surface, src_rect, at, clip, true);
}

fn blit_subrect_to_canvas_mode(
    canvas: &mut Canvas,
    surface: &Surface565,
    src_rect: [usize; 4],
    at: [i32; 2],
    clip: [i32; 4],
    keyed: bool,
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
            if keyed && pixel == COLOR_KEY_565 {
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
            backdrop_size: [MAP_WIDTH, MAP_HEIGHT],
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
            opaque_overlays: false,
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

        // Expansion tables (Downtown/Vacation/StudioTown/MagicTown) keep the
        // last matching row (0x4e215a..0x4e241a), and lookup dispatches by lot
        // ID across all six STR# tables with [0, 0] fallback (0x4e20a9..0x4e2440).
        let dt_table = StringTable {
            format: Format::Pascal,
            entries: vec![entry("21, 187, 236"), entry("21, 359, 152")],
        };
        let all_locs = LotLocations {
            classic: parse_lot_locations_table(&StringTable {
                format: Format::Pascal,
                entries: vec![entry("5, 11, 22")],
            }),
            downtown: parse_lot_locations_table_with(&dt_table, true, Some(49)),
            unleashed: locs,
            ..LotLocations::default()
        };
        // Unleashed screen listing Lot 21 still resolves via STR# 2 (last match).
        assert_eq!(all_locs.anchor_for(21, true), [359, 152]);
        assert_eq!(all_locs.anchor_for(5, true), [300, 120]);
        assert_eq!(all_locs.anchor_for(5, false), [11, 22]);
        // Missing STR# 7 (Magic Town) or unmapped lot ID retains [0, 0].
        assert_eq!(all_locs.anchor_for(90, false), [0, 0]);
        assert_eq!(all_locs.anchor_for(15, true), [0, 0]);

        // Legacy \0CST caret-delimited string table parsing (0x470178..0x470209).
        let cst = parse_cst_chunk(b"ignored^21, 410, 315^noise^22, 10, 20^unclosed");
        let cst_dt = parse_lot_locations_table_with(&cst, true, Some(49));
        assert_eq!(cst_dt.get(&21), Some(&[410, 315]));
        assert_eq!(cst_dt.get(&22), Some(&[10, 20]));

        // Interleaved language blocks in version -3 (Format::Localized) reset
        // insertion index on language change, putting later English blocks ahead
        // of earlier ones (0x470002, 0x470482).
        let entry_lang = |lang: u8, s: &str| TextEntry {
            language: lang,
            text: s.to_string(),
            comment: String::new(),
            raw_text: s.as_bytes().to_vec(),
            raw_comment: Vec::new(),
        };
        let interleaved = StringTable {
            format: Format::Localized,
            entries: vec![
                entry_lang(US_ENGLISH, "1, 235, 245"),
                entry_lang(2, "1, 999, 999"),
                entry_lang(US_ENGLISH, "1, 435, 445"),
            ],
        };
        let interleaved_locs = parse_lot_locations_table(&interleaved);
        assert_eq!(interleaved_locs.get(&1), Some(&[435, 445]));

        // Native IFF opener rejects damaged header bytes after "IFF FILE 2.5:"
        // and invalid nonzero resource-map offsets (0x41c2d6..0x41c4f7), while
        // keeping indexed files with trailing bytes or differing physical chunk
        // IDs (0x41c500, 0x41d33b).
        let valid_iff = sims_formats::iff::write(&[]).expect("write empty IFF");
        assert!(NativeIff::parse(valid_iff.clone()).is_some());
        let mut bad_sig = valid_iff.clone();
        bad_sig[20] ^= 0xff;
        assert!(NativeIff::parse(bad_sig).is_none());
        let mut bad_rsmp = valid_iff;
        bad_rsmp[60..64].copy_from_slice(&0x1234u32.to_be_bytes());
        assert!(NativeIff::parse(bad_rsmp).is_none());

        // Build an indexed IFF 2.5 where rsmp maps STR# 4 to offset 64 even
        // though the physical chunk at 64 carries ID 99, plus 1 trailing byte.
        let str_payload = b"\xff\xff\x01\x001, 481, 84\0".to_vec();
        let mut rsmp_payload = Vec::new();
        rsmp_payload.extend_from_slice(&[0u8; 8]);
        rsmp_payload.extend_from_slice(b"pmsr");
        rsmp_payload.extend_from_slice(&0u32.to_le_bytes());
        rsmp_payload.extend_from_slice(&1u32.to_le_bytes()); // 1 type
        rsmp_payload.extend_from_slice(&u32::from_be_bytes(*b"STR#").to_le_bytes());
        rsmp_payload.extend_from_slice(&1u32.to_le_bytes()); // 1 entry
        rsmp_payload.extend_from_slice(&64u32.to_le_bytes()); // offset 64
        rsmp_payload.extend_from_slice(&4u16.to_le_bytes()); // map ID = 4
        rsmp_payload.extend_from_slice(&0x0010u16.to_le_bytes()); // flags
        rsmp_payload.extend_from_slice(&[0u8, 0u8]); // empty NUL label + pad
        rsmp_payload.extend_from_slice(&[0xde, 0xad]); // trailing padding inside rsmp (0x41c038)
        let rsmp_offset = 64 + 76 + str_payload.len();
        let mut indexed = sims_formats::iff::write(&[
            sims_formats::iff::ChunkData {
                tag: *b"STR#",
                id: 99, // differs from rsmp entry ID 4
                flags: 0x0010,
                label: String::new(),
                data: str_payload,
            },
            sims_formats::iff::ChunkData {
                tag: *b"rsmp",
                id: 0,
                flags: 0,
                label: String::new(),
                data: rsmp_payload,
            },
        ])
        .expect("write indexed IFF");
        indexed[60..64].copy_from_slice(&(rsmp_offset as u32).to_be_bytes());
        indexed.push(0xaa); // trailing byte ignored when rsmp skips physical scan
        let parsed_indexed = NativeIff::parse(indexed).expect("indexed IFF opens");
        let tbl = load_native_string_table(&parsed_indexed, 4).expect("STR# 4 via rsmp");
        assert_eq!(parse_lot_locations_table(&tbl).get(&1), Some(&[481, 84]));

        // Missing SIMI 1 rejects the lot (0x427f21), SIMI version 0x4a is
        // accepted with globals[37] read (0x46cfb3), and a corrupt optional
        // BMP_ 513 still leaves an intact BMP_ 512 drawable as a single cell
        // (0x4ded62..0x4ded72, 0x54e685..0x54e6a7).
        let bmp_1x1 = {
            // Minimal valid uncompressed 24-bit 1×1 BMP.
            let mut b = vec![0u8; 58];
            b[0..2].copy_from_slice(b"BM");
            b[2..6].copy_from_slice(&58u32.to_le_bytes());
            b[10..14].copy_from_slice(&54u32.to_le_bytes());
            b[14..18].copy_from_slice(&40u32.to_le_bytes());
            b[18..22].copy_from_slice(&1i32.to_le_bytes());
            b[22..26].copy_from_slice(&1i32.to_le_bytes());
            b[26..28].copy_from_slice(&1u16.to_le_bytes());
            b[28..30].copy_from_slice(&24u16.to_le_bytes());
            b[54..57].copy_from_slice(&[0x10, 0x20, 0x30]);
            b
        };
        let house_bytes = sims_formats::iff::write(&[
            sims_formats::iff::ChunkData {
                tag: *b"BMP_",
                id: 512,
                flags: 0,
                label: String::new(),
                data: bmp_1x1,
            },
            sims_formats::iff::ChunkData {
                tag: *b"BMP_",
                id: 513,
                flags: 0,
                label: String::new(),
                data: b"corrupt".to_vec(),
            },
        ])
        .expect("write synthetic house IFF");
        let house_iff = NativeIff::parse(house_bytes).expect("synthetic house opens");
        assert_eq!(read_lot_filter_mask(&house_iff), None);
        let built = build_lot_cells(&house_iff, false, None)
            .expect("corrupt optional BMP_ 513 does not error")
            .expect("intact BMP_ 512 remains drawable");
        assert_eq!((built.normal.width, built.normal.height), (1, 1));
        assert!(built.roofs_off.is_none());

        let mut simi_4a = vec![0u8; 12 + 64 * 2];
        simi_4a[4..8].copy_from_slice(&0x4au32.to_le_bytes());
        simi_4a[8..12].copy_from_slice(b"IMIS");
        simi_4a[12 + 37 * 2..12 + 38 * 2].copy_from_slice(&0x0005u16.to_le_bytes());
        let simi_iff = NativeIff::parse(
            sims_formats::iff::write(&[sims_formats::iff::ChunkData {
                tag: *b"SIMI",
                id: 1,
                flags: 0x0010,
                label: String::new(),
                data: simi_4a,
            }])
            .expect("write SIMI 0x4a IFF"),
        )
        .expect("SIMI 0x4a IFF opens");
        assert_eq!(read_lot_filter_mask(&simi_iff), Some(0x0005));

        // Also verify an unpadded final rsmp entry (8 fixed bytes + single NUL
        // byte, no trailing pad byte past EOF) still opens (0x464c3e).
        let mut unpadded_rsmp = Vec::new();
        unpadded_rsmp.extend_from_slice(&[0u8; 8]);
        unpadded_rsmp.extend_from_slice(b"pmsr");
        unpadded_rsmp.extend_from_slice(&0u32.to_le_bytes());
        unpadded_rsmp.extend_from_slice(&1u32.to_le_bytes());
        unpadded_rsmp.extend_from_slice(&u32::from_be_bytes(*b"STR#").to_le_bytes());
        unpadded_rsmp.extend_from_slice(&1u32.to_le_bytes());
        unpadded_rsmp.extend_from_slice(&64u32.to_le_bytes());
        unpadded_rsmp.extend_from_slice(&4u16.to_le_bytes());
        unpadded_rsmp.extend_from_slice(&0x0010u16.to_le_bytes());
        unpadded_rsmp.push(0u8); // NUL with no alignment pad byte
        let mut unpadded_iff = sims_formats::iff::write(&[
            sims_formats::iff::ChunkData {
                tag: *b"STR#",
                id: 99,
                flags: 0x0010,
                label: String::new(),
                data: b"\xff\xff\x01\x005, 481, 84\0".to_vec(),
            },
            sims_formats::iff::ChunkData {
                tag: *b"rsmp",
                id: 0,
                flags: 0,
                label: String::new(),
                data: unpadded_rsmp,
            },
        ])
        .expect("write unpadded rsmp IFF");
        let unpadded_off = 64 + 76 + b"\xff\xff\x01\x005, 481, 84\0".len();
        unpadded_iff[60..64].copy_from_slice(&(unpadded_off as u32).to_be_bytes());
        let parsed_unpadded = NativeIff::parse(unpadded_iff).expect("unpadded rsmp opens");
        let utbl = load_native_string_table(&parsed_unpadded, 4).expect("STR# 4 via unpadded rsmp");
        assert_eq!(parse_lot_locations_table(&utbl).get(&5), Some(&[481, 84]));

        // Big-endian SIMI 1 (byte[0] = 1) with non-IMIS bytes 8..12 (0x464edd, 0x464936).
        let mut simi_be = vec![0u8; 12 + 64 * 2];
        simi_be[0] = 1;
        simi_be[4..8].copy_from_slice(&0x45u32.to_le_bytes());
        simi_be[8..12].copy_from_slice(b"XXXX");
        simi_be[12 + 37 * 2..12 + 38 * 2].copy_from_slice(&0x0002u16.to_be_bytes());
        let simi_be_iff = NativeIff::parse(
            sims_formats::iff::write(&[sims_formats::iff::ChunkData {
                tag: *b"SIMI",
                id: 1,
                flags: 0x0010,
                label: String::new(),
                data: simi_be,
            }])
            .expect("write big-endian SIMI IFF"),
        )
        .expect("big-endian SIMI IFF opens");
        assert_eq!(read_lot_filter_mask(&simi_be_iff), Some(0x0002));
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

        // Occupancy: native GetFamilyByLot (0x425d95) adds default family 0 when
        // absent (0x424c65..0x424cb5), sorts descending by (number, id) (0x425209),
        // looks up family ID 0 for lot 0, and picks the first matching family.
        let mut occ = FamilyOccupancy::default();
        occ.record_family_houses(1, 10, 5, 0);
        occ.record_family_houses(9, 10, 5, 41); // same number, higher ID wins over 1
        occ.record_family_houses(4, 20, 40, 0); // higher number
        occ.finalize_native();
        // Default family 0 was synthesized even though no FAMI #0 was recorded.
        assert_eq!(
            resolve_lot_status(0, NeighborhoodKind::Unleashed, LotZoning::Residential, &occ),
            LotStatus::Occupied {
                family_id: 0,
                vacation_lot: None,
            }
        );
        assert_eq!(
            resolve_lot_status(40, NeighborhoodKind::Vacation, LotZoning::Community, &occ),
            LotStatus::Community {
                occupied_by_family: Some(4)
            }
        );
        assert_eq!(
            resolve_lot_status(41, NeighborhoodKind::Vacation, LotZoning::Community, &occ),
            LotStatus::Community {
                occupied_by_family: Some(9)
            }
        );
        assert_eq!(
            resolve_lot_status(5, NeighborhoodKind::Unleashed, LotZoning::Residential, &occ),
            LotStatus::Occupied {
                family_id: 9,
                vacation_lot: Some(41),
            }
        );

        // On Unleashed, a residential lot 41 that is ONLY a family's secondary
        // house must NOT trigger the vacation badge (0x54eb49..0x54eb64).
        let mut sec_only_lot = make_lot(41, [10, 10], on.clone());
        sec_only_lot.vacation_badged = Some(badged.clone());
        sec_only_lot.status = resolve_lot_status(
            41,
            NeighborhoodKind::Unleashed,
            LotZoning::Residential,
            &occ,
        );
        sec_only_lot.has_primary_vacation_stay = occ.has_primary_vacation_stay(41);
        sec_only_lot.has_vacation_occupancy = occ.has_vacation_occupancy(41);
        assert!(!sec_only_lot.has_primary_vacation_stay);
        assert!(sec_only_lot.has_vacation_occupancy);
        let unleashed_sec_map = make_test_map(NeighborhoodKind::Unleashed, vec![sec_only_lot]);
        assert!(!unleashed_sec_map.has_vacation_badge(&unleashed_sec_map.lots[0]));

        // Custom 600×400 backdrop centers at (100, 100) in an 800×600 viewport
        // (0x568fa7..0x568fe6), numbered wave slots skip missing slot 1
        // (0x56bb7a..0x56bb8d), and Vacation port without trees is not drawn (0x56bac8).
        let mut custom_vac = make_test_map(
            NeighborhoodKind::Vacation,
            vec![make_lot(
                40,
                [0, 0],
                Surface565::filled(4, 4, pack_565([10, 200, 10])),
            )],
        );
        let bg_c = pack_565([20, 40, 80]);
        let wave2_c = pack_565([200, 200, 20]);
        let port_c = pack_565([240, 40, 240]);
        custom_vac.backdrop = Surface565::filled(600, 400, bg_c);
        custom_vac.lots[0].backdrop_size = [600, 400];
        custom_vac.large_frame = Some(Surface565::filled(800, 600, pack_565([240, 0, 0])));
        custom_vac.waves = vec![
            Some(Surface565::filled(600, 400, COLOR_KEY_565)),
            None,
            Some(Surface565::filled(600, 400, wave2_c)),
        ];
        custom_vac.opaque_overlays = true;
        custom_vac.vacation_port = Some(Surface565::filled(600, 400, port_c));
        custom_vac.vacation_trees = None;
        assert_eq!(custom_vac.map_origin_for([800, 600]), [100, 100]);
        assert_eq!(
            custom_vac.lots[0].screen_rect([800, 600]),
            [100, 100, 104, 104]
        );
        assert_eq!(
            custom_vac.lot_screen_rect(&custom_vac.lots[0], [800, 600]),
            [100, 100, 104, 104]
        );
        assert_eq!(custom_vac.hit_test([800, 600], [100, 100]), Some(40));
        assert_eq!(custom_vac.hit_test([800, 600], [0, 0]), None);
        let vac_frame1 = custom_vac.render(&RenderOptions {
            viewport: [800, 600],
            water_frame: Some(1),
            show_vacation_overlays: true,
            ..RenderOptions::default()
        });
        // Outer frame requires viewport width > 800 (0x569025), so (0, 0) stays black.
        assert_eq!(vac_frame1.color[0], [0, 0, 0]);
        // Frame 1 is None (not shifted to frame 2) and port alone is suppressed,
        // so (110, 110) stays backdrop color rather than wave2_c or port_c.
        assert_eq!(vac_frame1.color[110 * 800 + 110], widen_565(bg_c));
        // Local JPEG overlays keep color-keying disabled (0x4b8285), so solid
        // magenta (0xf81f) in frame 0 draws opaquely rather than transparent.
        let vac_frame0 = custom_vac.render(&RenderOptions {
            viewport: [800, 600],
            water_frame: Some(0),
            show_vacation_overlays: true,
            ..RenderOptions::default()
        });
        assert_eq!(vac_frame0.color[110 * 800 + 110], widen_565(COLOR_KEY_565));
        let vac_frame2 = custom_vac.render(&RenderOptions {
            viewport: [800, 600],
            water_frame: Some(2),
            show_vacation_overlays: true,
            ..RenderOptions::default()
        });
        assert_eq!(vac_frame2.color[110 * 800 + 110], widen_565(wave2_c));

        // Filter badges use a null/full-viewport clip (0x56bec5) so edge-lot
        // badges extend into the 1024×768 outer frame; Classic waves clip to
        // the centered backdrop rectangle (0x55f95f..0x55f98a).
        let mut badge_vac = make_test_map(
            NeighborhoodKind::Vacation,
            vec![{
                let mut l = make_lot(
                    40,
                    [10, 10],
                    Surface565::filled(4, 4, pack_565([0, 200, 0])),
                );
                l.filter_mask = CommunityFilter::Lodging.bit();
                l
            }],
        );
        let badge_px = pack_565([250, 220, 0]);
        badge_vac.filter_badges.insert(
            CommunityFilter::Lodging,
            Surface565::filled(12, 8, badge_px), // 3 cells of 4×8
        );
        // Lot 40 at anchor (10, 10) with Vacation offset (-24, -10) -> map (-14, 0),
        // screen (112 - 14, 84) = (98, 84), outside the 800×600 lot clip x >= 112.
        let badge_1024 = badge_vac.render(&RenderOptions {
            viewport: [1024, 768],
            active_filter: Some(CommunityFilter::Lodging),
            ..RenderOptions::default()
        });
        assert_eq!(badge_1024.color[84 * 1024 + 98], widen_565(badge_px));

        let mut classic_small = make_test_map(NeighborhoodKind::Classic, Vec::new());
        let frame_c = pack_565([40, 40, 40]);
        let left_c = pack_565([20, 200, 20]);
        let right_c = pack_565([200, 20, 200]);
        classic_small.backdrop = Surface565::filled(600, 400, bg_c);
        classic_small.large_frame = Some(Surface565::filled(1024, 768, frame_c));
        // Classic stretches the full wave into the centered 600×400 backdrop
        // (0x55f95f..0x55f98a -> 0x605091), so a 2-pixel-wide [left_c, right_c]
        // wave maps to x=212..512 (left_c) and x=512..812 (right_c), leaving
        // x=850 as outer frame.
        classic_small.waves = vec![Some(Surface565 {
            width: 2,
            height: 1,
            pixels: vec![left_c, right_c],
        })];
        classic_small.opaque_overlays = true;
        let classic_1024 = classic_small.render(&RenderOptions {
            viewport: [1024, 768],
            water_frame: Some(0),
            ..RenderOptions::default()
        });
        assert_eq!(classic_1024.color[200 * 1024 + 850], widen_565(frame_c));
        assert_eq!(classic_1024.color[200 * 1024 + 300], widen_565(left_c));
        assert_eq!(classic_1024.color[200 * 1024 + 700], widen_565(right_c));

        // When Classic's backdrop exceeds the viewport (900px backdrop, 800px
        // wave, 800px viewport), `cRZGraphic::Blt` (`0x5def65..0x5def81`)
        // subtracts the 100px right overhang from `src.right`, cropping the
        // wave to columns `0..700` before stretching to `0..800`.
        let mut classic_wide = make_test_map(NeighborhoodKind::Classic, Vec::new());
        let green_c = pack_565([20, 220, 20]);
        let red_c = pack_565([240, 20, 20]);
        classic_wide.backdrop = Surface565::filled(900, 600, bg_c);
        let mut striped_wave = Surface565::filled(800, 600, green_c);
        for y in 0..600 {
            for x in 700..800 {
                striped_wave.pixels[y * 800 + x] = red_c;
            }
        }
        classic_wide.waves = vec![Some(striped_wave)];
        classic_wide.opaque_overlays = true;
        let classic_800 = classic_wide.render(&RenderOptions {
            viewport: [800, 600],
            water_frame: Some(0),
            ..RenderOptions::default()
        });
        assert!(
            classic_800.color.iter().all(|&c| c == widen_565(green_c)),
            "rightmost 100 wave columns must be cropped out by 0x5def65"
        );

        // Downtown and Vacation overlays pass a null clip and use their own
        // dimensions (0x54cb3e, 0x56baf4, 0x56bb29, 0x56bb92), so an 800×600
        // wave over a 600×400 backdrop at origin (212, 184) draws at (850, 200).
        // Their playback cycles `0,1,2,3,4,4,3,2,1,0` (`0x54caf5`, `0x56baa5`),
        // so `water_frame: Some(5)` draws slot 4 and never slot 5.
        let mut dt_small = make_test_map(NeighborhoodKind::Downtown, Vec::new());
        let slot4_c = pack_565([10, 180, 240]);
        let slot5_c = pack_565([240, 10, 10]);
        dt_small.backdrop = Surface565::filled(600, 400, bg_c);
        dt_small.large_frame = Some(Surface565::filled(1024, 768, frame_c));
        dt_small.waves = vec![
            Some(Surface565::filled(800, 600, wave2_c)),
            None,
            None,
            None,
            Some(Surface565::filled(800, 600, slot4_c)),
            Some(Surface565::filled(800, 600, slot5_c)),
        ];
        dt_small.opaque_overlays = true;
        let dt_1024 = dt_small.render(&RenderOptions {
            viewport: [1024, 768],
            water_frame: Some(0),
            ..RenderOptions::default()
        });
        assert_eq!(dt_1024.color[200 * 1024 + 850], widen_565(wave2_c));
        let dt_step5 = dt_small.render(&RenderOptions {
            viewport: [1024, 768],
            water_frame: Some(5),
            ..RenderOptions::default()
        });
        assert_eq!(dt_step5.color[200 * 1024 + 850], widen_565(slot4_c));
        let mut vac_cycle = make_test_map(NeighborhoodKind::Vacation, Vec::new());
        vac_cycle.waves = vec![
            Some(Surface565::filled(10, 10, wave2_c)),
            None,
            None,
            None,
            Some(Surface565::filled(10, 10, slot4_c)),
            Some(Surface565::filled(10, 10, slot5_c)),
        ];
        vac_cycle.opaque_overlays = true;
        let vac_step5 = vac_cycle.render(&RenderOptions {
            water_frame: Some(5),
            ..RenderOptions::default()
        });
        assert_eq!(vac_step5.color[0], widen_565(slot4_c));

        // Unleashed skips widget updates when a custom wave slot is absent
        // (`0x557e87..0x557e90`), keeping the previous wave visible on slot 1.
        let mut ul_sparse = make_test_map(NeighborhoodKind::Unleashed, Vec::new());
        ul_sparse.waves = vec![
            Some(Surface565::filled(10, 10, wave2_c)),
            None,
            Some(Surface565::filled(10, 10, slot4_c)),
            None,
            None,
            None,
        ];
        ul_sparse.opaque_overlays = true;
        let ul_step1 = ul_sparse.render(&RenderOptions {
            water_frame: Some(1),
            ..RenderOptions::default()
        });
        assert_eq!(ul_step1.color[0], widen_565(wave2_c));

        // 4-component CMYK baseline JPEG decodes through the same YCbCr->RGB
        // path as shipped `gimex.dll` + `ijl10.dll` (`JPGColor = IJL_YCBCR`,
        // channel 4 ignored): (255, 0, 0, 255) -> RGB565 `0x4fe3`.
        let cmyk_1x1_jpeg: &[u8] = &[
            0xff, 0xd8, // SOI
            0xff, 0xee, 0x00, 0x0e, b'A', b'd', b'o', b'b', b'e', 0x00, 0x64, 0x00, 0x00, 0x00,
            0x00, 0x00, // Adobe APP14 transform=0 (CMYK)
            0xff, 0xdb, 0x00, 0x43, 0x00, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
            1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
            1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
            // SOF0: 8-bit, 1×1, 4 components (ids 1..=4, 1×1, qt=0)
            0xff, 0xc0, 0x00, 0x14, 0x08, 0x00, 0x01, 0x00, 0x01, 0x04, 0x01, 0x11, 0x00, 0x02,
            0x11, 0x00, 0x03, 0x11, 0x00, 0x04, 0x11, 0x00,
            // DHT DC 0: 1 symbol length 1 (0 -> size 10)
            0xff, 0xc4, 0x00, 0x14, 0x00, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x0a,
            // DHT AC 0: 1 symbol length 1 (0 -> EOB)
            0xff, 0xc4, 0x00, 0x14, 0x10, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x00,
            // SOS: 4 components (all dc=0, ac=0)
            0xff, 0xda, 0x00, 0x0e, 0x04, 0x01, 0x00, 0x02, 0x00, 0x03, 0x00, 0x04, 0x00, 0x00,
            0x3f, 0x00,
            // 4 blocks: (+1016 => 255), (-1024 => 0), (-1024 => 0), (+1016 => 255)
            0x7f, 0x80, 0x00, 0x00, 0x07, 0xf8, 0xff, 0xd9,
        ];
        let cmyk_decoded = decode_jpeg_to_surface565(cmyk_1x1_jpeg).expect("decode 4-comp CMYK");
        assert_eq!((cmyk_decoded.width, cmyk_decoded.height), (1, 1));
        assert_eq!(cmyk_decoded.pixels[0], 0x4fe3);

        // Header-only JPEG with zero SOS image scans must be rejected so the
        // loader falls back to bundled artwork rather than drawing gray.
        let zero_scan_jpeg: &[u8] = &[
            0xff, 0xd8, // SOI
            0xff, 0xdb, 0x00, 0x43, 0x00, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
            1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
            1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // SOF0: 8×8, 1 comp, NO SOS!
            0xff, 0xc0, 0x00, 0x0b, 0x08, 0x00, 0x08, 0x00, 0x08, 0x01, 0x01, 0x11, 0x00, 0xff,
            0xd9, // EOI
        ];
        assert!(decode_jpeg_to_surface565(zero_scan_jpeg).is_none());

        // On a 900px-wide backdrop, lot clip remains 800×600 from origin (0x56928b).
        let mut wide_map = make_test_map(
            NeighborhoodKind::Vacation,
            vec![make_lot(
                40,
                [850, 10],
                Surface565::filled(10, 10, pack_565([250, 250, 0])),
            )],
        );
        wide_map.backdrop = Surface565::filled(900, 600, bg_c);
        let wide_canvas = wide_map.render(&RenderOptions {
            viewport: [900, 600],
            ..RenderOptions::default()
        });
        assert_eq!(wide_canvas.color[15 * 900 + 855], widen_565(bg_c));

        // Version-0 FAMI retains default number = -1 (0x410083, 0x4104f9), so a
        // version-1 family #9 with number 0 beats version-0 family #10 on lot 5.
        let mut v0_fami = Vec::new();
        v0_fami.extend_from_slice(&0u32.to_le_bytes());
        v0_fami.extend_from_slice(&0i32.to_le_bytes()); // version 0
        v0_fami.extend_from_slice(b"IMAF");
        v0_fami.extend_from_slice(&5i32.to_le_bytes()); // house 5
        v0_fami.extend_from_slice(&[0u8, 0u8]); // empty NUL name + pad
        v0_fami.extend_from_slice(&0i32.to_le_bytes()); // 0 members
        let mut v1_fami = Vec::new();
        v1_fami.extend_from_slice(&0u32.to_le_bytes());
        v1_fami.extend_from_slice(&1i32.to_le_bytes()); // version 1
        v1_fami.extend_from_slice(b"IMAF");
        v1_fami.extend_from_slice(&5i32.to_le_bytes()); // house 5
        v1_fami.extend_from_slice(&[0u8, 0u8]); // empty NUL name + pad
        v1_fami.extend_from_slice(&0i32.to_le_bytes()); // number = 0
        v1_fami.extend_from_slice(&0i32.to_le_bytes()); // 0 members
        let mut legacy_occ = FamilyOccupancy::default();
        legacy_occ.record_fami_chunk(10, &v0_fami, 0x0010);
        legacy_occ.record_fami_chunk(9, &v1_fami, 0x0010);
        legacy_occ.finalize_native();
        assert_eq!(
            resolve_lot_status(
                5,
                NeighborhoodKind::Unleashed,
                LotZoning::Residential,
                &legacy_occ
            ),
            LotStatus::Occupied {
                family_id: 9,
                vacation_lot: None,
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

        // Multiple FAMI records sharing lot 5 (secondary 41 followed by 0),
        // negative nonzero secondary (-1), lot 0, and FAMI version 10 (no upper
        // version check at 0x410602) all preserve vacation stay eligibility (0x54f20e),
        // while non-default FAMI variants (flags >> 8 != 0) are ignored (0x41bad8).
        let make_fami_chunk = |version: i32, family_id: i32, house: i32, secondary: i32| {
            let mut b = Vec::new();
            b.extend_from_slice(&0u32.to_le_bytes()); // LE marker
            b.extend_from_slice(&version.to_le_bytes());
            b.extend_from_slice(b"IMAF");
            b.extend_from_slice(&house.to_le_bytes());
            b.extend_from_slice(&1i32.to_le_bytes()); // number
            b.extend_from_slice(&0i32.to_le_bytes()); // budget
            b.extend_from_slice(&0i32.to_le_bytes()); // arch
            b.extend_from_slice(&0i32.to_le_bytes()); // friends
            b.extend_from_slice(&0u32.to_le_bytes()); // flags
            if family_id != 0 {
                b.extend_from_slice(&0i32.to_le_bytes()); // 0 members
            }
            if version >= 8 {
                b.extend_from_slice(&secondary.to_le_bytes());
            }
            if version >= 9 {
                // Six trailing magic-history shorts consumed at 0x41063d.
                b.extend_from_slice(&[0u8; 12]);
            }
            b
        };
        let fami_iff = NativeIff::parse(
            sims_formats::iff::write(&[
                sims_formats::iff::ChunkData {
                    tag: *b"FAMI",
                    id: 5,
                    flags: 0x0110, // variant 1: must be ignored (0x41bad8)
                    label: String::new(),
                    data: make_fami_chunk(9, 5, 5, 45),
                },
                sims_formats::iff::ChunkData {
                    tag: *b"FAMI",
                    id: 1,
                    flags: 0x0010,
                    label: String::new(),
                    data: make_fami_chunk(8, 1, 5, 41),
                },
                sims_formats::iff::ChunkData {
                    tag: *b"FAMI",
                    id: 2,
                    flags: 0x0010,
                    label: String::new(),
                    data: make_fami_chunk(8, 2, 5, 0),
                },
                sims_formats::iff::ChunkData {
                    tag: *b"FAMI",
                    id: 3,
                    flags: 0x0010,
                    label: String::new(),
                    data: make_fami_chunk(9, 3, 6, -1),
                },
                sims_formats::iff::ChunkData {
                    tag: *b"FAMI",
                    id: 4,
                    flags: 0x0010,
                    label: String::new(),
                    data: make_fami_chunk(9, 4, 0, 41),
                },
                sims_formats::iff::ChunkData {
                    tag: *b"FAMI",
                    id: 6,
                    flags: 0x0010,
                    label: String::new(),
                    data: make_fami_chunk(10, 6, 7, 42),
                },
            ])
            .expect("write FAMI test IFF"),
        )
        .expect("FAMI test IFF opens");
        let mut multi_occ = FamilyOccupancy::default();
        for (id, data, flags) in fami_iff.all_variant_zero_sorted(b"FAMI") {
            multi_occ.record_fami_chunk(i32::from(id), data, flags);
        }
        assert!(multi_occ.has_primary_vacation_stay(5));
        assert!(multi_occ.has_primary_vacation_stay(6));
        assert!(multi_occ.has_primary_vacation_stay(0));
        assert!(multi_occ.has_primary_vacation_stay(7));
        assert!(multi_occ.has_vacation_occupancy(41));
        assert!(multi_occ.has_vacation_occupancy(42));
        // Variant-1 FAMI #5 referencing lot 45 was ignored.
        assert!(!multi_occ.has_vacation_occupancy(45));

        // Verify pure-Rust JPEG decoder handles a valid baseline 1×1 white JPEG.
        // Minimal standard baseline JFIF 1×1 grayscale JPEG (DC = +127 => white 255).
        let white_1x1_jpeg: &[u8] = &[
            0xff, 0xd8, // SOI
            0xff, 0xdb, 0x00, 0x43, 0x00, // DQT table 0 (all 1s)
            1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
            1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
            1, 1, 1, 1, 1, 1, // SOF0: 8-bit, 1×1, 1 component (id=1, 1×1, qt=0)
            0xff, 0xc0, 0x00, 0x0b, 0x08, 0x00, 0x01, 0x00, 0x01, 0x01, 0x01, 0x11, 0x00,
            // DHT DC table 0: 1 symbol of length 1 (code 0 -> size 10)
            0xff, 0xc4, 0x00, 0x14, 0x00, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x0a,
            // DHT AC table 0: 1 symbol of length 1 (code 0 -> EOB 0x00)
            0xff, 0xc4, 0x00, 0x14, 0x10, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x00,
            // SOS: 1 comp (id=1, dc=0, ac=0), ss=0, se=63, ah/al=0
            0xff, 0xda, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3f, 0x00,
            // Entropy bits: DC code '0' + 10 bits '1111111000' (+1016 => 1016/8 + 128 = 255)
            // + AC EOB '0' + 4 pad bits '1111' => 0111_1111 1000_0111 = 0x7f, 0x87
            0x7f, 0x87, // EOI
            0xff, 0xd9,
        ];
        let decoded = decode_jpeg_to_surface565(white_1x1_jpeg).expect("decode 1×1 JPEG");
        assert_eq!((decoded.width, decoded.height), (1, 1));
        assert_eq!(decoded.pixels[0], pack_565([255, 255, 255]));

        // Real 16×16 3-component YCbCr 4:2:0 solid-red baseline JPEG (with DRI=1).
        let red_16x16_jpeg: &[u8] = &[
            0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, 0x4a, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00,
            0x00, 0x48, 0x00, 0x48, 0x00, 0x00, 0xff, 0xe1, 0x00, 0x4c, 0x45, 0x78, 0x69, 0x66,
            0x00, 0x00, 0x4d, 0x4d, 0x00, 0x2a, 0x00, 0x00, 0x00, 0x08, 0x00, 0x01, 0x87, 0x69,
            0x00, 0x04, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x1a, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x03, 0xa0, 0x01, 0x00, 0x03, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00,
            0xa0, 0x02, 0x00, 0x04, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x10, 0xa0, 0x03,
            0x00, 0x04, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00,
            0xff, 0xed, 0x00, 0x38, 0x50, 0x68, 0x6f, 0x74, 0x6f, 0x73, 0x68, 0x6f, 0x70, 0x20,
            0x33, 0x2e, 0x30, 0x00, 0x38, 0x42, 0x49, 0x4d, 0x04, 0x04, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x38, 0x42, 0x49, 0x4d, 0x04, 0x25, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10,
            0xd4, 0x1d, 0x8c, 0xd9, 0x8f, 0x00, 0xb2, 0x04, 0xe9, 0x80, 0x09, 0x98, 0xec, 0xf8,
            0x42, 0x7e, 0xff, 0xc0, 0x00, 0x11, 0x08, 0x00, 0x10, 0x00, 0x10, 0x03, 0x01, 0x22,
            0x00, 0x02, 0x11, 0x01, 0x03, 0x11, 0x01, 0xff, 0xc4, 0x00, 0x1f, 0x00, 0x00, 0x01,
            0x05, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0xff, 0xc4,
            0x00, 0xb5, 0x10, 0x00, 0x02, 0x01, 0x03, 0x03, 0x02, 0x04, 0x03, 0x05, 0x05, 0x04,
            0x04, 0x00, 0x00, 0x01, 0x7d, 0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21,
            0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07, 0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xa1,
            0x08, 0x23, 0x42, 0xb1, 0xc1, 0x15, 0x52, 0xd1, 0xf0, 0x24, 0x33, 0x62, 0x72, 0x82,
            0x09, 0x0a, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x34,
            0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4a,
            0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68,
            0x69, 0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x83, 0x84, 0x85, 0x86,
            0x87, 0x88, 0x89, 0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2,
            0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7,
            0xb8, 0xb9, 0xba, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3,
            0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda, 0xe1, 0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7,
            0xe8, 0xe9, 0xea, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0xfa, 0xff,
            0xc4, 0x00, 0x1f, 0x01, 0x00, 0x03, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01,
            0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
            0x08, 0x09, 0x0a, 0x0b, 0xff, 0xc4, 0x00, 0xb5, 0x11, 0x00, 0x02, 0x01, 0x02, 0x04,
            0x04, 0x03, 0x04, 0x07, 0x05, 0x04, 0x04, 0x00, 0x01, 0x02, 0x77, 0x00, 0x01, 0x02,
            0x03, 0x11, 0x04, 0x05, 0x21, 0x31, 0x06, 0x12, 0x41, 0x51, 0x07, 0x61, 0x71, 0x13,
            0x22, 0x32, 0x81, 0x08, 0x14, 0x42, 0x91, 0xa1, 0xb1, 0xc1, 0x09, 0x23, 0x33, 0x52,
            0xf0, 0x15, 0x62, 0x72, 0xd1, 0x0a, 0x16, 0x24, 0x34, 0xe1, 0x25, 0xf1, 0x17, 0x18,
            0x19, 0x1a, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43,
            0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59,
            0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x73, 0x74, 0x75, 0x76, 0x77,
            0x78, 0x79, 0x7a, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x92, 0x93,
            0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8,
            0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3, 0xc4,
            0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9,
            0xda, 0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf2, 0xf3, 0xf4, 0xf5,
            0xf6, 0xf7, 0xf8, 0xf9, 0xfa, 0xff, 0xdb, 0x00, 0x43, 0x00, 0x02, 0x02, 0x02, 0x02,
            0x02, 0x02, 0x03, 0x02, 0x02, 0x03, 0x05, 0x03, 0x03, 0x03, 0x05, 0x06, 0x05, 0x05,
            0x05, 0x05, 0x06, 0x08, 0x06, 0x06, 0x06, 0x06, 0x06, 0x08, 0x0a, 0x08, 0x08, 0x08,
            0x08, 0x08, 0x08, 0x0a, 0x0a, 0x0a, 0x0a, 0x0a, 0x0a, 0x0a, 0x0a, 0x0c, 0x0c, 0x0c,
            0x0c, 0x0c, 0x0c, 0x0e, 0x0e, 0x0e, 0x0e, 0x0e, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f, 0x0f,
            0x0f, 0x0f, 0x0f, 0x0f, 0xff, 0xdb, 0x00, 0x43, 0x01, 0x02, 0x02, 0x02, 0x04, 0x04,
            0x04, 0x07, 0x04, 0x04, 0x07, 0x10, 0x0b, 0x09, 0x0b, 0x10, 0x10, 0x10, 0x10, 0x10,
            0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10,
            0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10,
            0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10,
            0x10, 0x10, 0x10, 0xff, 0xdd, 0x00, 0x04, 0x00, 0x01, 0xff, 0xda, 0x00, 0x0c, 0x03,
            0x01, 0x00, 0x02, 0x11, 0x03, 0x11, 0x00, 0x3f, 0x00, 0xf8, 0xbe, 0x8a, 0x28, 0xaf,
            0xe5, 0x33, 0xfd, 0xfc, 0x3f, 0xff, 0xd9,
        ];
        let red_decoded = decode_jpeg_to_surface565(red_16x16_jpeg).expect("decode 16×16 red JPEG");
        assert_eq!((red_decoded.width, red_decoded.height), (16, 16));
        assert!(
            red_decoded.pixels.iter().all(|&p| {
                let [r, g, b] = widen_565(p);
                r >= 240 && g <= 8 && b <= 8
            }),
            "expected solid red pixels"
        );
    }
}
