use sims_core::GameRng;
use sims_sim::load::{Context, LoadError};
use sims_sim::{Budget, World};
use std::collections::BTreeMap;

fn lot(globals: Vec<i16>) -> World {
    World {
        ticks: 0,
        globals,
        scheduler_cursor: 0,
        lot_rng: GameRng::from_state(1),
        global_rng: GameRng::from_state(2),
        budget: Budget {
            funds: 777,
            pending_funds: 0,
            lot_value: 0,
            objects_value: 0,
            architecture_value: 0,
            days: Vec::new(),
        },
        order: Vec::new(),
        types: BTreeMap::new(),
        objects: BTreeMap::new(),
        arrays: BTreeMap::new(),
    }
}

const CONTEXT: Context = Context {
    mode: -2,
    global25: 41,
    free_will: 1,
    expansions: 0xff,
    performance_class: 2,
};

#[test]
fn the_context_overlays_the_saved_globals() {
    // 32 saved words, as SIMI before version 0x41 holds.
    let saved: Vec<i16> = (100..132).collect();
    let mut world = lot(saved);
    world.apply_context(CONTEXT).unwrap();
    let g = &world.globals;
    assert_eq!(g.len(), 64);
    // Saved words survive unless the load sets them.
    assert_eq!(g[0], 100);
    assert_eq!(g[31], 131);
    // Words past the saved ones start at zero.
    assert_eq!(&g[34..], &[0; 30]);
    assert_eq!(g[15], -2, "mode is sign-extended");
    assert_eq!(g[25], 41);
    assert_eq!(g[29], 30);
    assert_eq!(g[30], 1);
    assert_eq!(g[16], 116, "speed in range is kept");
    assert_eq!(g[18], 116, "global 18 follows the speed");
    assert_eq!(g[20], 0xff);
    assert_eq!(g[33], 2);
    assert_eq!(g[32], 0);
    assert_eq!(g[9], 0);
}

#[test]
fn speed_is_clamped_and_a_negative_global_18_is_kept() {
    let mut saved = vec![0i16; 64];
    saved[16] = 5000;
    saved[18] = -1;
    saved[32] = 1;
    saved[40] = 9;
    let mut world = lot(saved);
    world.apply_context(CONTEXT).unwrap();
    assert_eq!(world.globals[16], 1000);
    assert_eq!(world.globals[18], -1);
    assert_eq!(world.globals[32], 0);
    assert_eq!(world.globals[40], 9);

    let mut saved = vec![0i16; 64];
    saved[16] = -7;
    saved[18] = 3;
    let mut world = lot(saved);
    world.apply_context(CONTEXT).unwrap();
    assert_eq!(world.globals[16], 0);
    assert_eq!(world.globals[18], 0);
}

#[test]
fn other_global_counts_are_rejected() {
    let mut world = lot(vec![0; 31]);
    assert_eq!(
        world.apply_context(CONTEXT),
        Err(LoadError::GlobalCount(31))
    );
}

#[test]
fn selecting_the_family_sets_its_globals_and_funds() {
    // A 32-word save has no budget mode global until the context is applied.
    let mut world = lot(vec![0; 32]);
    assert_eq!(world.select_family(3, 5, 1), Err(LoadError::NoContext));
    world.apply_context(CONTEXT).unwrap();
    world.select_family(3, 5, 123_456).unwrap();
    assert_eq!(world.globals[9], 3);
    assert_eq!(world.globals[10], 5);
    assert_eq!(world.budget.funds, 123_456);
    assert_eq!((world.globals[14], world.globals[13]), (12, 3456));

    // Negative funds split toward zero.
    world.set_funds(-123_456).unwrap();
    assert_eq!((world.globals[14], world.globals[13]), (-12, -3456));

    // An empty lot has no money, whatever the family record says.
    world.select_family(0, 5, 9999).unwrap();
    assert_eq!(world.budget.funds, 0);
    assert_eq!((world.globals[14], world.globals[13]), (0, 0));
}

#[test]
fn funds_do_not_change_outside_budget_mode_zero() {
    let mut world = lot(vec![0; 64]);
    world.apply_context(CONTEXT).unwrap();
    world.globals[32] = 1;
    world.select_family(3, 5, 500).unwrap();
    assert_eq!(world.globals[9], 3);
    assert_eq!(world.budget.funds, 777);
    assert_eq!(world.globals[13], 0);
}
