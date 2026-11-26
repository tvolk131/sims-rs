//! The original artwork the interface draws with, loaded once.

use crate::font::Font;
use crate::{Error, Result};
use sims_content::ui_graphics::UiGraphics;
use sims_formats::bitmap::Image;
use std::collections::BTreeMap;

/// The color the interface's bitmaps use for "no pixel here". The old
/// prototype keyed the same color (legacy/crates/sims-render/src/dialog).
/// The TGAs have an alpha channel instead and no pixel of this color.
pub const KEY: [u8; 3] = [255, 0, 255];

/// The font sizes the interface uses (`Fonts/variablesans_NN.ffn`):
/// sizes 7..14 plus 18 (`0x6f2b0c`, Child Report Card grade button).
pub const FONT_SIZES: [u8; 8] = [7, 8, 9, 10, 11, 12, 14, 18];

/// Bitmaps by resource ID, and fonts by size.
#[derive(Debug, Clone, Default)]
pub struct Art {
    images: BTreeMap<i32, Image>,
    fonts: BTreeMap<u8, Font>,
    /// Requested resources the install's tables do not name.
    pub missing: Vec<i32>,
}

/// Un-premultiplies a UI TGA's partially transparent pixels (`0 < a < 255`)
/// as the original's TGA loader does at `0x4deb3d..0x4deb9d` when
/// `[0x6c8c3c] == 1` (true for every UI TGA except the Magicland mist
/// overlays `0x1655..0x1657`): `c = trunc(c * (255.0f / a) + 0.5f) as u8`,
/// in 24-bit single precision (`0x4fad5c`) taking the low 8 bits on store
/// (`0x5dff82..0x5dff94`). Opaque and colorkeyed BMP pixels (`a == 0` or
/// `a == 255`) are untouched.
fn unpremultiply_tga(image: &mut Image) {
    for px in image.rgba.as_chunks_mut::<4>().0 {
        let a = px[3];
        if a > 0 && a < 255 {
            let scale = 255.0f32 / f32::from(a);
            for ch in &mut px[..3] {
                *ch = ((f32::from(*ch) * scale + 0.5f32).trunc() as i32) as u8;
            }
        }
    }
}

impl Art {
    /// Loads resources `ids` in `language` (falling back to the rows for
    /// every language) and the fonts of [`FONT_SIZES`].
    pub fn load(ui: &UiGraphics, language: u8, ids: &[i32]) -> Result<Self> {
        Self::load_with_optional(ui, language, ids, &[])
    }

