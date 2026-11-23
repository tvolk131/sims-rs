//! `Sims.exe` 1.91 UI oracle tour ratchet.
//!
//! Renders our HUD strips and destination-map chrome headlessly on the Goth
//! house (Lot 5) in the same states as Track 2's 32-PNG oracle tour
//! (`cache/ui-oracle-tour-20261122`, `manifest.json` + `shots/`) and scores
//! each UI region against the real `Sims.exe` 1.91 screenshot.
//!
//! Inputs replay the oracle's own `action_sequence` click coordinates, so a
//! control that moved also costs score. Only UI chrome regions are compared:
//! the lot view behind them differs (camera, clock, Sim positions).
//!
//! The score of a region is the share of its pixels whose largest RGB channel
//! difference from the oracle is at most [`TOLERANCE`]. Each region has a
//! floor in [`FLOORS`]; a score below its floor fails the test, so a fidelity
//! regression cannot slip in. Raise a floor when a fix lifts its score.
//!
//! Needs `SIMS_INSTALL` and `SIMS_UI_ORACLE` (the tour directory); skips
//! cleanly when either is unset, so CI without the install passes.
//! `SIMS_UI_ORACLE_OUT=<dir>` also writes our render and a diff mask per shot.

use flate2::read::ZlibDecoder;
use sims_game::destination_picker::{DestinationMapPicker, DestinationPickerAssets};
use sims_game::game::goth_house;
use sims_game::{Frames, Game, Options};
use sims_render::{Canvas, png};
use sims_ui::command::Mode;
use sims_vm::SpecialPanelRequest;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Largest per-channel difference a pixel may have and still match.
const TOLERANCE: u8 = 40;

/// Region floors: `(shot, region, floor)`. Measured score minus a small
/// margin for platform rounding; ratchet upward only.
const FLOORS: &[(&str, &str, f64)] = &[
    ("live-expanded-needs-800", "puck", 0.98),
    ("live-expanded-needs-800", "strip", 0.99),
    ("live-expanded-needs-1024", "puck", 0.86),
    ("live-expanded-needs-1024", "strip", 0.99),
    ("live-relationships-1024", "strip", 0.95),
    ("live-job-1024", "strip", 0.95),
    ("live-personality-1024", "strip", 0.96),
    ("live-house-1024", "strip", 0.94),
    ("view-roof-on", "view-radio", 0.99),
    ("view-walls-up", "view-radio", 0.99),
    ("view-cutaway", "view-radio", 0.99),
    ("view-walls-down", "view-radio", 0.99),
    ("buy-top-function", "strip", 0.99),
    ("buy-subcat-seating", "strip", 0.98),
    ("build-top", "strip", 0.97),
    ("build-terrain", "strip", 0.95),
    ("build-water", "strip", 0.98),
    ("build-wall", "strip", 0.96),
    ("build-wallpaper", "strip", 0.88),
    ("build-plants", "strip", 0.98),
    ("build-floor", "strip", 0.92),
    ("build-roof", "strip", 0.97),
    ("build-doors", "strip", 0.98),
    ("build-windows", "strip", 0.98),
    ("build-stairs", "strip", 0.98),
    ("build-fireplace", "strip", 0.98),
    ("camera-strip", "strip", 0.97),
    ("options-strip", "strip", 0.98),
    ("flow-magic-hole-picker", "frame", 0.99),
    ("flow-magic-hole-picker", "header", 0.90),
    ("flow-cab-downtown-picker", "frame", 0.99),
    ("flow-cab-downtown-picker", "header", 0.96),
];

/// `[x, y, width, height]`.
type Region = [i32; 4];

fn env_dir(name: &str) -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os(name)?);
    dir.is_dir().then_some(dir)
}

/// An 8-bit RGB, non-interlaced PNG (the only kind the tour holds).
struct Rgb {
    width: usize,
    height: usize,
    pixels: Vec<[u8; 3]>,
}

