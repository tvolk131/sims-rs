//! `sims_vm::boot` on a real install, when `SIMS_INSTALL` names one: the
//! Goth house loads and steps one tick at a time to the state `sims boot`
//! reaches.

use sims_core::CrtRng;
use sims_sim::{Headline, World};
use sims_vm::boot::{Frames, Lot, StepError};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// `sims boot <install> UserData/Houses/House05.iff --ticks 33 --answers 3`
/// gives this hash with the default frames (snapshot format 30).
const TICK_33: &str = "cb8c3c739e9152c4ce5e52dcb00917ed3f442f693da2236eb83e228e70821af2";
/// The same state's hash under snapshot format 29, which had no person
/// types made while the game runs ([`sims_sim::World::characters`]) at
/// the end.
const TICK_33_29: &str = "2638e19cbc85b069bb535a578ad198636d1fac4c136ef9e14276ace6349a3e76";
/// The same state's hash under snapshot format 28, which had no record
/// of the people whose animator was asked to stop its route
/// ([`sims_sim::World::stops_asked`]) at the end.
const TICK_33_28: &str = "d80e6bce974c4dee88c971c085394f566f7c27a277ad53e87c92d9d43b24767b";
/// The same state's hash under snapshot format 28 before the load kept
/// the people's saved head targets ([`without_saved_targets`]).
const TICK_33_UNTARGETED: &str = "52c4c0712c1a4f0ada5ca1de5ff4c150f7679011940a83d2d94549f4ce657941";
/// The same state's hash under snapshot format 27, which had no poses
/// the load left in the skins ([`sims_sim::World::held_poses`]) either.
const TICK_33_27: &str = "b589af8b447c8423b2eccc225c5369afb447769147e0eccc8f7948b5e85d742f";
/// The same state's hash under snapshot format 26, which had no
/// neighborhood data words ([`sims_sim::World::neighborhood_data`]) at
/// the end.
const TICK_33_26: &str = "6d75241b287a461413772353407deaa478c650786a071f394af53a0615877c17";
/// The same state's hash under snapshot format 25, which had no record of
/// the bones the skins moved ([`sims_sim::World::translated_bones`])
/// either.
const TICK_33_25: &str = "d1597b36e4f366b626771f3bdadc24971d9d3d3293cdb3971c0ff1c2070fbe4a";
/// The same state's hash under snapshot format 24, which had no base
/// layers ([`sims_sim::World::base_layers`]) either.
const TICK_33_24: &str = "f168838ed577652092d9d0f63d73c673ae6e35aa611cd7ce9b7763b1b1590016";
/// The same state's hash under snapshot format 23, which had no headlines
/// ([`sims_sim::World::headlines`]) either.
const TICK_33_23: &str = "9793d7b1dbe85cc28bdee601c5c48a3311c0b360188a34f9b1ce3a1fc1e24246";
/// The same state's hash under snapshot format 22, which had no families
/// ([`sims_sim::World::families`]) either.
const TICK_33_22: &str = "df613032818282316988f8f6da93c53ece0e93eb12e268c6f5b40b1f994f96e7";

/// `sims boot <install> UserData/Houses/House05.iff --ticks 1103 --answers
/// 3` gives this hash, the tick before the salesman leaves.
const TICK_1103: &str = "2de410b64f9b2ea57d1a1baad6eec85d655dd106c74a9465548b9a4590ffffe5";
/// The same state's hash under snapshot format 29, which had no person
/// types made while the game runs ([`sims_sim::World::characters`]) at
/// the end.
const TICK_1103_29: &str = "ff408bffa41b5e0634b688f511ed9efc8c162313cea6d17ad4d541fd6c84c413";
/// The same state's hash under snapshot format 28, which had no record
/// of the people whose animator was asked to stop its route
/// ([`sims_sim::World::stops_asked`]) at the end.
const TICK_1103_28: &str = "40fd32a2e877c42ba2d8a3a75dc2f73283306330384ce5b0f887b857b502e7a4";
/// The same state's hash under snapshot format 27, which had no poses
/// the load left in the skins ([`sims_sim::World::held_poses`]) either.
const TICK_1103_27: &str = "28cd08590f881bc7a17d6ae7b2257646d155f43c4a5897bf2bad690686daf806";
/// The same state's hash under snapshot format 26.
const TICK_1103_26: &str = "eba032ce275585a0e57d3ffaefe893019846999da12efe090fce4c12cd8d24c1";
/// The same state's hash under snapshot format 25.
const TICK_1103_25: &str = "e6694cc922b8164e7ef3c8b690ca9cf67a1037b21568fa7bc944e4997d9c00b5";
/// The same state's hash under snapshot format 24.
const TICK_1103_24: &str = "29bcebd6b87a9b15b737861bf8b4f7ebe13a6f2307607f4b18930f6002653f03";

fn digest(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The world's encoding as snapshot format 29 wrote it: without the
/// person types made while the game runs ([`sims_sim::World::characters`])
/// that format 30 added at the end. The older formats' encodings start
/// from this one.
fn format_29_bytes(world: &World) -> Vec<u8> {
    let mut bytes = world.encode();
    let made = postcard::to_allocvec(&world.characters).unwrap();
    assert_eq!(bytes.split_off(bytes.len() - made.len()), made);
    bytes
}

/// The world's encoding as snapshot format 28 wrote it: without the
/// people whose animator was asked to stop its route
/// ([`sims_sim::World::stops_asked`]) that format 29 added at the end
/// either, so a state pinned under format 28 can be shown to be the same
/// state but for them. It starts from [`format_29_bytes`].
fn format_28_bytes(world: &World) -> Vec<u8> {
    let mut bytes = format_29_bytes(world);
    let asked = postcard::to_allocvec(&world.stops_asked).unwrap();
    assert_eq!(bytes.split_off(bytes.len() - asked.len()), asked);
    bytes
}

/// The world's encoding as snapshot format 27 wrote it: without the poses
/// the load left in the skins ([`sims_sim::World::held_poses`]) that
/// format 28 added at the end either. It starts from [`format_28_bytes`].
fn format_27_bytes(world: &World) -> Vec<u8> {
    let mut bytes = format_28_bytes(world);
    let held = postcard::to_allocvec(&world.held_poses).unwrap();
    assert_eq!(bytes.split_off(bytes.len() - held.len()), held);
    bytes
}

/// The world's encoding as snapshot format 26 wrote it: without the
/// neighborhood's data words ([`sims_sim::World::neighborhood_data`]) that
/// format 27 added at the end, so a state pinned under format 26 can be
/// shown to be the same state but for them.
fn format_26_bytes(world: &World) -> Vec<u8> {
    let mut bytes = format_27_bytes(world);
    let data = postcard::to_allocvec(&world.neighborhood_data).unwrap();
    assert_eq!(bytes.split_off(bytes.len() - data.len()), data);
    bytes
}

/// The world's encoding as snapshot format 25 wrote it: without the record
/// of the bones the skins moved ([`sims_sim::World::translated_bones`])
/// that format 26 added at the end either, so a state pinned under format
/// 25 can be shown to be the same state but for it. It starts from
/// [`format_26_bytes`].
fn format_25_bytes(world: &World) -> Vec<u8> {
    let mut bytes = format_26_bytes(world);
    let moved = postcard::to_allocvec(&world.translated_bones).unwrap();
    assert_eq!(bytes.split_off(bytes.len() - moved.len()), moved);
    bytes
}

/// The world's encoding as snapshot format 24 wrote it: without the base
/// layers ([`sims_sim::World::base_layers`]) that format 25 added at the
/// end either. It starts from [`format_25_bytes`].
fn format_24_bytes(world: &World) -> Vec<u8> {
    let mut bytes = format_25_bytes(world);
    let layers = postcard::to_allocvec(&world.base_layers).unwrap();
    assert_eq!(bytes.split_off(bytes.len() - layers.len()), layers);
    bytes
}

/// The state as it was before the load kept each person's saved head
/// target (animator +0x21c, 0x5918ba): every target still the one the
/// load restored in `loaded` is cleared, as no measurement has written it,
/// and an animator then equal to [`sims_sim::animator::LOADED`] is dropped.
/// A state pinned before that change can be shown to be the same state but
/// for those targets.
fn without_saved_targets(world: &World, loaded: &World) -> World {
    let mut old = world.clone();
    for (id, animator) in old.animators.iter_mut() {
        let saved = loaded.animators.get(id).and_then(|a| a.head.target);
        if saved.is_some() && animator.head.target == saved {
            animator.head.target = None;
        }
    }
    old.animators
        .retain(|_, animator| *animator != sims_sim::animator::LOADED);
    old
}

#[test]
fn the_goth_house_steps_to_tick_33() {
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL not set; skipping");
        return;
    };
    let root = PathBuf::from(root);
    let house = root.join("UserData/Houses/House05.iff");
    let mut lot = Lot::load(&root, &house, &[3], Frames::default()).unwrap();
    assert!(lot.load_report().stopped.is_none());
    assert!(lot.load_report().finished.is_some());
    // The house saved camera direction 0, and the comparison harness saw
    // it open at rotation 0; Generic Sims Call 12 reads it.
    assert_eq!(lot.view_rotation(), 0);
    let loaded = lot.world().clone();
    let start = lot.world().ticks;
    for tick in 1..=33 {
        lot.step().unwrap();
        assert_eq!(lot.ticks(), tick);
        assert_eq!(lot.world().ticks, start.wrapping_add(tick));
    }
    // The family panel redraws after each update, so the Goth family's
    // friend count is up to date, as in every capture of the original.
    let goths = lot.world().families.iter().find(|f| f.id == 5).unwrap();
    assert_eq!((goths.friends, goths.stale), (0, false));
    // No headline shows.
    assert!(lot.world().headlines.is_empty());
    assert_eq!(lot.world().hash_hex(), TICK_33);
    assert_eq!(digest(&format_29_bytes(lot.world())), TICK_33_29);
    assert_eq!(digest(&format_28_bytes(lot.world())), TICK_33_28);
    // A person whose head has not measured since the load still holds the
    // target the save held; without them it is the state pinned before.
    let old = without_saved_targets(lot.world(), &loaded);
    assert_eq!(digest(&format_28_bytes(&old)), TICK_33_UNTARGETED);
    assert_eq!(digest(&format_27_bytes(&old)), TICK_33_27);
    assert_eq!(digest(&format_26_bytes(&old)), TICK_33_26);
    assert_eq!(digest(&format_25_bytes(&old)), TICK_33_25);
    let mut bytes = format_24_bytes(&old);
    assert_eq!(digest(&bytes), TICK_33_24);
    let headlines = postcard::to_allocvec(&old.headlines).unwrap();
    assert_eq!(bytes.split_off(bytes.len() - headlines.len()), headlines);
    assert_eq!(digest(&bytes), TICK_33_23);
    let families = postcard::to_allocvec(&old.families).unwrap();
    assert_eq!(bytes.split_off(bytes.len() - families.len()), families);
    assert_eq!(digest(&bytes), TICK_33_22);
}