    /// Loads required `ids`, optional `optional_ids`, and the fonts of
    /// [`FONT_SIZES`] in parallel, inserting all images and missing IDs in
    /// exact input order.
    pub fn load_with_optional(
        ui: &UiGraphics,
        language: u8,
        ids: &[i32],
        optional_ids: &[i32],
    ) -> Result<Self> {
        let decode_req = |&id: &i32| -> Result<(i32, Option<Image>)> {
            match ui.bitmap(id, language).map_err(Error::Content)? {
                Some(mut image) => {
                    image.key(KEY);
                    unpremultiply_tga(&mut image);
                    Ok((id, Some(image)))
                }
                None => Ok((id, None)),
            }
        };
        let decode_opt = |&id: &i32| -> (i32, Option<Image>) {
            match ui.bitmap(id, language) {
                Ok(Some(mut image)) => {
                    image.key(KEY);
                    unpremultiply_tga(&mut image);
                    (id, Some(image))
                }
                Ok(None) | Err(_) => (id, None),
            }
        };
        let decode_font = |size: u8| -> Result<(u8, Font)> {
            let bytes = ui
                .read(&format!("Fonts/variablesans_{size:02}.ffn"))
                .map_err(Error::Content)?;
            let font = sims_formats::ffn::Font::parse(&bytes).map_err(Error::Format)?;
            Ok((size, Font::new(font)))
        };
        let workers = std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1)
            .clamp(1, 8);
        let (req_results, opt_results, font_results) = if workers <= 1
            || ids.len() + optional_ids.len() <= 4
        {
            (
                ids.iter().map(decode_req).collect::<Vec<_>>(),
                optional_ids.iter().map(decode_opt).collect::<Vec<_>>(),
                FONT_SIZES.into_iter().map(decode_font).collect::<Vec<_>>(),
            )
        } else {
            // Keep total spawned workers <= 8 (bitmap workers + 1 font worker).
            enum BitmapJob {
                Required(i32),
                Optional(i32),
            }
            enum BitmapOut {
                Required(Result<(i32, Option<Image>)>),
                Optional((i32, Option<Image>)),
            }
            let mut jobs = Vec::with_capacity(ids.len() + optional_ids.len());
            jobs.extend(ids.iter().copied().map(BitmapJob::Required));
            jobs.extend(optional_ids.iter().copied().map(BitmapJob::Optional));
            let bitmap_workers = (workers - 1).max(1).min(jobs.len());
            let chunk_size = jobs.len().div_ceil(bitmap_workers);
            let (bitmap_out, font_out) = std::thread::scope(|scope| {
                let handles: Vec<_> = jobs
                    .chunks(chunk_size)
                    .map(|chunk| {
                        scope.spawn(move || {
                            chunk
                                .iter()
                                .map(|job| match job {
                                    BitmapJob::Required(id) => BitmapOut::Required(decode_req(id)),
                                    BitmapJob::Optional(id) => BitmapOut::Optional(decode_opt(id)),
                                })
                                .collect::<Vec<_>>()
                        })
                    })
                    .collect();
                let font_handle = scope
                    .spawn(move || FONT_SIZES.into_iter().map(decode_font).collect::<Vec<_>>());
                let mut out = Vec::with_capacity(jobs.len());
                for h in handles {
                    out.extend(h.join().expect("art bitmap worker panicked"));
                }
                let fonts = font_handle.join().expect("art font worker panicked");
                (out, fonts)
            });
            let mut req_out = Vec::with_capacity(ids.len());
            let mut opt_out = Vec::with_capacity(optional_ids.len());
            for item in bitmap_out {
                match item {
                    BitmapOut::Required(r) => req_out.push(r),
                    BitmapOut::Optional(o) => opt_out.push(o),
                }
            }
            (req_out, opt_out, font_out)
        };
        let mut art = Self::default();
        for item in req_results {
            let (id, image) = item?;
            match image {
                Some(image) => {
                    art.images.insert(id, image);
                }
                None => art.missing.push(id),
            }
        }
        for item in font_results {
            let (size, font) = item?;
            art.fonts.insert(size, font);
        }
        for (id, image) in opt_results {
            match image {
                Some(image) => {
                    art.images.insert(id, image);
                }
                None => art.missing.push(id),
            }
        }
        Ok(art)
    }

    /// Loads resource `id` in `language` too, if it can: one the tables
    /// do not name, or whose file is missing or unreadable, is only
    /// noted in [`Art::missing`], and what draws it goes without.
    pub fn load_optional(&mut self, ui: &UiGraphics, language: u8, id: i32) {
        match ui.bitmap(id, language) {
            Ok(Some(mut image)) => {
                image.key(KEY);
                unpremultiply_tga(&mut image);
                self.images.insert(id, image);
            }
            Ok(None) | Err(_) => self.missing.push(id),
        }
    }

    /// Adds or replaces an image, as tests and hosts with their own art do.
    pub fn insert(&mut self, id: i32, image: Image) {
        self.images.insert(id, image);
    }

    /// Adds or replaces a font.
    pub fn insert_font(&mut self, size: u8, font: Font) {
        self.fonts.insert(size, font);
    }

    pub fn image(&self, id: i32) -> Option<&Image> {
        self.images.get(&id)
    }

    pub fn font(&self, size: u8) -> Option<&Font> {
        self.fonts.get(&size)
    }

    /// Cell `cell` of a button strip: the original's buttons keep their
    /// states side by side in one bitmap, `cells` of them.
    pub fn cell(&self, id: i32, cells: u32, cell: u32) -> Option<Image> {
        let image = self.image(id)?;
        let width = image.width / cells.max(1);
        image.crop(width.checked_mul(cell)?, 0, width, image.height)
    }

    /// Builds a 100% original, clean-room procedural UI artwork and bitmap font
    /// bundle requiring zero EA/Maxis `UIGraphics.far` assets.
    ///
    /// Supplies crisp procedural bitmap fonts for every size in [`FONT_SIZES`]
    /// and clean beveled chrome, 9-slice frames, buttons, bars, pie-menu
    /// placeholder, and action-queue overlays so `Hud`, `PieMenu`, `Dialog`,
    /// `OptionsPanel`, `BuyMode`, `BuildMode`, and `CheatConsole` render
    /// legibly out of the box in standalone `open-pack/` mode.
    pub fn clean_room_procedural() -> Self {
        let mut art = Self::default();
        for size in FONT_SIZES {
            art.insert_font(size, build_clean_room_font(size));
        }

        // Core HUD chrome (`hud::RESOURCES` + `4918` tall Buy strip).
        art.insert(
            crate::hud::id::BACK_PATCH,
            build_bevel_panel(
                52,
                crate::hud::STRIP as u32,
                [26, 38, 62],
                [66, 94, 138],
                [14, 20, 34],
                [26, 38, 62],
            ),
        );
        art.insert(crate::hud::id::CLUSTER, build_cluster_bg());
        art.insert(
            crate::hud::id::STRIP,
            build_bevel_panel(
                580,
                crate::hud::STRIP as u32,
                [26, 38, 62],
                [66, 94, 138],
                [14, 20, 34],
                [88, 126, 178],
            ),
        );
        art.insert(4918, build_tall_strip_bg());
        art.insert(
            crate::hud::id::GAUGE,
            build_bevel_panel(
                108,
                crate::hud::STRIP as u32,
                [22, 33, 54],
                [56, 82, 122],
                [12, 18, 30],
                [76, 112, 160],
            ),
        );

        // Mode, camera, speed, and panel tab buttons (4-cell horizontal strips)
        // sized to match `crate::hud::layout`'s exact hit-target rectangles and
        // stamped with crisp clean-room procedural glyphs.
        let mode_buttons: &[(i32, u32, u32, [u8; 3], ButtonGlyph)] = &[
            (
                crate::hud::id::LIVE,
                50,
                50,
                [44, 134, 92],
                ButtonGlyph::Live,
            ),
            (
                crate::hud::id::BUY,
                45,
                47,
                [52, 112, 176],
                ButtonGlyph::Buy,
            ),
            (
                crate::hud::id::BUILD,
                39,
                39,
                [168, 118, 52],
                ButtonGlyph::Build,
            ),
            (
                crate::hud::id::OPTIONS,
                23,
                23,
                [104, 88, 154],
                ButtonGlyph::Options,
            ),
            (
                crate::hud::id::ROTATE_RIGHT,
                27,
                27,
                [56, 88, 132],
                ButtonGlyph::ArrowRight,
            ),
            (
                crate::hud::id::ROTATE_LEFT,
                27,
                27,
                [56, 88, 132],
                ButtonGlyph::ArrowLeft,
            ),
            (
                crate::hud::id::ZOOM_IN,
                27,
                27,
                [56, 88, 132],
                ButtonGlyph::Plus,
            ),
            (
                crate::hud::id::ZOOM_OUT,
                27,
                27,
                [56, 88, 132],
                ButtonGlyph::Minus,
            ),
            (
                crate::hud::id::FLOOR_DOWN,
                21,
                11,
                [58, 92, 136],
                ButtonGlyph::TriDown,
            ),
            (
                crate::hud::id::FLOOR_UP,
                22,
                15,
                [58, 92, 136],
                ButtonGlyph::TriUp,
            ),
            (
                crate::hud::id::WALLS_DOWN,
                22,
                15,
                [64, 98, 138],
                ButtonGlyph::WallsDown,
            ),
            (
                crate::hud::id::CUTAWAY,
                21,
                21,
                [64, 98, 138],
                ButtonGlyph::Cutaway,
            ),
            (
                crate::hud::id::WALLS_UP,
                21,
                21,
                [64, 98, 138],
                ButtonGlyph::WallsUp,
            ),
            (
                crate::hud::id::ROOF,
                22,
                19,
                [64, 98, 138],
                ButtonGlyph::Roof,
            ),
            (
                crate::hud::id::PHOTO,
                29,
                29,
                [72, 116, 152],
                ButtonGlyph::Camera,
            ),
            (
                crate::hud::id::NORMAL,
                11,
                15,
                [48, 128, 86],
                ButtonGlyph::Speed1,
            ),
            (
                crate::hud::id::FAST,
                20,
                15,
                [48, 128, 86],
                ButtonGlyph::Speed2,
            ),
            (
                crate::hud::id::ULTRA,
                32,
                15,
                [48, 128, 86],
                ButtonGlyph::Speed3,
            ),
            (
                crate::hud::id::NEEDS,
                32,
                39,
                [48, 132, 92],
                ButtonGlyph::Bars,
            ),
            (
                crate::hud::id::PERSONALITY,
                27,
                30,
                [132, 96, 154],
                ButtonGlyph::Star,
            ),
            (
                crate::hud::id::HOUSE,
                27,
                30,
                [156, 116, 56],
                ButtonGlyph::Build,
            ),
            (
                crate::hud::id::JOB,
                27,
                30,
                [66, 112, 168],
                ButtonGlyph::Briefcase,
            ),
            (
                crate::hud::id::RELATIONSHIPS,
                27,
                30,
                [164, 82, 108],
                ButtonGlyph::Heart,
            ),
            (
                crate::hud::id::INTERESTS,
                27,
                30,
                [66, 138, 146],
                ButtonGlyph::Diamond,
            ),
            (
                crate::hud::id::GIFTS,
                27,
                30,
                [162, 132, 54],
                ButtonGlyph::BoxIcon,
            ),
            (
                crate::hud::id::HELP,
                10,
                15,
                [62, 106, 158],
                ButtonGlyph::Question,
            ),
            (
                crate::hud::id::PAGE_BACK,
                9,
                49,
                [62, 98, 142],
                ButtonGlyph::ArrowLeft,
            ),
            (
                crate::hud::id::PAGE_ON,
                9,
                49,
                [62, 98, 142],
                ButtonGlyph::ArrowRight,
            ),
            (
                crate::hud::id::FAMILY,
                20,
                20,
                [64, 124, 96],
                ButtonGlyph::Build,
            ),
            (
                crate::hud::id::FRIENDS,
                20,
                20,
                [64, 108, 156],
                ButtonGlyph::Heart,
            ),
            (
                crate::hud::id::STARS,
                20,
                20,
                [174, 142, 52],
                ButtonGlyph::Star,
            ),
            (
                crate::hud::id::EVERYONE,
                20,
                20,
                [82, 104, 138],
                ButtonGlyph::Options,
            ),
            (
                crate::hud::id::FAME_FRIENDS,
                22,
                20,
                [168, 140, 56],
                ButtonGlyph::Star,
            ),
        ];
        for &(id, cw, ch, accent, glyph) in mode_buttons {
            art.insert(id, build_button_strip_with_glyph(cw, ch, 4, accent, glyph));
        }

        // Pause button: 4 columns x 2 rows of 15x15 cells (`60x30`, matching
        // `Kind::Pause` slicing `src_x = col * 15, src_y = row * 15`).
        art.insert(crate::hud::id::PAUSE, build_pause_button_sheet());

        // Mode trim strips.
        for (id, accent) in [
            (crate::hud::id::LIVE_TRIM, [56, 162, 112]),
            (crate::hud::id::BUY_TRIM, [64, 136, 208]),
            (crate::hud::id::BUILD_TRIM, [196, 140, 62]),
            (crate::hud::id::OPTIONS_TRIM, [128, 106, 186]),
        ] {
            art.insert(
                id,
                build_bevel_panel(16, 16, accent, [220, 236, 255], [22, 32, 52], accent),
            );
        }

        // Mood / motive / trait / skill / rating bars and small status icons.
        art.insert(
            crate::hud::id::GREEN_BARS,
            build_bar_fill(27, 25, [48, 204, 96], [26, 132, 60]),
        );
        art.insert(
            crate::hud::id::RED_BARS,
            build_bar_fill(27, 25, [222, 72, 68], [148, 34, 34]),
        );
        art.insert(
            crate::hud::id::TRAIT_FULL,
            build_bar_fill(80, 10, [74, 198, 228], [38, 128, 162]),
        );
        art.insert(
            crate::hud::id::TRAIT_EMPTY,
            build_bar_fill(80, 10, [36, 48, 72], [24, 34, 52]),
        );
        art.insert(
            crate::hud::id::SKILL_FULL,
            build_bar_fill(80, 10, [86, 212, 122], [44, 144, 74]),
        );
        art.insert(
            crate::hud::id::SKILL_EMPTY,
            build_bar_fill(80, 10, [36, 48, 72], [24, 34, 52]),
        );
        art.insert(
            crate::hud::id::RATING_EMPTY,
            build_bar_fill(80, 10, [36, 48, 72], [24, 34, 52]),
        );
        art.insert(
            crate::hud::id::FAME_BAR_FULL,
            build_bar_fill(80, 16, [238, 198, 66], [176, 132, 32]),
        );
        art.insert(
            crate::hud::id::FAME_BAR_EMPTY,
            build_bar_fill(80, 16, [38, 48, 70], [26, 34, 50]),
        );
        art.insert(
            crate::hud::id::CAREERS,
            build_button_strip(32, 32, 1, [74, 122, 178]),
        );
        art.insert(
            crate::hud::id::SALARY,
            build_bevel_panel(
                16,
                16,
                [72, 168, 92],
                [142, 224, 158],
                [32, 92, 46],
                [42, 120, 62],
            ),
        );
        art.insert(
            crate::hud::id::PERFORMANCE,
            build_bevel_panel(
                16,
                16,
                [82, 148, 214],
                [154, 206, 250],
                [40, 82, 128],
                [58, 114, 172],
            ),
        );
        art.insert(
            crate::hud::id::FRIENDS_NEEDED,
            build_bevel_panel(
                16,
                16,
                [206, 164, 62],
                [246, 214, 124],
                [128, 96, 30],
                [164, 128, 46],
            ),
        );
        art.insert(
            crate::hud::id::FRIENDS_ICON,
            build_bevel_panel(
                14,
                14,
                [68, 196, 118],
                [142, 238, 176],
                [30, 108, 60],
                [42, 138, 78],
            ),
        );
        art.insert(
            crate::hud::id::PINK_HEART,
            build_bevel_panel(
                14,
                14,
                [232, 132, 174],
                [252, 190, 216],
                [156, 68, 106],
                [192, 92, 136],
            ),
        );
        art.insert(
            crate::hud::id::RED_HEART,
            build_bevel_panel(
                14,
                14,
                [224, 58, 72],
                [250, 132, 142],
                [142, 24, 34],
                [176, 36, 48],
            ),
        );

        // Household portrait template (45x45 x 4 cells) & Catalog thumbnail template (45x45 x 4 cells).
        art.insert(
            crate::hud::id::TEMPLATE,
            build_button_strip(45, 45, 4, [54, 82, 122]),
        );
        art.insert(
            crate::thumbnail::TEMPLATE,
            build_button_strip(45, 45, 4, [44, 66, 98]),
        );

        // Dialog & 9-slice frames (PopupInfoTiles 3008, GenDlg 22, HotspotPopupTiler 5107).
        art.insert(
            crate::dialog::BUTTON,
            build_button_strip(65, 33, 4, [54, 102, 162]),
        );
        // Non-modal dialog dismiss box: four 12x11 cells (normal, selected,
        // hover, disabled).
        art.insert(
            crate::dialog::DISMISS,
            build_button_strip(
                crate::dialog::DISMISS_SIZE[0] as u32,
                crate::dialog::DISMISS_SIZE[1] as u32,
                4,
                [54, 102, 162],
            ),
        );
        art.insert(
            crate::dialog::BACKGROUND,
            build_nine_slice(
                12,
                [24, 38, 66],
                [86, 132, 192],
                [14, 22, 42],
                [108, 162, 224],
            ),
        );
        art.insert(
            crate::dialog::GENERIC_FRAME,
            build_nine_slice(
                10,
                [26, 40, 68],
                [88, 134, 194],
                [16, 24, 44],
                [110, 164, 226],
            ),
        );
        art.insert(
            crate::neighborhood::art_id::POPUP_TILER,
            build_nine_slice(
                12,
                [20, 32, 58],
                [76, 122, 184],
                [12, 18, 36],
                [96, 148, 214],
            ),
        );
        art.insert(
            crate::dialog::ICON_STUB,
            build_bevel_panel(
                133,
                103,
                [28, 42, 70],
                [74, 108, 158],
                [14, 22, 38],
                [86, 126, 182],
            ),
        );

        // Pie menu transparent 201x201 placeholder (`PieFace1.bmp`).
        art.insert(
            crate::pie::FACE,
            Image {
                width: crate::pie::FACE_SIZE[0] as u32,
                height: crate::pie::FACE_SIZE[1] as u32,
                rgba: vec![0; (crate::pie::FACE_SIZE[0] * crate::pie::FACE_SIZE[1] * 4) as usize],
            },
        );

        // Action queue frame (2 x 45x45 with blue `[0, 0, 255]` picture cutout) & red X cancel overlay.
        art.insert(crate::queue::FRAME, build_queue_frame_strip());
        art.insert(crate::queue::CANCEL_OVERLAY, build_queue_cancel_overlay());

        // Options panel artwork (`options::RESOURCES` + `SAVE_CARD`).
        art.insert(
            crate::options::id::TOOTHPICK_HORIZ,
            build_bevel_panel(
                201,
                2,
                [76, 112, 162],
                [116, 158, 212],
                [28, 42, 66],
                [76, 112, 162],
            ),
        );
        art.insert(
            crate::options::id::TOOTHPICK_VERT,
            build_bevel_panel(
                2,
                89,
                [76, 112, 162],
                [116, 158, 212],
                [28, 42, 66],
                [76, 112, 162],
            ),
        );
        for &(id, cw, ch, cells, accent) in &[
            (crate::options::id::SAVE, 39u32, 39u32, 4u32, [48, 134, 96]),
            (crate::options::id::NEIGHBORHOOD, 49, 37, 4, [62, 114, 174]),
            (crate::options::id::QUIT, 50, 40, 4, [168, 66, 72]),
            (crate::options::id::CHECKBOX, 20, 21, 6, [64, 132, 188]),
            (crate::options::id::RADIO, 20, 20, 6, [64, 132, 188]),
            (crate::options::id::SLIDER_ON, 11, 14, 4, [72, 194, 138]),
            (crate::options::id::SLIDER_OFF, 11, 14, 4, [74, 86, 108]),
            (crate::options::id::GRAPHICS, 38, 40, 4, [72, 118, 172]),
            (crate::options::id::SOUND, 47, 40, 4, [72, 138, 154]),
            (crate::options::id::PLAY, 55, 40, 4, [72, 148, 108]),
            (
                crate::options::id::TUTORIAL_RESET,
                22,
                30,
                4,
                [154, 116, 62],
            ),
            (crate::album::CAM_VIEW_ALBUM, 36, 36, 4, [72, 124, 178]),
        ] {
            art.insert(id, build_button_strip(cw, ch, cells, accent));
        }
        art.insert(
            crate::options::id::SAVE_CARD,
            build_bevel_panel(
                120,
                80,
                [30, 46, 76],
                [84, 124, 180],
                [16, 24, 42],
                [96, 142, 202],
            ),
        );

        // Camera, Buy Mode, and Build Mode optional button strips.
        for id in crate::camera::RESOURCES
            .iter()
            .copied()
            .chain(crate::buy::resource_ids())
            .chain(crate::build::resource_ids())
        {
            art.images
                .entry(id)
                .or_insert_with(|| build_button_strip(36, 36, 4, [58, 96, 146]));
        }

        art
    }
}

