//! `sims_vm::boot` on a real install, when `SIMS_INSTALL` names one: the
//! Goth house loads and steps one tick at a time to the state `sims boot`
//! reaches.

use sims_vm::boot::{Frames, Lot, StepError};
use std::path::PathBuf;

/// `sims boot <install> UserData/Houses/House05.iff --ticks 33 --answers 3`
/// gives this hash with the default frames (snapshot format 22).
const TICK_33: &str = "041181badfd4d6953814b6c46612398df1004fa583b1d2aa5bd11c59715d65c0";

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
    // 0x58957e) and runs on, until it stops on what is not ported yet.
    let mut steps = 0;
    let stopped = loop {
        steps += 1;
        assert!(steps <= 100, "the social did not stop");
        if let Err(e) = lot.step() {
            break e.to_string();
        }
    };
    assert_eq!(steps, 46, "{stopped}");
    for part in [
        "tick 48,",
        "object 21, tree 0x2025 node 13",
        "Generic Sims Call routine 15",
    ] {
        assert!(stopped.contains(part), "{stopped}");
    }
}