#[test]
fn an_old_town_home_loads_through_its_guid_table_and_steps() {
    // House 69 of the first neighborhood was saved under its family's
    // exported GUIDs; its GUID table (0x43a6fb) names the imported types.
    // The dialogs at ticks 1 and 3 take their first buttons.
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL not set; skipping");
        return;
    };
    let root = PathBuf::from(root);
    let house = root.join("UserData/Houses/House69.iff");
    let mut lot = Lot::load(&root, &house, &[2; 8], Frames::default()).unwrap();
    // The C runtime's stream as the original entered the lot
    // (`house69-ud1-ticks-20261119`): the dog file was read before the
    // load (`sims_vm::READ_BEFORE_THE_FIRST_LOAD`).
    assert_eq!(lot.world().crt_rng.state(), 2_984_970_331);
    let report = lot.load_report();
    assert!(report.stopped.is_none(), "{:?}", report.stopped);
    assert!(report.finished.is_some());
    for _ in 0..5 {
        lot.step().unwrap();
    }
    assert_eq!(lot.ticks(), 5);
}

/// UserData2 as the original left it after opening it for the first time,
/// in a fresh directory: the install's UserData2, with the `Characters`
/// and `Houses` folders and `Neighborhood.iff` from the folder
/// `SIMS_UD2_AFTER` names, and the emptied `Patch` folder. That folder is
/// track 2's second copy (`ud2-import-20261119/after-run2`), taken after
/// the import's last write and before the lot load, in the session it
/// recorded House 69 in (`house69-ud2-ticks-20261120`). It lacks the
/// neighborhood's other files (`LotZoning.iff` among them), which the open
/// does not change, so they come from the install.
/// The directory is per process, so runs at the same time do not collide.
fn imported_userdata2(root: &Path) -> Option<PathBuf> {
    let after = PathBuf::from(std::env::var_os("SIMS_UD2_AFTER")?);
    let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("userdata2-imported-{}", std::process::id()));
    if directory.exists() {
        std::fs::remove_dir_all(&directory).unwrap();
    }
    copy_directory(&root.join("UserData2"), &directory);
    for folder in ["Characters", "Houses", "Patch"] {
        std::fs::remove_dir_all(directory.join(folder)).unwrap();
    }
    copy_directory(&after.join("Characters"), &directory.join("Characters"));
    copy_directory(&after.join("Houses"), &directory.join("Houses"));
    std::fs::create_dir(directory.join("Patch")).unwrap();
    // The install's copy may be read-only, so it is removed, not overwritten.
    std::fs::remove_file(directory.join("Neighborhood.iff")).unwrap();
    std::fs::copy(
        after.join("Neighborhood.iff"),
        directory.join("Neighborhood.iff"),
    )
    .unwrap();
    Some(directory)
}

#[expect(clippy::disallowed_methods, reason = "sort paths before using them")]
fn copy_directory(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    let mut entries: Vec<PathBuf> = std::fs::read_dir(from)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        let target = to.join(path.file_name().unwrap());
        if path.is_dir() {
            copy_directory(&path, &target);
        } else {
            std::fs::copy(&path, &target).unwrap();
        }
    }
}

#[test]
fn an_imported_home_loads_from_the_originals_files_and_steps() {
    // House 69 of the second neighborhood, after the original's first open
    // imported its Patch families and filled its pools (2000, 3000 to 3009,
    // 5000, 5500 and 6000). Every dialog answers 2, as in track 2's recordings.
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL not set; skipping");
        return;
    };
    let root = PathBuf::from(root);
    let Some(directory) = imported_userdata2(&root) else {
        eprintln!("SIMS_UD2_AFTER not set; skipping");
        return;
    };
    let house = directory.join("Houses/House69.iff");
    let mut crt = CrtRng::from_state(1);
    for _ in 0..OPEN_DRAWS {
        crt.next_u15();
    }
    let frames = Frames::parse(RECORDED_FRAMES).unwrap();
    let mut lot = Lot::load_with(&root, &house, &[2; 8], frames, crt).unwrap();
    // The recording's C runtime state at entry, after the load's own draws.
    assert_eq!(lot.world().crt_rng.state(), 3_739_951_803);
    let report = lot.load_report();
    assert!(report.stopped.is_none(), "{:?}", report.stopped);
    assert!(report.finished.is_some());
    // The original's open: 444 records, the Charmings (family 13) in
    // House 69 under the GUIDs its import drew, and the pools.
    let world = lot.world();
    assert_eq!(world.neighbors.slots.len(), 444);
    let family = |id: i32| world.families.iter().find(|f| f.id == id).unwrap();
    assert_eq!(
        (family(13).house, family(13).members.as_slice()),
        (69, [0xd396_4bf5, 0xd868_148e].as_slice())
    );
    let pools = [2000, 5000, 5500, 6000].map(|id| (family(id).house, family(id).members.len()));
    assert_eq!(pools, [(0, 30), (0, 12), (0, 18), (0, 30)]);
    let tours = (3000..3010).map(|id| family(id).members.len());
    assert_eq!(tours.collect::<Vec<_>>(), [3, 4, 4, 3, 4, 4, 3, 3, 2, 3]);
    assert_eq!(lot.world().hash_hex(), IMPORTED_69);
    for _ in 0..1000 {
        lot.step().unwrap();
    }
    assert_eq!(lot.world().hash_hex(), IMPORTED_69_1000);
    std::fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn a_step_before_a_complete_load_says_so() {
    // Shown through the message the CLI prints.
    assert_eq!(
        StepError::Incomplete.to_string(),
        "ticks were asked for, but the load did not complete"
    );
    let frame = StepError::Frame {
        tick: 0,
        error: "x".into(),
        update: None,
    };
    assert_eq!(frame.to_string(), "the frame before tick 1 failed: x");
}

