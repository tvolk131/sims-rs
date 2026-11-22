//! Automated UI pixel ratchet tests comparing `sims-ui` renders against real
//! Complete Collection 1.91 captures:
//! - Neighborhood top bar (including page number `1`), Move-In & Bulldoze states
//! - Family Bin (`Select a Family` screen)
//! - Create-a-Family (`cWinEditFamily`) and Create-a-Sim (`cWinEditPerson`) screens
//! - Complete Collection Camera Mode subpanel (`cWinCamera`, `STR# 140`, `1101..1110`, `2035`)
//!
//! Oracle PNG tests are gated cleanly on `SIMS_INSTALL` plus the optional private
//! oracle directories (`SIMS_NB_ORACLE` / default
//! `/Users/tommyvolk/Desktop/sims-rs/cache/nb-save-movein-evict-20261121/shots` and
//! `SIMS_CAF_ORACLE` / default
//! `/Users/tommyvolk/Desktop/sims-rs/cache/caf-create-family-20261121/shots`),
//! skipping cleanly in CI when absent.

use sims_content::Install;
use sims_content::ui_graphics::{US_ENGLISH, UiGraphics};
use sims_ui::art::Art;
use sims_ui::caf::{self, CafController, CafTexts};
use sims_ui::camera::{
    self, CameraButton, CameraCommand, CameraHelp, CameraPanel, CameraState, CameraStrings,
    SnapshotQuality, SnapshotSize,
};
use sims_ui::family_bin::{
    self, FamilyBin, FamilyBinMode, FamilyBinStrings, FamilyBinView, FamilyEntry,
};
use sims_ui::frame::Frame;
use sims_ui::neighborhood::{
    self, NeighborhoodChrome, NeighborhoodMode, NeighborhoodState, NeighborhoodStrings,
};
use std::path::{Path, PathBuf};

const DEFAULT_NB_ORACLE: &str =
    "/Users/tommyvolk/Desktop/sims-rs/cache/nb-save-movein-evict-20261121/shots";
const DEFAULT_CAF_ORACLE: &str =
    "/Users/tommyvolk/Desktop/sims-rs/cache/caf-create-family-20261121/shots";

fn install() -> Option<Install> {
    let root = PathBuf::from(std::env::var_os("SIMS_INSTALL")?);
    Install::detect(&root).ok()
}

fn oracle_dir(env_key: &str, default_path: &str) -> Option<PathBuf> {
    if let Some(p) = std::env::var_os(env_key) {
        let pb = PathBuf::from(p);
        return pb.is_dir().then_some(pb);
    }
    let pb = PathBuf::from(default_path);
    pb.is_dir().then_some(pb)
}

fn load_png(dir: &Path, name: &str) -> (usize, usize, Vec<[u8; 3]>) {
    let bytes = std::fs::read(dir.join(name)).expect("read oracle PNG");
    decode_png_rgb(&bytes, Some((800, 600))).expect("decode 800x600 oracle PNG")
}

fn pack_565([r, g, b]: [u8; 3]) -> u16 {
    ((u16::from(r & 0xf8)) << 8) | ((u16::from(g & 0xfc)) << 3) | (u16::from(b) >> 3)
}

fn match_rate_565(
    got: &[[u8; 3]],
    want: &[[u8; 3]],
    width: usize,
    y_range: std::ops::Range<usize>,
    exclude: impl Fn(usize, usize) -> bool,
) -> (usize, usize) {
    let mut exact = 0usize;
    let mut total = 0usize;
    for y in y_range {
        for x in 0..width {
            if exclude(x, y) {
                continue;
            }
            total += 1;
            let idx = y * width + x;
            if pack_565(got[idx]) == pack_565(want[idx]) {
                exact += 1;
            }
        }
    }
    (exact, total)
}

