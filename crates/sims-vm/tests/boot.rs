//! `sims_vm::boot` on a real install, when `SIMS_INSTALL` names one: the
//! Goth house loads and steps one tick at a time to the state `sims boot`
//! reaches.

use sims_core::CrtRng;
use sims_formats::behavior::ObjectDefinition;
use sims_formats::iff::Iff;
use sims_sim::{Headline, World};
use sims_vm::Answer;
use sims_vm::boot::{Frames, Handoff, Lot, StepError};
use sims_vm::neighborhood_open::{LotTransition, PoolInput, PoolPerson, open_neighborhood};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// `sims boot <install> UserData/Houses/House05.iff --ticks 33 --answers 3`
/// gives this hash with the default frames (snapshot format 32).
const TICK_33: &str = "7fc16846136f3dd5b18a941d4173446d42a799451883d990ed10534b5e994e6d";
/// The same state's hash under snapshot format 31, which had no dirty
/// portrait types ([`sims_sim::World::dirty_portraits`]) at the end.
const TICK_33_31: &str = "2b92c91c5b17dc809efe537e4ee8eaf58c9472ed6bc6f59147f5b785a0c856d8";
/// The same state's hash under snapshot format 30, which had no person
/// types whose portrait the interface built ([`sims_sim::World::portraits`])
/// at the end.
const TICK_33_30: &str = "cb8c3c739e9152c4ce5e52dcb00917ed3f442f693da2236eb83e228e70821af2";
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
const TICK_1103: &str = "eedfc4514d339f5974e74f2cc8a9dfe7abf046ce1182fbd67ce1d1bfd280785b";
/// The same state's hash under snapshot format 31, which had no dirty
/// portrait types ([`sims_sim::World::dirty_portraits`]) at the end.
const TICK_1103_31: &str = "251262c7f68366bcb78e52134c37caa134bacd0ae674168202b5ef99fb328bdf";
/// The same state's hash under snapshot format 30, which had no person
/// types whose portrait the interface built ([`sims_sim::World::portraits`])
/// at the end.
const TICK_1103_30: &str = "2de410b64f9b2ea57d1a1baad6eec85d655dd106c74a9465548b9a4590ffffe5";
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

/// The tile the Goth day's boots give for the house view's centre
/// ([`Lot::load_viewed`], [`Lot::set_view_centre`]): the lot's middle tile. It is a stand-in
/// start tile, not the original's camera. Every start — including the four
/// no-room starts (`19,24`, `23,31`, `26,28`, `26,31`), which remove the cat
/// unput (`0x4e9706`) and fall back to `BMP_` 2002 (`0x4e9260`, no second
/// build at `0x4e938c`) — as well as `view_centre = None` reaches the exact
/// same state ([`every_start_gives_one_state`]).
/// Which camera the original opens the Goths' house with is not traced.
const GOTH_VIEW_CENTRE: [i8; 2] = [32, 32];

/// Steps `lot` once from every tile the portrait's search may start from
/// (inside the lot's border) and from `view_centre = None`, each from the
/// same save, and checks that all of them succeed and reach one state hash
/// (matching both the encoded world hash and the full `Saved` digest).
/// The lot is left as it was, with its view centre.
fn every_start_gives_one_state(lot: &mut Lot) {
    let saved = lot.save().unwrap();
    let centre = lot.view_centre();
    let size = lot.world().size as i8;
    let mut states = BTreeMap::new();
    for x in 1..size - 1 {
        for y in 1..size - 1 {
            lot.restore(saved.clone()).unwrap();
            lot.set_view_centre(Some([x, y]));
            lot.step().unwrap_or_else(|e| panic!("{x},{y}: {e}"));
            let world_hash = lot.world().hash_hex();
            lot.set_view_centre(centre);
            let full_hash = digest(&lot.save().unwrap().encode());
            *states.entry((world_hash, full_hash)).or_insert(0) += 1;
        }
    }
    lot.restore(saved.clone()).unwrap();
    lot.set_view_centre(None);
    lot.step().unwrap_or_else(|e| panic!("None: {e}"));
    let world_hash = lot.world().hash_hex();
    lot.set_view_centre(centre);
    let full_hash = digest(&lot.save().unwrap().encode());
    *states.entry((world_hash, full_hash)).or_insert(0) += 1;
    assert_eq!(states.len(), 1, "{states:?}");
    lot.restore(saved).unwrap();
    lot.set_view_centre(centre);
}

fn digest(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The encoding of `world` as snapshot format 31 wrote it: without the
/// portrait dirty and saved sets ([`sims_sim::World::dirty_portraits`],
/// [`sims_sim::World::saved_portraits`]) that format 32 added at the end.
/// The older formats' encodings start from this one.
fn format_31_bytes(world: &World) -> Vec<u8> {
    let mut bytes = world.encode();
    let saved = postcard::to_allocvec(&world.saved_portraits).unwrap();
    assert_eq!(bytes.split_off(bytes.len() - saved.len()), saved);
    let dirty = postcard::to_allocvec(&world.dirty_portraits).unwrap();
    assert_eq!(bytes.split_off(bytes.len() - dirty.len()), dirty);
    bytes
}

/// The encoding of `world` as snapshot format 30 wrote it: without the
/// person types whose portrait the interface has built
/// ([`sims_sim::World::portraits`]) that format 31 added at the end. The
/// older formats' encodings start from this one.
fn format_30_bytes(world: &World) -> Vec<u8> {
    let mut bytes = format_31_bytes(world);
    let built = postcard::to_allocvec(&world.portraits).unwrap();
    assert_eq!(bytes.split_off(bytes.len() - built.len()), built);
    bytes
}

/// The world's encoding as snapshot format 29 wrote it: without the
/// person types made while the game runs ([`sims_sim::World::characters`])
/// that format 30 added at the end. The older formats' encodings start
/// from this one.
fn format_29_bytes(world: &World) -> Vec<u8> {
    let mut bytes = format_30_bytes(world);
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
    assert_eq!(digest(&format_31_bytes(lot.world())), TICK_33_31);
    assert_eq!(digest(&format_30_bytes(lot.world())), TICK_33_30);
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
    // Track 2 recorded it for 1,000 updates (`house69-ud1-ticks-20261120`).
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL not set; skipping");
        return;
    };
    let root = PathBuf::from(root);
    let house = root.join("UserData/Houses/House69.iff");
    let frames = Frames::parse(UD1_RECORDED_FRAMES).unwrap();
    let bits = UD1_FRAME_TREE_BITS.into_iter().collect();
    let mut lot = Lot::load_with_frame_tree_bits(
        &root,
        &house,
        &[2; 8],
        frames,
        CrtRng::from_state(1),
        &bits,
    )
    .unwrap();
    lot.replay_dialogs(&UD1_DIALOGS.map(|(status, bitmap_id)| Answer {
        status,
        bitmap_id,
        text: None,
    }))
    .unwrap();
    // The C runtime's stream as the original entered the lot: the dog file
    // was read before the load (`sims_vm::READ_BEFORE_THE_FIRST_LOAD`).
    assert_eq!(lot.world().crt_rng.state(), 2_984_970_331);
    let report = lot.load_report();
    assert!(report.stopped.is_none(), "{:?}", report.stopped);
    assert!(report.finished.is_some());
    assert_eq!(lot.world().hash_hex(), OLD_TOWN_69);
    assert_eq!(digest(&format_31_bytes(lot.world())), OLD_TOWN_69_31);
    assert_eq!(digest(&format_30_bytes(lot.world())), OLD_TOWN_69_30);
    for _ in 0..1000 {
        lot.step().unwrap();
    }
    assert_eq!(lot.ticks(), 1000);
    assert_eq!(lot.world().hash_hex(), OLD_TOWN_69_1000);
    assert_eq!(digest(&format_31_bytes(lot.world())), OLD_TOWN_69_1000_31);
    assert_eq!(digest(&format_30_bytes(lot.world())), OLD_TOWN_69_1000_30);
}

