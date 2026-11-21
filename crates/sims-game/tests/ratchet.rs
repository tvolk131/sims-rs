//! The screenshot ratchet: the frames this game draws of the Goth house,
//! compared pixel for pixel with frozen screenshots of the original.
//!
//! The scenes are in `ratchet.txt` beside this file. Each gives the tick
//! and camera, whether the game is paused and the panel's strip open, the
//! original's screenshot and its SHA-256, the SHA-256 of our frame, the
//! regions left out (each with its reason), and for each class of pixel
//! the share that must match exactly (its floor, in percent). The
//! screenshots are private and are not in the repository: set
//! `SIMS_RATCHET` to the folder that holds them (binary PPM, 800 by 600)
//! and `SIMS_INSTALL` to the game. Without both it prints
//! `ratchet: skipped: no assets`.
//!
//! It fails when:
//!
//! - a screenshot's hash differs from the manifest's (the references are
//!   frozen: a new capture needs a new hash and a reason in its PR);
//! - our frame's hash differs from the pinned one (re-pin it, and give the
//!   new shares in the PR);
//! - a class falls below its floor (lower it on purpose, with the reason
//!   in the PR);
//! - a class with pixels has no floor, or a class with a floor has no
//!   pixels;
//! - the manifest does not parse: an unknown or repeated setting, a
//!   setting before the first scene, no scenes, a floor without exactly
//!   two decimals, or a rectangle outside the frame.
//!
//! `SIMS_RATCHET_UPDATE=1` writes our new hashes into the manifest and
//! raises each floor to the share reached; `floors = none` gives a new
//! scene its first floors. It never lowers or removes a floor, and when
//! any check fails it writes nothing. `SIMS_RATCHET_DIFF=<folder>` writes
//! each scene's difference image (pixels that differ in magenta, the rest
//! ours in grey) and our frame there. The folder must exist under a folder
//! named `cache` and outside the checkout's files (its own `cache/` is
//! fine), so the images never land in the repository.
//!
//! `cargo test` passes its filter and flags to every test binary. The
//! test is named `ratchet` and follows libtest's filters, `--exact`,
//! `--skip` and `--ignored`; filtered out, it prints `ratchet: filtered
//! out`.
//!
//! Classes come from the frame at run time, with no stored masks:
//!
//! - `panel`: the control panel ([`Game::over_panel`]);
//! - `people`: pixels a person drew last ([`Canvas::id_at`] names a
//!   person);
//! - `objects`: pixels any other object drew last;
//! - `surface`: the rest, drawn by nobody: terrain, grass, floors, walls,
//!   wall trims and the road;
//! - `frame`: every pixel not left out.
//!
//! A class with no pixels in a frame shows as `-` and has no floor.
//!
//! A pixel matches when both colours are the same in 16-bit colour (5, 6
//! and 5 bits), which is what the original draws in.
//!
//! The test has its own harness (`harness = false`), so the skip and the
//! table of shares are printed even when it passes.

use sha2::{Digest, Sha256};
use sims_game::game::goth_house;
use sims_game::{Game, Options};
use sims_render::{Camera, Canvas, Walls, Zoom};
use sims_ui::command::Command;
use sims_ui::hud::Kind;
use sims_vm::boot::Frames;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const WIDTH: usize = 800;
const HEIGHT: usize = 600;
const CLASSES: [&str; 5] = ["frame", "surface", "objects", "people", "panel"];

fn manifest_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/ratchet.txt")
}

