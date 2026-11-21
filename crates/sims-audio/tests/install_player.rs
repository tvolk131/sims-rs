//! Real sound events played end to end into PCM. Needs `SIMS_INSTALL`;
//! skips without it. Pins digests of rendered PCM, no game data.

use sha2::{Digest, Sha256};
use sims_audio::files::SoundFiles;
use sims_audio::module::Module;
use sims_audio::player::{BLOCK, NoSources, Player, RATE, Samples};
use sims_audio::stations::StationLists;
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
fn the_registry_grows_to_24593_buckets() {
    // Every patch and track line registers an object (21,809 IDs, three
    // shared by a patch and a track), then every station patch (366
    // more): the unpause walks 24,593 buckets.
    let Some(root) = install() else { return };
    let files = SoundFiles::scan(&root).unwrap();
    let mut player = player(&root, 1);
    let tables = player.registry().len();
    player.add_stations(&StationLists::build(&files));
    let registry = player.registry();
    assert_eq!(
        (tables, registry.len(), registry.buckets()),
        (21_809, 22_175, 24_593)
    );
}

#[test]
fn the_phone_rings() {
    let Some(root) = install() else { return };
    let files = SoundFiles::scan(&root).unwrap();
    let mut player = player(&root, 1);
    player.handle(
        &SoundEvent::Play {
            object: 7,
            name: "Phone_ring".into(),
        },
        &NoSources,
    );
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
        player.handle(
            &SoundEvent::Play {
                object: 3,
                name: "footstep_soft".into(),
            },
            &NoSources,
        );
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
    player.handle(
        &SoundEvent::Play {
            object: 3,
            name: "footstep_soft".into(),
        },
        &NoSources,
    );
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
    player.add_stations(&StationLists::build(&files));
    let mut faults: BTreeMap<String, usize> = BTreeMap::new();
    let mut unported: BTreeMap<String, usize> = BTreeMap::new();
    let mut kinds: BTreeMap<i64, usize> = BTreeMap::new();
    let mut still_running = 0;
    for (module, name) in &definitions {
        player.handle(&SoundEvent::StopAll, &NoSources);
        player.faults.clear();
        player.play_from(*module, 1, name, &NoSources);
        player.render(20 * BLOCK, &NoSources);
        if !player.is_idle() {
            still_running += 1;
        }
        for fault in &player.faults {
            if let Some(at) = fault.find(" is not one this player plays") {
                let kind = fault[..at].rsplit(' ').next().unwrap();
                *kinds.entry(kind.parse().unwrap()).or_default() += 1;
            }
            if let Some(at) = fault.find(" is not supported") {
                let instruction = fault[..at].rsplit(' ').next().unwrap();
                *unported.entry(instruction.to_owned()).or_default() += 1;
            }
            let kind = [
                ("is not one this player plays", "event of a kind not ported"),
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
    // remains are the files' loose ends (see `install_tables.rs`), 12
    // events with an empty track field (the original's lookup of track 0
    // finds nothing, as here) and the 2 events of kinds not ported:
    // pausing (103) and reloading (104). The 5 ambience modes (52) start
    // and stop scores, none of which this test loads. Of the 186 events
    // still running after 20 blocks, 5 change the music mode (36) and
    // start music or a river loop, or a slow fade of the music, that runs
    // on; the piano (43) plays notes for ever; the 60 that turn on a
    // television or radio play their station for ever; 119 more loop
    // (fires, appliances, voices that loop) and one,
    // `Grimreaper_loop_kill`, fades its own vol for 21 blocks: the loop
    // track it targets is not running, so `target` is itself.
    // (`shoo_tub_inside_vox` waits a random time, which the station
    // scripts' draws make short here. `ltub_running` starts nothing: a
    // later `[Patch]` line registers its track 68262 again as a patch.)
    assert_eq!(definitions.len(), 4494);
    assert_eq!((still_running, format!("{unported:?}")), (186, "{}".into()));
    assert_eq!(format!("{kinds:?}"), "{103: 1, 104: 1}");
    assert_eq!(
        format!("{faults:?}"),
        "{\"event names no track\": 14, \"event of a kind not ported\": 2, \
         \"hit list not found\": 1, \"patch not found\": 3, \"track entry point not found\": 10}"
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

#[test]
fn the_stations_are_built_from_their_directories() {
    let Some(root) = install() else { return };
    let files = SoundFiles::scan(&root).unwrap();
    let lists = StationLists::build(&files);
    // Each station's sub lists and patches.
    let counts: Vec<String> = sims_audio::stations::STATIONS
        .iter()
        .map(|&(number, _)| {
            let subs = &lists.hit_lists[&number];
            let patches: usize = subs.iter().map(|s| lists.hit_lists[s].len()).sum();
            format!("{number}:{}/{patches}", subs.len())
        })
        .collect();
    let listing: String = lists
        .hit_lists
        .iter()
        .map(|(id, list)| format!("{id}={list:?}\n"))
        .chain(
            lists
                .patches
                .iter()
                .map(|(id, path)| format!("{id}={path}\n")),
        )
        .collect();
    let digest: String = Sha256::digest(listing.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    // Music stations and modes are one directory each; the television
    // stations have a subdirectory per programme.
    assert_eq!(
        counts.join(" "),
        "232:1/7 233:1/5 234:1/5 235:1/5 221:1/4 222:1/6 223:1/5 103100:1/7 103101:1/1 \
         103102:1/7 103104:1/5 103000:9/33 103001:7/22 103002:6/26 103003:7/23 100100:1/5 \
         100102:1/6 100101:1/5 20900:1/8 20901:1/6 20902:1/6 14093:1/8 6100:1/8 65536:1/9 \
         65537:1/5 529:5/20 530:11/60 528:5/18 531:5/16 1586:1/5 1587:1/5 1588:1/5 1589:1/5 \
         1590:1/5"
    );
    assert_eq!(
        digest,
        "75cec23ec070f8669924b6d6a0c8750f38025ee86dc211b0f1c719abb6e73ad2"
    );
}

#[test]
fn a_television_plays_its_station() {
    let Some(root) = install() else { return };
    let files = SoundFiles::scan(&root).unwrap();
    let mut player = player(&root, 1);
    player.add_stations(&StationLists::build(&files));
    player.handle(
        &SoundEvent::Play {
            object: 5,
            name: "tv_action_cheap".into(),
        },
        &NoSources,
    );
    let out = player.render(2 * RATE as usize, &NoSources);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
    assert!(out.iter().any(|&s| s != 0));
    assert_eq!(
        digest(&out),
        "97ff325be5165800ba4cd9fc40679eb52741fb2cf9e2ddfe06f0d2eaaec5e9c1"
    );
    // The set going off ends it.
    player.handle(&SoundEvent::StopObject { object: 5 }, &NoSources);
    player.render(RATE as usize, &NoSources);
    assert!(player.is_idle());
}

#[test]
fn the_backyard_ambience_plays() {
    let Some(root) = install() else { return };
    let files = SoundFiles::scan(&root).unwrap();
    let mut player = player(&root, 3);
    player.add_scores(&files);
    player.add_stations(&sims_audio::stations::StationLists::build(&files));
    assert!(player.faults.is_empty(), "{:?}", player.faults);
    // As on entering a lot from the neighbourhood: the music modes are
    // recorded only once the neighbourhood's has been set.
    for name in ["bkground_nhood1", "setambience_backyard", "bkground_live"] {
        player.handle(
            &SoundEvent::Play {
                object: 0,
                name: name.into(),
            },
            &NoSources,
        );
    }
    assert!(player.faults.is_empty(), "{:?}", player.faults);
    // Noon, with the camera's value at 50: row 500 per mille.
    player.set_ambience_view(50, 12);
    let ambience = player.ambience().unwrap();
    assert_eq!((ambience.mode(), ambience.is_started()), (0, true));
    let out = player.render_stereo(60 * RATE as usize, &NoSources);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
    let ambience = player.ambience().unwrap();
    // The neighbourhood's music starts and live's fades it out within a
    // second; its scripts draw on the random numbers the ambience's
    // cells are picked with.
    let loud = out.iter().filter(|s| s.unsigned_abs() > 64).count();
    assert!(loud > out.len() / 20, "{loud} of {}", out.len());
    assert_eq!(
        format!(
            "{:?} {} {} {}",
            ambience.cursor(),
            ambience.active(),
            out.len(),
            digest(&out)
        ),
        "(13, 6) 3 2646000 43a4d0f5cfcb9688209f7f1760202de8ddbe276b450fbd6b35484c700e24f055"
    );
}
