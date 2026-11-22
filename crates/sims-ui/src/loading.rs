//! Complete Collection 1.91 loading screen (`cWinLoading`).
//!
//! Renders the native startup splash screen and neighborhood / lot / expansion
//! travel transition screens using artwork from `UIGraphics` (`Res_Other.RT`,
//! `Res_Nbhd.RT`) and localized rotating captions from `GameData/UIText.iff`
//! (`STR# 155` `splashprogress`, `STR# 271` Superstar loading captions).
//!
//! Like the rest of `sims-ui`, this module is pure CPU rendering and view-state
//! management: callers drive [`LoadingScreen::tick`] and [`LoadingScreen::draw`]
//! and forward returned [`LoadingAudioCue`] values to `sims-audio`
//! (`MusicMode::Load` / `Music/Modes/Load/loadloop.wav`).

use crate::art::Art;
use crate::font::Font;
use crate::frame::{Frame, Rect};
use crate::{Error, Result};
use sims_content::Install;
use sims_content::ui_graphics::UiGraphics;
use sims_formats::iff::Iff;
use sims_formats::strings::StringTable;

/// `GameData/UIText.iff` string table for startup / loading rotating captions
/// (`splashprogress`, 17 entries in Complete Collection).
pub const SPLASH_PROGRESS_STRINGS: u16 = 155;
/// `GameData/UIText.iff` string table for Superstar loading screen captions.
pub const CELEBRITY_LOADING_STRINGS: u16 = 271;

/// `kSetMusicMode` numeric ID for `MusicMode::Load` (`0x575170`).
pub const LOAD_MUSIC_MODE_ID: i64 = 5;
/// Relative install path for the native loading screen audio loop.
pub const LOAD_LOOP_ASSET: &str = "Music\\Modes\\Load\\loadloop.wav";

/// Native 800×600 splash dimensions.
pub const NATIVE_WIDTH: i32 = 800;
pub const NATIVE_HEIGHT: i32 = 600;
/// High-resolution 1024×768 splash dimensions used by Complete Collection
/// travel transition bitmaps when the viewport is at least 1024×768.
pub const LARGE_WIDTH: i32 = 1024;
pub const LARGE_HEIGHT: i32 = 768;

/// Default interval between rotating loading captions (milliseconds).
pub const DEFAULT_CAPTION_INTERVAL_MS: u64 = 2_500;

/// `UIGraphics` resource IDs for startup and travel loading screens
/// (`Res_Other.h`, `Res_Nbhd.h`).
pub mod art_id {
    /// `Other/MaxisLogo.bmp` (`kMaxisLogo`).
    pub const MAXIS_LOGO: i32 = 9002;
    /// `Other/setup.bmp` (localized via `Res_Other.RT`, `kSimsLogo`).
    pub const STARTUP_SPLASH: i32 = 9003;
    /// `Downtown/LargeBack.bmp` (`kLargeMask`, 1024×768 frame mask).
    pub const LARGE_MASK: i32 = 5001;
    /// `Studiotown/dlgframe_1024x768.bmp` (`kLargeMaskSS`).
    pub const LARGE_MASK_SS: i32 = 5002;
    /// `Downtown/Taxi_loadscreen_800x600.bmp` (`kDTGo`).
    pub const DOWNTOWN_800: i32 = 5424;
    /// `Downtown/Taxi_loadscreen_1024x768.bmp` (`kDTGoBig`).
    pub const DOWNTOWN_1024: i32 = 5425;
    /// `VIsland/vacation_loadscreen_800x600.bmp` (`kVacationGo`).
    pub const VACATION_800: i32 = 5500;
    /// `VIsland/vacation_loadscreen_1024x768.bmp` (`kVacationGoBig`).
    pub const VACATION_1024: i32 = 5501;
    /// `VIsland/vacation_loadscreen2_800x600.bmp` (`kVacationGo2`).
    pub const VACATION_ALT_800: i32 = 5502;
    /// `VIsland/vacation_loadscreen2_1024x768.bmp` (`kVacationGo2Big`).
    pub const VACATION_ALT_1024: i32 = 5503;
    /// `Community/Bus_loadscreen_800x600.bmp` (`kNeighborhoodGo`).
    pub const NEIGHBORHOOD_BUS_800: i32 = 5600;
    /// `Community/Bus_loadscreen_1024x768.bmp` (`kNeighborhoodGoBig`).
    pub const NEIGHBORHOOD_BUS_1024: i32 = 5601;
    /// `Studiotown/Studiotown_loadscreen_800x600.bmp` (`kStudiotownGo`).
    pub const STUDIO_TOWN_800: i32 = 5650;
    /// `Studiotown/Studiotown_loadscreen_1024x768.bmp` (`kStudiotownGoBig`).
    pub const STUDIO_TOWN_1024: i32 = 5651;
    /// `Studiotown/Studiotown_loadscreen_fan_800x600.bmp` (`kStudiotownFanGo`).
    pub const STUDIO_TOWN_FAN_800: i32 = 5652;
    /// `Studiotown/Studiotown_loadscreen_fan_1024x768.bmp` (`kStudiotownFanGoBig`).
    pub const STUDIO_TOWN_FAN_1024: i32 = 5653;
    /// `Magicland/magicland_loadscreen_800x600.bmp` (`kMagicLandGo`).
    pub const MAGIC_TOWN_800: i32 = 5654;
    /// `Magicland/magicland_loadscreen_1024x768.bmp` (`kMagicLandGoBig`).
    pub const MAGIC_TOWN_1024: i32 = 5655;
    /// `Magicland/magicland_loadscreen_hole_800x600.bmp` (`kMagicLandHoleGo`).
    pub const MAGIC_TOWN_HOLE_800: i32 = 5656;
    /// `Magicland/magicland_loadscreen_hole_1024x768.bmp` (`kMagicLandHoleGoBig`).
    pub const MAGIC_TOWN_HOLE_1024: i32 = 5657;
}

