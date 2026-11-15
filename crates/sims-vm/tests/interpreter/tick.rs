//! Ticks: the lot update, the scheduler's pass, an object's and a
//! person's tick, the registry's pass at the end of the update, and the
//! Dialog primitive the registry answers. Synthetic; no game data.

use crate::support::*;
use sims_core::F32;
use sims_sim::motives::{Curve, Rates, Tuning};
use sims_sim::{Dialog, HouseUsage, PersonTick, Usage};
use sims_vm::play::{Play, Recovery};
use sims_vm::{Answers, Player, Shown, SoundEvent, TickError, Update};

/// Dialog (opcode 36) operands of `kind`, waiting for an answer, with the
/// temp a three-button dialog writes in bits 4 to 6 of operand 7.
fn dialog(kind: u8, temp: u8) -> [u8; 8] {
    [0, 0, 0, 0, 0, kind, 0, temp << 4]
}

fn tuning() -> Tuning {
    let rates = || Rates::from_lookup(0, |_| None);
    let curve = |motive| Curve::parse(motive, "(-100;1)(100;1)", 8);
    Tuning {
        rates: [rates(), rates(), rates()],
        adult_mood: vec![curve(7)],
        child_mood: vec![curve(7)],
    }
}

/// A lot whose clock reads 0:00:00 and whose scheduler runs every object.
/// The lamp (flags 1, no countdown) runs `tree`. The Sim is selected and
/// sleeps; it is a visitor, so no lot service is asked of it, and its
/// motives do not decay.
fn lot(tree: sims_formats::behavior::Behavior) -> Fixture {
    let mut fx = Fixture::new();
    fx.world.globals = vec![0; 64];
    fx.world.ticks = 1;
    fx.add(LAMP_FILE, 4096, tree);
    fx.add(SIM_FILE, 4096, sleeper());
    fx.world.types.insert(8, SIM_GUID);
    fx.lamp_mut().registry_word = Some(0x0001_ffff);
    fx.lamp_mut().data[25] = 0;
    fx.thread(LAMP, 4096, &[5, 0, 0, 0]);
    let sim = fx.sim_mut();
    sim.registry_word = Some(0x0001_ffff);
    sim.selected = true;
    sim.data[25] = 0;
    sim.data[29] = -5;
    sim.data[34] = 0;
    // Its hands are empty, so its animator chooses no carry clip.
    sim.slots[1].object = 0;
    let person = sim.person.as_mut().unwrap();
    person.data[1] = 0;
    person.data[29] = 1;
    person.data[32] = 1;
    fx.thread(SIM, 4096, &[100]);
    fx
}

fn update(fx: &mut Fixture, player: &mut dyn Player) -> Result<Update, TickError> {
    let tuning = tuning();
    let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    interpreter.lot_update(&mut fx.world, &tuning, player)
}

fn answers(list: &[u8]) -> Answers {
    Answers {
        queue: list.iter().copied().collect(),
    }
}

/// A lamp that asks a dialog of `kind` and stores 1 in data word 0 on the
/// true branch and 2 on the false one, then sleeps.
fn asking(kind: u8, temp: u8) -> sims_formats::behavior::Behavior {
    tree(vec![
        ins(36, 1, 2, dialog(kind, temp)),
        assign(scope::MY_DATA, 0, scope::LITERAL, 1, 3),
        assign(scope::MY_DATA, 0, scope::LITERAL, 2, 3),
        ins(0, 3, 3, [0, 0, 0, 0, 0, 0, 0, 0]),
    ])
}

#[test]
fn a_one_button_dialog_yields_is_answered_at_the_end_of_the_update_and_then_returns_true() {
    let mut fx = lot(asking(0, 0));
    let mut player = answers(&[]);
    let first = update(&mut fx, &mut player).unwrap();
    // The lamp asked and yielded on the Dialog; the registry showed the
    // record and the only button answered it.
    assert_eq!(first.answered, vec![(LAMP, 2)]);
    assert_eq!(fx.lamp().stack[0].node, 0);
    assert_eq!(
        fx.world.dialogs.records,
        vec![Dialog {
            caller: LAMP,
            operands: dialog(0, 0),
            status: 2
        }]
    );
    // Next tick the Dialog reads the answer, takes the true branch, and
    // the record is done; the registry destroys it.
    let second = update(&mut fx, &mut player).unwrap();
    assert!(second.answered.is_empty());
    assert_eq!(fx.lamp().data[0], 1);
    assert!(fx.world.dialogs.records.is_empty());
}