#[test]
fn the_selected_goth_lists_pie_menus_and_picks_from_them() {
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL not set; skipping");
        return;
    };
    let root = PathBuf::from(root);
    let house = root.join("UserData/Houses/House05.iff");
    let mut lot = Lot::load(&root, &house, &[3], Frames::default()).unwrap();
    let sim = lot.load_report().finished.unwrap().sim.unwrap();
    lot.step().unwrap();
    let queue = |lot: &Lot| {
        lot.world().objects[&sim]
            .person
            .as_ref()
            .unwrap()
            .queue
            .clone()
    };

    // Object 10's listing runs check trees and fails partway, on an
    // action string with a token, whose formatter is not ported. The lot
    // is as it was.
    let before = lot.world().hash();
    let refused = lot.pie_menu(sim, 10).unwrap_err().to_string();
    assert!(refused.contains("0x44bf1a"), "{refused}");
    assert_eq!(lot.world().hash(), before);
    // Object 18's check tree reads the advertisement record (scope 12)
    // before Find Best Interaction first copies into it: the game's start
    // left it zeroed.
    lot.pie_menu(sim, 18).unwrap_or_else(|e| panic!("{e}"));

    // Person 16: listing changes the lot (its check trees' work), and
    // the change is kept.
    let before = lot.world().hash();
    let menu = lot.pie_menu(sim, 16).unwrap_or_else(|e| panic!("{e}"));
    assert_ne!(lot.world().hash(), before);

    assert!(!menu.entries.is_empty());
    assert!(
        menu.entries
            .iter()
            .all(|e| e.action.caller == sim as i16 && e.action.uid == 0)
    );

    // The fridge's check tree names its entries, one for each meal.
    let menu = lot.pie_menu(sim, 118).unwrap_or_else(|e| panic!("{e}"));
    let named: Vec<_> = menu
        .entries
        .iter()
        .filter_map(|e| e.name.as_deref())
        .collect();
    assert_eq!(
        named,
        ["Have Quick Breakfast", "Have Breakfast", "Serve Breakfast"]
    );

    // Object 1's first entry is queued.
    let menu = lot.pie_menu(sim, 1).unwrap_or_else(|e| panic!("{e}"));
    let action = menu.entries[0].action.clone();
    let queued = lot.pick(sim, action).unwrap();
    assert!(queued.is_some());
    assert_eq!(queue(&lot).last().map(|a| a.uid), queued);
    lot.step().unwrap();

    // Person 16's entries run at once (0x45d240). The first one's tree
    // runs a tree by name on my thread (0x44ba26 mode 0), which queues
    // the social on me, through the object it made, and that action is
    // copied over the running one (0x45d5a5): the pick returns the queued
    // action's number.
    let menu = lot.pie_menu(sim, 16).unwrap_or_else(|e| panic!("{e}"));
    let first = menu.entries[0].action.clone();
    let queued = lot.pick(sim, first).unwrap_or_else(|e| panic!("{e}"));
    let last = queue(&lot).last().cloned().unwrap();
    assert_eq!(Some(last.uid), queued);
    assert_eq!((last.caller, last.icon), (sim as i16, 16));
    assert_ne!(last.target, 16);
    // The social clears person 16's base slot (Animate Sim clip 0,
    // 0x58957e), sets person 21's word 72 (Generic Sims Call routine 15,
    // 0x44fe3c) and reads the Goth family's friend count (scope 24 field
    // 9, 0x425b5d).
    for _ in 0..258 {
        lot.step().unwrap_or_else(|e| panic!("{e}"));
        assert!(lot.world().headlines.is_empty());
    }
    // Tick 261: tree 0x200e shows the icon "+ " (kind 6, index 16:
    // sprite 616, 9 frames) over me for 100 ticks, and my tick's tail
    // counts it down once.
    lot.step().unwrap_or_else(|e| panic!("{e}"));
    let shown = |duration| Headline {
        priority: 1,
        duration,
    };
    assert_eq!(lot.world().headlines, BTreeMap::from([(sim, shown(99))]));
    // Tick 262: person 16 shows one too.
    lot.step().unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        lot.world().headlines,
        BTreeMap::from([(16, shown(99)), (sim, shown(98))])
    );
    // Tick 264: the social ends. Person word 73 was set, so it is cleared
    // and my footprint worked out again (0x45e756).
    lot.step().unwrap_or_else(|e| panic!("{e}"));
    assert_ne!(
        lot.world().objects[&sim].person.as_ref().unwrap().data[73],
        0
    );
    lot.step().unwrap_or_else(|e| panic!("{e}"));
    let person = lot.world().objects[&sim].person.as_ref().unwrap();
    assert_eq!((person.active.target, person.data[73]), (0, 0));
    // The headlines count down to 0 and stop showing.
    for _ in 0..96 {
        lot.step().unwrap_or_else(|e| panic!("{e}"));
    }
    assert_eq!(lot.world().headlines, BTreeMap::from([(16, shown(1))]));
    lot.step().unwrap_or_else(|e| panic!("{e}"));
    assert!(lot.world().headlines.is_empty());
    // The salesman (object 259) still has a door knock playing when he
    // is taken off the lot at tick 597; the reset that deletes him
    // removes the clip at once (0x586b63), and the lot runs on past it
    // (checked here to tick 625; it was run to 3625 without a stop).
    // His id is reused by another object before tick 625.
    while lot.ticks() < 596 {
        lot.step().unwrap_or_else(|e| panic!("{e}"));
    }
    let layer = lot.world().animators[&259].foreground.as_ref().unwrap();
    assert_eq!(layer.clip, "A2O-door-knock");
    lot.step().unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(lot.ticks(), 597);
    assert!(!lot.world().objects.contains_key(&259));
    assert!(!lot.world().animators.contains_key(&259));
    while lot.ticks() < 625 {
        lot.step().unwrap_or_else(|e| panic!("{e}"));
    }
}

/// The pie's entry the pointer reaches is checked again (0x48df28) as
/// autonomy checks: its check tree runs on the lot with the argument 1,
/// the advertisement record takes the entry's, and the person's temps
/// are put back.
#[test]
fn reaching_a_pie_entry_checks_its_action_again() {
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL not set; skipping");
        return;
    };
    let root = PathBuf::from(root);
    let house = root.join("UserData/Houses/House05.iff");
    let mut lot = Lot::load(&root, &house, &[3], Frames::default()).unwrap();
    let sim = lot.load_report().finished.unwrap().sim.unwrap();
    for _ in 0..50 {
        lot.step().unwrap();
    }
    let menu = lot.pie_menu(sim, 118).unwrap_or_else(|e| panic!("{e}"));
    let snack = menu
        .entries
        .iter()
        .find(|e| e.name.is_none() && e.action.table_index == 1)
        .expect("Have A Snack")
        .action
        .clone();
    // The listing's last check (the fridge's last entry) failed.
    assert!(!lot.world().objects[&sim].last_result);
    let temps = lot.world().objects[&sim].temps.clone();
    let before = lot.world().hash();
    let checked = lot
        .highlight(sim, snack.clone())
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(checked.action.flags, 12, "{:?}", checked.action);
    // Only its flags are the check's.
    assert_eq!(
        format!(
            "{:?}",
            sims_formats::saved_objects::Interaction {
                flags: snack.flags,
                ..checked.action.clone()
            }
        ),
        format!("{snack:?}")
    );
    let me = &lot.world().objects[&sim];
    assert!(me.last_result, "the entry's check tree passed");
    assert_eq!(me.temps, temps);
    assert_ne!(lot.world().hash(), before);
    assert!(lot.world().advertisement_record.is_some());
    // A target that is not there is refused, and the lot is unchanged.
    let before = lot.world().hash();
    let gone = sims_formats::saved_objects::Interaction {
        target: 30000,
        ..snack
    };
    assert!(lot.highlight(sim, gone).is_err());
    assert_eq!(lot.world().hash(), before);
}