/// The frames track 2's UserData1 House 69 recording drew
/// (`house69-ud1-ticks-20261120/watch.jsonl`, the frame watch outside an
/// update).
const UD1_RECORDED_FRAMES: &str = "0,1,2,3,4,32,62,92,101,102,128+30";
/// The saved frames whose arena words had bit 15 set when the original
/// read them (0x4746e4), as track 2's recording shows them at entry:
/// frame 1 of objects 263, 264, 265 and 268. The bit is what the arena
/// held before, not what the house file stores
/// (`20261115-household-selection-works-out-again-who-visitor.md`).
const UD1_FRAME_TREE_BITS: [(u16, usize); 4] = [(263, 1), (264, 1), (265, 1), (268, 1)];
/// The recording's dialogs after the load, in order. Each closed with
/// result 2. While the fourth was open, the wait check published 17
/// (`WinBtn.bmp`, 0x46d9c4), the bitmap of the button that closed it,
/// which global 12 holds until the next update. While the others were
/// open, global 12 did not change in the recording, so they publish
/// nothing.
const UD1_DIALOGS: [(u8, Option<i16>); 6] = [
    (2, None),
    (2, None),
    (2, None),
    (2, Some(17)),
    (2, None),
    (2, None),
];
/// UserData1's House 69 at entry (format 32), a regression pin of our
/// run. It differs from track 2's recording only in object 2's last
/// result; see `20261120-ud1-house-69-matches-the-recording.md`.
const OLD_TOWN_69: &str = "172352ec8506772dc53c275a815a04bcda552163e24aa4b2b851c7a164d03429";
/// The same state's hash under snapshot format 31, which had no dirty
/// portrait types ([`sims_sim::World::dirty_portraits`]) at the end.
const OLD_TOWN_69_31: &str = "cdf5e94eb507086c61bdd5e1d18d64327e5dbba897130a7de54ee87ef99c9945";
/// The same state's hash under snapshot format 30, which had no person
/// types whose portrait the interface built ([`sims_sim::World::portraits`])
/// at the end.
const OLD_TOWN_69_30: &str = "e34ca24ca0aef8aa000507db5bd03ee074e85845278853616c02e38c191eae7b";
/// The same after 1000 ticks, a regression pin, which differs from the
/// recording's update 1000 only in object 2's last result.
const OLD_TOWN_69_1000: &str = "b7c954177ae6ef7f39b84ed9be036d69bb463040400a72971c81bccc6e5d6df5";
/// The same state's hash under snapshot format 31.
const OLD_TOWN_69_1000_31: &str =
    "2b4741e731424e6c7ce34462cdaff5461f69e8f19a4f5ddec37eba09efeaaac6";
/// The same state's hash under snapshot format 30.
const OLD_TOWN_69_1000_30: &str =
    "21a7f3b2ade639b129c5008949647519d12432993ce3b30f13dd56fbd0e17a86";

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
    for _ in 0..FILE_BOOT_DRAWS {
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
    assert_eq!(digest(&format_31_bytes(lot.world())), IMPORTED_69_31);
    assert_eq!(digest(&format_30_bytes(lot.world())), IMPORTED_69_30);
    for _ in 0..1000 {
        lot.step().unwrap();
    }
    assert_eq!(lot.world().hash_hex(), IMPORTED_69_1000);
    assert_eq!(digest(&format_31_bytes(lot.world())), IMPORTED_69_1000_31);
    assert_eq!(digest(&format_30_bytes(lot.world())), IMPORTED_69_1000_30);
    std::fs::remove_dir_all(&directory).unwrap();
}

/// The families the open's five pools fill after `Patch`: the Townies
/// (2000), the ten tourist families (3000 to 3009), the celebrities
/// (5000), the fans (5500) and Magic Town (6000).
const POOLS: [u16; 14] = [
    2000, 3000, 3001, 3002, 3003, 3004, 3005, 3006, 3007, 3008, 3009, 5000, 5500, 6000,
];

/// The first person type file the pools made in the original's UserData2
/// (`User00030.iff`); its import made 2 to 29.
const FIRST_POOL_FILE: u32 = 30;

/// A `Characters/User%05d.iff` file's number.
fn user_file(path: &Path) -> Option<u32> {
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    name.strip_prefix("user")?
        .strip_suffix(".iff")?
        .parse()
        .ok()
}

/// The person type files in a folder, sorted by path.
#[expect(clippy::disallowed_methods, reason = "sort paths before using them")]
fn user_files(characters: &Path) -> Vec<(u32, PathBuf)> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(characters)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    paths.sort();
    paths
        .into_iter()
        .filter_map(|path| Some((user_file(&path)?, path)))
        .collect()
}

fn recorded_character(path: &Path) -> (Iff, u32) {
    let iff = Iff::parse(std::fs::read(path).unwrap()).unwrap();
    let objd = iff.data(iff.all(b"OBJD").next().unwrap());
    let guid = ObjectDefinition::parse(objd).unwrap().guid;
    (iff, guid)
}

/// Read only the pool outcomes the port needs from the capture. The
/// FAMI member GUIDs identify each file's pool; the files give its
/// template, clock word and profile. NBRS supplies the non-CRT editor
/// outcomes. The open creates its own families and neighbors.
fn recorded_pools(original: &sims_content::Neighborhood, files: &[(u32, PathBuf)]) -> PoolInput {
    let mut people = BTreeMap::<u16, Vec<PoolPerson>>::new();
    for (_, path) in files
        .iter()
        .filter(|(number, _)| *number >= FIRST_POOL_FILE)
    {
        let (character, guid) = recorded_character(path);
        let family = POOLS
            .into_iter()
            .find(|id| original.families[id].members.contains(&guid))
            .expect("every recorded pool file belongs to a pool");
        let record = original
            .neighbors
            .iter()
            .flatten()
            .find(|record| record.guid == guid)
            .unwrap();
        let words: &[usize] = match (family, record.person_data[58]) {
            (5000, _) => &[10, 11, 12, 15, 17, 18, 80, 81, 82],
            (3000..=3009, 9) => &[9, 10, 11, 12, 15, 17, 18, 56, 57],
            _ => &[],
        };
        people.entry(family).or_default().push(PoolPerson {
            character,
            editor_words: words
                .iter()
                .map(|&word| (word, record.person_data[word]))
                .collect(),
        });
    }
    PoolInput {
        people,
        loaded_skeletons: ["adult".to_owned(), "child".to_owned()].into(),
        transition: Some(LotTransition {
            reload: true,
            crt_draws: 64,
        }),
    }
}

