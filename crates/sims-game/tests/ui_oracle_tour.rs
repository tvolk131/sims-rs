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
    ("live-relationships-1024", "strip", 0.999),
    ("live-job-1024", "strip", 0.999),
    ("live-personality-1024", "strip", 0.999),
    ("live-house-1024", "strip", 0.999),
    ("view-roof-on", "view-radio", 0.99),
    ("view-walls-up", "view-radio", 0.99),
    ("view-cutaway", "view-radio", 0.99),
    ("view-walls-down", "view-radio", 0.99),
    ("buy-top-function", "strip", 0.99),
    ("buy-subcat-seating", "strip", 0.99),
    ("build-top", "strip", 0.97),
    ("build-terrain", "strip", 0.95),
    ("build-water", "strip", 0.98),
    ("build-wall", "strip", 0.96),
    ("build-wallpaper", "strip", 0.97),
    ("build-plants", "strip", 0.98),
    ("build-floor", "strip", 0.95),
    ("build-roof", "strip", 0.98),
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

/// The folder `name` names, or `None` when it is unset. A set variable
/// that is not a folder fails the test instead of skipping it, so a typo or
/// an unmounted fixture cannot pass with nothing checked.
fn env_dir(name: &str) -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os(name)?);
    assert!(dir.is_dir(), "{name}={} is not a folder", dir.display());
    Some(dir)
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

