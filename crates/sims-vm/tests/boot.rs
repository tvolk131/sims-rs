//! `sims_vm::boot` on a real install, when `SIMS_INSTALL` names one: the
//! Goth house loads and steps one tick at a time to the state `sims boot`
//! reaches.

use sims_sim::Headline;
use sims_vm::boot::{Frames, Lot, StepError};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// `sims boot <install> UserData/Houses/House05.iff --ticks 33 --answers 3`
/// gives this hash with the default frames (snapshot format 28).
const TICK_33: &str = "a2acc40dd00ab2d31b32a04f087a3b03ee26d360b1c443fcd159dd0c7d9df1a1";
/// The same state's hash under snapshot format 27, which had no record
/// of the people whose animator was asked to stop its route
/// ([`sims_sim::World::stops_asked`]) at the end.
const TICK_33_27: &str = "db1ce596748d8f9c5a39ba4197203ad261f354d545cc71a4ef8897d92bafba7d";
/// The same state's hash under snapshot format 26, which had no
/// neighborhood data words ([`sims_sim::World::neighborhood_data`])
/// either.
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
const TICK_1103: &str = "509fdba01d87a907a13b367a81ee677ed8b4f09bca0fdc88881af7f5bb56ae12";
/// The same state's hash under snapshot format 27.
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

/// The lot's encoding as snapshot format 27 wrote it: without the people
/// whose animator was asked to stop its route
/// ([`sims_sim::World::stops_asked`]) that format 28 added at the end, so
/// a state pinned under format 27 can be shown to be the same state but
/// for them.
fn format_27_bytes(lot: &Lot) -> Vec<u8> {
    let mut bytes = lot.world().encode();
    let asked = postcard::to_allocvec(&lot.world().stops_asked).unwrap();
    assert_eq!(bytes.split_off(bytes.len() - asked.len()), asked);
    bytes
}

/// The lot's encoding as snapshot format 26 wrote it: without the
/// neighborhood's data words ([`sims_sim::World::neighborhood_data`]) that
/// format 27 added at the end either. It starts from [`format_27_bytes`].
fn format_26_bytes(lot: &Lot) -> Vec<u8> {
    let mut bytes = format_27_bytes(lot);
    let data = postcard::to_allocvec(&lot.world().neighborhood_data).unwrap();
    assert_eq!(bytes.split_off(bytes.len() - data.len()), data);
    bytes
}

/// The lot's encoding as snapshot format 25 wrote it: without the record
/// of the bones the skins moved ([`sims_sim::World::translated_bones`])
/// that format 26 added at the end either, so a state pinned under format
/// 25 can be shown to be the same state but for it. It starts from
/// [`format_26_bytes`].
fn format_25_bytes(lot: &Lot) -> Vec<u8> {
    let mut bytes = format_26_bytes(lot);
    let moved = postcard::to_allocvec(&lot.world().translated_bones).unwrap();
    assert_eq!(bytes.split_off(bytes.len() - moved.len()), moved);
    bytes
}

