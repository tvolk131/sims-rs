//! Create-a-Family (CAF) 3D Sim dress-up turntable preview, CAF head/body
//! catalog enumerator, and 3D Sim portrait renderer (`BMP_` 2002 style).
//!
//! Ported from Complete Collection 1.91 `Sims.exe`:
//!
//! - **CAF head & body catalog enumeration** (`0x408bf5`, `0x409663`,
//!   `0x40a364`, `0x409255`, `0x40895a`, `0x40b050`):
//!   - Skips any appearance whose name begins with `"suit-"` (`0x409255`).
//!   - **Bodies** (`0x409663`): matches appearances starting with `b` + 3
//!     arbitrary bytes + gender (`m`|`u` for male, `f`|`u` for female) + age
//!     (`a` for adult, `c` for child) + 3 arbitrary bytes (body type, e.g.
//!     `fit`, `fat`, `skn`, `chd`). For each matched 9-byte prefix `P`, pairs
//!     it with every `.bmp` texture stem starting with `P + skin_tone`
//!     (`lgt`, `med`, `drk`), case-insensitively, as `BODY=<texture>`.
//!   - **Heads** (`0x40a364`): matches appearances starting with `c` + 3
//!     arbitrary bytes + gender (`m`|`u` or `f`|`u`) + age (`a` or `c`),
//!     explicitly rejecting any head whose 2nd byte is `'t'`/`'T'`
//!     (`0x40a450`–`0x40a461`, skipping NPC/townie `ct*` heads). For each
//!     matched 6-byte prefix `P`, pairs it with every `.bmp` texture stem
//!     starting with `P + skin_tone`, as `HEAD-HEAD=<texture>`.
//!   - **Default CAF hands** (`0x40b050`–`0x40b298`): open hands `hulo`/`huro`
//!     for children, `hflo`/`hfro` for adult females, and `hmlo`/`hmro` for
//!     adult males, each bound to `HAND=huao<tone>`.
//! - **3D CAF turntable preview** (`cWin3DView` at `0x56cbfb`–`0x56d039`,
//!   hosted by `cWinDesignCharacter` at `0x546459`–`0x546486`):
//!   - `100×220` (`0x64 × 0xdc`) viewport placed at `(618, 145)` inside the
//!     `800×600` CAF screen (`[0x6f5050]`, `[0x6f5054]`).
//!   - Uses house camera rotation `0` (`0x546459` → `0x4a7dfe`) and zoom `1`
//!     (`Zoom::Far`, `tile_width = 16 << 1 = 32`, `0x54646b` → `0x4a7b92`), with
//!     model scale `5.0` (`[0x667678]`), yielding
//!     `5.0 / (3√2 / 32) = 80√2 / 3 ≈ 37.712` pixels per skeleton unit.
//!   - Model transform `RotY(theta) * Scale(5.0) * Translate(anchor)` where
//!     `anchor` unprojects screen `(width/2, height - 20)` (`0x56cf7a`–`0x56cfba`).
//!     Composed with the house camera's `rot = 0` view rotation
//!     `RotY(3π/4) * RotX(-π/6)` (`0x49b0de`), the projection is
//!     `RotY(theta + 3π/4) * RotX(-π/6)`.
//!   - Turntable oscillator (`0x56cd1f`–`0x56cd5d`, `0x473424`–`0x4734e2`):
//!     `theta(t) = (π/4) * sin(2π * t / 10000) + π/4`, oscillating smoothly
//!     between `0` and `π/2` around `+45°` (facing straight towards the camera).
//!   - Ambient lighting is set to full white `0x00ffffff` (`1.0`, `0x56ceb2`).
//! - **3D Sim portrait renderer** (`0x4e77c6` and wrappers `0x4e725f`
//!   `BMP_ 2002`, `0x4e7448` `BMP_ 2003`, `0x4e7671` `BMP_ 2004`, `0x4e81f6`
//!   `BMP_ 2005`/`2006`, `0x4e8400` `BMP_ 2007`):
//!   - Collects all non-censored parts attached to `"HEAD"` (`0x4e810d`),
//!     skinned in bind pose with identity bone transforms (`0x599bf2(1)`).
//!   - Renders into a `200×200` (`0xc8 × 0xc8`) supersampled 16-bit 5-6-5
//!     buffer with scale `30.0 / (1 << zoom)` (`[0x667660] = 30.0`), giving a
//!     zoom-invariant `80√2 ≈ 113.137` pixels per HEAD-bone unit, and view
//!     rotation `RotY(0.05π + pitch) * RotZ(π/2) * RotY(yaw + π) * RotX(-π/6)`.
//!   - Applies ambient `(1.0, 1.0, 1.0, 1.0)` (`0x4e7ad6`–`0x4e7b02`), crops
//!     either to the fixed rectangle `[76, 45, 122, 125]` (`[0x4c, 0x2d, 0x7a, 0x7d]`)
//!     or an auto-scanned tight bounding box (`0x4c357c`), expands symmetrically
//!     to match the destination aspect ratio (`0x4e7d4e`–`0x4e7e1c`), and
//!     downsamples with a two-pass separable cubic Hermite filter (`0x4ba79c`,
//!     `f(t) = (2|t| - 3)t² + 1` for `|t| < 1`).

use crate::animation::Clip;
use crate::appearance::{Body, Outfit};
use crate::assets::Assets;
use crate::lot::Packed565;
use crate::math::{Transform, Vec3};
use crate::pie_head::{BASE_PITCH, HEAD_BONE};
use crate::raster::{Canvas, Shade, Target, draw_shaded_culled};
use crate::rig::{self, Pose};
use crate::skin::skin;
use crate::suit::Wardrobe;
use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, FRAC_PI_6, PI, SQRT_2};

/// Largest canvas side accepted by the CAF preview and portrait renderers,
/// preventing `u32` area overflow in [`Canvas::new`].
pub const MAX_SIDE: u32 = 4096;

/// Native `cWin3DView` widget dimensions in the `800×600` Create-a-Family
/// screen (`0x64 × 0xdc`).
pub const PREVIEW_SIZE: [u32; 2] = [100, 220];

/// Top-left position of `cWin3DView` inside the `800×600` CAF screen
/// (`[0x6f5050] = 618`, `[0x6f5054] = 145`).
pub const PREVIEW_RECT_ORIGIN: [i32; 2] = [618, 145];

/// Native model scale factor passed to `cWin3DView` (`[0x667678] = 5.0`).
pub const PREVIEW_SCALE: f32 = 5.0;

/// Distance in pixels from the bottom edge of `cWin3DView` to the Sim's
/// projected root origin (`0x56cf98`: `rect.bottom - 0x14`).
pub const PREVIEW_FOOT_MARGIN: f32 = 20.0;

/// Screen pixels per skeleton unit in `cWin3DView` at native `Zoom::Far`
/// (`zoom = 1`, `tile_width = 32`, `5.0 / (3√2 / 32) = 80√2 / 3 ≈ 37.71236`).
pub const PREVIEW_PIXELS_PER_UNIT: f32 = (80.0 * SQRT_2) / 3.0;

/// Full oscillation period of the CAF turntable in milliseconds (`0x56cd3d`: `0x2710`).
pub const TURNTABLE_PERIOD_MS: u32 = 10_000;

/// Half-sweep amplitude of the CAF turntable yaw (`0x56cd1f`: `0.25 * π`).
pub const TURNTABLE_AMPLITUDE: f32 = FRAC_PI_4;

/// Center yaw angle of the CAF turntable (`0x56cf39`: `0.25 * π`), which faces
/// the Sim straight towards the camera at house rotation `0`.
pub const TURNTABLE_BASE_YAW: f32 = FRAC_PI_4;

/// Ambient light level used by `cWin3DView` (`0x56ceb2`: `0x00ffffff`).
pub const PREVIEW_AMBIENT: f32 = 1.0;

/// Common CAF / dress-up idle animation clips for adults (`a2o-*`) referenced
/// by `Sims.exe` (`cWinDesignCharacter` / wardrobe `cWin3DView`).
pub const ADULT_IDLE_CLIPS: &[&str] = &[
    "a2o-idle-armsdown-breathe",
    "a2o-idle-transition-handships-armscrossed",
    "a2o-heyyou1",
    "a2o-celebrate-short",
];

/// Common CAF / dress-up idle animation clips for children (`c2o-*`)
/// referenced by `Sims.exe`.
pub const CHILD_IDLE_CLIPS: &[&str] =
    &["c2o-idle-armsdown-breathe", "c2o-heyyou1", "c2o-happy-clap"];

/// Intermediate supersampling resolution of `Sims.exe`'s 3D portrait renderer
/// (`0x4e77c6`: `0xc8 × 0xc8`).
pub const PORTRAIT_SUPERSAMPLE_SIZE: u32 = 200;

/// Zoom-invariant portrait scale in intermediate pixels per HEAD-bone unit:
/// `(30.0 / (1 << zoom)) / (3√2 / (16 << zoom)) = 80√2 ≈ 113.13708`
/// (`0x4e79ad` `[0x667660] = 30.0`, `0x49b0de`).
pub const PORTRAIT_SCALE_PIXELS: f32 = 80.0 * SQRT_2;