/// Whether the test harness's arguments select this test, named
/// `ratchet`: `cargo test` passes its filters and flags to every test
/// binary. Filters match as libtest's do (a part of the name, or the whole
/// name with `--exact`); `--skip` leaves it out; `--ignored` runs only
/// ignored tests, which this is not.
fn selected(args: &[String]) -> bool {
    const NAME: &str = "ratchet";
    // libtest's flags that take a value as the next argument.
    const WITH_VALUE: [&str; 7] = [
        "--test-threads",
        "--skip",
        "--logfile",
        "--format",
        "--color",
        "--shuffle-seed",
        "-Z",
    ];
    let (mut filters, mut skips) = (Vec::new(), Vec::new());
    let (mut exact, mut ignored_only) = (false, false);
    let mut words = args.iter();
    while let Some(word) = words.next() {
        let (flag, inline) = match word.split_once('=') {
            Some((flag, value)) if word.starts_with('-') => (flag, Some(value.to_string())),
            _ => (word.as_str(), None),
        };
        match flag {
            "--exact" => exact = true,
            "--ignored" => ignored_only = true,
            f if WITH_VALUE.contains(&f) => {
                let value = inline.or_else(|| words.next().cloned()).unwrap_or_default();
                if f == "--skip" {
                    skips.push(value);
                }
            }
            f if f.starts_with('-') => {}
            _ => filters.push(word.clone()),
        }
    }
    let matches = |pattern: &String| {
        if exact {
            pattern == NAME
        } else {
            NAME.contains(pattern.as_str())
        }
    };
    !ignored_only
        && (filters.is_empty() || filters.iter().any(matches))
        && !skips.iter().any(matches)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !selected(&args) {
        println!("ratchet: filtered out");
        return ExitCode::SUCCESS;
    }
    if args.iter().any(|a| a == "--list") {
        println!("ratchet: test");
        return ExitCode::SUCCESS;
    }
    let (Some(install), Some(references)) = (
        std::env::var_os("SIMS_INSTALL").map(PathBuf::from),
        std::env::var_os("SIMS_RATCHET").map(PathBuf::from),
    ) else {
        println!("ratchet: skipped: no assets (set SIMS_INSTALL and SIMS_RATCHET)");
        return ExitCode::SUCCESS;
    };
    match run(&install, &references) {
        Ok(()) => {
            println!("ratchet: ok");
            ExitCode::SUCCESS
        }
        Err(failures) => {
            for line in failures {
                println!("ratchet: FAILED: {line}");
            }
            ExitCode::FAILURE
        }
    }
}

/// One scene of the manifest.
#[derive(Debug)]
struct Scene {
    name: String,
    tick: u32,
    camera: Camera,
    paused: bool,
    strip: bool,
    file: String,
    original_sha256: String,
    ours_sha256: String,
    /// Left out: [left, top, right, bottom) rectangles.
    leave_out: Vec<[i64; 4]>,
    /// Each class's floor, in hundredths of a percent.
    floors: BTreeMap<&'static str, u32>,
    /// The manifest's lines that hold `ours` and `floors`, which an update
    /// rewrites.
    ours_line: usize,
    floors_line: usize,
}

/// Reads the manifest: a `[name]` line opens a scene, then one
/// `key = value` line for each setting. `#` starts a comment line.
fn parse(text: &str) -> Result<Vec<Scene>, String> {
    let mut scenes = Vec::new();
    let mut fields: Vec<(usize, &str, &str)> = Vec::new();
    let mut name: Option<&str> = None;
    let mut finish = |name: Option<&str>, fields: &mut Vec<(usize, &str, &str)>| {
        if let Some(name) = name {
            scenes.push(scene(name, fields));
        }
        fields.clear();
    };
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix('[') {
            finish(name, &mut fields);
            let new = rest
                .strip_suffix(']')
                .ok_or(format!("line {}: {line}", n + 1))?;
            // The name names the difference images too.
            if new.is_empty()
                || !new
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            {
                return Err(format!(
                    "line {}: a scene's name is lowercase letters, digits and hyphens: {new:?}",
                    n + 1
                ));
            }
            name = Some(new);
            continue;
        }
        let (key, value) = line
            .split_once(" = ")
            .ok_or(format!("line {}: not `key = value`: {line}", n + 1))?;
        if name.is_none() {
            return Err(format!(
                "line {}: a setting before the first scene: {line}",
                n + 1
            ));
        }
        fields.push((n, key, value));
    }
    finish(name, &mut fields);
    let scenes: Vec<Scene> = scenes.into_iter().collect::<Result<_, _>>()?;
    if scenes.is_empty() {
        return Err("no scenes".into());
    }
    let mut names = std::collections::BTreeSet::new();
    if let Some(twice) = scenes.iter().find(|s| !names.insert(s.name.as_str())) {
        return Err(format!("two scenes named {:?}", twice.name));
    }
    Ok(scenes)
}