/// Every `UIGraphics` resource ID used by the loading screens.
pub const RESOURCES: [i32; 19] = [
    art_id::STARTUP_SPLASH,
    art_id::LARGE_MASK,
    art_id::LARGE_MASK_SS,
    art_id::DOWNTOWN_800,
    art_id::DOWNTOWN_1024,
    art_id::VACATION_800,
    art_id::VACATION_1024,
    art_id::VACATION_ALT_800,
    art_id::VACATION_ALT_1024,
    art_id::NEIGHBORHOOD_BUS_800,
    art_id::NEIGHBORHOOD_BUS_1024,
    art_id::STUDIO_TOWN_800,
    art_id::STUDIO_TOWN_1024,
    art_id::STUDIO_TOWN_FAN_800,
    art_id::STUDIO_TOWN_FAN_1024,
    art_id::MAGIC_TOWN_800,
    art_id::MAGIC_TOWN_1024,
    art_id::MAGIC_TOWN_HOLE_800,
    art_id::MAGIC_TOWN_HOLE_1024,
];

/// Loads all loading-screen bitmaps and UI fonts from `ui`.
pub fn load_art(ui: &UiGraphics, language: u8) -> Result<Art> {
    Art::load(ui, language, &RESOURCES)
}

/// Backdrop fill outside the centered splash canvas.
const BACKDROP_BG: [u8; 3] = [10, 16, 44];
/// Fallback panel fill when splash artwork is not loaded.
const FALLBACK_PANEL_BG: [u8; 3] = [18, 30, 88];
/// Caption bar translucent dark navy backing.
const CAPTION_BG: [u8; 3] = [12, 20, 56];
/// Caption bar outline.
const CAPTION_BORDER: [u8; 3] = [74, 118, 210];
/// Progress track background and border.
const TRACK_BG: [u8; 3] = [14, 24, 62];
const TRACK_BORDER: [u8; 3] = [96, 148, 232];
/// Progress bar fill (classic Sims bright blue-cyan gradient tones).
const FILL_BASE: [u8; 3] = [44, 146, 236];
const FILL_HIGHLIGHT: [u8; 3] = [128, 216, 255];
/// Caption and header ink.
const TEXT_PRIMARY: [u8; 3] = [235, 244, 255];
const TEXT_ACCENT: [u8; 3] = [255, 232, 140];

