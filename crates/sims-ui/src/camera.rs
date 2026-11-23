//! Native Complete Collection 1.91 Camera Mode subpanel (`cWinCamera`, vtable
//! `0x66a334`, `0x50333e..0x5043e0`).
//!
//! In 1.91, pressing the Camera button (`hud::id::PHOTO`, resource `2034`) in
//! the control-panel cluster opens this 100 px tall subpanel inside the control
//! panel strip (`strip = [220, height - 100, width - 220, 100]` at 800×600,
//! `0x509f94..0x50a030`), draws `kCameraPatch` (`2035`) over the cluster at
//! `(165, 83)` (`0x53a1a9..0x53a1c3`), and exposes:
//!
//! - Snapshot frame size toggles: Small (`1101`), Medium (`1102`), Large
//!   (`1103`), and Custom (`1104`, visible only when a custom frame is active).
//! - Snapshot quality toggles: Low (`1106`), Medium (`1107`), High (`1108`).
//! - The Photo Album launcher button: `kCamViewAlbum` (`1105`, at `(226, 35)`).
//! - Vertical dividers `kCamToothPick1` (`4914` at `(133, 6)`) and
//!   `kCamToothPick2` (`4915` at `(206, 6)`).
//! - Context help balloons (`kCamPopupDetail` `1109` for Snapshot Sizes and
//!   `kCamPopupQualDetail` `1110` for Quality Settings).
//!
//! All button coordinates, tooltips, and help-balloon texts are loaded from
//! `GameData/UIText.iff` `STR# 140` (`cammodestrs`, `0x5034d7`), with zero
//! verbatim game prose embedded in source.

use crate::art::Art;
use crate::balloon::{self, Picture};
use crate::frame::{Frame, Rect};
use crate::hud::{self, LABEL};
use crate::neighborhood::draw_tooltip;
use crate::{Error, Result};
use sims_content::Install;
use sims_content::ui_graphics::UiGraphics;
use sims_formats::iff::Iff;
use sims_formats::strings::StringTable;

/// `GameData/UIText.iff` string table for Camera Mode coordinates, tooltips,
/// and help balloons (`cammodestrs`, `0x5034d7`: `0x8c`).
pub const CAM_MODE_STRINGS: u16 = 140;

/// Native cluster overlay offset for `kCameraPatch` (`2035`) relative to the
/// control-panel cluster top-left (`0x53a1a9..0x53a1c3`: `(0xa5, 0x53)`).
pub const PATCH_ORIGIN: [i32; 2] = [165, 83];

/// Vertical divider positions inside the Camera subpanel strip
/// (`cWinCamera::OnDraw`, `0x503c67..0x503cc4`: `(0x85, 6)` and `(0xce, 6)`).
pub const TOOTHPICK_LEFT_POS: [i32; 2] = [133, 6];
pub const TOOTHPICK_RIGHT_POS: [i32; 2] = [206, 6];

/// `UIGraphics/Res_CPanel.h` resource identifiers used by [`CameraPanel`].
pub mod id {
    /// Control-panel strip background (`PanelBack.BMP`, `kUniversalCPBkg`).
    pub const STRIP: i32 = crate::hud::id::STRIP;
    /// Cluster trim patch when Camera Mode is active (`CameraPatch.bmp`, 55×100, `0x539c57`).
    pub const PATCH: i32 = 2035;
    /// Small snapshot size toggle (`CamFrameSmall.bmp`, 128×27, 4 cells of 32×27).
    pub const FRAME_SMALL: i32 = 1101;
    /// Medium snapshot size toggle (`CamFrameMed.bmp`, 128×29, 4 cells of 32×29).
    pub const FRAME_MED: i32 = 1102;
    /// Large snapshot size toggle (`CamFrameLarge.bmp`, 128×28, 4 cells of 32×28).
    pub const FRAME_LARGE: i32 = 1103;
    /// Custom snapshot size badge (`CamFrameCust.bmp`, 240×58, 4 cells of 60×58).
    pub const FRAME_CUSTOM: i32 = 1104;
    /// Open Photo Album button (`CamViewAlbum.bmp`, 244×35, 4 cells of 61×35).
    pub const VIEW_ALBUM: i32 = 1105;
    /// Low JPEG quality toggle (`CamQualLow.bmp`, 128×28, 4 cells of 32×28).
    pub const QUAL_LOW: i32 = 1106;
    /// Medium JPEG quality toggle (`CamQualMed.bmp`, 128×29, 4 cells of 32×29).
    pub const QUAL_MED: i32 = 1107;
    /// High JPEG quality toggle (`CamQualHigh.bmp`, 128×28, 4 cells of 32×28).
    pub const QUAL_HIGH: i32 = 1108;
    /// Snapshot Sizes help-balloon illustration (`CamPopupDetail.bmp`, 100×100, `0x503469`).
    pub const POPUP_SIZE: i32 = 1109;
    /// Quality Settings help-balloon illustration (`CamPopupQualDetail.bmp`, 100×100, `0x50347b`).
    pub const POPUP_QUALITY: i32 = 1110;
    /// Left vertical divider (`CameraToothpkLeft.bmp`, 2×89, `0x50348d`).
    pub const TOOTHPICK_LEFT: i32 = 4914;
    /// Right vertical divider (`CameraToothpkRight.bmp`, 2×89, `0x50349f`).
    pub const TOOTHPICK_RIGHT: i32 = 4915;
}

/// Core `UIGraphics` resources required to draw [`CameraPanel`] and its cluster patch.
pub const RESOURCES: [i32; 12] = [
    id::STRIP,
    id::PATCH,
    id::FRAME_SMALL,
    id::FRAME_MED,
    id::FRAME_LARGE,
    id::FRAME_CUSTOM,
    id::VIEW_ALBUM,
    id::QUAL_LOW,
    id::QUAL_MED,
    id::QUAL_HIGH,
    id::TOOTHPICK_LEFT,
    id::TOOTHPICK_RIGHT,
];

