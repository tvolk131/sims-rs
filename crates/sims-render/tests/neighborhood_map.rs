//! Integration tests for the Complete Collection neighborhood map renderer
//! (`crates/sims-render/src/neighborhood.rs`) against a real `SIMS_INSTALL`.

use sha2::{Digest, Sha256};
use sims_content::Install;
use sims_render::neighborhood::{
    COLOR_KEY_565, CommunityFilter, DEFAULT_HIGHLIGHT_RGB, LotStatus, LotZoning, MAP_HEIGHT,
    MAP_WIDTH, NeighborhoodKind, NeighborhoodMap, RenderOptions, map_origin,
};
use std::collections::BTreeSet;
use std::path::PathBuf;

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
    // Goth family occupies lot 5; Old Town community lots 58, 61, 70..=75 are Community.
    assert!(matches!(
        map.lot(5).unwrap().status,
        LotStatus::Occupied { .. }
    ));
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
    // Outer frame area should be non-black (LargeBack.bmp drawn around map).
    assert!(
        c1024.color[0..1024 * 80].iter().any(|&c| c != [0, 0, 0]),
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
    // Outer border outside map clip never hits a lot.
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
    let expected_outline = sims_render::canvas::quantize565(DEFAULT_HIGHLIGHT_RGB);
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

    // Highlighting a set of lots without cutaway also changes pixels.
    let mut set = BTreeSet::new();
    set.insert(70);
    let selected = map.render(&RenderOptions {
        highlighted_lots: set,
        ..RenderOptions::default()
    });
    assert_ne!(selected.color, base.color);

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

    // Neighborhoods 1..=8 Unleashed maps all load and render cleanly.
    for n in 1..=8u8 {
        let map = NeighborhoodMap::load(&install, n, NeighborhoodKind::Unleashed, false)
            .unwrap_or_else(|e| panic!("Neighborhood {n} failed to load: {e}"));
        assert_eq!(map.lots.len(), 41, "Neighborhood {n} lot count");
        let canvas = map.render(&RenderOptions::default());
        assert_eq!((canvas.width, canvas.height), (800, 600));
    }

    // All 5 other map kinds load from Neighborhood 1 with exact native lot counts.
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
        // Spot-check first and last lot hit-testing on every sub-neighborhood.
        let first_id = map.lots.first().unwrap().id;
        let last_id = map.lots.last().unwrap().id;
        let p_first = opaque_screen_point(&map, first_id, [800, 600]);
        let p_last = opaque_screen_point(&map, last_id, [800, 600]);
        assert_eq!(canvas_800.id_at(p_first[0], p_first[1]), Some(first_id));
        assert_eq!(canvas_800.id_at(p_last[0], p_last[1]), Some(last_id));
    }

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
    let (ow, oh, oracle_rgb) = decode_png_rgb(&oracle_bytes);
    assert_eq!((ow, oh), (800, 600));

    let mut lot_total = 0usize;
    let mut lot_exact = 0usize;
    let mut all_total = 0usize;
    let mut all_exact = 0usize;
    for y in 80..600usize {
        for x in 0..800usize {
            let i = y * 800 + x;
            let got = sims_render::neighborhood::pack_565(canvas.color[i]);
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
    assert_eq!(
        lot_exact, lot_total,
        "lot pixel accuracy {lot_exact}/{lot_total} must be 100% exact"
    );
    assert!(
        all_exact * 1000 >= all_total * 995,
        "overall map accuracy {all_exact}/{all_total} below 99.5%"
    );
}

fn decode_png_rgb(data: &[u8]) -> (usize, usize, Vec<[u8; 3]>) {
    assert_eq!(&data[0..8], b"\x89PNG\r\n\x1a\n");
    let mut pos = 8usize;
    let (mut w, mut h, mut color_type) = (0usize, 0usize, 0u8);
    let mut idat = Vec::new();
    while pos + 12 <= data.len() {
        let len = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        let tag = &data[pos + 4..pos + 8];
        let body = &data[pos + 8..pos + 8 + len];
        pos += 12 + len;
        if tag == b"IHDR" {
            w = u32::from_be_bytes(body[0..4].try_into().unwrap()) as usize;
            h = u32::from_be_bytes(body[4..8].try_into().unwrap()) as usize;
            color_type = body[9];
        } else if tag == b"IDAT" {
            idat.extend_from_slice(body);
        } else if tag == b"IEND" {
            break;
        }
    }
    let raw = inflate_zlib(&idat);
    let bpp = match color_type {
        2 => 3usize,
        6 => 4usize,
        other => panic!("unsupported PNG color type {other}"),
    };
    let stride = w * bpp;
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
                other => panic!("unsupported PNG filter {other}"),
            };
            cur[i] = src[i].wrapping_add(pred);
        }
        recon[y * stride..(y + 1) * stride].copy_from_slice(&cur);
    }
    let rgb = recon
        .chunks_exact(bpp)
        .map(|p| [p[0], p[1], p[2]])
        .collect();
    (w, h, rgb)
}