/// Which transition / splash screen is active.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum LoadingKind {
    /// Initial game startup splash (`Other/setup.bmp`).
    #[default]
    Startup,
    /// Transitioning to the main neighborhood or Unleashed community lot.
    Neighborhood,
    /// Loading into a residential or community lot.
    Lot,
    /// Traveling to Hot Date Downtown by taxi.
    Downtown,
    /// Traveling to Vacation Island (`alternate` selects `vacation_loadscreen2`).
    Vacation { alternate: bool },
    /// Traveling to Superstar Studio Town (`fan_bus` selects the fan-bus art).
    StudioTown { fan_bus: bool },
    /// Traveling to Makin' Magic Magic Town (`via_hole` selects the magic hole art).
    MagicTown { via_hole: bool },
}

impl LoadingKind {
    /// Returns `(standard_800_id, large_1024_id)` in `UIGraphics`.
    pub const fn artwork_ids(self) -> (i32, Option<i32>) {
        match self {
            Self::Startup => (art_id::STARTUP_SPLASH, None),
            Self::Neighborhood | Self::Lot => (
                art_id::NEIGHBORHOOD_BUS_800,
                Some(art_id::NEIGHBORHOOD_BUS_1024),
            ),
            Self::Downtown => (art_id::DOWNTOWN_800, Some(art_id::DOWNTOWN_1024)),
            Self::Vacation { alternate: false } => {
                (art_id::VACATION_800, Some(art_id::VACATION_1024))
            }
            Self::Vacation { alternate: true } => {
                (art_id::VACATION_ALT_800, Some(art_id::VACATION_ALT_1024))
            }
            Self::StudioTown { fan_bus: false } => {
                (art_id::STUDIO_TOWN_800, Some(art_id::STUDIO_TOWN_1024))
            }
            Self::StudioTown { fan_bus: true } => (
                art_id::STUDIO_TOWN_FAN_800,
                Some(art_id::STUDIO_TOWN_FAN_1024),
            ),
            Self::MagicTown { via_hole: false } => {
                (art_id::MAGIC_TOWN_800, Some(art_id::MAGIC_TOWN_1024))
            }
            Self::MagicTown { via_hole: true } => (
                art_id::MAGIC_TOWN_HOLE_800,
                Some(art_id::MAGIC_TOWN_HOLE_1024),
            ),
        }
    }

    /// Selects the best available artwork ID for a viewport of `(width, height)`.
    pub fn select_art_id(self, width: u32, height: u32, art: &Art) -> Option<i32> {
        let (std_id, big_id) = self.artwork_ids();
        if width >= LARGE_WIDTH as u32
            && height >= LARGE_HEIGHT as u32
            && let Some(big) = big_id
            && art.image(big).is_some()
        {
            return Some(big);
        }
        if art.image(std_id).is_some() {
            return Some(std_id);
        }
        if let Some(big) = big_id
            && art.image(big).is_some()
        {
            return Some(big);
        }
        art.image(art_id::STARTUP_SPLASH)
            .map(|_| art_id::STARTUP_SPLASH)
    }
}

/// Audio lifecycle cue emitted by [`LoadingScreen`] so hosts can drive
/// `sims_audio::music::ModeMusicPlayer` (`MusicMode::Load` / `loadloop.wav`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadingAudioCue {
    /// Enter loading audio (`MusicMode::Load`, `Music\Modes\Load\loadloop.wav`).
    StartLoadLoop {
        mode_id: i64,
        asset_path: &'static str,
    },
    /// Leave loading audio (caller transitions to destination music mode).
    StopLoadLoop,
}

/// Localized loading captions read from `GameData/UIText.iff` (`STR# 155` and
/// `STR# 271`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoadingStrings {
    /// `STR# 155`: rotating progress captions (`splashprogress`).
    pub progress_captions: Vec<String>,
    /// `STR# 271`: Superstar celebrity loading captions.
    pub celebrity_captions: Vec<String>,
}

impl LoadingStrings {
    /// Loads `STR# 155` and `STR# 271` from the install's `GameData/UIText.iff`
    /// in `language` (1 = US English).
    pub fn load(install: &Install, language: u8) -> Result<Self> {
        let path = install
            .file("GameData/UIText.iff")
            .map_err(Error::Content)?;
        let bytes = std::fs::read(&path).map_err(Error::Io)?;
        Self::from_iff_bytes(bytes, language)
    }

