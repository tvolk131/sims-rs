//! `sims_vm::boot` on a real install, when `SIMS_INSTALL` names one: the
//! Goth house loads and steps one tick at a time to the state `sims boot`
//! reaches.

use sims_sim::{Headline, World};
use sims_vm::boot::{Frames, Lot, StepError};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// `sims boot <install> UserData/Houses/House05.iff --ticks 33 --answers 3`
/// gives this hash with the default frames (snapshot format 29).
const TICK_33: &str = "9249815b8077306c96d733178dcab54436baf719a1383f9bc70c83ee8f50c9b6";
/// The same state's hash under snapshot format 28, which had no record
/// of the people whose animator was asked to stop its route
/// ([`sims_sim::World::stops_asked`]) at the end.
const TICK_33_28: &str = "33259a54e8ab263c1167787fd11a9a16edfea06fa3ce7f3d354a6f65792fe3fc";
/// The same state's hash under snapshot format 28 before the load kept
/// the people's saved head targets ([`without_saved_targets`]).
const TICK_33_UNTARGETED: &str = "a2acc40dd00ab2d31b32a04f087a3b03ee26d360b1c443fcd159dd0c7d9df1a1";
/// The same state's hash under snapshot format 27, which had no poses
/// the load left in the skins ([`sims_sim::World::held_poses`]) either.
const TICK_33_27: &str = "db1ce596748d8f9c5a39ba4197203ad261f354d545cc71a4ef8897d92bafba7d";
/// The same state's hash under snapshot format 26, which had no
/// neighborhood data words ([`sims_sim::World::neighborhood_data`]) at
/// the end.
const TICK_33_26: &str = "915a9af271f2d48b191c3c1d7f4d0add337aef4c5a5899fae0a1ba02c5657e11";
/// The same state's hash under snapshot format 25, which had no record of
/// the bones the skins moved ([`sims_sim::World::translated_bones`])
/// either.
const TICK_33_25: &str = "3900ae00e04dd717ef5d4a03d9e7629f7843f95d22e54c97733349d16cd147f3";
/// The same state's hash under snapshot format 24, which had no base
/// layers ([`sims_sim::World::base_layers`]) either.
const TICK_33_24: &str = "da80de5807a89da3fdae78c8759d3e411bdd87c5dc7aae0bafceec963cf59796";
/// The same state's hash under snapshot format 23, which had no headlines
/// ([`sims_sim::World::headlines`]) either.
const TICK_33_23: &str = "9d5a04c006c0e77b014642afe9113d5b5ec7c250787caf001b6ede4a16bc9e99";
/// The same state's hash under snapshot format 22, which had no families
/// ([`sims_sim::World::families`]) either.
const TICK_33_22: &str = "041181badfd4d6953814b6c46612398df1004fa583b1d2aa5bd11c59715d65c0";

/// `sims boot <install> UserData/Houses/House05.iff --ticks 1103 --answers
/// 3` gives this hash, the tick before the salesman leaves.
const TICK_1103: &str = "df971523b4a1263f417dae2dbbab87df48de82a8bdf0ef91ae216472ccdfb17f";
/// The same state's hash under snapshot format 28, which had no record
/// of the people whose animator was asked to stop its route
/// ([`sims_sim::World::stops_asked`]) at the end.
const TICK_1103_28: &str = "509fdba01d87a907a13b367a81ee677ed8b4f09bca0fdc88881af7f5bb56ae12";
/// The same state's hash under snapshot format 27, which had no poses
/// the load left in the skins ([`sims_sim::World::held_poses`]) either.
const TICK_1103_27: &str = "f5330f8aed627dda3529a1132e90fc15feb7584ae2757133b3d95074b1084f7a";
/// The same state's hash under snapshot format 26.
const TICK_1103_26: &str = "ce627e5ef44b7133e74322f8d5dcf1929821114529b846a4c1e442475470399a";
/// The same state's hash under snapshot format 25.
const TICK_1103_25: &str = "54a5adc12bd7ae9641a7029acb435272121c6678fc938ac95036f788b6b5bc86";
/// The same state's hash under snapshot format 24.
const TICK_1103_24: &str = "714496702b0ff2dbda94c299c217e2ae7414172e6a71775950901b0cfa6222b5";