#[test]
fn the_snack_order_pays_for_the_snack() {
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL not set; skipping");
        return;
    };
    let root = PathBuf::from(root);
    let house = root.join("UserData/Houses/House05.iff");
    let mut lot = Lot::load(&root, &house, &[3], Frames::default()).unwrap();
    let sim = lot.load_report().finished.unwrap().sim.unwrap();
    assert_eq!(sim, 21);
    // The save's first day is today's entry (0x46d1e4).
    let today = |lot: &Lot| lot.world().budget.days[0].unwrap();
    assert_eq!(lot.world().budget.funds, 2542);
    assert_eq!(today(&lot), [20, 0, 0, 0, 0, 0, 15060, 33654]);
    for _ in 0..50 {
        assert!(lot.step_play().unwrap().errors.is_empty());
    }
    // The fridge's Have A Snack (menu 1).
    assert!(lot.queue_interaction(sim, 118, 1).unwrap().is_some());
    for step in 51..820 {
        let update = lot.step_play().unwrap();
        assert!(update.errors.is_empty(), "step {step}: {:?}", update.errors);
        assert_eq!(lot.world().budget.funds, 2542, "step {step}");
    }
    // Step 820: Mortimer's tree 0x1007 spends 5 in category 3, and tree
    // 0x104a names his running action. His animator starts to carry the
    // snack (0x5903a3): the posture's clip 0x26 in the carry slot.
    let update = lot.step_play().unwrap();
    assert!(update.errors.is_empty(), "{:?}", update.errors);
    assert_eq!(lot.world().ticks, 89264);
    assert_eq!(lot.world().budget.funds, 2537);
    assert_eq!(today(&lot), [20, 0, 0, 5, 0, 0, 15060, 33654]);
    assert_eq!(
        (lot.world().globals[14], lot.world().globals[13]),
        (0, 2537)
    );
    let person = lot.world().objects[&sim].person.as_ref().unwrap();
    assert_ne!(person.active.flags & 0x80, 0);
    let a = &lot.world().animators[&sim];
    assert_eq!((a.carry_mode, a.carry_selection), (1, 0));
    let carry = a.carry.as_ref().unwrap();
    assert_eq!(
        (carry.clip.as_str(), carry.priority, carry.mode),
        ("a2o-rarm-carry-loop", 10, 0)
    );
    assert_eq!(carry.speed.to_bits(), 0x3f80_0000);
    // The rest of his tick hands person word 1, the 5 spent, to the
    // interface (0x4a69aa) and clears it.
    assert_eq!(person.data[1], 0);
    // The carry clip stops at its end two steps later, and stays chosen.
    for step in 821..836 {
        let update = lot.step_play().unwrap();
        assert!(update.errors.is_empty(), "step {step}: {:?}", update.errors);
    }
    let a = &lot.world().animators[&sim];
    assert_eq!(a.carry_selection, 0);
    let carry = a.carry.as_ref().unwrap();
    assert!(carry.ended && carry.finished());
    // Step 836: walking with the snack, his animator turns by less than a
    // step (0x58dfc0). It has arrived, so it faces its destination's
    // direction at once (0x58dfcf) and stands.
    let update = lot.step_play().unwrap();
    assert!(update.errors.is_empty(), "{:?}", update.errors);
    let a = &lot.world().animators[&sim];
    assert_eq!(a.state, 3);
    assert_eq!(
        a.position.map(|v| v.to_bits()),
        [0x4394_0000, 0x43e0_0000, 0x3fb4_6fe8]
    );
    assert_eq!(a.destination, a.position);
    for step in 837..1035 {
        let update = lot.step_play().unwrap();
        assert!(update.errors.is_empty(), "step {step}: {:?}", update.errors);
    }
    // Step 1035: his tree 0x1050 runs object 121's function with flag 1
    // (Run Functional Tree): the object becomes his running action's icon
    // (person +0xa54, 0x45e5d4), and the use keeps the flag.
    let person = |lot: &Lot| lot.world().objects[&sim].person.clone().unwrap();
    assert_eq!(person(&lot).active.icon, 263);
    let update = lot.step_play().unwrap();
    assert!(update.errors.is_empty(), "{:?}", update.errors);
    let after = person(&lot);
    assert_eq!((after.active.target, after.active.icon), (263, 121));
    let uses: Vec<_> = after
        .object_uses
        .iter()
        .map(|u| (u.target, u.unknown))
        .collect();
    assert_eq!(uses, [(263, 0), (121, 1)]);
    for step in 1036..1088 {
        let update = lot.step_play().unwrap();
        assert!(update.errors.is_empty(), "step {step}: {:?}", update.errors);
    }
    // Step 1088: his tree 0x1002 takes the snack (object 263, in his hand)
    // off the lot (Find Location For mode 1, 0x44b519).
    assert_eq!(lot.world().objects[&263].data[2], sim as i16);
    let update = lot.step_play().unwrap();
    assert!(update.errors.is_empty(), "{:?}", update.errors);
    let snack = &lot.world().objects[&263];
    assert_eq!((snack.position, snack.data[2]), ([-16, -16, 1], 0));
    for step in 1089..1104 {
        let update = lot.step_play().unwrap();
        assert!(update.errors.is_empty(), "step {step}: {:?}", update.errors);
    }
    // Person 259, the traveling salesman, turns by less than a step too,
    // at step 1103. At step 1104 it removes itself (Remove Object Instance
    // in tree 374), and is gone at the end of the update.
    let update = lot.step_play().unwrap();
    assert!(update.errors.is_empty(), "{:?}", update.errors);
    assert_eq!(update.removed, [259]);
    // The update started at tick count 89547.
    assert_eq!(lot.world().ticks, 89548);
    assert_gone(&lot, 259);
    for step in 1105..2703 {
        let update = lot.step_play().unwrap();
        assert!(update.errors.is_empty(), "step {step}: {:?}", update.errors);
    }
    // Step 2703: the carpool (object 8, tree 0x1017 node 4) makes the
    // school bus, a multi-tile object.
    let update = lot.step_play().unwrap();
    assert!(update.errors.is_empty(), "{:?}", update.errors);
    assert_school_bus(&lot);
    for step in 2704..3603 {
        let update = lot.step_play().unwrap();
        assert!(update.errors.is_empty(), "step {step}: {:?}", update.errors);
    }
    // Step 3603: object 354 makes two neighbors' people off the lot and
    // removes them (Create New Object Instance mode 6, flag 0x4).
    // Their IDs are 12 and 24, the lowest free, as in strict mode.
    let update = lot.step_play().unwrap();
    assert!(update.errors.is_empty(), "{:?}", update.errors);
    assert_eq!(update.removed, [12, 24]);
    // Step 15799: person 21 is off the lot, where its model matrix
    // stands at height 0 (0x45ee1d); the run before that was ported
    // recorded an error there.
    for step in 3604..=15799 {
        let update = lot.step_play().unwrap();
        assert!(update.errors.is_empty(), "step {step}: {:?}", update.errors);
    }
    // No fire is lit on the way: the run before Snap's slot offset was
    // fixed lit one at step 13199 (docs/decisions/
    // 20261115-snap-offset-order.md).
    assert!(!lot.world().objects.values().any(|o| o.guid == 0x24c9_5f99));
}

/// The school bus (SchoolBus.iff) the carpool makes at tick 2703: its lead
/// tile, object 12, and 29 more tiles of its group, three across and ten
/// long, with the lead at the group's origin.
fn assert_school_bus(lot: &Lot) {
    let world = lot.world();
    let lead = &world.objects[&12];
    assert_eq!(lead.guid, 0x50AE_1B48);
    assert_eq!(lead.position, [728, 488, 1]);
    let tiles: Vec<_> = world
        .objects
        .values()
        .filter_map(|o| o.multipart.as_ref().filter(|m| m.parent == 12))
        .map(|m| m.group)
        .collect();
    assert_eq!(tiles.len(), 29);
    for a in 0..3 {
        for b in 0..10 {
            assert_eq!(
                tiles.contains(&[a, b, 0]),
                [a, b] != [0, 0],
                "tile {a}, {b}"
            );
        }
    }
}

/// Person `id` is off the lot and out of every list the sim keeps of it.
fn assert_gone(lot: &Lot, id: u16) {
    let world = lot.world();
    assert!(!world.objects.contains_key(&id));
    assert!(!world.people.contains(&id));
    assert!(!world.order.contains(&id));
    assert!(!world.animators.contains_key(&id));
    assert!(!world.routing.routes.contains_key(&id));
    assert!(!world.person_ticks.contains_key(&id));
    assert!(!world.removals.contains(&id));
}

#[test]
fn strict_mode_takes_the_salesman_off_the_lot_at_tick_1104() {
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL not set; skipping");
        return;
    };
    let root = PathBuf::from(root);
    let house = root.join("UserData/Houses/House05.iff");
    let mut lot = Lot::load(&root, &house, &[3], Frames::default()).unwrap();
    for _ in 1..1104 {
        lot.step().unwrap();
    }
    assert_eq!(lot.world().hash_hex(), TICK_1103);
    assert_eq!(digest(&format_29_bytes(lot.world())), TICK_1103_29);
    assert_eq!(digest(&format_28_bytes(lot.world())), TICK_1103_28);
    assert_eq!(digest(&format_27_bytes(lot.world())), TICK_1103_27);
    assert_eq!(digest(&format_26_bytes(lot.world())), TICK_1103_26);
    assert_eq!(digest(&format_25_bytes(lot.world())), TICK_1103_25);
    assert_eq!(digest(&format_24_bytes(lot.world())), TICK_1103_24);
    // The salesman (GUID 0x44A86A6C, made by object 382 at tick 6).
    assert_eq!(lot.world().objects[&259].guid, 0x44A8_6A6C);
    let update = lot.step().unwrap();
    assert_eq!(update.removed, [259]);
    assert_gone(&lot, 259);
}

#[test]
fn the_player_selects_another_goth() {
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL not set; skipping");
        return;
    };
    let root = PathBuf::from(root);
    let house = root.join("UserData/Houses/House05.iff");
    let mut lot = Lot::load(&root, &house, &[3], Frames::default()).unwrap();
    for _ in 0..10 {
        lot.step().unwrap();
    }
    let mortimer = lot.world().selected().unwrap();
    let [first, _] = lot.world().indicators;
    // Another person who lives on the lot (person word 32 is 0) and is of
    // the Goths' family (word 61).
    let family = lot.world().objects[&mortimer].person.as_ref().unwrap().data[61];
    let other = lot
        .world()
        .people
        .iter()
        .copied()
        .find(|&id| {
            let data = &lot.world().objects[&id].person.as_ref().unwrap().data;
            id != mortimer && data[32] == 0 && data[61] == family
        })
        .unwrap();
    // Deselecting Mortimer refreshes his animator at the tick count
    // (0x58da12 with 0).
    lot.select(other).unwrap();
    let ticks = lot.world().ticks as i32;
    let animator = &lot.world().animators[&mortimer];
    assert_eq!(animator.clock, animator.clock_at(ticks));
    let world = lot.world();
    assert_eq!(world.selected(), Some(other));
    assert!(!world.objects[&mortimer].selected);
    assert_eq!(world.globals[3], other as i16);
    assert_eq!(world.objects[&first].data[2], other as i16);
    assert_eq!(world.objects[&other].slots[2].object, first as i16);
    assert_eq!(world.objects[&mortimer].slots[2].object, 0);
    // Selecting him back gives the indicator back.
    lot.select(mortimer).unwrap();
    assert_eq!(lot.world().selected(), Some(mortimer));
    assert_eq!(lot.world().objects[&first].data[2], mortimer as i16);
    // A selection that fails changes nothing.
    let before = lot.world().hash();
    assert!(lot.select(first).is_err());
    assert_eq!(lot.world().hash(), before);
    // The lot goes on in strict mode.
    for _ in 0..50 {
        lot.step().unwrap();
    }
}