    /// Parses `STR# 155` and `STR# 271` from raw `UIText.iff` bytes.
    pub fn from_iff_bytes(bytes: Vec<u8>, language: u8) -> Result<Self> {
        let iff = Iff::parse(bytes).map_err(Error::Format)?;
        let read_table = |id: u16| -> Result<Vec<String>> {
            let Some(chunk) = iff.get(b"STR#", id) else {
                return Ok(Vec::new());
            };
            let table = StringTable::parse(iff.data(chunk)).map_err(Error::Format)?;
            Ok((0..table.entries.len())
                .filter_map(|i| {
                    table
                        .get(i, language)
                        .or_else(|| table.get(i, 1))
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                })
                .collect())
        };
        Ok(Self {
            progress_captions: read_table(SPLASH_PROGRESS_STRINGS)?,
            celebrity_captions: read_table(CELEBRITY_LOADING_STRINGS)?,
        })
    }

    /// Returns the active caption pool for `kind`.
    pub fn captions_for(&self, kind: LoadingKind) -> &[String] {
        if matches!(kind, LoadingKind::StudioTown { .. }) && !self.celebrity_captions.is_empty() {
            &self.celebrity_captions
        } else {
            &self.progress_captions
        }
    }

    /// Returns the rotating caption at `index` (wrapping modulo pool length).
    pub fn caption_at(&self, kind: LoadingKind, index: usize) -> Option<&str> {
        let pool = self.captions_for(kind);
        if pool.is_empty() {
            return None;
        }
        Some(pool[index % pool.len()].as_str())
    }
}

/// Read-only view state rendered by [`LoadingScreen`].
#[derive(Debug, Clone, PartialEq)]
pub struct LoadingState {
    pub kind: LoadingKind,
    /// Normalized completion ratio (`0.0..=1.0`). Non-finite values clamp to `0.0`.
    pub progress: f32,
    /// Optional destination / lot name header (e.g. a lot or sub-neighborhood title).
    pub destination_label: Option<String>,
    /// Optional explicit status caption overriding the rotating `STR# 155` pool.
    pub caption_override: Option<String>,
}

impl Default for LoadingState {
    fn default() -> Self {
        Self {
            kind: LoadingKind::Startup,
            progress: 0.0,
            destination_label: None,
            caption_override: None,
        }
    }
}

impl LoadingState {
    /// Clamped progress in `0.0..=1.0`, mapping NaN/negative to `0.0`.
    pub fn clamped_progress(&self) -> f32 {
        if !self.progress.is_finite() {
            return 0.0;
        }
        self.progress.clamp(0.0, 1.0)
    }

    /// Integer completion percentage `0..=100`.
    pub fn percent(&self) -> u8 {
        (self.clamped_progress() * 100.0).round().clamp(0.0, 100.0) as u8
    }
}

/// Computed layout for the loading screen at a given viewport size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadingLayout {
    pub frame_size: [i32; 2],
    /// Centered (or full-viewport on compact frames) splash canvas rectangle.
    pub canvas_rect: Rect,
    /// Header band for destination label (if any).
    pub header_rect: Rect,
    /// Animated progress bar outer frame.
    pub progress_rect: Rect,
    /// Localized rotating caption band below the progress bar.
    pub caption_rect: Rect,
    /// True when the viewport is narrower than 800 px or shorter than 600 px.
    pub compact: bool,
}