#[test]
fn an_opened_neighborhood_matches_the_originals_import() {
    // Our open of the install's UserData2, from house 0 through Patch
    // and the five pools, against track 2's second run (SIMS_UD2_AFTER).
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL not set; skipping");
        return;
    };
    let root = PathBuf::from(root);
    let Some(after) = std::env::var_os("SIMS_UD2_AFTER") else {
        eprintln!("SIMS_UD2_AFTER not set; skipping");
        return;
    };
    let after = PathBuf::from(after);
    let output = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("userdata2-opened-{}", std::process::id()));
    if output.exists() {
        std::fs::remove_dir_all(&output).unwrap();
    }
    copy_directory(&root.join("UserData2"), &output);

    // The GUIDs the original drew, in file order: each made type's file
    // number follows its draw.
    let theirs: Vec<(u32, PathBuf)> = user_files(&after.join("Characters"));
    let original = sims_content::Neighborhood::open(&after).unwrap();
    let pools = recorded_pools(&original, &theirs);
    let mut guids = theirs
        .iter()
        .filter(|(number, _)| (2..FIRST_POOL_FILE).contains(number))
        .map(|(_, path)| recorded_character(path).1);
    let mut crt = CrtRng::from_state(1);
    for _ in 0..OPEN_DRAWS {
        crt.next_u15();
    }
    let (neighborhood, opened) = open_neighborhood(
        &root,
        &root.join("UserData2"),
        &output,
        &mut || guids.next().expect("the original drew no more GUIDs"),
        &|house| std::fs::read(after.join(format!("Houses/House{house:02}.iff"))).ok(),
        &pools,
        &mut crt,
    )
    .unwrap();
    assert!(guids.next().is_none(), "the import drew fewer GUIDs");
    assert_eq!(opened.patch.imports.len(), 6);
    assert_eq!(opened.patch.next_file, 30);
    assert_eq!(opened.pools.next_file, 153);
    assert_eq!(opened.pools.update_draws, 176_444);
    assert_eq!(opened.pools.townie_draws, 44_096);
    assert_eq!(opened.pools.tourist_draws, 97_027);
    assert_eq!(crt.state(), 0x0d1a_a553);
    assert!(opened.pools.loaded_skeletons.is_empty() && opened.pools.skeleton_files.is_empty());

    // The five house files, byte for byte, given the original's leftover
    // heap bytes.
    for house in [56, 63, 69, 76, 77] {
        let name = format!("Houses/House{house}.iff");
        assert!(
            std::fs::read(output.join(&name)).unwrap() == std::fs::read(after.join(&name)).unwrap(),
            "{name} differs"
        );
    }
    // Each imported family file is used up: the housed ones became their
    // house files and the houseless one was deleted, so the copy's `Patch`
    // folder is empty, as the original left it.
    assert_eq!(std::fs::read_dir(output.join("Patch")).unwrap().count(), 0);
    // The inventories the view's set-up and each housed import made, which
    // the pools leave alone.
    assert_eq!(
        format!("{:?}", neighborhood.inventories),
        format!("{:?}", original.inventories)
    );

    assert_eq!(neighborhood.families.len(), original.families.len());
    for id in POOLS {
        assert_eq!(
            neighborhood.families[&id], original.families[&id],
            "family {id}"
        );
    }
    assert_eq!(neighborhood.options, original.options);
    assert_eq!(neighborhood.neighbors.len(), 444);
    assert_eq!(neighborhood.neighbors.len(), original.neighbors.len());
    for (slot, (ours, theirs)) in neighborhood.neighbors[321..]
        .iter()
        .zip(&original.neighbors[321..])
        .enumerate()
    {
        assert_eq!(ours, theirs, "pool record {}", 322 + slot);
    }

    let house = output.join("Houses/House69.iff");
    let frames = Frames::parse(RECORDED_FRAMES).unwrap();
    // Run 2's handoff, as fitted: the adult skeleton read and the
    // indicators made before the lot load.
    let handoff = Handoff {
        skeletons_read: vec!["adult".to_owned()],
        indicators_made: true,
    };
    let lot =
        Lot::load_after_pools(&root, neighborhood, &house, &[2; 8], frames, crt, &handoff).unwrap();
    assert!(lot.load_report().finished.is_some());
    assert_eq!(lot.world().crt_rng.state(), 3_739_951_803);
    assert_eq!(lot.world().hash_hex(), IMPORTED_69);
    assert_eq!(digest(&format_31_bytes(lot.world())), IMPORTED_69_31);
    assert_eq!(digest(&format_30_bytes(lot.world())), IMPORTED_69_30);
    std::fs::remove_dir_all(&output).unwrap();

    // Run 2's pools found `adult` and `child` loaded. Loaded on first use
    // instead (0x5a03cd), their catalogs' builds draw before the first
    // celebrity's copy, so the next pool person's GUID draw is off.
    let lazy = output.with_extension("lazy");
    if lazy.exists() {
        std::fs::remove_dir_all(&lazy).unwrap();
    }
    copy_directory(&root.join("UserData2"), &lazy);
    let mut pools = recorded_pools(&original, &theirs);
    pools.loaded_skeletons.clear();
    let mut guids = theirs
        .iter()
        .filter(|(number, _)| (2..FIRST_POOL_FILE).contains(number))
        .map(|(_, path)| recorded_character(path).1);
    let mut crt = CrtRng::from_state(1);
    for _ in 0..OPEN_DRAWS {
        crt.next_u15();
    }
    let error = open_neighborhood(
        &root,
        &root.join("UserData2"),
        &lazy,
        &mut || guids.next().expect("the original drew no more GUIDs"),
        &|house| std::fs::read(after.join(format!("Houses/House{house:02}.iff"))).ok(),
        &pools,
        &mut crt,
    )
    .unwrap_err()
    .to_string();
    assert!(error.starts_with("0x4725ff: User00031 "), "{error}");
    std::fs::remove_dir_all(&lazy).unwrap();
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

    // Object 10 (phone) runs Call Cab's check tree (0x2074), which formats
    // `$$Local:1` action strings through 0x44bf1a into cab fares and
    // service entries.
    let phone = lot.pie_menu(sim, 10).unwrap_or_else(|e| panic!("{e}"));
    let phone_named: Vec<_> = phone
        .entries
        .iter()
        .filter_map(|e| e.name.as_deref())
        .collect();
    assert_eq!(
        phone_named,
        [
            "Call Cab.../Go to Magic Town - $50",
            "Call Cab.../Go to Studio Town - $50",
            "Call Cab.../Explore Old Town",
            "Call Cab.../Go On Vacation - $500",
            "Call Cab.../Go Downtown - $50",
            "Services.../Caterer",
        ]
    );
    // Objects 4 and 253 (the other phones on the lot) list Call Cab too.
    for target in [4u16, 253] {
        let menu = lot.pie_menu(sim, target).unwrap_or_else(|e| panic!("{e}"));
        assert!(
            menu.entries
                .iter()
                .any(|e| e.name.as_deref() == Some("Call Cab.../Go to Magic Town - $50")),
            "object {target}: {menu:?}"
        );
    }
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