/// 100% original clean-room 5×7 pixel glyph bitmaps for printable ASCII
/// `0x20..=0x7E` plus Windows-1252 `0xA7` (`§`). Each byte is one row of 5
/// pixels (MSB bit 4 = leftmost pixel, bit 0 = rightmost pixel).
fn clean_room_glyph_5x7(code: u8) -> [u8; 7] {
    match code {
        b' ' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        b'!' => [0x04, 0x04, 0x04, 0x04, 0x04, 0x00, 0x04],
        b'"' => [0x0A, 0x0A, 0x00, 0x00, 0x00, 0x00, 0x00],
        b'#' => [0x0A, 0x1F, 0x0A, 0x0A, 0x1F, 0x0A, 0x00],
        b'$' => [0x04, 0x0F, 0x14, 0x0E, 0x05, 0x1E, 0x04],
        b'%' => [0x19, 0x1A, 0x02, 0x04, 0x08, 0x0B, 0x13],
        b'&' => [0x0C, 0x12, 0x14, 0x08, 0x15, 0x12, 0x0D],
        b'\'' => [0x04, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00],
        b'(' => [0x02, 0x04, 0x08, 0x08, 0x08, 0x04, 0x02],
        b')' => [0x08, 0x04, 0x02, 0x02, 0x02, 0x04, 0x08],
        b'*' => [0x00, 0x0A, 0x04, 0x1F, 0x04, 0x0A, 0x00],
        b'+' => [0x00, 0x04, 0x04, 0x1F, 0x04, 0x04, 0x00],
        b',' => [0x00, 0x00, 0x00, 0x00, 0x04, 0x04, 0x08],
        b'-' => [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00],
        b'.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x06, 0x06],
        b'/' => [0x01, 0x02, 0x02, 0x04, 0x08, 0x08, 0x10],
        b'0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        b'1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        b'2' => [0x0E, 0x11, 0x01, 0x06, 0x08, 0x10, 0x1F],
        b'3' => [0x1F, 0x02, 0x04, 0x06, 0x01, 0x11, 0x0E],
        b'4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        b'5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        b'6' => [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        b'7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        b'8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        b'9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
        b':' => [0x00, 0x06, 0x06, 0x00, 0x06, 0x06, 0x00],
        b';' => [0x00, 0x04, 0x04, 0x00, 0x04, 0x04, 0x08],
        b'<' => [0x02, 0x04, 0x08, 0x10, 0x08, 0x04, 0x02],
        b'=' => [0x00, 0x00, 0x1F, 0x00, 0x1F, 0x00, 0x00],
        b'>' => [0x08, 0x04, 0x02, 0x01, 0x02, 0x04, 0x08],
        b'?' => [0x0E, 0x11, 0x01, 0x06, 0x04, 0x00, 0x04],
        b'@' => [0x0E, 0x11, 0x17, 0x15, 0x17, 0x10, 0x0E],
        b'A' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        b'B' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        b'C' => [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
        b'D' => [0x1C, 0x12, 0x11, 0x11, 0x11, 0x12, 0x1C],
        b'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        b'F' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
        b'G' => [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F],
        b'H' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        b'I' => [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
        b'J' => [0x07, 0x02, 0x02, 0x02, 0x12, 0x12, 0x0C],
        b'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        b'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
        b'M' => [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
        b'N' => [0x11, 0x19, 0x15, 0x13, 0x11, 0x11, 0x11],
        b'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        b'P' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
        b'Q' => [0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D],
        b'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        b'S' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
        b'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        b'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        b'V' => [0x11, 0x11, 0x11, 0x11, 0x0A, 0x0A, 0x04],
        b'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x1B, 0x11],
        b'X' => [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11],
        b'Y' => [0x11, 0x11, 0x0A, 0x04, 0x04, 0x04, 0x04],
        b'Z' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F],
        b'[' => [0x0E, 0x08, 0x08, 0x08, 0x08, 0x08, 0x0E],
        b'\\' => [0x10, 0x08, 0x08, 0x04, 0x02, 0x02, 0x01],
        b']' => [0x0E, 0x02, 0x02, 0x02, 0x02, 0x02, 0x0E],
        b'^' => [0x04, 0x0A, 0x11, 0x00, 0x00, 0x00, 0x00],
        b'_' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1F],
        b'`' => [0x08, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00],
        b'a' => [0x00, 0x00, 0x0E, 0x01, 0x0F, 0x11, 0x0F],
        b'b' => [0x10, 0x10, 0x1E, 0x11, 0x11, 0x11, 0x1E],
        b'c' => [0x00, 0x00, 0x0E, 0x10, 0x10, 0x11, 0x0E],
        b'd' => [0x01, 0x01, 0x0F, 0x11, 0x11, 0x11, 0x0F],
        b'e' => [0x00, 0x00, 0x0E, 0x11, 0x1F, 0x10, 0x0E],
        b'f' => [0x06, 0x08, 0x1E, 0x08, 0x08, 0x08, 0x08],
        b'g' => [0x00, 0x00, 0x0F, 0x11, 0x0F, 0x01, 0x0E],
        b'h' => [0x10, 0x10, 0x1E, 0x11, 0x11, 0x11, 0x11],
        b'i' => [0x04, 0x00, 0x0C, 0x04, 0x04, 0x04, 0x0E],
        b'j' => [0x02, 0x00, 0x06, 0x02, 0x02, 0x12, 0x0C],
        b'k' => [0x10, 0x10, 0x12, 0x14, 0x18, 0x14, 0x12],
        b'l' => [0x0C, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
        b'm' => [0x00, 0x00, 0x1A, 0x15, 0x15, 0x11, 0x11],
        b'n' => [0x00, 0x00, 0x1E, 0x11, 0x11, 0x11, 0x11],
        b'o' => [0x00, 0x00, 0x0E, 0x11, 0x11, 0x11, 0x0E],
        b'p' => [0x00, 0x00, 0x1E, 0x11, 0x1E, 0x10, 0x10],
        b'q' => [0x00, 0x00, 0x0F, 0x11, 0x0F, 0x01, 0x01],
        b'r' => [0x00, 0x00, 0x16, 0x19, 0x10, 0x10, 0x10],
        b's' => [0x00, 0x00, 0x0F, 0x10, 0x0E, 0x01, 0x1E],
        b't' => [0x08, 0x08, 0x1E, 0x08, 0x08, 0x09, 0x06],
        b'u' => [0x00, 0x00, 0x11, 0x11, 0x11, 0x13, 0x0D],
        b'v' => [0x00, 0x00, 0x11, 0x11, 0x0A, 0x0A, 0x04],
        b'w' => [0x00, 0x00, 0x11, 0x11, 0x15, 0x15, 0x0A],
        b'x' => [0x00, 0x00, 0x11, 0x0A, 0x04, 0x0A, 0x11],
        b'y' => [0x00, 0x00, 0x11, 0x11, 0x0F, 0x01, 0x0E],
        b'z' => [0x00, 0x00, 0x1F, 0x02, 0x04, 0x08, 0x1F],
        b'{' => [0x02, 0x04, 0x04, 0x08, 0x04, 0x04, 0x02],
        b'|' => [0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        b'}' => [0x08, 0x04, 0x04, 0x02, 0x04, 0x04, 0x08],
        b'~' => [0x00, 0x08, 0x15, 0x02, 0x00, 0x00, 0x00],
        0xA7 => [0x0E, 0x10, 0x0E, 0x11, 0x0E, 0x01, 0x0E], // § (Simoleons)
        _ => [0x0E, 0x11, 0x02, 0x04, 0x00, 0x04, 0x00],
    }
}

fn build_clean_room_font(size: u8) -> Font {
    let scale_x: usize = if size >= 18 { 2 } else { 1 };
    let scale_y: usize = if size >= 14 { 2 } else { 1 };
    let glyph_w = 5 * scale_x;
    let glyph_h = 7 * scale_y;
    let codes: Vec<u8> = (0x21u8..=0x7E).chain(std::iter::once(0xA7u8)).collect();
    let cols = 16usize;
    let rows = codes.len().div_ceil(cols);
    let cell_w = glyph_w + 1;
    let cell_h = glyph_h + 1;
    let atlas_w = (cols * cell_w) as u16;
    let atlas_h = (rows * cell_h) as u16;
    let mut coverage = vec![0u8; usize::from(atlas_w) * usize::from(atlas_h)];
    let mut glyphs = Vec::with_capacity(codes.len());
    for (idx, &code) in codes.iter().enumerate() {
        let col = idx % cols;
        let row = idx / cols;
        let ax = col * cell_w;
        let ay = row * cell_h;
        let bits = clean_room_glyph_5x7(code);
        for (ry, &mask) in bits.iter().enumerate() {
            for rx in 0..5usize {
                if (mask >> (4 - rx)) & 1 != 0 {
                    for sy in 0..scale_y {
                        for sx in 0..scale_x {
                            let px = ax + rx * scale_x + sx;
                            let py = ay + ry * scale_y + sy;
                            coverage[py * usize::from(atlas_w) + px] = 15;
                        }
                    }
                }
            }
        }
        glyphs.push(sims_formats::ffn::Glyph {
            code: u16::from(code),
            width: glyph_w as u8,
            height: glyph_h as u8,
            atlas_x: ax as u16,
            atlas_y: ay as u16,
            first_bearing: 0,
            bearing_x: 1,
            bearing_y: 0,
        });
    }
    Font::new(sims_formats::ffn::Font {
        flags: 0,
        glyphs,
        width: atlas_w,
        height: atlas_h,
        coverage,
    })
}

fn shade_rgb(rgb: [u8; 3], num: u16, den: u16) -> [u8; 3] {
    [
        (u16::from(rgb[0]) * num / den).min(255) as u8,
        (u16::from(rgb[1]) * num / den).min(255) as u8,
        (u16::from(rgb[2]) * num / den).min(255) as u8,
    ]
}

fn build_bevel_panel(
    width: u32,
    height: u32,
    fill: [u8; 3],
    top_hi: [u8; 3],
    bot_lo: [u8; 3],
    border: [u8; 3],
) -> Image {
    let w = width as usize;
    let h = height as usize;
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let rgb = if x == 0 || y == 0 || x + 1 == w || y + 1 == h {
                border
            } else if y == 1 || x == 1 {
                top_hi
            } else if y + 2 == h || x + 2 == w {
                bot_lo
            } else {
                fill
            };
            let p = (y * w + x) * 4;
            rgba[p] = rgb[0];
            rgba[p + 1] = rgb[1];
            rgba[p + 2] = rgb[2];
            rgba[p + 3] = 255;
        }
    }
    Image {
        width,
        height,
        rgba,
    }
}

#[derive(Clone, Copy)]
enum ButtonGlyph {
    None,
    Live,
    Buy,
    Build,
    Options,
    ArrowLeft,
    ArrowRight,
    Plus,
    Minus,
    TriUp,
    TriDown,
    WallsDown,
    Cutaway,
    WallsUp,
    Roof,
    Camera,
    Pause,
    Speed1,
    Speed2,
    Speed3,
    Bars,
    Star,
    Briefcase,
    Heart,
    Diamond,
    BoxIcon,
    Question,
}

fn build_button_strip(cell_w: u32, cell_h: u32, cells: u32, accent: [u8; 3]) -> Image {
    build_button_strip_with_glyph(cell_w, cell_h, cells, accent, ButtonGlyph::None)
}

fn build_button_strip_with_glyph(
    cell_w: u32,
    cell_h: u32,
    cells: u32,
    accent: [u8; 3],
    glyph: ButtonGlyph,
) -> Image {
    let total_w = cell_w * cells;
    let w = total_w as usize;
    let h = cell_h as usize;
    let cw = cell_w as usize;
    let mut rgba = vec![0u8; w * h * 4];
    for cell in 0..cells as usize {
        let (fill, hi, lo, border, fg) = match cell {
            0 => (
                shade_rgb(accent, 78, 100),
                shade_rgb(accent, 128, 100),
                shade_rgb(accent, 42, 100),
                shade_rgb(accent, 110, 100),
                [232u8, 242, 255],
            ),
            1 => (
                shade_rgb(accent, 118, 100),
                [210, 246, 255],
                shade_rgb(accent, 55, 100),
                [120, 232, 255],
                [255u8, 255, 255],
            ),
            2 => (
                shade_rgb(accent, 102, 100),
                shade_rgb(accent, 148, 100),
                shade_rgb(accent, 50, 100),
                [188, 224, 255],
                [248u8, 252, 255],
            ),
            _ => (
                [46, 54, 68],
                [64, 74, 92],
                [28, 34, 44],
                [58, 68, 84],
                [108u8, 120, 138],
            ),
        };
        let x0 = cell * cw;
        for y in 0..h {
            for lx in 0..cw {
                let rgb = if lx == 0 || y == 0 || lx + 1 == cw || y + 1 == h {
                    border
                } else if lx == 1 || y == 1 {
                    hi
                } else if lx + 2 == cw || y + 2 == h {
                    lo
                } else {
                    fill
                };
                let p = (y * w + (x0 + lx)) * 4;
                rgba[p] = rgb[0];
                rgba[p + 1] = rgb[1];
                rgba[p + 2] = rgb[2];
                rgba[p + 3] = 255;
            }
        }
        stamp_button_glyph(&mut rgba, w, x0, cw, h, glyph, fg);
    }
    Image {
        width: total_w,
        height: cell_h,
        rgba,
    }
}

fn stamp_button_glyph(
    rgba: &mut [u8],
    stride: usize,
    x0: usize,
    cw: usize,
    ch: usize,
    glyph: ButtonGlyph,
    fg: [u8; 3],
) {
    if matches!(glyph, ButtonGlyph::None) || cw < 6 || ch < 6 {
        return;
    }
    let cx = (cw / 2) as i32;
    let cy = (ch / 2) as i32;
    let r = ((cw.min(ch) as i32) / 3).max(2);
    let mut put = |lx: i32, ly: i32| {
        if lx >= 2 && ly >= 2 && (lx as usize) + 2 < cw && (ly as usize) + 2 < ch {
            let p = (ly as usize * stride + (x0 + lx as usize)) * 4;
            rgba[p] = fg[0];
            rgba[p + 1] = fg[1];
            rgba[p + 2] = fg[2];
            rgba[p + 3] = 255;
        }
    };
    let draw_tri_right = |put: &mut dyn FnMut(i32, i32), ox: i32, oy: i32, size: i32| {
        for dy in -size..=size {
            let span = size - dy.abs();
            for dx in 0..=span {
                put(ox + dx, oy + dy);
            }
        }
    };
    match glyph {
        ButtonGlyph::None => {}
        ButtonGlyph::Live => {
            // Plumb-bob diamond + person shoulders.
            let d = (r / 2).max(3);
            for dy in -d..=d {
                let w = (d - dy.abs()).max(0);
                for dx in -w..=w {
                    put(cx + dx, cy - r / 2 + dy);
                }
            }
            for dy in 0..=r / 2 {
                for dx in -(r / 2 + 2)..=(r / 2 + 2) {
                    put(cx + dx, cy + 3 + dy);
                }
            }
        }
        ButtonGlyph::Buy => {
            // Chair silhouette.
            for dy in -r..=r {
                put(cx - r / 2, cy + dy);
                put(cx - r / 2 + 1, cy + dy);
            }
            for dx in -r / 2..=r / 2 {
                put(cx + dx, cy + 1);
                put(cx + dx, cy + 2);
            }
            for dy in 2..=r {
                put(cx + r / 2, cy + dy);
                put(cx + r / 2 - 1, cy + dy);
            }
        }
        ButtonGlyph::Build => {
            // House gable + walls.
            for dy in 0..=r / 2 {
                for dx in -dy * 2..=dy * 2 {
                    if dx.abs() <= r {
                        put(cx + dx, cy - r + dy);
                    }
                }
            }
            for dy in -r / 2 + 1..=r {
                for dx in -(r * 3 / 4)..=(r * 3 / 4) {
                    if dy < r / 3 || dx.abs() > r / 4 {
                        put(cx + dx, cy + dy);
                    }
                }
            }
        }
        ButtonGlyph::Options => {
            // Three horizontal menu dots.
            for ox in [-4, 0, 4] {
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        put(cx + ox + dx, cy + dy);
                    }
                }
            }
        }
        ButtonGlyph::ArrowRight => {
            let s = (r / 2).clamp(2, 4);
            draw_tri_right(&mut put, cx - s / 2, cy, s);
        }
        ButtonGlyph::ArrowLeft => {
            let s = (r / 2).clamp(2, 4);
            for dy in -s..=s {
                let span = s - dy.abs();
                for dx in 0..=span {
                    put(cx + s / 2 - dx, cy + dy);
                }
            }
        }
        ButtonGlyph::Plus => {
            for d in -r / 2..=r / 2 {
                put(cx + d, cy);
                put(cx + d, cy - 1);
                put(cx, cy + d);
                put(cx - 1, cy + d);
            }
        }
        ButtonGlyph::Minus => {
            for d in -r / 2..=r / 2 {
                put(cx + d, cy);
                put(cx + d, cy - 1);
            }
        }
        ButtonGlyph::TriUp => {
            let s = (ch as i32 / 4).clamp(2, 4);
            for dy in 0..=s {
                for dx in -dy..=dy {
                    put(cx + dx, cy - s / 2 + dy);
                }
            }
        }
        ButtonGlyph::TriDown => {
            let s = (ch as i32 / 4).clamp(2, 4);
            for dy in 0..=s {
                let w = s - dy;
                for dx in -w..=w {
                    put(cx + dx, cy - s / 2 + dy);
                }
            }
        }
        ButtonGlyph::WallsDown => {
            for dx in -r..=r {
                put(cx + dx, cy + 1);
                put(cx + dx, cy + 2);
            }
        }
        ButtonGlyph::Cutaway => {
            for dx in -r..=r {
                put(cx + dx, cy + r / 2);
            }
            for dy in -r / 2..=r / 2 {
                put(cx - r, cy + dy);
                put(cx - r + 1, cy + dy);
            }
        }
        ButtonGlyph::WallsUp => {
            for dy in -r / 2..=r / 2 {
                for dx in -r / 2..=r / 2 {
                    if dx.abs() >= r / 2 - 1 || dy.abs() >= r / 2 - 1 {
                        put(cx + dx, cy + dy);
                    }
                }
            }
        }
        ButtonGlyph::Roof => {
            let s = (r / 2 + 1).clamp(2, 5);
            for dy in 0..=s {
                put(cx - dy, cy - s / 2 + dy);
                put(cx + dy, cy - s / 2 + dy);
                if dy > 0 {
                    put(cx - dy + 1, cy - s / 2 + dy);
                    put(cx + dy - 1, cy - s / 2 + dy);
                }
            }
        }
        ButtonGlyph::Camera => {
            for dy in -r / 2..=r / 2 {
                for dx in -r..=r {
                    if dx * dx + dy * dy > (r / 3) * (r / 3) {
                        put(cx + dx, cy + dy);
                    }
                }
            }
        }
        ButtonGlyph::Pause => {
            for dy in -3..=3 {
                put(cx - 2, cy + dy);
                put(cx - 1, cy + dy);
                put(cx + 1, cy + dy);
                put(cx + 2, cy + dy);
            }
        }
        ButtonGlyph::Speed1 => {
            draw_tri_right(&mut put, cx - 1, cy, 3);
        }
        ButtonGlyph::Speed2 => {
            draw_tri_right(&mut put, cx - 5, cy, 3);
            draw_tri_right(&mut put, cx + 1, cy, 3);
        }
        ButtonGlyph::Speed3 => {
            draw_tri_right(&mut put, cx - 9, cy, 3);
            draw_tri_right(&mut put, cx - 3, cy, 3);
            draw_tri_right(&mut put, cx + 3, cy, 3);
        }
        ButtonGlyph::Bars => {
            for row in [-4, 0, 4] {
                for dx in -r..=r {
                    put(cx + dx, cy + row);
                    put(cx + dx, cy + row + 1);
                }
            }
        }
        ButtonGlyph::Star | ButtonGlyph::Diamond => {
            for dy in -r..=r {
                let w = r - dy.abs();
                for dx in -w..=w {
                    put(cx + dx, cy + dy);
                }
            }
        }
        ButtonGlyph::Briefcase => {
            for dx in -r / 3..=r / 3 {
                put(cx + dx, cy - r / 2 - 1);
            }
            for dy in -r / 2..=r / 2 {
                for dx in -r..=r {
                    put(cx + dx, cy + dy);
                }
            }
        }
        ButtonGlyph::Heart => {
            for dy in -r / 2..=r / 2 {
                for dx in -r / 2..=r / 2 {
                    if dy < 0 && dx == 0 {
                        continue;
                    }
                    if dy <= 0 || dx.abs() <= r / 2 - dy {
                        put(cx + dx, cy + dy);
                    }
                }
            }
        }
        ButtonGlyph::BoxIcon => {
            for dy in -r / 2..=r / 2 {
                for dx in -r / 2..=r / 2 {
                    if dx == 0 || dy == 0 || dx.abs() == r / 2 || dy.abs() == r / 2 {
                        put(cx + dx, cy + dy);
                    }
                }
            }
        }
        ButtonGlyph::Question => {
            for dx in -1..=1 {
                put(cx + dx, cy - 3);
            }
            put(cx + 1, cy - 2);
            put(cx, cy - 1);
            put(cx, cy);
            put(cx, cy + 2);
            put(cx, cy + 3);
        }
    }
}