/// Fixed crop rectangle `[left, top, right, bottom]` inside the `200×200`
/// intermediate buffer passed by `BMP_` 2002, 2003, 2004, and 2007
/// (`0x4e736a`–`0x4e7384`: `[0x4c, 0x2d, 0x7a, 0x7d]`).
pub const PORTRAIT_FIXED_CROP: [u32; 4] = [76, 45, 122, 125];

/// Single frame dimensions of `BMP_` 2002 (`'faces'`, `[0x6f2660] = 35`,
/// `[0x6f2664] = 41`).
pub const FACES_2002_FRAME_SIZE: [u32; 2] = [35, 41];

/// Full 3-frame strip dimensions of `BMP_` 2002 (`'faces'`).
pub const FACES_2002_STRIP_SIZE: [u32; 2] = [105, 41];

/// Native 16-bit 5-6-5 background colour of `BMP_` 2002 (`0x4e7351`: `0x000a`,
/// dark navy blue `(0, 0, 80)`).
pub const FACES_2002_BG_565: u16 = 0x000a;

/// Native `(pitch, yaw)` angles in radians for the 3 frames of `BMP_` 2002
/// (`0x4e72f2`–`0x4e7338`: neutral, happy/up `+9°/-7°`, sad/down `-9°/+7°`).
pub const FACES_2002_ANGLES: [(f32, f32); 3] = [
    (0.0, 0.0),
    (0.05 * PI, -0.038_888_89 * PI),
    (-0.05 * PI, 0.038_888_89 * PI),
];

/// Single frame dimensions of `BMP_` 2003 (`'rel. images'`).
pub const REL_2003_FRAME_SIZE: [u32; 2] = [40, 40];

/// Full 5-frame strip dimensions of `BMP_` 2003 (`'rel. images'`).
pub const REL_2003_STRIP_SIZE: [u32; 2] = [200, 40];

/// Native pitch and yaw for `BMP_` 2003, 2005, and 2006 (`[0x66941c] = -5°`,
/// `[0x669420] = +18°`).
pub const REL_2003_PITCH: f32 = -0.027_777_778 * PI;
pub const REL_2003_YAW: f32 = 0.10 * PI;

/// Native 5-6-5 background colours for the 5 relationship frames of `BMP_` 2003
/// (`0x4e7448` → `0x4e7556`), matching `User00172.iff`.
pub const REL_2003_BG_565: [u16; 5] = [
    pack_rgb565([0xd1, 0x00, 0x00]),
    pack_rgb565([0x88, 0x2e, 0x2f]),
    pack_rgb565([0x40, 0x5d, 0x5f]),
    pack_rgb565([0x20, 0x93, 0x4c]),
    pack_rgb565([0x00, 0xca, 0x39]),
];

/// Native 5-6-5 background colour for `BMP_` 2004 (`'newimage'`, `25×25`).
pub const NEW_IMAGE_2004_BG_565: u16 = pack_rgb565([0x00, 0x00, 0xf8]);

/// Native 5-6-5 background colour for `BMP_` 2005 (`16×16`) and `BMP_` 2006
/// (`32×32`) speech-bubble head thumbnails.
pub const SPEECH_BG_565: u16 = pack_rgb565([0xf8, 0xfc, 0xf8]);

/// Native 5-6-5 background colour for `BMP_` 2007 (`'web_image'`, `45×45`
/// outer matte).
pub const WEB_IMAGE_2007_BG_565: u16 = pack_rgb565([0xc8, 0xc4, 0xc0]);

/// Sim age category in Create-a-Family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Age {
    Adult,
    Child,
}

impl Age {
    /// Skeleton name (`"adult"` or `"child"`).
    pub const fn skeleton_name(self) -> &'static str {
        match self {
            Self::Adult => "adult",
            Self::Child => "child",
        }
    }

    /// Single-letter appearance/texture age code (`'a'` or `'c'`,
    /// `0x6c0a74` / `0x6c0f94`).
    pub const fn code(self) -> u8 {
        match self {
            Self::Adult => b'a',
            Self::Child => b'c',
        }
    }

    /// Default numeric age stored in `STR# 200` row 13 (`"27"` or `"9"`).
    pub const fn default_age_years(self) -> u8 {
        match self {
            Self::Adult => 27,
            Self::Child => 9,
        }
    }

    /// Default CAF idle animation clip names for this age.
    pub const fn idle_clips(self) -> &'static [&'static str] {
        match self {
            Self::Adult => ADULT_IDLE_CLIPS,
            Self::Child => CHILD_IDLE_CLIPS,
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
    /// Single-letter appearance/texture gender code (`'m'` or `'f'`).
    pub const fn code(self) -> u8 {
        match self {
            Self::Male => b'm',
            Self::Female => b'f',
        }
    }

    /// Profile string stored in `STR# 200` row 12 (`"male"` or `"female"`).
    pub const fn profile_name(self) -> &'static str {
        match self {
            Self::Male => "male",
            Self::Female => "female",
        }
    }
}

/// Sim skin tone in Create-a-Family (`STR# 200` row 14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SkinTone {
    Light,
    Medium,
    Dark,
}

impl SkinTone {
    /// Three-letter skin tone tag used in texture stems (`"lgt"`, `"med"`, `"drk"`).
    pub const fn code(self) -> &'static str {
        match self {
            Self::Light => "lgt",
            Self::Medium => "med",
            Self::Dark => "drk",
        }
    }

    /// All three CAF skin tones in UI order.
    pub const ALL: [Self; 3] = [Self::Light, Self::Medium, Self::Dark];
}

/// Body build tag parsed from bytes `6..9` of a CAF body appearance name
/// (e.g. `b002fafit_01` → [`BodyType::Fit`]).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum BodyType {
    Fit,
    Fat,
    Skinny,
    Child,
    Other(String),
}

impl BodyType {
    /// Parses the 3-character body-type code (case-insensitively).
    pub fn from_code(code: &str) -> Self {
        if code.eq_ignore_ascii_case("fit") {
            Self::Fit
        } else if code.eq_ignore_ascii_case("fat") {
            Self::Fat
        } else if code.eq_ignore_ascii_case("skn") {
            Self::Skinny
        } else if code.eq_ignore_ascii_case("chd") {
            Self::Child
        } else {
            Self::Other(code.to_ascii_lowercase())
        }
    }

    /// The lowercase 3-character code.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Fit => "fit",
            Self::Fat => "fat",
            Self::Skinny => "skn",
            Self::Child => "chd",
            Self::Other(s) => s.as_str(),
        }
    }
}

/// One selectable CAF head or body entry produced by catalog enumeration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogEntry {
    /// Lowercase appearance name (e.g. `"b002fafit_01"` or `"c809fa_beth"`).
    pub appearance: String,
    /// Full texture override string passed to [`Outfit`] (e.g.
    /// `"BODY=b002fafitmed_mom02"` or `"HEAD-HEAD=c809famed_beth3"`).
    pub texture_binding: String,
    /// Lowercase texture stem without extension (e.g. `"b002fafitmed_mom02"`).
    pub texture_name: String,
    /// Parsed body build for body entries, or `None` for head entries.
    pub body_type: Option<BodyType>,
}

/// Selectable CAF head and body catalogs for a given `(Age, Gender, SkinTone)`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CafCatalog {
    pub heads: Vec<CatalogEntry>,
    pub bodies: Vec<CatalogEntry>,
}

/// Returns the native CAF default open hands `[(left_app, left_tex), (right_app, right_tex)]`
/// matching `0x40b050`–`0x40b298` and `User00172.iff` (`STR# 200` rows 17–18).
pub fn default_hands(age: Age, gender: Gender, skin_tone: SkinTone) -> [(String, String); 2] {
    let (left, right) = match (age, gender) {
        (Age::Child, _) => ("hulo", "huro"),
        (Age::Adult, Gender::Female) => ("hflo", "hfro"),
        (Age::Adult, Gender::Male) => ("hmlo", "hmro"),
    };
    let tex = format!("HAND=huao{}", skin_tone.code());
    [(left.to_owned(), tex.clone()), (right.to_owned(), tex)]
}

/// A complete Create-a-Family Sim configuration ready to load and render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimConfig {
    pub age: Age,
    pub gender: Gender,
    pub skin_tone: SkinTone,
    /// Head `(appearance, texture_binding)`, e.g. `("c809fa_beth", "HEAD-HEAD=c809famed_beth3")`.
    pub head: (String, String),
    /// Body `(appearance, texture_binding)`, e.g. `("b002fafit_01", "BODY=b002fafitmed_mom02")`.
    pub body: (String, String),
    /// Optional explicit left/right hands; defaults to [`default_hands`].
    pub hands: Option<[(String, String); 2]>,
}