// The pins from tick 2881 on follow the run with Snap's slot offset in
// the original's order, Special Effect shown and global 11's break of
// the idle call (docs/decisions/20261115-snap-offset-order.md), and Set
// To Next mode 3 following a group's links
// (docs/decisions/20261118-set-to-next-follows-group-links.md). From tick
// 3603 on they also follow the factory's signal 0xe2, which works out
// again who is a visitor
// (docs/decisions/20261118-new-person-works-out-visitors-again.md). From
// tick 2881 on they also follow Remove Object Instance saving a removed
// person into its neighborhood record (0x451bee, 0x425a69;
// docs/decisions/20261119-generic-sims-call-6-and-the-saved-neighbor-record.md):
// with that save left out, the earlier pins all held. Besides this
// snapshot format, they keep only format 29's hash: the state changed
// with those fixes, so no earlier format's is kept.

/// `sims boot <install> UserData/Houses/House05.iff --ticks 2881
/// --answers 3` gives this hash.
const TICK_2881: &str = "74fec2adeef2e9989a771cda7c804c71c316f3a3079cfadb0297303ba1e2f7bd";
/// The same state's hash under snapshot format 29, which had no person
/// types made while the game runs ([`sims_sim::World::characters`]) at
/// the end.
const TICK_2881_29: &str = "02c627905a1ecda4dd7f7c69cd34c4b761e151f17cf3219087aa42b916c27f04";
/// The same at tick 3432.
const TICK_3432: &str = "4d6afff42d138ca4897cc6444879538129409a21f8e5e6e4380cbaa5283d1158";
/// The same state's hash under snapshot format 29, which had no person
/// types made while the game runs ([`sims_sim::World::characters`]) at
/// the end.
const TICK_3432_29: &str = "0319f1e79ae994cd1bb261ff5f97d337733974aa88521e358802f97cad66877b";
/// The same at tick 3602, the update before the two neighbors are made.
const TICK_3602: &str = "fc4abfd2c3b7d10cc490e0aed6c6e6c02d4c69c078470319f30ba73844315c91";
/// The same state's hash under snapshot format 29, which had no person
/// types made while the game runs ([`sims_sim::World::characters`]) at
/// the end.
const TICK_3602_29: &str = "72420f0bab54d2357f6c2007b3d94bab905067a0cafa242c4e6902302706f1b7";
/// The same at tick 3603, after the two neighbors are made.
const TICK_3603: &str = "512738a0ecce31e27efd58e297632a3de66775f4df3eed76adf002f068a48d67";
/// The same state's hash under snapshot format 29, which had no person
/// types made while the game runs ([`sims_sim::World::characters`]) at
/// the end.
const TICK_3603_29: &str = "d2596aa251487225ce687aa8b69daf749e857d87b650e3a00c4cc34b877f9e89";
/// The same at tick 3912.
const TICK_3912: &str = "869973a29b9ec91cb38a0a5c3dc461315dcadc2034942635bd276928dc2a733f";
/// The same state's hash under snapshot format 29, which had no person
/// types made while the game runs ([`sims_sim::World::characters`]) at
/// the end.
const TICK_3912_29: &str = "4709b76086b6c75eb736f45d20bae15272df19352656c1a8a53312d73c23a162";
/// The same at tick 9906.
const TICK_9906: &str = "f323956e4f37085572651410c13ca52d3e15a7a9d78123a1c219149ee7e871fe";
/// The same state's hash under snapshot format 29, which had no person
/// types made while the game runs ([`sims_sim::World::characters`]) at
/// the end.
const TICK_9906_29: &str = "344de11cfa7fe1181f731dba94d0b501e3777d169f1fecdc3c1cad04a9aedc38";
/// The same at tick 20186.
const TICK_20186: &str = "0185475fce351b2e8b459e16dc55cd40ab919b40fb322e9ac6efc116309fe57b";
/// The same state's hash under snapshot format 29, which had no person
/// types made while the game runs ([`sims_sim::World::characters`]) at
/// the end.
const TICK_20186_29: &str = "6ead1bb05af619268565b915d4f00ac2e1ac5649e03ee6ce1f075926cd9cec58";
/// The same at tick 31478, when the day turns.
const TICK_31478: &str = "34cfca302c1a9b9392b3fcabee36deacdecca0f198df291215abadc0dfe0bc4d";
/// The same state's hash under snapshot format 29, which had no person
/// types made while the game runs ([`sims_sim::World::characters`]) at
/// the end.
const TICK_31478_29: &str = "fc3480edf33f5a146bf2541956ee129cd0200203ae66069e6c763a6835b8f8ed";
/// Where to start the C runtime's generator for House 69's load in track
/// 2's second run: state 1 after this many draws, fitted to the recording's
/// entry state (state 1 after 405,850 draws, of which our load makes the
/// last 4,730). It stands for the draws of the open's import and pools
/// before the load, which are not ported (6b E). That nothing reseeded the
/// generator in between is inferred from the fit, not observed.
const OPEN_DRAWS: u32 = 401_120;
/// The frames track 2's House 69 recordings drew: after updates 0 to 2,
/// 29, 59, 89, 101 and 102, then every 30th from 123
/// (`house69-ud2-ticks-20261120/watch.jsonl`, the frame watch outside
/// an update).
const RECORDED_FRAMES: &str = "0,1,2,29,59,89,101,102,123+30";
/// The imported House 69's state at entry (format 30), which matches
/// track 2's recording of it (`house69-ud2-ticks-20261120`).
const IMPORTED_69: &str = "647c524ea5e613ef5d94a2398525f0f3004ce7921af4ba278ee704c321722f29";
/// The same after 1000 ticks, with the recording's frames.
const IMPORTED_69_1000: &str = "ff8c3f7ff4cc765e2245fe69915db58b569a9242cd919dfdbbc5c7e88790fc80";
/// The same at tick 43200, a whole day from the load.
const TICK_43200: &str = "d26a6a16a451728e40c9b6ada581665a4dfae60ddc7cd8ab306649e3eac14bb0";
/// The same state's hash under snapshot format 29, which had no person
/// types made while the game runs ([`sims_sim::World::characters`]) at
/// the end.
const TICK_43200_29: &str = "693eb621859e6301a85064667b2f697684d3922fad1159a1a665dc67e323f256";