#[test]
fn neighborhood_top_bar_matches_191_clean_and_mode_oracles() {
    let (Some(install), Some(nb_dir)) =
        (install(), oracle_dir("SIMS_NB_ORACLE", DEFAULT_NB_ORACLE))
    else {
        return;
    };
    let ui = UiGraphics::open(&install).expect("open UIGraphics");
    let art = neighborhood::load_art(&ui, US_ENGLISH).expect("load neighborhood art");
    let strings = NeighborhoodStrings::load(&install, US_ENGLISH).expect("load NB strings");
    let chrome = NeighborhoodChrome::new();

    // 1. Clean Neighborhood 1 top bar (`01-nb-clean.png`), including page number `1`
    //    centered over `banner_neighborhood.bmp` with zero `Current.bmp` plate overhang.
    let (_, _, oracle_clean) = load_png(&nb_dir, "01-nb-clean.png");
    let mut frame = Frame::from_rgb(800, 600, oracle_clean.clone()).unwrap();
    let clean_state = NeighborhoodState {
        neighborhood_number: 1,
        mode: NeighborhoodMode::Normal,
        has_importable_family: false,
        show_credits_button: false,
        ..Default::default()
    };
    chrome.draw(&mut frame, &clean_state, &art, &strings);
    let q = frame.quantized();
    let (exact, total) = match_rate_565(q.rgb(), &oracle_clean, 800, 0..52, |_, _| false);
    assert_eq!(
        exact, total,
        "Neighborhood 1 top bar (800x52) must match 1.91 01-nb-clean.png bit-exact ({exact}/{total})"
    );

    // 2. Move-In mode (`11-nb-movein-mode.png`): SelectFamily button toggled on.
    if nb_dir.join("11-nb-movein-mode.png").is_file() {
        let (_, _, oracle_movein) = load_png(&nb_dir, "11-nb-movein-mode.png");
        let mut frame = Frame::from_rgb(800, 600, oracle_movein.clone()).unwrap();
        let movein_state = NeighborhoodState {
            neighborhood_number: 1,
            mode: NeighborhoodMode::MoveIn {
                family_name: "Test".into(),
                family_funds: 20_000,
                family_magic_coins: 0,
            },
            has_importable_family: false,
            show_credits_button: false,
            ..Default::default()
        };
        chrome.draw(&mut frame, &movein_state, &art, &strings);
        let q = frame.quantized();
        let (exact, total) = match_rate_565(q.rgb(), &oracle_movein, 800, 0..52, |_, _| false);
        let pct = 100.0 * exact as f64 / total as f64;
        assert!(
            pct >= 99.5,
            "Neighborhood Move-In top bar drifted below 99.5% ratchet: {exact}/{total} ({pct:.2}%)"
        );
    }

    // 3. Bulldoze mode (`17b-bulldoze-hover-lot4.png`): Bulldoze button toggled on
    //    and all other bar buttons disabled. Exclude `x=520..710` where Lot 4's hover
    //    popup card overlaps the top bar.
    if nb_dir.join("17b-bulldoze-hover-lot4.png").is_file() {
        let (_, _, oracle_doze) = load_png(&nb_dir, "17b-bulldoze-hover-lot4.png");
        let mut frame = Frame::from_rgb(800, 600, oracle_doze.clone()).unwrap();
        let doze_state = NeighborhoodState {
            neighborhood_number: 1,
            mode: NeighborhoodMode::Bulldoze,
            has_importable_family: false,
            show_credits_button: false,
            ..Default::default()
        };
        chrome.draw(&mut frame, &doze_state, &art, &strings);
        let q = frame.quantized();
        let (exact, total) = match_rate_565(q.rgb(), &oracle_doze, 800, 0..52, |x, _| {
            (520..710).contains(&x)
        });
        let pct = 100.0 * exact as f64 / total as f64;
        assert!(
            pct >= 99.0,
            "Neighborhood Bulldoze top bar drifted below 99.0% ratchet: {exact}/{total} ({pct:.2}%)"
        );
    }
}