impl SimConfig {
    /// Builds a [`SimConfig`] from enumerated [`CatalogEntry`] selections.
    pub fn from_entries(
        age: Age,
        gender: Gender,
        skin_tone: SkinTone,
        head: &CatalogEntry,
        body: &CatalogEntry,
    ) -> Self {
        Self {
            age,
            gender,
            skin_tone,
            head: (head.appearance.clone(), head.texture_binding.clone()),
            body: (body.appearance.clone(), body.texture_binding.clone()),
            hands: None,
        }
    }

    /// Builds a [`SimConfig`] from raw appearance names and texture stems
    /// (without the `HEAD-HEAD=` / `BODY=` prefixes).
    pub fn from_names(
        age: Age,
        gender: Gender,
        skin_tone: SkinTone,
        head_appearance: impl Into<String>,
        head_texture: impl AsRef<str>,
        body_appearance: impl Into<String>,
        body_texture: impl AsRef<str>,
    ) -> Self {
        Self {
            age,
            gender,
            skin_tone,
            head: (
                head_appearance.into(),
                format!("HEAD-HEAD={}", head_texture.as_ref()),
            ),
            body: (
                body_appearance.into(),
                format!("BODY={}", body_texture.as_ref()),
            ),
            hands: None,
        }
    }

    /// Resolves this configuration into an [`Outfit`] ordered identically to
    /// [`Outfit::from_body_strings`]: body, left hand, right hand, head.
    pub fn outfit(&self) -> Outfit {
        let hands = self
            .hands
            .clone()
            .unwrap_or_else(|| default_hands(self.age, self.gender, self.skin_tone));
        let mut parts = vec![
            self.body.clone(),
            hands[0].clone(),
            hands[1].clone(),
            self.head.clone(),
        ];
        parts.retain(|(name, _)| !name.is_empty());
        Outfit { parts }
    }

    /// Loads this Sim's 3D [`Body`] (skeleton, meshes, and textures) from `assets`.
    pub fn load_body(&self, assets: &Assets) -> Result<Body, String> {
        Body::load(assets, self.age.skeleton_name(), &self.outfit())
    }
}

/// Checks whether `name` starts with the CAF body prefix (`0x409663`):
/// `b` + 3 arbitrary bytes + (`gender` | `u`) + `age` + 3 arbitrary bytes.
/// Returns the 9-byte matched prefix when matched.
fn match_body_prefix(name: &str, age: Age, gender: Gender) -> Option<&str> {
    let b = name.as_bytes();
    if b.len() < 9 || !name.is_char_boundary(9) {
        return None;
    }
    if !b[0].eq_ignore_ascii_case(&b'b') {
        return None;
    }
    let g = b[4].to_ascii_lowercase();
    if g != gender.code() && g != b'u' {
        return None;
    }
    if b[5].to_ascii_lowercase() != age.code() {
        return None;
    }
    Some(&name[..9])
}

/// Checks whether `name` starts with the CAF head prefix (`0x40a364`):
/// `c` + 3 arbitrary bytes + (`gender` | `u`) + `age`, rejecting `c[tT]*`
/// (`0x40a450`–`0x40a461`). Returns the 6-byte matched prefix when matched.
fn match_head_prefix(name: &str, age: Age, gender: Gender) -> Option<&str> {
    let b = name.as_bytes();
    if b.len() < 6 || !name.is_char_boundary(6) {
        return None;
    }
    if !b[0].eq_ignore_ascii_case(&b'c') {
        return None;
    }
    // 0x40a450..0x40a461: reject NPC/townie heads whose 2nd char is 't'/'T'.
    if b[1].eq_ignore_ascii_case(&b't') {
        return None;
    }
    let g = b[4].to_ascii_lowercase();
    if g != gender.code() && g != b'u' {
        return None;
    }
    if b[5].to_ascii_lowercase() != age.code() {
        return None;
    }
    Some(&name[..6])
}

/// Whether `name` is excluded by `0x409255`'s `"suit-"` prefix filter.
fn is_suit_appearance(name: &str) -> bool {
    name.len() >= 5 && name.as_bytes()[..5].eq_ignore_ascii_case(b"suit-")
}

/// Checks whether `texture` begins with `prefix` followed immediately by
/// `tone` (case-insensitive ASCII), matching `0x4097ec` and `0x40a525`.
fn texture_matches_prefix(texture: &str, prefix: &str, tone: &str) -> bool {
    let tb = texture.as_bytes();
    let pb = prefix.as_bytes();
    let sb = tone.as_bytes();
    if tb.len() < pb.len() + sb.len() {
        return false;
    }
    tb[..pb.len()].eq_ignore_ascii_case(pb)
        && tb[pb.len()..pb.len() + sb.len()].eq_ignore_ascii_case(sb)
}

/// Enumerates all selectable CAF body outfits from a pre-built [`Wardrobe`],
/// matching `Sims.exe` `0x409663`.
pub fn enumerate_bodies(
    wardrobe: &Wardrobe,
    age: Age,
    gender: Gender,
    skin_tone: SkinTone,
) -> Vec<CatalogEntry> {
    let tone = skin_tone.code();
    let mut entries = Vec::new();
    for appearance in &wardrobe.appearances {
        if is_suit_appearance(appearance) {
            continue;
        }
        let Some(prefix) = match_body_prefix(appearance, age, gender) else {
            continue;
        };
        let body_type = BodyType::from_code(&prefix[6..9]);
        let app_lower = appearance.to_ascii_lowercase();
        for texture in &wardrobe.textures {
            if texture_matches_prefix(texture, prefix, tone) {
                let tex_lower = texture.to_ascii_lowercase();
                entries.push(CatalogEntry {
                    appearance: app_lower.clone(),
                    texture_binding: format!("BODY={tex_lower}"),
                    texture_name: tex_lower,
                    body_type: Some(body_type.clone()),
                });
            }
        }
    }
    entries
}

/// Enumerates all selectable CAF heads from a pre-built [`Wardrobe`],
/// matching `Sims.exe` `0x40a364`.
pub fn enumerate_heads(
    wardrobe: &Wardrobe,
    age: Age,
    gender: Gender,
    skin_tone: SkinTone,
) -> Vec<CatalogEntry> {
    let tone = skin_tone.code();
    let mut entries = Vec::new();
    for appearance in &wardrobe.appearances {
        if is_suit_appearance(appearance) {
            continue;
        }
        let Some(prefix) = match_head_prefix(appearance, age, gender) else {
            continue;
        };
        let app_lower = appearance.to_ascii_lowercase();
        for texture in &wardrobe.textures {
            if texture_matches_prefix(texture, prefix, tone) {
                let tex_lower = texture.to_ascii_lowercase();
                entries.push(CatalogEntry {
                    appearance: app_lower.clone(),
                    texture_binding: format!("HEAD-HEAD={tex_lower}"),
                    texture_name: tex_lower,
                    body_type: None,
                });
            }
        }
    }
    entries
}

/// Enumerates both CAF heads and CAF bodies from a pre-built [`Wardrobe`].
pub fn enumerate_catalog_from_wardrobe(
    wardrobe: &Wardrobe,
    age: Age,
    gender: Gender,
    skin_tone: SkinTone,
) -> CafCatalog {
    CafCatalog {
        heads: enumerate_heads(wardrobe, age, gender, skin_tone),
        bodies: enumerate_bodies(wardrobe, age, gender, skin_tone),
    }
}

/// Enumerates selectable CAF heads and bodies directly from an install's
/// [`Assets`] (`0x408bf5`).
pub fn enumerate_catalog(
    assets: &Assets,
    age: Age,
    gender: Gender,
    skin_tone: SkinTone,
) -> CafCatalog {
    let wardrobe = Wardrobe::of(assets);
    enumerate_catalog_from_wardrobe(&wardrobe, age, gender, skin_tone)
}

// ============================================================================
// 3D CAF Turntable Preview (cWin3DView, 0x56cbfb–0x56d039)
// ============================================================================

/// Parameters for rendering the 3D Create-a-Family dress-up turntable preview.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TurntableParams {
    /// Output image dimensions in pixels (native default: [`PREVIEW_SIZE`] `100×220`).
    pub size: [u32; 2],
    /// Turntable Y-rotation in radians (native center: [`TURNTABLE_BASE_YAW`] `π/4`,
    /// or computed from elapsed time via [`turntable_yaw`]).
    pub yaw: f32,
    /// Model scale factor (native default: [`PREVIEW_SCALE`] `5.0`).
    pub scale: f32,
    /// House camera zoom level (native CAF default: [`sims_render::Zoom::Far`],
    /// `Sims.exe` zoom 1, `tile_width = 32`).
    pub zoom: sims_render::Zoom,
    /// Distance in pixels from the bottom edge of the viewport to the Sim's
    /// root origin (native default: [`PREVIEW_FOOT_MARGIN`] `20.0`).
    pub foot_margin: f32,
    /// Direct3D 6 ambient light level (native default: [`PREVIEW_AMBIENT`] `1.0`).
    pub ambient: f32,
    /// RGBA background fill color (default transparent `[0, 0, 0, 0]`).
    pub background: [u8; 4],
}