#[test]
fn strict_mode_runs_the_goth_houses_whole_day() {
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL not set; skipping");
        return;
    };
    let root = PathBuf::from(root);
    let house = root.join("UserData/Houses/House05.iff");
    // The second answer is the adoption call's at tick 17707 (see below).
    let mut lot = Lot::load(&root, &house, &[3, 3], Frames::default()).unwrap();
    // The clock: globals 0, 5 and 6 are the hour, minute and second.
    let clock = |lot: &Lot| {
        let globals = &lot.world().globals;
        (globals[0], globals[5], globals[6])
    };
    assert_eq!(clock(&lot), (6, 30, 44));
    for _ in 1..=2703 {
        lot.step().unwrap();
    }
    assert_school_bus(&lot);
    for _ in 2704..=2881 {
        lot.step().unwrap();
        // At tick 2747 objects 12 and 334 run Special Effect, which sets
        // global 11. The idle call clears it after the update that set it
        // (0x46da98, 0x46dacc), so no update ends with it set.
        assert_eq!(lot.world().globals[11], 0);
    }
    assert_eq!(lot.world().hash_hex(), TICK_2881);
    assert_eq!(digest(&format_29_bytes(lot.world())), TICK_2881_29);
    // Each update is two game seconds, 30 a game minute, so a day is
    // 43,200 updates.
    assert_eq!(clock(&lot), (8, 6, 46));
    for _ in 2882..=3432 {
        lot.step().unwrap();
    }
    assert_eq!(lot.world().hash_hex(), TICK_3432);
    assert_eq!(digest(&format_29_bytes(lot.world())), TICK_3432_29);
    assert_eq!(clock(&lot), (8, 25, 8));
    for _ in 3433..=3602 {
        lot.step().unwrap();
    }
    assert_eq!(lot.world().hash_hex(), TICK_3602);
    assert_eq!(digest(&format_29_bytes(lot.world())), TICK_3602_29);
    assert_eq!(clock(&lot), (8, 30, 48));
    // Tick 3603: object 354 (tree 0x1022) picks two neighbor records at
    // random, makes the person of each off the lot (Create New Object
    // Instance mode 6, flag 0x4) as objects 12 and 24, the lowest IDs
    // free, writes their person words 80 and 81, and removes them both;
    // they are deleted at the end of the update. The factory's signal
    // 0xe2 makes each a visitor (0x42a2b4), so each runs "init visitor"
    // (tree 0x200b), with its seven random draws.
    let tick = lot.step().unwrap();
    assert_eq!(tick.removed, [12, 24]);
    assert!(!lot.world().objects.contains_key(&12));
    assert!(!lot.world().objects.contains_key(&24));
    assert_eq!(lot.world().hash_hex(), TICK_3603);
    assert_eq!(digest(&format_29_bytes(lot.world())), TICK_3603_29);
    // Each update to tick 4154 leaves the three random streams as track
    // 2's recording `goth-day-20261115` has them. At 4155 the recording's
    // global stream moves, and not here: taken to be the censor mosaic
    // drawn over person 18 (0x4ac707 → 0x4ac301), which the strict boot
    // does not draw (unverified). The lot and C runtime streams go on
    // matching under frames fitted to the recording (see the test after
    // this one). These are the
    // recording's values after 3603.
    let streams = |lot: &Lot| {
        let world = lot.world();
        (
            world.lot_rng.state(),
            world.global_rng.state(),
            world.crt_rng.state(),
        )
    };
    assert_eq!(streams(&lot), (3_752_373_095, 270_289_136, 2_930_731_800));
    for _ in 3604..=3912 {
        lot.step().unwrap();
    }
    assert_eq!(lot.world().hash_hex(), TICK_3912);
    assert_eq!(digest(&format_29_bytes(lot.world())), TICK_3912_29);
    assert_eq!(clock(&lot), (8, 41, 8));
    for _ in 3913..=4154 {
        lot.step().unwrap();
    }
    // The recording's streams at 4154.
    assert_eq!(streams(&lot), (1_611_871_021, 270_289_136, 2_930_731_800));
    for _ in 4155..=9906 {
        lot.step().unwrap();
    }
    assert_eq!(lot.world().hash_hex(), TICK_9906);
    assert_eq!(digest(&format_29_bytes(lot.world())), TICK_9906_29);
    assert_eq!(clock(&lot), (12, 0, 56));
    // The date: globals 1, 7 and 8 are the day, month and year, global 28
    // the days played.
    let date = |lot: &Lot| {
        let globals = &lot.world().globals;
        (globals[1], globals[7], globals[8], globals[28])
    };
    assert_eq!(date(&lot), (16, 6, 1997, 0));
    // Tick 15750: a person off the lot gets its model matrix at height 0
    // (0x45ee1d). Tick 17707: the neighbor controller (object 5, tree
    // 0x100b) has neighbor 24 call about adopting a baby, a dialog of
    // kind 1 (STR 301); strict mode answers it with the second load
    // answer, 3, "No". Tick 18170: object 26's route loses its stack
    // object and fails (0x45ae8d).
    for _ in 9907..=20186 {
        lot.step().unwrap();
    }
    assert_eq!(lot.world().hash_hex(), TICK_20186);
    assert_eq!(digest(&format_29_bytes(lot.world())), TICK_20186_29);
    // Tick 24605: object 28's route ends while the person it asked to
    // move runs that action, which is cancelled as the player cancels a
    // running action (0x45d617).
    for _ in 20187..=31477 {
        lot.step().unwrap();
    }
    assert_eq!(clock(&lot), (23, 59, 58));
    // The values the house was saved with: the objects' (simulator
    // +0xa0) and the architecture's (+0xa4).
    let values = |lot: &Lot| {
        let budget = &lot.world().budget;
        (budget.objects_value, budget.architecture_value)
    };
    assert_eq!(values(&lot), (14660, 15277));
    // Tick 31478: the hours reach 24 and the day turns (0x46d3e9): the
    // date, the objects' daily update, the budget, and signal 0xdc.
    lot.step().unwrap();
    // The statistics work the objects' value out again after the objects
    // lost value for the day (unverified: the run has left the recording
    // by then). The run before Set To Next mode 3 followed a group's
    // links had 12653. The architecture's value is not ported and keeps
    // the saved one.
    assert_eq!(values(&lot), (12524, 15277));
    assert_eq!(lot.world().hash_hex(), TICK_31478);
    assert_eq!(digest(&format_29_bytes(lot.world())), TICK_31478_29);
    assert_eq!((date(&lot), clock(&lot)), ((17, 6, 1997, 1), (0, 0, 0)));
    // Tick 37803: object 359 (tree 0x100a) makes an object ahead of its
    // stack object (Create New Object Instance mode 3, flags 0x19).
    for _ in 31479..=43200 {
        lot.step().unwrap();
    }
    // A whole day from the load: the clock is back where it started.
    assert_eq!(lot.world().hash_hex(), TICK_43200);
    assert_eq!(digest(&format_29_bytes(lot.world())), TICK_43200_29);
    assert_eq!((date(&lot), clock(&lot)), ((17, 6, 1997, 1), (6, 30, 44)));
}

/// The Goth house under frames fitted to the recording
/// `goth-day-20261115`, which watched no frames: one after each of the
/// first three updates and after every 30th from 33 through 4143, then
/// after every 30th from 4154. The recording's frames drift against its
/// updates (see `docs/decisions/20261118-goth-day-frame-schedule.md`);
/// these two stretches are the ones its streams constrain, and the
/// schedule is unverified. A frame counts the rooms, and which updates
/// find them counted shows in the lot stream (at 5490 here).
#[test]
fn the_goth_day_under_the_recordings_frames_keeps_its_streams() {
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL not set; skipping");
        return;
    };
    let root = PathBuf::from(root);
    let house = root.join("UserData/Houses/House05.iff");
    let mut after = vec![0, 1, 2, 3];
    after.extend((33..=4143).step_by(30));
    let frames = Frames {
        after,
        every: vec![(4154, 30)],
    };
    let mut lot = Lot::load(&root, &house, &[3, 3], frames).unwrap();
    let streams = |lot: &Lot| {
        let world = lot.world();
        (
            world.lot_rng.state(),
            world.global_rng.state(),
            world.crt_rng.state(),
        )
    };
    // The recording's lot and C runtime streams after each update named.
    // Its global stream is 270,289,136 through 4154 and moves from 4155,
    // taken to be the censor mosaic, which is not drawn here (unverified;
    // `docs/decisions/20261118-goth-day-global-draws-are-censor-mosaic.md`).
    // Nothing the port runs draws from it, so here it stays.
    let recorded: [(u32, u32, u32); 11] = [
        (4154, 1_611_871_021, 2_930_731_800),
        (5490, 629_487_932, 3_377_018_407),
        // At 6756 the recording's C runtime stream moves 1,920 draws as
        // a cat's skeleton is built, one skeleton's worth: the kat file was
        // read before (`sims_vm::READ_BEFORE_THE_FIRST_LOAD`).
        (6756, 1_545_421_094, 3_332_047_783),
        (10358, 2_810_847_759, 2_767_048_999),
        (13960, 2_153_618_513, 1_658_190_486),
        (16111, 3_053_854_566, 1_658_190_486),
        // Tick 16112: object 26's Burn moves person 18 to its tile's
        // centre, which resets an animator playing "A2O-panic-loop2". The
        // reset takes the playing clip off (0x592878); with the fade gate
        // clear it is not faded out (`Animator::clear_foreground`).
        (16112, 758_465_277, 1_658_190_486),
        // Another cat at 17562, one skeleton's worth again.
        (17562, 2_627_106_894, 750_951_283),
        // Tick 17929 (after this): person 18's tree 0x0189 takes a
        // neighbor out of its family (Generic Sims Call routine 6,
        // 0x44fc53).
        (17928, 387_178_320, 750_951_283),
        // Tick 21237: a person of the type 0xA23F4AEA is made, of the
        // adult skeleton only (1,392 draws). Its record took the person of
        // that type removed at 18072 (Remove Object Instance saves a
        // removed person into its record, 0x425a69), so its interests are
        // not low and no default traits are drawn.
        (21237, 3_454_917_543, 2_789_594_787),
        (24715, 3_833_359_413, 1_381_078_483),
    ];
    let mut tick = 0;
    for (at, lot_rng, crt) in recorded {
        while tick < at {
            lot.step().unwrap();
            tick += 1;
        }
        assert_eq!(streams(&lot), (lot_rng, 270_289_136, crt), "after {at}");
    }
    // Tick 24716: person 12, having taken itself off the lot earlier in
    // its tick, runs Go To Routing Slot (tree 0x2040 node 7) before its
    // animator's tick, past the test of whether its stack object is on the
    // lot, where the route reads its animator. Unported: what the refresh
    // at the take-off (0x58da12) wrote, if the house view drew the person.
    match lot.step().unwrap_err() {
        StepError::Tick {
            tick: 24716, error, ..
        } => {
            for part in [
                "object 12, tree 0x2040 node 7",
                "Go To Routing Slot (opcode 45) after person 12 left the lot",
            ] {
                assert!(error.contains(part), "{error}");
            }
        }
        other => panic!("{other}"),
    }
}

// The Newbies' pins follow Remove Object Instance saving a removed
// person into its neighborhood record (0x451bee, 0x425a69;
// docs/decisions/20261119-generic-sims-call-6-and-the-saved-neighbor-record.md):
// with that save left out, the earlier pins all held.