/// Optional help-balloon illustrations for Camera Mode (`kCamPopupDetail`, `kCamPopupQualDetail`).
pub const HELP_RESOURCES: [i32; 2] = [id::POPUP_SIZE, id::POPUP_QUALITY];

/// Loads all Camera Mode subpanel resources and fonts from `ui`.
pub fn load_art(ui: &UiGraphics, language: u8) -> Result<Art> {
    Art::load_with_optional(ui, language, &RESOURCES, &HELP_RESOURCES)
}

/// Snapshot crop frame size presets (`0x4cfb08`, `0x50413d`: `0=Small`,
/// `1=Medium`, `2=Large`, `3=Custom`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SnapshotSize {
    Small = 0,
    #[default]
    Medium = 1,
    Large = 2,
    Custom = 3,
}

impl SnapshotSize {
    /// Camera Mode frame dimensions `[width, height]` in pixels (`0x4cfb08`
    /// reads the table at `0x6f0ef0`, filled at `0x4cfac9..0x4cfaf3`:
    /// Small 200×150, Medium 400×300, Large 600×400). A Custom frame is
    /// clamped between Small and Large, as `0x4cfb5e` clamps it.
    pub fn dimensions(self, custom_dims: [u32; 2]) -> [u32; 2] {
        match self {
            Self::Small => [200, 150],
            Self::Medium => [400, 300],
            Self::Large => [600, 400],
            Self::Custom => [
                custom_dims[0].clamp(200, 600),
                custom_dims[1].clamp(150, 400),
            ],
        }
    }
}

/// The Camera Mode frame around the pointer `[x, y]` (`0x4b14a1`), as
/// `[left, top, right, bottom]` with exclusive right and bottom. The frame
/// is `size[0]` wide, centred on the pointer, and ends at the pointer's
/// row: `[x - w/2 - 1, y - h - 2, x + w/2 - 1, y]`. A frame that crosses
/// the `viewport` is first shifted back inside it (left before right, top
/// before bottom, `0x4b150b..0x4b154b`) and then intersected with it
/// (`0x4a76ed`).
pub fn snapshot_frame(pointer: [i32; 2], size: [u32; 2], viewport: [i32; 4]) -> [i32; 4] {
    let [x, y] = pointer;
    let w = i32::try_from(size[0]).unwrap_or(i32::MAX);
    let h = i32::try_from(size[1]).unwrap_or(i32::MAX);
    // `cdq; sub eax, edx; sar eax, 1`: halving rounds toward zero.
    let half = w / 2;
    let [vl, vt, vr, vb] = viewport;
    let mut r = [x - half - 1, y - h - 2, x + half - 1, y];
    let shift_x = if r[0] < vl {
        vl - r[0]
    } else if r[2] >= vr {
        vr - r[2] - 1
    } else {
        0
    };
    r[0] += shift_x;
    r[2] += shift_x;
    let shift_y = if r[1] < vt {
        vt - r[1]
    } else if r[3] >= vb {
        vb - r[3] - 1
    } else {
        0
    };
    r[1] += shift_y;
    r[3] += shift_y;
    [r[0].max(vl), r[1].max(vt), r[2].min(vr), r[3].min(vb)]
}

/// The pixels a Camera Mode click stores (`0x4d1f6e..0x4d1f98`): the
/// [`snapshot_frame`] less its one-pixel outline on every side, so the
/// photo is `w - 2` by `h` pixels when the frame fits the viewport.
pub fn snapshot_capture_rect(pointer: [i32; 2], size: [u32; 2], viewport: [i32; 4]) -> [i32; 4] {
    let f = snapshot_frame(pointer, size, viewport);
    [f[0] + 1, f[1] + 1, f[2] - 1, f[3] - 1]
}

/// Snapshot JPEG compression quality (`0x4cfc2a`, `0x50427b`: `0=Low`,
/// `1=Medium`, `2=High`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SnapshotQuality {
    Low = 0,
    Medium = 1,
    #[default]
    High = 2,
}

impl SnapshotQuality {
    /// The quality number native passes to the album image writer
    /// (`0x4cfc53`: Low 20, Medium 50, High 90; `0x4a9144` → `0x4ed79c`
    /// stores it at entry `+8`, and `0x4ed52c` passes it to the file writer
    /// `0x4b7fb4`). The port stores album photos losslessly, so this number
    /// does not yet change any pixel (a recorded residual).
    pub const fn jpeg_quality(self) -> u8 {
        match self {
            Self::Low => 20,
            Self::Medium => 50,
            Self::High => 90,
        }
    }
}

/// The eight buttons of `cWinCamera` in exact native table order (`0x66a310`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CameraButton {
    Small = 0,
    Medium = 1,
    Large = 2,
    Custom = 3,
    ViewAlbum = 4,
    QualityLow = 5,
    QualityMedium = 6,
    QualityHigh = 7,
}

impl CameraButton {
    /// All eight buttons in native `0x66a310` order.
    pub const ALL: [Self; 8] = [
        Self::Small,
        Self::Medium,
        Self::Large,
        Self::Custom,
        Self::ViewAlbum,
        Self::QualityLow,
        Self::QualityMedium,
        Self::QualityHigh,
    ];

    pub const fn index(self) -> usize {
        self as usize
    }

    /// `UIGraphics` 4-cell button strip resource ID (`0x66a310`).
    pub const fn art_id(self) -> i32 {
        match self {
            Self::Small => id::FRAME_SMALL,
            Self::Medium => id::FRAME_MED,
            Self::Large => id::FRAME_LARGE,
            Self::Custom => id::FRAME_CUSTOM,
            Self::ViewAlbum => id::VIEW_ALBUM,
            Self::QualityLow => id::QUAL_LOW,
            Self::QualityMedium => id::QUAL_MED,
            Self::QualityHigh => id::QUAL_HIGH,
        }
    }