impl Default for TurntableParams {
    fn default() -> Self {
        Self {
            size: PREVIEW_SIZE,
            yaw: TURNTABLE_BASE_YAW,
            scale: PREVIEW_SCALE,
            zoom: sims_render::Zoom::Far,
            foot_margin: PREVIEW_FOOT_MARGIN,
            ambient: PREVIEW_AMBIENT,
            background: [0, 0, 0, 0],
        }
    }
}

impl TurntableParams {
    /// Builds native CAF turntable parameters at `elapsed_ms` milliseconds
    /// into the 10-second sinusoidal sweep.
    pub fn at_time(elapsed_ms: u64) -> Self {
        Self {
            yaw: turntable_yaw(elapsed_ms),
            ..Self::default()
        }
    }

    /// Screen pixels per skeleton unit for this [`TurntableParams::scale`] and
    /// [`TurntableParams::zoom`]: `scale / (3√2 / tile_width)`.
    pub fn pixels_per_unit(&self) -> f32 {
        let tile_width = self.zoom.tile_width() as f32;
        self.scale * tile_width / (3.0 * SQRT_2)
    }
}

/// Computes the CAF turntable yaw angle in radians at `elapsed_ms` milliseconds
/// (`0x56cd1f`–`0x56cd5d`, `0x4734a4`–`0x4734e2`, `0x56cf39`–`0x56cf51`):
/// `(π/4) * sin(2π * elapsed_ms / 10000) + π/4`.
///
/// Oscillates symmetrically between `0` and `π/2` around `π/4` (facing
/// straight towards the viewer).
pub fn turntable_yaw(elapsed_ms: u64) -> f32 {
    let phase = std::f64::consts::TAU * (elapsed_ms % u64::from(TURNTABLE_PERIOD_MS)) as f64
        / f64::from(TURNTABLE_PERIOD_MS);
    TURNTABLE_AMPLITUDE * (phase.sin() as f32) + TURNTABLE_BASE_YAW
}

/// Right-handed rotation about the X axis (`0x5c3a20` axis 0).
#[inline]
fn rot_x(v: Vec3, angle: f32) -> Vec3 {
    let (s, c) = angle.sin_cos();
    Vec3::new(v.x, v.y * c - v.z * s, v.y * s + v.z * c)
}

/// Right-handed rotation about the Y axis (`0x5c3a20` axis 1).
#[inline]
fn rot_y(v: Vec3, angle: f32) -> Vec3 {
    let (s, c) = angle.sin_cos();
    Vec3::new(v.x * c + v.z * s, v.y, -v.x * s + v.z * c)
}

/// Right-handed rotation about the Z axis (`0x5c3a20` axis 2).
#[inline]
fn rot_z(v: Vec3, angle: f32) -> Vec3 {
    let (s, c) = angle.sin_cos();
    Vec3::new(v.x * c - v.y * s, v.x * s + v.y * c, v.z)
}

/// Projects a posed Sim vertex `point` (in skeleton/world units) into
/// `cWin3DView` screen pixels and view depth, matching `0x56cf13`–`0x56d039`
/// composed with the house camera at rotation `0` (`0x49b0de`):
/// `RotY(yaw + 3π/4) * RotX(-π/6)`.
///
/// `origin` is the screen anchor of the Sim's root (typically
/// `[(width / 2) as f32 + 0.5, (height as f32 - foot_margin) + 0.5]` for
/// Direct3D 6 integer pixel centres).
pub fn project_turntable(point: Vec3, origin: [f32; 2], yaw: f32, pixels_per_unit: f32) -> Vec3 {
    let v = rot_y(point, yaw + 3.0 * FRAC_PI_4);
    let v = rot_x(v, -FRAC_PI_6);
    Vec3::new(
        origin[0] + v.x * pixels_per_unit,
        origin[1] - v.y * pixels_per_unit,
        v.z,
    )
}

/// Target wrapper matching Direct3D 6's half-up 8-bit modulation rounding
/// (same rule as `PieTarget` in [`crate::pie_head`]).
struct D3D6Target<'a>(&'a mut Canvas);

impl Target for D3D6Target<'_> {
    fn size(&self) -> (u32, u32) {
        self.0.size()
    }

    fn passes(&self, x: u32, y: u32, z: f32) -> bool {
        self.0.passes(x, y, z)
    }

    fn write(&mut self, x: u32, y: u32, z: f32, texel: [u8; 4], s: f32) {
        let lit = |c: u8| (f32::from(c) * s + 0.5).clamp(0.0, 255.0) as u8;
        let [r, g, b, a] = texel;
        self.0.write(x, y, z, [lit(r), lit(g), lit(b), a], 1.0);
    }
}

/// Renders a full-body 3D CAF dress-up turntable preview of `body` into an
/// RGBA [`Canvas`] matching `cWin3DView` (`0x56cd9a`–`0x56d039`).
///
/// When `pose` is `None`, the skeleton's rest pose is used. Returns an error
/// if `params.size` exceeds [`MAX_SIDE`] on either axis, if `params` contains
/// non-finite values, or if `body` has no visible (non-censored) parts.
pub fn render_turntable(
    body: &Body,
    pose: Option<&Pose>,
    params: &TurntableParams,
) -> Result<Canvas, String> {
    let [width, height] = params.size;
    if width == 0 || height == 0 {
        return Ok(Canvas::new(width, height, params.background));
    }
    if width > MAX_SIDE || height > MAX_SIDE || width.checked_mul(height).is_none() {
        return Err(format!(
            "CAF preview size {width}x{height} exceeds {MAX_SIDE}x{MAX_SIDE}"
        ));
    }
    if !params.yaw.is_finite()
        || !params.scale.is_finite()
        || params.scale <= 0.0
        || !params.foot_margin.is_finite()
        || !params.ambient.is_finite()
    {
        return Err("CAF preview parameters must be finite with positive scale".to_owned());
    }

    let rest_pose;
    let pose = match pose {
        Some(p) => {
            if p.local.len() != body.skeleton.bones.len() {
                return Err(format!(
                    "pose has {} bones but skeleton {:?} has {}",
                    p.local.len(),
                    body.skeleton.name,
                    body.skeleton.bones.len()
                ));
            }
            p
        }
        None => {
            rest_pose = Pose::rest(&body.skeleton);
            &rest_pose
        }
    };

    let visible_parts: Vec<_> = body.parts.iter().filter(|p| p.censor == 0).collect();
    if visible_parts.is_empty() {
        return Err("body has no visible parts to draw".to_owned());
    }

    let world = pose.world(&body.skeleton, Transform::IDENTITY);
    let mut canvas = Canvas::new(width, height, params.background);
    // 0x56cf7a..0x56cfa4: anchor at (width/2, height - foot_margin), plus (+0.5, +0.5)
    // for Direct3D 6 integer pixel centres.
    let origin = [
        (width / 2) as f32 + 0.5,
        (height as f32 - params.foot_margin) + 0.5,
    ];
    let ppu = params.pixels_per_unit();
    let ambient = Shade::Grey(params.ambient.clamp(0.0, 1.0));
    let mut target = D3D6Target(&mut canvas);

    for part in visible_parts {
        let skinned = skin(part, &world);
        let shade = vec![ambient; skinned.positions.len()];
        let proj = |p| project_turntable(p, origin, params.yaw, ppu);
        if part.texture_name.is_some() {
            let mut packed = Packed565(&mut target);
            draw_shaded_culled(&mut packed, part, &skinned, &shade, proj);
        } else {
            draw_shaded_culled(&mut target, part, &skinned, &shade, proj);
        }
    }

    Ok(canvas)
}

/// Convenience helper that poses `body` from an optional animation `clip` at
/// `phase` (`0.0..=1.0`) and renders the CAF turntable preview.
pub fn render_turntable_clip(
    body: &Body,
    clip: Option<&Clip>,
    phase: f32,
    repeat: bool,
    params: &TurntableParams,
) -> Result<Canvas, String> {
    let pose = clip.map(|c| c.pose(&body.skeleton, phase, repeat));
    render_turntable(body, pose.as_ref(), params)
}

// ============================================================================
// 3D Sim Portrait Renderer (0x4e77c6 & BMP_ 2002..2007)
// ============================================================================

/// Packs an 8-bit RGB triple into 16-bit 5-6-5 (`r >> 3`, `g >> 2`, `b >> 3`),
/// matching `Sims.exe` `0x49f0ae`.
#[inline]
pub const fn pack_rgb565(rgb: [u8; 3]) -> u16 {
    (((rgb[0] >> 3) as u16) << 11) | (((rgb[1] >> 2) as u16) << 5) | ((rgb[2] >> 3) as u16)
}

/// Unpacks a 16-bit 5-6-5 colour to 8-bit RGB by zero-filling the low bits
/// (`r5 << 3`, `g6 << 2`, `b5 << 3`), matching `Sims.exe`'s 5-6-5 → 24-bit
/// `BMP_` export in `User00172.iff` (`0x000a` → `[0, 0, 80]`).
#[inline]
pub const fn unpack_rgb565(pixel: u16) -> [u8; 3] {
    [
        (((pixel >> 11) & 0x1f) << 3) as u8,
        (((pixel >> 5) & 0x3f) << 2) as u8,
        ((pixel & 0x1f) << 3) as u8,
    ]
}