#[test]
fn the_second_button_takes_the_false_branch_and_marks_the_registry() {
    let mut fx = lot(asking(1, 0));
    let mut player = answers(&[3]);
    assert_eq!(
        update(&mut fx, &mut player).unwrap().answered,
        vec![(LAMP, 3)]
    );
    update(&mut fx, &mut player).unwrap();
    assert_eq!(fx.lamp().data[0], 2);
    assert_eq!(fx.world.dialogs.declined, -1);
}

#[test]
fn a_three_button_dialog_writes_its_temp() {
    for (answer, data, temp) in [(2, 1, 9), (3, 2, 0), (4, 2, 1)] {
        let mut fx = lot(asking(2, 5));
        fx.lamp_mut().temps[5] = 9;
        let mut player = answers(&[answer]);
        update(&mut fx, &mut player).unwrap();
        update(&mut fx, &mut player).unwrap();
        assert_eq!((fx.lamp().data[0], fx.lamp().temps[5]), (data, temp));
        assert_eq!(fx.world.dialogs.declined, if answer == 3 { -1 } else { 0 });
    }
}

#[test]
fn a_dialog_that_only_informs_goes_on_at_once() {
    let mut operands = dialog(0, 0);
    operands[7] = 1;
    let mut fx = lot(tree(vec![
        ins(36, 1, 2, operands),
        assign(scope::MY_DATA, 0, scope::LITERAL, 1, 3),
        assign(scope::MY_DATA, 0, scope::LITERAL, 2, 3),
        ins(0, 3, 3, [0; 8]),
    ]));
    let tick = update(&mut fx, &mut answers(&[])).unwrap();
    assert_eq!(fx.lamp().data[0], 1);
    // Shown and answered, it is done at once, and the registry's pass,
    // which starts over after each record it acts on, destroys it.
    assert_eq!(tick.answered, vec![(LAMP, 2)]);
    assert!(fx.world.dialogs.records.is_empty());
}

#[test]
fn a_choice_without_an_answer_and_unported_dialogs_fail_loudly() {
    let mut fx = lot(asking(1, 0));
    match update(&mut fx, &mut answers(&[])) {
        Err(TickError::Unanswered { shown, given }) => {
            assert_eq!(
                shown,
                Shown {
                    tick: 2,
                    object: LAMP,
                    answers: &[2, 3]
                }
            );
            assert_eq!(given, None);
        }
        other => panic!("{other:?}"),
    }
    // An answer the dialog does not offer.
    let mut fx = lot(asking(1, 0));
    assert!(matches!(
        update(&mut fx, &mut answers(&[4])),
        Err(TickError::Unanswered { given: Some(4), .. })
    ));
    // A dialog that does not stop the game.
    let mut operands = dialog(0, 0);
    operands[7] = 0x80;
    let mut fx = lot(tree(vec![ins(36, 1, 1, operands), ins(0, 1, 1, [0; 8])]));
    assert!(matches!(
        update(&mut fx, &mut answers(&[])),
        Err(TickError::Unsupported(_))
    ));
    // A text dialog and the special panels.
    for kind in [3, 5, 15] {
        let mut fx = lot(asking(kind, 0));
        assert!(matches!(
            update(&mut fx, &mut answers(&[])),
            Err(TickError::Fault(_))
        ));
    }
}

#[test]
fn the_scheduler_counts_down_or_ticks_and_the_clock_advances_two_seconds() {
    let mut fx = lot(sleeper());
    fx.thread(LAMP, 4096, &[3]);
    let first = update(&mut fx, &mut answers(&[])).unwrap();
    assert!(first.full);
    assert_eq!(first.ticked, vec![LAMP, SIM]);
    assert_eq!(fx.lamp().countdown(), Some(2));
    // A person is never handed to the scheduler's countdown: its Sleep
    // counts itself, so it is ticked every time.
    let second = update(&mut fx, &mut answers(&[])).unwrap();
    assert_eq!((second.counted, second.ticked), (vec![LAMP], vec![SIM]));
    assert_eq!(fx.lamp().countdown(), Some(1));
    assert_eq!((fx.world.ticks, fx.world.globals[6]), (3, 4));
    assert_eq!(fx.world.scheduler_cursor, 0);
}