    /// Default native 1-cell `[width, height]` from the shipped 4-cell strips.
    pub const fn native_size(self) -> [i32; 2] {
        match self {
            Self::Small => [32, 27],
            Self::Medium => [32, 29],
            Self::Large => [32, 28],
            Self::Custom => [60, 58],
            Self::ViewAlbum => [61, 35],
            Self::QualityLow => [32, 28],
            Self::QualityMedium => [32, 29],
            Self::QualityHigh => [32, 28],
        }
    }

    /// Native strip-relative `[x, y]` origin from US English `STR# 140`.
    pub const fn default_origin(self) -> [i32; 2] {
        match self {
            Self::Small => [82, 70],
            Self::Medium => [82, 36],
            Self::Large => [82, 4],
            Self::Custom => [11, 11],
            Self::ViewAlbum => [226, 35],
            Self::QualityLow => [154, 70],
            Self::QualityMedium => [154, 36],
            Self::QualityHigh => [154, 4],
        }
    }

    /// Whether this button is currently toggled/selected for `state`.
    pub const fn is_selected(self, state: &CameraState) -> bool {
        match self {
            Self::Small => matches!(state.size, SnapshotSize::Small),
            Self::Medium => matches!(state.size, SnapshotSize::Medium),
            Self::Large => matches!(state.size, SnapshotSize::Large),
            Self::Custom => matches!(state.size, SnapshotSize::Custom),
            Self::ViewAlbum => false,
            Self::QualityLow => matches!(state.quality, SnapshotQuality::Low),
            Self::QualityMedium => matches!(state.quality, SnapshotQuality::Medium),
            Self::QualityHigh => matches!(state.quality, SnapshotQuality::High),
        }
    }

    /// Which help balloon topic this button opens (`0x50408f..0x5040ff`), if any.
    pub const fn help_topic(self) -> Option<CameraHelp> {
        match self {
            Self::Small | Self::Medium | Self::Large | Self::Custom => {
                Some(CameraHelp::SnapshotSizes)
            }
            Self::QualityLow | Self::QualityMedium | Self::QualityHigh => {
                Some(CameraHelp::QualitySettings)
            }
            Self::ViewAlbum => None,
        }
    }
}

/// Which Camera Mode help balloon is open (`0x50408f..0x504116`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CameraHelp {
    SnapshotSizes,
    QualitySettings,
}

impl CameraHelp {
    /// Builds a [`balloon::Text`] with the localized title, body, and native
    /// `100×100` illustration (`1109` or `1110`) from `strings`.
    pub fn balloon_text(self, strings: &CameraStrings) -> balloon::Text {
        match self {
            Self::SnapshotSizes => balloon::Text {
                title: strings.size_help_title.clone(),
                body: strings.size_help_body.clone(),
                pictures: [Some(Picture::Image(id::POPUP_SIZE)), None],
            },
            Self::QualitySettings => balloon::Text {
                title: strings.quality_help_title.clone(),
                body: strings.quality_help_body.clone(),
                pictures: [Some(Picture::Image(id::POPUP_QUALITY)), None],
            },
        }
    }
}

/// Read-only Camera Mode settings rendered by [`CameraPanel`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CameraState {
    pub size: SnapshotSize,
    pub quality: SnapshotQuality,
    /// Active custom frame dimensions `[width, height]` shown under the
    /// Custom button when `size == SnapshotSize::Custom` (`0x5041a8..0x504277`).
    pub custom_dimensions: [u32; 2],
    /// Whether the Photo Album button (`1105`) is enabled (`true` on occupied
    /// residential lots / whenever an album exists).
    pub album_enabled: bool,
    /// Whether Quick Tips help balloons should open on Camera buttons (`0x503453`).
    pub quick_tips: bool,
}

impl Default for CameraState {
    fn default() -> Self {
        Self {
            size: SnapshotSize::Medium,
            quality: SnapshotQuality::High,
            custom_dimensions: [320, 240],
            album_enabled: true,
            quick_tips: false,
        }
    }
}

/// Commands emitted when the player interacts with [`CameraPanel`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraCommand {
    /// Selected a snapshot frame size (`0x503f85..0x503ff0`).
    SetSize(SnapshotSize),
    /// Selected a snapshot compression quality (`0x503e0c..0x503e83`).
    SetQuality(SnapshotQuality),
    /// Pressed `kCamViewAlbum` (`1105`) to open the Photo Album (`0x503e85`).
    OpenPhotoAlbum,
    /// Requested the Camera Mode help balloon for a size/quality button.
    Help(CameraHelp),
}

/// Localized coordinates, tooltips, and help-balloon strings loaded from
/// `GameData/UIText.iff` `STR# 140` (`0x5034d7..0x503855`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraStrings {
    /// Strip-relative button positions parsed from even entries `0, 2, .., 14`.
    pub origins: [[i32; 2]; 8],
    /// Button tooltips from odd entries `1, 3, .., 15`.
    pub tooltips: [String; 8],
    /// `STR# 140[16..=17]`: Snapshot Sizes help balloon title and body.
    pub size_help_title: String,
    pub size_help_body: String,
    /// `STR# 140[18..=19]`: Quality Settings help balloon title and body.
    pub quality_help_title: String,
    pub quality_help_body: String,
}

impl Default for CameraStrings {
    fn default() -> Self {
        Self {
            origins: CameraButton::ALL.map(CameraButton::default_origin),
            tooltips: std::array::from_fn(|_| String::new()),
            size_help_title: String::new(),
            size_help_body: String::new(),
            quality_help_title: String::new(),
            quality_help_body: String::new(),
        }
    }
}