fn digest(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The world's encoding as snapshot format 28 wrote it: without the
/// people whose animator was asked to stop its route
/// ([`sims_sim::World::stops_asked`]) that format 29 added at the end, so
/// a state pinned under format 28 can be shown to be the same state but
/// for them.
fn format_28_bytes(world: &World) -> Vec<u8> {
    let mut bytes = world.encode();
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
    // The lot runs on until person 259 (the salesman), whose route to a
    // slot failed as it wants to be interrupted (0x45b18b), is taken off
    // the lot while its animator plays a door knock: the reset that
    // deletes it would remove that clip, which can fade it out, and that
    // is not ported.
    let mut steps = 359;
    let stopped = loop {
        steps += 1;
        assert!(steps <= 1000, "the lot did not stop");
        if let Err(e) = lot.step() {
            break e.to_string();
        }
    };
    assert_eq!(steps, 595, "{stopped}");
    for part in ["tick 597,", "object 259", "A2O-door-knock", "0x592878"] {
        assert!(stopped.contains(part), "{stopped}");
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
    for step in 3604..15799 {
        let update = lot.step_play().unwrap();
        assert!(update.errors.is_empty(), "step {step}: {:?}", update.errors);
    }
    // No fire is lit on the way: the run before Snap's slot offset was
    // fixed lit one at step 13199 (docs/decisions/
    // 20261115-snap-offset-order.md). Step 15799: person 21 is off the
    // lot, where its model matrix (0x45ee1d) is not ported; play mode
    // records the error and restarts it.
    assert!(!lot.world().objects.values().any(|o| o.guid == 0x24c9_5f99));
    let update = lot.step_play().unwrap();
    let [error] = &update.errors[..] else {
        panic!("{:?}", update.errors);
    };
    assert_eq!(error.object, 21);
    assert!(
        format!("{error:?}").contains("model matrix off the lot (0x45ee1d)"),
        "{error:?}"
    );
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
// the idle call; see docs/decisions/20261115-snap-offset-order.md. They hold
// only this snapshot format: the state changed with those fixes, so no
// earlier format's hash is kept for them.

/// `sims boot <install> UserData/Houses/House05.iff --ticks 2881
/// --answers 3` gives this hash.
const TICK_2881: &str = "e32437a9c22924668ff3a1823a2526ad919dfde7a11abdaf47df3d27730d4412";
/// The same at tick 3432.
const TICK_3432: &str = "e29881bf373b9fad3b2e787ee1e6988e3e590e9af9e7b1dcc1426b8cc109b17b";
/// The same at tick 3602, the last whose lot random stream matches track
/// 2's recording `goth-day-20261115`.
const TICK_3602: &str = "cd8867b4ef4ca527e10b05ba35250ca602327313ce41aa2abcaeaf8b3c234779";
/// The same at tick 3603.
const TICK_3603: &str = "75a42babdbbe59e92a462a31753870ab92fb435532f8037e0c73e863ec5a55ac";
/// The same at tick 3912.
const TICK_3912: &str = "21fb83c7f0ab9c284793e6453e08502e6e062cc4c0dd14f5c71c475f08df4a5b";
/// The same at tick 9906.
const TICK_9906: &str = "6f5c53af3fd9d4b4c09ed427336f2f54436abe9b3b757a4921a5a661f8f7ae9f";
/// The same at tick 20186.
const TICK_20186: &str = "75d9aa22fc75355097f0b41385527cc926e58d803c7b601625e3e57645e837c3";
/// The same at tick 31478, when the day turns.
const TICK_31478: &str = "c413cafdef718d3be7a54188f50c2cfaa3039430874b09a4acfdf698f8f92c2e";
/// The same at tick 43200, a whole day from the load.
const TICK_43200: &str = "9350f8b0de69676e1df4c7999151a58dbe5c851c610d944a33ddac53b3eacd07";

#[test]
fn strict_mode_runs_the_goth_houses_whole_day() {
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL not set; skipping");
        return;
    };
    let root = PathBuf::from(root);
    let house = root.join("UserData/Houses/House05.iff");
    let mut lot = Lot::load(&root, &house, &[3], Frames::default()).unwrap();
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
    // Each update is two game seconds, 30 a game minute, so a day is
    // 43,200 updates.
    assert_eq!(clock(&lot), (8, 6, 46));
    for _ in 2882..=3432 {
        lot.step().unwrap();
    }
    assert_eq!(lot.world().hash_hex(), TICK_3432);
    assert_eq!(clock(&lot), (8, 25, 8));
    for _ in 3433..=3602 {
        lot.step().unwrap();
    }
    assert_eq!(lot.world().hash_hex(), TICK_3602);
    assert_eq!(clock(&lot), (8, 30, 48));
    // Tick 3603: object 354 (tree 0x1022) picks two neighbor records at
    // random, makes the person of each off the lot (Create New Object
    // Instance mode 6, flag 0x4) as objects 12 and 24, the lowest IDs
    // free, writes their person words 80 and 81, and removes them both;
    // they are deleted at the end of the update.
    let tick = lot.step().unwrap();
    assert_eq!(tick.removed, [12, 24]);
    assert!(!lot.world().objects.contains_key(&12));
    assert!(!lot.world().objects.contains_key(&24));
    assert_eq!(lot.world().hash_hex(), TICK_3603);
    for _ in 3604..=3912 {
        lot.step().unwrap();
    }
    assert_eq!(lot.world().hash_hex(), TICK_3912);
    assert_eq!(clock(&lot), (8, 41, 8));
    for _ in 3913..=9906 {
        lot.step().unwrap();
    }
    assert_eq!(lot.world().hash_hex(), TICK_9906);
    assert_eq!(clock(&lot), (12, 0, 56));
    // The date: globals 1, 7 and 8 are the day, month and year, global 28
    // the days played.
    let date = |lot: &Lot| {
        let globals = &lot.world().globals;
        (globals[1], globals[7], globals[8], globals[28])
    };
    assert_eq!(date(&lot), (16, 6, 1997, 0));
    // Tick 20187: object 16's tree 0x017f makes object 43, which turns to
    // the facing of the frame's stack object (0x433f75).
    for _ in 9907..=20186 {
        lot.step().unwrap();
    }
    assert_eq!(lot.world().hash_hex(), TICK_20186);
    // Tick 24770: a placement tests the walls beside the object's tile
    // (0x476f5a). Tick 26573: a person that took itself off removes
    // itself and yields (0x58da12), and a person using a changed object
    // resets fully as it lets go (0x45e814). Tick 33097: Look Towards
    // mode 1 turns towards the camera. Tick 36311: Create New Object
    // Instance mode 0 with flags 0x9 (0x44e1e6, 0x44e51e).
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
    // lost value for the day. This run's objects lose more than those of
    // the run before the fixes (12524), being in other states by then
    // (unverified). The architecture's value is not ported and keeps the
    // saved one.
    assert_eq!(values(&lot), (12653, 15277));
    assert_eq!(lot.world().hash_hex(), TICK_31478);
    assert_eq!((date(&lot), clock(&lot)), ((17, 6, 1997, 1), (0, 0, 0)));
    for _ in 31479..=43200 {
        lot.step().unwrap();
    }
    // A whole day from the load: the clock is back where it started.
    assert_eq!(lot.world().hash_hex(), TICK_43200);
    assert_eq!((date(&lot), clock(&lot)), ((17, 6, 1997, 1), (6, 30, 44)));
}

/// The Newbies' house (House07) after 1802 updates, with `--answers 3`.
const NEWBIE_1802: &str = "477ad5cf8a137143d15448487786dbdfac30dd59e6bdf2aa5eca9f82b7381bb9";
/// The same state's hash under snapshot format 28, which had no record
/// of the people whose animator was asked to stop its route
/// ([`sims_sim::World::stops_asked`]) at the end.
const NEWBIE_1802_28: &str = "dbc6f3b365d70897ebc5d07bdeb086e9819880e83eddb954ba3b12672ac65d50";
/// The same state's hash under snapshot format 27, which had no poses
/// the load left in the skins ([`sims_sim::World::held_poses`]) either.
const NEWBIE_1802_27: &str = "561330e47a0f5f0a1553e4382c16c87a15ff8878fff533fd375a36f24fbf177b";
/// The same state's hash under snapshot format 26.
const NEWBIE_1802_26: &str = "d7980d86529f1b84be10c103d0d522e5484fc54ee2484859beba71b74c8c1d58";
/// The Newbies' house after 4052 updates, the last whose frame strict
/// mode draws.
const NEWBIE_4052: &str = "e4cba62b5344fa29526919fccf72069dac72442ad111c06a979dddf32d56a23b";
/// The same state's hash under snapshot format 28, which had no record
/// of the people whose animator was asked to stop its route
/// ([`sims_sim::World::stops_asked`]) at the end.
const NEWBIE_4052_28: &str = "f89e3f50e2670999480dd1d852b0ea22e125db6d060e80d10f37a62ee7b8e25f";
/// The same state's hash under snapshot format 27, which had no poses
/// the load left in the skins ([`sims_sim::World::held_poses`]) either.
const NEWBIE_4052_27: &str = "39b2fb2f49cdda87cfa0f1c6a56acc5a2aaac8e72fdf82ac1d02ef8e6b590d14";

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
    assert_eq!(digest(&format_28_bytes(lot.world())), NEWBIE_1802_28);
    assert_eq!(digest(&format_27_bytes(lot.world())), NEWBIE_1802_27);
    assert_eq!(digest(&format_26_bytes(lot.world())), NEWBIE_1802_26);
    // Tick 1803: object 290 (tree 0x1022) makes two neighbors' people off
    // the lot (Create New Object Instance mode 6, flag 0x4) and removes
    // them, as the Goths' object 354 does at tick 3603. Unverified: the
    // recording ends at update 935.
    let tick = lot.step().unwrap();
    assert_eq!(tick.removed.len(), 2);
    for _ in 1804..=4052 {
        lot.step().unwrap();
    }
    assert_eq!(lot.world().hash_hex(), NEWBIE_4052);
    assert_eq!(digest(&format_28_bytes(lot.world())), NEWBIE_4052_28);
    assert_eq!(digest(&format_27_bytes(lot.world())), NEWBIE_4052_27);
    // The frame drawn after tick 4053 finds person 140's skin waiting for
    // its update (0x590d26), which is not ported.
    let stopped = lot.step().unwrap_err().to_string();
    for part in ["4053", "person 140", "0x590d26"] {
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