impl LoadingLayout {
    /// Computes the loading screen layout for a viewport of `width` × `height`.
    ///
    /// At `width >= 800 && height >= 600`, centers the native `800×600` (or
    /// `1024×768` when `>= 1024×768` and large art is loaded) canvas cleanly.
    /// Below `800×600`, switches to a compact layout that keeps every element
    /// strictly inside the viewport with non-overlapping bands.
    pub fn compute(width: u32, height: u32, kind: LoadingKind, art: &Art) -> Self {
        let w = i32::try_from(width).unwrap_or(i32::MAX).max(0);
        let h = i32::try_from(height).unwrap_or(i32::MAX).max(0);
        let compact = w < NATIVE_WIDTH || h < NATIVE_HEIGHT;

        let canvas_rect = if compact {
            Rect::new(0, 0, w, h)
        } else {
            let (cw, ch) = match kind.select_art_id(width, height, art) {
                Some(id) => match art.image(id) {
                    Some(img) => ((img.width as i32).min(w), (img.height as i32).min(h)),
                    None => (NATIVE_WIDTH, NATIVE_HEIGHT),
                },
                None => (NATIVE_WIDTH, NATIVE_HEIGHT),
            };
            Rect::new((w - cw) / 2, (h - ch) / 2, cw, ch)
        };

        let pad_x = if compact { (w / 20).clamp(6, 20) } else { 48 };
        let inner_w = (canvas_rect.width - pad_x * 2).max(0);
        let inner_x = canvas_rect.x + (canvas_rect.width - inner_w) / 2;

        let bar_h = if compact { 14.min(h / 10).max(6) } else { 18 };
        let caption_h = if compact {
            28.min((h - bar_h).max(0) / 3).max(16.min(h))
        } else {
            34
        };
        let header_h = if compact {
            20.min((h - bar_h - caption_h).max(0) / 3)
        } else {
            26
        };
        let gap = if compact { 4 } else { 8 };
        let bottom_margin = if compact { 8.min(h / 12) } else { 28 };

        let caption_y =
            (canvas_rect.y + canvas_rect.height - bottom_margin - caption_h).max(canvas_rect.y);
        let progress_y = (caption_y - gap - bar_h).max(canvas_rect.y);
        let header_y = (progress_y - gap - header_h).max(canvas_rect.y);

        // Ensure strict non-overlap even on tiny viewports (e.g. 160x120).
        let final_caption_h = (canvas_rect.y + canvas_rect.height - caption_y)
            .min(caption_h)
            .max(0);
        let final_bar_h = (caption_y - progress_y).min(bar_h).max(0);
        let final_header_h = (progress_y - header_y).min(header_h).max(0);

        Self {
            frame_size: [w, h],
            canvas_rect,
            header_rect: Rect::new(inner_x, header_y, inner_w, final_header_h),
            progress_rect: Rect::new(inner_x, progress_y, inner_w, final_bar_h),
            caption_rect: Rect::new(inner_x, caption_y, inner_w, final_caption_h),
            compact,
        }
    }
}

/// Stateful controller for the loading screen's caption rotation, progress bar
/// animation, and audio loop lifecycle.
#[derive(Debug, Clone)]
pub struct LoadingScreen {
    elapsed_ms: u64,
    caption_interval_ms: u64,
    caption_index: usize,
    audio_active: bool,
}

impl Default for LoadingScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl LoadingScreen {
    /// Creates an idle loading screen controller with the default 2.5 s caption
    /// rotation interval.
    pub fn new() -> Self {
        Self {
            elapsed_ms: 0,
            caption_interval_ms: DEFAULT_CAPTION_INTERVAL_MS,
            caption_index: 0,
            audio_active: false,
        }
    }

    /// Sets the caption rotation interval in milliseconds (clamped to `>= 250`).
    pub fn set_caption_interval_ms(&mut self, interval_ms: u64) {
        self.caption_interval_ms = interval_ms.max(250);
    }

    /// Current caption rotation index.
    pub fn caption_index(&self) -> usize {
        self.caption_index
    }

    /// Total elapsed milliseconds since [`Self::begin`].
    pub fn elapsed_ms(&self) -> u64 {
        self.elapsed_ms
    }

    /// Whether the loading audio loop is currently active.
    pub fn is_audio_active(&self) -> bool {
        self.audio_active
    }

    /// Begins a loading transition, resetting timers and emitting
    /// [`LoadingAudioCue::StartLoadLoop`] if the load loop is not already active.
    pub fn begin(&mut self, start_caption_index: usize) -> Option<LoadingAudioCue> {
        self.elapsed_ms = 0;
        self.caption_index = start_caption_index;
        if !self.audio_active {
            self.audio_active = true;
            Some(LoadingAudioCue::StartLoadLoop {
                mode_id: LOAD_MUSIC_MODE_ID,
                asset_path: LOAD_LOOP_ASSET,
            })
        } else {
            None
        }
    }

    /// Finishes the loading transition, emitting [`LoadingAudioCue::StopLoadLoop`]
    /// if the load loop was active.
    pub fn finish(&mut self) -> Option<LoadingAudioCue> {
        if self.audio_active {
            self.audio_active = false;
            Some(LoadingAudioCue::StopLoadLoop)
        } else {
            None
        }
    }

    /// Advances the loading animation and caption rotation by `delta_ms`.
    pub fn tick(&mut self, delta_ms: u64) {
        let prev_step = self.elapsed_ms / self.caption_interval_ms;
        self.elapsed_ms = self.elapsed_ms.saturating_add(delta_ms);
        let next_step = self.elapsed_ms / self.caption_interval_ms;
        let advance = next_step.saturating_sub(prev_step) as usize;
        self.caption_index = self.caption_index.wrapping_add(advance);
    }