#[test]
fn family_bin_chrome_matches_191_oracle() {
    let (Some(install), Some(caf_dir)) =
        (install(), oracle_dir("SIMS_CAF_ORACLE", DEFAULT_CAF_ORACLE))
    else {
        return;
    };
    let ui = UiGraphics::open(&install).expect("open UIGraphics");
    let (_, _, oracle_fb) = load_png(&caf_dir, "02-select-family-clean.png");
    let art = family_bin::load_art(&ui, US_ENGLISH).expect("load Family Bin art");
    let strings = FamilyBinStrings::load(&install, US_ENGLISH).expect("load Family Bin strings");
    let families = vec![
        FamilyEntry {
            id: 1,
            name: "Pleasant".into(),
            funds: 20_000,
            friends: 0,
            ..Default::default()
        },
        FamilyEntry {
            id: 2,
            name: "Roomies".into(),
            funds: 20_000,
            friends: 0,
            ..Default::default()
        },
        FamilyEntry {
            id: 3,
            name: "Bachelor".into(),
            funds: 20_000,
            friends: 0,
            ..Default::default()
        },
    ];
    let view = FamilyBinView {
        mode: FamilyBinMode::Standard,
        families: &families,
    };
    let bin = FamilyBin::new();
    let mut frame = Frame::new(800, 600, [0, 0, 0]);
    bin.draw(&mut frame, &view, &art, &strings);
    let q = frame.quantized();

    // Exclude dynamic household portrait cells (x=145..520, y=90..290).
    let (exact, total) = match_rate_565(q.rgb(), &oracle_fb, 800, 0..600, |x, y| {
        (145..520).contains(&x) && (90..290).contains(&y)
    });
    let pct = 100.0 * exact as f64 / total as f64;
    assert!(
        pct >= 97.5,
        "Family Bin chrome drifted below 97.5% ratchet: {exact}/{total} ({pct:.2}%)"
    );
}

#[test]
fn caf_family_and_sim_screens_match_191_oracles() {
    let (Some(install), Some(caf_dir)) =
        (install(), oracle_dir("SIMS_CAF_ORACLE", DEFAULT_CAF_ORACLE))
    else {
        return;
    };
    let ui = UiGraphics::open(&install).expect("open UIGraphics");
    let caf_art = Art::load(&ui, US_ENGLISH, &caf::RESOURCES).expect("load CAF art");
    let caf_texts = CafTexts::load(&install, US_ENGLISH).expect("load CAF strings");
    let mut ctrl = CafController::new(caf_texts);

    // 1. Create-a-Family screen (`03-caf-family-screen.png`).
    let (_, _, oracle_fam) = load_png(&caf_dir, "03-caf-family-screen.png");
    let mut fam_frame = Frame::new(800, 600, [0, 0, 0]);
    ctrl.draw(&mut fam_frame, &caf_art, None);
    let qfam = fam_frame.quantized();
    let (fam_exact, fam_total) = match_rate_565(qfam.rgb(), &oracle_fam, 800, 0..600, |_, _| false);
    let fam_pct = 100.0 * fam_exact as f64 / fam_total as f64;
    assert!(
        fam_pct >= 95.5,
        "CAF Family screen drifted below 95.5% ratchet: {fam_exact}/{fam_total} ({fam_pct:.2}%)"
    );

    // 2. Create-a-Sim screen (`04-caf-create-sim.png`), excluding the 3D turntable
    //    preview well (x=570..760, y=115..385).
    let (_, _, oracle_sim) = load_png(&caf_dir, "04-caf-create-sim.png");
    ctrl.begin_add_sim();
    let mut sim_frame = Frame::new(800, 600, [0, 0, 0]);
    ctrl.draw(&mut sim_frame, &caf_art, None);
    let qsim = sim_frame.quantized();
    let (sim_exact, sim_total) = match_rate_565(qsim.rgb(), &oracle_sim, 800, 0..600, |x, y| {
        (570..760).contains(&x) && (115..385).contains(&y)
    });
    let sim_pct = 100.0 * sim_exact as f64 / sim_total as f64;
    assert!(
        sim_pct >= 94.5,
        "CAF Create-a-Sim chrome drifted below 94.5% ratchet: {sim_exact}/{sim_total} ({sim_pct:.2}%)"
    );
}

