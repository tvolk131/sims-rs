//! `sims_vm::boot` on a real install, when `SIMS_INSTALL` names one: the
//! Goth house loads and steps one tick at a time to the state `sims boot`
//! reaches.

use sims_vm::boot::{Frames, Lot, StepError};
use std::path::PathBuf;

/// `sims boot <install> UserData/Houses/House05.iff --ticks 33 --answers 3`
/// gives this hash with the default frames (snapshot format 23).
const TICK_33: &str = "1b29b0a0c793e9b4457b21d72c61016498811f27c4d926c73e5c52ee11d30e34";
/// The same state's hash under snapshot format 22, which had no families
/// ([`sims_sim::World::families`]) at the end.
const TICK_33_22: &str = "041181badfd4d6953814b6c46612398df1004fa583b1d2aa5bd11c59715d65c0";

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
    let start = lot.world().ticks;
    for tick in 1..=33 {
        lot.step().unwrap();
        assert_eq!(lot.ticks(), tick);
        assert_eq!(lot.world().ticks, start.wrapping_add(tick));
    }
    assert_eq!(lot.world().hash_hex(), TICK_33);
    use sha2::Digest;
    let mut bytes = lot.world().encode();
    let families = postcard::to_allocvec(&lot.world().families).unwrap();
    assert_eq!(bytes.split_off(bytes.len() - families.len()), families);
    let digest: String = sha2::Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(digest, TICK_33_22);
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

    // Object 18's listing runs check trees and fails partway: one reads
    // the advertisement record (scope 12) before Find Best Interaction
    // has first copied into it, which the load does not reach. The lot is
    // as it was.
    let before = lot.world().hash();
    let refused = lot.pie_menu(sim, 18).unwrap_err().to_string();
    assert!(
        refused.contains("scope 12") && refused.contains("0x6e5100"),
        "{refused}"
    );
    assert_eq!(lot.world().hash(), before);

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
    // 0x44fe3c), reads the Goth family's friend count (scope 24 field 9,
    // 0x425b5d) and runs on, until it stops on what is not ported yet.
    let mut steps = 0;
    let stopped = loop {
        steps += 1;
        assert!(steps <= 300, "the social did not stop");
        if let Err(e) = lot.step() {
            break e.to_string();
        }
    };
    assert_eq!(steps, 259, "{stopped}");
    for part in [
        "tick 261,",
        "object 21, tree 0x200e node 21",
        "Set Balloon/Headline showing an icon of kind 6",
    ] {
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
    // step (0x58dfc0).
    let update = lot.step_play().unwrap();
    let [error] = &update.errors[..] else {
        panic!("{:?}", update.errors);
    };
    assert_eq!((error.object, error.tick), (sim, 89279));
    assert!(
        error.fault.to_string().contains("0x58dfc0"),
        "{}",
        error.fault
    );
}