fn read_png(path: &Path) -> Rgb {
    let data = std::fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    assert_eq!(
        &data[..8],
        b"\x89PNG\r\n\x1a\n",
        "{} is not a PNG",
        path.display()
    );
    let mut pos = 8;
    let (mut width, mut height) = (0usize, 0usize);
    let mut idat = Vec::new();
    while pos + 8 <= data.len() {
        let len = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        let kind = &data[pos + 4..pos + 8];
        let body = &data[pos + 8..pos + 8 + len];
        match kind {
            b"IHDR" => {
                width = u32::from_be_bytes(body[0..4].try_into().unwrap()) as usize;
                height = u32::from_be_bytes(body[4..8].try_into().unwrap()) as usize;
                assert_eq!(
                    (body[8], body[9], body[12]),
                    (8, 2, 0),
                    "{}: only 8-bit RGB non-interlaced PNGs are supported",
                    path.display()
                );
            }
            b"IDAT" => idat.extend_from_slice(body),
            b"IEND" => break,
            _ => {}
        }
        pos += 12 + len;
    }
    let mut raw = Vec::new();
    ZlibDecoder::new(&idat[..])
        .read_to_end(&mut raw)
        .expect("inflate PNG");
    let stride = width * 3;
    let mut out = vec![0u8; stride * height];
    for y in 0..height {
        let filter = raw[y * (stride + 1)];
        let line = &raw[y * (stride + 1) + 1..(y + 1) * (stride + 1)];
        for x in 0..stride {
            let a = if x >= 3 { out[y * stride + x - 3] } else { 0 };
            let b = if y > 0 { out[(y - 1) * stride + x] } else { 0 };
            let c = if x >= 3 && y > 0 {
                out[(y - 1) * stride + x - 3]
            } else {
                0
            };
            let pred = match filter {
                0 => 0,
                1 => a,
                2 => b,
                3 => ((u16::from(a) + u16::from(b)) / 2) as u8,
                4 => {
                    let p = i16::from(a) + i16::from(b) - i16::from(c);
                    let (pa, pb, pc) = (
                        (p - i16::from(a)).abs(),
                        (p - i16::from(b)).abs(),
                        (p - i16::from(c)).abs(),
                    );
                    if pa <= pb && pa <= pc {
                        a
                    } else if pb <= pc {
                        b
                    } else {
                        c
                    }
                }
                f => panic!("bad PNG filter {f}"),
            };
            out[y * stride + x] = line[x].wrapping_add(pred);
        }
    }
    Rgb {
        width,
        height,
        pixels: out.as_chunks::<3>().0.to_vec(),
    }
}

fn ours_at(canvas: &Canvas, x: usize, y: usize) -> [u8; 3] {
    let p = canvas.color[y * canvas.width + x];
    [(p >> 16) as u8, (p >> 8) as u8, p as u8]
}

/// Share of `region`'s pixels within [`TOLERANCE`] of the oracle.
fn score(ours: &Canvas, oracle: &Rgb, region: Region) -> f64 {
    assert_eq!((ours.width, ours.height), (oracle.width, oracle.height));
    let [rx, ry, rw, rh] = region;
    let (mut matched, mut total) = (0u64, 0u64);
    for y in ry.max(0)..(ry + rh).min(oracle.height as i32) {
        for x in rx.max(0)..(rx + rw).min(oracle.width as i32) {
            let (x, y) = (x as usize, y as usize);
            let a = ours_at(ours, x, y);
            let b = oracle.pixels[y * oracle.width + x];
            let diff = (0..3).map(|i| a[i].abs_diff(b[i])).max().unwrap_or(0);
            total += 1;
            if diff <= TOLERANCE {
                matched += 1;
            }
        }
    }
    if total == 0 {
        0.0
    } else {
        matched as f64 / total as f64
    }
}

/// Share of pixels outside the centered 800x600 map viewport (the dark-blue
/// beveled 1024x768 frame) within [`TOLERANCE`].
fn frame_score(ours: &Canvas, oracle: &Rgb) -> f64 {
    let inner = [112, 84, 800, 600];
    let (mut matched, mut total) = (0u64, 0u64);
    for y in 0..oracle.height {
        for x in 0..oracle.width {
            let (xi, yi) = (x as i32, y as i32);
            if xi >= inner[0]
                && xi < inner[0] + inner[2]
                && yi >= inner[1]
                && yi < inner[1] + inner[3]
            {
                continue;
            }
            let a = ours_at(ours, x, y);
            let b = oracle.pixels[y * oracle.width + x];
            total += 1;
            if (0..3).all(|i| a[i].abs_diff(b[i]) <= TOLERANCE) {
                matched += 1;
            }
        }
    }
    matched as f64 / total.max(1) as f64
}