/// At tick 1500 of the Goth house, person 18 runs menu 1 of object 217,
/// which may be joined. Its pie menu on another person ends with the offer
/// to join it (0x45866d, 0x41ee99), whose caller is that person; picking
/// it queues the offer on that person, who takes it up and keeps its join
/// word while it runs.
#[test]
fn the_pie_menu_on_another_goth_offers_to_join_what_i_run() {
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL not set; skipping");
        return;
    };
    let root = PathBuf::from(root);
    let house = root.join("UserData/Houses/House05.iff");
    let mut lot = Lot::load(&root, &house, &[3, 3], Frames::default()).unwrap();
    while lot.ticks() < 1500 {
        lot.step().unwrap_or_else(|e| panic!("{e}"));
    }
    let (me, other) = (18, 16);
    let running = |lot: &Lot, id: u16| {
        lot.world().objects[&id]
            .person
            .as_ref()
            .unwrap()
            .active
            .clone()
    };
    let mine = running(&lot, me);
    assert_eq!((mine.target, mine.table_index), (217, 1));
    let menu = lot.pie_menu(me, other).unwrap_or_else(|e| panic!("{e}"));
    let (offer, entries) = menu.entries.split_last().unwrap();
    assert!(
        entries
            .iter()
            .all(|e| e.action.join == 0 && e.action.caller == me as i16)
    );
    let offer = offer.action.clone();
    assert_eq!(
        (
            offer.join,
            offer.caller,
            offer.target,
            offer.table_index,
            offer.args,
            offer.priority,
            offer.flags & 0x1c,
        ),
        (1, other as i16, 217, 2, [0, 0, me as i16, 0], 50, 0xc)
    );
    // Picking it queues it on the other person, not on me.
    let mine_queued = lot.world().objects[&me]
        .person
        .as_ref()
        .unwrap()
        .queue
        .len();
    let uid = lot
        .pick(me, offer.clone())
        .unwrap_or_else(|e| panic!("{e}"))
        .expect("queued");
    assert_eq!(
        lot.world().objects[&me]
            .person
            .as_ref()
            .unwrap()
            .queue
            .len(),
        mine_queued
    );
    let queue = &lot.world().objects[&other].person.as_ref().unwrap().queue;
    assert!(queue.iter().any(|a| a.uid == uid && a.join == 1));
    // The other person starts it: Idle For Input's check passes the join's
    // arguments, and the running action keeps its join word.
    for _ in 0..100 {
        lot.step().unwrap_or_else(|e| panic!("{e}"));
    }
    let joined = running(&lot, other);
    assert_eq!(
        (joined.uid, joined.join, joined.target, joined.table_index),
        (uid, 1, 217, 2)
    );
}

/// At tick 6000 of the Goth house, visitors (persons 24 and 26) are on the
/// lot. Listing their pie menus on category 4 and 7 objects (objects 112,
/// 113, 124–129 and 211–213) checks for `Token - Hotel Key` (type 0xd,
/// 0x3ac022d6) in their neighbor-ID inventory (0x42a8a1, 0x43e21a,
/// 0x41f660) instead of failing loudly. (In `UserData/Neighborhood.iff`
/// both visitors already carry empty inventories, so missing-inventory
/// creation is covered by the synthetic fixture tests.)
#[test]
fn a_goth_visitor_lists_category_4_and_7_objects_via_the_hotel_key_check() {
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL not set; skipping");
        return;
    };
    let root = PathBuf::from(root);
    let house = root.join("UserData/Houses/House05.iff");
    let mut lot = Lot::load(&root, &house, &[3, 3], Frames::default()).unwrap();
    while lot.ticks() < 6000 {
        lot.step().unwrap_or_else(|e| panic!("{e}"));
    }
    let visitors: Vec<u16> = lot
        .world()
        .objects
        .values()
        .filter(|o| o.person.as_ref().is_some_and(|p| p.data[32] != 0))
        .map(|o| o.id)
        .collect();
    assert!(
        visitors.contains(&24) && visitors.contains(&26),
        "visitors at 6000: {visitors:?}"
    );
    let targets = [112u16, 113, 124, 125, 126, 127, 128, 129, 211, 212, 213];
    for &target in &targets {
        assert!(
            matches!(lot.world().objects[&target].data[59], 4 | 7),
            "object {target} category {}",
            lot.world().objects[&target].data[59]
        );
    }

    let inventories_before = lot.world().neighbors.inventories.len();
    for &visitor in &[24u16, 26] {
        let neighbor = lot.world().objects[&visitor].person.as_ref().unwrap().data[31];
        assert!(
            lot.world()
                .neighbors
                .inventories
                .iter()
                .any(|inv| inv.neighbor == neighbor),
            "visitor {visitor} (neighbor {neighbor}) already has a saved inventory"
        );
        for &target in &targets {
            lot.pie_menu(visitor, target)
                .unwrap_or_else(|e| panic!("visitor {visitor} on {target}: {e}"));
        }
    }
    assert_eq!(lot.world().neighbors.inventories.len(), inventories_before);
}