fn build_pause_button_sheet() -> Image {
    let top = build_button_strip_with_glyph(15, 15, 4, [56, 142, 178], ButtonGlyph::Pause);
    let bot = build_button_strip_with_glyph(15, 15, 4, [186, 56, 62], ButtonGlyph::Pause);
    let mut rgba = Vec::with_capacity(top.rgba.len() + bot.rgba.len());
    rgba.extend_from_slice(&top.rgba);
    rgba.extend_from_slice(&bot.rgba);
    Image {
        width: top.width,
        height: top.height + bot.height,
        rgba,
    }
}

fn build_cluster_bg() -> Image {
    let width = crate::hud::CLUSTER[0] as u32;
    let height = crate::hud::CLUSTER[1] as u32;
    let mut img = build_bevel_panel(
        width,
        height,
        [24, 36, 60],
        [68, 98, 146],
        [12, 18, 32],
        [92, 132, 186],
    );
    // Keep upper-right notch above the bottom strip transparent so the world shows through.
    let w = width as usize;
    let notch_h = (height as usize).saturating_sub(crate::hud::STRIP as usize);
    for y in 0..notch_h {
        for x in 152..w {
            img.rgba[(y * w + x) * 4 + 3] = 0;
        }
    }
    img
}

fn build_tall_strip_bg() -> Image {
    let width = 580u32;
    let height = crate::hud::TALL_STRIP as u32;
    let strip_h = crate::hud::STRIP as usize;
    let top_pad = (height as usize).saturating_sub(strip_h);
    let bottom = build_bevel_panel(
        width,
        strip_h as u32,
        [26, 38, 62],
        [66, 94, 138],
        [14, 20, 34],
        [88, 126, 178],
    );
    let mut rgba = vec![0u8; width as usize * height as usize * 4];
    let offset = top_pad * width as usize * 4;
    rgba[offset..].copy_from_slice(&bottom.rgba);
    Image {
        width,
        height,
        rgba,
    }
}