/// Unpacks a 16-bit 5-6-5 colour with MSB bit-replication (matching Direct3D 6
/// texture expansion in [`crate::lot::Packed565`]).
#[inline]
const fn unpack_rgb565_msb(pixel: u16) -> [u8; 3] {
    let r5 = ((pixel >> 11) & 0x1f) as u8;
    let g6 = ((pixel >> 5) & 0x3f) as u8;
    let b5 = (pixel & 0x1f) as u8;
    [
        (r5 << 3) | (r5 >> 2),
        (g6 << 2) | (g6 >> 4),
        (b5 << 3) | (b5 >> 2),
    ]
}

/// How the `200×200` supersampled portrait buffer is cropped prior to
/// aspect-ratio fitting and Hermite downsampling (`0x4e7cba`–`0x4e7e1c`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortraitCrop {
    /// Fixed `[left, top, right, bottom]` rectangle inside `200×200`
    /// (native default: [`PORTRAIT_FIXED_CROP`] `[76, 45, 122, 125]`).
    Fixed([u32; 4]),
    /// Scans the tight bounding box of drawn head pixels (`0x4c357c`) and
    /// expands outward by `[left, top, right, bottom]` pad pixels.
    Auto { pad: [u32; 4] },
}

/// Parameters for rendering a single 3D Sim head portrait frame (`0x4e77c6`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PortraitFrameSpec {
    /// Output frame dimensions `[width, height]` in pixels.
    pub size: [u32; 2],
    /// Pitch angle in radians added to [`BASE_PITCH`] (`0.05π`).
    pub pitch: f32,
    /// Yaw angle in radians added to `π`.
    pub yaw: f32,
    /// Crop strategy applied to the `200×200` supersampled render.
    pub crop: PortraitCrop,
    /// Solid 8-bit RGB background (opaque output) or `None` for transparent
    /// alpha `[0, 0, 0, 0]` outside the head.
    pub background: Option<[u8; 3]>,
    /// When `true` and `background` is `Some(_)`, quantizes the intermediate
    /// buffer and Hermite output through 16-bit 5-6-5 exactly like native
    /// `Sims.exe` `BMP_` chunks (`0x4ba79c`).
    pub quantize_565: bool,
}

impl Default for PortraitFrameSpec {
    fn default() -> Self {
        Self {
            size: FACES_2002_FRAME_SIZE,
            pitch: 0.0,
            yaw: 0.0,
            crop: PortraitCrop::Fixed(PORTRAIT_FIXED_CROP),
            background: Some(unpack_rgb565(FACES_2002_BG_565)),
            quantize_565: true,
        }
    }
}

/// Standard `Sims.exe` `BMP_` portrait presets saved into `UserXXXXX.iff`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PortraitKind {
    /// `BMP_` 2002 `'faces'`: 3× `35×41` (`105×41`) neutral / happy / sad
    /// strip on dark navy `0x000a` (`0x4e725f`).
    Faces2002,
    /// `BMP_` 2003 `'rel. images'`: 5× `40×40` (`200×40`) relationship-mood
    /// strip at pitch `-5°`, yaw `+18°` (`0x4e7448`).
    Relationship2003,
    /// `BMP_` 2004 `'newimage'`: `25×25` thumbnail with `23×23` head inset by
    /// 1 px on solid blue (`0x4e7671`).
    NewImage2004,
    /// `BMP_` 2005 `'speech_med'`: `16×16` auto-cropped head (`0x4e81f6`).
    SpeechMed2005,
    /// `BMP_` 2006 `'speech_large'`: `32×32` auto-cropped head (`0x4e81f6`).
    SpeechLarge2006,
    /// `BMP_` 2007 `'web_image'`: `45×45` framed thumbnail with `35×41` head
    /// composited at `(5, 2)` (`0x4e8400`).
    WebImage2007,
}

/// Projects a HEAD-bone-local vertex `point` into the `200×200` supersampled
/// portrait buffer (`0x4e7923`–`0x4e7ac6`), matching:
/// `RotY(0.05π + pitch) * RotZ(π/2) * RotY(yaw + π) * RotX(-π/6)`
/// scaled by [`PORTRAIT_SCALE_PIXELS`] (`80√2`).
pub fn project_portrait(point: Vec3, origin: [f32; 2], pitch: f32, yaw: f32) -> Vec3 {
    let v = rot_y(point, BASE_PITCH + pitch);
    let v = rot_z(v, FRAC_PI_2);
    let v = rot_y(v, yaw + PI);
    let v = rot_x(v, -FRAC_PI_6);
    Vec3::new(
        origin[0] + v.x * PORTRAIT_SCALE_PIXELS,
        origin[1] - v.y * PORTRAIT_SCALE_PIXELS,
        v.z,
    )
}

/// Expands `crop` (`[left, top, right, bottom]`) symmetrically inside
/// `src_size` to match `dst_size`'s aspect ratio, matching `0x4e7d4e`–`0x4e7e1c`.
pub fn fit_crop_aspect(crop: [u32; 4], src_size: [u32; 2], dst_size: [u32; 2]) -> [u32; 4] {
    let [src_w, src_h] = src_size;
    let [dst_w, dst_h] = dst_size;
    let mut left = crop[0].min(src_w);
    let mut top = crop[1].min(src_h);
    let mut right = crop[2].clamp(left, src_w);
    let mut bottom = crop[3].clamp(top, src_h);
    let crop_w = right - left;
    let crop_h = bottom - top;
    if crop_w == 0 || crop_h == 0 || dst_w == 0 || dst_h == 0 {
        return [left, top, right, bottom];
    }

    let lhs = u64::from(crop_w) * u64::from(dst_h);
    let rhs = u64::from(dst_w) * u64::from(crop_h);
    if lhs < rhs {
        // Narrower than target aspect: widen horizontally.
        let target_w = (((f64::from(crop_h) * f64::from(dst_w)) / f64::from(dst_h)).round() as u32)
            .clamp(crop_w, src_w);
        let need = target_w - crop_w;
        let half = need / 2;
        let take_left = half.min(left);
        left -= take_left;
        let rem_after_left = need - take_left;
        let take_right = rem_after_left.min(src_w - right);
        right += take_right;
        let rem = rem_after_left - take_right;
        if rem > 0 {
            left = left.saturating_sub(rem);
        }
    } else if lhs > rhs {
        // Wider than target aspect: heighten vertically.
        let target_h = (((f64::from(crop_w) * f64::from(dst_h)) / f64::from(dst_w)).round() as u32)
            .clamp(crop_h, src_h);
        let need = target_h - crop_h;
        let half = need / 2;
        let take_top = half.min(top);
        top -= take_top;
        let rem_after_top = need - take_top;
        let take_bottom = rem_after_top.min(src_h - bottom);
        bottom += take_bottom;
        let rem = rem_after_top - take_bottom;
        if rem > 0 {
            top = top.saturating_sub(rem);
        }
    }
    [left, top, right, bottom]
}

/// Cubic Hermite reconstruction kernel (`0x4babb5`–`0x4babda`):
/// `f(t) = (2|t| - 3)t² + 1` for `|t| < 1`, `0` otherwise.
#[inline]
fn hermite_kernel(t: f64) -> f64 {
    let a = t.abs();
    if a < 1.0 {
        (2.0 * a - 3.0) * a * a + 1.0
    } else {
        0.0
    }
}

/// Mirror boundary indexing (`0x4babeb`–`0x4bac00`): reflects indices outside
/// `0..len` back into `0..len`.
#[inline]
fn mirror_index(mut idx: i32, len: i32) -> usize {
    if len <= 1 {
        return 0;
    }
    while idx < 0 || idx >= len {
        if idx < 0 {
            idx = -idx;
        }
        if idx >= len {
            idx = 2 * len - idx - 2;
        }
    }
    idx as usize
}

/// Precomputed 1D Hermite contribution table for resampling `src_len` → `dst_len`
/// (`0x4ba79c`–`0x4bac5a`).
fn build_hermite_taps(src_len: u32, dst_len: u32) -> Vec<Vec<(usize, f64)>> {
    let scale = f64::from(dst_len) / f64::from(src_len);
    let (width, fscale) = if scale < 1.0 {
        (1.0 / scale, scale)
    } else {
        (1.0, 1.0)
    };
    let mut table = Vec::with_capacity(dst_len as usize);
    for i in 0..dst_len {
        let center = f64::from(i) / scale;
        // 0x4bab33 (ceil(center - width)), 0x4bab65 (floor(center + width))
        let left = (center - width).ceil() as i32;
        let right = (center + width).floor() as i32;
        let mut taps = Vec::with_capacity((right - left + 1).max(0) as usize);
        let mut sum = 0.0;
        for j in left..=right {
            let w = hermite_kernel((center - f64::from(j)) * fscale) * fscale;
            sum += w;
            let src_idx = mirror_index(j, src_len as i32);
            taps.push((src_idx, w));
        }
        if sum > 0.0 {
            let inv = 1.0 / sum;
            for (_, w) in &mut taps {
                *w *= inv;
            }
        }
        table.push(taps);
    }
    table
}