    /// Resolves the caption string currently displayed for `state`.
    pub fn active_caption<'a>(
        &self,
        state: &'a LoadingState,
        strings: &'a LoadingStrings,
    ) -> std::borrow::Cow<'a, str> {
        if let Some(ref custom) = state.caption_override
            && !custom.is_empty()
        {
            return std::borrow::Cow::Borrowed(custom.as_str());
        }
        if let Some(localized) = strings.caption_at(state.kind, self.caption_index) {
            return std::borrow::Cow::Borrowed(localized);
        }
        std::borrow::Cow::Owned(format!("{}%", state.percent()))
    }

    /// Draws the loading screen into `frame` and returns the computed layout.
    pub fn draw(
        &self,
        frame: &mut Frame,
        state: &LoadingState,
        art: &Art,
        strings: &LoadingStrings,
    ) -> LoadingLayout {
        let layout = LoadingLayout::compute(frame.width(), frame.height(), state.kind, art);
        let fw = frame.width() as i32;
        let fh = frame.height() as i32;

        // Fill outer letterbox / pillarbox backdrop.
        frame.fill(Rect::new(0, 0, fw, fh), BACKDROP_BG);

        // Draw splash artwork (centered on >= 800x600, scaled-to-fit/crop on compact).
        if let Some(art_id) = state.kind.select_art_id(frame.width(), frame.height(), art)
            && let Some(img) = art.image(art_id)
        {
            if layout.compact {
                draw_scaled_nearest(frame, img, layout.canvas_rect);
            } else {
                let ox = layout.canvas_rect.x + (layout.canvas_rect.width - img.width as i32) / 2;
                let oy = layout.canvas_rect.y + (layout.canvas_rect.height - img.height as i32) / 2;
                frame.draw_all(img, [ox, oy]);
            }
        } else {
            frame.fill(layout.canvas_rect, FALLBACK_PANEL_BG);
            draw_rect_border(frame, layout.canvas_rect, CAPTION_BORDER);
        }

        // Destination header (if supplied and room permits).
        if let Some(ref label) = state.destination_label
            && !label.is_empty()
            && layout.header_rect.height >= 12
            && layout.header_rect.width >= 20
        {
            fill_alpha(frame, layout.header_rect, CAPTION_BG, 200);
            if let Some(font) = pick_font(art, if layout.compact { 10 } else { 12 }) {
                draw_centered_clipped(frame, font, layout.header_rect, label, TEXT_ACCENT);
            }
        }

        // Animated progress bar.
        if layout.progress_rect.width >= 4 && layout.progress_rect.height >= 4 {
            fill_alpha(frame, layout.progress_rect, TRACK_BG, 230);
            draw_rect_border(frame, layout.progress_rect, TRACK_BORDER);

            let inner = Rect::new(
                layout.progress_rect.x + 2,
                layout.progress_rect.y + 2,
                (layout.progress_rect.width - 4).max(0),
                (layout.progress_rect.height - 4).max(0),
            );
            let fill_w = ((inner.width as f32) * state.clamped_progress()).round() as i32;
            let fill_w = fill_w.clamp(0, inner.width);
            if fill_w > 0 && inner.height > 0 {
                let fill_rect = Rect::new(inner.x, inner.y, fill_w, inner.height);
                frame.fill(fill_rect, FILL_BASE);

                // Top specular highlight line.
                let highlight_h = (inner.height / 3).max(1);
                fill_alpha(
                    frame,
                    Rect::new(inner.x, inner.y, fill_w, highlight_h),
                    FILL_HIGHLIGHT,
                    140,
                );

                // Animated shimmer sweep across the filled portion.
                let shimmer_w = (inner.width / 8).clamp(8, 48).min(fill_w);
                if shimmer_w > 0 && fill_w > 4 {
                    let span = (fill_w + shimmer_w).max(1) as u64;
                    let phase = ((self.elapsed_ms / 16) % span) as i32;
                    let sx = (inner.x + phase - shimmer_w).max(inner.x);
                    let ex = (inner.x + phase).min(inner.x + fill_w);
                    if ex > sx {
                        fill_alpha(
                            frame,
                            Rect::new(sx, inner.y, ex - sx, inner.height),
                            FILL_HIGHLIGHT,
                            110,
                        );
                    }
                }
            }
        }

        // Rotating localized caption bar.
        if layout.caption_rect.width >= 16 && layout.caption_rect.height >= 12 {
            fill_alpha(frame, layout.caption_rect, CAPTION_BG, 215);
            draw_rect_border(frame, layout.caption_rect, CAPTION_BORDER);
            let caption = self.active_caption(state, strings);
            if let Some(font) = pick_font(art, if layout.compact { 9 } else { 11 }) {
                draw_centered_clipped(frame, font, layout.caption_rect, &caption, TEXT_PRIMARY);
            }
        }

        layout
    }
}