/// The relationships capture shows the lot running (manifest: AM 6:30,
/// "Bella 100/100, Cassandra 75/75"): the save's rows hold one or two
/// values, and the household's first ticks fill each row's lifetime
/// (value 2) from its daily one. Runs the lot to the second tick, where
/// ours has filled them, and pauses again.
fn run_until_relationships_settle(game: &mut Game) {
    if game.paused() {
        game.toggle_pause();
    }
    let mut ticks = 0;
    for _ in 0..1_000 {
        if ticks >= 2 {
            break;
        }
        ticks += game.advance(10_000);
    }
    assert!(ticks >= 2, "the lot stopped after {ticks} ticks");
    game.toggle_pause();
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
        if shot == "live-relationships-1024" {
            run_until_relationships_settle(&mut game);
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

/// The need controls' rects in the panel's order: each control's help
/// area (`0x513330`), as the HUD laid it out.
fn need_controls(game: &Game) -> [Option<[i32; 2]>; 8] {
    let mut controls = [None; 8];
    for item in game.hud_items() {
        if let Some(sims_ui::command::Command::Help(sims_ui::balloon::Help::Need {
            pet: false,
            index,
        })) = item.command
        {
            controls[usize::from(index)] = Some([item.rect.x, item.rect.y]);
        }
    }
    controls
}

/// The 800 Needs shot caught three falling needs (Hunger, Energy, Bladder)
/// one arrow deep, the arrow at full red (565 keeps a red of 248 and up at
/// 255), and the other four slots drawn in the panel's blue: on the
/// bottom row they show as blue arrows over the panel's edge. Our bars'
/// draw (`0x50c2fa`, `0x50c818`) at our layout's need controls must put
/// every one of those pixels where the original did, in its colour, and
/// the original must show no red pixel we do not draw.
#[test]
fn needs_trend_arrows_match_the_oracle() {
    use sims_ui::needs_history::{Arrows, BLUE, RED, SLOTS};
    let (Some(install), Some(oracle)) = (env_dir("SIMS_INSTALL"), env_dir("SIMS_UI_ORACLE")) else {
        eprintln!("skipping: set SIMS_INSTALL and SIMS_UI_ORACLE to check the trend arrows");
        return;
    };
    let shot = read_png(&oracle.join("shots/live-expanded-needs-800.png"));
    let mut game = open_goth(&install, [800, 600]);
    if !game.panel_expanded() {
        game.hotkey_mode(Mode::Live);
    }
    game.render();
    let controls = need_controls(&game);
    let falling = Arrows {
        falling: true,
        colours: {
            let mut c = [BLUE; SLOTS];
            c[0] = RED;
            c
        },
    };
    let mut drawn = std::collections::BTreeMap::new();
    for need in [0, 1, 6] {
        let control = controls[need].expect("a need control");
        for (point, rgb) in falling.pixels(control) {
            drawn.insert(point, rgb);
        }
    }
    assert_eq!(drawn.len(), 3 * SLOTS * 9);
    // The quantized 565 colour, as the oracle holds it.
    let q = |rgb: u32| {
        let [_, r, g, b] = rgb.to_be_bytes();
        let (r, g, b) = (r >> 3, g >> 2, b >> 3);
        [
            (r << 3) | (r >> 2),
            (g << 2) | (g >> 4),
            (b << 3) | (b >> 2),
        ]
    };
    let at = |[x, y]: [i32; 2]| shot.pixels[y as usize * shot.width + x as usize];
    let wrong: Vec<_> = drawn
        .iter()
        .filter(|&(&p, &rgb)| at(p) != q(rgb))
        .map(|(&p, &rgb)| (p, q(rgb), at(p)))
        .collect();
    assert!(wrong.is_empty(), "pixels unlike the oracle: {wrong:?}");
    let missing: Vec<[i32; 2]> = (500..600)
        .flat_map(|y| (420..800).map(move |x| [x, y]))
        .filter(|&p| at(p) == [255, 0, 0] && !drawn.contains_key(&p))
        .collect();
    assert!(
        missing.is_empty(),
        "oracle arrow pixels we do not draw: {missing:?}"
    );
}

/// The trend arrows as the lot runs: the window loop's frames sample the
/// needs on the presentation clock (at ultra speed, every 48 ms), the bars
/// light falling needs, and the HUD's draw puts each bar's arrows on the
/// canvas in the colours its draw (`0x50c2fa`) gave, fading on to the
/// darker red. Static frames show none.
#[test]
fn needs_trend_arrows_show_as_the_lot_runs() {
    use sims_ui::needs_history::DARK_RED;
    let Some(install) = env_dir("SIMS_INSTALL") else {
        eprintln!("skipping: set SIMS_INSTALL to run the trend arrows on the Goth house");
        return;
    };
    let trends = |game: &Game| -> Vec<(sims_ui::frame::Rect, sims_ui::needs_history::Arrows)> {
        game.hud_items()
            .iter()
            .filter_map(|i| match &i.kind {
                sims_ui::hud::Kind::Trend(a) => Some((i.rect, *a)),
                _ => None,
            })
            .collect()
    };
    let mut game = open_goth(&install, [800, 600]);
    if !game.panel_expanded() {
        game.hotkey_mode(Mode::Live);
    }
    game.render();
    assert!(trends(&game).is_empty(), "a static frame shows no arrows");
    game.set_speed(sims_ui::command::Speed::Ultra);
    let mut dark = false;
    let mut shown = 0;
    for _ in 0..120 {
        game.advance_while(33_333, 33_333, |_| true);
        game.render();
        let lit = trends(&game);
        if lit.is_empty() {
            continue;
        }
        shown += 1;
        let controls = need_controls(&game);
        for (rect, arrows) in lit {
            assert!(
                controls.contains(&Some([rect.x, rect.y])),
                "arrows at a need control: {rect:?}"
            );
            for ([x, y], rgb) in arrows.pixels([rect.x, rect.y]) {
                let [_, r, g, b] = rgb.to_be_bytes();
                assert_eq!(ours_at(game.canvas(), x as usize, y as usize), [r, g, b]);
            }
            dark |= arrows.colours.contains(&DARK_RED);
        }
        if dark {
            break;
        }
    }
    assert!(shown > 0, "no need bar lit as the lot ran");
    assert!(dark, "no arrow settled to the darker red");
}

/// The 1.91 neighborhood captures (`SIMS_NB_ORACLE` overrides).
const DEFAULT_NB_ORACLE: &str =
    "/Users/tommyvolk/Desktop/sims-rs/cache/nb-save-movein-evict-20261121/shots";

/// The share of `region` where `ours` and `oracle` agree within one step
/// of each 5-6-5 channel (the capture widens 5-bit values differently).
fn near_565(ours: &Rgb, oracle: &Rgb, [x, y, w, h]: Region) -> f64 {
    let q = |[r, g, b]: [u8; 3]| [i32::from(r >> 3), i32::from(g >> 2), i32::from(b >> 3)];
    let mut same = 0usize;
    for py in y..y + h {
        for px in x..x + w {
            let i = py as usize * oracle.width + px as usize;
            let (a, b) = (q(ours.pixels[i]), q(oracle.pixels[i]));
            same += usize::from(a.iter().zip(&b).all(|(a, b)| (a - b).abs() <= 1));
        }
    }
    same as f64 / (w * h) as f64
}

/// The lot card hangs from its lot as the original's tooltip places it
/// (`0x5ddbda`: under a lot too near the top, else above it; `0x5da2b0`:
/// pulled back to end a pixel short of the right edge), blends over the map
/// through its tiler's alpha less 70 (`0x54f939`, `0x4ba004`), and shows
/// each member's framed thumbnail (`BMP_` 2004, `0x550b02`). Checked
/// against `02-nb-hover-goth.png` (Lot 5, three members, card below the
/// lot) and `05-nb-hover-newbie.png` (Lot 7, two members, card above it and
/// over the top bar).
#[test]
fn neighborhood_lot_cards_match_the_oracle() {
    let Some(install) = env_dir("SIMS_INSTALL") else {
        eprintln!("skipping: set SIMS_INSTALL to check the lot cards");
        return;
    };
    let shots = std::env::var_os("SIMS_NB_ORACLE")
        .map_or_else(|| PathBuf::from(DEFAULT_NB_ORACLE), PathBuf::from);
    if !shots.join("02-nb-hover-goth.png").is_file() {
        eprintln!("skipping: no neighborhood captures at {}", shots.display());
        return;
    }
    let dir = std::env::temp_dir().join(format!("sims-lot-cards-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let mut shell = sims_game::neighborhood::NeighborhoodShell::open(
        install,
        dir.join("saves.redb"),
        1,
        true,
        vec![3],
        Frames::default(),
    )
    .expect("open Neighborhood 1");
    shell.resize([800, 600]);
    // (lot, capture, card, portrait row)
    let cases: [(u16, &str, Region, Region); 2] = [
        (
            5,
            "02-nb-hover-goth.png",
            [524, 252, 275, 180],
            [549, 382, 75, 25],
        ),
        (
            7,
            "05-nb-hover-newbie.png",
            [524, 27, 275, 180],
            [549, 157, 50, 25],
        ),
    ];
    for (lot, shot, card, portraits) in cases {
        let oracle = read_png(&shots.join(shot));
        let map_lot = shell.map().lot(lot).expect("lot");
        let [left, top, right, bottom] = shell.map().lot_screen_rect(map_lot, [800, 600]);
        let point = (top..bottom)
            .flat_map(|y| (left..right).map(move |x| [x, y]))
            .find(|&p| shell.map().hit_test([800, 600], p) == Some(lot))
            .expect("a point on the lot");
        shell.hover(point);
        // Past the card's delay and the 300 ms fade.
        for _ in 0..40 {
            shell.tick_shell(16_000);
        }
        shell.render();
        let canvas = shell.canvas();
        let ours = Rgb {
            width: canvas.width,
            height: canvas.height,
            pixels: canvas
                .color
                .iter()
                .map(|&p| [(p >> 16) as u8, (p >> 8) as u8, p as u8])
                .collect(),
        };
        let whole = near_565(&ours, &oracle, card);
        let faces = near_565(&ours, &oracle, portraits);
        eprintln!("lot {lot}: card {whole:.4}, portraits {faces:.4}");
        // The misses are the map's water frames at the card's edges.
        assert!(
            whole >= 0.97,
            "lot {lot} card {whole:.4} < 0.97 against {shot}"
        );
        assert!(
            faces >= 0.94,
            "lot {lot} portraits {faces:.4} < 0.94 against {shot}"
        );
        shell.hover([400, 590]);
        for _ in 0..40 {
            shell.tick_shell(16_000);
        }
    }
}