#[test]
fn objects_without_flag_1_are_skipped_and_flag_4_is_cleared() {
    let mut fx = lot(sleeper());
    fx.thread(LAMP, 4096, &[3]);
    fx.lamp_mut().registry_word = Some(0x0004_ffff);
    let tick = update(&mut fx, &mut answers(&[])).unwrap();
    assert_eq!(tick.ticked, vec![SIM]);
    assert_eq!(fx.lamp().registry_word, Some(0x0000_ffff));
}

#[test]
fn a_partial_pass_runs_only_flag_2_objects_and_leaves_the_clock() {
    let mut fx = lot(sleeper());
    fx.thread(LAMP, 4096, &[3]);
    fx.world.globals[17] = 1;
    fx.lamp_mut().registry_word = Some(0x0003_ffff);
    let tick = update(&mut fx, &mut answers(&[])).unwrap();
    assert!(!tick.full);
    assert_eq!(tick.ticked, vec![LAMP]);
    assert_eq!((fx.world.ticks, fx.world.globals[6]), (1, 0));
    // Globals 19 and 10 together make it partial too.
    fx.world.globals[17] = 0;
    fx.world.globals[19] = 1;
    fx.world.globals[10] = 1;
    assert!(!update(&mut fx, &mut answers(&[])).unwrap().full);
}

#[test]
fn the_minute_turns_at_sixty_seconds() {
    let mut fx = lot(sleeper());
    fx.thread(LAMP, 4096, &[3]);
    fx.world.globals[6] = 58;
    update(&mut fx, &mut answers(&[])).unwrap();
    assert_eq!((fx.world.globals[5], fx.world.globals[6]), (1, 0));
}

/// A lot at `hours`:59:58 with time of day `period` and one counted room.
fn before_the_hour(hours: i16, period: i16) -> Fixture {
    let mut fx = lot(sleeper());
    fx.thread(LAMP, 4096, &[3]);
    fx.world.globals[0] = hours;
    fx.world.globals[4] = period;
    fx.world.globals[5] = 59;
    fx.world.globals[6] = 58;
    fx.world
        .rooms
        .rooms
        .insert(1, sims_sim::rooms::Room::default());
    fx.world.rooms.pending = 0;
    fx
}

#[test]
fn the_hour_turns_and_a_new_time_of_day_marks_every_room() {
    // 6:59 is dawn (3); 7:00 is day (0).
    let mut fx = before_the_hour(6, 3);
    update(&mut fx, &mut answers(&[])).unwrap();
    let clock = |fx: &Fixture| {
        let g = &fx.world.globals;
        (g[0], g[5], g[6], g[4])
    };
    assert_eq!(clock(&fx), (7, 0, 0, 0));
    assert!(fx.world.rooms.rooms[&1].dirty);
    assert_eq!(fx.world.rooms.pending, 3);
    // 7:59 to 8:00 stays day: no room is marked.
    let mut fx = before_the_hour(7, 0);
    update(&mut fx, &mut answers(&[])).unwrap();
    assert_eq!(clock(&fx), (8, 0, 0, 0));
    assert!(!fx.world.rooms.rooms[&1].dirty);
    assert_eq!(fx.world.rooms.pending, 0);
}

#[test]
fn a_new_time_of_day_grows_the_lawn_when_global_36_is_set() {
    use sims_formats::lot::Array;
    // Grass 0 and target 100 on the whole lot, but (1, 1) is grown, so
    // its diagonal neighbour (2, 2) grows by 5 × 4.
    let lawn = |fx: &mut Fixture| {
        let size = fx.world.size as usize;
        let array = |fill: u8| Array {
            width: size,
            height: size,
            stride: 1,
            encoding: 0,
            data: vec![fill; size * size],
        };
        let mut grass = array(0);
        grass.data[size + 1] = 100;
        fx.world.arrays.insert(6, grass);
        fx.world.arrays.insert(7, array(100));
    };
    let cell = |fx: &Fixture, x: usize, y: usize| {
        let grass = &fx.world.arrays[&6];
        grass.data[x * grass.height + y]
    };
    // 17:59 to 18:00 turns day to dusk.
    let mut fx = before_the_hour(17, 0);
    lawn(&mut fx);
    fx.world.globals[36] = 1;
    update(&mut fx, &mut answers(&[])).unwrap();
    assert_eq!(cell(&fx, 2, 2), 20);
    assert_eq!(cell(&fx, 3, 3), 0);
    // Without global 36 the lawn stays.
    let mut fx = before_the_hour(17, 0);
    lawn(&mut fx);
    update(&mut fx, &mut answers(&[])).unwrap();
    assert_eq!(cell(&fx, 2, 2), 0);
    // An hour that keeps the time of day sends no signal.
    let mut fx = before_the_hour(8, 0);
    lawn(&mut fx);
    fx.world.globals[36] = 1;
    update(&mut fx, &mut answers(&[])).unwrap();
    assert_eq!(cell(&fx, 2, 2), 0);
    // A lot without the arrays fails loudly, before the time of day or
    // the rooms change.
    let mut fx = before_the_hour(17, 0);
    fx.world.globals[36] = 1;
    assert!(matches!(
        update(&mut fx, &mut answers(&[])),
        Err(TickError::Unsupported(m)) if m.contains("the lawn grows")
    ));
    assert_eq!(fx.world.globals[4], 0);
    assert!(!fx.world.rooms.rooms[&1].dirty);
}