/// The lot's encoding as snapshot format 24 wrote it: without the base
/// layers ([`sims_sim::World::base_layers`]) that format 25 added at the
/// end either. It starts from [`format_25_bytes`].
fn format_24_bytes(lot: &Lot) -> Vec<u8> {
    let mut bytes = format_25_bytes(lot);
    let layers = postcard::to_allocvec(&lot.world().base_layers).unwrap();
    assert_eq!(bytes.split_off(bytes.len() - layers.len()), layers);
    bytes
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
    assert_eq!(digest(&format_27_bytes(&lot)), TICK_33_27);
    assert_eq!(digest(&format_26_bytes(&lot)), TICK_33_26);
    assert_eq!(digest(&format_25_bytes(&lot)), TICK_33_25);
    let mut bytes = format_24_bytes(&lot);
    assert_eq!(digest(&bytes), TICK_33_24);
    let headlines = postcard::to_allocvec(&lot.world().headlines).unwrap();
    assert_eq!(bytes.split_off(bytes.len() - headlines.len()), headlines);
    assert_eq!(digest(&bytes), TICK_33_23);
    let families = postcard::to_allocvec(&lot.world().families).unwrap();
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
    // Their IDs, 374 and 375, come after 354's, so each has its tick in
    // the same pass while queued, with its animator's refresh at the
    // take-off pending (0x58da12).
    let update = lot.step_play().unwrap();
    assert!(update.errors.is_empty(), "{:?}", update.errors);
    assert_eq!(update.removed, [374, 375]);
    assert!(update.ticked.contains(&374) && update.ticked.contains(&375));
    for step in 3604..13199 {
        let update = lot.step_play().unwrap();
        assert!(update.errors.is_empty(), "step {step}: {:?}", update.errors);
    }
    // Step 13199: object 18 (tree 0x101c node 15) sets something alight
    // with Burn (opcode 9), which is not ported. Unverified: the strict
    // run, without the snack, does not get there.
    let update = lot.step_play().unwrap();
    let [error] = &update.errors[..] else {
        panic!("{:?}", update.errors);
    };
    assert_eq!(error.object, 18);
    assert_eq!(
        error.fault.at.map(|at| (at.tree, at.node)),
        Some((0x101c, 15)),
        "{error:?}"
    );
    assert!(
        matches!(error.fault.trap, sims_vm::Trap::Unimplemented { opcode: 9 }),
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
    assert_eq!(digest(&format_27_bytes(&lot)), TICK_1103_27);
    assert_eq!(digest(&format_26_bytes(&lot)), TICK_1103_26);
    assert_eq!(digest(&format_25_bytes(&lot)), TICK_1103_25);
    assert_eq!(digest(&format_24_bytes(&lot)), TICK_1103_24);
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
    // In strict mode deselecting Mortimer fails on his animator's refresh
    // (0x58da12), which is not ported, and changes nothing.
    let before = lot.world().hash();
    let refused = lot.select(other).unwrap_err().to_string();
    assert!(refused.contains("0x58da12"), "{refused}");
    assert_eq!(lot.world().hash(), before);
    // Play mode leaves the refresh out.
    lot.select_play(other).unwrap();
    let world = lot.world();
    assert_eq!(world.selected(), Some(other));
    assert!(!world.objects[&mortimer].selected);
    assert_eq!(world.globals[3], other as i16);
    assert_eq!(world.objects[&first].data[2], other as i16);
    assert_eq!(world.objects[&other].slots[2].object, first as i16);
    assert_eq!(world.objects[&mortimer].slots[2].object, 0);
    // Selecting him back gives the indicator back.
    lot.select_play(mortimer).unwrap();
    assert_eq!(lot.world().selected(), Some(mortimer));
    assert_eq!(lot.world().objects[&first].data[2], mortimer as i16);
    // A selection that fails changes nothing.
    let before = lot.world().hash();
    assert!(lot.select_play(first).is_err());
    assert_eq!(lot.world().hash(), before);
    for _ in 0..50 {
        assert!(lot.step_play().unwrap().errors.is_empty());
    }
}

/// `sims boot <install> UserData/Houses/House05.iff --ticks 2881
/// --answers 3` gives this hash.
const TICK_2881: &str = "f9866cb5c07966e41f015df5cf5d7b71e66741c3d5973d8c9e997977285ef30a";
/// The same state's hash under snapshot format 27.
const TICK_2881_27: &str = "e727213029f1e8560551ab8a4f8526994f21d862e167b3106f877ac3b70ff03b";
/// The same state's hash under snapshot format 26.
const TICK_2881_26: &str = "16c83a8e5f49c088dce71c67874ba7ccbb367c952b4fde8a4b3d7c78ad7a081f";
/// The same state's hash under snapshot format 25.
const TICK_2881_25: &str = "677bfb1d0fef771671a6b2881729604bb50d3fd2dc6ea18ca829d6cca7861aef";
/// The same state's hash under snapshot format 24.
const TICK_2881_24: &str = "cdd5b72c0ed8481f4db525817d1145f5445b6d2562c2ea4abfbfbfcfae4e26bf";

/// `sims boot <install> UserData/Houses/House05.iff --ticks 3432
/// --answers 3` gives this hash.
const TICK_3432: &str = "6bbc6f39d6dbe24d0a6c3ab2dff1ed57a57bd5369e71b9fced29984e01210699";
/// The same state's hash under snapshot format 27.
const TICK_3432_27: &str = "59dc53999c14e69ba5e83f556ff1135c75c60baa92b4701d844a5879ce983acc";
/// The same state's hash under snapshot format 26.
const TICK_3432_26: &str = "359095d7d0e8a141ac07a3662a3aec4de888d755c663754aa624e057a6596f5b";
/// The same state's hash under snapshot format 25.
const TICK_3432_25: &str = "fbb3d6685e20066970fce710ca97e397b921bad0384876d05a6bcb530a62d9a9";
/// The same state's hash under snapshot format 24.
const TICK_3432_24: &str = "c8d4fccd905caec7a03bb6d2872d83ba0ce462ffbc6c3e0bc1432034dade3f9a";

/// `sims boot <install> UserData/Houses/House05.iff --ticks 3602
/// --answers 3` gives this hash.
const TICK_3602: &str = "237e310cc1a35b9f46aac605f57ce8793f1575d9623b10de89d426219c08ec39";
/// The same state's hash under snapshot format 27.
const TICK_3602_27: &str = "d20b5848afafab4f927b77f6762af0e2152e7a90c61325a807398c77868a1741";
/// The same state's hash under snapshot format 26.
const TICK_3602_26: &str = "07ce1fc95d4c26d2b5466524759ddaa0d9cf6fbcde3c631c9b60e907c8bbe6a0";
const TICK_3603: &str = "97e03ff640aec9f42a222dfb78c3b01e5a71f85a2f83ed82af5f827740ec1121";
/// The same state's hash under snapshot format 27.
const TICK_3603_27: &str = "95156650bf0372faec254d37e4002e3c7341a67a71fbbd82254fbc28baa9b2e9";
/// The same state's hash under snapshot format 26.
const TICK_3603_26: &str = "82ff7567a2b43e03b5be77f7f547b8e8859c74657edecb66ce65514b42e144db";
const TICK_3912: &str = "91d31ecc335fff6c7cccc48b894956e2a07221cdb650cdfbb3c1fafa94f71a16";
/// The same state's hash under snapshot format 27.
const TICK_3912_27: &str = "0045276c46ed63146dac1d55f0042b964bfb169fb1f0e26e5987f6b3c760e11c";
/// The same state's hash under snapshot format 26.
const TICK_3912_26: &str = "41652f2fbb6dad9eec17b3fcb49d4546da8f2fa6b4bab1957cd831491f6cc285";
/// `sims boot <install> UserData/Houses/House05.iff --ticks 9906
/// --answers 3` gives this hash, the last before tick 9907.
const TICK_9906: &str = "1e87830c107bef2b188791d6c9482e8afdfe1da341131b4482dfaac5661c0f94";
/// The same state's hash under snapshot format 27.
const TICK_9906_27: &str = "c8240fc703c03716105cd4d2e554ef225259aac9799c8d3e91de604f25cee599";
/// The first update of the next day, when the day turns. Unverified: no
/// capture reaches it.
const TICK_31478: &str = "6bf514d7454e7fb7e7ad6bb3d88b87f7f48f77676278a23febb58f629aa44a5b";
/// A whole day, 43,200 updates, from the load. Unverified.
const TICK_43200: &str = "2dd1601267938348fef7e293e71049f902799f94742cf082a42d65fc935eb87b";

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
    }
    assert_eq!(lot.world().hash_hex(), TICK_2881);
    assert_eq!(digest(&format_27_bytes(&lot)), TICK_2881_27);
    assert_eq!(digest(&format_26_bytes(&lot)), TICK_2881_26);
    assert_eq!(digest(&format_25_bytes(&lot)), TICK_2881_25);
    assert_eq!(digest(&format_24_bytes(&lot)), TICK_2881_24);
    // Each update is two game seconds, 30 a game minute, so a day is
    // 43,200 updates.
    assert_eq!(clock(&lot), (8, 6, 46));
    for _ in 2882..=3432 {
        lot.step().unwrap();
    }
    assert_eq!(lot.world().hash_hex(), TICK_3432);
    assert_eq!(digest(&format_27_bytes(&lot)), TICK_3432_27);
    assert_eq!(digest(&format_26_bytes(&lot)), TICK_3432_26);
    assert_eq!(digest(&format_25_bytes(&lot)), TICK_3432_25);
    assert_eq!(digest(&format_24_bytes(&lot)), TICK_3432_24);
    assert_eq!(clock(&lot), (8, 25, 8));
    // Tick 3433: object 334, the lead of the car junk's group, snaps
    // itself to a place where it does not fit (0x454e82).
    lot.step().unwrap();
    // Tick 3434: its group is taken off the lot (Find Location mode 1)
    // and queued for removal, and at the end of the update all 18 tiles
    // are deleted (0x440f37).
    let junk = [
        334, 336, 340, 342, 348, 349, 350, 351, 352, 355, 356, 357, 368, 369, 370, 371, 372, 373,
    ];
    assert!(junk.iter().all(|id| lot.world().objects.contains_key(id)));
    let tick = lot.step().unwrap();
    assert_eq!(tick.removed, [334]);
    assert!(junk.iter().all(|id| !lot.world().objects.contains_key(id)));
    for _ in 3435..=3602 {
        lot.step().unwrap();
    }
    assert_eq!(lot.world().hash_hex(), TICK_3602);
    assert_eq!(digest(&format_27_bytes(&lot)), TICK_3602_27);
    assert_eq!(digest(&format_26_bytes(&lot)), TICK_3602_26);
    assert_eq!(clock(&lot), (8, 30, 48));
    // Tick 3603: object 354 (tree 0x1022) picks two neighbor records at
    // random, makes the person of each off the lot (Create New Object
    // Instance mode 6, flag 0x4) as objects 334 and 336, writes their
    // person words 80 and 81, and removes them both; they are deleted at
    // the end of the update.
    let tick = lot.step().unwrap();
    assert_eq!(tick.removed, [334, 336]);
    assert!(!lot.world().objects.contains_key(&334));
    assert!(!lot.world().objects.contains_key(&336));
    assert_eq!(lot.world().hash_hex(), TICK_3603);
    assert_eq!(digest(&format_27_bytes(&lot)), TICK_3603_27);
    assert_eq!(digest(&format_26_bytes(&lot)), TICK_3603_26);
    for _ in 3604..=3912 {
        lot.step().unwrap();
    }
    assert_eq!(lot.world().hash_hex(), TICK_3912);
    assert_eq!(digest(&format_27_bytes(&lot)), TICK_3912_27);
    assert_eq!(digest(&format_26_bytes(&lot)), TICK_3912_26);
    assert_eq!(clock(&lot), (8, 41, 8));
    // Tick 3913: object 16, seated, asks for a route, and stands up first
    // (0x4832d4). Tick 6756: a pet is made (the pet guard 0x440d63 lets it
    // be, with the expansions installed), its Main's first argument the
    // maker's temp 0, -2, as in the capture (Create flag 0x10); from 6760 it looks for a place
    // to go with no seats (0x4823d0), on its own pace (planner data word
    // 17) and turns at a pet's rate (0x58df67). Tick 6784: it stops
    // running (animator state 11). Tick 7702: an inventory count goes up
    // (Manage Inventory operation 0). Tick 9337: a route finds no door and
    // gives up with result 2.
    for _ in 3913..=9906 {
        lot.step().unwrap();
    }
    assert_eq!(lot.world().hash_hex(), TICK_9906);
    assert_eq!(digest(&format_27_bytes(&lot)), TICK_9906_27);
    assert_eq!(clock(&lot), (12, 0, 56));
    // Tick 9907: object 5 walks the neighborhood's lots in order of
    // distance (Set To Next mode 11). Tick 30320: object 243 makes person
    // 36 (Create New Object Instance), with person words the original
    // hands to 0x45810c, which the port leaves out as only what is drawn
    // (unverified).
    // The date: globals 1, 7 and 8 are the day, month and year, global 28
    // the days played.
    let date = |lot: &Lot| {
        let globals = &lot.world().globals;
        (globals[1], globals[7], globals[8], globals[28])
    };
    assert_eq!(date(&lot), (16, 6, 1997, 0));
    for _ in 9907..=31477 {
        lot.step().unwrap();
    }
    assert_eq!(clock(&lot), (23, 59, 58));
    // Tick 31478: the hours reach 24 and the day turns (0x46d3e9): the
    // date, the objects' daily update, the budget, and signal 0xdc.
    lot.step().unwrap();
    assert_eq!(lot.world().hash_hex(), TICK_31478);
    assert_eq!((date(&lot), clock(&lot)), ((17, 6, 1997, 1), (0, 0, 0)));
    // Tick 34719: a Snap blocked by a person asks it to make way
    // (0x45bd5e). Tick 35102: a balloon shows a neighbor's picture
    // (0x44b862). Tick 37426: a puddle is made on the tile in front of its
    // maker (Create mode 9) and joins the group beside it (0x423368).
    // Tick 40935: an action queued before others drops them (0x45d4c8).
    for _ in 31479..=43200 {
        lot.step().unwrap();
    }
    // A whole day from the load: the clock is back where it started.
    assert_eq!(lot.world().hash_hex(), TICK_43200);
    assert_eq!((date(&lot), clock(&lot)), ((17, 6, 1997, 1), (6, 30, 44)));
}

/// The Newbies' house (House07) after 1802 updates, with `--answers 3`.
const NEWBIE_1802: &str = "dbc6f3b365d70897ebc5d07bdeb086e9819880e83eddb954ba3b12672ac65d50";
/// The same state's hash under snapshot format 27.
const NEWBIE_1802_27: &str = "561330e47a0f5f0a1553e4382c16c87a15ff8878fff533fd375a36f24fbf177b";
/// The same state's hash under snapshot format 26.
const NEWBIE_1802_26: &str = "d7980d86529f1b84be10c103d0d522e5484fc54ee2484859beba71b74c8c1d58";
/// The Newbies' house after 4052 updates, the last whose frame strict
/// mode draws.
const NEWBIE_4052: &str = "f89e3f50e2670999480dd1d852b0ea22e125db6d060e80d10f37a62ee7b8e25f";
/// The same state's hash under snapshot format 27.
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
    assert_eq!(digest(&format_27_bytes(&lot)), NEWBIE_1802_27);
    assert_eq!(digest(&format_26_bytes(&lot)), NEWBIE_1802_26);
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
    assert_eq!(digest(&format_27_bytes(&lot)), NEWBIE_4052_27);
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