/// Resamples the cropped rectangle `crop` (`[left, top, right, bottom]`) from
/// `src` into a `dst_size` RGBA [`Canvas`] using `Sims.exe`'s two-pass
/// separable cubic Hermite filter (`0x4ba79c`–`0x4bb4a0`).
///
/// When `quantize_565` is `true`, channels are filtered in 5-6-5 integer space
/// and unpacked via [`unpack_rgb565`] to match `User00172.iff` `BMP_` files.
/// When `premultiplied_alpha` is `true`, filtering runs in premultiplied RGBA
/// so transparent backgrounds do not bleed colour fringes into the silhouette.
pub fn hermite_resample(
    src: &Canvas,
    crop: [u32; 4],
    dst_size: [u32; 2],
    quantize_565: bool,
    premultiplied_alpha: bool,
) -> Canvas {
    let [left, top, right, bottom] = crop;
    let [dst_w, dst_h] = dst_size;
    let crop_w = right.saturating_sub(left);
    let crop_h = bottom.saturating_sub(top);
    if crop_w == 0 || crop_h == 0 || dst_w == 0 || dst_h == 0 {
        return Canvas::new(dst_w, dst_h, [0; 4]);
    }

    let x_taps = build_hermite_taps(crop_w, dst_w);
    let y_taps = build_hermite_taps(crop_h, dst_h);

    // Horizontal pass: crop_h rows × dst_w columns.
    let mut temp = vec![[0.0f64; 4]; (crop_h * dst_w) as usize];
    for sy in 0..crop_h {
        let src_row = ((top + sy) * src.width) as usize;
        let tmp_row = (sy * dst_w) as usize;
        for (dx, taps) in x_taps.iter().enumerate() {
            let mut acc = [0.0f64; 4];
            for &(sx, w) in taps {
                let p = (src_row + (left as usize) + sx) * 4;
                if quantize_565 && !premultiplied_alpha {
                    // 0x4bad73 unpacks 5-6-5 texels to 8-bit (r5<<3, g6<<2, b5<<3).
                    let rgb =
                        unpack_rgb565(pack_rgb565([src.rgba[p], src.rgba[p + 1], src.rgba[p + 2]]));
                    acc[0] += f64::from(rgb[0]) * w;
                    acc[1] += f64::from(rgb[1]) * w;
                    acc[2] += f64::from(rgb[2]) * w;
                    acc[3] += 255.0 * w;
                } else if premultiplied_alpha {
                    let a = f64::from(src.rgba[p + 3]) / 255.0;
                    acc[0] += f64::from(src.rgba[p]) * a * w;
                    acc[1] += f64::from(src.rgba[p + 1]) * a * w;
                    acc[2] += f64::from(src.rgba[p + 2]) * a * w;
                    acc[3] += f64::from(src.rgba[p + 3]) * w;
                } else {
                    acc[0] += f64::from(src.rgba[p]) * w;
                    acc[1] += f64::from(src.rgba[p + 1]) * w;
                    acc[2] += f64::from(src.rgba[p + 2]) * w;
                    acc[3] += f64::from(src.rgba[p + 3]) * w;
                }
            }
            if quantize_565 && !premultiplied_alpha {
                // 0x4bae50..0x4bae80: round to 8-bit (+0.5, ftol), pack 5-6-5, then
                // unpack back to 8-bit (r5<<3, g6<<2, b5<<3) for the vertical pass.
                let r8 = (acc[0] + 0.5).floor().clamp(0.0, 255.0) as u8;
                let g8 = (acc[1] + 0.5).floor().clamp(0.0, 255.0) as u8;
                let b8 = (acc[2] + 0.5).floor().clamp(0.0, 255.0) as u8;
                let rgb = unpack_rgb565(pack_rgb565([r8, g8, b8]));
                acc[0] = f64::from(rgb[0]);
                acc[1] = f64::from(rgb[1]);
                acc[2] = f64::from(rgb[2]);
            }
            temp[tmp_row + dx] = acc;
        }
    }

    // Vertical pass: dst_h rows × dst_w columns.
    let mut out = Canvas::new(dst_w, dst_h, [0; 4]);
    for (dy, taps) in y_taps.iter().enumerate() {
        let out_row = dy * (dst_w as usize);
        for dx in 0..(dst_w as usize) {
            let mut acc = [0.0f64; 4];
            for &(sy, w) in taps {
                let px = temp[sy * (dst_w as usize) + dx];
                acc[0] += px[0] * w;
                acc[1] += px[1] * w;
                acc[2] += px[2] * w;
                acc[3] += px[3] * w;
            }
            let p = (out_row + dx) * 4;
            if quantize_565 && !premultiplied_alpha {
                let r8 = (acc[0] + 0.5).floor().clamp(0.0, 255.0) as u8;
                let g8 = (acc[1] + 0.5).floor().clamp(0.0, 255.0) as u8;
                let b8 = (acc[2] + 0.5).floor().clamp(0.0, 255.0) as u8;
                let rgb = unpack_rgb565(pack_rgb565([r8, g8, b8]));
                out.rgba[p] = rgb[0];
                out.rgba[p + 1] = rgb[1];
                out.rgba[p + 2] = rgb[2];
                out.rgba[p + 3] = 255;
            } else if premultiplied_alpha {
                let a = (acc[3] + 0.5).floor().clamp(0.0, 255.0) as u8;
                if a == 0 {
                    out.rgba[p..p + 4].copy_from_slice(&[0, 0, 0, 0]);
                } else {
                    let a_norm = acc[3] / 255.0;
                    out.rgba[p] = (acc[0] / a_norm + 0.5).floor().clamp(0.0, 255.0) as u8;
                    out.rgba[p + 1] = (acc[1] / a_norm + 0.5).floor().clamp(0.0, 255.0) as u8;
                    out.rgba[p + 2] = (acc[2] / a_norm + 0.5).floor().clamp(0.0, 255.0) as u8;
                    out.rgba[p + 3] = a;
                }
            } else {
                out.rgba[p] = (acc[0] + 0.5).floor().clamp(0.0, 255.0) as u8;
                out.rgba[p + 1] = (acc[1] + 0.5).floor().clamp(0.0, 255.0) as u8;
                out.rgba[p + 2] = (acc[2] + 0.5).floor().clamp(0.0, 255.0) as u8;
                out.rgba[p + 3] = (acc[3] + 0.5).floor().clamp(0.0, 255.0) as u8;
            }
        }
    }

    out
}

/// Scans the tight bounding box `[left, top, right, bottom]` of covered
/// (non-background) pixels in `canvas` (`0x4c357c`), expanding outward by
/// `pad = [left, top, right, bottom]` clamped to `canvas`'s bounds.
fn auto_crop_bounds(canvas: &Canvas, bg: Option<[u8; 3]>, pad: [u32; 4]) -> [u32; 4] {
    let mut min_x = canvas.width;
    let mut min_y = canvas.height;
    let mut max_x = 0u32;
    let mut max_y = 0u32;
    let mut found = false;

    for y in 0..canvas.height {
        for x in 0..canvas.width {
            let p = ((y * canvas.width + x) * 4) as usize;
            let hit = match bg {
                None => canvas.rgba[p + 3] > 0,
                Some(bg_rgb) => {
                    canvas.rgba[p] != bg_rgb[0]
                        || canvas.rgba[p + 1] != bg_rgb[1]
                        || canvas.rgba[p + 2] != bg_rgb[2]
                }
            };
            if hit {
                found = true;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x + 1);
                max_y = max_y.max(y + 1);
            }
        }
    }

    if !found {
        return [0, 0, canvas.width, canvas.height];
    }

    [
        min_x.saturating_sub(pad[0]),
        min_y.saturating_sub(pad[1]),
        max_x.saturating_add(pad[2]).min(canvas.width),
        max_y.saturating_add(pad[3]).min(canvas.height),
    ]
}

fn skinned_head_parts(
    body: &Body,
) -> Result<Vec<(&crate::appearance::Part, crate::skin::Skinned)>, String> {
    let head_bone = rig::bone(&body.skeleton, HEAD_BONE)
        .ok_or_else(|| "skeleton has no HEAD bone".to_owned())?;
    let identity = vec![Transform::IDENTITY; body.skeleton.bones.len()];
    let parts: Vec<_> = body
        .parts
        .iter()
        .filter(|p| p.censor == 0 && p.bone == head_bone)
        .map(|p| (p, skin(p, &identity)))
        .collect();
    let count: usize = parts.iter().map(|(_, s)| s.positions.len()).sum();
    if count == 0 {
        return Err("body has no head to draw".to_owned());
    }
    Ok(parts)
}