fn open_goth(root: &Path, size: [i32; 2]) -> Game {
    let mut game = Game::open(&Options {
        house: goth_house(root),
        install: root.to_path_buf(),
        answers: vec![3],
        play: false,
        frames: Frames::default(),
    })
    .expect("open Goth house");
    game.resize(size);
    game.render();
    game
}

fn click(game: &mut Game, point: [i32; 2]) {
    game.hover(point);
    game.press(point);
    game.release(point);
    game.render();
}

/// Bottom-left gadget (puck) and the 100px control-panel strip.
fn hud_regions(size: [i32; 2]) -> [(&'static str, Region); 2] {
    let [w, h] = size;
    [
        ("puck", [0, h - 183, 220, 183]),
        ("strip", [220, h - 100, w - 220, 100]),
    ]
}

/// The 4-way view radio (Roof On, Walls Up, Cutaway, Walls Down) on the
/// puck at 1024x768 (oracle clicks at `(42,673)..(112,691)`).
const VIEW_RADIO: Region = [24, 656, 106, 50];

struct Scored {
    shot: &'static str,
    region: &'static str,
    score: f64,
}

fn write_debug(out: Option<&Path>, shot: &str, ours: &Canvas) {
    if let Some(dir) = out {
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::write(dir.join(format!("{shot}.png")), png::encode(ours));
    }
}

#[test]
fn ui_oracle_tour_ratchet() {
    let (Some(install), Some(oracle)) = (env_dir("SIMS_INSTALL"), env_dir("SIMS_UI_ORACLE")) else {
        eprintln!("skipping: set SIMS_INSTALL and SIMS_UI_ORACLE to score the 1.91 UI oracle tour");
        return;
    };
    let shots = oracle.join("shots");
    let out = std::env::var_os("SIMS_UI_ORACLE_OUT").map(PathBuf::from);
    let out = out.as_deref();
    let load = |name: &str| read_png(&shots.join(format!("{name}.png")));
    let mut results: Vec<Scored> = Vec::new();
    let mut push = |shot: &'static str, region: &'static str, score: f64| {
        results.push(Scored {
            shot,
            region,
            score,
        });
    };

    // Live mode, expanded Needs panel, at both resolutions.
    for (shot, size) in [
        ("live-expanded-needs-800", [800, 600]),
        ("live-expanded-needs-1024", [1024, 768]),
    ] {
        let mut game = open_goth(&install, size);
        if !game.panel_expanded() {
            game.hotkey_mode(Mode::Live);
        }
        // The 1024 oracle pressed `p` after expanding the panel and caught
        // the pause button on its lit (red) blink row: one blink period on
        // the presentation clock, with no simulation ticks.
        if size == [1024, 768] && !game.paused() {
            game.toggle_pause();
            game.advance_while(sims_ui::hud::PAUSE_BLINK_US, 0, |_| false);
        }
        game.render();
        let oracle = load(shot);
        write_debug(out, shot, game.canvas());
        for (region, rect) in hud_regions(size) {
            push(shot, region, score(game.canvas(), &oracle, rect));
        }
    }

    let size = [1024, 768];

    // Live Mode tabs, clicked at the oracle's own coordinates.
    for (shot, point) in [
        ("live-relationships-1024", [502, 686]),
        ("live-job-1024", [502, 718]),
        ("live-personality-1024", [470, 718]),
        ("live-house-1024", [502, 750]),
    ] {
        let mut game = open_goth(&install, size);
        if !game.panel_expanded() {
            game.hotkey_mode(Mode::Live);
        }
        click(&mut game, point);
        let oracle = load(shot);
        write_debug(out, shot, game.canvas());
        push(
            shot,
            "strip",
            score(game.canvas(), &oracle, hud_regions(size)[1].1),
        );
    }

    // 4-way view radio, clicked at the oracle's own coordinates.
    for (shot, point) in [
        ("view-roof-on", [42, 673]),
        ("view-walls-up", [65, 678]),
        ("view-cutaway", [88, 684]),
        ("view-walls-down", [112, 691]),
    ] {
        let mut game = open_goth(&install, size);
        click(&mut game, point);
        let oracle = load(shot);
        write_debug(out, shot, game.canvas());
        push(
            shot,
            "view-radio",
            score(game.canvas(), &oracle, VIEW_RADIO),
        );
    }

    let strip = hud_regions(size)[1].1;

    // Buy Mode top level, then Seating (hovering the first chair slot).
    {
        let mut game = open_goth(&install, size);
        game.hotkey_mode(Mode::Buy);
        game.render();
        let oracle = load("buy-top-function");
        write_debug(out, "buy-top-function", game.canvas());
        push(
            "buy-top-function",
            "strip",
            score(game.canvas(), &oracle, strip),
        );

        click(&mut game, [258, 692]);
        game.hover([465, 696]);
        game.render();
        let oracle = load("buy-subcat-seating");
        write_debug(out, "buy-subcat-seating", game.canvas());
        push(
            "buy-subcat-seating",
            "strip",
            score(game.canvas(), &oracle, strip),
        );
    }

    // Build Mode top level and key tools.
    for (shot, tool) in [
        ("build-top", None),
        ("build-terrain", Some([290, 696])),
        ("build-water", Some([332, 696])),
        ("build-wall", Some([374, 696])),
        ("build-wallpaper", Some([416, 696])),
        ("build-plants", Some([290, 744])),
        ("build-floor", Some([332, 744])),
        ("build-roof", Some([458, 744])),
        ("build-doors", Some([374, 744])),
        ("build-windows", Some([416, 744])),
        ("build-stairs", Some([458, 696])),
        ("build-fireplace", Some([506, 696])),
    ] {
        let mut game = open_goth(&install, size);
        game.hotkey_mode(Mode::Build);
        game.render();
        if let Some(point) = tool {
            click(&mut game, point);
        }
        let oracle = load(shot);
        write_debug(out, shot, game.canvas());
        push(shot, "strip", score(game.canvas(), &oracle, strip));
    }

    // Camera strip (F4) and Options strip (F5).
    {
        let mut game = open_goth(&install, size);
        game.toggle_camera_strip();
        game.render();
        let oracle = load("camera-strip");
        write_debug(out, "camera-strip", game.canvas());
        push(
            "camera-strip",
            "strip",
            score(game.canvas(), &oracle, strip),
        );
    }
    {
        let mut game = open_goth(&install, size);
        game.hotkey_mode(Mode::Options);
        game.render();
        let oracle = load("options-strip");
        write_debug(out, "options-strip", game.canvas());
        push(
            "options-strip",
            "strip",
            score(game.canvas(), &oracle, strip),
        );
    }

    // Destination map pickers at 1024x768: the beveled frame around the
    // centered 800x600 map and the header pill.
    let assets = DestinationPickerAssets::load(
        &sims_content::Install::detect(&install).expect("open install"),
    );
    for (shot, request) in [
        (
            "flow-magic-hole-picker",
            SpecialPanelRequest::MagicTownLotPicker { via_blimp: false },
        ),
        (
            "flow-cab-downtown-picker",
            SpecialPanelRequest::DowntownLotPicker,
        ),
    ] {
        let inst = sims_content::Install::detect(&install).expect("open install");
        let mut picker =
            DestinationMapPicker::open(&inst, 1, request, Vec::new(), 0, 0, 0).expect("picker");
        let mut canvas = Canvas::new(1024, 768, [0, 0, 0]);
        picker.draw(&mut canvas, &assets, 0);
        let oracle = load(shot);
        write_debug(out, shot, &canvas);
        push(shot, "frame", frame_score(&canvas, &oracle));
        push(shot, "header", score(&canvas, &oracle, [112, 84, 800, 62]));
    }

    println!("Sims.exe 1.91 UI oracle similarity (share of pixels within {TOLERANCE}/255):");
    println!(
        "{:<28} {:<11} {:>7} {:>7}",
        "shot", "region", "score", "floor"
    );
    let mut failures = Vec::new();
    for r in &results {
        let floor = FLOORS
            .iter()
            .find(|(s, g, _)| *s == r.shot && *g == r.region)
            .map(|f| f.2)
            .unwrap_or_else(|| panic!("no floor for {}/{}", r.shot, r.region));
        println!(
            "{:<28} {:<11} {:>7.4} {:>7.4}",
            r.shot, r.region, r.score, floor
        );
        if r.score + 1e-9 < floor {
            failures.push(format!(
                "{}/{}: {:.4} < {:.4}",
                r.shot, r.region, r.score, floor
            ));
        }
    }
    assert_eq!(results.len(), FLOORS.len(), "every floor must be scored");
    assert!(failures.is_empty(), "UI oracle regressions: {failures:#?}");
}
