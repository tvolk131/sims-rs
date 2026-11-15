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

    // Person 16: every entry routes through a queued action that runs at
    // once (0x45d240), which is not ported, and each refusal leaves the
    // lot as it was.
    let menu = lot.pie_menu(sim, 16).unwrap_or_else(|e| panic!("{e}"));
    assert!(!menu.actions.is_empty());
    assert!(
        menu.actions
            .iter()
            .all(|a| a.caller == sim as i16 && a.uid == 0)
    );
    let before = lot.world().hash();
    for action in menu.actions {
        let refused = lot.pick(sim, action).unwrap_err().to_string();
        assert!(refused.contains("0x45d240"), "{refused}");
        assert_eq!(lot.world().hash(), before);
    }

    // Object 1's first entry is queued.
    let menu = lot.pie_menu(sim, 1).unwrap_or_else(|e| panic!("{e}"));
    let action = menu.actions[0].clone();
    let queued = lot.pick(sim, action).unwrap();
    assert!(queued.is_some());
    assert_eq!(queue(&lot).last().map(|a| a.uid), queued);
    lot.step().unwrap();
}
