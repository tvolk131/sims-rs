//! Ticks: the lot update, the scheduler's pass, an object's and a
//! person's tick, the registry's pass at the end of the update, and the
//! Dialog primitive the registry answers. Synthetic; no game data.

use crate::support::*;
use sims_core::F32;
use sims_sim::motives::{Curve, Rates, Tuning};
use sims_sim::{Dialog, PersonTick};
use sims_vm::{Answers, Player, Shown, TickError, Update};

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
fn the_minute_turns_at_sixty_seconds_and_the_hour_fails_loudly() {
    let mut fx = lot(sleeper());
    fx.thread(LAMP, 4096, &[3]);
    fx.world.globals[6] = 58;
    update(&mut fx, &mut answers(&[])).unwrap();
    assert_eq!((fx.world.globals[5], fx.world.globals[6]), (1, 0));
    fx.world.globals[5] = 59;
    fx.world.globals[6] = 58;
    assert!(matches!(
        update(&mut fx, &mut answers(&[])),
        Err(TickError::Unsupported(m)) if m.contains("hour")
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

#[test]
fn what_a_person_tick_does_not_port_fails_loudly() {
    // Every tenth tick an awake member of the household asks the lot.
    let mut fx = lot(sleeper());
    fx.thread(LAMP, 4096, &[3]);
    fx.world.ticks = 10;
    let person = fx.sim_mut().person.as_mut().unwrap();
    person.data[32] = 0;
    person.motives[11] = 0.0;
    assert!(matches!(
        update(&mut fx, &mut answers(&[])),
        Err(TickError::Fault(f)) if f.to_string().contains("0x4165e3")
    ));
    // Person word 1.
    let mut fx = lot(sleeper());
    fx.thread(LAMP, 4096, &[3]);
    fx.sim_mut().person.as_mut().unwrap().data[1] = 1;
    assert!(matches!(
        update(&mut fx, &mut answers(&[])),
        Err(TickError::Fault(f)) if f.to_string().contains("0x4a69aa")
    ));
    // Entering an indoor room.
    let mut fx = lot(sleeper());
    fx.thread(LAMP, 4096, &[3]);
    fx.world
        .rooms
        .rooms
        .insert(3, sims_sim::rooms::Room::default());
    let sim = fx.sim_mut();
    sim.data[29] = 3;
    sim.person.as_mut().unwrap().data[65] = 0;
    assert!(matches!(
        update(&mut fx, &mut answers(&[])),
        Err(TickError::Fault(f)) if f.to_string().contains("0x468802")
    ));
}