impl CameraStrings {
    /// Loads `STR# 140` from the install's `GameData/UIText.iff`.
    pub fn load(install: &Install, language: u8) -> Result<Self> {
        let path = install
            .file("GameData/UIText.iff")
            .map_err(Error::Content)?;
        let bytes = std::fs::read(&path).map_err(Error::Io)?;
        Self::from_iff_bytes(bytes, language)
    }

    /// Parses `STR# 140` from raw `UIText.iff` bytes.
    pub fn from_iff_bytes(bytes: Vec<u8>, language: u8) -> Result<Self> {
        let iff = Iff::parse(bytes).map_err(Error::Format)?;
        let mut out = Self::default();
        let Some(chunk) = iff.get(b"STR#", CAM_MODE_STRINGS) else {
            return Ok(out);
        };
        let table = StringTable::parse(iff.data(chunk)).map_err(Error::Format)?;
        let get = |idx: usize| -> Option<&str> {
            table
                .get(idx, language)
                .or_else(|| table.get(idx, 1))
                .filter(|s| !s.is_empty())
        };
        for btn in CameraButton::ALL {
            let i = btn.index();
            if let Some(coord_str) = get(i * 2)
                && let Some(pos) = parse_point(coord_str)
            {
                out.origins[i] = pos;
            }
            if let Some(tip) = get(i * 2 + 1) {
                out.tooltips[i] = tip.to_owned();
            }
        }
        if let Some(s) = get(16) {
            out.size_help_title = s.to_owned();
        }
        if let Some(s) = get(17) {
            out.size_help_body = normalize_crlf(s);
        }
        if let Some(s) = get(18) {
            out.quality_help_title = s.to_owned();
        }
        if let Some(s) = get(19) {
            out.quality_help_body = normalize_crlf(s);
        }
        Ok(out)
    }

    /// Tooltip text for `button`.
    pub fn tooltip(&self, button: CameraButton) -> &str {
        &self.tooltips[button.index()]
    }
}

fn parse_point(s: &str) -> Option<[i32; 2]> {
    let inner = s.trim().strip_prefix('(')?.strip_suffix(')')?;
    let (xs, ys) = inner.split_once(';')?;
    let x: i32 = xs.trim().parse().ok()?;
    let y: i32 = ys.trim().parse().ok()?;
    // Reject hostile or absurdly large coordinates from malformed string tables.
    if !(-4096..=4096).contains(&x) || !(-4096..=4096).contains(&y) {
        return None;
    }
    Some([x, y])
}

fn normalize_crlf(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\r', "\n")
}

/// Laid-out position for one visible button inside [`CameraLayout`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CameraButtonPlacement {
    pub button: CameraButton,
    pub rect: Rect,
}

/// Laid-out geometry for [`CameraPanel`] inside a control-panel strip rectangle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraLayout {
    pub strip: Rect,
    pub compact: bool,
    pub toothpick_left: Option<Rect>,
    pub toothpick_right: Option<Rect>,
    pub buttons: Vec<CameraButtonPlacement>,
}

impl CameraLayout {
    /// Minimum strip width that fits the native 1.91 layout (`226 + 61 + 8 = 295`).
    pub const MIN_NATIVE_WIDTH: i32 = 296;

    /// Computes button and divider rectangles inside `strip`.
    ///
    /// When `strip.width >= MIN_NATIVE_WIDTH` and `strip.height >= 100`, every
    /// control matches `cWinCamera`'s exact pixel coordinates from `STR# 140`
    /// and `0x503c67..0x503cc4`. Narrower strips pack visible buttons cleanly
    /// without overlapping hit boxes.
    pub fn compute(strip: Rect, _state: &CameraState, strings: &CameraStrings, art: &Art) -> Self {
        if strip.width <= 0 || strip.height <= 0 {
            return Self {
                strip,
                compact: false,
                toothpick_left: None,
                toothpick_right: None,
                buttons: Vec::new(),
            };
        }

        let compact = strip.width < Self::MIN_NATIVE_WIDTH || strip.height < hud::STRIP;
        let mut buttons = Vec::with_capacity(8);

        if !compact {
            let strip_right = strip.x.saturating_add(strip.width);
            let strip_bottom = strip.y.saturating_add(strip.height);
            let tp_rect = |pos: [i32; 2], art_id: i32| {
                let (w, h) = art
                    .image(art_id)
                    .map_or((2, 89), |img| (img.width as i32, img.height as i32));
                Rect::new(
                    strip.x.saturating_add(pos[0]),
                    strip.y.saturating_add(pos[1]),
                    w,
                    h.min(strip.height),
                )
            };
            let toothpick_left = Some(tp_rect(TOOTHPICK_LEFT_POS, id::TOOTHPICK_LEFT));
            let toothpick_right = Some(tp_rect(TOOTHPICK_RIGHT_POS, id::TOOTHPICK_RIGHT));

            // 0x5041a8..0x5041da: native `cWinCamera` creates all 8 buttons and
            // toggles Custom (`1104`) between enabled (cell 1) and disabled (cell 3).
            for btn in CameraButton::ALL {
                let [ox, oy] = strings.origins[btn.index()];
                let [cw, ch] = button_cell_size(art, btn);
                let x = strip.x.saturating_add(ox);
                let y = strip.y.saturating_add(oy);
                let x0 = x.clamp(strip.x, strip_right);
                let y0 = y.clamp(strip.y, strip_bottom);
                let x1 = x.saturating_add(cw).clamp(x0, strip_right);
                let y1 = y.saturating_add(ch).clamp(y0, strip_bottom);
                if x1 > x0 && y1 > y0 {
                    buttons.push(CameraButtonPlacement {
                        button: btn,
                        rect: Rect::new(x0, y0, x1 - x0, y1 - y0),
                    });
                }
            }

            Self {
                strip,
                compact: false,
                toothpick_left,
                toothpick_right,
                buttons,
            }
        } else {
            // Compact non-overlapping layout: 3 columns (Size stack, Quality stack,
            // View Album) packed inside `strip`.
            let pad = 4;
            let avail_w = (strip.width - pad * 2).max(1);
            let avail_h = (strip.height - pad * 2).max(1);
            let col_w = (avail_w / 3).max(1);
            let row_h = (avail_h / 3).max(1);
            let stack = [
                (CameraButton::Large, 0, 0),
                (CameraButton::Medium, 0, 1),
                (CameraButton::Small, 0, 2),
                (CameraButton::QualityHigh, 1, 0),
                (CameraButton::QualityMedium, 1, 1),
                (CameraButton::QualityLow, 1, 2),
            ];
            for (btn, col, row) in stack {
                let x = strip.x + pad + col * col_w;
                let y = strip.y + pad + row * row_h;
                let w = (col_w - 2).max(1).min((strip.x + strip.width - x).max(0));
                let h = (row_h - 1).max(1).min((strip.y + strip.height - y).max(0));
                if w > 0 && h > 0 {
                    buttons.push(CameraButtonPlacement {
                        button: btn,
                        rect: Rect::new(x, y, w, h),
                    });
                }
            }
            let album_x = strip.x + pad + col_w * 2;
            let album_w = (strip.x + strip.width - pad - album_x).max(0);
            let album_h = (avail_h / 2).clamp(1, 35);
            let album_y = strip.y + (strip.height - album_h) / 2;
            if album_w > 0 && album_h > 0 {
                buttons.push(CameraButtonPlacement {
                    button: CameraButton::ViewAlbum,
                    rect: Rect::new(album_x, album_y, album_w, album_h),
                });
            }
            Self {
                strip,
                compact: true,
                toothpick_left: None,
                toothpick_right: None,
                buttons,
            }
        }
    }