fn inflate_zlib(data: &[u8]) -> Vec<u8> {
    // Minimal RFC 1950 + RFC 1951 inflate using python3? No, pure Rust!
    // Wait: let's implement a tiny RFC 1951 fixed/dynamic Huffman inflater right here.
    assert!(data.len() >= 6);
    assert_eq!(data[0] & 0x0f, 8, "deflate method");
    let mut r = BitReader {
        bytes: &data[2..data.len() - 4],
        bit_pos: 0,
    };
    let mut out = Vec::new();
    loop {
        let bfinal = r.bits(1);
        let btype = r.bits(2);
        match btype {
            0 => {
                r.align_byte();
                let len = r.bits(16) as usize;
                let _nlen = r.bits(16);
                let byte_pos = r.bit_pos / 8;
                out.extend_from_slice(&r.bytes[byte_pos..byte_pos + len]);
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
                    let hlit = r.bits(5) as usize + 257;
                    let hdist = r.bits(5) as usize + 1;
                    let hclen = r.bits(4) as usize + 4;
                    let order = [
                        16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
                    ];
                    let mut cl = [0u8; 19];
                    for i in 0..hclen {
                        cl[order[i]] = r.bits(3) as u8;
                    }
                    let cl_tree = Huff::build(&cl);
                    let mut lengths = Vec::with_capacity(hlit + hdist);
                    while lengths.len() < hlit + hdist {
                        let sym = cl_tree.decode(&mut r);
                        match sym {
                            0..=15 => lengths.push(sym as u8),
                            16 => {
                                let rep = r.bits(2) as usize + 3;
                                let prev = *lengths.last().unwrap();
                                lengths.resize(lengths.len() + rep, prev);
                            }
                            17 => {
                                let rep = r.bits(3) as usize + 3;
                                lengths.resize(lengths.len() + rep, 0);
                            }
                            18 => {
                                let rep = r.bits(7) as usize + 11;
                                lengths.resize(lengths.len() + rep, 0);
                            }
                            _ => unreachable!(),
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
                    let sym = lit_tree.decode(&mut r);
                    if sym < 256 {
                        out.push(sym as u8);
                    } else if sym == 256 {
                        break;
                    } else {
                        let li = sym - 257;
                        let length = LEN_BASE[li] + r.bits(LEN_EX[li]) as usize;
                        let di = dist_tree.decode(&mut r);
                        let dist = DIST_BASE[di] + r.bits(DIST_EX[di]) as usize;
                        for _ in 0..length {
                            let b = out[out.len() - dist];
                            out.push(b);
                        }
                    }
                }
            }
            _ => panic!("invalid deflate block type"),
        }
        if bfinal == 1 {
            break;
        }
    }
    out
}

struct BitReader<'a> {
    bytes: &'a [u8],
    bit_pos: usize,
}

impl BitReader<'_> {
    fn bits(&mut self, n: u8) -> u32 {
        let mut v = 0u32;
        for i in 0..n {
            let byte = self.bytes[self.bit_pos / 8];
            let bit = (byte >> (self.bit_pos % 8)) & 1;
            v |= u32::from(bit) << i;
            self.bit_pos += 1;
        }
        v
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

    fn decode(&self, r: &mut BitReader<'_>) -> usize {
        let mut code = 0i32;
        let mut first = 0i32;
        let mut index = 0i32;
        for len in 1..16 {
            code |= r.bits(1) as i32;
            let count = i32::from(self.counts[len]);
            if code - count < first {
                return usize::from(self.symbols[(index + (code - first)) as usize]);
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        panic!("invalid Huffman code");
    }
}