/// A floor or share in hundredths of a percent: `96.51` is 9651. A floor
/// is written with exactly two decimals.
fn hundredths(word: &str) -> Option<u32> {
    let (whole, part) = word.split_once('.')?;
    if whole.is_empty()
        || part.len() != 2
        || !whole
            .bytes()
            .chain(part.bytes())
            .all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let value = whole.parse::<u32>().ok()?.checked_mul(100)? + part.parse::<u32>().ok()?;
    (value <= 10_000).then_some(value)
}

fn scene(name: &str, fields: &[(usize, &str, &str)]) -> Result<Scene, String> {
    const KEYS: [&str; 13] = [
        "about",
        "tick",
        "zoom",
        "rotation",
        "level",
        "origin",
        "walls",
        "paused",
        "strip",
        "original",
        "ours",
        "leave_out",
        "floors",
    ];
    for (n, key, _) in fields {
        if !KEYS.contains(key) {
            return Err(format!("{name}: line {}: no setting {key:?}", n + 1));
        }
        if *key != "leave_out" && fields.iter().filter(|f| f.1 == *key).count() > 1 {
            return Err(format!("{name}: `{key}` is given twice"));
        }
    }
    let get = |key: &str| -> Result<(usize, &str), String> {
        fields
            .iter()
            .find(|f| f.1 == key)
            .map(|f| (f.0, f.2))
            .ok_or(format!("{name}: no `{key}`"))
    };
    let number = |key: &str, word: &str| -> Result<i64, String> {
        word.parse()
            .map_err(|_| format!("{name}: `{key}`: {word:?}"))
    };
    let yes = |key: &str| -> Result<bool, String> {
        match get(key)?.1 {
            "yes" => Ok(true),
            "no" => Ok(false),
            other => Err(format!("{name}: `{key}` is yes or no, not {other:?}")),
        }
    };
    let origin: Vec<i64> = get("origin")?
        .1
        .split_whitespace()
        .map(|w| number("origin", w))
        .collect::<Result<_, _>>()?;
    let [x, y] = origin[..] else {
        return Err(format!("{name}: `origin` is two numbers"));
    };
    let camera = Camera {
        zoom: match get("zoom")?.1 {
            "far" => Zoom::Far,
            "medium" => Zoom::Medium,
            "near" => Zoom::Near,
            other => return Err(format!("{name}: zoom {other:?}")),
        },
        rotation: u8::try_from(number("rotation", get("rotation")?.1)?)
            .map_err(|e| format!("{name}: {e}"))?,
        level: i32::try_from(number("level", get("level")?.1)?)
            .map_err(|e| format!("{name}: {e}"))?,
        origin: [
            i32::try_from(x).map_err(|e| format!("{name}: {e}"))?,
            i32::try_from(y).map_err(|e| format!("{name}: {e}"))?,
        ],
        walls: match get("walls")?.1 {
            "up" => Walls::Up,
            "cutaway" => Walls::Cutaway,
            "down" => Walls::Down,
            other => return Err(format!("{name}: walls {other:?}")),
        },
        focus: Default::default(),
    };
    let (file, original_sha256) = get("original")?
        .1
        .split_once(' ')
        .ok_or(format!("{name}: `original` is a file and its SHA-256"))?;
    let (ours_line, ours) = get("ours")?;
    let mut leave_out = Vec::new();
    for (_, _, value) in fields.iter().filter(|f| f.1 == "leave_out") {
        let words: Vec<&str> = value.splitn(5, ' ').collect();
        if words.len() < 5 {
            return Err(format!("{name}: `leave_out` is a rectangle and a reason"));
        }
        let mut rect = [0; 4];
        for (r, w) in rect.iter_mut().zip(&words) {
            *r = number("leave_out", w)?;
        }
        let [left, top, right, bottom] = rect;
        // Inside the frame and not empty: a typo must not hide the rest
        // of a row.
        if !(0 <= left && left < right && right <= WIDTH as i64)
            || !(0 <= top && top < bottom && bottom <= HEIGHT as i64)
        {
            return Err(format!(
                "{name}: `leave_out` {rect:?} is not a rectangle inside {WIDTH} by {HEIGHT}"
            ));
        }
        leave_out.push(rect);
    }
    let (floors_line, floor_words) = get("floors")?;
    // `none` starts a new scene: an update sets its floors.
    let words: Vec<&str> = if floor_words == "none" {
        Vec::new()
    } else {
        floor_words.split_whitespace().collect()
    };
    let mut floors = BTreeMap::new();
    for pair in words.chunks(2) {
        let [class, value] = pair else {
            return Err(format!("{name}: `floors` is class and share pairs"));
        };
        let Some(&class) = CLASSES.iter().find(|c| *c == class) else {
            return Err(format!("{name}: no class {class:?}"));
        };
        let floor = hundredths(value).ok_or(format!(
            "{name}: floor {value:?} is not a share with two decimals, at most 100.00"
        ))?;
        if floors.insert(class, floor).is_some() {
            return Err(format!("{name}: two floors for {class}"));
        }
    }
    Ok(Scene {
        name: name.to_string(),
        tick: u32::try_from(number("tick", get("tick")?.1)?).map_err(|e| format!("{name}: {e}"))?,
        camera,
        paused: yes("paused")?,
        strip: yes("strip")?,
        file: file.to_string(),
        original_sha256: original_sha256.to_string(),
        ours_sha256: ours.to_string(),
        leave_out,
        floors,
        ours_line,
        floors_line,
    })
}