    /// Returns the visible button whose rectangle contains `(x, y)`, if any.
    pub fn hit(&self, x: i32, y: i32) -> Option<CameraButton> {
        self.buttons
            .iter()
            .find(|p| p.rect.contains(x, y))
            .map(|p| p.button)
    }

    /// Looks up the placement for `button`, if visible.
    pub fn button(&self, button: CameraButton) -> Option<CameraButtonPlacement> {
        self.buttons.iter().copied().find(|p| p.button == button)
    }
}

/// Interactive state for the Complete Collection Camera Mode subpanel (`cWinCamera`).
#[derive(Debug, Clone, Default)]
pub struct CameraPanel {
    hovered: Option<CameraButton>,
    pressed: Option<CameraButton>,
    pointer: [i32; 2],
}

impl CameraPanel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn hovered(&self) -> Option<CameraButton> {
        self.hovered
    }

    pub fn pressed(&self) -> Option<CameraButton> {
        self.pressed
    }

    pub fn clear_pointer(&mut self) {
        self.hovered = None;
        self.pressed = None;
    }

    /// Standard control-panel strip rectangle for a viewport of `[width, height]`.
    pub fn strip_rect(frame_size: [u32; 2]) -> Rect {
        let w = i32::try_from(frame_size[0]).unwrap_or(i32::MAX).max(0);
        let h = i32::try_from(frame_size[1]).unwrap_or(i32::MAX).max(0);
        let x = hud::CLUSTER[0].min(w);
        let y = (h - hud::STRIP).max(0);
        Rect::new(x, y, (w - x).max(0), hud::STRIP.min(h))
    }

    /// Updates hover state for pointer `(x, y)`.
    pub fn hover(
        &mut self,
        x: i32,
        y: i32,
        strip: Rect,
        state: &CameraState,
        strings: &CameraStrings,
        art: &Art,
    ) -> Option<CameraButton> {
        self.pointer = [x, y];
        let layout = CameraLayout::compute(strip, state, strings, art);
        self.hovered = layout.hit(x, y);
        self.hovered
    }

    /// Records a mouse button press at `(x, y)`. Returns `true` if the Camera
    /// subpanel strip captured the press.
    pub fn press(
        &mut self,
        x: i32,
        y: i32,
        strip: Rect,
        state: &CameraState,
        strings: &CameraStrings,
        art: &Art,
    ) -> bool {
        self.pointer = [x, y];
        let layout = CameraLayout::compute(strip, state, strings, art);
        let hit = layout.hit(x, y);
        self.hovered = hit;
        if let Some(btn) = hit {
            if is_enabled(btn, state) {
                self.pressed = Some(btn);
            }
            return true;
        }
        strip.contains(x, y)
    }

    /// Returns `Some(CameraCommand::Help(topic))` when Quick Tips are active
    /// and the pointer is hovering a size or quality button (`0x503d43..0x5040ff`).
    pub fn quick_tip_command(&self, state: &CameraState) -> Option<CameraCommand> {
        if !state.quick_tips {
            return None;
        }
        self.hovered?.help_topic().map(CameraCommand::Help)
    }

    /// Lays out the active Camera Mode Quick Tips help balloon above `strip`
    /// when `state.quick_tips` is enabled and the pointer hovers a size or
    /// quality control (`0x504008..0x504116`).
    pub fn help_balloon(
        &self,
        strip: Rect,
        state: &CameraState,
        strings: &CameraStrings,
        art: &Art,
    ) -> Option<balloon::Balloon> {
        if !state.quick_tips {
            return None;
        }
        let topic = self.hovered?.help_topic()?;
        let text = topic.balloon_text(strings);
        Some(balloon::lay_out(art, &text, strip))
    }

    /// Completes a mouse release at `(x, y)`, emitting a [`CameraCommand`] when
    /// released over the same enabled button that was pressed.
    pub fn release(
        &mut self,
        x: i32,
        y: i32,
        strip: Rect,
        state: &CameraState,
        strings: &CameraStrings,
        art: &Art,
    ) -> Option<CameraCommand> {
        self.pointer = [x, y];
        let pressed = self.pressed.take();
        let layout = CameraLayout::compute(strip, state, strings, art);
        let hit = layout.hit(x, y);
        self.hovered = hit;
        let btn = pressed.filter(|&b| Some(b) == hit && is_enabled(b, state))?;
        if state.quick_tips
            && let Some(topic) = btn.help_topic()
        {
            return Some(CameraCommand::Help(topic));
        }
        Some(match btn {
            CameraButton::Small => CameraCommand::SetSize(SnapshotSize::Small),
            CameraButton::Medium => CameraCommand::SetSize(SnapshotSize::Medium),
            CameraButton::Large => CameraCommand::SetSize(SnapshotSize::Large),
            CameraButton::Custom => CameraCommand::SetSize(SnapshotSize::Custom),
            CameraButton::ViewAlbum => CameraCommand::OpenPhotoAlbum,
            CameraButton::QualityLow => CameraCommand::SetQuality(SnapshotQuality::Low),
            CameraButton::QualityMedium => CameraCommand::SetQuality(SnapshotQuality::Medium),
            CameraButton::QualityHigh => CameraCommand::SetQuality(SnapshotQuality::High),
        })
    }

    /// Draws the Camera Mode subpanel (`PanelBack.BMP`, vertical dividers,
    /// 4-cell size/quality/album buttons, optional custom dimension readout,
    /// and hover tooltip) into `frame` at `strip`.
    ///
    /// Pass `draw_strip_background = false` if the caller has already drawn the
    /// shared `hud::id::STRIP` background underneath the control-panel cluster.
    pub fn draw(
        &self,
        frame: &mut Frame,
        strip: Rect,
        state: &CameraState,
        strings: &CameraStrings,
        art: &Art,
        draw_strip_background: bool,
    ) -> CameraLayout {
        let layout = CameraLayout::compute(strip, state, strings, art);
        if strip.width <= 0 || strip.height <= 0 {
            return layout;
        }

        if draw_strip_background {
            draw_strip_bkg(frame, strip, art);
        }

        if let Some(tp) = layout.toothpick_left
            && let Some(img) = art.image(id::TOOTHPICK_LEFT)
        {
            frame.draw(img, Rect::new(0, 0, tp.width, tp.height), [tp.x, tp.y]);
        }
        if let Some(tp) = layout.toothpick_right
            && let Some(img) = art.image(id::TOOTHPICK_RIGHT)
        {
            frame.draw(img, Rect::new(0, 0, tp.width, tp.height), [tp.x, tp.y]);
        }

        for placement in &layout.buttons {
            let btn = placement.button;
            let enabled = is_enabled(btn, state);
            let cell = if !enabled {
                3
            } else if btn.is_selected(state) || self.pressed == Some(btn) {
                1
            } else if self.hovered == Some(btn) {
                2
            } else {
                0
            };
            draw_four_cell_button(frame, art, btn.art_id(), cell, placement.rect);

            // 0x503688..0x5036f5 & 0x5041e0..0x504277: custom size button
            // draws "<w> x <h>" centered along its bottom edge in 9 pt font
            // only while Custom size is active.
            if btn == CameraButton::Custom
                && matches!(state.size, SnapshotSize::Custom)
                && let Some(font) = art.font(9).or_else(|| art.font(8))
            {
                let label = format!(
                    "{} x {}",
                    state.custom_dimensions[0], state.custom_dimensions[1]
                );
                let tw = font.width(&label);
                let tx = placement.rect.x + (placement.rect.width - tw) / 2;
                let ty = placement.rect.y + placement.rect.height - font.line_height;
                font.draw(frame, &label, [tx, ty.max(placement.rect.y)], LABEL);
            }
        }

        if let Some(help) = self.help_balloon(strip, state, strings, art) {
            balloon::draw(frame, art, &help);
        } else if let Some(hovered) = self.hovered
            && let Some(placement) = layout.button(hovered)
        {
            let tip = strings.tooltip(hovered);
            if !tip.is_empty() {
                draw_tooltip(
                    frame,
                    art,
                    tip,
                    [
                        placement.rect.x + placement.rect.width / 2,
                        (placement.rect.y - 18).max(0),
                    ],
                );
            }
        }

        layout
    }

    /// Draws `kCameraPatch` (`2035`) over the control-panel cluster at its
    /// native offset `(165, 83)` (`0x53a1a9..0x53a1c3`) when Camera Mode is open.
    pub fn draw_cluster_patch(frame: &mut Frame, cluster_origin: [i32; 2], art: &Art) {
        if let Some(patch) = art.image(id::PATCH) {
            frame.draw_all(
                patch,
                [
                    cluster_origin[0].saturating_add(PATCH_ORIGIN[0]),
                    cluster_origin[1].saturating_add(PATCH_ORIGIN[1]),
                ],
            );
        }
    }
}

