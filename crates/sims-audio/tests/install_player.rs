//! Real sound events played end to end into PCM. Needs `SIMS_INSTALL`;
//! skips without it. Pins digests of rendered PCM, no game data.

use sha2::{Digest, Sha256};
use sims_audio::files::SoundFiles;
use sims_audio::module::Module;
use sims_audio::player::{BLOCK, NoSources, Player, RATE, Samples};
use sims_audio::{Pcm, SoundEvent};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn install() -> Option<PathBuf> {
    match std::env::var_os("SIMS_INSTALL") {
        Some(path) => Some(PathBuf::from(path)),
        None => {
            eprintln!("SIMS_INSTALL is not set; skipping the install sound player");
            None
        }
    }
}

fn player(root: &Path, seed: u64) -> Player<SoundFiles> {
    let files = SoundFiles::scan(root).unwrap();
    let modules = Module::load_all(&files).unwrap();
    Player::new(modules, files, seed)
}

fn decoded(files: &SoundFiles, path: &str) -> Vec<i16> {
    let pcm = sims_audio::decode::decode(&files.get(path).unwrap().read().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!((pcm.rate, pcm.channels), (RATE, 1), "{path}");
    pcm.samples
}

fn digest(samples: &[i16]) -> String {
    let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The rendered sound is the sample, then silence to the end of the block
/// in which it ended.
fn assert_plays(out: &[i16], sample: &[i16]) {
    assert!(
        out.len() >= sample.len(),
        "{} < {}",
        out.len(),
        sample.len()
    );
    assert!(
        out[..sample.len()] == *sample,
        "rendered PCM differs from the sample"
    );
    assert!(out[sample.len()..].iter().all(|&s| s == 0));
    assert!(out.len() - sample.len() < BLOCK);
}

#[test]
fn the_phone_rings() {
    let Some(root) = install() else { return };
    let files = SoundFiles::scan(&root).unwrap();
    let mut player = player(&root, 1);
    player.handle(&SoundEvent::Play {
        object: 7,
        name: "Phone_ring".into(),
    });
    let out = player.render_until_idle(10 * RATE as usize, &NoSources);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
    assert!(player.is_idle());
    assert_plays(&out, &decoded(&files, "sounddata\\phone\\Phone_ring.xa"));
    assert_eq!(
        format!("{} {}", out.len(), digest(&out)),
        "57600 7c80c9c31e36190b39af9de8232ab099624491d07d901120cbcaf01d91866640"
    );
}

#[test]
fn a_sim_takes_a_soft_footstep() {
    let Some(root) = install() else { return };
    let files = SoundFiles::scan(&root).unwrap();
    let footsteps: Vec<(String, Vec<i16>)> = (1..=5)
        .map(|n| format!("sounddata\\footstep\\footstep_soft{n}.xa"))
        .map(|p| {
            let samples = decoded(&files, &p);
            (p, samples)
        })
        .collect();
    let mut chosen = Vec::new();
    for seed in 1..=8 {
        let mut player = player(&root, seed);
        // `SimSpeed` (global 100) at 1: the routine goes on to play.
        player.set_global(100, 1);
        player.handle(&SoundEvent::Play {
            object: 3,
            name: "footstep_soft".into(),
        });
        let out = player.render_until_idle(5 * RATE as usize, &NoSources);
        assert!(player.faults.is_empty(), "{:?}", player.faults);
        let trimmed = out.len() - out.iter().rev().take_while(|&&s| s == 0).count();
        let Some((path, sample)) = footsteps
            .iter()
            .find(|(_, s)| s.len() >= trimmed && out[..s.len().min(out.len())] == s[..])
        else {
            panic!(
                "seed {seed}: rendered {} frames, not a footstep sample",
                out.len()
            );
        };
        assert_plays(&out, sample);
        chosen.push(format!(
            "{seed}: {} {}",
            path.rsplit('\\').next().unwrap(),
            &digest(&out)[..16]
        ));
    }
    // Each seed picks one of the five samples and plays it unchanged.
    assert_eq!(
        chosen,
        [
            "1: footstep_soft4.xa a26534af81aefb44",
            "2: footstep_soft1.xa d87bc948c3a4427e",
            "3: footstep_soft1.xa d87bc948c3a4427e",
            "4: footstep_soft1.xa d87bc948c3a4427e",
            "5: footstep_soft3.xa 876e27b375a26b71",
            "6: footstep_soft3.xa 876e27b375a26b71",
            "7: footstep_soft1.xa d87bc948c3a4427e",
            "8: footstep_soft4.xa a26534af81aefb44",
        ]
    );

    // At `SimSpeed` 2 the routine ends without a note.
    let mut player = player(&root, 1);
    player.set_global(100, 2);
    player.handle(&SoundEvent::Play {
        object: 3,
        name: "footstep_soft".into(),
    });
    let out = player.render_until_idle(RATE as usize, &NoSources);
    assert!(player.is_idle() && player.faults.is_empty());
    assert!(out.is_empty());
}

/// Hands every patch the same short silent sample, so every event can
/// run without decoding the install.
struct Stub;

impl Samples for Stub {
    fn load(&mut self, _: &str) -> Option<Pcm> {
        Some(Pcm {
            rate: RATE,
            channels: 1,
            samples: vec![0; 200],
        })
    }
}

#[test]
fn every_event_definition_runs_without_a_bad_instruction() {
    let Some(root) = install() else { return };
    let files = SoundFiles::scan(&root).unwrap();
    let modules = Module::load_all(&files).unwrap();
    // Every definition, module by module: a name several modules define
    // is run once per module, not only as the module that wins.
    let definitions: Vec<(usize, String)> = modules
        .iter()
        .enumerate()
        .flat_map(|(index, m)| m.hot.events.iter().map(move |(n, _)| (index, n.clone())))
        .collect();
    let mut player = Player::new(modules, Stub, 1);
    let mut faults: BTreeMap<String, usize> = BTreeMap::new();
    let mut unported: BTreeMap<String, usize> = BTreeMap::new();
    let mut still_running = 0;
    for (module, name) in &definitions {
        player.handle(&SoundEvent::StopAll);
        player.faults.clear();
        player.play_from(*module, 1, name);
        player.render(20 * BLOCK, &NoSources);
        if !player.is_idle() {
            still_running += 1;
        }
        for fault in &player.faults {
            if let Some(at) = fault.find(" is not supported") {
                let instruction = fault[..at].rsplit(' ').next().unwrap();
                *unported.entry(instruction.to_owned()).or_default() += 1;
            }
            let kind = [
                (
                    "is not one this player plays",
                    "event of a kind other than play",
                ),
                ("is not supported", "instruction not ported"),
                ("entry point not found", "track entry point not found"),
                ("no track", "event names no track"),
                ("no hit list", "hit list not found"),
                ("no patch", "patch not found"),
            ]
            .iter()
            .find(|(text, _)| fault.contains(text))
            .map_or(fault.as_str(), |(_, kind)| kind);
            *faults.entry(kind.to_owned()).or_default() += 1;
        }
    }
    // No instruction is decoded wrong and none reached is unported. What
    // remains are the files' loose ends (see `install_tables.rs`) and the
    // 190 events of other kinds (music modes, television, sequenced hit
    // lists and more). The 121 events still running after 20 blocks loop
    // (fires, appliances, voices that loop).
    assert_eq!(definitions.len(), 4494);
    assert_eq!((still_running, format!("{unported:?}")), (121, "{}".into()));
    assert_eq!(
        format!("{faults:?}"),
        "{\"event names no track\": 2, \"event of a kind other than play\": 190, \
         \"hit list not found\": 1, \"patch not found\": 1, \"track entry point not found\": 10}"
    );
}

#[test]
fn the_first_module_defining_an_event_name_wins() {
    let Some(root) = install() else { return };
    let files = SoundFiles::scan(&root).unwrap();
    let modules = Module::load_all(&files).unwrap();
    // Names (ignoring case) defined by more than one module, and how many
    // of the shadowed definitions differ from the one that wins.
    let mut first: BTreeMap<String, (usize, &sims_audio::hot::Event)> = BTreeMap::new();
    let mut shadowed = Vec::new();
    let mut differ = 0;
    for (index, module) in modules.iter().enumerate() {
        for (name, event) in &module.hot.events {
            let key = name.to_ascii_lowercase();
            match first.get(&key) {
                Some(&(winner, wins)) => {
                    shadowed.push(format!(
                        "{key}: {} over {}",
                        modules[winner].name, module.name
                    ));
                    if wins != event {
                        differ += 1;
                    }
                }
                None => {
                    first.insert(key, (index, event));
                }
            }
        }
    }
    assert_eq!((shadowed.len(), differ), (13, 5), "{shadowed:#?}");
}