/// The Newbies' house (House07) after 1802 updates, with `--answers 3`.
const NEWBIE_1802: &str = "c397370c8585740aaff3dac3f5bc359d53ed94f40e77ac180050ba567bbc04fc";
/// The same state's hash under snapshot format 29, which had no person
/// types made while the game runs ([`sims_sim::World::characters`]) at
/// the end.
const NEWBIE_1802_29: &str = "5ffaaac2956a18cfd478b80897d74c6240f94968ce0e2f91ea28129a96e5ecb9";
/// The same state's hash under snapshot format 28, which had no record
/// of the people whose animator was asked to stop its route
/// ([`sims_sim::World::stops_asked`]) at the end.
const NEWBIE_1802_28: &str = "e72ece6d1a408b1f0a9f2f07e325ccea5750440ef072da2dadb4ddf9049ee5ff";
/// The same state's hash under snapshot format 27, which had no poses
/// the load left in the skins ([`sims_sim::World::held_poses`]) either.
const NEWBIE_1802_27: &str = "a6321d66ebfffedb8d11815266050f55d91e3f472519d3fc78338ae5300ddc54";
/// The same state's hash under snapshot format 26.
const NEWBIE_1802_26: &str = "12cc6cdec085b56e02b84b76b5e585ee01aba4b5ac685476ba62f59ae5b5dcb0";
/// The Newbies' house after 4052 updates, the last whose frame strict
/// mode draws.
const NEWBIE_4052: &str = "e5aab4b1952c8105fcb755965a505813f0c30d5fad99c993e2483076180c0bf5";
/// The same state's hash under snapshot format 29, which had no person
/// types made while the game runs ([`sims_sim::World::characters`]) at
/// the end.
const NEWBIE_4052_29: &str = "e5487823c9720cf5f543cdfc3b61ada1f3951c7f737b16514131f9b72f8f9620";
/// The same state's hash under snapshot format 28, which had no record
/// of the people whose animator was asked to stop its route
/// ([`sims_sim::World::stops_asked`]) at the end.
const NEWBIE_4052_28: &str = "10f36f32f90e6fa870856228b1f4da42d0884a2141f7f85136a5475ee9f8d7c8";
/// The same state's hash under snapshot format 27, which had no poses
/// the load left in the skins ([`sims_sim::World::held_poses`]) either.
const NEWBIE_4052_27: &str = "3c963dcfc9902a97aa02bef939b8ccf2dbf0dea19af97ab584f82730c190e724";

#[test]
fn a_newbie_gives_up_on_a_route_with_its_failure_feedback() {
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL not set; skipping");
        return;
    };
    let root = PathBuf::from(root);
    let house = root.join("UserData/Houses/House07.iff");
    let mut lot = Lot::load(&root, &house, &[3], Frames::default()).unwrap();
    for _ in 1..=1472 {
        lot.step().unwrap();
    }
    let person = |lot: &Lot| lot.world().objects[&105].person.as_ref().unwrap().data[62];
    assert_eq!(person(&lot), 0);
    // Tick 1473: person 105's tree 0x1017, on object 64, routes to a slot
    // it may end near and gives up with result 3 (step 16). It runs
    // the Route failure feedback tree (0x18e) on object 64 with the
    // result, which returns in the same update; then the route ends
    // failed (step 17) and the person's tree goes on.
    lot.step().unwrap();
    assert_eq!(person(&lot), 3);
    let tail: Vec<_> = lot.world().objects[&105]
        .stack
        .iter()
        .rev()
        .take(2)
        .map(|f| (f.tree, f.node, f.stack_object))
        .collect();
    assert_eq!(tail, [(8300, 5, 105), (8233, 9, 64)]);
    for _ in 1474..=1802 {
        lot.step().unwrap();
    }
    assert_eq!(lot.world().hash_hex(), NEWBIE_1802);
    assert_eq!(digest(&format_29_bytes(lot.world())), NEWBIE_1802_29);
    assert_eq!(digest(&format_28_bytes(lot.world())), NEWBIE_1802_28);
    assert_eq!(digest(&format_27_bytes(lot.world())), NEWBIE_1802_27);
    assert_eq!(digest(&format_26_bytes(lot.world())), NEWBIE_1802_26);
    // Tick 1803: object 290 (tree 0x1022) makes two neighbors' people off
    // the lot (Create New Object Instance mode 6, flag 0x4) and removes
    // them, as the Goths' object 354 does at tick 3603. Neither is of the
    // Newbies' family, so the factory's signal 0xe2 makes each a visitor
    // and each runs "init visitor" (see
    // docs/decisions/20261118-new-person-works-out-visitors-again.md).
    // Unverified: the recording (`house07-ticks-20261115`) ends at update
    // 1000.
    let tick = lot.step().unwrap();
    assert_eq!(tick.removed.len(), 2);
    for _ in 1804..=4052 {
        lot.step().unwrap();
    }
    assert_eq!(lot.world().hash_hex(), NEWBIE_4052);
    assert_eq!(digest(&format_29_bytes(lot.world())), NEWBIE_4052_29);
    assert_eq!(digest(&format_28_bytes(lot.world())), NEWBIE_4052_28);
    assert_eq!(digest(&format_27_bytes(lot.world())), NEWBIE_4052_27);
    // The frame drawn after tick 4053 finds person 142's skin waiting for
    // its update (0x590d26), which is not ported. Before the visitor pass
    // above it was person 140's.
    let stopped = lot.step().unwrap_err().to_string();
    for part in ["4053", "person 142", "0x590d26"] {
        assert!(stopped.contains(part), "{stopped}");
    }
}

/// The player cancels the snack from its icon in the queue
/// (`Lot::cancel`, 0x45d5f8): queued, then running.
#[test]
fn the_player_cancels_the_snack_queued_and_running() {
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL not set; skipping");
        return;
    };
    let root = PathBuf::from(root);
    let house = root.join("UserData/Houses/House05.iff");
    let mut lot = Lot::load(&root, &house, &[3], Frames::default()).unwrap();
    let sim = lot.load_report().finished.unwrap().sim.unwrap();
    let person = |lot: &Lot| lot.world().objects[&sim].person.clone().unwrap();
    for _ in 0..50 {
        assert!(lot.step_play().unwrap().errors.is_empty());
    }
    // Queued: it is taken off, and the fridge runs its function 4 (it has
    // no tree there). A second press finds nothing to take.
    let uid = lot.queue_interaction(sim, 118, 1).unwrap().unwrap();
    assert_eq!(person(&lot).queue.len(), 1);
    let cancelled = lot.cancel(sim, uid).unwrap();
    assert!(cancelled.counts);
    assert!(person(&lot).queue.is_empty());
    assert!(!lot.cancel(sim, uid).unwrap().counts);
    // Running: Mortimer takes the order in step 150.
    let uid = lot.queue_interaction(sim, 118, 1).unwrap().unwrap();
    let mut step = 50;
    while person(&lot).active.uid != uid {
        step += 1;
        assert!(lot.step_play().unwrap().errors.is_empty(), "step {step}");
    }
    assert_eq!(step, 150);
    let before = person(&lot);
    assert_eq!(before.active.flags & 0x100, 0);
    assert_ne!(before.data[33], 0);
    assert!(lot.cancel(sim, uid).unwrap().counts);
    // The running action is flagged and its priority word cleared; he
    // would not be interrupted anyway, so his own menu 33 is queued, with
    // no icon.
    let after = person(&lot);
    assert_eq!(after.active.uid, uid);
    assert_eq!(after.active.flags, before.active.flags | 0x100);
    assert_eq!(after.data[33], 0);
    let [queued] = &after.queue[..] else {
        panic!("{:?}", after.queue);
    };
    assert_eq!(
        (
            queued.caller,
            queued.target,
            queued.icon,
            queued.table_index
        ),
        (sim as i16, sim as i16, 0, 33)
    );
    assert_eq!((queued.priority, queued.uid), (50, uid + 1));
    // He is walking to the fridge. Step 182: his route, asked to stop
    // while its animator is under way, stops at the walk's next step
    // (+0x284), at (433, 392); step 183: the cancelled action ends, his
    // menu 33 with it, and he is free. Unverified.
    for at in step + 1..=182 {
        let update = lot.step_play().unwrap();
        assert!(update.errors.is_empty(), "step {at}: {:?}", update.errors);
        assert_eq!(person(&lot).active.uid, uid, "step {at}");
    }
    assert_eq!(lot.world().objects[&sim].position, [433, 392, 1]);
    let update = lot.step_play().unwrap();
    assert!(update.errors.is_empty(), "{:?}", update.errors);
    let free = person(&lot);
    assert_eq!(free.active.uid, 0);
    assert!(free.queue.is_empty());
    assert_eq!(lot.world().objects[&sim].position, [433, 392, 1]);
}