fn fill_alpha(frame: &mut Frame, rect: Rect, color: [u8; 3], alpha: u8) {
    let fw = frame.width() as i32;
    let fh = frame.height() as i32;
    let x0 = rect.x.max(0);
    let y0 = rect.y.max(0);
    let x1 = (rect.x + rect.width).min(fw);
    let y1 = (rect.y + rect.height).min(fh);
    for y in y0..y1 {
        for x in x0..x1 {
            frame.blend(x, y, color, alpha);
        }
    }
}

fn pick_font(art: &Art, preferred: u8) -> Option<&Font> {
    art.font(preferred)
        .or_else(|| art.font(10))
        .or_else(|| art.font(9))
        .or_else(|| art.font(8))
}

fn draw_rect_border(frame: &mut Frame, rect: Rect, color: [u8; 3]) {
    if rect.width <= 0 || rect.height <= 0 {
        return;
    }
    frame.fill(Rect::new(rect.x, rect.y, rect.width, 1), color);
    frame.fill(
        Rect::new(rect.x, rect.y + rect.height - 1, rect.width, 1),
        color,
    );
    frame.fill(Rect::new(rect.x, rect.y, 1, rect.height), color);
    frame.fill(
        Rect::new(rect.x + rect.width - 1, rect.y, 1, rect.height),
        color,
    );
}

fn draw_scaled_nearest(frame: &mut Frame, img: &sims_formats::bitmap::Image, dst: Rect) {
    if img.width == 0 || img.height == 0 || dst.width <= 0 || dst.height <= 0 {
        return;
    }
    let fw = frame.width() as i32;
    let fh = frame.height() as i32;
    let x0 = dst.x.max(0);
    let y0 = dst.y.max(0);
    let x1 = (dst.x + dst.width).min(fw);
    let y1 = (dst.y + dst.height).min(fh);
    for y in y0..y1 {
        let sy = (((y - dst.y) as u64 * img.height as u64) / dst.height as u64)
            .min(img.height as u64 - 1) as usize;
        for x in x0..x1 {
            let sx = (((x - dst.x) as u64 * img.width as u64) / dst.width as u64)
                .min(img.width as u64 - 1) as usize;
            let idx = (sy * img.width as usize + sx) * 4;
            let a = img.rgba[idx + 3];
            if a > 0 {
                frame.blend(
                    x,
                    y,
                    [img.rgba[idx], img.rgba[idx + 1], img.rgba[idx + 2]],
                    a,
                );
            }
        }
    }
}

fn draw_centered_clipped(frame: &mut Frame, font: &Font, rect: Rect, text: &str, color: [u8; 3]) {
    let pad = 6;
    let max_w = (rect.width - pad * 2).max(0);
    if max_w == 0 {
        return;
    }
    let fitted = truncate_to_width(font, text, max_w);
    let text_w = font.width(&fitted);
    let tx = rect.x + (rect.width - text_w) / 2;
    let ty = rect.y + (rect.height - font.line_height) / 2;
    font.draw(frame, &fitted, [tx, ty], color);
}

fn truncate_to_width(font: &Font, text: &str, max_w: i32) -> String {
    if font.width(text) <= max_w {
        return text.to_owned();
    }
    let ellipsis = "...";
    let ellipsis_w = font.width(ellipsis);
    if ellipsis_w >= max_w {
        return String::new();
    }
    let mut out = String::new();
    for ch in text.chars() {
        out.push(ch);
        if font.width(&out) + ellipsis_w > max_w {
            out.pop();
            break;
        }
    }
    out.push_str(ellipsis);
    out
}