/// Renders one 3D Sim portrait frame using `Sims.exe`'s supersampled `200×200`
/// render + aspect-fit crop + separable cubic Hermite downsample pipeline
/// (`0x4e77c6`).
pub fn render_portrait_frame(body: &Body, spec: &PortraitFrameSpec) -> Result<Canvas, String> {
    let [width, height] = spec.size;
    if width == 0 || height == 0 {
        let bg = spec
            .background
            .map_or([0, 0, 0, 0], |[r, g, b]| [r, g, b, 255]);
        return Ok(Canvas::new(width, height, bg));
    }
    if width > MAX_SIDE || height > MAX_SIDE || width.checked_mul(height).is_none() {
        return Err(format!(
            "portrait size {width}x{height} exceeds {MAX_SIDE}x{MAX_SIDE}"
        ));
    }
    if !spec.pitch.is_finite() || !spec.yaw.is_finite() {
        return Err("portrait pitch and yaw must be finite".to_owned());
    }

    let parts = skinned_head_parts(body)?;
    let ss = PORTRAIT_SUPERSAMPLE_SIZE;
    let quant_bg = spec.background.map(|rgb| {
        if spec.quantize_565 {
            unpack_rgb565_msb(pack_rgb565(rgb))
        } else {
            rgb
        }
    });
    let clear = quant_bg.map_or([0, 0, 0, 0], |[r, g, b]| [r, g, b, 255]);
    let mut hi = Canvas::new(ss, ss, clear);
    // 0x4e7953..0x4e7982: center (100, 100) + (+0.5, +0.5) for D3D6 integer pixel centres.
    let origin = [(ss / 2) as f32 + 0.5, (ss / 2) as f32 + 0.5];
    let ambient = Shade::Grey(1.0);
    let mut target = D3D6Target(&mut hi);

    for (part, skinned) in &parts {
        let shade = vec![ambient; skinned.positions.len()];
        let proj = |p| project_portrait(p, origin, spec.pitch, spec.yaw);
        if part.texture_name.is_some() {
            let mut packed = Packed565(&mut target);
            draw_shaded_culled(&mut packed, part, skinned, &shade, proj);
        } else {
            draw_shaded_culled(&mut target, part, skinned, &shade, proj);
        }
    }

    let raw_crop = match spec.crop {
        PortraitCrop::Fixed(rect) => rect,
        PortraitCrop::Auto { pad } => auto_crop_bounds(&hi, quant_bg, pad),
    };
    let crop = fit_crop_aspect(raw_crop, [ss, ss], [width, height]);
    let premul = spec.background.is_none();
    Ok(hermite_resample(
        &hi,
        crop,
        [width, height],
        spec.quantize_565 && !premul,
        premul,
    ))
}

/// Blits `src` onto `dst` at `(dst_x, dst_y)`, clipping to `dst`'s bounds.
fn blit_canvas(dst: &mut Canvas, src: &Canvas, dst_x: u32, dst_y: u32) {
    for sy in 0..src.height {
        let ty = dst_y + sy;
        if ty >= dst.height {
            break;
        }
        for sx in 0..src.width {
            let tx = dst_x + sx;
            if tx >= dst.width {
                break;
            }
            let sp = ((sy * src.width + sx) * 4) as usize;
            let dp = ((ty * dst.width + tx) * 4) as usize;
            dst.rgba[dp..dp + 4].copy_from_slice(&src.rgba[sp..sp + 4]);
        }
    }
}

/// Renders a complete native `Sims.exe` `BMP_` portrait resource (`2002`–`2007`)
/// for `body`.
pub fn render_portrait(body: &Body, kind: PortraitKind) -> Result<Canvas, String> {
    match kind {
        PortraitKind::Faces2002 => {
            let [fw, fh] = FACES_2002_FRAME_SIZE;
            let bg = unpack_rgb565(FACES_2002_BG_565);
            let mut strip = Canvas::new(fw * 3, fh, [bg[0], bg[1], bg[2], 255]);
            for (i, &(pitch, yaw)) in FACES_2002_ANGLES.iter().enumerate() {
                let frame = render_portrait_frame(
                    body,
                    &PortraitFrameSpec {
                        size: [fw, fh],
                        pitch,
                        yaw,
                        crop: PortraitCrop::Fixed(PORTRAIT_FIXED_CROP),
                        background: Some(bg),
                        quantize_565: true,
                    },
                )?;
                blit_canvas(&mut strip, &frame, (i as u32) * fw, 0);
            }
            Ok(strip)
        }
        PortraitKind::Relationship2003 => {
            let [fw, fh] = REL_2003_FRAME_SIZE;
            let mut strip = Canvas::new(fw * 5, fh, [0, 0, 0, 255]);
            for (i, &bg565) in REL_2003_BG_565.iter().enumerate() {
                let bg = unpack_rgb565(bg565);
                let frame = render_portrait_frame(
                    body,
                    &PortraitFrameSpec {
                        size: [fw, fh],
                        pitch: REL_2003_PITCH,
                        yaw: REL_2003_YAW,
                        crop: PortraitCrop::Fixed(PORTRAIT_FIXED_CROP),
                        background: Some(bg),
                        quantize_565: true,
                    },
                )?;
                blit_canvas(&mut strip, &frame, (i as u32) * fw, 0);
            }
            Ok(strip)
        }
        PortraitKind::NewImage2004 => {
            let border = unpack_rgb565(NEW_IMAGE_2004_BG_565);
            let mut out = Canvas::new(25, 25, [border[0], border[1], border[2], 255]);
            let inner = render_portrait_frame(
                body,
                &PortraitFrameSpec {
                    size: [23, 23],
                    pitch: 0.0,
                    yaw: 0.0,
                    crop: PortraitCrop::Fixed(PORTRAIT_FIXED_CROP),
                    background: Some([0, 0, 0]),
                    quantize_565: true,
                },
            )?;
            blit_canvas(&mut out, &inner, 1, 1);
            Ok(out)
        }
        PortraitKind::SpeechMed2005 => {
            let bg = unpack_rgb565(SPEECH_BG_565);
            render_portrait_frame(
                body,
                &PortraitFrameSpec {
                    size: [16, 16],
                    pitch: REL_2003_PITCH,
                    yaw: REL_2003_YAW,
                    crop: PortraitCrop::Auto { pad: [0; 4] },
                    background: Some(bg),
                    quantize_565: true,
                },
            )
        }
        PortraitKind::SpeechLarge2006 => {
            let bg = unpack_rgb565(SPEECH_BG_565);
            render_portrait_frame(
                body,
                &PortraitFrameSpec {
                    size: [32, 32],
                    pitch: REL_2003_PITCH,
                    yaw: REL_2003_YAW,
                    crop: PortraitCrop::Auto { pad: [0; 4] },
                    background: Some(bg),
                    quantize_565: true,
                },
            )
        }
        PortraitKind::WebImage2007 => {
            let outer_bg = unpack_rgb565(WEB_IMAGE_2007_BG_565);
            let mut out = Canvas::new(45, 45, [outer_bg[0], outer_bg[1], outer_bg[2], 255]);
            let inner = render_portrait_frame(
                body,
                &PortraitFrameSpec {
                    size: FACES_2002_FRAME_SIZE,
                    pitch: 0.0,
                    yaw: 0.0,
                    crop: PortraitCrop::Fixed(PORTRAIT_FIXED_CROP),
                    background: Some(outer_bg),
                    quantize_565: true,
                },
            )?;
            blit_canvas(&mut out, &inner, 5, 2);
            Ok(out)
        }
    }
}

