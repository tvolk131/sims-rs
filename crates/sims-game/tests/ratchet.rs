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
//! and `SIMS_INSTALL` to the game.
//!
//! Without `SIMS_RATCHET` it prints `ratchet: skipped: no assets` and
//! passes; the workspace's other tests run with only `SIMS_INSTALL` set.
//! Every other setting that is partial or wrong fails, on the skip too,
//! and before `--help`, `--list` or a filter ends the run: `SIMS_INSTALL`
//! set but empty, or not a folder holding a regular file at some depth;
//! `SIMS_RATCHET` without `SIMS_INSTALL`, or not such a folder; any
//! `SIMS_RATCHET_` variable, whatever its value (its name matched as
//! bytes, so one that is not text counts), without `SIMS_RATCHET`; and a
//! `SIMS_RATCHET_` variable other than `SIMS_RATCHET_UPDATE`,
//! `SIMS_RATCHET_NEW` and `SIMS_RATCHET_DIFF`.
//!
//! It fails when:
//!
//! - a screenshot's hash differs from the manifest's (the references are
//!   frozen: a new capture needs a new hash and a reason in its PR);
//! - our frame's hash differs from the pinned one (re-pin it, and give the
//!   new shares in the PR). The pin covers the whole frame, regions left
//!   out included: our drawing is deterministic, so it is the check that
//!   nothing we draw changed. The regions left out only shape the shares,
//!   where they hide the original's animations;
//! - a class falls below its floor (lower it on purpose, with the reason
//!   in the PR);
//! - a class with pixels has no floor, a class with a floor has no pixels,
//!   or a scene leaves every pixel out;
//! - the manifest does not parse: an unknown or repeated setting, a
//!   setting before the first scene, a missing baseline scene, a baseline
//!   scene written as new, `ours = none` without `floors = none` or the
//!   reverse, a screenshot that is not a plain file name, a floor without
//!   exactly two decimals or above 100.00, or a rectangle outside the
//!   frame;
//! - a screenshot, with links resolved, is not in `SIMS_RATCHET`.
//!
//! `SIMS_RATCHET_UPDATE=1` writes our new hashes into the manifest and
//! raises each floor to the share reached. It never adds, lowers or
//! removes a floor. A new scene is written with `ours = none` and
//! `floors = none`, and `SIMS_RATCHET_NEW=<name>[,<name>…]` sets its hash
//! and first floors; that is the only way a floor is added. Once set, its
//! name joins `BASELINE`, and `SIMS_RATCHET_NEW` refuses a baseline scene.
//! When any check fails nothing is written. The manifest is replaced
//! whole (a new file, then a rename), and only if it is unchanged since
//! the run read it. Run one updater at a time: the hash check catches
//! edits made between read and write, not two updaters racing.
//!
//! `SIMS_RATCHET_DIFF=<folder>` writes each scene's difference image
//! (pixels that differ in magenta, the rest ours in grey) and our frame
//! into a new folder the run makes inside it. The folder, with links
//! resolved, must lie under a folder named `cache` and outside this
//! checkout, and each image is a new file, never written over.
//!
//! `cargo test` passes its filter and flags to every test binary. The
//! test is named `ratchet` and reads its arguments as libtest does: a flag
//! that takes a value takes the next word whatever it is, every word after
//! `--` is a filter, and filters, `--exact`, `--skip`, `--ignored` and
//! `--include-ignored` select as libtest's do. What libtest refuses fails
//! here before any filter or list decides anything: an unknown flag, a
//! flag without its value, a value on a flag that takes none
//! (`--list=foo`), a bad value, and `--ignored` with `--include-ignored`.
//! Filtered out, it prints `ratchet: filtered out`. Before anything else
//! it checks its own floor and argument parsing, so a release run covers
//! them too.
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
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const NAME: &str = "ratchet";
const WIDTH: usize = 800;
const HEIGHT: usize = 600;
const CLASSES: [&str; 5] = ["frame", "surface", "objects", "people", "panel"];
/// The scenes the manifest must keep: removing one is not a way to pass.
/// A PR that adds a scene adds its name here.
const BASELINE: [&str; 8] = [
    "near-r2-0712-bench",
    "near-r2-0805",
    "far-r0-cut-0816",
    "far-r1-cut-0816",
    "medium-r1-cut-0816",
    "near-r3-down-0816",
    "near-r0-up-0816",
    "night-far-r0-cut-2153",
];