/// Opens or closes the panel's strip (the live mode's needs and the
/// like) with the mode's own button, as the player does, to match the
/// screenshot.
fn set_strip(game: &mut Game, open: bool) -> Result<(), String> {
    for _ in 0..2 {
        game.render();
        let items = game.panel();
        if items.iter().any(|i| matches!(i.kind, Kind::Stretched(_))) == open {
            return Ok(());
        }
        let toggle = items
            .iter()
            .find(|i| i.command == Some(Command::TogglePanel))
            .ok_or("the panel has no button that opens or closes its strip")?
            .rect;
        game.press([toggle.x + toggle.width / 2, toggle.y + toggle.height / 2]);
    }
    Err(format!(
        "the strip does not {}",
        if open { "open" } else { "close" }
    ))
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A binary PPM (P6, 8 bits a channel) of 800 by 600.
fn read_ppm(bytes: &[u8]) -> Result<Vec<[u8; 3]>, String> {
    let mut fields = Vec::new();
    let mut at = 0;
    while fields.len() < 4 {
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        if bytes.get(at) == Some(&b'#') {
            while at < bytes.len() && bytes[at] != b'\n' {
                at += 1;
            }
            continue;
        }
        if at >= bytes.len() {
            return Err("the header ends early".into());
        }
        let start = at;
        while at < bytes.len() && !bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        fields.push(String::from_utf8_lossy(&bytes[start..at]).into_owned());
    }
    let size = format!("{WIDTH} {HEIGHT}");
    if fields[0] != "P6" || format!("{} {}", fields[1], fields[2]) != size || fields[3] != "255" {
        return Err(format!("not an 8-bit P6 of {size}: {fields:?}"));
    }
    let pixels = bytes.get(at + 1..).unwrap_or_default();
    if pixels.len() != WIDTH * HEIGHT * 3 {
        return Err(format!("{} bytes of pixels", pixels.len()));
    }
    Ok(pixels.chunks(3).map(|c| [c[0], c[1], c[2]]).collect())
}

/// The colour in 16-bit colour: 5 bits of red, 6 of green, 5 of blue.
fn key([r, g, b]: [u8; 3]) -> [u8; 3] {
    [r >> 3, g >> 2, b >> 3]
}

/// Matched and counted pixels for each class.
type Shares = BTreeMap<&'static str, (u64, u64)>;

/// The share matched, in hundredths of a percent, rounded down: a share
/// meets a floor exactly when this is at least the floor. `counted` is
/// not zero.
fn share_of((matched, counted): (u64, u64)) -> u32 {
    u32::try_from(matched * 10_000 / counted).expect("at most 10000")
}

fn shown(hundredths: u32) -> String {
    format!("{}.{:02}", hundredths / 100, hundredths % 100)
}

fn compare(
    game: &Game,
    canvas: &Canvas,
    original: &[[u8; 3]],
    leave_out: &[[i64; 4]],
    mut diff: Option<&mut Canvas>,
) -> Shares {
    let world = game.lot().world();
    let ours = canvas.rgb565();
    let mut shares: Shares = CLASSES.iter().map(|&c| (c, (0, 0))).collect();
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let (xi, yi) = (x as i64, y as i64);
            if leave_out
                .iter()
                .any(|r| (r[0]..r[2]).contains(&xi) && (r[1]..r[3]).contains(&yi))
            {
                continue;
            }
            let point = [x as i32, y as i32];
            let class = if game.over_panel(point) {
                "panel"
            } else {
                match canvas.id_at(point[0], point[1]) {
                    None => "surface",
                    Some(id) => match world.objects.get(&id) {
                        Some(o) if o.person.is_some() => "people",
                        _ => "objects",
                    },
                }
            };
            let i = y * WIDTH + x;
            let same = key(ours[i]) == key(original[i]);
            for c in ["frame", class] {
                let entry = shares.get_mut(c).expect("a class");
                entry.0 += u64::from(same);
                entry.1 += 1;
            }
            if let Some(d) = diff.as_deref_mut() {
                let [r, g, b] = ours[i];
                let grey = ((u32::from(r) + u32::from(g) + u32::from(b)) / 6) as u8;
                d.color[i] = if same {
                    [grey, grey, grey]
                } else {
                    [255, 0, 255]
                };
            }
        }
    }
    shares
}