/// Renders a single 3D Sim portrait frame with a transparent alpha background
/// (`[0, 0, 0, 0]` outside the head) using the `BMP_` 2002 fixed crop framing.
pub fn render_portrait_transparent(
    body: &Body,
    size: [u32; 2],
    pitch: f32,
    yaw: f32,
) -> Result<Canvas, String> {
    render_portrait_frame(
        body,
        &PortraitFrameSpec {
            size,
            pitch,
            yaw,
            crop: PortraitCrop::Fixed(PORTRAIT_FIXED_CROP),
            background: None,
            quantize_565: false,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::appearance::Part;
    use crate::texture::Texture;
    use sims_formats::avatar::{Bone, Mesh, MeshBinding, Skeleton, Vertex, Wiggle};

    fn sample_wardrobe() -> Wardrobe {
        Wardrobe {
            appearances: vec![
                "b002fafit_01".into(),
                "b004mafit_01".into(),
                "b009fcchd_01".into(),
                "b010uafit_01".into(),
                "suit-b002fafit_01".into(),
                "c001fa_classic".into(),
                "c809fa_beth".into(),
                "ct01fa_townie".into(),
                "cT99ma_npc".into(),
                "c006fc_nerd".into(),
                "c010ua_unisex".into(),
                "suit-c001fa_classic".into(),
            ],
            textures: vec![
                "b002fafitmed_mom02".into(),
                "b002fafitlgt_mom01".into(),
                "b004mafitlgt_rat".into(),
                "b009fcchdmed_nerd".into(),
                "b010uafitmed_uni".into(),
                "c001famed_01".into(),
                "c809famed_beth3".into(),
                "ct01famed_townie".into(),
                "c006fcmed_nerd".into(),
                "c010uamed_uni".into(),
            ],
        }
    }

    fn synthetic_body() -> Body {
        let skeleton = Skeleton {
            name: "adult".into(),
            bones: vec![
                Bone {
                    name: "ROOT".into(),
                    parent: None,
                    translation: [0.0, 0.0, 0.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    properties: Vec::new(),
                    flags: [0; 3],
                    wiggle: Wiggle::default(),
                },
                Bone {
                    name: "HEAD".into(),
                    parent: Some(0),
                    translation: [0.0, 4.0, 0.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    properties: Vec::new(),
                    flags: [0; 3],
                    wiggle: Wiggle::default(),
                },
            ],
        };
        let body_quad = Mesh {
            name: "test-PELVIS-BODY".into(),
            texture: String::new(),
            bones: vec!["ROOT".into()],
            bindings: vec![MeshBinding {
                bone: 0,
                first_vertex: 0,
                vertex_count: 4,
                first_blend: 0,
                blend_count: 0,
            }],
            triangles: vec![[0, 1, 2], [0, 2, 3], [0, 2, 1], [0, 3, 2]],
            vertices: vec![
                Vertex {
                    position: [-0.5, 0.0, 0.0],
                    normal: [0.0, 0.0, 1.0],
                    uv: [0.0, 0.0],
                    bone: 0,
                },
                Vertex {
                    position: [0.5, 0.0, 0.0],
                    normal: [0.0, 0.0, 1.0],
                    uv: [1.0, 0.0],
                    bone: 0,
                },
                Vertex {
                    position: [0.5, 4.0, 0.0],
                    normal: [0.0, 0.0, 1.0],
                    uv: [1.0, 1.0],
                    bone: 0,
                },
                Vertex {
                    position: [-0.5, 4.0, 0.0],
                    normal: [0.0, 0.0, 1.0],
                    uv: [0.0, 1.0],
                    bone: 0,
                },
            ],
            blends: Vec::new(),
        };
        // In HEAD-bone local space: +X is up (crown), +Y is ear, +Z is forward (nose).
        let head_quad = Mesh {
            name: "test-HEAD-HEAD".into(),
            texture: String::new(),
            bones: vec!["HEAD".into()],
            bindings: vec![MeshBinding {
                bone: 0,
                first_vertex: 0,
                vertex_count: 4,
                first_blend: 0,
                blend_count: 0,
            }],
            triangles: vec![[0, 1, 2], [0, 2, 3], [0, 2, 1], [0, 3, 2]],
            vertices: vec![
                Vertex {
                    position: [-0.2, -0.25, 0.25],
                    normal: [0.0, 0.0, 1.0],
                    uv: [0.0, 0.0],
                    bone: 0,
                },
                Vertex {
                    position: [0.5, -0.25, 0.25],
                    normal: [0.0, 0.0, 1.0],
                    uv: [1.0, 0.0],
                    bone: 0,
                },
                Vertex {
                    position: [0.5, 0.25, 0.25],
                    normal: [0.0, 0.0, 1.0],
                    uv: [1.0, 1.0],
                    bone: 0,
                },
                Vertex {
                    position: [-0.2, 0.25, 0.25],
                    normal: [0.0, 0.0, 1.0],
                    uv: [0.0, 1.0],
                    bone: 0,
                },
            ],
            blends: Vec::new(),
        };
        Body {
            skeleton,
            parts: vec![
                Part {
                    appearance: "test-body".into(),
                    bone: 0,
                    bones: vec![0],
                    mesh: body_quad,
                    texture_name: Some("body".into()),
                    texture: Texture::solid([180, 120, 90, 255]),
                    censor: 0,
                },
                Part {
                    appearance: "test-head".into(),
                    bone: 1,
                    bones: vec![1],
                    mesh: head_quad,
                    texture_name: Some("head".into()),
                    texture: Texture::solid([220, 180, 140, 255]),
                    censor: 0,
                },
            ],
        }
    }

    #[test]
    fn catalog_enumerator_filters_age_gender_tone_suit_and_townie_heads() {
        let wardrobe = sample_wardrobe();
        let cat = enumerate_catalog_from_wardrobe(
            &wardrobe,
            Age::Adult,
            Gender::Female,
            SkinTone::Medium,
        );
        let head_names: Vec<&str> = cat.heads.iter().map(|e| e.appearance.as_str()).collect();
        let body_names: Vec<&str> = cat.bodies.iter().map(|e| e.appearance.as_str()).collect();
        // Matches female and unisex adult heads, excluding ct* and suit-*.
        assert_eq!(
            head_names,
            ["c001fa_classic", "c809fa_beth", "c010ua_unisex"]
        );
        assert_eq!(cat.heads[1].texture_binding, "HEAD-HEAD=c809famed_beth3");
        // Matches female and unisex adult bodies, excluding child/male/light/suit-*.
        assert_eq!(body_names, ["b002fafit_01", "b010uafit_01"]);
        assert_eq!(cat.bodies[0].body_type, Some(BodyType::Fit));
        assert_eq!(cat.bodies[0].texture_binding, "BODY=b002fafitmed_mom02");

        let child = enumerate_catalog_from_wardrobe(
            &wardrobe,
            Age::Child,
            Gender::Female,
            SkinTone::Medium,
        );
        assert_eq!(child.heads.len(), 1);
        assert_eq!(child.heads[0].appearance, "c006fc_nerd");
        assert_eq!(child.bodies.len(), 1);
        assert_eq!(child.bodies[0].body_type, Some(BodyType::Child));
    }

    #[test]
    fn turntable_yaw_oscillates_between_zero_and_half_pi() {
        assert!((turntable_yaw(0) - FRAC_PI_4).abs() < 1e-6);
        assert!((turntable_yaw(2500) - FRAC_PI_2).abs() < 1e-6);
        assert!((turntable_yaw(5000) - FRAC_PI_4).abs() < 1e-6);
        assert!(turntable_yaw(7500).abs() < 1e-6);
        assert!((turntable_yaw(10_000) - FRAC_PI_4).abs() < 1e-6);
    }

    #[test]
    fn fit_crop_aspect_matches_native_bmp_2002_and_2003_boxes() {
        assert_eq!(
            fit_crop_aspect(PORTRAIT_FIXED_CROP, [200, 200], [35, 41]),
            [65, 45, 133, 125]
        );
        assert_eq!(
            fit_crop_aspect(PORTRAIT_FIXED_CROP, [200, 200], [40, 40]),
            [59, 45, 139, 125]
        );
    }

    #[test]
    fn synthetic_turntable_and_portraits_render_and_validate_bounds() {
        let body = synthetic_body();
        let turntable = render_turntable(&body, None, &TurntableParams::default()).unwrap();
        assert_eq!((turntable.width, turntable.height), (100, 220));
        assert!(
            turntable.covered() > 200,
            "turntable should draw body pixels"
        );

        let faces = render_portrait(&body, PortraitKind::Faces2002).unwrap();
        assert_eq!((faces.width, faces.height), (105, 41));
        // Top-left corner is background navy (0, 0, 80, 255).
        assert_eq!(&faces.rgba[0..4], &[0, 0, 80, 255]);

        let rel = render_portrait(&body, PortraitKind::Relationship2003).unwrap();
        assert_eq!((rel.width, rel.height), (200, 40));

        let transparent = render_portrait_transparent(&body, [35, 41], 0.0, 0.0).unwrap();
        assert_eq!(&transparent.rgba[0..4], &[0, 0, 0, 0]);
        assert!(
            transparent
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .any(|px| px[3] > 200),
            "transparent portrait should have opaque head pixels"
        );

        // Guardrails: oversized canvases & non-finite angles return Err.
        let bad = TurntableParams {
            size: [MAX_SIDE + 1, 100],
            ..TurntableParams::default()
        };
        assert!(render_turntable(&body, None, &bad).is_err());

        assert!(render_portrait_transparent(&body, [35, 41], f32::NAN, 0.0).is_err());
    }

    #[test]
    fn default_hands_match_native_age_gender_and_tone() {
        assert_eq!(
            default_hands(Age::Adult, Gender::Female, SkinTone::Medium),
            [
                ("hflo".into(), "HAND=huaomed".into()),
                ("hfro".into(), "HAND=huaomed".into()),
            ]
        );
        assert_eq!(
            default_hands(Age::Adult, Gender::Male, SkinTone::Light),
            [
                ("hmlo".into(), "HAND=huaolgt".into()),
                ("hmro".into(), "HAND=huaolgt".into()),
            ]
        );
        assert_eq!(
            default_hands(Age::Child, Gender::Female, SkinTone::Dark),
            [
                ("hulo".into(), "HAND=huaodrk".into()),
                ("huro".into(), "HAND=huaodrk".into()),
            ]
        );
    }
}