fn build_bar_fill(width: u32, height: u32, top: [u8; 3], bot: [u8; 3]) -> Image {
    let w = width as usize;
    let h = height as usize;
    let mut rgba = vec![0u8; w * h * 4];
    let denom = h.saturating_sub(1).max(1);
    for y in 0..h {
        let t = y;
        let rgb = [
            ((usize::from(top[0]) * (denom - t) + usize::from(bot[0]) * t) / denom) as u8,
            ((usize::from(top[1]) * (denom - t) + usize::from(bot[1]) * t) / denom) as u8,
            ((usize::from(top[2]) * (denom - t) + usize::from(bot[2]) * t) / denom) as u8,
        ];
        for x in 0..w {
            let p = (y * w + x) * 4;
            rgba[p] = rgb[0];
            rgba[p + 1] = rgb[1];
            rgba[p + 2] = rgb[2];
            rgba[p + 3] = 255;
        }
    }
    Image {
        width,
        height,
        rgba,
    }
}

fn build_nine_slice(tile: u32, fill: [u8; 3], hi: [u8; 3], lo: [u8; 3], border: [u8; 3]) -> Image {
    build_bevel_panel(tile * 3, tile * 3, fill, hi, lo, border)
}

fn build_queue_frame_strip() -> Image {
    let cell = 45usize;
    let width = (cell * 2) as u32;
    let height = cell as u32;
    let w = width as usize;
    let mut rgba = vec![0u8; w * cell * 4];
    for (idx, border) in [[148u8, 168, 196], [242u8, 214, 72]]
        .into_iter()
        .enumerate()
    {
        let x0 = idx * cell;
        for y in 0..cell {
            for lx in 0..cell {
                let p = (y * w + x0 + lx) * 4;
                let on_edge = lx < 3 || y < 3 || lx >= cell - 3 || y >= cell - 3;
                let rgb = if on_edge { border } else { [0, 0, 255] };
                rgba[p] = rgb[0];
                rgba[p + 1] = rgb[1];
                rgba[p + 2] = rgb[2];
                rgba[p + 3] = 255;
            }
        }
    }
    Image {
        width,
        height,
        rgba,
    }
}