/// Runs the lot on to `tick`, answering each dialog with its first
/// button.
fn run_to(game: &mut Game, tick: u32) -> Result<(), String> {
    while game.lot().ticks() < tick {
        let left = tick - game.lot().ticks();
        if game.run(left) < left {
            if let Some(stop) = game.stopped() {
                return Err(stop.to_string());
            }
            let caller = game
                .asking()
                .ok_or(format!("the lot stopped short of {tick}"))?
                .shown
                .object;
            game.answer(caller, 0);
        }
    }
    Ok(())
}

/// The folder for difference images. It must exist under a folder named
/// `cache`, and not inside this checkout's files unless under its own
/// `cache/` (which git ignores), so no image lands in the repository.
fn diff_folder(dir: &Path) -> Result<PathBuf, String> {
    let dir = dir
        .canonicalize()
        .map_err(|e| format!("SIMS_RATCHET_DIFF {dir:?}: {e}"))?;
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .map_err(|e| format!("the checkout's folder: {e}"))?;
    let under_cache = dir.components().any(|c| c.as_os_str() == "cache");
    let outside_checkout = !dir.starts_with(&root) || dir.starts_with(root.join("cache"));
    if !dir.is_dir() || !under_cache || !outside_checkout {
        return Err(format!(
            "SIMS_RATCHET_DIFF {dir:?} is not a folder under `cache` outside the checkout's files"
        ));
    }
    Ok(dir)
}