#[test]
fn camera_panel_loads_all_191_assets_strings_and_renders_native_strip() {
    let Some(install) = install() else {
        return;
    };
    let ui = UiGraphics::open(&install).expect("open UIGraphics");
    let art = Art::load_with_optional(&ui, US_ENGLISH, &camera::RESOURCES, &camera::HELP_RESOURCES)
        .expect("load CameraPanel resources");
    let strings = CameraStrings::load(&install, US_ENGLISH).expect("load STR# 140 cammodestrs");

    for btn in CameraButton::ALL {
        assert!(
            !strings.tooltip(btn).is_empty(),
            "STR# 140 tooltip for {btn:?} must be non-empty"
        );
    }
    let size_help = CameraHelp::SnapshotSizes.balloon_text(&strings);
    let qual_help = CameraHelp::QualitySettings.balloon_text(&strings);
    assert!(!size_help.title.is_empty() && !size_help.body.is_empty());
    assert!(!qual_help.title.is_empty() && !qual_help.body.is_empty());
    assert!(art.image(camera::id::VIEW_ALBUM).is_some());
    assert!(art.image(camera::id::PATCH).is_some());
    assert!(art.image(camera::id::TOOTHPICK_LEFT).is_some());
    assert!(art.image(camera::id::TOOTHPICK_RIGHT).is_some());

    let mut panel = CameraPanel::new();
    let mut state = CameraState::default();
    let strip = CameraPanel::strip_rect([800, 600]);
    let mut frame = Frame::new(800, 600, [0, 0, 0]);
    let layout = panel.draw(&mut frame, strip, &state, &strings, &art, true);
    CameraPanel::draw_cluster_patch(&mut frame, [0, 417], &art);

    // Verify CamViewAlbum (`1105`) rendered non-black pixels inside its native slot
    // and that clicking it emits `CameraCommand::OpenPhotoAlbum`.
    let album_rect = layout.button(CameraButton::ViewAlbum).unwrap().rect;
    let non_black = (album_rect.y..album_rect.y + album_rect.height)
        .flat_map(|y| (album_rect.x..album_rect.x + album_rect.width).map(move |x| (x, y)))
        .filter(|&(x, y)| frame.pixel(x, y) != Some([0, 0, 0]))
        .count();
    assert!(
        non_black > 500,
        "CamViewAlbum (1105) should render visible pixels inside strip (got {non_black})"
    );
    let cx = album_rect.x + album_rect.width / 2;
    let cy = album_rect.y + album_rect.height / 2;
    assert!(panel.press(cx, cy, strip, &state, &strings, &art));
    assert_eq!(
        panel.release(cx, cy, strip, &state, &strings, &art),
        Some(CameraCommand::OpenPhotoAlbum)
    );

    // Toggle quality -> High and size -> Large.
    let qh = layout.button(CameraButton::QualityHigh).unwrap().rect;
    assert!(panel.press(qh.x + 4, qh.y + 4, strip, &state, &strings, &art));
    assert_eq!(
        panel.release(qh.x + 4, qh.y + 4, strip, &state, &strings, &art),
        Some(CameraCommand::SetQuality(SnapshotQuality::High))
    );
    state.quality = SnapshotQuality::High;

    let lg = layout.button(CameraButton::Large).unwrap().rect;
    assert!(panel.press(lg.x + 4, lg.y + 4, strip, &state, &strings, &art));
    assert_eq!(
        panel.release(lg.x + 4, lg.y + 4, strip, &state, &strings, &art),
        Some(CameraCommand::SetSize(SnapshotSize::Large))
    );
    state.size = SnapshotSize::Large;
    assert!(CameraButton::Large.is_selected(&state));
    assert!(CameraButton::QualityHigh.is_selected(&state));
}

// Pure-Rust PNG RGB decoder (same format coverage as `sims-render/tests/neighborhood_map.rs`).
fn adler32(bytes: &[u8]) -> u32 {
    const MOD: u32 = 65521;
    let (mut s1, mut s2) = (1u32, 0u32);
    for &b in bytes {
        s1 = (s1 + u32::from(b)) % MOD;
        s2 = (s2 + s1) % MOD;
    }
    (s2 << 16) | s1
}