fn manifest_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/ratchet.txt")
}

/// The checkout this test was built from, links resolved.
fn checkout_root() -> Result<PathBuf, String> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .map_err(|e| format!("the checkout's folder: {e}"))
}

/// What libtest's arguments ask of this binary.
#[derive(Debug, Default)]
struct Args {
    list: bool,
    exact: bool,
    ignored_only: bool,
    include_ignored: bool,
    help: bool,
    filters: Vec<String>,
    skips: Vec<String>,
}

impl Args {
    /// Reads the arguments once, as libtest does: a flag that takes a value
    /// takes the next word whatever it is (`--skip --list` skips the name
    /// `--list`), and every word after `--` is a filter. What libtest
    /// refuses is an error here too, before any filter or list decides
    /// anything: an unknown flag, a flag without its value, a value given
    /// to a flag that takes none (`--list=foo`), a bad value, and
    /// `--ignored` with `--include-ignored`.
    fn parse(words: &[String]) -> Result<Args, String> {
        const PLAIN: [&str; 17] = [
            "--list",
            "--exact",
            "--ignored",
            "--include-ignored",
            "--force-run-in-process",
            "--exclude-should-panic",
            "--test",
            "--bench",
            "--nocapture",
            "--no-capture",
            "--quiet",
            "-q",
            "--show-output",
            "--report-time",
            "--ensure-time",
            "--shuffle",
            "--help",
        ];
        const WITH_VALUE: [&str; 7] = [
            "--test-threads",
            "--skip",
            "--logfile",
            "--format",
            "--color",
            "--shuffle-seed",
            "-Z",
        ];
        let mut args = Args::default();
        let mut words = words.iter();
        while let Some(word) = words.next() {
            if word == "--" {
                args.filters.extend(words.by_ref().cloned());
                break;
            }
            if !word.starts_with('-') || word == "-" {
                args.filters.push(word.clone());
                continue;
            }
            let (flag, inline) = match word.split_once('=') {
                Some((flag, value)) => (flag, Some(value.to_string())),
                None => (word.as_str(), None),
            };
            if PLAIN.contains(&flag) || flag == "-h" {
                if inline.is_some() {
                    return Err(format!("{flag} takes no value: {word:?}"));
                }
                match flag {
                    "--list" => args.list = true,
                    "--exact" => args.exact = true,
                    "--ignored" => args.ignored_only = true,
                    "--include-ignored" => args.include_ignored = true,
                    "--help" | "-h" => args.help = true,
                    _ => {}
                }
                continue;
            }
            if !WITH_VALUE.contains(&flag) {
                return Err(format!("unknown flag {word:?}"));
            }
            let value = inline
                .or_else(|| words.next().cloned())
                .ok_or(format!("{flag} needs a value"))?;
            let good = match flag {
                "--test-threads" => value.parse::<usize>().is_ok_and(|n| n > 0),
                "--format" => ["pretty", "terse", "json", "junit"].contains(&value.as_str()),
                "--color" => ["auto", "always", "never"].contains(&value.as_str()),
                "--shuffle-seed" => value.parse::<u64>().is_ok(),
                "-Z" => value == "unstable-options",
                _ => true,
            };
            if !good {
                return Err(format!("{flag} {value:?} is not a value it takes"));
            }
            if flag == "--skip" {
                args.skips.push(value);
            }
        }
        if args.ignored_only && args.include_ignored {
            return Err("--ignored and --include-ignored are mutually exclusive".into());
        }
        Ok(args)
    }

