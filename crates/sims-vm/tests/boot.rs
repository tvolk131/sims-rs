//! `sims_vm::boot` on a real install, when `SIMS_INSTALL` names one: the
//! Goth house loads and steps one tick at a time to the state `sims boot`
//! reaches.

use sims_vm::boot::{Frames, Lot, StepError};
use std::path::PathBuf;

/// `sims boot <install> UserData/Houses/House05.iff --ticks 33 --answers 3`
/// gives this hash with the default frames (snapshot format 21).
const TICK_33: &str = "f04b6b12f014338ffae07e2991a99d70791ef7bd9699fe9c659714804e846c1e";

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

    // Object 18's listing runs check trees and fails partway, at a scope
    // not ported, and the lot is as it was.
    let before = lot.world().hash();
    let refused = lot.pie_menu(sim, 18).unwrap_err().to_string();
    assert!(refused.contains("scope 12"), "{refused}");
    assert_eq!(lot.world().hash(), before);

    // Person 16: listing changes the lot (its check trees' work), and
    // the change is kept.
    let before = lot.world().hash();
    let menu = lot.pie_menu(sim, 16).unwrap_or_else(|e| panic!("{e}"));
    assert_ne!(lot.world().hash(), before);

    // Every entry queues an action that runs at once (0x45d240), whose
    // tree runs a tree by name on another object (0x44ba26), which is not
    // ported, and each refusal leaves the lot as it was.
    assert!(!menu.entries.is_empty());
    assert!(
        menu.entries
            .iter()
            .all(|e| e.action.caller == sim as i16 && e.action.uid == 0)
    );
    let before = lot.world().hash();
    for entry in menu.entries {
        let refused = lot.pick(sim, entry.action).unwrap_err().to_string();
        assert!(refused.contains("0x44ba26"), "{refused}");
        assert_eq!(lot.world().hash(), before);
    }

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
}