#[test]
fn the_time_of_day_follows_the_hour() {
    for (hours, period) in [
        (1, 2),
        (5, 2),
        (6, 3),
        (7, 0),
        (17, 0),
        (18, 1),
        (19, 2),
        (23, 2),
    ] {
        // Start an hour before, at a time of day it cannot be.
        let mut fx = before_the_hour(hours - 1, 9);
        update(&mut fx, &mut answers(&[])).unwrap();
        assert_eq!(fx.world.globals[4], period, "hour {hours}");
    }
}

#[test]
fn the_day_turning_fails_loudly() {
    let mut fx = before_the_hour(23, 2);
    assert!(matches!(
        update(&mut fx, &mut answers(&[])),
        Err(TickError::Unsupported(m)) if m.contains("the day turns")
    ));
}

#[test]
fn a_thread_that_returns_fails_with_error_0x3e9() {
    let mut fx = lot(tree(vec![ins(0, RETURN_TRUE, RETURN_TRUE, [0; 8])]));
    fx.thread(LAMP, 4096, &[0]);
    match update(&mut fx, &mut answers(&[])) {
        Err(TickError::Fault(fault)) => {
            assert_eq!(fault.object, LAMP);
            assert!(matches!(
                fault.trap,
                sims_vm::Trap::Native { code: 0x3e9, .. }
            ));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_lockout_word_counts_down_after_a_tick() {
    let mut fx = lot(sleeper());
    fx.thread(LAMP, 4096, &[3]);
    fx.lamp_mut().data[25] = 2;
    update(&mut fx, &mut answers(&[])).unwrap();
    assert_eq!(fx.lamp().data[25], 1);
}

#[test]
fn queued_removals_go_after_a_full_pass_only() {
    let mut fx = lot(sleeper());
    fx.thread(LAMP, 4096, &[3]);
    // An object off the lot that holds nothing, which the removal ported
    // so far can take, and that the scheduler does not run.
    let spare = fx.add_object(3);
    spare.data[2] = 0;
    spare.slots.truncate(1);
    spare.position = [-16, -16, 1];
    fx.world.removals = vec![3, 9];
    fx.world.globals[17] = 1;
    update(&mut fx, &mut answers(&[])).unwrap();
    assert_eq!(fx.world.removals, vec![3, 9]);
    fx.world.globals[17] = 0;
    let tick = update(&mut fx, &mut answers(&[])).unwrap();
    // An ID that names no object is dropped.
    assert_eq!(tick.removed, vec![3]);
    assert!(fx.world.removals.is_empty());
    assert!(!fx.world.objects.contains_key(&3));
}

#[test]
fn a_queued_tile_goes_with_its_whole_group() {
    use crate::group_factory::{MASTER, counting, create, dynamic, group};
    let made = |dynamic_group: bool| {
        let mut fx = lot(sleeper());
        crate::rooms::build(&mut fx);
        let file = group(&mut fx);
        counting(&mut fx, file, false);
        if dynamic_group {
            dynamic(&mut fx, file);
        }
        assert_eq!(create(&mut fx, MASTER), Ok(3));
        fx.world.removals = vec![4];
        fx
    };
    // 0x4424c1: the immediate removal, which deletes the group from its
    // lead (0x440f37).
    let mut fx = made(false);
    let tick = update(&mut fx, &mut answers(&[])).unwrap();
    assert_eq!(tick.removed, vec![4]);
    for id in [3, 4, 5] {
        assert!(!fx.world.objects.contains_key(&id), "tile {id}");
    }
    assert!(fx.world.next_parts.is_empty());
    // A tile whose type has the dynamic group function leaves its group
    // first (0x4424b0, 0x42346a), which is not ported.
    let mut fx = made(true);
    let stopped = update(&mut fx, &mut answers(&[])).unwrap_err().to_string();
    assert!(stopped.contains("0x42346a"), "{stopped}");
    assert!(fx.world.objects.contains_key(&4));
}

#[test]
fn registry_byte_0x12_is_cleared_on_a_tick_count_that_is_a_multiple_of_40() {
    // 0x4424cb: after a full pass, once the update has counted its tick.
    let mut fx = lot(sleeper());
    fx.thread(LAMP, 4096, &[3]);
    fx.world.neighbors.relationships_changed = true;
    fx.world.ticks = 38;
    update(&mut fx, &mut answers(&[])).unwrap();
    assert_eq!(fx.world.ticks, 39);
    assert!(fx.world.neighbors.relationships_changed);
    // A partial pass leaves it.
    fx.world.globals[17] = 1;
    update(&mut fx, &mut answers(&[])).unwrap();
    assert_eq!(fx.world.ticks, 39);
    assert!(fx.world.neighbors.relationships_changed);
    fx.world.globals[17] = 0;
    update(&mut fx, &mut answers(&[])).unwrap();
    assert_eq!(fx.world.ticks, 40);
    assert!(!fx.world.neighbors.relationships_changed);
}

#[test]
fn no_selected_sim_fails_loudly() {
    let mut fx = lot(sleeper());
    fx.thread(LAMP, 4096, &[3]);
    fx.sim_mut().selected = false;
    assert!(matches!(
        update(&mut fx, &mut answers(&[])),
        Err(TickError::Unsupported(m)) if m.contains("0x445501")
    ));
}

#[test]
fn a_person_decays_every_sixty_ticks_and_takes_the_room_score() {
    let mut fx = lot(sleeper());
    fx.thread(LAMP, 4096, &[3]);
    fx.world.ticks = 60;
    fx.world.rooms.outdoor_score = F32::from_f32(7.0);
    fx.world.rooms.rooms.insert(
        0,
        sims_sim::rooms::Room {
            outdoors: true,
            ..Default::default()
        },
    );
    let sim = fx.sim_mut();
    sim.data[29] = 0;
    let person = sim.person.as_mut().unwrap();
    person.data[29] = 0;
    person.data[27] = 4;
    let before = person.motives;
    update(&mut fx, &mut answers(&[])).unwrap();
    let person = fx.sim().person.as_ref().unwrap();
    assert_ne!(person.motives[7], before[7], "hunger decayed");
    assert_eq!(person.motives[13], 7.0);
    assert_eq!(person.data[27], 5);
    assert_eq!(
        fx.world.person_ticks[&SIM],
        PersonTick {
            last_decay: 60,
            room: 0
        }
    );
    // The next decay waits 60 ticks.
    let hunger = person.motives[7];
    update(&mut fx, &mut answers(&[])).unwrap();
    assert_eq!(fx.sim().person.as_ref().unwrap().motives[7], hunger);
}

#[test]
fn a_person_with_no_room_yet_keeps_motive_13() {
    let mut fx = lot(sleeper());
    fx.thread(LAMP, 4096, &[3]);
    let before = fx.sim().person.as_ref().unwrap().motives[13];
    update(&mut fx, &mut answers(&[])).unwrap();
    assert_eq!(fx.sim().person.as_ref().unwrap().motives[13], before);
    assert_eq!(fx.world.person_ticks[&SIM].room, -5);
}

/// The house's usage counts after one update at tick count `ticks` of a
/// person with person word 32 `visitor`, alertness `alertness` and
/// `routes` route contexts, starting from today's counts 7 and 3.
fn usage_after(ticks: u32, visitor: i16, alertness: f32, routes: i32) -> Usage {
    let mut fx = lot(sleeper());
    fx.thread(LAMP, 4096, &[3]);
    fx.world.ticks = ticks;
    fx.world.usage.today = Usage {
        total: 7,
        routing: 3,
    };
    let person = fx.sim_mut().person.as_mut().unwrap();
    person.data[32] = visitor;
    person.motives[11] = alertness;
    person.routing_frames = routes;
    update(&mut fx, &mut answers(&[])).unwrap();
    assert_eq!(fx.world.usage.history, [Usage::default(); 5]);
    fx.world.usage.today
}

#[test]
fn every_tenth_tick_an_awake_member_of_the_household_is_counted() {
    // 0x45ce0d to 0x45ce5c, and 0x4165e3.
    let counted = Usage {
        total: 8,
        routing: 3,
    };
    let unchanged = Usage {
        total: 7,
        routing: 3,
    };
    assert_eq!(usage_after(10, 0, 0.0, 0), counted);
    // With a route context the routing count goes up too.
    assert_eq!(
        usage_after(10, 0, 0.0, 1),
        Usage {
            total: 8,
            routing: 4
        }
    );
    // Not on other ticks, nor for a visitor or a person who is asleep.
    assert_eq!(usage_after(11, 0, 0.0, 0), unchanged);
    assert_eq!(usage_after(10, 1, 0.0, 0), unchanged);
    assert_eq!(usage_after(10, 0, -1.0, 0), unchanged);
}

#[test]
fn the_usage_counts_wrap() {
    let mut usage = HouseUsage {
        today: Usage {
            total: i32::MAX,
            routing: i32::MAX,
        },
        ..HouseUsage::default()
    };
    usage.sample(true);
    assert_eq!(
        usage.today,
        Usage {
            total: i32::MIN,
            routing: i32::MIN
        }
    );
}

#[test]
fn person_word_1_goes_to_the_interface_and_is_cleared() {
    // 0x4a69aa shows the value over the person, as money from -10000 up,
    // as text below -20000; the tick then clears it (0x45ce7e).
    for value in [5, -5, -10000, i16::MAX, -20001, i16::MIN] {
        let mut fx = lot(sleeper());
        fx.thread(LAMP, 4096, &[3]);
        fx.sim_mut().person.as_mut().unwrap().data[1] = value;
        update(&mut fx, &mut answers(&[])).unwrap();
        assert_eq!(fx.sim().person.as_ref().unwrap().data[1], 0, "{value}");
    }
}

#[test]
fn what_a_person_tick_does_not_port_fails_loudly() {
    // Person word 1 from -20000 up to -10000, which plays a sound as it is
    // shown.
    for value in [-20000, -10001] {
        let mut fx = lot(sleeper());
        fx.thread(LAMP, 4096, &[3]);
        fx.sim_mut().person.as_mut().unwrap().data[1] = value;
        assert!(matches!(
            update(&mut fx, &mut answers(&[])),
            Err(TickError::Fault(f)) if f.to_string().contains("0x4a69aa")
        ));
    }
}

/// A person in an indoor room marks it as occupied (0x468802, called
/// from the person's room step). A dark room with no lights then shows
/// a changed light, so the lot marks it to be counted again
/// (0x4688bf, 0x466b55): the room is dirty and its level, 0, waits
/// (pending bit 1).
#[test]
fn a_person_marks_an_indoor_room_as_occupied() {
    let mut fx = lot(sleeper());
    fx.thread(LAMP, 4096, &[3]);
    fx.world
        .rooms
        .rooms
        .insert(3, sims_sim::rooms::Room::default());
    let sim = fx.sim_mut();
    sim.data[29] = 3;
    sim.person.as_mut().unwrap().data[65] = 0;
    assert!(!fx.world.rooms.rooms[&3].occupied);
    update(&mut fx, &mut answers(&[])).unwrap();
    let room = &fx.world.rooms.rooms[&3];
    assert!(room.occupied);
    assert!(room.dirty);
    assert_eq!(fx.world.rooms.pending, 1);
}

fn play_update(fx: &mut Fixture, play: &mut Play) -> Update {
    let tuning = tuning();
    let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    interpreter
        .lot_update_with(&mut fx.world, &tuning, &mut answers(&[]), Some(play))
        .expect("play mode carries on past an object's error")
}

/// Play mode catches the error a thread that returns raises (0x3e9),
/// resets the object and stops the pass with the cursor on it, as the
/// original's scheduler does after its error handler (0x439a6f). An
/// object that faults again on its first tick after the reset is parked.
/// A lot whose lamp's Main returns, and whose lamp's Reset (function 3)
/// stores 7 in data word 0.
fn returning_lamp() -> Fixture {
    let mut fx = lot(tree(vec![ins(0, RETURN_TRUE, RETURN_TRUE, [0; 8])]));
    // The lamp's Main is the tree that returns, and its Reset (function
    // 3) stores 7 in data word 0.
    fx.add(
        LAMP_FILE,
        4097,
        tree(vec![assign(
            scope::MY_DATA,
            0,
            scope::LITERAL,
            7,
            RETURN_TRUE,
        )]),
    );
    let callbacks = [4096, 4096, 0, 4097].map(|action| sims_formats::execution::ObjectFunction {
        condition: 0,
        action,
    });
    fx.program
        .libraries
        .get_mut(&LAMP_FILE)
        .unwrap()
        .callbacks
        .insert(128, callbacks.to_vec());
    fx.thread(LAMP, 4096, &[0]);
    fx
}

#[test]
fn play_mode_resets_an_object_after_its_error_and_parks_one_that_repeats_it() {
    let mut fx = returning_lamp();
    let mut play = Play::default();
    let first = play_update(&mut fx, &mut play);
    assert!(!first.full);
    assert_eq!(first.ticked, vec![LAMP]);
    let [error] = first.errors.as_slice() else {
        panic!("{:?}", first.errors);
    };
    assert_eq!((error.object, error.in_thread, error.tick), (LAMP, true, 1));
    assert!(matches!(
        error.fault.trap,
        sims_vm::Trap::Native { code: 0x3e9, .. }
    ));
    // The original's handling: the Reset tree ran, and the thread
    // restarted at Main.
    assert_eq!(error.recovery, Recovery::Reset);
    assert_eq!(fx.lamp().data[0], 7);
    assert_eq!(fx.lamp().stack.len(), 1);
    assert_eq!(fx.lamp().stack[0].tree, 4096);
    // Flag 4 (the original's virtual function 4) is set, the clock waits
    // and the cursor stays on the lamp.
    assert_eq!(fx.lamp().flags() & 4, 4);
    assert_eq!((fx.world.ticks, fx.world.globals[6]), (1, 0));
    assert_eq!(fx.world.scheduler_cursor, LAMP as i16);

    // The next pass starts at the lamp, clears flag 4 and ticks it; it
    // returns again, so it is parked.
    let second = play_update(&mut fx, &mut play);
    assert_eq!(second.ticked, vec![LAMP]);
    assert!(matches!(
        second.errors.as_slice(),
        [e] if e.recovery == Recovery::Parked(None)
    ));
    assert_eq!(fx.lamp().flags() & 1, 0);

    // A parked object is skipped, and the lot goes on.
    let third = play_update(&mut fx, &mut play);
    assert!(third.full && third.errors.is_empty());
    assert_eq!(third.ticked, vec![SIM]);
    assert_eq!(fx.world.ticks, 2);
}

/// A fault in the rest of a person's tick, outside its thread, is caught
/// too (an approximation: the original has no such error).
#[test]
fn play_mode_catches_a_fault_in_the_rest_of_a_persons_tick() {
    let mut fx = lot(sleeper());
    fx.thread(LAMP, 4096, &[3]);
    fx.sim_mut().person.as_mut().unwrap().data[1] = -15000;
    let mut play = Play::default();
    let update = play_update(&mut fx, &mut play);
    assert!(!update.full);
    let [error] = update.errors.as_slice() else {
        panic!("{:?}", update.errors);
    };
    assert_eq!((error.object, error.in_thread), (SIM, false));
    assert!(error.fault.to_string().contains("0x4a69aa"));
}

/// An object deleted after play mode reset it is no longer the fresh one,
/// so a later object given its ID is not parked for its first error.
#[test]
fn play_mode_forgets_a_reset_object_once_it_is_deleted() {
    let mut fx = returning_lamp();
    let mut play = Play::default();
    let first = play_update(&mut fx, &mut play);
    assert_eq!(first.errors[0].recovery, Recovery::Reset);
    // The lamp now waits on a countdown, so it is not ticked again, and
    // is queued for removal, which the removal ported so far can take
    // once it holds nothing and is off the lot.
    let lamp = fx.lamp_mut();
    lamp.set_countdown(5);
    lamp.data[2] = 0;
    lamp.slots.truncate(1);
    lamp.position = [-16, -16, 1];
    fx.world.removals = vec![LAMP];
    let second = play_update(&mut fx, &mut play);
    assert!(second.full && second.errors.is_empty());
    assert_eq!(second.counted, vec![LAMP]);
    assert_eq!(second.removed, vec![LAMP]);
    assert_eq!(play, Play::default());
    // A new object with the lamp's ID that returns is reset, not parked.
    let mut fx2 = returning_lamp();
    fx2.world.ticks = fx.world.ticks;
    let third = play_update(&mut fx2, &mut play);
    assert_eq!(third.errors[0].recovery, Recovery::Reset);
}

/// A fault in the rest of a person's tick after one caught in its thread
/// is noted with it, and the person is dealt with once.
#[test]
fn play_mode_keeps_a_thread_fault_that_a_later_one_follows() {
    let mut fx = lot(sleeper());
    fx.thread(LAMP, 4096, &[3]);
    fx.add(
        SIM_FILE,
        4097,
        tree(vec![ins(0, RETURN_TRUE, RETURN_TRUE, [0; 8])]),
    );
    fx.thread(SIM, 4097, &[0]);
    fx.sim_mut().person.as_mut().unwrap().data[1] = -15000;
    let mut play = Play::default();
    let update = play_update(&mut fx, &mut play);
    let [error] = update.errors.as_slice() else {
        panic!("{:?}", update.errors);
    };
    assert_eq!((error.object, error.in_thread), (SIM, true));
    assert!(matches!(
        error.fault.trap,
        sims_vm::Trap::Native { code: 0x3e9, .. }
    ));
    let then = error.then.as_ref().expect("the later fault is kept");
    assert!(then.to_string().contains("0x4a69aa"), "{then}");
}

/// Without an error, play mode's update is strict mode's, and so is the
/// lot it leaves.
#[test]
fn play_mode_without_an_error_is_strict_mode() {
    let mut strict = lot(sleeper());
    strict.thread(LAMP, 4096, &[3]);
    let mut played = lot(sleeper());
    played.thread(LAMP, 4096, &[3]);
    let mut play = Play::default();
    for _ in 0..3 {
        let a = update(&mut strict, &mut answers(&[])).unwrap();
        let b = play_update(&mut played, &mut play);
        assert_eq!(a, b);
    }
    assert_eq!(strict.world.hash_hex(), played.world.hash_hex());
}

#[test]
fn an_update_returns_its_sound_events_in_order_without_changing_the_lot() {
    // The lamp plays sound 5, stops its sounds, then sleeps.
    let lamp = tree(vec![
        ins(23, 1, 1, [5, 0, 0, 0, 0, 0, 0, 0]),
        ins(48, 2, 2, [0; 8]),
        ins(0, 2, 2, [0; 8]),
    ]);
    let mut fx = lot(lamp.clone());
    fx.program
        .libraries
        .get_mut(&LAMP_FILE)
        .unwrap()
        .sounds
        .insert(5, "lamp_on".into());
    let mut quiet = lot(lamp.clone());
    let mut player = answers(&[]);
    let first = update(&mut fx, &mut player).unwrap();
    assert_eq!(
        first.sounds,
        [
            SoundEvent::Play {
                object: LAMP as i16,
                name: "lamp_on".into()
            },
            SoundEvent::Stop {
                object: LAMP as i16
            },
        ]
    );
    // A lot whose sound plays nothing ends the same.
    let second = update(&mut quiet, &mut player).unwrap();
    assert_eq!(
        second.sounds,
        [SoundEvent::Stop {
            object: LAMP as i16
        }]
    );
    assert_eq!(fx.world.hash(), quiet.world.hash());
    // An interpreter with its own sink gets them instead.
    let mut own = lot(lamp);
    let sink = std::cell::RefCell::new(Vec::new());
    let tuning = tuning();
    let mut interpreter = sims_vm::Interpreter::new(&own.program, &own.catalog, own.stream);
    interpreter.sounds = Some(&sink);
    let third = interpreter
        .lot_update(&mut own.world, &tuning, &mut player)
        .unwrap();
    assert!(third.sounds.is_empty());
    assert_eq!(
        sink.into_inner(),
        [SoundEvent::Stop {
            object: LAMP as i16
        }]
    );
}

#[test]
fn a_person_queued_for_removal_after_a_partial_pass_fails_loudly() {
    // The queue waits for a full pass, and the person's animator is not
    // refreshed meanwhile (0x58da12), which depends on whether the view
    // drew it.
    let mut fx = lot(sleeper());
    fx.thread(LAMP, 4096, &[3]);
    fx.world.globals[17] = 1;
    fx.lamp_mut().registry_word = Some(0x0003_ffff);
    fx.world.removals.push(SIM);
    let error = update(&mut fx, &mut answers(&[])).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("stays queued for removal after a partial pass"),
        "{error}"
    );
}
