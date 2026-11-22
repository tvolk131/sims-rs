//! Integration tests for the Complete Collection neighborhood map renderer
//! (`crates/sims-render/src/neighborhood.rs`) against a real `SIMS_INSTALL`.

use sha2::{Digest, Sha256};
use sims_content::Install;
use sims_render::neighborhood::{
    COLOR_KEY_565, CommunityFilter, DEFAULT_HIGHLIGHT_RGB, LotStatus, LotZoning, MAP_HEIGHT,
    MAP_WIDTH, NeighborhoodKind, NeighborhoodMap, RenderOptions, Surface565, map_origin, pack_565,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

#[test]
fn neighborhood_map_supports_external_struct_literals() {
    let map = NeighborhoodMap {
        kind: NeighborhoodKind::Unleashed,
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
        lots: Vec::new(),
    };
    let canvas = map.render(&RenderOptions::default());
    assert_eq!((canvas.width, canvas.height), (800, 600));
}

fn install() -> Option<Install> {
    let root = PathBuf::from(std::env::var_os("SIMS_INSTALL")?);
    Some(
        Install::detect(&root).expect("SIMS_INSTALL should be a valid Complete Collection install"),
    )
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Finds a guaranteed opaque pixel inside `lot` and returns its screen coords
/// in `viewport`.
fn opaque_screen_point(map: &NeighborhoodMap, lot_id: u16, viewport: [u32; 2]) -> [i32; 2] {
    let lot = map.lot(lot_id).expect("lot should exist");
    let [ox, oy] = map_origin(viewport);
    for y in 0..lot.normal.height {
        for x in 0..lot.normal.width {
            if lot.normal.pixels[y * lot.normal.width + x] != COLOR_KEY_565 {
                let sx = ox + lot.cell_origin[0] + x as i32;
                let sy = oy + lot.cell_origin[1] + y as i32;
                if sx >= ox
                    && sx < ox + MAP_WIDTH as i32
                    && sy >= oy
                    && sy < oy + MAP_HEIGHT as i32
                    && map.hit_test(viewport, [sx, sy]) == Some(lot_id)
                {
                    return [sx, sy];
                }
            }
        }
    }
    panic!("lot {lot_id} has no visible hit-testable pixel");
}

#[test]
fn neighborhood_1_unleashed_renders_800x600_and_1024x768_and_hits_all_41_lots() {
    let Some(install) = install() else {
        return;
    };
    let map = NeighborhoodMap::load(&install, 1, NeighborhoodKind::Unleashed, false)
        .expect("load Neighborhood 1 Unleashed map");

    // Complete Collection Neighborhood 1 has 10 base lots (1..=10) + 31 Unleashed lots (50..=80).
    assert_eq!(map.lots.len(), 41);
    for id in 1..=10u16 {
        let lot = map.lot(id).expect("base lot present");
        assert_eq!(lot.zoning, LotZoning::Residential);
    }
    for id in 50..=80u16 {
        assert!(map.lot(id).is_some(), "Unleashed lot {id} present");
    }
    // Goth family occupies lot 5 with FAMI chunk ID 5 (0x410673 / 0x4e1e73).
    assert_eq!(
        map.lot(5).unwrap().status,
        LotStatus::Occupied {
            family_id: 5,
            vacation_lot: None,
        }
    );
    assert!(matches!(
        map.lot(58).unwrap().status,
        LotStatus::Community { .. }
    ));
    assert_ne!(map.lot(70).unwrap().filter_mask, 0);

    // Render 800×600 with wave frame 1 (waves002.bmp) and verify deterministic output.
    let c800 = map.render(&RenderOptions {
        viewport: [800, 600],
        water_frame: Some(1),
        ..RenderOptions::default()
    });
    assert_eq!((c800.width, c800.height), (800, 600));
    let digest_800 = sha256_hex(&c800.rgba());
    let c800_repeat = map.render(&RenderOptions {
        viewport: [800, 600],
        water_frame: Some(1),
        ..RenderOptions::default()
    });
    assert_eq!(digest_800, sha256_hex(&c800_repeat.rgba()));

    // Render 1024×768 with outer LargeBack frame and verify the centered 800×600
    // interior matches c800 pixel-for-pixel.
    let c1024 = map.render(&RenderOptions {
        viewport: [1024, 768],
        water_frame: Some(1),
        ..RenderOptions::default()
    });
    assert_eq!((c1024.width, c1024.height), (1024, 768));
    assert!(
        c1024.color[0..1024 * 80].iter().any(|&c| c != 0),
        "1024×768 outer frame should be drawn"
    );
    let [ox, oy] = map_origin([1024, 768]);
    assert_eq!([ox, oy], [112, 84]);
    for y in 0..600usize {
        for x in 0..800usize {
            assert_eq!(
                c1024.color[(y + oy as usize) * 1024 + (x + ox as usize)],
                c800.color[y * 800 + x],
                "1024×768 interior mismatch at ({x}, {y})"
            );
        }
    }

    // Every single one of the 41 lots must hit-test cleanly at both viewports
    // and agree with Canvas::id_at.
    for lot in &map.lots {
        let p800 = opaque_screen_point(&map, lot.id, [800, 600]);
        assert_eq!(c800.id_at(p800[0], p800[1]), Some(lot.id));
        let p1024 = [p800[0] + ox, p800[1] + oy];
        assert_eq!(map.hit_test([1024, 768], p1024), Some(lot.id));
        assert_eq!(c1024.id_at(p1024[0], p1024[1]), Some(lot.id));
    }
    // Native unclipped silhouette check: at 1024×768, lot 2 has an opaque pixel
    // at screen (914, 150) beyond the 912 map draw clip (0x5e4071..0x5e40da).
    assert_eq!(map.hit_test([1024, 768], [914, 150]), Some(2));
    assert_eq!(map.hit_test([1024, 768], [10, 10]), None);
    assert_eq!(c1024.id_at(10, 10), None);
}

#[test]
fn hover_cutaway_highlight_and_community_filter_badges_modify_expected_pixels() {
    let Some(install) = install() else {
        return;
    };
    let map = NeighborhoodMap::load(&install, 1, NeighborhoodKind::Unleashed, false)
        .expect("load Neighborhood 1");

    let base = map.render(&RenderOptions::default());

    // Hovering Goth house (lot 5) with cutaway + native yellow highlight changes
    // pixels strictly within lot 5's screen rectangle.
    let hovered = map.render(&RenderOptions {
        hovered_lot: Some(5),
        hover_roofs_off: true,
        highlight_hovered: true,
        ..RenderOptions::default()
    });
    let rect5 = map.lot(5).unwrap().screen_rect([800, 600]);
    let mut changed_in_rect = 0usize;
    let mut changed_outside = 0usize;
    let mut outline_pixels = 0usize;
    let expected_outline =
        sims_render::pack_rgb(sims_render::canvas::quantize565(DEFAULT_HIGHLIGHT_RGB));
    for y in 0..600i32 {
        for x in 0..800i32 {
            let idx = y as usize * 800 + x as usize;
            if hovered.color[idx] != base.color[idx] {
                if x >= rect5[0] && x < rect5[2] && y >= rect5[1] && y < rect5[3] {
                    changed_in_rect += 1;
                    if hovered.color[idx] == expected_outline {
                        outline_pixels += 1;
                    }
                } else {
                    changed_outside += 1;
                }
            }
        }
    }
    assert!(
        changed_in_rect > 500,
        "expected Goth house pixels to change"
    );
    assert!(
        outline_pixels > 50,
        "expected native yellow edge outline pixels"
    );
    assert_eq!(
        changed_outside, 0,
        "hover must not alter pixels outside lot 5"
    );

    // Highlighting stock Unleashed Lot 1 across its full two-cell strip
    // (0x54f347..0x54f3e1) outlines the cell-0/cell-1 boundary pixel at
    // (559, 83) with native yellow 5-6-5 0xf7e3.
    let mut lot1_set = BTreeSet::new();
    lot1_set.insert(1);
    let lot1_hi = map.render(&RenderOptions {
        highlighted_lots: lot1_set,
        ..RenderOptions::default()
    });
    assert_eq!(
        sims_render::neighborhood::pack_565(lot1_hi.pixel_rgb(559, 83)),
        0xf7e3,
        "Unleashed Lot 1 two-cell strip boundary at (559, 83) must be outlined"
    );

    // Highlighting a set of lots without cutaway also changes pixels.
    let mut set = BTreeSet::new();
    set.insert(70);
    let selected = map.render(&RenderOptions {
        highlighted_lots: set,
        ..RenderOptions::default()
    });
    assert_ne!(selected.color, base.color);

    // Community filter (Lodging = 0x01) on Unleashed applies native blue
    // highlight (0x54f520) to ALL Community lots (including House74 whose mask
    // is 0x0002, not Lodging) even when no badge is drawn on House74.
    let lodging = map.render(&RenderOptions {
        active_filter: Some(CommunityFilter::Lodging),
        filter_frame: 0,
        ..RenderOptions::default()
    });
    let p74 = opaque_screen_point(&map, 74, [800, 600]);
    let idx74 = p74[1] as usize * 800 + p74[0] as usize;
    assert_ne!(
        lodging.color[idx74], base.color[idx74],
        "Unleashed community lot 74 must receive blue community highlight"
    );

    // Community filter badge (Food = 0x02) draws over matching community lots
    // (such as Old Town lot 70) and animates across frames 0..3.
    let filtered_0 = map.render(&RenderOptions {
        active_filter: Some(CommunityFilter::Food),
        filter_frame: 0,
        ..RenderOptions::default()
    });
    let filtered_2 = map.render(&RenderOptions {
        active_filter: Some(CommunityFilter::Food),
        filter_frame: 2,
        ..RenderOptions::default()
    });
    assert_ne!(filtered_0.color, base.color);
    assert_ne!(filtered_0.color, filtered_2.color);

    // Occupancy status markers draw over residential lots.
    let marked = map.render(&RenderOptions {
        show_occupancy_markers: true,
        ..RenderOptions::default()
    });
    assert_ne!(marked.color, base.color);
}

#[test]
fn all_eight_neighborhoods_and_all_sub_neighborhood_maps_load_and_render() {
    let Some(install) = install() else {
        return;
    };

    for n in 1..=8u8 {
        let map = NeighborhoodMap::load(&install, n, NeighborhoodKind::Unleashed, false)
            .unwrap_or_else(|e| panic!("Neighborhood {n} failed to load: {e}"));
        assert_eq!(map.lots.len(), 41, "Neighborhood {n} lot count");
        let canvas = map.render(&RenderOptions::default());
        assert_eq!((canvas.width, canvas.height), (800, 600));
    }

    let cases = [
        (NeighborhoodKind::Classic, 10usize),
        (NeighborhoodKind::Downtown, 10),
        (NeighborhoodKind::Vacation, 9),
        (NeighborhoodKind::StudioTown, 9),
        (NeighborhoodKind::MagicTown, 9),
    ];
    for (kind, expected_lots) in cases {
        let map = NeighborhoodMap::load(&install, 1, kind, false)
            .unwrap_or_else(|e| panic!("{kind:?} failed to load: {e}"));
        assert_eq!(map.lots.len(), expected_lots, "{kind:?} lot count");
        let canvas_800 = map.render(&RenderOptions {
            viewport: [800, 600],
            water_frame: Some(0),
            ..RenderOptions::default()
        });
        assert_eq!((canvas_800.width, canvas_800.height), (800, 600));
        let canvas_1024 = map.render(&RenderOptions {
            viewport: [1024, 768],
            water_frame: Some(0),
            ..RenderOptions::default()
        });
        assert_eq!((canvas_1024.width, canvas_1024.height), (1024, 768));
        let first_id = map.lots.first().unwrap().id;
        let last_id = map.lots.last().unwrap().id;
        let p_first = opaque_screen_point(&map, first_id, [800, 600]);
        let p_last = opaque_screen_point(&map, last_id, [800, 600]);
        assert_eq!(canvas_800.id_at(p_first[0], p_first[1]), Some(first_id));
        assert_eq!(canvas_800.id_at(p_last[0], p_last[1]), Some(last_id));
    }

    // Verify Vacation post-lot wave order (0x56baeb..0x56bbb7): at (179, 559),
    // wave frame 4 (5414) draws over port & trees yielding 5-6-5 0x3a4d.
    let vac = NeighborhoodMap::load(&install, 1, NeighborhoodKind::Vacation, false).unwrap();
    let vac_c = vac.render(&RenderOptions {
        viewport: [800, 600],
        water_frame: Some(4),
        ..RenderOptions::default()
    });
    assert_eq!(
        sims_render::neighborhood::pack_565(vac_c.pixel_rgb(179, 559)),
        0x3a4d,
        "Vacation wave frame 5414 should draw after port and trees"
    );

    // Verify Magic Town waves draw at y + 62 (0x542560), so top 62 rows stay
    // identical to the no-wave render.
    let magic = NeighborhoodMap::load(&install, 1, NeighborhoodKind::MagicTown, false).unwrap();
    let magic_no_wave = magic.render(&RenderOptions::default());
    let magic_wave = magic.render(&RenderOptions {
        water_frame: Some(0),
        ..RenderOptions::default()
    });
    assert_eq!(
        &magic_no_wave.color[..62 * 800],
        &magic_wave.color[..62 * 800],
        "Magic Town wave offset (0, 62) must leave rows 0..62 untouched"
    );
    assert_ne!(magic_no_wave.color, magic_wave.color);

    // Non-US Unleashed backdrop loads and differs from US backdrop.
    let us = NeighborhoodMap::load(&install, 1, NeighborhoodKind::Unleashed, false).unwrap();
    let non_us = NeighborhoodMap::load(&install, 1, NeighborhoodKind::Unleashed, true).unwrap();
    assert_ne!(us.backdrop.pixels, non_us.backdrop.pixels);
}

/// Optional pixel-level oracle check against a live Complete Collection 1.91
/// Neighborhood 1 PNG capture (pass `SIMS_HOOD_ORACLE=/path/to/c2-hood.png`).
#[test]
fn neighborhood_1_matches_live_oracle_capture_if_provided() {
    let Some(install) = install() else {
        return;
    };
    let Ok(oracle_path) = std::env::var("SIMS_HOOD_ORACLE") else {
        return;
    };
    let map = NeighborhoodMap::load(&install, 1, NeighborhoodKind::Unleashed, false)
        .expect("load Neighborhood 1");
    let canvas = map.render(&RenderOptions {
        viewport: [800, 600],
        water_frame: Some(1), // NScreen_unleashed_waves002.bmp
        ..RenderOptions::default()
    });
    let oracle_bytes = std::fs::read(&oracle_path).expect("read SIMS_HOOD_ORACLE PNG");
    let (ow, oh, oracle_rgb) =
        decode_png_rgb(&oracle_bytes, Some((800, 600))).expect("valid 800×600 oracle PNG");
    assert_eq!((ow, oh), (800, 600));

    let mut lot_total = 0usize;
    let mut lot_exact = 0usize;
    let mut all_total = 0usize;
    let mut all_exact = 0usize;
    for y in 80..600usize {
        for x in 0..800usize {
            let i = y * 800 + x;
            let got = sims_render::neighborhood::pack_565(sims_render::unpack_rgb(canvas.color[i]));
            let want = sims_render::neighborhood::pack_565(oracle_rgb[i]);
            all_total += 1;
            if got == want {
                all_exact += 1;
            }
            if canvas.id_at(x as i32, y as i32).is_some() {
                lot_total += 1;
                if got == want {
                    lot_exact += 1;
                }
            }
        }
    }
    eprintln!(
        "SIMS_HOOD_ORACLE y=80..600: all={all_exact}/{all_total} ({:.4}%), all 41 lots={lot_exact}/{lot_total} ({:.4}%)",
        100.0 * all_exact as f64 / all_total as f64,
        100.0 * lot_exact as f64 / lot_total as f64,
    );
    // Pin the exact observed lot-pixel denominator so a dropped silhouette
    // pixel cannot shrink the denominator and slip past the 100% check.
    assert_eq!(lot_total, 178_134, "unexpected lot-pixel count");
    assert_eq!(
        lot_exact, lot_total,
        "lot pixel accuracy {lot_exact}/{lot_total} must be 100% exact"
    );
    assert!(
        all_exact * 1000 >= all_total * 995,
        "overall map accuracy {all_exact}/{all_total} below 99.5%"
    );
}

/// Optional pixel-level ratchet against Track 2's real 1.91 Neighborhood 1
/// captures (`01-nb-clean.png` and `02-nb-hover-goth.png` in `SIMS_NB_ORACLE`
/// or `/Users/tommyvolk/Desktop/sims-rs/cache/nb-save-movein-evict-20261121/shots`).
///
/// Verifies:
/// 1. Clean Neighborhood 1 map (`y = 52..600`, all 41 lots) matches `01-nb-clean.png`.
/// 2. Plain hover on Goth Lot 5 with default [`RenderOptions`] (`highlight_hovered: false`,
///    `hover_roof_fade: 1.0`) matches `02-nb-hover-goth.png` with zero yellow outline pixels.
/// 3. [`sims_render::neighborhood::HoverFadeState`] smoothly cross-fades roofs
///    between `normal` and `roofs_off` over 300 ms.
#[test]
fn neighborhood_hover_cutaway_and_crossfade_match_191_oracle() {
    const DEFAULT_NB_ORACLE: &str =
        "/Users/tommyvolk/Desktop/sims-rs/cache/nb-save-movein-evict-20261121/shots";
    let Some(install) = install() else {
        return;
    };
    let oracle_dir = std::env::var_os("SIMS_NB_ORACLE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_NB_ORACLE));
    if !oracle_dir.join("01-nb-clean.png").is_file()
        || !oracle_dir.join("02-nb-hover-goth.png").is_file()
    {
        return;
    }

    let map = NeighborhoodMap::load(&install, 1, NeighborhoodKind::Unleashed, false)
        .expect("load Neighborhood 1");

    // 1. Clean map matches `01-nb-clean.png` across all 41 lot silhouettes.
    let clean_bytes = std::fs::read(oracle_dir.join("01-nb-clean.png")).expect("read clean PNG");
    let (_, _, clean_oracle) =
        decode_png_rgb(&clean_bytes, Some((800, 600))).expect("decode clean");
    let clean_canvas = map.render(&RenderOptions::default());
    let mut clean_lot_exact = 0usize;
    let mut clean_lot_total = 0usize;
    for y in 52..600usize {
        for x in 0..800usize {
            if clean_canvas.id_at(x as i32, y as i32).is_some() {
                clean_lot_total += 1;
                let idx = y * 800 + x;
                if sims_render::neighborhood::pack_565(sims_render::unpack_rgb(
                    clean_canvas.color[idx],
                )) == sims_render::neighborhood::pack_565(clean_oracle[idx])
                {
                    clean_lot_exact += 1;
                }
            }
        }
    }
    assert!(
        clean_lot_total > 170_000,
        "unexpected lot-pixel count {clean_lot_total}"
    );
    assert_eq!(
        clean_lot_exact, clean_lot_total,
        "all 41 lots on clean Neighborhood 1 must match 01-nb-clean.png 100% ({clean_lot_exact}/{clean_lot_total})"
    );

    // 2. Plain hover on Goth Lot 5 uses default `highlight_hovered: false` (zero
    //    yellow outline pixels) and matches `02-nb-hover-goth.png` inside Lot 5's
    //    unobstructed cutaway silhouette (above the Goth popup card at `y = 259`).
    let hover_bytes =
        std::fs::read(oracle_dir.join("02-nb-hover-goth.png")).expect("read hover PNG");
    let (_, _, hover_oracle) =
        decode_png_rgb(&hover_bytes, Some((800, 600))).expect("decode hover");
    let hover_opts = RenderOptions {
        hovered_lot: Some(5),
        ..RenderOptions::default()
    };
    assert!(
        !hover_opts.highlight_hovered,
        "plain hover must default to highlight_hovered = false"
    );
    let hover_canvas = map.render(&hover_opts);
    let rect5 = map.lot(5).unwrap().screen_rect([800, 600]);
    let yellow_565 = sims_render::neighborhood::pack_565(DEFAULT_HIGHLIGHT_RGB);
    let mut added_yellow = 0usize;
    let mut lot5_exact = 0usize;
    let mut lot5_total = 0usize;
    for y in rect5[1].max(52)..rect5[3].min(259) {
        for x in rect5[0].max(0)..rect5[2].min(800) {
            if hover_canvas.id_at(x, y) != Some(5) {
                continue;
            }
            let idx = y as usize * 800 + x as usize;
            let got = sims_render::neighborhood::pack_565(sims_render::unpack_rgb(
                hover_canvas.color[idx],
            ));
            let base = sims_render::neighborhood::pack_565(sims_render::unpack_rgb(
                clean_canvas.color[idx],
            ));
            let want = sims_render::neighborhood::pack_565(hover_oracle[idx]);
            if got == yellow_565 && base != yellow_565 {
                added_yellow += 1;
            }
            lot5_total += 1;
            if got == want {
                lot5_exact += 1;
            }
        }
    }
    assert_eq!(
        added_yellow, 0,
        "default hover on Goth Lot 5 must not add yellow outline pixels"
    );
    assert!(
        lot5_total > 4_000,
        "unexpected Goth Lot 5 cutaway silhouette count {lot5_total}"
    );
    let lot5_pct = 100.0 * lot5_exact as f64 / lot5_total as f64;
    assert!(
        lot5_pct >= 98.5,
        "Goth Lot 5 hover cutaway drifted below 98.5% ratchet vs 02-nb-hover-goth.png: {lot5_exact}/{lot5_total} ({lot5_pct:.2}%)"
    );

    // 3. Smooth 300 ms `HoverFadeState` produces a distinct intermediate cross-fade
    //    frame at t = 150 ms between roof-on (`clean_canvas`) and cutaway (`hover_canvas`).
    let mut fade_state = sims_render::neighborhood::HoverFadeState::new();
    fade_state.set_hovered(Some(5), 1000);
    let mut mid_opts = RenderOptions::default();
    fade_state.apply_to(&mut mid_opts, 1150);
    let mid_canvas = map.render(&mid_opts);
    let mut diff_from_clean = 0usize;
    let mut diff_from_full = 0usize;
    for y in rect5[1].max(52)..rect5[3].min(600) {
        for x in rect5[0].max(0)..rect5[2].min(800) {
            let idx = y as usize * 800 + x as usize;
            if mid_canvas.color[idx] != clean_canvas.color[idx] {
                diff_from_clean += 1;
            }
            if mid_canvas.color[idx] != hover_canvas.color[idx] {
                diff_from_full += 1;
            }
        }
    }
    assert!(
        diff_from_clean > 500 && diff_from_full > 500,
        "150 ms cross-fade must be intermediate between normal and cutaway (diff_clean={diff_from_clean}, diff_full={diff_from_full})"
    );
}

#[test]
fn test_png_decoder_rejects_short_and_corrupted_inputs() {
    assert!(decode_png_rgb(&[0; 7], None).is_err());
    assert!(decode_png_rgb(b"\x89PNG\r\n\x1a\n", None).is_err());

    // Build a valid 1×1 RGB PNG, then flip an Adler-32 bit (with valid outer CRC)
    // and verify the decoder rejects the bad zlib checksum.
    let mut png = Vec::from(&b"\x89PNG\r\n\x1a\n"[..]);
    let push_chunk = |png: &mut Vec<u8>, tag: &[u8; 4], body: &[u8]| {
        png.extend((body.len() as u32).to_be_bytes());
        let start = png.len();
        png.extend(tag);
        png.extend(body);
        let crc = crc32(&png[start..]);
        png.extend(crc.to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend(1u32.to_be_bytes());
    ihdr.extend(1u32.to_be_bytes());
    ihdr.extend([8, 2, 0, 0, 0]);
    push_chunk(&mut png, b"IHDR", &ihdr);
    // Uncompressed zlib block for raw scanline [0, 10, 20, 30]
    let raw = [0u8, 10, 20, 30];
    let mut zlib = vec![0x78, 0x01, 0x01, 4, 0, !4u8, !0u8];
    zlib.extend(raw);
    zlib.extend(adler32(&raw).to_be_bytes());
    let idat_offset = png.len();
    push_chunk(&mut png, b"IDAT", &zlib);
    push_chunk(&mut png, b"IEND", &[]);
    assert!(decode_png_rgb(&png, Some((1, 1))).is_ok());
    assert!(decode_png_rgb(&png, Some((800, 600))).is_err());

    // Flip last bit of Adler-32 and recompute outer IDAT CRC32.
    let adler_last = idat_offset + 8 + zlib.len() - 1;
    png[adler_last] ^= 1;
    let new_crc = crc32(&png[idat_offset + 4..idat_offset + 8 + zlib.len()]);
    png[idat_offset + 8 + zlib.len()..idat_offset + 12 + zlib.len()]
        .copy_from_slice(&new_crc.to_be_bytes());
    assert!(decode_png_rgb(&png, Some((1, 1))).is_err());
}

fn adler32(bytes: &[u8]) -> u32 {
    const MOD: u32 = 65521;
    let (mut s1, mut s2) = (1u32, 0u32);
    for &b in bytes {
        s1 = (s1 + u32::from(b)) % MOD;
        s2 = (s2 + s1) % MOD;
    }
    (s2 << 16) | s1
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn decode_png_rgb(
    data: &[u8],
    expected_dims: Option<(usize, usize)>,
) -> Result<(usize, usize, Vec<[u8; 3]>), String> {
    if data.len() < 8 || &data[0..8] != b"\x89PNG\r\n\x1a\n" {
        return Err("invalid PNG signature".into());
    }
    let mut pos = 8usize;
    let (mut w, mut h, mut color_type) = (0usize, 0usize, 0u8);
    let mut saw_ihdr = false;
    let mut saw_iend = false;
    let mut idat = Vec::new();
    while pos + 12 <= data.len() {
        let len = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        let end = pos
            .checked_add(12)
            .and_then(|n| n.checked_add(len))
            .ok_or_else(|| "PNG chunk length overflow".to_string())?;
        if end > data.len() {
            return Err("truncated PNG chunk".into());
        }
        let tag_and_body = &data[pos + 4..pos + 8 + len];
        let want_crc = u32::from_be_bytes(data[pos + 8 + len..end].try_into().unwrap());
        if crc32(tag_and_body) != want_crc {
            return Err("PNG chunk CRC mismatch".into());
        }
        let tag = &data[pos + 4..pos + 8];
        let body = &data[pos + 8..pos + 8 + len];
        pos = end;
        if tag == b"IHDR" {
            if body.len() != 13 {
                return Err("bad IHDR length".into());
            }
            w = u32::from_be_bytes(body[0..4].try_into().unwrap()) as usize;
            h = u32::from_be_bytes(body[4..8].try_into().unwrap()) as usize;
            let bit_depth = body[8];
            color_type = body[9];
            if w == 0
                || h == 0
                || w.checked_mul(h).is_none_or(|n| n > 4096 * 4096)
                || bit_depth != 8
            {
                return Err(format!("unsupported IHDR {w}×{h} depth={bit_depth}"));
            }
            if let Some(want) = expected_dims
                && (w, h) != want
            {
                return Err(format!(
                    "unexpected PNG dimensions {w}×{h} != {}×{}",
                    want.0, want.1
                ));
            }
            saw_ihdr = true;
        } else if tag == b"IDAT" {
            idat.extend_from_slice(body);
        } else if tag == b"IEND" {
            saw_iend = true;
            break;
        }
    }
    if !saw_ihdr || !saw_iend {
        return Err("missing IHDR or IEND".into());
    }
    let bpp = match color_type {
        2 => 3usize,
        6 => 4usize,
        other => return Err(format!("unsupported PNG color type {other}")),
    };
    let stride = w
        .checked_mul(bpp)
        .ok_or_else(|| "PNG stride overflow".to_string())?;
    let expected_raw = h
        .checked_mul(stride + 1)
        .ok_or_else(|| "PNG buffer overflow".to_string())?;
    let raw = inflate_zlib(&idat, expected_raw)?;
    if raw.len() != expected_raw {
        return Err(format!(
            "inflated PNG size {} != expected {}",
            raw.len(),
            expected_raw
        ));
    }
    let mut recon = vec![0u8; h * stride];
    for y in 0..h {
        let flt = raw[y * (stride + 1)];
        let src = &raw[y * (stride + 1) + 1..(y + 1) * (stride + 1)];
        let mut cur = vec![0u8; stride];
        for i in 0..stride {
            let a = if i >= bpp { cur[i - bpp] } else { 0 };
            let b = if y > 0 {
                recon[(y - 1) * stride + i]
            } else {
                0
            };
            let c = if y > 0 && i >= bpp {
                recon[(y - 1) * stride + i - bpp]
            } else {
                0
            };
            let pred = match flt {
                0 => 0,
                1 => a,
                2 => b,
                3 => ((u16::from(a) + u16::from(b)) >> 1) as u8,
                4 => {
                    let p = i32::from(a) + i32::from(b) - i32::from(c);
                    let pa = (p - i32::from(a)).abs();
                    let pb = (p - i32::from(b)).abs();
                    let pc = (p - i32::from(c)).abs();
                    if pa <= pb && pa <= pc {
                        a
                    } else if pb <= pc {
                        b
                    } else {
                        c
                    }
                }
                other => return Err(format!("unsupported PNG filter {other}")),
            };
            cur[i] = src[i].wrapping_add(pred);
        }
        recon[y * stride..(y + 1) * stride].copy_from_slice(&cur);
    }
    let rgb = recon
        .chunks_exact(bpp)
        .map(|p| [p[0], p[1], p[2]])
        .collect();
    Ok((w, h, rgb))
}

fn inflate_zlib(data: &[u8], max_out: usize) -> Result<Vec<u8>, String> {
    if data.len() < 6 || data[0] & 0x0f != 8 {
        return Err("invalid zlib stream".into());
    }
    let mut r = BitReader {
        bytes: &data[2..data.len() - 4],
        bit_pos: 0,
    };
    let mut out = Vec::with_capacity(max_out.min(1 << 20));
    loop {
        let bfinal = r.bits(1)?;
        let btype = r.bits(2)?;
        match btype {
            0 => {
                r.align_byte();
                let len = r.bits(16)? as usize;
                let _nlen = r.bits(16)?;
                let byte_pos = r.bit_pos / 8;
                let slice = r
                    .bytes
                    .get(byte_pos..byte_pos + len)
                    .ok_or_else(|| "truncated stored block".to_string())?;
                if out.len() + slice.len() > max_out {
                    return Err("zlib output exceeds expected PNG size".into());
                }
                out.extend_from_slice(slice);
                r.bit_pos += len * 8;
            }
            1 | 2 => {
                let (lit_tree, dist_tree) = if btype == 1 {
                    let mut ll = [0u8; 288];
                    ll[..144].fill(8);
                    ll[144..256].fill(9);
                    ll[256..280].fill(7);
                    ll[280..288].fill(8);
                    (Huff::build(&ll), Huff::build(&[5u8; 32]))
                } else {
                    let hlit = r.bits(5)? as usize + 257;
                    let hdist = r.bits(5)? as usize + 1;
                    let hclen = r.bits(4)? as usize + 4;
                    let order = [
                        16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
                    ];
                    let mut cl = [0u8; 19];
                    for i in 0..hclen {
                        cl[order[i]] = r.bits(3)? as u8;
                    }
                    let cl_tree = Huff::build(&cl);
                    let mut lengths = Vec::with_capacity(hlit + hdist);
                    while lengths.len() < hlit + hdist {
                        let sym = cl_tree.decode(&mut r)?;
                        match sym {
                            0..=15 => lengths.push(sym as u8),
                            16 => {
                                let rep = r.bits(2)? as usize + 3;
                                let prev = *lengths
                                    .last()
                                    .ok_or_else(|| "repeat with empty lengths".to_string())?;
                                lengths.resize(lengths.len() + rep, prev);
                            }
                            17 => {
                                let rep = r.bits(3)? as usize + 3;
                                lengths.resize(lengths.len() + rep, 0);
                            }
                            18 => {
                                let rep = r.bits(7)? as usize + 11;
                                lengths.resize(lengths.len() + rep, 0);
                            }
                            _ => return Err("invalid code-length symbol".into()),
                        }
                    }
                    (
                        Huff::build(&lengths[..hlit]),
                        Huff::build(&lengths[hlit..hlit + hdist]),
                    )
                };
                const LEN_BASE: [usize; 29] = [
                    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67,
                    83, 99, 115, 131, 163, 195, 227, 258,
                ];
                const LEN_EX: [u8; 29] = [
                    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5,
                    5, 5, 0,
                ];
                const DIST_BASE: [usize; 30] = [
                    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769,
                    1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
                ];
                const DIST_EX: [u8; 30] = [
                    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11,
                    11, 12, 12, 13, 13,
                ];
                loop {
                    let sym = lit_tree.decode(&mut r)?;
                    if sym < 256 {
                        if out.len() >= max_out {
                            return Err("zlib output exceeds expected PNG size".into());
                        }
                        out.push(sym as u8);
                    } else if sym == 256 {
                        break;
                    } else {
                        let li = sym - 257;
                        if li >= LEN_BASE.len() {
                            return Err("invalid deflate length symbol".into());
                        }
                        let length = LEN_BASE[li] + r.bits(LEN_EX[li])? as usize;
                        let di = dist_tree.decode(&mut r)?;
                        if di >= DIST_BASE.len() {
                            return Err("invalid deflate distance symbol".into());
                        }
                        let dist = DIST_BASE[di] + r.bits(DIST_EX[di])? as usize;
                        if dist == 0 || dist > out.len() || out.len() + length > max_out {
                            return Err("invalid deflate back-reference".into());
                        }
                        for _ in 0..length {
                            let b = out[out.len() - dist];
                            out.push(b);
                        }
                    }
                }
            }
            _ => return Err("invalid deflate block type".into()),
        }
        if bfinal == 1 {
            break;
        }
    }
    let want_adler = u32::from_be_bytes(data[data.len() - 4..].try_into().unwrap());
    if adler32(&out) != want_adler {
        return Err("zlib Adler-32 checksum mismatch".into());
    }
    Ok(out)
}

struct BitReader<'a> {
    bytes: &'a [u8],
    bit_pos: usize,
}

impl BitReader<'_> {
    fn bits(&mut self, n: u8) -> Result<u32, String> {
        let mut v = 0u32;
        for i in 0..n {
            let byte = *self
                .bytes
                .get(self.bit_pos / 8)
                .ok_or_else(|| "truncated deflate stream".to_string())?;
            let bit = (byte >> (self.bit_pos % 8)) & 1;
            v |= u32::from(bit) << i;
            self.bit_pos += 1;
        }
        Ok(v)
    }
    fn align_byte(&mut self) {
        let rem = self.bit_pos % 8;
        if rem != 0 {
            self.bit_pos += 8 - rem;
        }
    }
}

struct Huff {
    counts: [u16; 16],
    symbols: Vec<u16>,
}

impl Huff {
    fn build(lengths: &[u8]) -> Self {
        let mut counts = [0u16; 16];
        for &len in lengths {
            if len > 0 {
                counts[len as usize] += 1;
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
                let o = &mut offsets[len as usize];
                symbols[*o] = sym as u16;
                *o += 1;
            }
        }
        Self { counts, symbols }
    }

    fn decode(&self, r: &mut BitReader<'_>) -> Result<usize, String> {
        let mut code = 0i32;
        let mut first = 0i32;
        let mut index = 0i32;
        for len in 1..16 {
            code |= r.bits(1)? as i32;
            let count = i32::from(self.counts[len]);
            if code - count < first {
                return self
                    .symbols
                    .get((index + (code - first)) as usize)
                    .map(|&s| usize::from(s))
                    .ok_or_else(|| "invalid Huffman symbol index".to_string());
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err("invalid Huffman code".into())
    }
}