/// Every person's pie menu on every object of the Goth house lists cleanly
/// at ticks 0, 1500, and 6000 (the three probe points from
/// `docs/decisions/20261120-check-tree-faults-and-failed-play-steps.md`,
/// closing Call Cab's `$$Local:1` formatter as the last of the three
/// Goth pie-listing failures).
#[test]
fn every_person_lists_every_object_on_the_goth_house_at_0_1500_and_6000() {
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL not set; skipping");
        return;
    };
    let root = PathBuf::from(root);
    let house = root.join("UserData/Houses/House05.iff");
    let mut lot = Lot::load(&root, &house, &[3, 3], Frames::default()).unwrap();
    for target_tick in [0u32, 1500, 6000] {
        while lot.ticks() < target_tick {
            lot.step()
                .unwrap_or_else(|e| panic!("tick {}: {e}", lot.ticks()));
        }
        let saved = lot.save().unwrap();
        let people: Vec<u16> = lot.world().people.clone();
        let objects: Vec<u16> = lot.world().objects.keys().copied().collect();
        assert!(!people.is_empty(), "tick {target_tick}");
        for &person in &people {
            for &object in &objects {
                lot.restore(saved.clone()).unwrap();
                lot.pie_menu(person, object).unwrap_or_else(|e| {
                    panic!("tick {target_tick}: person {person} on object {object} failed: {e}")
                });
            }
        }
        lot.restore(saved).unwrap();
    }
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
    assert_eq!(digest(&format_31_bytes(lot.world())), TICK_1103_31);
    assert_eq!(digest(&format_30_bytes(lot.world())), TICK_1103_30);
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
const TICK_2881: &str = "1c35e511b989396f535182e4708f0236c47508b22c36d895aac10ecbe7fd47a0";
/// The same state's hash under snapshot format 31, which had no dirty
/// portrait types ([`sims_sim::World::dirty_portraits`]) at the end.
const TICK_2881_31: &str = "19d7b50f5b2bfac7a956243431234932a50b70cd127d10df879aa2cb447b4d2e";
/// The same state's hash under snapshot format 30, which had no person
/// types whose portrait the interface built ([`sims_sim::World::portraits`])
/// at the end.
const TICK_2881_30: &str = "74fec2adeef2e9989a771cda7c804c71c316f3a3079cfadb0297303ba1e2f7bd";
/// The same state's hash under snapshot format 29, which had no person
/// types made while the game runs ([`sims_sim::World::characters`]) at
/// the end.
const TICK_2881_29: &str = "02c627905a1ecda4dd7f7c69cd34c4b761e151f17cf3219087aa42b916c27f04";
/// The same at tick 3432.
const TICK_3432: &str = "bdec938512fe1b57e3794375aa79497b8995cd3497c1023d51284c72067e391d";
/// The same state's hash under snapshot format 31.
const TICK_3432_31: &str = "0fde2e4c331a9886d91f904c397e027e543eac965bd597eccc47db63f78726c1";
/// The same state's hash under snapshot format 30, which had no person
/// types whose portrait the interface built ([`sims_sim::World::portraits`])
/// at the end.
const TICK_3432_30: &str = "4d6afff42d138ca4897cc6444879538129409a21f8e5e6e4380cbaa5283d1158";
/// The same state's hash under snapshot format 29, which had no person
/// types made while the game runs ([`sims_sim::World::characters`]) at
/// the end.
const TICK_3432_29: &str = "0319f1e79ae994cd1bb261ff5f97d337733974aa88521e358802f97cad66877b";
/// The same at tick 3602, the update before the two neighbors are made.
const TICK_3602: &str = "86a4973ecc82ea69659cb0e0bdbf9f8b867004cbab69be314352729162cc854e";
/// The same state's hash under snapshot format 31.
const TICK_3602_31: &str = "e8e4f6fbf1ca47ca227b41ab16b18614f5ee5a560948325613282e536807af56";
/// The same state's hash under snapshot format 30, which had no person
/// types whose portrait the interface built ([`sims_sim::World::portraits`])
/// at the end.
const TICK_3602_30: &str = "fc4abfd2c3b7d10cc490e0aed6c6e6c02d4c69c078470319f30ba73844315c91";
/// The same state's hash under snapshot format 29, which had no person
/// types made while the game runs ([`sims_sim::World::characters`]) at
/// the end.
const TICK_3602_29: &str = "72420f0bab54d2357f6c2007b3d94bab905067a0cafa242c4e6902302706f1b7";
/// The same at tick 3603, after the two neighbors are made.
const TICK_3603: &str = "d63adb8f22e90455f0a702c6352a4337796083ab267ed5fda6257746d08d3f02";
/// The same state's hash under snapshot format 31.
const TICK_3603_31: &str = "98fc002d96b1afd8e4a8abf5de638d86b6c70dfcb567e0bd79c8bc21c6d04a4f";
/// The same state's hash under snapshot format 30, which had no person
/// types whose portrait the interface built ([`sims_sim::World::portraits`])
/// at the end.
const TICK_3603_30: &str = "512738a0ecce31e27efd58e297632a3de66775f4df3eed76adf002f068a48d67";
/// The same state's hash under snapshot format 29, which had no person
/// types made while the game runs ([`sims_sim::World::characters`]) at
/// the end.
const TICK_3603_29: &str = "d2596aa251487225ce687aa8b69daf749e857d87b650e3a00c4cc34b877f9e89";
/// The same at tick 3912.
const TICK_3912: &str = "d1ed0b309460fd0e1d338b8c71b28a6a2c713aa6b92c378af2da745c47ff0f83";
/// The same state's hash under snapshot format 31.
const TICK_3912_31: &str = "95d787696736ad2187b34b1248218e59db3e54721c7372884380bf53a1df9a6f";
/// The same state's hash under snapshot format 30, which had no person
/// types whose portrait the interface built ([`sims_sim::World::portraits`])
/// at the end.
const TICK_3912_30: &str = "869973a29b9ec91cb38a0a5c3dc461315dcadc2034942635bd276928dc2a733f";
/// The same state's hash under snapshot format 29, which had no person
/// types made while the game runs ([`sims_sim::World::characters`]) at
/// the end.
const TICK_3912_29: &str = "4709b76086b6c75eb736f45d20bae15272df19352656c1a8a53312d73c23a162";
/// The same at tick 9906.
const TICK_9906: &str = "d630e0f02bf88ba7b955223daafcbb29668db33e57839cf2db396a54699c5298";
/// The same state's hash under snapshot format 31.
const TICK_9906_31: &str = "cd6bf73af970a9fb740e063ad6f2f52614f525c8fb85d413fe6f4910dc9c83f5";
/// The same state's hash under snapshot format 30, which had no person
/// types whose portrait the interface built ([`sims_sim::World::portraits`])
/// at the end.
const TICK_9906_30: &str = "0729bdb1be01619a16bdfb2852da323bf1237ecc1bf8bfa156ea52b75a506992";
/// The same state's hash under snapshot format 29, which had no person
/// types made while the game runs ([`sims_sim::World::characters`]) at
/// the end.
const TICK_9906_29: &str = "458debd6fa82f9cd865b4334cdd950af01942ee517cf039657728eb6422a54f2";
/// The same at tick 20186.
const TICK_20186: &str = "58aec17557a340a501be645247d8e9b97353d810e0a456577edf140e26102882";
/// The same state's hash under snapshot format 31.
const TICK_20186_31: &str = "20c02362265912d3469be08f31e90e6bfecd59ddd33db3332e1b4be3da0ae0bb";
/// The same state's hash under snapshot format 30, which had no person
/// types whose portrait the interface built ([`sims_sim::World::portraits`])
/// at the end.
const TICK_20186_30: &str = "5ee29d0a10fc9f489b1cb1c76025ec81bcebc77fbf5c508072bf867c48dce5bb";
/// The same state's hash under snapshot format 29, which had no person
/// types made while the game runs ([`sims_sim::World::characters`]) at
/// the end.
const TICK_20186_29: &str = "a282f437760e7a7b5308007acc06d094a26bf6a563f7bc9863ac5d1b3013d80e";
/// The same at tick 31478, when the day turns.
const TICK_31478: &str = "89e73f89da7f801d50e6305a9636520249ece061c8bf8628c563d72b1e9d7bab";
/// The same state's hash under snapshot format 31.
const TICK_31478_31: &str = "eadda68a0fdb21dd003cafccca2a89018f0a07dbac3dfd77683c7e3dad2c9b75";
/// The same state's hash under snapshot format 30, which had no person
/// types whose portrait the interface built ([`sims_sim::World::portraits`])
/// at the end.
const TICK_31478_30: &str = "e4d6da3c5ee38e6bed9bcd533159e916ee1680f163539a83e5a34900890b19ff";
/// The same state's hash under snapshot format 29, which had no person
/// types made while the game runs ([`sims_sim::World::characters`]) at
/// the end.
const TICK_31478_29: &str = "387eded5b6d27220500d121b74d48bb67b17b6f2875c81a9e3e65a0111825476";
/// Where to start the C runtime's generator for House 69's load in track
/// 2's second run: state 1 after this many draws, fitted so that our
/// load's own 4,730 draws reach the recording's entry state (state 1 after
/// 405,850 draws). It is the offset for booting from the original's own
/// files with [`Lot::load_with`], so it stands for the open's import and
/// pools and for any of our load's draws the original makes earlier
/// (the indicators, the adult skeleton). That the generator ran on from
/// state 1 with no reseed is assumed, not observed.
const FILE_BOOT_DRAWS: u32 = 401_120;
/// Where our open of run 2 starts the C runtime: state 1 after this many
/// draws, fitted so that the first pool person's GUID low word and traits
/// are the original's. It stands for startup, the neighborhood view and
/// the `Patch` import, which our open does not draw for. That the
/// generator ran on from state 1 with no reseed is assumed, not observed.
const OPEN_DRAWS: u32 = 84_947;
/// The frames track 2's House 69 recordings drew: after updates 0 to 2,
/// 29, 59, 89, 101 and 102, then every 30th from 123
/// (`house69-ud2-ticks-20261120/watch.jsonl`, the frame watch outside
/// an update).
const RECORDED_FRAMES: &str = "0,1,2,29,59,89,101,102,123+30";
/// The imported House 69's state at entry (format 32), a regression pin of
/// our run. Track 2's recording of it (`house69-ud2-ticks-20261120`)
/// differs at entry in object 2's last result; see the decision
/// `20261120-ud2-house-69-boots-from-the-originals-import.md` for what
/// the comparison reads and skips.
const IMPORTED_69: &str = "ffd020edb61f09e4953b7efb5f5019cb0553148dfd23e5112071c8f411fb8ff6";
/// The same state's hash under snapshot format 31, which had no dirty
/// portrait types ([`sims_sim::World::dirty_portraits`]) at the end.
const IMPORTED_69_31: &str = "1bc128440944c2a9bfd8dc189aaa51249ee96b3eb0187dc721dcad28e4519447";
/// The same state's hash under snapshot format 30, which had no person
/// types whose portrait the interface built ([`sims_sim::World::portraits`])
/// at the end.
const IMPORTED_69_30: &str = "9ca0dd8afe6c5b848be0d6cdd3c8b03c006e1f2d612843c1a6bb8fc02a6bccb0";
/// The same after 1000 ticks, with the recording's frames: a regression
/// pin, which differs from the recording's update 1000 in object 2's last
/// result.
const IMPORTED_69_1000: &str = "125946e1704ba6aa9c044e84c013e7f1feeb215edc8aecf4a90492525d603038";
/// The same state's hash under snapshot format 31.
const IMPORTED_69_1000_31: &str =
    "d0a4eeb7b002caf2bd1cf872c829a9517d11665cf5dc68a9cfd6c6a01c73c210";
