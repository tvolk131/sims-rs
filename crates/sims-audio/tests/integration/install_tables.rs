//! Every sound table of a real install parses and holds together. Needs
//! `SIMS_INSTALL`; skips without it. Pins counts, no game data.

use sims_audio::files::SoundFiles;
use sims_audio::fsc::Score;
use sims_audio::hit::{JUMP, Operand};
use sims_audio::module::Module;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

fn install() -> Option<PathBuf> {
    match std::env::var_os("SIMS_INSTALL") {
        Some(path) => Some(PathBuf::from(path)),
        None => {
            eprintln!("SIMS_INSTALL is not set; skipping the install sound tables");
            None
        }
    }
}

#[test]
fn shadowed_copies_of_the_tables_are_identical() {
    let Some(root) = install() else { return };
    let files = SoundFiles::scan(&root).unwrap();
    let mut compared = 0;
    for shadow in &files.shadowed {
        if !["hit", "hsm", "hot", "fsc"].contains(&shadow.extension().as_str()) {
            continue;
        }
        let winner = files.get(&shadow.path).unwrap();
        assert_eq!(
            shadow.read().unwrap(),
            winner.read().unwrap(),
            "{}",
            shadow.path
        );
        compared += 1;
    }
    assert_eq!(compared, 81);
}