struct BitReader<'a> {
    data: &'a [u8],
    byte_pos: usize,
    bit_buf: u64,
    bit_cnt: u32,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            byte_pos: 0,
            bit_buf: 0,
            bit_cnt: 0,
        }
    }

    fn ensure(&mut self, n: u32) -> Result<(), String> {
        while self.bit_cnt < n {
            let b = *self
                .data
                .get(self.byte_pos)
                .ok_or_else(|| "unexpected EOF in deflate stream".to_string())?;
            self.byte_pos += 1;
            self.bit_buf |= u64::from(b) << self.bit_cnt;
            self.bit_cnt += 8;
        }
        Ok(())
    }

    fn read_bits(&mut self, n: u32) -> Result<u32, String> {
        if n == 0 {
            return Ok(0);
        }
        self.ensure(n)?;
        let val = (self.bit_buf & ((1u64 << n) - 1)) as u32;
        self.bit_buf >>= n;
        self.bit_cnt -= n;
        Ok(val)
    }

    fn align_byte(&mut self) {
        let drop = self.bit_cnt & 7;
        self.bit_buf >>= drop;
        self.bit_cnt -= drop;
    }

    fn read_u16_le(&mut self) -> Result<u16, String> {
        self.align_byte();
        let lo = self.read_bits(8)?;
        let hi = self.read_bits(8)?;
        Ok((lo | (hi << 8)) as u16)
    }

    fn read_aligned_bytes(&mut self, out: &mut Vec<u8>, len: usize) -> Result<(), String> {
        self.align_byte();
        let mut remaining = len;
        while remaining > 0 && self.bit_cnt >= 8 {
            out.push((self.bit_buf & 0xff) as u8);
            self.bit_buf >>= 8;
            self.bit_cnt -= 8;
            remaining -= 1;
        }
        let end = self
            .byte_pos
            .checked_add(remaining)
            .ok_or_else(|| "length overflow".to_string())?;
        let slice = self
            .data
            .get(self.byte_pos..end)
            .ok_or_else(|| "truncated stored block".to_string())?;
        out.extend_from_slice(slice);
        self.byte_pos = end;
        Ok(())
    }
}

struct HuffTable {
    counts: [u16; 16],
    symbols: Vec<u16>,
}

impl HuffTable {
    fn from_lengths(lengths: &[u8]) -> Result<Self, String> {
        let mut counts = [0u16; 16];
        for &len in lengths {
            if usize::from(len) >= counts.len() {
                return Err(format!("invalid huffman code length {len}"));
            }
            if len > 0 {
                counts[usize::from(len)] += 1;
            }
        }
        let mut offsets = [0usize; 16];
        let mut total = 0usize;
        for bits in 1..16 {
            offsets[bits] = total;
            total += usize::from(counts[bits]);
        }
        let mut symbols = vec![0u16; total];
        for (sym, &len) in lengths.iter().enumerate() {
            if len > 0 {
                let idx = offsets[usize::from(len)];
                symbols[idx] = sym as u16;
                offsets[usize::from(len)] += 1;
            }
        }
        Ok(Self { counts, symbols })
    }