fn run(install: &Path, references: &Path) -> Result<(), Vec<String>> {
    let fail = |e: String| vec![e];
    let path = manifest_path();
    let text = std::fs::read_to_string(&path).map_err(|e| fail(format!("{path:?}: {e}")))?;
    let scenes = parse(&text).map_err(|e| fail(format!("{path:?}: {e}")))?;
    let update = std::env::var_os("SIMS_RATCHET_UPDATE").is_some_and(|v| v == "1");
    let diff_dir = std::env::var_os("SIMS_RATCHET_DIFF")
        .map(|d| diff_folder(Path::new(&d)))
        .transpose()
        .map_err(fail)?;

    // Every reference first: a changed screenshot stops the run.
    let mut originals = Vec::new();
    for scene in &scenes {
        let name = &scene.name;
        let file = references.join(&scene.file);
        let bytes = std::fs::read(&file).map_err(|e| fail(format!("{name}: {file:?}: {e}")))?;
        let got = sha256(&bytes);
        if got != scene.original_sha256 {
            return Err(fail(format!(
                "{name}: {file:?} has SHA-256 {got}, not the manifest's {}: the references \
                 are frozen",
                scene.original_sha256
            )));
        }
        originals.push(read_ppm(&bytes).map_err(|e| fail(format!("{name}: {e}")))?);
    }

    let mut game = Game::open(&Options {
        house: goth_house(install),
        install: install.to_path_buf(),
        answers: vec![3],
        play: false,
        frames: Frames::default(),
    })
    .map_err(|e| fail(format!("the Goth house does not open: {e}")))?;
    game.resize([WIDTH as i32, HEIGHT as i32]);

    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let mut order: Vec<usize> = (0..scenes.len()).collect();
    order.sort_by_key(|&i| scenes[i].tick);
    let mut failures = Vec::new();
    println!(
        "ratchet: {:<24} {:>8} {:>8} {:>8} {:>8} {:>8}",
        "scene", "frame", "surface", "objects", "people", "panel"
    );
    for i in order {
        let scene = &scenes[i];
        let name = &scene.name;
        run_to(&mut game, scene.tick).map_err(|e| fail(format!("{name}: {e}")))?;
        game.view.camera = scene.camera;
        set_strip(&mut game, scene.strip).map_err(|e| fail(format!("{name}: {e}")))?;
        // The original was paused for its screenshot. Paused, the lot's
        // clock stands, so the game runs on unpaused after the frame.
        if scene.paused {
            game.toggle_pause();
        }
        game.render();
        let canvas = game.canvas().clone();
        if scene.paused {
            game.toggle_pause();
        }
        let mut diff = diff_dir
            .as_ref()
            .map(|_| Canvas::new(WIDTH, HEIGHT, [0, 0, 0]));
        let shares = compare(
            &game,
            &canvas,
            &originals[i],
            &scene.leave_out,
            diff.as_mut(),
        );
        if let (Some(dir), Some(d)) = (&diff_dir, &diff) {
            for (out, image) in [
                (dir.join(format!("{name}.png")), d),
                (dir.join(format!("{name}-ours.png")), &canvas),
            ] {
                if let Err(e) = std::fs::write(&out, sims_render::png::encode(image)) {
                    failures.push(format!("{name}: {out:?}: {e}"));
                }
            }
        }
        // A class with no pixels in the frame shows as `-` and keeps no
        // floor: there is nothing to match.
        let row: Vec<String> = CLASSES
            .iter()
            .map(|c| match shares[c] {
                (_, 0) => format!("{:>8}", "-"),
                share => format!("{:>7}%", shown(share_of(share))),
            })
            .collect();
        println!("ratchet: {name:<24} {}", row.join(" "));

        let ours = sha256(&canvas.color.concat());
        if ours != scene.ours_sha256 {
            if update {
                lines[scene.ours_line] = format!("ours = {ours}");
            } else {
                failures.push(format!(
                    "{name}: our frame's SHA-256 is {ours}, not the pinned {}: re-pin it \
                     (SIMS_RATCHET_UPDATE=1) and give the new shares in the PR",
                    scene.ours_sha256
                ));
            }
        }
        let mut floors = Vec::new();
        for c in CLASSES {
            let floor = scene.floors.get(c).copied();
            if shares[c].1 == 0 {
                // The frame lost what the floor was kept for.
                if let Some(floor) = floor {
                    failures.push(format!(
                        "{name}: {c} has no pixels, but it has a floor of {}%: remove the \
                         floor by hand, with the reason in the PR",
                        shown(floor)
                    ));
                }
                continue;
            }
            let share = share_of(shares[c]);
            let kept = match floor {
                Some(floor) if share < floor => {
                    failures.push(format!(
                        "{name}: {c} matches {}%, below its floor {}%",
                        shown(share),
                        shown(floor)
                    ));
                    floor
                }
                Some(floor) if update => floor.max(share),
                Some(floor) => floor,
                None if update => share,
                None => {
                    failures.push(format!(
                        "{name}: {c} matches {}% and has no floor: set one \
                         (SIMS_RATCHET_UPDATE=1)",
                        shown(share)
                    ));
                    continue;
                }
            };
            floors.push(format!("{c} {}", shown(kept)));
        }
        if update {
            lines[scene.floors_line] = format!("floors = {}", floors.join(" "));
        }
    }
    // An update that failed writes nothing: it must not re-pin a frame
    // that fails, or keep a floor it could not meet.
    if update && failures.is_empty() {
        let text = lines.join("\n") + "\n";
        std::fs::write(&path, text).map_err(|e| fail(format!("{path:?}: {e}")))?;
        println!("ratchet: wrote {path:?}");
    } else if update {
        println!("ratchet: wrote nothing: the run failed");
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures)
    }
}