#[test]
fn every_sound_module_parses_and_its_references_resolve() {
    let Some(root) = install() else { return };
    let files = SoundFiles::scan(&root).unwrap();
    let modules = Module::load_all(&files).unwrap();
    let names: Vec<&str> = modules.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names.len(), 27, "{names:?}");

    let mut opcodes: BTreeMap<&str, usize> = BTreeMap::new();
    let mut jumps: BTreeMap<&str, usize> = BTreeMap::new();
    let mut totals: BTreeMap<&str, usize> = BTreeMap::new();
    // Problems: the decoding rules are wrong. Must be none.
    let mut problems = Vec::new();
    // Gaps: what the files themselves leave dangling, counted by kind.
    let mut gaps: BTreeMap<&str, usize> = BTreeMap::new();
    let mut gap = |kind: &'static str, detail: String| {
        eprintln!("gap: {kind}: {detail}");
        *gaps.entry(kind).or_default() += 1;
    };
    // Patches of every module, for hit lists and tracks that name a
    // patch another module defines.
    let all_patches: BTreeSet<u32> = modules
        .iter()
        .flat_map(|m| m.hot.patches.keys().copied())
        .collect();
    let all_tracks: BTreeSet<u32> = modules
        .iter()
        .flat_map(|m| m.hot.tracks.keys().copied())
        .collect();
    for module in &modules {
        let name = &module.name;
        let code = module
            .program
            .disassemble(&|id| module.is_target(id))
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let starts: BTreeSet<u32> = code.iter().map(|i| i.address).collect();
        *totals.entry("instructions").or_default() += code.len();
        for instruction in &code {
            *opcodes.entry(instruction.name()).or_default() += 1;
            if instruction.opcode == JUMP {
                let form = match instruction.operands[0].0 {
                    Operand::Target => "track data ID",
                    _ => "register",
                };
                *jumps.entry(form).or_default() += 1;
            }
            for &(kind, value) in &instruction.operands {
                if kind == Operand::Target && !module.is_target(value as u32) {
                    problems.push(format!(
                        "{name}: {:#x} names no track data",
                        instruction.address
                    ));
                }
            }
        }
        for (&id, &address) in &module.hot.track_data {
            if !starts.contains(&address) {
                problems.push(format!(
                    "{name}: track data {id:#x} at {address:#x} is mid-instruction"
                ));
            }
        }
        // Every HSM label `tkd_X = id` with a label `X = address` agrees
        // with the HOT file's track data.
        // with the HOT file's track data. The HSM writes each pair on
        // consecutive lines; a name may repeat (`tkd_Generic` comes once
        // per included source), so pairs are read in order, not by name.
        for pair in module.symbols.lines.windows(2) {
            let [(symbol, value), (label, address)] = pair else {
                continue;
            };
            if symbol.strip_prefix("tkd_") != Some(label.as_str()) {
                continue;
            }
            *totals.entry("tkd labels").or_default() += 1;
            if module.address(*value as u32) != Some(*address as u32) {
                problems.push(format!("{name}: {symbol} disagrees with [TrackData]"));
            }
        }
        for (&id, track) in &module.hot.tracks {
            *totals.entry("tracks").or_default() += 1;
            match module.entry(id) {
                Some(entry) if module.is_target(entry) => {}
                Some(0) => *totals.entry("tracks without an entry point").or_default() += 1,
                _ => gap(
                    "track entry names no track data",
                    format!("{name}: track {id} {:?}", track.entry),
                ),
            }
            match module.number(&track.patch) {
                Some(0) => {}
                Some(patch) if all_patches.contains(&(patch as u32)) => {}
                _ => gap(
                    "track patch names no patch",
                    format!("{name}: track {id} {:?}", track.patch),
                ),
            }
        }
        for patches in module.hot.hit_lists.values() {
            *totals.entry("hit lists").or_default() += 1;
            for patch in patches {
                *totals.entry("hit list items").or_default() += 1;
                if !all_patches.contains(patch) {
                    *totals.entry("hit list items naming no patch").or_default() += 1;
                }
            }
        }
        *totals.entry("hit list items unread").or_default() += module.hot.hit_list_unread.len();
        for (patch_id, patch) in &module.hot.patches {
            *totals.entry("patches").or_default() += 1;
            if files.get(&patch.path).is_none() {
                gap(
                    "patch file not in the install",
                    format!("{name}: patch {patch_id} {}", patch.path),
                );
            }
        }
        for (event, mapping) in &module.hot.events {
            *totals.entry("events").or_default() += 1;
            let Some(kind) = module.number(&mapping.kind) else {
                problems.push(format!("{name}: event {event} kind {:?}", mapping.kind));
                continue;
            };
            *totals
                .entry(if kind == 1 {
                    "events that play"
                } else {
                    "events of other kinds"
                })
                .or_default() += 1;
            if kind != 1 {
                continue;
            }
            match mapping.track.as_ref().and_then(|t| module.number(t)) {
                Some(track) if module.hot.tracks.contains_key(&(track as u32)) => {}
                Some(track) if all_tracks.contains(&(track as u32)) => {
                    *totals
                        .entry("events playing another module's track")
                        .or_default() += 1;
                }
                _ => gap(
                    "event track names no track",
                    format!("{name}: event {event} {:?}", mapping.track),
                ),
            }
        }
    }
    eprintln!("{opcodes:?}\n{jumps:?}\n{totals:?}\n{gaps:?}");
    assert!(
        problems.is_empty(),
        "{} problems: {problems:#?}",
        problems.len()
    );
    // Two jumps, both in the base game's generated module, only decode as
    // register jumps; see `sims_audio::hit`.
    assert_eq!(
        format!("{jumps:?}"),
        r#"{"register": 2, "track data ID": 399}"#
    );
    assert_eq!(
        format!("{totals:?}"),
        concat!(
            r#"{"events": 4494, "events of other kinds": 178, "events that play": 4316, "#,
            r#""hit list items": 19080, "hit list items unread": 6, "hit lists": 6344, "#,
            r#""instructions": 25353, "patches": 17515, "tkd labels": 4363, "tracks": 4316}"#
        )
    );
    // The files' own loose ends: sample files the install lacks (one path
    // even names a build machine's drive), events with an empty track,
    // and entry points named by a symbol the module's HSM file does not
    // define (`tkd_Generic` in one download, `tkd_GenericInterrupt` in two
    // expansion modules; other modules define both).
    assert_eq!(
        format!("{gaps:?}"),
        concat!(
            r#"{"event track names no track": 14, "patch file not in the install": 155, "#,
            r#""track entry names no track data": 10, "track patch names no patch": 4}"#
        )
    );
    assert_eq!(
        format!("{opcodes:?}"),
        concat!(
            r#"{"add": 5, "call": 2504, "cmp": 135, "div": 5, "end": 4166, "getsrcdatafield": 97, "#,
            r#""ifeq": 66, "ifge": 37, "ifgt": 11, "ifle": 6, "iflt": 44, "ifne": 43, "jump": 401, "#,
            r#""kill_trk": 8, "loadb": 381, "loadl": 9826, "loop": 142, "mul": 2, "note_on": 1305, "#,
            r#""play_trk": 2, "rand": 49, "seqgroup_kill": 58, "seqgroup_return": 8, "set": 1242, "#,
            r#""set_loop": 1040, "setll": 2, "setlt": 1, "smart_choose": 1188, "smart_setlist": 1190, "#,
            r#""sub": 1, "test": 23, "test1": 1, "test2": 1, "test3": 1, "test4": 1, "wait": 50, "#,
            r#""wait_samp": 1304, "waiteq": 7}"#
        )
    );
}

/// The directories that hold ambient samples.
const AMBIENT: [&str; 4] = [
    "SoundData\\Outdoors",
    "SoundData\\MTOutdoors",
    "ExpansionShared\\Sound\\Park",
    "ExpansionShared\\Sound\\Beach",
];