const fn is_enabled(button: CameraButton, state: &CameraState) -> bool {
    match button {
        CameraButton::ViewAlbum => state.album_enabled,
        CameraButton::Custom => matches!(state.size, SnapshotSize::Custom),
        _ => true,
    }
}

fn button_cell_size(art: &Art, button: CameraButton) -> [i32; 2] {
    if let Some(img) = art.image(button.art_id())
        && img.width >= 4
        && img.height > 0
    {
        return [(img.width / 4) as i32, img.height as i32];
    }
    button.native_size()
}

fn draw_strip_bkg(frame: &mut Frame, strip: Rect, art: &Art) {
    if let Some(bg) = art.image(id::STRIP)
        && bg.width > 0
        && bg.height > 0
    {
        let bw = bg.width as i32;
        let bh = (bg.height as i32).min(strip.height);
        let fw = i32::try_from(frame.width()).unwrap_or(i32::MAX);
        let visible_w = strip.width.min(fw.saturating_sub(strip.x).max(0));
        if strip.width > bw && bw > 16 {
            // Preserve the 8 px right bezel of `PanelBack.bmp` (`804×100`) on
            // viewports wider than 1024 px, matching `hud.rs` `Kind::Stretched`.
            const RIGHT_BEZEL: i32 = 8;
            let body_w = bw - RIGHT_BEZEL;
            let fill_col = bw - 16;
            frame.draw(
                bg,
                Rect::new(0, 0, body_w.min(visible_w), bh),
                [strip.x, strip.y],
            );
            let bezel_x = strip.width - RIGHT_BEZEL;
            let fill_end = bezel_x.min(visible_w);
            for x in body_w..fill_end {
                frame.draw(bg, Rect::new(fill_col, 0, 1, bh), [strip.x + x, strip.y]);
            }
            if visible_w > bezel_x {
                let draw_w = (visible_w - bezel_x).min(RIGHT_BEZEL);
                frame.draw(
                    bg,
                    Rect::new(body_w, 0, draw_w, bh),
                    [strip.x + bezel_x, strip.y],
                );
            }
        } else {
            let direct_w = bw.min(visible_w);
            frame.draw(bg, Rect::new(0, 0, direct_w, bh), [strip.x, strip.y]);
            for x in direct_w..visible_w {
                frame.draw(bg, Rect::new(bw - 1, 0, 1, bh), [strip.x + x, strip.y]);
            }
        }
    } else {
        frame.fill(strip, [24, 34, 112]);
    }
}