/// The same state's hash under snapshot format 30.
const IMPORTED_69_1000_30: &str =
    "8d0258ba3e1a8922a59f365ad4a4031d76a9365d97d20a264c1e039cd829a921";
/// The same at tick 43200, a whole day from the load.
const TICK_43200: &str = "df6eb544be87b93233011f355999ac9abb0c36dc5e914f2f7c0fe2f49967b19a";
/// The same state's hash under snapshot format 31.
const TICK_43200_31: &str = "c3b7a6b9a8180d9e5e6463ba4e3638313cdec8d52bdaae28918bb05ce987e915";
/// The same state's hash under snapshot format 30, which had no person
/// types whose portrait the interface built ([`sims_sim::World::portraits`])
/// at the end.
const TICK_43200_30: &str = "b9a5b6ba4df149edcf1f7f68be72a6c2a02e4f692864bce1b543e062f29379e4";
/// The same state's hash under snapshot format 29, which had no person
/// types made while the game runs ([`sims_sim::World::characters`]) at
/// the end.
const TICK_43200_29: &str = "723a4322520c6497fcc6317247dd5079f2b4812e64bc11f7a114dbef7883c0bd";

#[test]
fn strict_mode_runs_the_goth_houses_whole_day() {
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL not set; skipping");
        return;
    };
    let root = PathBuf::from(root);
    let house = root.join("UserData/Houses/House05.iff");
    // The second answer is the adoption call's at tick 17707 (see below).
    let mut lot = Lot::load_viewed(
        &root,
        &house,
        &[3, 3],
        Frames::default(),
        Some(GOTH_VIEW_CENTRE),
    )
    .unwrap();
    assert_eq!(lot.view_centre(), Some(GOTH_VIEW_CENTRE));
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
    assert_eq!(digest(&format_31_bytes(lot.world())), TICK_2881_31);
    assert_eq!(digest(&format_30_bytes(lot.world())), TICK_2881_30);
    assert_eq!(digest(&format_29_bytes(lot.world())), TICK_2881_29);
    // Each update is two game seconds, 30 a game minute, so a day is
    // 43,200 updates.
    assert_eq!(clock(&lot), (8, 6, 46));
    for _ in 2882..=3432 {
        lot.step().unwrap();
    }
    assert_eq!(lot.world().hash_hex(), TICK_3432);
    assert_eq!(digest(&format_31_bytes(lot.world())), TICK_3432_31);
    assert_eq!(digest(&format_30_bytes(lot.world())), TICK_3432_30);
    assert_eq!(digest(&format_29_bytes(lot.world())), TICK_3432_29);
    assert_eq!(clock(&lot), (8, 25, 8));
    for _ in 3433..=3602 {
        lot.step().unwrap();
    }
    assert_eq!(lot.world().hash_hex(), TICK_3602);
    assert_eq!(digest(&format_31_bytes(lot.world())), TICK_3602_31);
    assert_eq!(digest(&format_30_bytes(lot.world())), TICK_3602_30);
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
    assert_eq!(digest(&format_31_bytes(lot.world())), TICK_3603_31);
    assert_eq!(digest(&format_30_bytes(lot.world())), TICK_3603_30);
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
    assert_eq!(digest(&format_31_bytes(lot.world())), TICK_3912_31);
    assert_eq!(digest(&format_30_bytes(lot.world())), TICK_3912_30);
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
    assert_eq!(digest(&format_31_bytes(lot.world())), TICK_9906_31);
    assert_eq!(digest(&format_30_bytes(lot.world())), TICK_9906_30);
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
    assert_eq!(digest(&format_31_bytes(lot.world())), TICK_20186_31);
    assert_eq!(digest(&format_30_bytes(lot.world())), TICK_20186_30);
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
    // links had 12653; before Find Location For mode 5 fixed its swapped
    // axes it had 12524. The architecture's value is not ported and keeps
    // the saved one.
    assert_eq!(values(&lot), (12539, 15277));
    assert_eq!(lot.world().hash_hex(), TICK_31478);
    assert_eq!(digest(&format_31_bytes(lot.world())), TICK_31478_31);
    assert_eq!(digest(&format_30_bytes(lot.world())), TICK_31478_30);
    assert_eq!(digest(&format_29_bytes(lot.world())), TICK_31478_29);
    assert_eq!((date(&lot), clock(&lot)), ((17, 6, 1997, 1), (0, 0, 0)));
    // Tick 37803: object 359 (tree 0x100a) makes an object ahead of its
    // stack object (Create New Object Instance mode 3, flags 0x19).
    for _ in 31479..=43200 {
        lot.step().unwrap();
    }
    // A whole day from the load: the clock is back where it started.
    assert_eq!(lot.world().hash_hex(), TICK_43200);
    assert_eq!(digest(&format_31_bytes(lot.world())), TICK_43200_31);
    assert_eq!(digest(&format_30_bytes(lot.world())), TICK_43200_30);
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
    let mut lot = Lot::load_viewed(&root, &house, &[3, 3], frames, Some(GOTH_VIEW_CENTRE)).unwrap();
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
    let recorded: [(u32, u32, u32); 15] = [
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
        // Tick 24716: person 12 takes itself off the lot and then, before
        // its animator's tick, runs Go To Routing Slot, which reads its
        // animator. The step runs both ways, with and without the
        // animator's refresh at the take-off (0x58da12), and the two agree
        // (`docs/decisions/20261120-a-step-runs-both-ways-while-a-refresh-is-pending.md`).
        (24716, 2_341_770_929, 1_381_078_483),
        // Tick 24719: object 18's dialog shows a neighbor's portrait (icon
        // mode 2). Nobody of that cat's type is on the lot, so the
        // interface makes one and removes it at once (0x4e9652, 0x44123f):
        // one cat skeleton's worth of draws.
        (24719, 2_341_770_929, 226_159_443),
        // Tick 24766: a dog is made, one dog skeleton's worth (1,776): the
        // dog file was read before the load too.
        (24766, 3_051_544_665, 3_626_526_211),
        (36717, 2_118_795_333, 3_741_955_181),
    ];
    let mut tick = 0;
    for (at, lot_rng, crt) in recorded {
        while tick < at {
            if tick == 24718 {
                // At 24719 every start — including the four no-room tiles
                // (19,24; 23,31; 26,28; 26,31) via 0x4e9706 + BMP_ 2002 —
                // and view_centre = None all reach the exact same state.
                every_start_gives_one_state(&mut lot);
            }
            lot.step().unwrap();
            tick += 1;
        }
        assert_eq!(streams(&lot), (lot_rng, 270_289_136, crt), "after {at}");
    }
    // From 36718 the lot stream moves one update before or after the
    // recording's at a few updates: 36718, 36929, 37058, 37126, and 37215
    // from the fitted frame schedule's drift against the recording's frames,
    // and after 40317 by one update on the raccoon's draws — inferred from
    // the binary's unmodeled Special Effect PIP paint (0x4b3489 → 0x58da12)
    // refreshing the raccoon's animator (`goth-day-20261115` watched no
    // frames) — before matching again at the next update. The C runtime
    // stream matches the recording's at every update of the day.
    //
    // At tick 39610 the raccoon (object 10) runs Special Effect with flag
    // 0x40 clear while everyone sleeps at idle speed 1000; message 0x10c
    // drops the lot back to normal speed 760 (0x44abda..0x44abee →
    // 0x4f684c..0x4f6873), matching `goth-day-20261115`'s update 39610.
    while tick < 39_609 {
        lot.step().unwrap();
        tick += 1;
    }
    assert_eq!(
        (lot.world().speed(), lot.world().return_speed()),
        (1000, 760)
    );
    lot.step().unwrap();
    tick += 1;
    assert_eq!(
        (lot.world().speed(), lot.world().return_speed()),
        (760, 760)
    );
    while tick < 40_494 {
        lot.step().unwrap();
        tick += 1;
    }
    assert_eq!(
        (lot.world().speed(), lot.world().return_speed()),
        (760, 760)
    );
    lot.step().unwrap();
    tick += 1;
    assert_eq!((lot.world().speed(), lot.world().return_speed()), (760, -1));
    // At tick 41446 (one update after the recording's 41445 — inferred to
    // follow an unmodeled Special Effect PIP paint refreshing the raccoon's
    // animator via 0x4b3489 → 0x58da12 around update 40317; the recording
    // watched no frames), the raccoon (object 10) runs Wander (tree 419
    // node 8: Find Location For mode 5) and places its wander target
    // (object 37, GUID 0x000007C4) at [648, 312, 1] (tile (40, 19)),
    // matching `goth-day-20261115`'s update 41445.
    while tick < 41_446 {
        lot.step().unwrap();
        tick += 1;
    }
    let wander = &lot.world().objects[&37];
    assert_eq!(wander.guid, 0x0000_07c4);
    assert_eq!(wander.position, [648, 312, 1]);
    while tick < 43_200 {
        lot.step().unwrap();
        tick += 1;
    }
    assert_eq!(
        streams(&lot),
        (5_524_294, 270_289_136, 2_700_240_999),
        "after 43200"
    );
}