    /// Whether these arguments run the test `name` (which is not ignored).
    fn selects(&self, name: &str) -> bool {
        let matches = |pattern: &String| {
            if self.exact {
                pattern == name
            } else {
                name.contains(pattern.as_str())
            }
        };
        (!self.ignored_only || self.include_ignored)
            && (self.filters.is_empty() || self.filters.iter().any(matches))
            && !self.skips.iter().any(matches)
    }
}

/// The run's settings, from the environment.
struct Config {
    install: PathBuf,
    references: PathBuf,
    update: bool,
    new: Vec<String>,
    diff: Option<PathBuf>,
}

impl Config {
    /// `None` when `SIMS_RATCHET` is not set (the skip); an error for any
    /// other partial, empty or unknown setting. `SIMS_INSTALL` is checked
    /// whenever it is set, on the skip too.
    fn from_env() -> Result<Option<Config>, String> {
        const SETTINGS: [&str; 3] = [
            "SIMS_RATCHET_UPDATE",
            "SIMS_RATCHET_NEW",
            "SIMS_RATCHET_DIFF",
        ];
        let var = |key: &str| -> Result<Option<String>, String> {
            match std::env::var_os(key) {
                None => Ok(None),
                Some(v) if v.is_empty() => Err(format!("{key} is set but empty")),
                Some(v) => v
                    .into_string()
                    .map(Some)
                    .map_err(|v| format!("{key} is not text: {v:?}")),
            }
        };
        // Every SIMS_RATCHET_ variable, whatever its value: a misspelt one
        // must not be ignored.
        // matched on the key's bytes, so a key that is not text is caught too.
        let mut set: Vec<String> = std::env::vars_os()
            .filter(|(k, _)| k.as_encoded_bytes().starts_with(b"SIMS_RATCHET_"))
            .map(|(k, _)| k.to_string_lossy().into_owned())
            .collect();
        set.sort();
        if let Some(unknown) = set.iter().find(|k| !SETTINGS.contains(&k.as_str())) {
            return Err(format!(
                "{unknown} is not a setting (the settings are {})",
                SETTINGS.join(", ")
            ));
        }
        let install = var("SIMS_INSTALL")?.map(PathBuf::from);
        if let Some(install) = &install {
            holds_a_file("SIMS_INSTALL", install)?;
        }
        let Some(references) = var("SIMS_RATCHET")?.map(PathBuf::from) else {
            if !set.is_empty() {
                return Err(format!(
                    "{} is set, but SIMS_RATCHET is not",
                    set.join(" and ")
                ));
            }
            return Ok(None);
        };
        let install = install.ok_or("SIMS_RATCHET is set, but SIMS_INSTALL is not")?;
        holds_a_file("SIMS_RATCHET", &references)?;
        let update = match var("SIMS_RATCHET_UPDATE")?.as_deref() {
            None | Some("0") => false,
            Some("1") => true,
            Some(other) => return Err(format!("SIMS_RATCHET_UPDATE is 1 or 0, not {other:?}")),
        };
        let new: Vec<String> = var("SIMS_RATCHET_NEW")?
            .map(|v| v.split(',').map(str::to_string).collect())
            .unwrap_or_default();
        let diff = var("SIMS_RATCHET_DIFF")?.map(PathBuf::from);
        Ok(Some(Config {
            install,
            references,
            update,
            new,
            diff,
        }))
    }
}

/// An error unless `dir` is a folder holding at least one regular file,
/// at any depth: a folder of empty folders is as good as none. A link to
/// a file counts; a link to a folder is not followed.
fn holds_a_file(key: &str, dir: &Path) -> Result<(), String> {
    if !dir.is_dir() {
        return Err(format!("{key} {dir:?} is not a folder"));
    }
    let mut folders = vec![dir.to_path_buf()];
    while let Some(folder) = folders.pop() {
        // Only whether any file exists: the order does not matter.
        #[allow(clippy::disallowed_methods)]
        let entries = std::fs::read_dir(&folder).map_err(|e| format!("{key} {folder:?}: {e}"))?;
        for entry in entries {
            let path = entry.map_err(|e| format!("{key} {folder:?}: {e}"))?.path();
            if path.is_file() {
                return Ok(());
            }
            if path.symlink_metadata().is_ok_and(|m| m.is_dir()) {
                folders.push(path);
            }
        }
    }
    Err(format!("{key} {dir:?} holds no file"))
}