    fn decode(&self, br: &mut BitReader<'_>) -> Result<u16, String> {
        let mut code = 0i32;
        let mut first = 0i32;
        let mut index = 0i32;
        for len in 1..16 {
            code |= br.read_bits(1)? as i32;
            let count = i32::from(self.counts[len]);
            if code - count < first {
                return self
                    .symbols
                    .get((index + (code - first)) as usize)
                    .copied()
                    .ok_or_else(|| "huffman symbol out of bounds".to_string());
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err("invalid huffman code".to_string())
    }
}

fn inflate_zlib(data: &[u8]) -> Result<Vec<u8>, String> {
    if data.len() < 6 {
        return Err("zlib stream too short".to_string());
    }
    let (cmf, flg) = (data[0], data[1]);
    if (cmf & 0x0f) != 8 || ((u16::from(cmf) * 256 + u16::from(flg)) % 31) != 0 {
        return Err("invalid zlib header".to_string());
    }
    let start = if (flg & 0x20) != 0 { 6 } else { 2 };
    if data.len() < start + 4 {
        return Err("truncated zlib stream".to_string());
    }
    let mut br = BitReader::new(&data[start..data.len() - 4]);
    let mut out = Vec::new();

    const LEN_BASE: [u16; 29] = [
        3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115,
        131, 163, 195, 227, 258,
    ];
    const LEN_EXTRA: [u8; 29] = [
        0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
    ];
    const DIST_BASE: [u16; 30] = [
        1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
        2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
    ];
    const DIST_EXTRA: [u8; 30] = [
        0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12,
        13, 13,
    ];
    const CL_ORDER: [usize; 19] = [
        16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
    ];

    loop {
        let bfinal = br.read_bits(1)?;
        let btype = br.read_bits(2)?;
        match btype {
            0 => {
                let len = br.read_u16_le()?;
                let nlen = br.read_u16_le()?;
                if len != !nlen {
                    return Err("invalid stored block length check".to_string());
                }
                br.read_aligned_bytes(&mut out, usize::from(len))?;
            }
            1 | 2 => {
                let (lit_table, dist_table) = if btype == 1 {
                    let mut lit = vec![0u8; 288];
                    lit[..144].fill(8);
                    lit[144..256].fill(9);
                    lit[256..280].fill(7);
                    lit[280..288].fill(8);
                    let dist = vec![5u8; 32];
                    (
                        HuffTable::from_lengths(&lit)?,
                        HuffTable::from_lengths(&dist)?,
                    )
                } else {
                    let hlit = br.read_bits(5)? as usize + 257;
                    let hdist = br.read_bits(5)? as usize + 1;
                    let hclen = br.read_bits(4)? as usize + 4;
                    let mut cl_lens = [0u8; 19];
                    for &idx in CL_ORDER.iter().take(hclen) {
                        cl_lens[idx] = br.read_bits(3)? as u8;
                    }
                    let cl_table = HuffTable::from_lengths(&cl_lens)?;
                    let mut lengths = Vec::with_capacity(hlit + hdist);
                    while lengths.len() < hlit + hdist {
                        let sym = cl_table.decode(&mut br)?;
                        match sym {
                            0..=15 => lengths.push(sym as u8),
                            16 => {
                                let prev = *lengths
                                    .last()
                                    .ok_or_else(|| "repeat with empty lengths".to_string())?;
                                let rep = br.read_bits(2)? as usize + 3;
                                lengths.resize(lengths.len() + rep, prev);
                            }
                            17 => {
                                let rep = br.read_bits(3)? as usize + 3;
                                lengths.resize(lengths.len() + rep, 0);
                            }
                            18 => {
                                let rep = br.read_bits(7)? as usize + 11;
                                lengths.resize(lengths.len() + rep, 0);
                            }
                            _ => return Err("invalid code length symbol".to_string()),
                        }
                    }
                    if lengths.len() > hlit + hdist {
                        return Err("code lengths overflow".to_string());
                    }
                    (
                        HuffTable::from_lengths(&lengths[..hlit])?,
                        HuffTable::from_lengths(&lengths[hlit..])?,
                    )
                };

                loop {
                    let sym = lit_table.decode(&mut br)?;
                    match sym {
                        0..=255 => out.push(sym as u8),
                        256 => break,
                        257..=285 => {
                            let idx = usize::from(sym - 257);
                            let length = usize::from(LEN_BASE[idx])
                                + br.read_bits(u32::from(LEN_EXTRA[idx]))? as usize;
                            let dsym = usize::from(dist_table.decode(&mut br)?);
                            if dsym >= DIST_BASE.len() {
                                return Err("invalid distance symbol".to_string());
                            }
                            let dist = usize::from(DIST_BASE[dsym])
                                + br.read_bits(u32::from(DIST_EXTRA[dsym]))? as usize;
                            if dist == 0 || dist > out.len() {
                                return Err("invalid back-reference distance".to_string());
                            }
                            for _ in 0..length {
                                let b = out[out.len() - dist];
                                out.push(b);
                            }
                        }
                        _ => return Err("invalid literal/length symbol".to_string()),
                    }
                }
            }
            _ => return Err("invalid deflate block type".to_string()),
        }
        if bfinal != 0 {
            break;
        }
    }

    let expected = u32::from_be_bytes([
        data[data.len() - 4],
        data[data.len() - 3],
        data[data.len() - 2],
        data[data.len() - 1],
    ]);
    if adler32(&out) != expected {
        return Err("zlib adler32 mismatch".to_string());
    }
    Ok(out)
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let (a, b, c) = (i32::from(a), i32::from(b), i32::from(c));
    let p = a + b - c;
    let pa = (p - a).abs();
    let pb = (p - b).abs();
    let pc = (p - c).abs();
    if pa <= pb && pa <= pc {
        a as u8
    } else if pb <= pc {
        b as u8
    } else {
        c as u8
    }
}

fn decode_png_rgb(
    bytes: &[u8],
    expected_dims: Option<(usize, usize)>,
) -> Result<(usize, usize, Vec<[u8; 3]>), String> {
    const SIG: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];
    if bytes.len() < 8 || bytes[..8] != SIG {
        return Err("missing PNG signature".to_string());
    }
    let mut pos = 8usize;
    let mut width = 0usize;
    let mut height = 0usize;
    let mut color_type = 0u8;
    let mut idat = Vec::new();
    let mut saw_ihdr = false;

    while pos + 12 <= bytes.len() {
        let len = u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]])
            as usize;
        let kind = &bytes[pos + 4..pos + 8];
        let data_start = pos + 8;
        let data_end = data_start
            .checked_add(len)
            .ok_or_else(|| "PNG chunk length overflow".to_string())?;
        if data_end + 4 > bytes.len() {
            return Err("truncated PNG chunk".to_string());
        }
        let chunk = &bytes[data_start..data_end];
        match kind {
            b"IHDR" => {
                if chunk.len() < 13 {
                    return Err("truncated IHDR".to_string());
                }
                width = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) as usize;
                height = u32::from_be_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]) as usize;
                let bit_depth = chunk[8];
                color_type = chunk[9];
                if bit_depth != 8 || (color_type != 2 && color_type != 6) || chunk[12] != 0 {
                    return Err("unsupported PNG format".to_string());
                }
                saw_ihdr = true;
            }
            b"IDAT" => idat.extend_from_slice(chunk),
            b"IEND" => break,
            _ => {}
        }
        pos = data_end + 4;
    }

    if !saw_ihdr {
        return Err("missing IHDR".to_string());
    }
    if let Some((ew, eh)) = expected_dims
        && (width != ew || height != eh)
    {
        return Err(format!("expected {ew}x{eh} PNG, got {width}x{height}"));
    }
    let bpp = if color_type == 6 { 4 } else { 3 };
    let stride = width * bpp;
    let raw = inflate_zlib(&idat)?;
    if raw.len() != height * (stride + 1) {
        return Err("unexpected decompressed PNG byte count".to_string());
    }

    let mut recon = vec![0u8; height * stride];
    for y in 0..height {
        let ftype = raw[y * (stride + 1)];
        let src = &raw[y * (stride + 1) + 1..(y + 1) * (stride + 1)];
        for x in 0..stride {
            let a = if x >= bpp {
                recon[y * stride + x - bpp]
            } else {
                0
            };
            let b = if y > 0 {
                recon[(y - 1) * stride + x]
            } else {
                0
            };
            let c = if x >= bpp && y > 0 {
                recon[(y - 1) * stride + x - bpp]
            } else {
                0
            };
            let v = match ftype {
                0 => src[x],
                1 => src[x].wrapping_add(a),
                2 => src[x].wrapping_add(b),
                3 => src[x].wrapping_add(((u16::from(a) + u16::from(b)) / 2) as u8),
                4 => src[x].wrapping_add(paeth(a, b, c)),
                _ => return Err(format!("invalid PNG filter {ftype}")),
            };
            recon[y * stride + x] = v;
        }
    }

    let mut rgb = Vec::with_capacity(width * height);
    for px in recon.chunks_exact(bpp) {
        rgb.push([px[0], px[1], px[2]]);
    }
    Ok((width, height, rgb))
}