// The Newbies' pins follow Remove Object Instance saving a removed
// person into its neighborhood record (0x451bee, 0x425a69;
// docs/decisions/20261119-generic-sims-call-6-and-the-saved-neighbor-record.md):
// with that save left out, the earlier pins all held.

/// The Newbies' house (House07) after 1802 updates, with `--answers 3`.
const NEWBIE_1802: &str = "c48b2f3896ce0bd2dfe1574288d69b62bdedca4ff09d1f1c760e941eb370ad45";
/// The same state's hash under snapshot format 31, which had no dirty
/// portrait types ([`sims_sim::World::dirty_portraits`]) at the end.
const NEWBIE_1802_31: &str = "db81050f8a226c22fda89496a8485387f772ecb33480d53c5e2207532ecd2ac0";
/// The same state's hash under snapshot format 30, which had no person
/// types whose portrait the interface built ([`sims_sim::World::portraits`])
/// at the end.
const NEWBIE_1802_30: &str = "87e686fc5db2ece81643b2c87505e4c1589441acf30022c1cb4d977f96a89593";
/// The same state's hash under snapshot format 29, which had no person
/// types made while the game runs ([`sims_sim::World::characters`]) at
/// the end.
const NEWBIE_1802_29: &str = "41b1b65f24bb01bf31171825fc963936d786a77ddf8375b860eac46cf8b24ce2";
/// The same state's hash under snapshot format 28, which had no record
/// of the people whose animator was asked to stop its route
/// ([`sims_sim::World::stops_asked`]) at the end.
const NEWBIE_1802_28: &str = "2a2d5d3e9f9bea42598e7efc8b424d84338f16db99682958d71b2b0cb4ac6d42";
/// The same state's hash under snapshot format 27, which had no poses
/// the load left in the skins ([`sims_sim::World::held_poses`]) either.
const NEWBIE_1802_27: &str = "5055fe7a40f7c1ade093d0748e7294bf92251264d4452304a8601eac89d341fc";
/// The same state's hash under snapshot format 26.
const NEWBIE_1802_26: &str = "db8b8f44bbc8273bcfb0b13e6ba1d405d5cfc78b4c6ec70d7483d240289c7af4";
/// The Newbies' house after 4052 updates, the last whose frame strict
/// mode draws.
const NEWBIE_4052: &str = "13bf15702ca5c43185cc4add8972b30668eb79677c4646ad5857dcac3fdca23a";
/// The same state's hash under snapshot format 31.
const NEWBIE_4052_31: &str = "330dc7e29a0fdff17b58601f3f0ff0190430ac430e933757264d8f450cd9b33e";
/// The same state's hash under snapshot format 30, which had no person
/// types whose portrait the interface built ([`sims_sim::World::portraits`])
/// at the end.
const NEWBIE_4052_30: &str = "161e7f3ce888e5ff2f4dc3257998cde6ec3ddd3eb2e1c23c728e63dad0de7f65";
/// The same state's hash under snapshot format 29, which had no person
/// types made while the game runs ([`sims_sim::World::characters`]) at
/// the end.
const NEWBIE_4052_29: &str = "d98344ac74d997ccd7e795052582aec11ce493fc5ac2a0bf0f9ae78ad349a80b";
/// The same state's hash under snapshot format 28, which had no record
/// of the people whose animator was asked to stop its route
/// ([`sims_sim::World::stops_asked`]) at the end.
const NEWBIE_4052_28: &str = "13c57c305ad63fc916977d987443e251f954254245e6cbaa66916c28768c291e";
/// The same state's hash under snapshot format 27, which had no poses
/// the load left in the skins ([`sims_sim::World::held_poses`]) either.
const NEWBIE_4052_27: &str = "9f09c38494febb26e5951e1e49a1a6fcfd389c19a5d2ffdac219557bd3b69fd2";

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
    assert_eq!(digest(&format_31_bytes(lot.world())), NEWBIE_1802_31);
    assert_eq!(digest(&format_30_bytes(lot.world())), NEWBIE_1802_30);
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
    assert_eq!(digest(&format_31_bytes(lot.world())), NEWBIE_4052_31);
    assert_eq!(digest(&format_30_bytes(lot.world())), NEWBIE_4052_30);
    assert_eq!(digest(&format_29_bytes(lot.world())), NEWBIE_4052_29);
    assert_eq!(digest(&format_28_bytes(lot.world())), NEWBIE_4052_28);
    assert_eq!(digest(&format_27_bytes(lot.world())), NEWBIE_4052_27);
    // The frame drawn after tick 4053 draws newly arrived person 142 for the
    // first time (`undrawn` / byte +0xdb): step 3 (`0x5893a9` → `0x589361` at
    // `0x58939e`) sets byte +0xd9 unconditionally, so `0x590d26` runs
    // `update_skin` (`0x5992b3`) and tick 4053 now completes cleanly.
    let tick4053 = lot.step().unwrap();
    assert!(tick4053.errors.is_empty());
    assert!(tick4053.frame_errors.is_empty());
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
fn a_paused_goth_house_wakes_counter_117_at_idle_20000() {
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL not set; skipping");
        return;
    };
    let root = PathBuf::from(root);
    let house = root.join("UserData/Houses/House05.iff");
    let mut lot = Lot::load(&root, &house, &[3], Frames::default()).unwrap();
    // On Complete Collection, Counter - Kitchen - Moderate (0x91767a36, IDs
    // 114..=117) resets on load (0x43a516 -> 0x431fbe) to data[0] = 0
    // (straight DGRP #100) with data[60] = 1 (scheduler bit 17: runs in
    // paused partial passes, 0x46d553) and enters Main -> Idle(20000).
    for id in 114..=117 {
        let counter = &lot.world().objects[&id];
        assert_eq!(counter.data[0], 0, "counter {id} starts at DGRP #100");
        assert_eq!(counter.data[60], 1, "counter {id} runs while paused");
        assert_ne!(counter.flags() & 2, 0, "counter {id} has scheduler flag 2");
    }
    for _ in 0..3189 {
        assert!(lot.step().unwrap().full);
    }
    assert_eq!(lot.world().objects[&117].data[0], 0);
    assert_eq!(lot.pause(), Ok(true));
    let clock = |lot: &Lot| {
        let g = &lot.world().globals;
        (lot.world().ticks, g[0], g[5], g[6])
    };
    let frozen = clock(&lot);
    for paused_tick in 1..16817u32 {
        let update = lot.step().unwrap();
        assert!(!update.full, "paused tick {paused_tick}");
        assert_eq!(
            lot.world().objects[&117].data[0],
            0,
            "counter 117 still sleeping at paused tick {paused_tick}"
        );
    }
    let wake = lot.step().unwrap();
    assert!(!wake.full, "paused tick 16817");
    assert_eq!(clock(&lot), frozen, "clock stands still while paused");
    assert_eq!(
        lot.world().objects[&117].data[0],
        1,
        "counter 117 wakes from Idle(20000) while paused and selects corner DGRP #101"
    );
    for id in [114, 115, 116] {
        assert_eq!(
            lot.world().objects[&id].data[0],
            0,
            "straight counter {id} stays at DGRP #100"
        );
    }
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