fn main() -> ExitCode {
    if let Err(e) = self_check() {
        println!("ratchet: FAILED: self-check: {e}");
        return ExitCode::FAILURE;
    }
    let args = match Args::parse(&std::env::args().skip(1).collect::<Vec<_>>()) {
        Ok(args) => args,
        Err(e) => {
            println!("ratchet: FAILED: arguments: {e}");
            return ExitCode::FAILURE;
        }
    };
    // The settings are checked before any exit that passes: a bad setting
    // fails `--help`, `--list` and a filtered-out run too.
    let config = match Config::from_env() {
        Ok(config) => config,
        Err(e) => {
            println!("ratchet: FAILED: {e}");
            return ExitCode::FAILURE;
        }
    };
    if args.help {
        println!("ratchet: the screenshot ratchet; see crates/sims-game/tests/ratchet.rs");
        return ExitCode::SUCCESS;
    }
    if !args.selects(NAME) {
        println!("ratchet: filtered out");
        return ExitCode::SUCCESS;
    }
    if args.list {
        println!("{NAME}: test");
        return ExitCode::SUCCESS;
    }
    let Some(config) = config else {
        println!("ratchet: skipped: no assets (SIMS_RATCHET is not set)");
        return ExitCode::SUCCESS;
    };
    match run(&config) {
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

/// Checks the floor and argument parsing on every run, whatever the
/// build: a floor that parsed wrong would turn its class's check off.
fn self_check() -> Result<(), String> {
    for (word, want) in [
        ("96.51", Some(9651)),
        ("0.00", Some(0)),
        ("100.00", Some(10_000)),
        ("007.50", Some(750)),
        ("100.01", None),
        ("101.00", None),
        ("1000.00", None),
        ("42949672.96", None),
        ("4294967296.00", None),
        ("96.5", None),
        ("96.511", None),
        (".51", None),
        ("-1.00", None),
        ("+1.00", None),
        ("NaN", None),
        ("1e2", None),
        ("", None),
    ] {
        if hundredths(word) != want {
            return Err(format!(
                "floor {word:?} reads as {:?}, not {want:?}",
                hundredths(word)
            ));
        }
    }
    for (line, list, runs) in [
        ("", false, true),
        ("--skip --list", false, true),
        ("-- --list", false, false),
        ("--test-threads 1", false, true),
        ("--test-threads=1 ratch", false, true),
        ("--skip ratchet", false, false),
        ("--skip=rat", false, false),
        ("rat --exact", false, false),
        ("ratchet --exact", false, true),
        ("unrelated --list", true, false),
        ("--list", true, true),
        ("--ignored", false, false),
        ("--include-ignored", false, true),
        ("--skip=", false, false),
        ("--color always -q --nocapture", false, true),
    ] {
        let words: Vec<String> = line.split_whitespace().map(str::to_string).collect();
        let args = Args::parse(&words).map_err(|e| format!("arguments {line:?}: {e}"))?;
        if args.list != list || args.selects(NAME) != runs {
            return Err(format!(
                "arguments {line:?}: list {}, runs {}; want list {list}, runs {runs}",
                args.list,
                args.selects(NAME)
            ));
        }
    }
    for line in [
        "--skip",
        "ratchet --skip",
        "--list=foo",
        "--exact=yes",
        "--ignored --include-ignored",
        "--frobnicate",
        "-x",
        "--test-threads 0",
        "--test-threads many",
        "--color sometimes",
        "--format fancy",
        "-Z nightly",
    ] {
        let words: Vec<String> = line.split_whitespace().map(str::to_string).collect();
        if Args::parse(&words).is_ok() {
            return Err(format!(
                "arguments {line:?} are accepted, but libtest refuses them"
            ));
        }
    }
    for (text, want) in [
        ("# only comments\n", "no `[near-r2-0712-bench]`"),
        ("tick = 1\n[a]\n", "before the first scene"),
    ] {
        match parse(text) {
            Err(e) if e.contains(want) => {}
            other => {
                return Err(format!(
                    "manifest {text:?} gives {:?}, not an error naming {want:?}",
                    other.map(|s| s.len())
                ));
            }
        }
    }
    // One scene's settings, with `original`, `ours` and `floors` varied.
    let hash = "0".repeat(64);
    for (original, ours, floors, want) in [
        ("a.ppm", hash.as_str(), "frame 1.00", None),
        ("a.ppm", "none", "none", None),
        (
            "a.ppm",
            "none",
            "frame 1.00",
            Some("both `ours = none` and `floors = none`"),
        ),
        (
            "a.ppm",
            hash.as_str(),
            "none",
            Some("both `ours = none` and `floors = none`"),
        ),
        (
            "/tmp/a.ppm",
            hash.as_str(),
            "frame 1.00",
            Some("plain file name"),
        ),
        (
            "../a.ppm",
            hash.as_str(),
            "frame 1.00",
            Some("plain file name"),
        ),
        (
            "sub/a.ppm",
            hash.as_str(),
            "frame 1.00",
            Some("plain file name"),
        ),
        (
            "sub\\a.ppm",
            hash.as_str(),
            "frame 1.00",
            Some("plain file name"),
        ),
        (".", hash.as_str(), "frame 1.00", Some("plain file name")),
    ] {
        let original = format!("{original} {hash}");
        let fields = [
            (0, "tick", "1"),
            (1, "zoom", "far"),
            (2, "rotation", "0"),
            (3, "level", "1"),
            (4, "origin", "0 0"),
            (5, "walls", "up"),
            (6, "paused", "yes"),
            (7, "strip", "no"),
            (8, "original", original.as_str()),
            (9, "ours", ours),
            (10, "floors", floors),
        ];
        match (scene("a", &fields), want) {
            (Ok(_), None) => {}
            (Err(e), Some(want)) if e.contains(want) => {}
            (got, _) => {
                return Err(format!(
                    "scene with original {original:?}, ours {ours:?}, floors {floors:?} gives \
                     {:?}, want {want:?}",
                    got.map(|s| s.name)
                ));
            }
        }
    }
    Ok(())
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
    /// Written with `floors = none`: only `SIMS_RATCHET_NEW` sets its
    /// floors.
    new: bool,
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
    let mut names = std::collections::BTreeSet::new();
    if let Some(twice) = scenes.iter().find(|s| !names.insert(s.name.as_str())) {
        return Err(format!("two scenes named {:?}", twice.name));
    }
    if let Some(missing) = BASELINE.iter().find(|b| !names.contains(*b)) {
        return Err(format!(
            "no `[{missing}]`: the baseline scenes stay in the manifest"
        ));
    }
    if let Some(reset) = scenes
        .iter()
        .find(|s| s.new && BASELINE.contains(&s.name.as_str()))
    {
        return Err(format!(
            "`[{}]` is a baseline scene: it keeps its hash and floors (`none` is only for a \
             new scene)",
            reset.name
        ));
    }
    Ok(scenes)
}

/// A floor in hundredths of a percent: `96.51` is 9651. A floor is
/// written with digits, a point and exactly two decimals, at most
/// `100.00`.
fn hundredths(word: &str) -> Option<u32> {
    let (whole, part) = word.split_once('.')?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    // At most three whole digits before any arithmetic, so nothing can
    // overflow.
    if !digits(whole) || whole.len() > 3 || part.len() != 2 || !digits(part) {
        return None;
    }
    let value = whole.parse::<u32>().ok()? * 100 + part.parse::<u32>().ok()?;
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
    // A plain file name: the references are the files in SIMS_RATCHET.
    let plain = matches!(
        Path::new(file).components().collect::<Vec<_>>()[..],
        [std::path::Component::Normal(_)]
    );
    if !plain || file.contains(['/', '\\']) {
        return Err(format!(
            "{name}: `original` {file:?} is not a plain file name inside SIMS_RATCHET"
        ));
    }
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
    // `floors = none` with `ours = none` starts a new scene:
    // SIMS_RATCHET_NEW sets both. One without the other is an error.
    let new = floor_words == "none";
    if new != (ours == "none") {
        return Err(format!(
            "{name}: a new scene has both `ours = none` and `floors = none`, not one of them"
        ));
    }
    let words: Vec<&str> = if new {
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
        new,
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

/// Where the difference images go: a new folder this run makes inside
/// the folder `SIMS_RATCHET_DIFF` names, so no earlier file or link is
/// written through.
struct DiffOut {
    dir: PathBuf,
}

impl DiffOut {
    fn create(parent: &Path) -> Result<DiffOut, String> {
        let root = checkout_root()?;
        let parent = parent
            .canonicalize()
            .map_err(|e| format!("SIMS_RATCHET_DIFF {parent:?}: {e}"))?;
        Self::allowed(&parent, &root)?;
        // A name no earlier run used: the first free `ratchet-<pid>-<n>`.
        let mut n = 0;
        let dir = loop {
            let dir = parent.join(format!("ratchet-{}-{n}", std::process::id()));
            match std::fs::create_dir(&dir) {
                Ok(()) => break dir,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && n < 1000 => n += 1,
                Err(e) => return Err(format!("{dir:?}: {e}")),
            }
        };
        let dir = dir.canonicalize().map_err(|e| format!("{dir:?}: {e}"))?;
        Self::allowed(&dir, &root)?;
        Ok(DiffOut { dir })
    }

    /// Under a folder named `cache`, and outside the checkout (its own
    /// `cache/` included), links already resolved.
    fn allowed(dir: &Path, root: &Path) -> Result<(), String> {
        if dir.starts_with(root) {
            return Err(format!(
                "SIMS_RATCHET_DIFF {dir:?} is inside the checkout {root:?}"
            ));
        }
        if !dir.components().any(|c| c.as_os_str() == "cache") {
            return Err(format!(
                "SIMS_RATCHET_DIFF {dir:?} is not under a folder named `cache`"
            ));
        }
        Ok(())
    }

    /// Writes a new file; an existing file or link of that name fails.
    fn write(&self, file: &str, bytes: &[u8]) -> Result<(), String> {
        let path = self.dir.join(file);
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .and_then(|mut f| f.write_all(bytes))
            .map_err(|e| format!("{path:?}: {e}"))
    }
}

/// Replaces the manifest whole, if it is unchanged since the run read it
/// (`read` is its SHA-256 then): a new file beside it, then a rename.
fn replace_manifest(path: &Path, read: &str, text: &str) -> Result<(), String> {
    let now = std::fs::read(path).map_err(|e| format!("{path:?}: {e}"))?;
    if sha256(&now) != read {
        return Err(format!(
            "{path:?} changed since the run read it: nothing written; run again"
        ));
    }
    let new = path.with_extension(format!("txt.new-{}", std::process::id()));
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&new)
        .and_then(|mut f| f.write_all(text.as_bytes()).and_then(|()| f.sync_all()))
        .and_then(|()| std::fs::rename(&new, path));
    if let Err(e) = written {
        let _ = std::fs::remove_file(&new);
        return Err(format!("{path:?}: {e}"));
    }
    Ok(())
}

fn run(config: &Config) -> Result<(), Vec<String>> {
    let fail = |e: String| vec![e];
    let path = manifest_path();
    let bytes = std::fs::read(&path).map_err(|e| fail(format!("{path:?}: {e}")))?;
    let read = sha256(&bytes);
    let text = String::from_utf8(bytes).map_err(|e| fail(format!("{path:?}: {e}")))?;
    let scenes = parse(&text).map_err(|e| fail(format!("{path:?}: {e}")))?;
    for name in &config.new {
        if BASELINE.contains(&name.as_str()) {
            return Err(fail(format!(
                "SIMS_RATCHET_NEW names {name:?}, a baseline scene: its floors are never set \
                 anew"
            )));
        }
        match scenes.iter().find(|s| &s.name == name) {
            Some(s) if s.new => {}
            Some(_) => {
                return Err(fail(format!(
                    "SIMS_RATCHET_NEW names {name:?}, which has floors: only a scene \
                     written with `floors = none` is new"
                )));
            }
            None => return Err(fail(format!("SIMS_RATCHET_NEW names no scene {name:?}"))),
        }
    }
    let diff = config
        .diff
        .as_deref()
        .map(DiffOut::create)
        .transpose()
        .map_err(fail)?;
    if let Some(diff) = &diff {
        println!("ratchet: difference images in {:?}", diff.dir);
    }

    // Every reference first: a changed screenshot stops the run.
    let corpus = config
        .references
        .canonicalize()
        .map_err(|e| fail(format!("SIMS_RATCHET {:?}: {e}", config.references)))?;
    let mut originals = Vec::new();
    for scene in &scenes {
        let name = &scene.name;
        let file = config.references.join(&scene.file);
        // With links resolved, the file is still in the folder.
        let real = file
            .canonicalize()
            .map_err(|e| fail(format!("{name}: {file:?}: {e}")))?;
        if real.parent() != Some(corpus.as_path()) {
            return Err(fail(format!(
                "{name}: {file:?} resolves to {real:?}, outside SIMS_RATCHET {corpus:?}"
            )));
        }
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

    let install = &config.install;
    let mut game = Game::open(&Options {
        house: goth_house(install),
        install: install.clone(),
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
        let starting = config.new.contains(name);
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
        let mut image = diff.as_ref().map(|_| Canvas::new(WIDTH, HEIGHT, [0, 0, 0]));
        let shares = compare(
            &game,
            &canvas,
            &originals[i],
            &scene.leave_out,
            image.as_mut(),
        );
        if let (Some(diff), Some(image)) = (&diff, &image) {
            for (file, picture) in [
                (format!("{name}.png"), image),
                (format!("{name}-ours.png"), &canvas),
            ] {
                if let Err(e) = diff.write(&file, &sims_render::png::encode(picture)) {
                    failures.push(format!("{name}: {e}"));
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

        if shares["frame"].1 == 0 {
            failures.push(format!("{name}: every pixel is left out"));
            continue;
        }
        let ours = sha256(&canvas.color.concat());
        if ours != scene.ours_sha256 {
            if config.update || starting {
                lines[scene.ours_line] = format!("ours = {ours}");
            } else {
                failures.push(format!(
                    "{name}: our frame's SHA-256 is {ours}, not the pinned {}: re-pin it \
                     (SIMS_RATCHET_UPDATE=1) and give the new shares in the PR",
                    scene.ours_sha256
                ));
            }
        }
        if scene.new && !starting {
            failures.push(format!(
                "{name} has no floors yet: set them with SIMS_RATCHET_NEW={name}"
            ));
            continue;
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
                _ if starting => share,
                Some(floor) if share < floor => {
                    failures.push(format!(
                        "{name}: {c} matches {}%, below its floor {}%",
                        shown(share),
                        shown(floor)
                    ));
                    floor
                }
                Some(floor) if config.update => floor.max(share),
                Some(floor) => floor,
                None => {
                    failures.push(format!(
                        "{name}: {c} matches {}% and has no floor: an existing scene's floors \
                         are kept by hand; only SIMS_RATCHET_NEW sets a new scene's",
                        shown(share)
                    ));
                    continue;
                }
            };
            floors.push(format!("{c} {}", shown(kept)));
        }
        if config.update || starting {
            lines[scene.floors_line] = format!("floors = {}", floors.join(" "));
        }
    }
    let writes = config.update || !config.new.is_empty();
    // A run that failed writes nothing: it must not re-pin a frame that
    // fails, or keep a floor it could not meet.
    if writes && failures.is_empty() {
        let text = lines.join("\n") + "\n";
        replace_manifest(&path, &read, &text).map_err(fail)?;
        println!("ratchet: wrote {path:?}");
    } else if writes {
        println!("ratchet: wrote nothing: the run failed");
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures)
    }
}