fn build_queue_cancel_overlay() -> Image {
    let s = 45usize;
    let mut rgba = vec![0u8; s * s * 4];
    for y in 6..(s - 6) {
        for x in 6..(s - 6) {
            let d1 = (x as i32 - y as i32).abs();
            let d2 = (x as i32 + y as i32 - (s as i32 - 1)).abs();
            if d1 <= 2 || d2 <= 2 {
                let p = (y * s + x) * 4;
                rgba[p] = 224;
                rgba[p + 1] = 44;
                rgba[p + 2] = 44;
                rgba[p + 3] = 255;
            }
        }
    }
    Image {
        width: s as u32,
        height: s as u32,
        rgba,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unpremultiply_tga_restores_straight_rgb_for_partial_alpha() {
        let mut image = Image {
            width: 4,
            height: 1,
            rgba: vec![
                // Transparent (alpha 0): untouched.
                10, 20, 30, 0, // Opaque (alpha 255): untouched.
                10, 20, 30, 255,
                // Single-precision rounding (0x4fad5c): shipped UniversalBack.TGA
                // (160, 75) has blue 56, alpha 112 -> 127 (not f64's 128).
                64, 32, 56, 112,
                // Low-byte wrap (0x5dff82..0x5dff94): shipped HotspotPopupTiler.TGA
                // pixel has blue 82, alpha 35 -> trunc(597.928) & 0xff = 85.
                0, 35, 82, 35,
            ],
        };
        unpremultiply_tga(&mut image);
        assert_eq!(
            image.rgba,
            vec![
                10, 20, 30, 0, 10, 20, 30, 255, 146, 73, 127, 112, 0, 255, 85, 35,
            ]
        );
    }

    #[test]
    fn clean_room_procedural_art_supplies_all_fonts_and_core_resources() {
        let art = Art::clean_room_procedural();
        assert!(art.missing.is_empty());
        for size in FONT_SIZES {
            let font = art
                .font(size)
                .unwrap_or_else(|| panic!("missing clean-room font size {size}"));
            assert!(
                font.width("Hello §1,000!") > 0,
                "font size {size} should have positive text width"
            );
            assert!(font.line_height > 0);
        }
        for &id in crate::hud::RESOURCES
            .iter()
            .chain(&crate::options::RESOURCES)
            .chain(&crate::dialog::RESOURCES)
            .chain(&crate::pie::RESOURCES)
            .chain(&crate::queue::RESOURCES)
            .chain(&crate::thumbnail::RESOURCES)
            .chain(&crate::cheat_console::RESOURCES)
        {
            assert!(
                art.image(id).is_some(),
                "clean-room Art missing required resource ID {id}"
            );
        }

        // Verify Pause button sheet matches 4x2 cells of 15x15 (`60x30`).
        let pause_img = art.image(crate::hud::id::PAUSE).expect("pause sheet");
        assert_eq!((pause_img.width, pause_img.height), (60, 30));

        // Verify every HUD button's procedural cell dimensions match its exact
        // hit-target rectangle in `crate::hud::layout`, and verify hit testing
        // and distinct Pause state rendering across (normal, paused unlit, paused lit).
        let controls = crate::hud::Controls::default();
        let snapshot = crate::household::Snapshot::read(&sims_sim::World::empty(), &[]);
        let items = crate::hud::layout([800, 600], &snapshot, &controls);
        let mut checked_buttons = 0usize;
        for item in &items {
            if let crate::hud::Kind::Button { id, cells, .. } = item.kind
                && let Some(cell) = art.cell(id, cells, 0)
            {
                assert_eq!(
                    (cell.width as i32, cell.height as i32),
                    (item.rect.width, item.rect.height),
                    "button {id} procedural cell {}x{} disagrees with layout hit target {}x{}",
                    cell.width,
                    cell.height,
                    item.rect.width,
                    item.rect.height
                );
                checked_buttons += 1;
            }
        }
        assert!(
            checked_buttons >= 15,
            "expected at least 15 HUD buttons checked against layout rects, got {checked_buttons}"
        );

        // Hit-test Options & Pause centers via `crate::hud::press`.
        let options_item = items
            .iter()
            .find(|i| {
                matches!(
                    i.kind,
                    crate::hud::Kind::Button {
                        id: crate::hud::id::OPTIONS,
                        ..
                    }
                )
            })
            .expect("Options button in layout");
        let opt_center = [
            options_item.rect.x + options_item.rect.width / 2,
            options_item.rect.y + options_item.rect.height / 2,
        ];
        assert_eq!(
            crate::hud::press(&items, opt_center),
            Some(crate::command::Command::SetMode(
                crate::command::Mode::Options
            ))
        );

        let pause_item = items
            .iter()
            .find(|i| matches!(i.kind, crate::hud::Kind::Pause { .. }))
            .expect("Pause button in layout");
        let pause_center = [
            pause_item.rect.x + pause_item.rect.width / 2,
            pause_item.rect.y + pause_item.rect.height / 2,
        ];
        assert_eq!(
            crate::hud::press(&items, pause_center),
            Some(crate::command::Command::TogglePause)
        );

        // Render Pause in normal (row 0 col 0), paused unlit (row 0 col 1), and
        // paused lit (row 1 col 1) and verify all three states produce distinct pixels.
        let render_pause = |paused: bool, pause_lit: bool| -> Vec<u8> {
            let mut snap = crate::household::Snapshot::read(&sims_sim::World::empty(), &[]);
            snap.paused = paused;
            let ctl = crate::hud::Controls {
                pause_lit,
                ..Default::default()
            };
            let layout_items = crate::hud::layout([800, 600], &snap, &ctl);
            let mut frame = crate::frame::Frame::new(800, 600, [0, 0, 0]);
            let faces = crate::household::Faces::default();
            crate::hud::draw(&mut frame, &art, &faces, &layout_items, &ctl);
            let pr = pause_item.rect;
            let mut slice = Vec::new();
            for y in pr.y..pr.y + pr.height {
                for x in pr.x..pr.x + pr.width {
                    slice.extend_from_slice(&frame.pixel(x, y).expect("pause pixel in bounds"));
                }
            }
            slice
        };
        let px_normal = render_pause(false, false);
        let px_paused_cyan = render_pause(true, false);
        let px_paused_red = render_pause(true, true);
        assert_ne!(
            px_normal, px_paused_cyan,
            "Pause normal vs selected must differ"
        );
        assert_ne!(
            px_paused_cyan, px_paused_red,
            "Pause row 0 (cyan) vs row 1 (red) blink states must differ"
        );
    }
}
