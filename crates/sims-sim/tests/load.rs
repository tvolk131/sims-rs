use sims_core::{CrtRng, GameRng};
use sims_formats::lot::Simulator;
use sims_formats::tatt::{Entry, Tatt};
use sims_sim::load::{Context, LoadError};
use sims_sim::{Budget, TypeAttributes, World};

fn lot(globals: Vec<i16>) -> World {
    World {
        globals,
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
        ..World::empty()
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
    // The saved family stays until it is cleared, after the lot is resized.
    assert_eq!(g[9], 109);
    world.clear_family().unwrap();
    assert_eq!(world.globals[9], 0);
    assert_eq!(world.globals[10], 110);
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

fn simulator() -> Simulator {
    Simulator {
        version: 0x3e,
        globals: (0..32).collect(),
        ticks: 88444,
        scheduler_cursor: 3,
        funds: 2515,
        lot_rng: 11,
        global_rng: 22,
        pending_funds: 1,
        lot_value: 2,
        objects_value: 3,
        architecture_value: 4,
        budget_days: vec![Some([1; 8]), None],
        tail: Vec::new(),
    }
}

#[test]
fn the_simulator_state_is_read_into_the_lot_the_game_holds() {
    let mut world = World::empty();
    assert_eq!((world.size, world.globals.len()), (64, 64));
    // What the game holds that no house file does.
    world.size = 40;
    world.crt_rng = CrtRng::from_state(99);
    world.order = vec![7];
    world.removals = vec![7];
    world.event_owner = 7;
    world.read_simulator(&simulator());
    assert_eq!(world.ticks, 88444);
    assert_eq!(world.globals, (0..32).collect::<Vec<i16>>());
    assert_eq!(world.scheduler_cursor, 3);
    assert_eq!((world.lot_rng.state(), world.global_rng.state()), (11, 22));
    assert_eq!((world.budget.funds, world.budget.pending_funds), (2515, 1));
    assert_eq!(world.budget.days, [Some([1; 8]), None]);
    assert_eq!((world.size, world.crt_rng.state()), (40, 99));
    assert_eq!(
        (&world.order[..], &world.removals[..]),
        (&[7][..], &[7][..])
    );
    assert_eq!(world.event_owner, 7);
}

fn row(guid: u32, values: &[i16]) -> Entry {
    Entry {
        guid,
        values: values.to_vec(),
        count_end: 0,
        end: 0,
    }
}

fn attributes(guid: u32, values: &[i16]) -> TypeAttributes {
    TypeAttributes {
        guid,
        values: values.to_vec(),
    }
}

fn read(records: &[TypeAttributes], rows: Vec<Entry>) -> Vec<TypeAttributes> {
    let mut world = World::empty();
    world.type_attributes = records.to_vec();
    world.read_type_attributes(&Tatt {
        version: 0,
        packed: true,
        entries: rows,
        count_end: 0,
        consumed: 0,
    });
    world.type_attributes
}

#[test]
fn saved_type_attributes_fill_the_first_record_of_their_type() {
    let records = [
        attributes(1, &[9, 9, 9]),
        attributes(2, &[9]),
        attributes(1, &[9, 9]),
    ];
    // A short row zeroes the rest; a row for no record is skipped.
    let rows = vec![row(1, &[5, 6]), row(77, &[1; 128]), row(2, &[4])];
    assert_eq!(
        read(&records, rows),
        [
            attributes(1, &[5, 6, 0]),
            attributes(2, &[4]),
            attributes(1, &[9, 9]),
        ]
    );
}

#[test]
fn a_row_too_long_for_its_record_ends_the_read() {
    let records = [
        attributes(1, &[9]),
        attributes(2, &[9]),
        attributes(3, &[9]),
    ];
    let rows = vec![row(1, &[5]), row(2, &[4, 4]), row(3, &[3])];
    assert_eq!(
        read(&records, rows),
        [
            attributes(1, &[5]),
            attributes(2, &[9]),
            attributes(3, &[9])
        ]
    );
    // So does a row for no record that is longer than the scratch buffer.
    let rows = vec![row(77, &[1; 129]), row(1, &[5])];
    assert_eq!(read(&records, rows), records);
}

#[test]
fn the_scheduler_word_keeps_a_countdown_and_flags() {
    let mut object = sims_sim::Object {
        id: 1,
        guid: 1,
        footprint: [0; 4],
        position: [0; 3],
        unknown_short: 0,
        attributes: Vec::new(),
        temps: Vec::new(),
        data: Vec::new(),
        obsolete_word: 0,
        stack_capacity_bytes: 0,
        stack: Vec::new(),
        relationship_flag: 0,
        relationships: Vec::new(),
        slots: Vec::new(),
        dynamic_sprites: Vec::new(),
        multipart: None,
        portal: None,
        person: None,
        registry_word: None,
        last_result: false,
        removal_skips_resets: false,
    };
    // Without a word there is nothing to set.
    object.set_countdown(5);
    object.set_flags(1, true);
    assert_eq!((object.countdown(), object.flags()), (None, 0));
    assert_eq!(object.registry_word, None);
    object.registry_word = Some(0x0009_1fd3);
    assert_eq!((object.countdown(), object.flags()), (Some(8147), 9));
    object.set_countdown(-2);
    assert_eq!(object.registry_word, Some(0x0009_fffe));
    object.set_flags(8, false);
    object.set_flags(2, true);
    assert_eq!((object.countdown(), object.flags()), (Some(-2), 3));
    // The top flag bit makes the word negative; nothing else changes.
    object.set_flags(0x8000, true);
    assert_eq!((object.countdown(), object.flags()), (Some(-2), 0x8003));
    object.set_countdown(19999);
    assert_eq!((object.countdown(), object.flags()), (Some(19999), 0x8003));
}