/// The player cancels the snack Mortimer is eating (play mode, the snack
/// order, cancelled at step 900): his trees see the cancel and he puts
/// the snack down with Drop (opcode 5, 0x454430), one tile in front of
/// him, then stops.
#[test]
fn a_cancelled_snack_is_put_down_in_front_of_the_eater() {
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL not set; skipping");
        return;
    };
    let root = PathBuf::from(root);
    let house = root.join("UserData/Houses/House05.iff");
    let mut lot = Lot::load(&root, &house, &[3], Frames::default()).unwrap();
    let sim = lot.load_report().finished.unwrap().sim.unwrap();
    let snack = 263;
    for _ in 0..50 {
        assert!(lot.step_play().unwrap().errors.is_empty());
    }
    assert!(lot.queue_interaction(sim, 118, 1).unwrap().is_some());
    for step in 51..900 {
        let update = lot.step_play().unwrap();
        assert!(update.errors.is_empty(), "step {step}: {:?}", update.errors);
    }
    let person = |lot: &Lot| lot.world().objects[&sim].person.clone().unwrap();
    // He eats the snack he holds in his first container slot.
    let eating = person(&lot).active;
    assert_eq!((eating.uid, eating.target), (5, snack as i16));
    let held = |lot: &Lot| lot.world().objects[&sim].slots[1].object;
    assert_eq!(held(&lot), snack as i16);
    assert!(lot.cancel(sim, eating.uid).unwrap().counts);
    for step in 900..1009 {
        let update = lot.step_play().unwrap();
        assert!(update.errors.is_empty(), "step {step}: {:?}", update.errors);
        assert_eq!(held(&lot), snack as i16, "step {step}");
    }
    // Step 1009: his tree 0x0139 (node 14) drops it. He stands at (296,
    // 456) facing 6, so it goes to (280, 456), on his level: the step's
    // first word goes to +0xf4, our second coordinate (0x4544ab).
    let update = lot.step_play().unwrap();
    assert!(update.errors.is_empty(), "{:?}", update.errors);
    assert_eq!(held(&lot), 0);
    let me = &lot.world().objects[&sim];
    assert_eq!((me.position, me.data[1]), ([296, 456, 1], 6));
    assert_eq!(lot.world().objects[&snack].position, [280, 456, 1]);
    // Step 1028: the cancelled action ends, and he is free.
    for step in 1010..1028 {
        let update = lot.step_play().unwrap();
        assert!(update.errors.is_empty(), "step {step}: {:?}", update.errors);
        assert_eq!(person(&lot).active.uid, 5, "step {step}");
    }
    let update = lot.step_play().unwrap();
    assert!(update.errors.is_empty(), "{:?}", update.errors);
    let free = person(&lot);
    assert_eq!(free.active.uid, 0);
    assert!(free.queue.is_empty());
}

/// The snack order cancelled at step 825, as Mortimer walks away from
/// the fridge with the snack: his route, asked to stop while its
/// animator is under way (0x45b3eb), stops at the walk's next step, the
/// tile's centre at (296, 456). There he puts the snack down in front of
/// him at step 894, as when he is cancelled eating it, and he is free at
/// step 913. Unverified.
#[test]
fn a_snack_cancelled_on_the_way_to_the_chair_is_put_down_at_the_next_tile() {
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL not set; skipping");
        return;
    };
    let root = PathBuf::from(root);
    let house = root.join("UserData/Houses/House05.iff");
    let mut lot = Lot::load(&root, &house, &[3], Frames::default()).unwrap();
    let sim = lot.load_report().finished.unwrap().sim.unwrap();
    for _ in 0..50 {
        assert!(lot.step_play().unwrap().errors.is_empty());
    }
    assert!(lot.queue_interaction(sim, 118, 1).unwrap().is_some());
    for step in 51..825 {
        let update = lot.step_play().unwrap();
        assert!(update.errors.is_empty(), "step {step}: {:?}", update.errors);
    }
    let me = &lot.world().objects[&sim];
    let snack = me.person.as_ref().unwrap().active.uid;
    assert_eq!((snack, me.slots[1].object), (5, 263));
    assert!(lot.cancel(sim, snack).unwrap().counts);
    let held = |lot: &Lot| lot.world().objects[&sim].slots[1].object;
    for step in 825..894 {
        let update = lot.step_play().unwrap();
        assert!(update.errors.is_empty(), "step {step}: {:?}", update.errors);
        assert_eq!(held(&lot), 263, "step {step}");
    }
    assert_eq!(lot.world().objects[&sim].position, [296, 456, 1]);
    let update = lot.step_play().unwrap();
    assert!(update.errors.is_empty(), "{:?}", update.errors);
    assert_eq!(held(&lot), 0);
    assert_eq!(lot.world().objects[&263].position, [280, 456, 1]);
    let person = |lot: &Lot| lot.world().objects[&sim].person.clone().unwrap();
    for step in 895..913 {
        let update = lot.step_play().unwrap();
        assert!(update.errors.is_empty(), "step {step}: {:?}", update.errors);
        assert_eq!(person(&lot).active.uid, 5, "step {step}");
    }
    let update = lot.step_play().unwrap();
    assert!(update.errors.is_empty(), "{:?}", update.errors);
    let free = person(&lot);
    assert_eq!(free.active.uid, 0);
    assert!(free.queue.is_empty());
}

#[test]
fn a_paused_goth_house_goes_on_updating_with_its_clock_standing() {
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL not set; skipping");
        return;
    };
    let root = PathBuf::from(root);
    let house = root.join("UserData/Houses/House05.iff");
    let mut lot = Lot::load(&root, &house, &[3], Frames::default()).unwrap();
    // The house loads at normal speed, with no speed to go back to.
    assert_eq!(lot.world().speed(), sims_sim::speed::NORMAL);
    assert_eq!(lot.world().globals[18], -1);
    for _ in 0..40 {
        lot.step().unwrap();
    }
    assert_eq!(lot.set_speed(924), Ok(14));
    assert_eq!(lot.world().speed(), 924);
    assert_eq!(lot.world().globals[18], -1);
    assert_eq!(lot.pause(), Ok(true));
    assert_eq!(lot.pause(), Ok(false));
    let clock = |lot: &Lot| {
        let g = &lot.world().globals;
        (lot.world().ticks, g[0], g[5], g[6])
    };
    let before = clock(&lot);
    for tick in 41..=240 {
        let update = lot.step().unwrap();
        assert!(!update.full, "tick {tick}");
        assert_eq!(lot.ticks(), tick);
    }
    assert_eq!(clock(&lot), before);
    assert_eq!(lot.unpause(), Ok(true));
    for _ in 0..100 {
        assert!(lot.step().unwrap().full);
    }
    assert_eq!(lot.world().ticks, before.0.wrapping_add(100));
}

#[test]
fn the_player_answers_a_dialog_when_it_is_shown() {
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL not set; skipping");
        return;
    };
    let root = PathBuf::from(root);
    let house = root.join("UserData/Houses/House05.iff");
    // Answered at load, as `sims boot --answers 3` does.
    let mut listed = Lot::load(&root, &house, &[3], Frames::default()).unwrap();
    // Answered when it is shown, as the window does.
    let mut asked = Lot::load(&root, &house, &[], Frames::default()).unwrap();
    asked.wait_for_answers();
    listed.step().unwrap();
    asked.step().unwrap();
    // Tick 2 shows the Help System dialog, which offers two answers.
    let before = asked.world().hash();
    assert_eq!(asked.step().unwrap_err(), StepError::Asking { tick: 2 });
    let asking = asked.asking().expect("a dialog waits");
    assert_eq!(asking.shown.answers, [2, 3]);
    assert!(
        asking.world.dialogs.records.iter().any(|r| r.status == 1),
        "the dialog's record, shown"
    );
    // The lot is as it was before the tick, and stays so: it is not
    // halted, a step asks again, and it takes no orders.
    assert_eq!(asked.ticks(), 1);
    assert_eq!(asked.world().hash(), before);
    assert_eq!(asked.step().unwrap_err(), StepError::Asking { tick: 2 });
    let person = asked.world().selected().unwrap();
    assert_eq!(
        asked.queue_interaction(person, person, 0).unwrap_err(),
        sims_vm::boot::OrderError::Asking
    );
    assert_eq!(asked.world().hash(), before);
    // Only an answer it offers is taken.
    assert!(matches!(
        asked.answer(4),
        Err(sims_vm::boot::AnswerError::NotOffered { answer: 4, .. })
    ));
    asked.answer(3).unwrap();
    assert!(asked.asking().is_none());
    assert_eq!(asked.answer(3), Err(sims_vm::boot::AnswerError::NotAsked));
    // Until the tick runs again, the lot is as it was before it, and an
    // order would be given to that lot: it takes none.
    assert_eq!(
        asked.pie_menu(person, person).unwrap_err(),
        sims_vm::boot::OrderError::Asking
    );
    assert_eq!(asked.world().hash(), before);
    // From there the two lots are the same, tick for tick.
    for tick in 2..=200 {
        let a = listed.step().unwrap();
        let b = asked.step().unwrap();
        assert_eq!(asked.ticks(), tick);
        assert_eq!(asked.world().hash(), listed.world().hash(), "tick {tick}");
        assert_eq!(a.answered, b.answered, "tick {tick}");
        assert_eq!(a.sounds, b.sounds, "tick {tick}");
    }
}

#[test]
fn a_lot_that_does_not_wait_halts_at_an_unanswered_dialog() {
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL not set; skipping");
        return;
    };
    let root = PathBuf::from(root);
    let house = root.join("UserData/Houses/House05.iff");
    let mut lot = Lot::load(&root, &house, &[], Frames::default()).unwrap();
    lot.step().unwrap();
    let stopped = lot.step().unwrap_err();
    assert!(
        matches!(stopped, StepError::Tick { tick: 2, .. }),
        "{stopped}"
    );
    assert!(lot.asking().is_none());
    assert_eq!(lot.step().unwrap_err(), StepError::Halted);
}