#[test]
fn every_ambient_score_parses_and_its_samples_resolve() {
    let Some(root) = install() else { return };
    let files = SoundFiles::scan(&root).unwrap();
    let mut summary = Vec::new();
    for entry in files.iter().filter(|e| e.extension() == "fsc") {
        let score =
            Score::parse(&entry.read().unwrap()).unwrap_or_else(|e| panic!("{}: {e}", entry.path));
        let directory = entry.path.rsplit_once('\\').map_or("", |(d, _)| d);
        let cells: Vec<_> = score.cells.iter().flatten().collect();
        // Where each cell's sample is: the score's own directory, one of
        // the other ambient directories, or nowhere.
        let mut found: BTreeMap<&str, usize> = BTreeMap::new();
        for cell in &cells {
            let in_dir = |d: &str| files.get(&format!("{d}\\{}", cell.file)).is_some();
            let place = if in_dir(directory) {
                "own"
            } else if AMBIENT.iter().any(|d| in_dir(d)) {
                "other"
            } else {
                "missing"
            };
            *found.entry(place).or_default() += 1;
        }
        summary.push(format!(
            "{} v{} {}x{}: {found:?}",
            entry.path, score.version, score.rows, score.columns
        ));
    }
    eprintln!("{summary:#?}");
    // Most samples sit next to their score. The two big scores also use
    // samples of the other ambient directories, and six samples are in no
    // directory at all. Where the original looks is **unverified**.
    assert_eq!(
        summary,
        [
            r#"ExpansionShared\Sound\Beach\beach.fsc v7 7x14: {"own": 67}"#,
            r#"ExpansionShared\Sound\Park\hotdatepark_amb.fsc v7 8x14: {"own": 106}"#,
            r#"SoundData\MTOutdoors\magic_amb.fsc v7 26x14: {"missing": 2, "other": 50, "own": 170}"#,
            r#"SoundData\Outdoors\sim_amb.fsc v7 26x14: {"missing": 4, "other": 6, "own": 352}"#,
        ]
    );
}

/// Follows an event from its name to the HIT code that runs it and the
/// samples it can play: the path the VM will take.
fn trace(modules: &[Module], files: &SoundFiles, event: &str) -> String {
    let (module, mapping) = modules
        .iter()
        .find_map(|m| m.hot.event(event).map(|e| (m, e)))
        .unwrap_or_else(|| panic!("no event {event}"));
    let kind = module.number(&mapping.kind).unwrap();
    let track = module.number(mapping.track.as_ref().unwrap()).unwrap() as u32;
    let entry = module.entry(track).unwrap();
    let address = module.address(entry).unwrap();
    let patch = module.number(&module.hot.tracks[&track].patch).unwrap() as u32;
    let mut code = Vec::new();
    let mut at = address;
    loop {
        let instruction = module
            .program
            .decode(at, &|id| module.is_target(id))
            .unwrap();
        code.push(format!("{} {:?}", instruction.name(), instruction.values()));
        at += instruction.length;
        if instruction.name() == "end" {
            break;
        }
    }
    let sample = |patch: u32| {
        let path = &module.hot.patches[&patch].path;
        assert!(files.get(path).is_some(), "{path}");
        path.clone()
    };
    let patch = if patch == 0 {
        String::from("none")
    } else {
        sample(patch)
    };
    format!(
        "{} kind {kind} track {track} entry {entry:#x} at {address:#x} patch {patch}: {}",
        module.name,
        code.join("; ")
    )
}

#[test]
fn a_phone_ring_and_a_footstep_lead_to_their_samples() {
    let Some(root) = install() else { return };
    let files = SoundFiles::scan(&root).unwrap();
    let modules = Module::load_all(&files).unwrap();
    assert_eq!(
        trace(&modules, &files, "Phone_ring"),
        "sounddata\\simsgeneratedhitsource kind 1 track 3479 entry 0x100 at 0x10 \
         patch sounddata\\phone\\Phone_ring.xa: note_on [5]; wait_samp []; end []"
    );
    // A footstep sets `interrupt` (register 55), then compares 2 with the
    // game speed (global 100) and branches to the routine that picks one
    // of five soft footstep samples (hit list 217) when they differ.
    assert_eq!(
        trace(&modules, &files, "footstep_soft"),
        "sounddata\\simsgeneratedhitsource kind 1 track 1910 entry 0x193 at 0x9b1 \
         patch none: loadb [55, 1]; loadb [5, 2]; cmp [5, 100]; ifne [404]; end []"
    );
    let generated = modules
        .iter()
        .find(|m| m.name == "sounddata\\simsgeneratedhitsource")
        .unwrap();
    let footsteps: Vec<&str> = generated.hot.hit_lists[&217]
        .iter()
        .map(|p| generated.hot.patches[p].path.as_str())
        .collect();
    assert_eq!(footsteps.len(), 5);
    assert!(
        footsteps
            .iter()
            .all(|p| p.contains("footstep_soft") && files.get(p).is_some())
    );
}