fn draw_four_cell_button(frame: &mut Frame, art: &Art, id: i32, cell: u32, dest: Rect) {
    if dest.width <= 0 || dest.height <= 0 {
        return;
    }
    if let Some(img) = art.image(id)
        && img.width >= 4
        && img.height > 0
    {
        let cw = (img.width / 4) as i32;
        let ch = img.height as i32;
        let sx = cw.saturating_mul(cell.min(3) as i32);
        if dest.width == cw && dest.height == ch {
            frame.draw(img, Rect::new(sx, 0, cw, ch), [dest.x, dest.y]);
        } else {
            let fw = i32::try_from(frame.width()).unwrap_or(i32::MAX);
            let fh = i32::try_from(frame.height()).unwrap_or(i32::MAX);
            let x0 = dest.x.max(0);
            let y0 = dest.y.max(0);
            let x1 = dest.x.saturating_add(dest.width).min(fw);
            let y1 = dest.y.saturating_add(dest.height).min(fh);
            for y in y0..y1 {
                let sy = ((i64::from(y - dest.y) * i64::from(ch)) / i64::from(dest.height)) as u32;
                for x in x0..x1 {
                    let rx =
                        ((i64::from(x - dest.x) * i64::from(cw)) / i64::from(dest.width)) as u32;
                    if let Some([r, g, b, a]) = img.pixel(sx as u32 + rx, sy) {
                        frame.blend(x, y, [r, g, b], a);
                    }
                }
            }
        }
    } else {
        let color = match cell {
            1 => [0, 180, 220],
            2 => [80, 120, 210],
            3 => [48, 56, 88],
            _ => [40, 64, 156],
        };
        frame.fill(dest, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_sizes_are_the_native_table() {
        assert_eq!(SnapshotSize::Small.dimensions([0, 0]), [200, 150]);
        assert_eq!(SnapshotSize::Medium.dimensions([0, 0]), [400, 300]);
        assert_eq!(SnapshotSize::Large.dimensions([0, 0]), [600, 400]);
        assert_eq!(SnapshotSize::Custom.dimensions([10, 900]), [200, 400]);
        assert_eq!(SnapshotSize::Custom.dimensions([321, 222]), [321, 222]);
        assert_eq!(SnapshotQuality::Low.jpeg_quality(), 20);
        assert_eq!(SnapshotQuality::Medium.jpeg_quality(), 50);
        assert_eq!(SnapshotQuality::High.jpeg_quality(), 90);
    }

    #[test]
    fn the_snapshot_frame_ends_at_the_pointer_and_stays_in_the_viewport() {
        let view = [0, 0, 800, 500];
        // Inside: centred on x, ending at y.
        assert_eq!(
            snapshot_frame([400, 450], [400, 300], view),
            [199, 148, 599, 450]
        );
        assert_eq!(
            snapshot_capture_rect([400, 450], [400, 300], view),
            [200, 149, 598, 449]
        );
        // Odd widths halve toward zero.
        assert_eq!(
            snapshot_frame([400, 450], [201, 150], view)[..3],
            [299, 298, 499]
        );
        // Off the left and top: shifted right and down.
        assert_eq!(snapshot_frame([10, 20], [200, 150], view), [0, 0, 200, 152]);
        // Off the right and bottom: shifted left and up, one short of the edge.
        assert_eq!(
            snapshot_frame([795, 499], [200, 150], view),
            [599, 347, 799, 499]
        );
        // Larger than the viewport: shifted, then intersected.
        assert_eq!(
            snapshot_frame([100, 100], [600, 400], [0, 0, 320, 240]),
            [0, 0, 320, 240]
        );
    }

    fn rects_overlap(a: Rect, b: Rect) -> bool {
        a.width > 0
            && a.height > 0
            && b.width > 0
            && b.height > 0
            && a.x < b.x + b.width
            && a.x + a.width > b.x
            && a.y < b.y + b.height
            && a.y + a.height > b.y
    }

    #[test]
    fn native_layout_matches_191_coords_and_keeps_custom_button_in_strip() {
        let art = Art::default();
        let strings = CameraStrings::default();
        let strip = CameraPanel::strip_rect([800, 600]);
        assert_eq!(strip, Rect::new(220, 500, 580, 100));

        // All 8 native buttons are placed (Custom sits at (11, 11) in disabled cell 3
        // until Custom size is active, matching `0x5041a8..0x5041da`).
        let mut state = CameraState::default();
        let layout = CameraLayout::compute(strip, &state, &strings, &art);
        assert!(!layout.compact);
        assert_eq!(layout.buttons.len(), 8);
        assert_eq!(
            layout.button(CameraButton::Custom).unwrap().rect,
            Rect::new(220 + 11, 500 + 11, 60, 58)
        );
        assert!(!is_enabled(CameraButton::Custom, &state));
        assert_eq!(
            layout.button(CameraButton::ViewAlbum).unwrap().rect,
            Rect::new(220 + 226, 500 + 35, 61, 35)
        );
        assert_eq!(
            layout.toothpick_left.unwrap(),
            Rect::new(220 + 133, 500 + 6, 2, 89)
        );
        assert_eq!(
            layout.toothpick_right.unwrap(),
            Rect::new(220 + 206, 500 + 6, 2, 89)
        );

        state.size = SnapshotSize::Custom;
        assert!(is_enabled(CameraButton::Custom, &state));
        let with_custom = CameraLayout::compute(strip, &state, &strings, &art);
        assert_eq!(with_custom.buttons.len(), 8);
        for i in 0..with_custom.buttons.len() {
            for j in (i + 1)..with_custom.buttons.len() {
                assert!(
                    !rects_overlap(with_custom.buttons[i].rect, with_custom.buttons[j].rect),
                    "buttons {:?} and {:?} overlap",
                    with_custom.buttons[i].button,
                    with_custom.buttons[j].button
                );
            }
        }

        // Out-of-range coordinates in malformed STR# 140 are rejected safely.
        assert_eq!(parse_point("(2147483647;0)"), None);
        assert_eq!(parse_point("(-5000;10)"), None);
        let mut bad_strings = CameraStrings::default();
        bad_strings.origins[CameraButton::Small.index()] = [10_000, 10_000];
        let clamped = CameraLayout::compute(strip, &state, &bad_strings, &art);
        assert!(clamped.button(CameraButton::Small).is_none());
    }

    #[test]
    fn click_interactions_emit_size_quality_album_and_quick_tip_help_commands() {
        let art = Art::default();
        let strings = CameraStrings::default();
        let strip = CameraPanel::strip_rect([800, 600]);
        let mut state = CameraState::default();
        let mut panel = CameraPanel::new();
        let layout = CameraLayout::compute(strip, &state, &strings, &art);

        let click = |panel: &mut CameraPanel, state: &CameraState, btn: CameraButton| {
            let r = layout.button(btn).unwrap().rect;
            let (cx, cy) = (r.x + r.width / 2, r.y + r.height / 2);
            assert!(panel.press(cx, cy, strip, state, &strings, &art));
            panel.release(cx, cy, strip, state, &strings, &art)
        };

        assert_eq!(
            click(&mut panel, &state, CameraButton::Large),
            Some(CameraCommand::SetSize(SnapshotSize::Large))
        );
        assert_eq!(
            click(&mut panel, &state, CameraButton::QualityLow),
            Some(CameraCommand::SetQuality(SnapshotQuality::Low))
        );
        assert_eq!(
            click(&mut panel, &state, CameraButton::ViewAlbum),
            Some(CameraCommand::OpenPhotoAlbum)
        );
        // Custom is disabled while Medium is active.
        assert_eq!(click(&mut panel, &state, CameraButton::Custom), None);

        // Quick Tips mode routes size/quality controls to `CameraCommand::Help`.
        state.quick_tips = true;
        let qr = layout.button(CameraButton::QualityHigh).unwrap().rect;
        panel.hover(qr.x + 4, qr.y + 4, strip, &state, &strings, &art);
        assert_eq!(
            panel.quick_tip_command(&state),
            Some(CameraCommand::Help(CameraHelp::QualitySettings))
        );
        assert!(panel.help_balloon(strip, &state, &strings, &art).is_some());
        assert_eq!(
            click(&mut panel, &state, CameraButton::Large),
            Some(CameraCommand::Help(CameraHelp::SnapshotSizes))
        );
        // ViewAlbum still opens the album directly in Quick Tips mode.
        assert_eq!(
            click(&mut panel, &state, CameraButton::ViewAlbum),
            Some(CameraCommand::OpenPhotoAlbum)
        );

        // Disabled ViewAlbum ignores activation.
        state.album_enabled = false;
        assert_eq!(click(&mut panel, &state, CameraButton::ViewAlbum), None);
    }

    #[test]
    fn compact_strip_packs_buttons_without_overlap() {
        let art = Art::default();
        let strings = CameraStrings::default();
        let state = CameraState::default();
        let strip = Rect::new(0, 0, 200, 80);
        let layout = CameraLayout::compute(strip, &state, &strings, &art);
        assert!(layout.compact);
        assert_eq!(layout.buttons.len(), 7);
        for i in 0..layout.buttons.len() {
            let a = layout.buttons[i].rect;
            assert!(a.x >= 0 && a.x + a.width <= 200);
            assert!(a.y >= 0 && a.y + a.height <= 80);
            for j in (i + 1)..layout.buttons.len() {
                assert!(!rects_overlap(a, layout.buttons[j].rect));
            }
        }
    }

    #[test]
    fn help_topics_build_balloon_texts_with_native_popup_images() {
        let strings = CameraStrings {
            size_help_title: "Sizes".into(),
            size_help_body: "Pick size".into(),
            quality_help_title: "Quality".into(),
            quality_help_body: "Pick quality".into(),
            ..Default::default()
        };
        let sz = CameraHelp::SnapshotSizes.balloon_text(&strings);
        assert_eq!(sz.title, "Sizes");
        assert_eq!(sz.pictures, [Some(Picture::Image(id::POPUP_SIZE)), None]);
        let ql = CameraHelp::QualitySettings.balloon_text(&strings);
        assert_eq!(ql.title, "Quality");
        assert_eq!(ql.pictures, [Some(Picture::Image(id::POPUP_QUALITY)), None]);
    }
}