#[test]
fn the_default_button_labels_hold_no_token() {
    // A dialog's string that is number 0, missing or empty is the default
    // its caller gives, a button's label from STR# 142 of
    // `GameData/UIText.iff` (0x435fee, 0x43600a, 0x436a21), and the
    // formatter (0x44bf1a) reads it as it reads the others. The port
    // leaves those strings out of the formatter's work
    // (`Machine::dialog_strings`) only when the link finds no table of
    // them holding a `$` (`Program::labels_hold_no_token`), and fails
    // loudly otherwise. The install's labels, in US English (language 1),
    // hold no `$`, so no token.
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL not set; skipping");
        return;
    };
    let path = PathBuf::from(root).join("GameData/UIText.iff");
    let iff = sims_formats::iff::Iff::parse(std::fs::read(path).unwrap()).unwrap();
    let chunk = iff.get(b"STR#", 142).unwrap();
    let table = sims_formats::strings::StringTable::parse(iff.data(chunk)).unwrap();
    let labels: Vec<&str> = (0..table.entries.len())
        .filter_map(|i| table.get(i, 1))
        .collect();
    assert_eq!(labels[..4], ["OK", "Cancel", "Yes", "No"]);
    for label in labels {
        assert!(!label.contains('$'), "{label:?}");
    }
    // And the boot finds so.
    let root = PathBuf::from(std::env::var_os("SIMS_INSTALL").unwrap());
    let house = root.join("UserData/Houses/House05.iff");
    let lot = Lot::load(&root, &house, &[3], Frames::default()).unwrap();
    assert!(lot.program().labels_hold_no_token);
}

#[test]
fn the_installs_career_texts_hold_no_career_token() {
    // A person's dialog with a career token writes career texts
    // (0x44c966), and a pass searches again from one past the place it
    // wrote (0x44ca9f to 0x44cad3): the port runs them only when no string
    // of `work.iff` holds a career token past its first character
    // (`Program::career_texts_end`). The install's do not; its career
    // levels' strings hold a `$` (in the names of suits), but not a token.
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL not set; skipping");
        return;
    };
    let root = PathBuf::from(root);
    let house = root.join("UserData/Houses/House05.iff");
    let lot = Lot::load(&root, &house, &[3], Frames::default()).unwrap();
    assert!(lot.program().career_texts_end);
}

#[test]
fn play_mode_goth_multi_day_stability() {
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL not set; skipping");
        return;
    };
    let root = PathBuf::from(root);
    let house = root.join("UserData/Houses/House05.iff");
    let mut lot = Lot::load_viewed(
        &root,
        &house,
        &[],
        Frames::default(),
        Some(GOTH_VIEW_CENTRE),
    )
    .unwrap();
    lot.wait_for_answers();
    assert_eq!(lot.world().indicators[0], 383);

    // Run 50,000 Play Mode ticks (> 1.15 full Sim days, well past tick 16,407
    // and midnight rollover at tick 31,478) answering any interactive dialogs
    // or special panels without ever halting the lot.
    const TARGET_TICKS: u32 = 50_000;
    while lot.ticks() < TARGET_TICKS {
        match lot.step_play() {
            Ok(_) => {}
            Err(StepError::Asking { .. }) => {
                if let Some(asking) = lot.asking() {
                    let choice = *asking.shown.answers.last().unwrap_or(&2);
                    lot.answer(choice).unwrap();
                } else if let Some(asking_sp) = lot.asking_special_panel() {
                    let resp = match asking_sp.shown.request {
                        sims_vm::SpecialPanelRequest::CallNeighbor { .. } => {
                            sims_vm::SpecialPanelResponse::Neighbor(None)
                        }
                        sims_vm::SpecialPanelRequest::DowntownLotPicker
                        | sims_vm::SpecialPanelRequest::VacationLotPicker { .. }
                        | sims_vm::SpecialPanelRequest::OldTownLotPicker { .. }
                        | sims_vm::SpecialPanelRequest::StudioTownLotPicker { .. }
                        | sims_vm::SpecialPanelRequest::MagicTownLotPicker { .. } => {
                            sims_vm::SpecialPanelResponse::Lot(None)
                        }
                        sims_vm::SpecialPanelRequest::WardrobeShop { .. }
                        | sims_vm::SpecialPanelRequest::CharmTransform { .. }
                        | sims_vm::SpecialPanelRequest::PetPicker { .. } => {
                            sims_vm::SpecialPanelResponse::Selection {
                                accepted: false,
                                index: 0,
                            }
                        }
                        sims_vm::SpecialPanelRequest::SpellBook { .. }
                        | sims_vm::SpecialPanelRequest::CookBook { .. } => {
                            sims_vm::SpecialPanelResponse::Dismissed
                        }
                    };
                    lot.answer_special_panel(resp).unwrap();
                } else {
                    panic!("StepError::Asking with neither dialog nor special panel");
                }
            }
            Err(other) => {
                panic!(
                    "Play Mode halted on Goth at tick {} (world.ticks={}): {other}",
                    lot.ticks(),
                    lot.world().ticks
                );
            }
        }
    }
    assert_eq!(lot.ticks(), TARGET_TICKS);
    assert!(
        lot.world().globals[28] >= 1,
        "expected midnight day rollover (days_played={})",
        lot.world().globals[28]
    );
    let plumb_bob = lot.world().indicators[0];
    assert_eq!(plumb_bob, 383);
    assert!(
        lot.world().objects.contains_key(&plumb_bob),
        "plumb bob (object 383) must remain registered after 50,000 Play Mode ticks"
    );
}
