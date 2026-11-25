//! Ticks: the lot update, the scheduler's pass, an object's and a
//! person's tick, the registry's pass at the end of the update, and the
//! Dialog primitive the registry answers. Synthetic; no game data.

use crate::support::*;
use sims_core::F32;
use sims_sim::motives::{Curve, Rates, Tuning};
use sims_sim::{Dialog, HouseUsage, PersonTick, Usage};
use sims_vm::play::{Play, Recovery};
use sims_vm::{
    Answer, Answers, CALL_NEIGHBOR_CANCELLED, Interpreter, Player, ScriptedPlayer, Shown,
    ShownSpecialPanel, SoundEvent, SpecialPanelRequest, SpecialPanelResponse, SpecialPanels,
    TickError, Trap, Update,
};
use std::cell::RefCell;
use std::collections::BTreeMap;

/// Dialog (opcode 36) operands of `kind`, waiting for an answer, with the
/// temp a three-button dialog writes in bits 4 to 6 of operand 7.
fn dialog(kind: u8, temp: u8) -> [u8; 8] {
    [0, 0, 0, 0, 0, kind, 0, temp << 4]
}

pub fn tuning() -> Tuning {
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
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    interpreter.lot_update(&mut fx.world, &tuning, player)
}

fn answers(list: &[u8]) -> Answers {
    Answers::choices(list)
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
    fx.world.dialogs.declined = 5;
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
fn a_dialog_click_publishes_its_bitmap_until_the_next_poll() {
    for (bitmap_id, expected) in [(Some(17), 17), (None, 42), (Some(0), 0)] {
        let mut fx = lot(asking(1, 0));
        fx.world.globals[12] = 42;
        let mut player = Answers::replay(&[Answer {
            status: 2,
            bitmap_id,
            text: None,
        }]);
        assert_eq!(update(&mut fx, &mut player).unwrap().answered, [(LAMP, 2)]);
        assert_eq!(fx.world.globals[12], expected);
        let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
        interpreter.before_frame(&mut fx.world).unwrap();
        assert_eq!(fx.world.globals[12], 0);
    }
}

#[test]
fn a_text_entry_dialog_captures_entered_text_and_takes_true_branch() {
    // `Answers::choices` must wait for an explicit player answer on kind 3
    // rather than auto-accepting it as a 1-button notice.
    let mut waiting = lot(asking(3, 0));
    match update(&mut waiting, &mut answers(&[])) {
        Err(TickError::Unanswered { shown, given }) => {
            assert_eq!(
                shown,
                Shown {
                    tick: 2,
                    object: LAMP,
                    answers: &[2],
                    text_entry: true,
                }
            );
            assert_eq!(given, None);
        }
        other => panic!("expected kind 3 to wait for text input, got {other:?}"),
    }

    let mut fx = lot(asking(3, 0));
    let mut player = Answers::replay(&[Answer {
        status: 2,
        bitmap_id: Some(17),
        text: Some("Bella".into()),
    }]);
    let first = update(&mut fx, &mut player).unwrap();
    assert_eq!(first.answered, [(LAMP, 2)]);
    assert_eq!(first.dialog_text.as_deref(), Some("Bella"));
    assert_eq!(fx.world.globals[12], 17);
    update(&mut fx, &mut player).unwrap();
    assert_eq!(fx.lamp().data[0], 1);
}

#[test]
fn a_three_button_dialog_writes_its_temp() {
    for (answer, data, temp) in [(2, 1, 9), (3, 2, 0), (4, 2, 1)] {
        let mut fx = lot(asking(2, 5));
        fx.world.dialogs.declined = 5;
        fx.lamp_mut().temps[5] = 9;
        let mut player = answers(&[answer]);
        update(&mut fx, &mut player).unwrap();
        update(&mut fx, &mut player).unwrap();
        assert_eq!((fx.lamp().data[0], fx.lamp().temps[5]), (data, temp));
        assert_eq!(fx.world.dialogs.declined, if answer == 3 { -1 } else { 5 });
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
                    answers: &[2, 3],
                    text_entry: false,
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
    // Unknown Dialog kinds (> 15) fail loudly; stateful special panels also
    // fail loudly when `Interpreter::special_panels` is not attached.
    for kind in [5, 16, 127] {
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

/// A lot at 23:59:58 on 6 March 2000, the tenth day played, with six
/// budget days, usage counts, empty level arrays on a lot 10 tiles wide,
/// and a lamp worth its price of 100 that loses 10 the first day and 3
/// after, down to 50. The lamp was made on 9 May 1999.
fn before_midnight() -> Fixture {
    use sims_formats::lot::Array;
    let mut fx = before_the_hour(23, 2);
    let g = &mut fx.world.globals;
    (g[1], g[7], g[8], g[28], g[29]) = (6, 3, 2000, 10, 30);
    fx.world.size = 10;
    fx.world.budget.days = (1..=6).map(|n| Some([n; 8])).collect();
    fx.world.usage = HouseUsage {
        history: std::array::from_fn(|n| Usage {
            total: n as i32 + 1,
            routing: n as i32 + 1,
        }),
        today: Usage {
            total: 9,
            routing: 8,
        },
    };
    let array = |stride: usize| Array {
        width: 64,
        height: 64,
        stride,
        encoding: 0,
        data: vec![0; 64 * 64 * stride],
    };
    for (id, stride) in [(1, 2), (101, 2), (2, 14), (102, 14), (8, 1), (108, 1)] {
        fx.world.arrays.insert(id, array(stride));
    }
    let words = fx.words(LAMP_FILE);
    (words[18], words[35], words[36], words[37], words[38]) = (100, 10, 3, 0, 50);
    let lamp = fx.lamp_mut();
    lamp.data[41] = 100;
    (lamp.data[45], lamp.data[46], lamp.data[47]) = (1999, 5, 9);
    lamp.data[59] = 0;
    fx.sim_mut().data[59] = 0;
    fx
}

/// Puts a floor on tile (3, 3) of level 1.
fn floor(fx: &mut Fixture) {
    let at = (3 * 64 + 3) * 2;
    fx.world.arrays.get_mut(&1).unwrap().data[at] = 7;
}

#[test]
fn the_day_turns_the_date_and_ages_and_depreciates_the_objects() {
    let mut fx = before_midnight();
    floor(&mut fx);
    let sim_value = fx.sim().data[41];
    update(&mut fx, &mut answers(&[])).unwrap();
    let g = &fx.world.globals;
    // 0x46d3e9: the hours go back by 24, the day and the days played go
    // up; the month stays.
    assert_eq!((g[0], g[1], g[7], g[8], g[28]), (0, 7, 3, 2000, 11));
    // Midnight is night, as 23:00 was: no new time of day.
    assert_eq!(g[4], 2);
    // 0x4322e7: from 9 May 1999 to 7 March 2000 is no whole year and 9
    // whole months (the 7th is before the 9th).
    let lamp = fx.lamp();
    assert_eq!((lamp.data[18], lamp.data[48]), (0, 9));
    // 0x432243: still at its price, the lamp loses its first day's 10.
    assert_eq!(lamp.data[41], 90);
    // The Sim's definition word 37 is set: it keeps its value.
    assert_eq!(fx.sim().data[41], sim_value);
    // 0x46d417: the four oldest days all take yesterday's (the forward
    // copy), yesterday takes today's, and today is cleared.
    let days: Vec<_> = fx.world.budget.days.iter().map(|d| d.unwrap()[0]).collect();
    assert_eq!(days, [0, 1, 2, 2, 2, 2]);
    // Signal 0xdc, 0x416453: the same for the usage counts.
    let usage = &fx.world.usage;
    assert_eq!((usage.today.total, usage.today.routing), (0, 0));
    assert_eq!((usage.history[0].total, usage.history[0].routing), (9, 8));
    assert!(
        usage.history[1..]
            .iter()
            .all(|u| u.total == 1 && u.routing == 1)
    );
    // 0x416085: the floor counts for global 22; global 23 is the lot's
    // size, and global 37 has no feature.
    let g = &fx.world.globals;
    assert_eq!((g[22], g[23], g[37]), (1, 10, 0));
}

#[test]
fn the_lamp_loses_its_daily_value_down_to_its_limit() {
    let mut fx = before_midnight();
    floor(&mut fx);
    // Below its price it loses word 36's 3 a day.
    fx.lamp_mut().data[41] = 60;
    update(&mut fx, &mut answers(&[])).unwrap();
    assert_eq!(fx.lamp().data[41], 57);
    // But not below word 38's 50.
    let mut fx = before_midnight();
    floor(&mut fx);
    fx.lamp_mut().data[41] = 52;
    update(&mut fx, &mut answers(&[])).unwrap();
    assert_eq!(fx.lamp().data[41], 50);
    // At its price with no first day's loss (word 35 at 0), the daily one.
    let mut fx = before_midnight();
    floor(&mut fx);
    fx.words(LAMP_FILE)[35] = 0;
    update(&mut fx, &mut answers(&[])).unwrap();
    assert_eq!(fx.lamp().data[41], 97);
    // Word 37 set: it keeps its value.
    let mut fx = before_midnight();
    floor(&mut fx);
    fx.words(LAMP_FILE)[37] = 1;
    update(&mut fx, &mut answers(&[])).unwrap();
    assert_eq!(fx.lamp().data[41], 100);
}

#[test]
fn the_month_turns_after_its_thirtieth_day_and_the_year_after_twelve() {
    let mut fx = before_midnight();
    floor(&mut fx);
    (fx.world.globals[1], fx.world.globals[29]) = (30, 0);
    update(&mut fx, &mut answers(&[])).unwrap();
    let g = &fx.world.globals;
    // 0x473544: every month has 30 days; global 29 takes the new one's.
    assert_eq!((g[1], g[7], g[8], g[29]), (1, 4, 2000, 30));
    let mut fx = before_midnight();
    floor(&mut fx);
    (fx.world.globals[1], fx.world.globals[7]) = (30, 12);
    update(&mut fx, &mut answers(&[])).unwrap();
    let g = &fx.world.globals;
    assert_eq!((g[1], g[7], g[8]), (1, 1, 2001));
    // The 29th turns to the 30th and global 29 is not written.
    let mut fx = before_midnight();
    floor(&mut fx);
    (fx.world.globals[1], fx.world.globals[29]) = (29, 0);
    update(&mut fx, &mut answers(&[])).unwrap();
    let g = &fx.world.globals;
    assert_eq!((g[1], g[7], g[29]), (30, 3, 0));
}

#[test]
fn global_22_counts_floors_and_objects_off_tiles_with_flag_0x20() {
    let tile = (5 * 64 + 5) as usize;
    // No floor, no wall and no object on the lot: 0.
    let mut fx = before_midnight();
    fx.world.globals[22] = 7;
    update(&mut fx, &mut answers(&[])).unwrap();
    assert_eq!(fx.world.globals[22], 0);
    // A floor on a tile with flag 0x20 does not count.
    let mut fx = before_midnight();
    floor(&mut fx);
    fx.world.arrays.get_mut(&8).unwrap().data[3 * 64 + 3] = 0x20;
    update(&mut fx, &mut answers(&[])).unwrap();
    assert_eq!(fx.world.globals[22], 0);
    // The Sim on tile (5, 5) of the lot counts: its definition's words 39,
    // 40 and 69 are set and its type is not 7.
    let mut fx = before_midnight();
    fx.sim_mut().position = [5 * 16 + 8, 5 * 16 + 8, 1];
    update(&mut fx, &mut answers(&[])).unwrap();
    assert_eq!(fx.world.globals[22], 1);
    // Not on a tile with flag 0x20.
    let mut fx = before_midnight();
    fx.sim_mut().position = [5 * 16 + 8, 5 * 16 + 8, 1];
    fx.world.arrays.get_mut(&8).unwrap().data[tile] = 0x20;
    update(&mut fx, &mut answers(&[])).unwrap();
    assert_eq!(fx.world.globals[22], 0);
    // The lamp is of type 7: passed over wherever it is.
    let mut fx = before_midnight();
    fx.words(LAMP_FILE)[9] = 7;
    fx.lamp_mut().position = [5 * 16 + 8, 5 * 16 + 8, 1];
    update(&mut fx, &mut answers(&[])).unwrap();
    assert_eq!(fx.world.globals[22], 0);
    // A pool byte up to 0x10 on the ground level counts.
    use sims_formats::lot::Array;
    let mut fx = before_midnight();
    let mut pool = vec![0xffu8; 64 * 64];
    pool[tile] = 0x10;
    fx.world.arrays.insert(
        9,
        Array {
            width: 64,
            height: 64,
            stride: 1,
            encoding: 0,
            data: pool,
        },
    );
    update(&mut fx, &mut answers(&[])).unwrap();
    assert_eq!(fx.world.globals[22], 1);
}

#[test]
fn the_objects_value_is_worked_out_again_after_the_days_loss() {
    // 0x444cb9, 0x416118: the objects that stand on the lot and are valued
    // (0x4354d5) sum their values (0x43219a) into simulator +0xa0, after
    // the loss of value. The architecture's value (+0xa4) is not ported
    // and keeps what it held.
    let at = |a: i32| [a * 16 + 8, 5 * 16 + 8, 1];
    let run = |change: &dyn Fn(&mut Fixture)| {
        let mut fx = before_midnight();
        floor(&mut fx);
        fx.lamp_mut().position = at(5);
        fx.sim_mut().position = [-16, -16, 1];
        // The lamp's catalog master (itself) has word 69 clear, and
        // neither has flag 0x10 nor counts half.
        fx.words(LAMP_FILE)[69] = 0;
        for id in [LAMP, SIM] {
            let object = fx.world.objects.get_mut(&id).unwrap();
            (object.data[15], object.data[43]) = (0, 0);
        }
        let budget = &mut fx.world.budget;
        (budget.objects_value, budget.architecture_value) = (1234, 5678);
        change(&mut fx);
        update(&mut fx, &mut answers(&[])).unwrap();
        let budget = &fx.world.budget;
        (budget.objects_value, budget.architecture_value)
    };
    // The lamp, at its price of 100, lost its first day's 10.
    assert_eq!(run(&|_| {}), (90, 5678));
    // The Sim on the lot adds its 82: a person (definition word 9 is 2) is
    // valued whatever its master's word 69.
    assert_eq!(run(&|fx| fx.sim_mut().position = at(6)), (172, 5678));
    // Off the lot, the lamp counts for neither.
    assert_eq!(run(&|fx| fx.lamp_mut().position = [-16, -16, 1]), (0, 5678));
    // Flag 0x10 of data word 43 keeps it out; flag 8 alone does not.
    assert_eq!(run(&|fx| fx.lamp_mut().data[43] = 0x10), (0, 5678));
    assert_eq!(run(&|fx| fx.lamp_mut().data[43] = 8), (90, 5678));
    // Its master's word 69 not 0 keeps it out (it goes to the
    // architecture's value, which is not ported).
    assert_eq!(run(&|fx| fx.words(LAMP_FILE)[69] = 4), (0, 5678));
    // Data word 15 not 0 halves the value, toward zero.
    assert_eq!(run(&|fx| fx.lamp_mut().data[15] = 1), (45, 5678));
    let negative = |fx: &mut Fixture| {
        fx.words(LAMP_FILE)[37] = 1;
        (fx.lamp_mut().data[41], fx.lamp_mut().data[15]) = (-7, -1);
    };
    assert_eq!(run(&negative), (-3, 5678));
    // A tile of a group whose definition says so (word 10; word 11 -1 makes
    // it its own catalog master) but that is no multi-tile object counts
    // 0.
    let unlinked = |fx: &mut Fixture| {
        let words = fx.words(LAMP_FILE);
        (words[10], words[11]) = (1, -1);
    };
    assert_eq!(run(&unlinked), (0, 5678));
    // A group: its lead, the lamp, counts its own value, halved because a
    // later tile of its chain has word 15 set. The tile, not a lead, is
    // passed over.
    let group = |fx: &mut Fixture| {
        use sims_formats::saved_objects::Multipart;
        let words = fx.words(LAMP_FILE);
        (words[10], words[11]) = (1, -1);
        let tile = |parent| {
            Some(Multipart {
                offset: [0; 3],
                group: [0; 3],
                parent,
            })
        };
        fx.lamp_mut().multipart = tile(0);
        let other = fx.add_object(50);
        other.position = at(6);
        other.multipart = tile(LAMP as i16);
        (other.data[15], other.data[41], other.data[43]) = (1, 1000, 0);
        fx.world.next_parts.insert(LAMP, 50);
    };
    assert_eq!(run(&group), (45, 5678));
}

#[test]
fn what_the_day_turn_does_not_port_fails_loudly() {
    let unsupported = |fx: &mut Fixture, what: &str| {
        let result = update(fx, &mut answers(&[]));
        assert!(
            matches!(&result, Err(TickError::Fault(f)) if matches!(&f.trap, Trap::Unsupported(m) if m.contains(what))),
            "{result:?}"
        );
    };
    // Walls with no floor count the lot as built (0x4795c5, 0x47b08c).
    let mut fx = before_midnight();
    fx.world.arrays.get_mut(&2).unwrap().data[(4 * 64 + 4) * 14] = 1;
    update(&mut fx, &mut answers(&[])).unwrap();
    assert_eq!(fx.world.globals[22], 1);
    // Budget days not all kept.
    let mut fx = before_midnight();
    fx.world.budget.days.pop();
    unsupported(&mut fx, "budget days");
    let mut fx = before_midnight();
    fx.world.budget.days[1] = None;
    unsupported(&mut fx, "not saved");
    // A tile of a group with no catalog master on the lot: the original
    // reads its master through a null pointer (0x4354f9).
    let mut fx = before_midnight();
    floor(&mut fx);
    fx.words(LAMP_FILE)[10] = 1;
    fx.lamp_mut().position = [5 * 16 + 8, 5 * 16 + 8, 1];
    fx.lamp_mut().data[43] = 0;
    unsupported(&mut fx, "no catalog master");
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
    use crate::group_factory::{MASTER, counting, create, group};
    let mut fx = lot(sleeper());
    crate::rooms::build(&mut fx);
    let file = group(&mut fx);
    counting(&mut fx, file, false);
    assert_eq!(create(&mut fx, MASTER), Ok(3));
    fx.world.removals = vec![4];
    // 0x4424c1: the immediate removal, which deletes the group from its
    // lead (0x440f37).
    let tick = update(&mut fx, &mut answers(&[])).unwrap();
    assert_eq!(tick.removed, vec![4]);
    for id in [3, 4, 5] {
        assert!(!fx.world.objects.contains_key(&id), "tile {id}");
    }
    assert!(fx.world.next_parts.is_empty());
}

#[test]
fn a_queued_tile_of_a_dynamic_group_leaves_its_group_and_goes_alone() {
    use crate::group_factory::{MASTER, counting, create, dynamic, group};
    let made = |queued: u16| {
        let mut fx = lot(sleeper());
        crate::rooms::build(&mut fx);
        let file = group(&mut fx);
        counting(&mut fx, file, false);
        dynamic(&mut fx, file);
        assert_eq!(create(&mut fx, MASTER), Ok(3));
        fx.world.removals = vec![queued];
        fx
    };
    let lead = |fx: &Fixture, id: u16| fx.world.objects[&id].multipart.as_ref().unwrap().parent;
    // 0x442450 to 0x4424b0: the tile's type (tile 4's) has the dynamic
    // group function, so it leaves its group first (0x42346a, 0x4228d6),
    // and the deletion then takes it alone. The lead keeps tile 5.
    let mut fx = made(4);
    let tick = update(&mut fx, &mut answers(&[])).unwrap();
    assert_eq!(tick.removed, vec![4]);
    assert!(!fx.world.objects.contains_key(&4));
    assert_eq!(fx.world.next_parts, BTreeMap::from([(3, 5)]));
    assert_eq!((lead(&fx, 3), lead(&fx, 5)), (0, 3));
    // The lead's type has no function 8: the deletion goes from the lead
    // and takes the whole group (0x440f37).
    let mut fx = made(3);
    let tick = update(&mut fx, &mut answers(&[])).unwrap();
    assert_eq!(tick.removed, vec![3]);
    for id in [3, 4, 5] {
        assert!(!fx.world.objects.contains_key(&id), "tile {id}");
    }
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
fn no_selected_sim_auto_selects_household_member_or_stays_none() {
    let mut fx = lot(sleeper());
    fx.thread(LAMP, 4096, &[3]);
    fx.sim_mut().selected = false;
    // With no family members registered, 0x445501 leaves nobody selected.
    update(&mut fx, &mut answers(&[])).unwrap();
    assert_eq!(sims_vm::selected(&fx.world), None);

    // With the Sim's GUID in the lot's family, 0x445501 selects it cleanly.
    let sim_guid = fx.sim().guid;
    fx.world.globals[9] = 5;
    fx.world.families.push(sims_sim::neighbors::Family {
        id: 5,
        house: 5,
        members: vec![sim_guid],
        friends: 0,
        stale: false,
    });
    update(&mut fx, &mut answers(&[])).unwrap();
    assert_eq!(sims_vm::selected(&fx.world), Some(SIM));
    assert_eq!(fx.world.globals[3], SIM as i16);
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
/// person with person word 32 `visitor`, alertness `alertness`, `routes`
/// route contexts and `frames` saved routing frames, starting from today's
/// counts 7 and 3.
fn usage_after(ticks: u32, visitor: i16, alertness: f32, routes: i32, frames: i32) -> Usage {
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
    person.routing_frames = frames;
    let context = sims_sim::route::Context {
        candidates: Vec::new(),
        slot: sims_sim::route::RouteSlot {
            offset: [0; 3],
            number: -1,
            standing: 0,
            sitting: 0,
            ground: 0,
            flags: 0,
            snap_target: -1,
            proximity: [16; 3],
            gradient: 0x3e40_0000,
            facing: -2,
            resolution: 16,
        },
        target: 0,
        person: 0,
        chosen: -1,
        best: -1,
        word_58: 0,
        time: 0,
        saved: [0; 2],
        word_68: 0,
        start: [0; 3],
        byte_78: true,
        objects: [0; 2],
        byte_84: false,
        result: 0,
        word_8c: 0,
        byte_8e: false,
        word_94: 0,
        limit: 0x30,
        action: 0,
    };
    if routes > 0 {
        fx.world.routing.routes.entry(SIM).or_default().contexts = vec![context; routes as usize];
    }
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
    assert_eq!(usage_after(10, 0, 0.0, 0, 0), counted);
    // With a route context the routing count goes up too.
    assert_eq!(
        usage_after(10, 0, 0.0, 1, 0),
        Usage {
            total: 8,
            routing: 4
        }
    );
    // The count reads the contexts the person has now (0x45ce45 calls
    // 0x45c349, which tests the vector, 0x45c349 to 0x45c363), not how
    // many the save said it had.
    assert_eq!(usage_after(10, 0, 0.0, 0, 2), counted);
    // Not on other ticks, nor for a visitor or a person who is asleep.
    assert_eq!(usage_after(11, 0, 0.0, 0, 0), unchanged);
    assert_eq!(usage_after(10, 1, 0.0, 0, 0), unchanged);
    assert_eq!(usage_after(10, 0, -1.0, 0, 0), unchanged);
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

/// An update that fails after play mode caught an object's error still
/// hands back the error caught ([`sims_vm::Interpreter::lot_update_kept`]):
/// here the lamp's error is caught, and then a dialog no one answers stops
/// the update at its end.
#[test]
fn a_play_update_that_fails_keeps_the_errors_caught_before() {
    let mut fx = returning_lamp();
    fx.world.dialogs.records.push(Dialog {
        caller: SIM,
        operands: dialog(1, 0),
        status: 0,
    });
    let mut play = Play::default();
    let tuning = tuning();
    let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    let (update, result) =
        interpreter.lot_update_kept(&mut fx.world, &tuning, &mut answers(&[]), Some(&mut play));
    assert!(
        matches!(result, Err(TickError::Unanswered { given: None, .. })),
        "{result:?}"
    );
    assert_eq!(update.ticked, vec![LAMP]);
    let [error] = update.errors.as_slice() else {
        panic!("{:?}", update.errors);
    };
    assert_eq!(
        (error.object, error.recovery.clone()),
        (LAMP, Recovery::Reset)
    );
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

fn update_with_panels(
    fx: &mut Fixture,
    panels: &RefCell<SpecialPanels>,
    player: &mut dyn Player,
) -> Result<Update, TickError> {
    let tuning = tuning();
    let mut interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    interpreter.special_panels = Some(panels);
    interpreter.lot_update(&mut fx.world, &tuning, player)
}

/// Runs a tree on `SIM` (a person) instead of `LAMP`, storing 1 on True and 2 on
/// False in `SIM`'s data word 0.
fn person_lot(tree: sims_formats::behavior::Behavior) -> Fixture {
    let mut fx = lot(sleeper());
    fx.add(SIM_FILE, 4096, tree);
    fx.thread(SIM, 4096, &[1]);
    fx
}

#[test]
fn special_dialog_travel_lot_pickers_kinds_5_7_8_11_13() {
    // Kind 5 (Downtown, 21..=30): valid lot -> temp 0 = 25, True; out-of-range -> resets to 0 and re-prompts.
    let mut fx = lot(asking(5, 0));
    let panels = RefCell::new(SpecialPanels::default());
    let mut player = ScriptedPlayer::new(
        answers(&[]),
        &[
            SpecialPanelResponse::Lot(Some(99)), // out of 21..=30 -> resets downtown_lot = 0
            SpecialPanelResponse::Lot(Some(25)),
        ],
    );
    let u1 = update_with_panels(&mut fx, &panels, &mut player).unwrap();
    assert_eq!(
        u1.special_panels_answered,
        vec![(
            SpecialPanelRequest::DowntownLotPicker,
            SpecialPanelResponse::Lot(Some(99))
        )]
    );
    assert_eq!(panels.borrow().downtown_lot, 0);
    // Tick 2 re-arms and receives lot 25.
    let u2 = update_with_panels(&mut fx, &panels, &mut player).unwrap();
    assert_eq!(
        u2.special_panels_answered,
        vec![(
            SpecialPanelRequest::DowntownLotPicker,
            SpecialPanelResponse::Lot(Some(25))
        )]
    );
    assert_eq!(panels.borrow().downtown_lot, 25);
    // Tick 3 consumes 25 into temp 0 and takes True (data[0] = 1).
    update_with_panels(&mut fx, &panels, &mut player).unwrap();
    assert_eq!((fx.lamp().data[0], fx.lamp().temps[0]), (1, 25));
    assert_eq!(panels.borrow().downtown_lot, 0);

    // Kind 7 (Vacation Island, 40..=48) checks on_residential (`0x40745f`).
    let mut fx = lot(asking(7, 0));
    fx.world.globals[10] = 40;
    fx.program.zoning.insert(40, 2);
    let panels = RefCell::new(SpecialPanels::default());
    let mut player = ScriptedPlayer::new(answers(&[]), &[SpecialPanelResponse::Lot(Some(44))]);
    let u1 = update_with_panels(&mut fx, &panels, &mut player).unwrap();
    assert_eq!(
        u1.special_panels_answered,
        vec![(
            SpecialPanelRequest::VacationLotPicker {
                on_residential: false
            },
            SpecialPanelResponse::Lot(Some(44))
        )]
    );
    update_with_panels(&mut fx, &panels, &mut player).unwrap();
    assert_eq!((fx.lamp().data[0], fx.lamp().temps[0]), (1, 44));

    // Kind 8 (Old Town / Community): requires LotZoning == 2 (`0x42b9e3`).
    let mut fx = lot(asking(8, 0));
    fx.program.expansions = 0x20;
    fx.program.zoning.insert(5, 1);
    fx.program.zoning.insert(58, 2);
    let panels = RefCell::new(SpecialPanels::default());
    let mut player = ScriptedPlayer::new(
        answers(&[]),
        &[
            SpecialPanelResponse::Lot(Some(5)), // residential -> rejected, resets to 0
            SpecialPanelResponse::Lot(Some(58)),
        ],
    );
    let u1 = update_with_panels(&mut fx, &panels, &mut player).unwrap();
    assert_eq!(
        u1.special_panels_answered[0].0,
        SpecialPanelRequest::OldTownLotPicker { unleashed: true }
    );
    assert_eq!(panels.borrow().old_town_lot, 0);
    update_with_panels(&mut fx, &panels, &mut player).unwrap();
    assert_eq!(panels.borrow().old_town_lot, 58);
    update_with_panels(&mut fx, &panels, &mut player).unwrap();
    assert_eq!((fx.lamp().data[0], fx.lamp().temps[0]), (1, 58));

    // Kind 11 (Studio Town, 81..=89) + Generic Sims Call 35 (`reg+0x10c` star limo).
    let mut fx = person_lot(tree(vec![
        ins(1, 1, 2, [35, 0, 0, 0, 0, 0, 0, 0]), // Generic Sims Call 35 -> sets studio_caller
        ins(36, 2, 3, dialog(11, 0)),
        assign(scope::MY_DATA, 0, scope::LITERAL, 1, 4),
        assign(scope::MY_DATA, 0, scope::LITERAL, 2, 4),
        ins(0, 4, 4, [0; 8]),
    ]));
    {
        let p = fx.sim_mut().person.as_mut().unwrap();
        p.data[80] = 5; // fame/star > 0
        p.data[56] = 0;
    }
    let panels = RefCell::new(SpecialPanels::default());
    let mut player = ScriptedPlayer::new(answers(&[]), &[SpecialPanelResponse::Lot(Some(81))]);
    let u1 = update_with_panels(&mut fx, &panels, &mut player).unwrap();
    assert_eq!(
        u1.special_panels_answered,
        vec![(
            SpecialPanelRequest::StudioTownLotPicker { star_limo: true },
            SpecialPanelResponse::Lot(Some(81))
        )]
    );
    assert_eq!(panels.borrow().studio_caller, None);
    update_with_panels(&mut fx, &panels, &mut player).unwrap();
    assert_eq!((fx.sim().data[0], fx.sim().temps[0]), (1, 81));

    // Kind 13 (Magic Town, 90..=98): `via_blimp` follows global[34] != 0.
    for (g34, via_blimp) in [(0, false), (1, true)] {
        let mut fx = lot(asking(13, 0));
        fx.world.globals[34] = g34;
        let panels = RefCell::new(SpecialPanels::default());
        let mut player = ScriptedPlayer::new(answers(&[]), &[SpecialPanelResponse::Lot(Some(93))]);
        let u1 = update_with_panels(&mut fx, &panels, &mut player).unwrap();
        assert_eq!(
            u1.special_panels_answered,
            vec![(
                SpecialPanelRequest::MagicTownLotPicker { via_blimp },
                SpecialPanelResponse::Lot(Some(93))
            )]
        );
        update_with_panels(&mut fx, &panels, &mut player).unwrap();
        assert_eq!((fx.lamp().data[0], fx.lamp().temps[0]), (1, 93));
    }
}

#[test]
fn special_dialog_wardrobe_kind6_and_charm_kind14() {
    // Non-person calling kind 6 or 14 throws native error 0x1c.
    for kind in [6, 14] {
        let mut fx = lot(asking(kind, 0));
        let panels = RefCell::new(SpecialPanels::default());
        let err = update_with_panels(&mut fx, &panels, &mut answers(&[])).unwrap_err();
        assert!(
            matches!(err, TickError::Fault(f) if matches!(f.trap, Trap::Native { code: 0x1c, .. })),
            "kind {kind} on non-person must throw 0x1c"
        );
    }

    // Kind 6: passes input temp 0 as mode, accepts (True) and cancels (False).
    for (accepted, expected_branch, idx) in [(true, 1, 7), (false, 2, 3)] {
        let mut fx = person_lot(asking(6, 0));
        fx.sim_mut().temps[0] = 2; // formal wear mode
        let panels = RefCell::new(SpecialPanels::default());
        let mut player = ScriptedPlayer::new(
            answers(&[]),
            &[SpecialPanelResponse::Selection {
                accepted,
                index: idx,
            }],
        );
        let u1 = update_with_panels(&mut fx, &panels, &mut player).unwrap();
        assert_eq!(
            u1.special_panels_answered,
            vec![(
                SpecialPanelRequest::WardrobeShop {
                    person: SIM,
                    mode: 2
                },
                SpecialPanelResponse::Selection {
                    accepted,
                    index: idx
                }
            )]
        );
        update_with_panels(&mut fx, &panels, &mut player).unwrap();
        assert_eq!(
            (fx.sim().data[0], fx.sim().temps[0]),
            (expected_branch, idx)
        );
        assert_eq!(panels.borrow().wardrobe_caller, None);
    }

    // Kind 14: Charm Transform accept & cancel.
    for (accepted, expected_branch, idx) in [(true, 1, 4), (false, 2, 1)] {
        let mut fx = person_lot(asking(14, 0));
        let panels = RefCell::new(SpecialPanels::default());
        let mut player = ScriptedPlayer::new(
            answers(&[]),
            &[SpecialPanelResponse::Selection {
                accepted,
                index: idx,
            }],
        );
        let u1 = update_with_panels(&mut fx, &panels, &mut player).unwrap();
        assert_eq!(
            u1.special_panels_answered,
            vec![(
                SpecialPanelRequest::CharmTransform { person: SIM },
                SpecialPanelResponse::Selection {
                    accepted,
                    index: idx
                }
            )]
        );
        update_with_panels(&mut fx, &panels, &mut player).unwrap();
        assert_eq!(
            (fx.sim().data[0], fx.sim().temps[0]),
            (expected_branch, idx)
        );
        assert_eq!(panels.borrow().charm_caller, None);
    }
}

#[test]
fn special_dialog_call_neighbor_kind10_and_pet_picker_kind9() {
    // Kind 10: Call Neighbor (accept sets temp 0 & True; cancel writes -9999, leaves temp 0 untouched & False).
    let mut fx = person_lot(asking(10, 0));
    fx.sim_mut().person.as_mut().unwrap().data[31] = 12;
    fx.sim_mut().temps[0] = 77;
    let panels = RefCell::new(SpecialPanels::default());
    let mut player = ScriptedPlayer::new(answers(&[]), &[SpecialPanelResponse::Neighbor(Some(5))]);
    let u1 = update_with_panels(&mut fx, &panels, &mut player).unwrap();
    assert_eq!(
        u1.special_panels_answered,
        vec![(
            SpecialPanelRequest::CallNeighbor {
                caller_neighbor: 12
            },
            SpecialPanelResponse::Neighbor(Some(5))
        )]
    );
    update_with_panels(&mut fx, &panels, &mut player).unwrap();
    assert_eq!((fx.sim().data[0], fx.sim().temps[0]), (1, 5));
    assert_eq!(panels.borrow().call_neighbor, 0);

    // Kind 10 cancel:
    let mut fx = person_lot(asking(10, 0));
    fx.sim_mut().person.as_mut().unwrap().data[31] = 12;
    fx.sim_mut().temps[0] = 77;
    let panels = RefCell::new(SpecialPanels::default());
    let mut player = ScriptedPlayer::new(answers(&[]), &[SpecialPanelResponse::Neighbor(None)]);
    update_with_panels(&mut fx, &panels, &mut player).unwrap();
    assert_eq!(panels.borrow().call_neighbor, CALL_NEIGHBOR_CANCELLED);
    update_with_panels(&mut fx, &panels, &mut player).unwrap();
    assert_eq!((fx.sim().data[0], fx.sim().temps[0]), (2, 77));
    assert_eq!(panels.borrow().call_neighbor, 0);

    // Kind 9: Pet Picker reads stack_object as a person; accept preserves world.dialogs.declined
    // for Make New Character (`0x44b064`), cancel resets declined to -1.
    for (accepted, expected_branch, idx, expected_declined) in [(true, 1, 9, 9), (false, 2, 4, -1)]
    {
        let mut fx = lot(asking(9, 0));
        fx.lamp_mut().stack[0].stack_object = SIM as i16;
        let panels = RefCell::new(SpecialPanels::default());
        let mut player = ScriptedPlayer::new(
            answers(&[]),
            &[SpecialPanelResponse::Selection {
                accepted,
                index: idx,
            }],
        );
        let u1 = update_with_panels(&mut fx, &panels, &mut player).unwrap();
        assert_eq!(
            u1.special_panels_answered,
            vec![(
                SpecialPanelRequest::PetPicker { pet: SIM },
                SpecialPanelResponse::Selection {
                    accepted,
                    index: idx
                }
            )]
        );
        update_with_panels(&mut fx, &panels, &mut player).unwrap();
        assert_eq!(
            (
                fx.lamp().data[0],
                fx.lamp().temps[0],
                fx.world.dialogs.declined
            ),
            (expected_branch, idx, expected_declined)
        );
    }

    // Kind 9 with non-person stack_object throws native error 0x1c.
    let mut fx = lot(asking(9, 0));
    fx.lamp_mut().stack[0].stack_object = LAMP as i16;
    let panels = RefCell::new(SpecialPanels::default());
    let err = update_with_panels(&mut fx, &panels, &mut answers(&[])).unwrap_err();
    assert!(
        matches!(err, TickError::Fault(f) if matches!(f.trap, Trap::Native { code: 0x1c, .. }))
    );
}

#[test]
fn special_dialog_reference_books_kind12_and_kind15_run_inline_on_same_tick() {
    for (kind, expected_req) in [
        (12, SpecialPanelRequest::SpellBook { caller: Some(SIM) }),
        (15, SpecialPanelRequest::CookBook { caller: Some(SIM) }),
    ] {
        // Headless auto-dismiss completes on tick 1 without yielding!
        let mut fx = person_lot(asking(kind, 0));
        let panels = RefCell::new(SpecialPanels::default());
        let mut player = ScriptedPlayer::new(answers(&[]), &[]);
        let u1 = update_with_panels(&mut fx, &panels, &mut player).unwrap();
        assert_eq!(fx.sim().data[0], 1, "kind {kind} must take True on tick 1");
        assert_eq!(
            u1.special_panels_answered,
            vec![(expected_req.clone(), SpecialPanelResponse::Dismissed)]
        );

        // Interactive (`wait_for_books = true`) pauses cleanly BEFORE play-mode recovery.
        let mut fx = person_lot(asking(kind, 0));
        let panels = RefCell::new(SpecialPanels::default());
        let mut player = ScriptedPlayer {
            answers: answers(&[]),
            special_panels: std::collections::VecDeque::new(),
            wait_for_books: true,
        };
        let mut play = Play::default();
        let tuning = tuning();
        let mut interp = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
        interp.special_panels = Some(&panels);
        let (u, res) = interp.lot_update_kept(&mut fx.world, &tuning, &mut player, Some(&mut play));
        assert!(
            u.errors.is_empty(),
            "kind {kind} pause must not trip play-mode recovery"
        );
        assert_eq!(
            res,
            Err(TickError::UnansweredSpecialPanel {
                shown: ShownSpecialPanel {
                    tick: 2,
                    request: expected_req.clone(),
                },
                given: None,
            })
        );
    }

    // Non-person callers pass `NULL` to `0x5107dc` (`caller: None`) with NO `0x1c` fault
    // (`0x4518e6..0x45197c`).
    for (kind, expected_req) in [
        (12, SpecialPanelRequest::SpellBook { caller: None }),
        (15, SpecialPanelRequest::CookBook { caller: None }),
    ] {
        let mut fx = lot(asking(kind, 0));
        let panels = RefCell::new(SpecialPanels::default());
        let mut player = ScriptedPlayer::new(answers(&[]), &[]);
        let u1 = update_with_panels(&mut fx, &panels, &mut player).unwrap();
        assert_eq!(
            fx.lamp().data[0],
            1,
            "non-person kind {kind} must succeed and take True"
        );
        assert_eq!(
            u1.special_panels_answered,
            vec![(expected_req, SpecialPanelResponse::Dismissed)]
        );
    }
}

#[test]
fn special_dialog_reference_book_in_run_tree_by_name_callbacks_and_budget_boundary() {
    // 1. Spellbook opened inside Run Tree By Name mode 0 (me's thread) and mode 1
    //    (stack object's thread) must NOT trigger native 0x44f: headless auto-dismiss
    //    completes on tick 1, and interactive wait unwinds cleanly before play-mode recovery.
    for mode in [0u8, 1u8] {
        let mut fx = person_lot(tree(vec![
            // Stack object = LAMP so mode 1 targets LAMP's thread while mode 0 uses SIM's.
            assign(scope::STACK_OBJECT, 0, scope::LITERAL, LAMP as i16, 1),
            // Run Tree By Name (opcode 28): STR# 303 ([47, 1]), entry 1 ("book_cb"), mode 0 or 1.
            ins(28, 2, 3, [47, 1, 0, 0, 1, mode, 0, 0]),
            assign(scope::MY_DATA, 0, scope::LITERAL, 1, 4),
            assign(scope::MY_DATA, 0, scope::LITERAL, 2, 4),
            ins(0, 4, 4, [0; 8]),
        ]));
        fx.program
            .libraries
            .get_mut(&SIM_FILE)
            .unwrap()
            .tree_names
            .push("book_cb".to_owned());
        fx.program
            .libraries
            .get_mut(&LAMP_FILE)
            .unwrap()
            .tree_names
            .push("book_cb".to_owned());
        fx.program
            .libraries
            .get_mut(&LAMP_FILE)
            .unwrap()
            .names
            .insert(4097, "book_cb".to_owned());
        fx.add(
            LAMP_FILE,
            4097,
            tree(vec![ins(36, 0xfe, 0xff, dialog(12, 0))]),
        );
        let expected_caller = if mode == 0 { Some(SIM) } else { None };

        // Interactive wait pauses cleanly without 0x44f or play-mode recovery.
        let mut interactive_world = fx.world.clone();
        let panels = RefCell::new(SpecialPanels::default());
        let mut waiting_player = ScriptedPlayer {
            answers: answers(&[]),
            special_panels: std::collections::VecDeque::new(),
            wait_for_books: true,
        };
        let mut play = Play::default();
        let tuning = tuning();
        let mut interp = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
        interp.special_panels = Some(&panels);
        let (u, res) = interp.lot_update_kept(
            &mut interactive_world,
            &tuning,
            &mut waiting_player,
            Some(&mut play),
        );
        assert!(
            u.errors.is_empty(),
            "mode {mode} nested book pause must not trip play-mode recovery: {:?}",
            u.errors
        );
        assert_eq!(
            res,
            Err(TickError::UnansweredSpecialPanel {
                shown: ShownSpecialPanel {
                    tick: 2,
                    request: SpecialPanelRequest::SpellBook {
                        caller: expected_caller,
                    },
                },
                given: None,
            })
        );

        // Replaying with Dismissed completes cleanly on the exact same tick!
        let mut dismiss_player =
            ScriptedPlayer::new(answers(&[]), &[SpecialPanelResponse::Dismissed]);
        let u_done = update_with_panels(&mut fx, &panels, &mut dismiss_player).unwrap();
        assert_eq!(
            fx.sim().data[0],
            1,
            "mode {mode} nested book must return True"
        );
        assert_eq!(
            u_done.special_panels_answered,
            vec![(
                SpecialPanelRequest::SpellBook {
                    caller: expected_caller,
                },
                SpecialPanelResponse::Dismissed
            )]
        );
    }

    // 2. Instruction-budget boundary: a Spellbook consumes 1 instruction of TICK_BUDGET
    //    (458_605). Run 458_602 instructions before Dialog(12), then 1 instruction + Sleep(1)
    //    (total = 458_605 -> succeeds) vs 2 instructions + Sleep(1) (total = 458_606 ->
    //    Trap::BudgetExhausted { budget: TICK_BUDGET }).
    let outer_iters: i16 = 228; // 228 * (1000 * 2 + 3) = 456_684 instructions
    let rem_iters: i16 = 959; // 959 * 2 = 1_918 instructions -> 458_602 before Dialog(12)
    assert_eq!(
        usize::try_from(outer_iters).unwrap() * 2003 + usize::try_from(rem_iters).unwrap() * 2 + 3,
        sims_vm::TICK_BUDGET
    );
    let build_budget_tree = |one_over: bool| {
        let thousand = 1000i16.to_le_bytes();
        let outer = outer_iters.to_le_bytes();
        let rem = rem_iters.to_le_bytes();
        tree(vec![
            // 0: temp[0] += 1 -> 1
            ins(
                2,
                1,
                1,
                [0, 0, 1, 0, 0, op::ADD, scope::TEMPS, scope::LITERAL],
            ),
            // 1: temp[0] < 1000 ? 0 : 2
            ins(
                2,
                0,
                2,
                [
                    0,
                    0,
                    thousand[0],
                    thousand[1],
                    0,
                    1,
                    scope::TEMPS,
                    scope::LITERAL,
                ],
            ),
            // 2: temp[0] := 0 -> 3
            assign(scope::TEMPS, 0, scope::LITERAL, 0, 3),
            // 3: temp[1] += 1 -> 4
            ins(
                2,
                4,
                4,
                [1, 0, 1, 0, 0, op::ADD, scope::TEMPS, scope::LITERAL],
            ),
            // 4: temp[1] < outer_iters ? 0 : 5
            ins(
                2,
                0,
                5,
                [1, 0, outer[0], outer[1], 0, 1, scope::TEMPS, scope::LITERAL],
            ),
            // 5: temp[2] += 1 -> 6
            ins(
                2,
                6,
                6,
                [2, 0, 1, 0, 0, op::ADD, scope::TEMPS, scope::LITERAL],
            ),
            // 6: temp[2] < rem_iters ? 5 : 7
            ins(
                2,
                5,
                7,
                [2, 0, rem[0], rem[1], 0, 1, scope::TEMPS, scope::LITERAL],
            ),
            // 7: Dialog kind 12 (instruction 458_603) -> 8
            ins(36, 8, 8, dialog(12, 0)),
            // 8: temp[3] := 1 (instruction 458_604) -> 10 if one_over else 9
            assign(
                scope::TEMPS,
                3,
                scope::LITERAL,
                1,
                if one_over { 10 } else { 9 },
            ),
            // 9: Sleep (instruction 458_605 if !one_over, 458_606 if one_over)
            ins(0, 9, 9, [0; 8]),
            // 10: extra instruction (458_605) before Sleep (458_606 -> BudgetExhausted)
            assign(scope::TEMPS, 3, scope::LITERAL, 2, 9),
        ])
    };
    let mut fx_ok = person_lot(build_budget_tree(false));
    let panels_ok = RefCell::new(SpecialPanels::default());
    let mut player_ok = ScriptedPlayer::new(answers(&[]), &[]);
    let u_ok = update_with_panels(&mut fx_ok, &panels_ok, &mut player_ok)
        .expect("book at exact TICK_BUDGET boundary must succeed");
    assert_eq!(u_ok.special_panels_answered.len(), 1);
    assert_eq!(fx_ok.sim().temps[3], 1);

    let mut fx_over = person_lot(build_budget_tree(true));
    let panels_over = RefCell::new(SpecialPanels::default());
    let mut player_over = ScriptedPlayer::new(answers(&[]), &[]);
    let err_over = update_with_panels(&mut fx_over, &panels_over, &mut player_over).unwrap_err();
    assert!(
        matches!(err_over, TickError::Fault(ref f) if matches!(f.trap, Trap::BudgetExhausted { .. })),
        "exceeding TICK_BUDGET by 1 instruction past book must trap BudgetExhausted, got {err_over:?}"
    );
}

#[test]
fn special_dialog_informs_precheck_kind13_gate_and_pet_picker_speed_hold() {
    // 1. `0x451562..0x4515a9` + `0x4443ab`: `informs` (operand 7 bit 0) runs AFTER
    //    the record pre-check (closing SHOWN -> DONE) and handles kinds 5, 7, 8, 11, 13
    //    through `end_of_update` without portrait formatting.
    for (kind, expected_req, response) in [
        (
            5,
            SpecialPanelRequest::DowntownLotPicker,
            SpecialPanelResponse::Lot(Some(22)),
        ),
        (
            7,
            SpecialPanelRequest::VacationLotPicker {
                on_residential: true,
            },
            SpecialPanelResponse::Lot(Some(41)),
        ),
        (
            8,
            SpecialPanelRequest::OldTownLotPicker { unleashed: false },
            SpecialPanelResponse::Lot(Some(58)),
        ),
        (
            11,
            SpecialPanelRequest::StudioTownLotPicker { star_limo: false },
            SpecialPanelResponse::Lot(Some(81)),
        ),
        (
            13,
            SpecialPanelRequest::MagicTownLotPicker { via_blimp: true },
            SpecialPanelResponse::Lot(Some(93)),
        ),
    ] {
        // Set operand 7 = 0x81 (bit 0 = informs, bit 7 = non-modal) AND global[34] = 0:
        // `0x4443ab` passes literal `0` to the dedicated constructors, ignoring bit 7,
        // and `0x551285` initializes `[dialog+0xec] = 1` (`via_blimp: true`)!
        let mut fx = lot(tree(vec![
            ins(36, 1, 2, [0, 0, 0, 0, 0, kind, 0, 0x81]),
            assign(scope::MY_DATA, 0, scope::LITERAL, 1, 3),
            assign(scope::MY_DATA, 0, scope::LITERAL, 2, 3),
            ins(0, 3, 3, [0; 8]),
        ]));
        fx.world.globals[34] = 0;
        fx.world.dialogs.records.push(sims_sim::Dialog {
            caller: LAMP,
            operands: dialog(0, 0),
            status: 1, // SHOWN
        });
        let panels = RefCell::new(SpecialPanels::default());
        let mut player = ScriptedPlayer::new(answers(&[]), std::slice::from_ref(&response));
        let u = update_with_panels(&mut fx, &panels, &mut player).unwrap();
        assert_eq!(fx.lamp().data[0], 1, "informs kind {kind} must take True");
        assert_eq!(u.special_panels_answered, vec![(expected_req, response)]);
        assert!(
            fx.world.dialogs.records.is_empty(),
            "old SHOWN record and informs record must both be closed & drained"
        );
    }

    // Unsupported informs kind (e.g. kind 6) must STILL run the SHOWN -> DONE pre-check first.
    let mut fx = lot(tree(vec![
        ins(36, 1, 2, [0, 0, 0, 0, 0, 6, 0, 1]),
        assign(scope::MY_DATA, 0, scope::LITERAL, 1, 3),
        assign(scope::MY_DATA, 0, scope::LITERAL, 2, 3),
        ins(0, 3, 3, [0; 8]),
    ]));
    fx.world.dialogs.records.push(sims_sim::Dialog {
        caller: LAMP,
        operands: dialog(0, 0),
        status: 1, // SHOWN
    });
    let panels = RefCell::new(SpecialPanels::default());
    let err = update_with_panels(&mut fx, &panels, &mut answers(&[])).unwrap_err();
    assert!(matches!(
        err,
        TickError::Fault(f) if matches!(f.trap, Trap::Unsupported(_))
    ));
    assert_eq!(
        fx.world.dialogs.records[0].status,
        5, // DONE
        "0x451562..0x45158d pre-check must close SHOWN record before informs trap"
    );

    // 2. Kind 13 `[0x6f2a58]+0xf8` gate (`magic_picker_blocked`): yields without arming -1
    //    while blocked, then arms -1 and opens MagicTownLotPicker once cleared.
    let mut fx = lot(asking(13, 0));
    let panels = RefCell::new(SpecialPanels {
        magic_picker_blocked: true,
        ..SpecialPanels::default()
    });
    let mut player = ScriptedPlayer::new(answers(&[]), &[SpecialPanelResponse::Lot(Some(93))]);
    let u_blocked = update_with_panels(&mut fx, &panels, &mut player).unwrap();
    assert!(u_blocked.special_panels_answered.is_empty());
    assert_eq!(
        panels.borrow().magic_lot,
        0,
        "kind 13 must NOT arm magic_lot = -1 while magic_picker_blocked is true"
    );
    panels.borrow_mut().magic_picker_blocked = false;
    let u_open = update_with_panels(&mut fx, &panels, &mut player).unwrap();
    assert_eq!(
        u_open.special_panels_answered,
        vec![(
            SpecialPanelRequest::MagicTownLotPicker { via_blimp: false },
            SpecialPanelResponse::Lot(Some(93))
        )]
    );
    assert_eq!(panels.borrow().magic_lot, 93);

    // 3. Kind 9 Pet Picker saves speed, sets 760, and restores via `World::set_speed`
    //    (`0x441bb2..0x441de8`), which updates `return_speed` when idle speed hold is active!
    let mut fx = lot(asking(9, 0));
    fx.lamp_mut().stack[0].stack_object = SIM as i16;
    fx.sim_mut().person.as_mut().unwrap().motives[11] = -10.0;
    fx.world.globals[18] = -1;
    fx.world.set_speed(25);
    assert!(fx.world.idle_speed(true));
    assert_eq!(fx.world.speed(), 1000);
    assert_eq!(fx.world.return_speed(), 25);
    let panels = RefCell::new(SpecialPanels::default());
    let mut player = ScriptedPlayer::new(
        answers(&[]),
        &[SpecialPanelResponse::Selection {
            accepted: true,
            index: 2,
        }],
    );
    update_with_panels(&mut fx, &panels, &mut player).unwrap();
    assert_eq!(
        fx.world.speed(),
        1000,
        "active idle-speed hold must stay at 1000"
    );
    assert_eq!(
        fx.world.return_speed(),
        1000,
        "restoring saved speed (1000) through World::set_speed during idle hold updates return_speed"
    );

    // 4. Headless `ScriptedPlayer` with a queued `Lot(Some(93))` preceded by a Spellbook
    //    must auto-dismiss the book WITHOUT consuming the queued picker response!
    let mut fx = person_lot(tree(vec![
        ins(36, 1, 1, dialog(12, 0)), // inline Spellbook
        ins(36, 2, 2, dialog(13, 0)), // Magic Town lot picker
        assign(scope::MY_DATA, 0, scope::LITERAL, 1, 3),
        ins(0, 3, 3, [0; 8]),
    ]));
    let panels = RefCell::new(SpecialPanels::default());
    let mut player = ScriptedPlayer::new(answers(&[]), &[SpecialPanelResponse::Lot(Some(93))]);
    let u1 = update_with_panels(&mut fx, &panels, &mut player).unwrap();
    assert_eq!(
        u1.special_panels_answered,
        vec![
            (
                SpecialPanelRequest::SpellBook { caller: Some(SIM) },
                SpecialPanelResponse::Dismissed
            ),
            (
                SpecialPanelRequest::MagicTownLotPicker { via_blimp: false },
                SpecialPanelResponse::Lot(Some(93))
            ),
        ]
    );
    assert_eq!(panels.borrow().magic_lot, 93);
}

#[test]
fn special_panel_end_of_update_order_and_mutual_exclusion() {
    // 0x4413e1..0x442320: Kind 10 (call_neighbor) falls through to Kind 5 (downtown_lot),
    // which falls through to the mutually exclusive chain (8 -> 6 -> 14 -> 9 -> 7 -> 11 -> 13).
    let mut fx = lot(sleeper());
    fx.program.zoning.insert(58, 2);
    let panels = RefCell::new(SpecialPanels {
        call_neighbor: -7,
        downtown_lot: -1,
        old_town_lot: -1,
        magic_lot: -1,
        ..SpecialPanels::default()
    });
    let mut player = ScriptedPlayer::new(
        answers(&[]),
        &[
            SpecialPanelResponse::Neighbor(Some(3)),
            SpecialPanelResponse::Lot(Some(22)),
            SpecialPanelResponse::Lot(Some(58)),
            SpecialPanelResponse::Lot(Some(93)),
        ],
    );
    // Tick 1 fires Kind 10, Kind 5, and Kind 8 (jumping past Kind 13!).
    let u1 = update_with_panels(&mut fx, &panels, &mut player).unwrap();
    assert_eq!(
        u1.special_panels_answered
            .iter()
            .map(|(r, _)| r.kind())
            .collect::<Vec<_>>(),
        vec![10, 5, 8]
    );
    assert_eq!(panels.borrow().magic_lot, -1);
    // Tick 2 fires the remaining Kind 13!
    let u2 = update_with_panels(&mut fx, &panels, &mut player).unwrap();
    assert_eq!(
        u2.special_panels_answered
            .iter()
            .map(|(r, _)| r.kind())
            .collect::<Vec<_>>(),
        vec![13]
    );
    assert_eq!(panels.borrow().magic_lot, 93);
}

#[test]
fn play_mode_catches_missing_object_in_removal_cleanup_while_strict_mode_fails() {
    use sims_formats::execution::ObjectFunction;
    use sims_formats::saved_objects::{Interaction, ObjectUse, Relationship};

    // Give the lamp a Reset (function 3) tree that runs Set To Next mode 9
    // (opcode 31, byte 3 = 9) with local 0 pointing to missing object 383,
    // raising `Trap::native(10, "Set To Next reference object is missing")` (0x0a)
    // during `end_of_update` -> `remove_now` -> `delete_one` -> `reset`.
    let make_fx = || {
        let mut fx = lot(sleeper());
        fx.add(
            LAMP_FILE,
            4097,
            tree(vec![
                assign(scope::LOCALS, 0, scope::LITERAL, 383, 1),
                ins(31, RETURN_TRUE, RETURN_FALSE, [0, 0, 0, 0, 9, 0, 0, 0]),
            ]),
        );
        let callbacks = [0, 4096, 0, 4097].map(|action| ObjectFunction {
            condition: 0,
            action,
        });
        fx.program
            .libraries
            .get_mut(&LAMP_FILE)
            .unwrap()
            .callbacks
            .insert(128, callbacks.to_vec());
        // Prepare the lamp off-lot and queued for removal.
        let lamp = fx.lamp_mut();
        lamp.set_countdown(5);
        lamp.data[2] = 0;
        lamp.slots.truncate(1);
        lamp.position = [-16, -16, 1];
        // Attach a relationship, a queued action, an object use, and a route
        // targeting the lamp on SIM so we can verify zero dangling references
        // after Play Mode force-removes it.
        fx.sim_mut().relationships.push(Relationship {
            present: 1,
            target: i32::from(LAMP),
            values: vec![10],
        });
        let sim_person = fx.sim_mut().person.as_mut().unwrap();
        let mut action = sim_person.last.clone();
        action.target = LAMP as i16;
        action.icon = LAMP as i16;
        action.uid = 77;
        sim_person.queue.push(action.clone());
        sim_person.last = Interaction {
            target: LAMP as i16,
            icon: LAMP as i16,
            ..action
        };
        sim_person.object_uses.push(ObjectUse {
            target: LAMP as i16,
            stack_depth: 1,
            unknown: 0,
        });
        fx.world.routing.routes.insert(
            SIM,
            sims_sim::route::Route {
                contexts: vec![sims_sim::route::Context {
                    candidates: Vec::new(),
                    slot: sims_sim::route::RouteSlot {
                        offset: [0; 3],
                        number: 1,
                        standing: 0,
                        sitting: 0,
                        ground: 0,
                        flags: 0,
                        snap_target: 0,
                        proximity: [0; 3],
                        gradient: 0,
                        facing: 0,
                        resolution: 0,
                    },
                    target: LAMP,
                    person: SIM,
                    chosen: -1,
                    best: -1,
                    word_58: 0,
                    time: 0,
                    saved: [0; 2],
                    word_68: 0,
                    start: [0; 3],
                    byte_78: false,
                    objects: [0; 2],
                    byte_84: false,
                    result: 0,
                    word_8c: 0,
                    byte_8e: false,
                    word_94: 0,
                    limit: 0x30,
                    action: 0,
                }],
                ..Default::default()
            },
        );
        fx.world.removals = vec![LAMP];
        fx
    };

    // Strict Mode still fails loudly with native error 0x0a.
    let mut strict = make_fx();
    // Clear object_uses on strict fixture so drop_references doesn't fail on
    // person reset before reaching the lamp's Reset tree.
    strict
        .sim_mut()
        .person
        .as_mut()
        .unwrap()
        .object_uses
        .clear();
    let strict_err = update(&mut strict, &mut answers(&[])).unwrap_err();
    assert!(
        matches!(
            strict_err,
            TickError::Fault(ref f) if f.trap.code() == Some(10)
        ),
        "strict mode must fail with native error 0x0a: {strict_err:?}"
    );

    // Play Mode catches the 0x0a fault in `update.errors`, force-removes the
    // queued object cleanly, leaves zero dangling references, and keeps ticking.
    let mut played = make_fx();
    played
        .sim_mut()
        .person
        .as_mut()
        .unwrap()
        .object_uses
        .clear();
    let mut play = Play::default();
    let u1 = play_update(&mut played, &mut play);
    assert!(u1.full);
    assert_eq!(u1.removed, vec![LAMP]);
    assert_eq!(u1.errors.len(), 1, "{:?}", u1.errors);
    assert_eq!(u1.errors[0].object, LAMP);
    assert!(!u1.errors[0].in_thread);
    assert_eq!(u1.errors[0].fault.trap.code(), Some(10));
    assert!(!played.world.objects.contains_key(&LAMP));
    assert!(!played.world.order.contains(&LAMP));
    assert!(played.world.removals.is_empty());
    let sim = &played.world.objects[&SIM];
    assert!(sim.relationships.is_empty());
    let person = sim.person.as_ref().unwrap();
    assert!(person.queue.is_empty());
    assert!(person.object_uses.is_empty());
    assert_eq!(person.last.target, 0);
    assert_eq!(
        played.world.routing.routes[&SIM].contexts[0].target,
        u16::MAX
    );

    // Subsequent ticks continue cleanly with zero errors.
    let u2 = play_update(&mut played, &mut play);
    assert!(u2.full && u2.errors.is_empty());
    assert_eq!(played.world.ticks, 3);
}

#[test]
fn play_mode_force_removes_faulting_multipart_group_and_preserves_indicators() {
    use sims_formats::execution::ObjectFunction;
    use sims_formats::saved_objects::{Multipart, Slot};

    let mut fx = lot(sleeper());
    // Reset tree triggers explicit error branch (253) during deletion.
    fx.add(LAMP_FILE, 4097, tree(vec![ins(2, ERROR, ERROR, [0; 8])]));
    let callbacks = [0, 4096, 0, 4097].map(|action| ObjectFunction {
        condition: 0,
        action,
    });
    fx.program
        .libraries
        .get_mut(&LAMP_FILE)
        .unwrap()
        .callbacks
        .insert(128, callbacks.to_vec());

    // Build a 2-tile object (LAMP lead + partner 90) plus a held child (91)
    // and a protected indicator (383) sitting in a slot.
    let mut partner = fx.lamp().clone();
    partner.id = 90;
    partner.position = [-16, -16, 1];
    partner.data[2] = 0;
    partner.slots.truncate(1);
    partner.multipart = Some(Multipart {
        parent: LAMP as i16,
        group: [1, 0, 0],
        offset: [1, 0, 0],
    });
    let mut child = fx.lamp().clone();
    child.id = 91;
    child.position = [-16, -16, 1];
    child.data[2] = LAMP as i16;
    child.slots.truncate(1);
    let mut indicator = fx.lamp().clone();
    indicator.id = 383;
    indicator.guid = sims_vm::INDICATORS[0];
    indicator.position = [-16, -16, 1];
    indicator.data[2] = LAMP as i16;
    indicator.slots.truncate(1);

    fx.world.objects.insert(90, partner);
    fx.world.objects.insert(91, child);
    fx.world.objects.insert(383, indicator);
    fx.world.order.extend([90, 91, 383]);
    fx.world.next_parts.insert(LAMP, 90);
    fx.world.indicators = [383, 0];

    let lamp = fx.lamp_mut();
    lamp.set_countdown(5);
    lamp.position = [-16, -16, 1];
    lamp.data[2] = 0;
    lamp.multipart = Some(Multipart {
        parent: 0,
        group: [0, 0, 0],
        offset: [0, 0, 0],
    });
    lamp.slots = vec![
        Slot {
            unknown: 0,
            object: 0,
        },
        Slot {
            unknown: 0,
            object: 91,
        },
        Slot {
            unknown: 0,
            object: 383,
        },
    ];
    fx.world.removals = vec![LAMP];

    let mut play = Play::default();
    let u1 = play_update(&mut fx, &mut play);
    assert!(u1.full);
    assert_eq!(u1.removed, vec![LAMP]);
    assert_eq!(u1.errors.len(), 1);
    // Lead, partner tile, and regular held child are all gone; indicator 383 survives!
    assert!(!fx.world.objects.contains_key(&LAMP));
    assert!(!fx.world.objects.contains_key(&90));
    assert!(!fx.world.objects.contains_key(&91));
    assert!(!fx.world.next_parts.contains_key(&LAMP));
    assert!(fx.world.objects.contains_key(&383));
    assert_eq!(fx.world.indicators[0], 383);
    assert_eq!(fx.world.objects[&383].data[2], 0);
    // Next tick runs cleanly.
    let u2 = play_update(&mut fx, &mut play);
    assert!(u2.full && u2.errors.is_empty());
}

/// A star marker held by an object that Play Mode force-removes is parked
/// off the lot and goes back to the available star pool, as a star taken
/// off a head does (0x4481eb), not left in the in-use pool: a stale in-use
/// entry would also make the next refill two stars instead of four
/// (0x446356).
#[test]
fn play_mode_force_removal_returns_a_held_star_to_the_pool() {
    use sims_formats::execution::ObjectFunction;
    use sims_formats::saved_objects::Slot;

    const STAR_MARKER: u32 = 0xE933_7020;
    const STAR: u16 = 383;
    let mut fx = lot(sleeper());
    // Reset tree triggers explicit error branch (253) during deletion.
    fx.add(LAMP_FILE, 4097, tree(vec![ins(2, ERROR, ERROR, [0; 8])]));
    let callbacks = [0, 4096, 0, 4097].map(|action| ObjectFunction {
        condition: 0,
        action,
    });
    fx.program
        .libraries
        .get_mut(&LAMP_FILE)
        .unwrap()
        .callbacks
        .insert(128, callbacks.to_vec());

    let mut star = fx.lamp().clone();
    star.id = STAR;
    star.guid = STAR_MARKER;
    star.position = [-16, -16, 1];
    star.data[2] = LAMP as i16;
    star.slots.truncate(1);
    fx.world.objects.insert(STAR, star);
    fx.world.order.push(STAR);
    fx.world.indicator_queues.stars_in_use = vec![STAR];
    fx.world.indicator_queues.available_stars = vec![7];

    let lamp = fx.lamp_mut();
    lamp.set_countdown(5);
    lamp.slots = vec![
        Slot {
            unknown: 0,
            object: 0,
        },
        Slot {
            unknown: 0,
            object: STAR as i16,
        },
    ];
    fx.world.removals = vec![LAMP];

    let mut play = Play::default();
    let u1 = play_update(&mut fx, &mut play);
    assert_eq!(u1.removed, vec![LAMP]);
    assert_eq!(u1.errors.len(), 1, "the lamp is force-removed");
    assert!(!fx.world.objects.contains_key(&LAMP));
    assert!(fx.world.objects.contains_key(&STAR));
    assert_eq!(fx.world.objects[&STAR].data[2], 0);
    assert!(fx.world.indicator_queues.stars_in_use.is_empty());
    assert_eq!(fx.world.indicator_queues.available_stars, vec![7, STAR]);
}

#[test]
fn play_mode_survives_partial_pass_person_removal_and_dialog_owner_removal() {
    // 1. Partial-pass `+0x109` person removal deferral.
    let make_partial_person_fx = || {
        let mut fx = lot(sleeper());
        // Global 17 (PAUSED) = 1 forces a partial pass.
        fx.world.globals[17] = 1;
        fx.world.removals = vec![SIM];
        fx
    };
    let mut strict = make_partial_person_fx();
    let strict_err = update(&mut strict, &mut answers(&[])).unwrap_err();
    assert!(
        matches!(strict_err, TickError::Unsupported(ref msg) if msg.contains("+0x109")),
        "strict mode must reject queued person after partial pass: {strict_err:?}"
    );

    let mut played = make_partial_person_fx();
    let mut play = Play::default();
    // Tick 1 (partial pass): Play Mode defers SIM's removal without halting.
    let u1 = play_update(&mut played, &mut play);
    assert!(!u1.full);
    assert!(u1.errors.is_empty());
    assert_eq!(played.world.removals, vec![SIM]);
    assert!(played.world.objects.contains_key(&SIM));

    // Tick 2 (unpaused full pass): SIM ticks without tripping the strict +0x109
    // check on `taken_off`, and is then removed at `end_of_update` (force-removed
    // if rooms aren't built on this minimal synthetic lot) without halting!
    played.world.globals[17] = 0;
    let u2 = play_update(&mut played, &mut play);
    assert!(u2.full);
    assert_eq!(u2.removed, vec![SIM]);
    assert!(!played.world.objects.contains_key(&SIM));
    assert!(played.world.removals.is_empty());

    // Tick 3 continues cleanly.
    let u3 = play_update(&mut played, &mut play);
    assert!(u3.full && u3.errors.is_empty());

    // 2. Dialog-owning object removal.
    let make_dialog_owner_fx = || {
        let mut fx = lot(sleeper());
        let lamp = fx.lamp_mut();
        lamp.set_countdown(5);
        lamp.data[2] = 0;
        lamp.slots.truncate(1);
        lamp.position = [-16, -16, 1];
        fx.world.dialogs.records.push(Dialog {
            caller: LAMP,
            operands: dialog(0, 0),
            status: 1, // SHOWN
        });
        fx.world.removals = vec![LAMP];
        fx
    };
    let mut strict_dlg = make_dialog_owner_fx();
    let strict_dlg_err = update(&mut strict_dlg, &mut answers(&[])).unwrap_err();
    assert!(
        matches!(strict_dlg_err, TickError::Unsupported(ref msg) if msg.contains("owns a dialog")),
        "strict mode must reject dialog-owning removal: {strict_dlg_err:?}"
    );

    let mut played_dlg = make_dialog_owner_fx();
    let mut play_dlg = Play::default();
    let u_dlg = play_update(&mut played_dlg, &mut play_dlg);
    assert!(u_dlg.full);
    assert!(u_dlg.errors.is_empty());
    assert_eq!(u_dlg.removed, vec![LAMP]);
    assert!(!played_dlg.world.objects.contains_key(&LAMP));
    assert!(played_dlg.world.dialogs.records.is_empty());
    let u_next = play_update(&mut played_dlg, &mut play_dlg);
    assert!(u_next.full && u_next.errors.is_empty());
}

#[test]
fn play_mode_survives_daily_rollover_faults_while_strict_mode_fails() {
    let make_midnight_base = || {
        let mut fx = lot(sleeper());
        // Clock at 23:59:58 so the next full update triggers midnight rollover.
        fx.world.globals[0] = 23; // HOURS
        fx.world.globals[5] = 59; // MINUTES
        fx.world.globals[6] = 58; // SECONDS
        fx.world.globals[1] = 10; // DAY
        fx.world.globals[7] = 6; // MONTH
        fx.world.globals[8] = 1997; // YEAR
        fx.world.globals[28] = 0; // DAYS_PLAYED
        fx
    };

    // 1. Unsaved budget day ONLY (no missing object in `world.order`), so the
    // budget fallback diagnostic and shifted day arrays are verified directly.
    let make_budget_fx = || {
        let mut fx = make_midnight_base();
        fx.world.budget.days = vec![Some([7, 0, 0, 0, 0, 0, 0, 0]), None, None, None, None, None];
        fx
    };
    let mut strict_budget = make_budget_fx();
    let strict_budget_err = update(&mut strict_budget, &mut answers(&[])).unwrap_err();
    assert!(
        matches!(
            strict_budget_err,
            TickError::Fault(ref f) if matches!(f.trap, Trap::Unsupported(ref msg) if msg.contains("0x46d417"))
        ),
        "strict mode must fail on unsaved budget day: {strict_budget_err:?}"
    );
    let mut played_budget = make_budget_fx();
    let mut play_budget = Play::default();
    let ub1 = play_update(&mut played_budget, &mut play_budget);
    assert!(ub1.full);
    assert!(
        ub1.errors.iter().any(|e| matches!(
            e.fault.trap,
            Trap::Unsupported(ref msg) if msg.contains("0x46d417")
        ) && e.recovery == Recovery::Reset),
        "expected unsaved budget diagnostic with Recovery::Reset, got {:?}",
        ub1.errors
    );
    assert_eq!(played_budget.world.budget.days[0], Some([0; 8]));
    assert_eq!(
        played_budget.world.budget.days[1],
        Some([7, 0, 0, 0, 0, 0, 0, 0])
    );
    for day in &played_budget.world.budget.days[2..] {
        assert_eq!(*day, Some([0; 8]));
    }
    assert_eq!(
        (
            played_budget.world.globals[0],
            played_budget.world.globals[5],
            played_budget.world.globals[6],
            played_budget.world.globals[1],
            played_budget.world.globals[28],
        ),
        (0, 0, 0, 11, 1)
    );
    let ub2 = play_update(&mut played_budget, &mut play_budget);
    assert!(ub2.full && ub2.errors.is_empty());

    // 2. Missing object ID in `world.order` triggers per-object faults in both
    // `daily::turn_day_play` and `day_statistics_play` without halting.
    let make_missing_fx = || {
        let mut fx = make_midnight_base();
        fx.world.budget.days = vec![Some([0; 8]); 6];
        fx.world.order.push(777);
        fx
    };
    let mut strict = make_missing_fx();
    let strict_err = update(&mut strict, &mut answers(&[])).unwrap_err();
    assert!(
        matches!(strict_err, TickError::Fault(ref f) if f.object == 777),
        "strict mode must fail on missing object 777 during daily rollover: {strict_err:?}"
    );

    let mut played = make_missing_fx();
    let mut play = Play::default();
    let u1 = play_update(&mut played, &mut play);
    assert!(u1.full);
    assert!(
        u1.errors
            .iter()
            .any(|e| e.object == 777 && e.recovery == Recovery::Reset),
        "expected missing-object 777 diagnostic, got {:?}",
        u1.errors
    );
    assert_eq!(
        (
            played.world.globals[0],
            played.world.globals[5],
            played.world.globals[6],
            played.world.globals[1],
            played.world.globals[28],
        ),
        (0, 0, 0, 11, 1)
    );
    let u2 = play_update(&mut played, &mut play);
    assert!(u2.full && u2.errors.is_empty());
}

#[test]
fn play_mode_catches_dynamic_group_leave_group_fault_and_removes_tile() {
    use sims_formats::execution::ObjectFunction;
    use sims_formats::saved_objects::Multipart;

    let make_dyn_fx = || {
        let mut fx = lot(sleeper());
        fx.world.size = 64;
        let empty_grid = || sims_formats::lot::Array {
            width: 64,
            height: 64,
            stride: 2,
            encoding: 0,
            data: vec![0; 64 * 64 * 2],
        };
        fx.world.arrays.insert(3, empty_grid());
        fx.world.arrays.insert(103, empty_grid());
        fx.world.rooms.map = [vec![0; 64 * 64], vec![0; 64 * 64]];
        fx.world.rooms.rooms.insert(
            0,
            sims_sim::rooms::Room {
                outdoors: true,
                ..Default::default()
            },
        );
        // Give LAMP a definition with word[10] != 0 (multi-tile group), a
        // catalog master OBJD (word[11] == -1), and a faulting callback
        // function 8 (DYNAMIC_GROUP).
        let lib = fx.program.libraries.get_mut(&LAMP_FILE).unwrap();
        lib.definitions.get_mut(&128).unwrap()[10] = 1;
        let mut master_def = lib.definitions[&128];
        master_def[11] = -1;
        lib.definitions.insert(129, master_def);
        lib.definition_order.push(129);
        fx.add(LAMP_FILE, 4098, tree(vec![ins(2, ERROR, ERROR, [0; 8])]));
        let mut callbacks = vec![
            ObjectFunction {
                condition: 0,
                action: 0,
            };
            9
        ];
        callbacks[1].action = 4096;
        callbacks[8].action = 4098;
        fx.program
            .libraries
            .get_mut(&LAMP_FILE)
            .unwrap()
            .callbacks
            .insert(128, callbacks);

        let mut partner = fx.lamp().clone();
        partner.id = 90;
        partner.position = [32, 32, 1];
        partner.data[2] = 0;
        partner.slots.truncate(1);
        partner.multipart = Some(Multipart {
            parent: LAMP as i16,
            group: [1, 0, 0],
            offset: [1, 0, 0],
        });
        fx.world.objects.insert(90, partner);
        fx.world.order.push(90);
        fx.world.next_parts.insert(LAMP, 90);

        let lamp = fx.lamp_mut();
        lamp.set_countdown(5);
        lamp.position = [-16, -16, 1];
        lamp.data[2] = 0;
        lamp.slots.truncate(1);
        lamp.multipart = Some(Multipart {
            parent: 0,
            group: [0, 0, 0],
            offset: [0, 0, 0],
        });
        fx.world.removals = vec![LAMP];
        fx
    };

    // Strict Mode fails when `leave_group` -> `find_neighbours(90)` runs the
    // faulting DYNAMIC_GROUP callback on partner 90.
    let mut strict = make_dyn_fx();
    assert!(update(&mut strict, &mut answers(&[])).is_err());

    // Play Mode catches the `leave_group` fault (recorded as Recovery::Reset
    // because LAMP is unlinked and then removed normally), does not delete
    // surviving partner tile 90, removes LAMP, and keeps ticking!
    let mut played = make_dyn_fx();
    let mut play = Play::default();
    let u1 = play_update(&mut played, &mut play);
    assert!(u1.full);
    assert_eq!(u1.removed, vec![LAMP]);
    assert_eq!(u1.errors.len(), 1);
    assert_eq!(u1.errors[0].recovery, Recovery::Reset);
    assert!(!played.world.objects.contains_key(&LAMP));
    assert!(played.world.objects.contains_key(&90));
    assert_eq!(
        played.world.objects[&90].multipart.as_ref().unwrap().parent,
        0
    );
    let u2 = play_update(&mut played, &mut play);
    assert!(u2.full && u2.errors.is_empty());
}

#[test]
fn play_mode_forced_removal_preserves_promoted_tile_neighbor_after_move_tree_fault() {
    use sims_formats::saved_objects::Slot;

    let mut fx = lot(sleeper());
    // Give LAMP's definition a non-zero inset (word 24 = 4) so `move_tree`
    // rejects diagonal facing 1 (`Trap::Unsupported`) AFTER `take_off_one` has
    // already promoted slot-0 neighbor `B` to the grid head of tile (2, 3, 1)!
    fx.program
        .libraries
        .get_mut(&LAMP_FILE)
        .unwrap()
        .definitions
        .get_mut(&128)
        .unwrap()[24] = 4;

    // Make sure the lot has an object grid (array 3, 64*64*2 bytes) and rooms.
    fx.world.size = 64;
    let empty_grid = || sims_formats::lot::Array {
        width: 64,
        height: 64,
        stride: 2,
        encoding: 0,
        data: vec![0; 64 * 64 * 2],
    };
    fx.world.arrays.insert(3, empty_grid());
    fx.world.arrays.insert(103, empty_grid());
    // Ensure room_at(OFF_LOT) succeeds so `move_tree` reaches `footprint` and
    // faults on diagonal facing with insets (after `relink` promoted `B`).
    fx.world.rooms.map = [vec![0; 64 * 64], vec![0; 64 * 64]];
    fx.world.rooms.rooms.insert(
        0,
        sims_sim::rooms::Room {
            outdoors: true,
            ..Default::default()
        },
    );

    let tile_pos = [2 * 16 + 8, 3 * 16 + 8, 1];
    let cell = ((2 * 64 + 3) * 2) as usize;
    let b_id = 90u16;
    let mut b = fx.lamp().clone();
    b.id = b_id;
    b.position = tile_pos;
    b.data[1] = 0; // valid cardinal facing
    b.data[2] = 0;
    b.data[3] = -1;
    b.data[14] = 0; // slot 0 behind LAMP
    b.data[26] = LAMP as i16;
    b.slots = vec![Slot {
        unknown: 0,
        object: 0,
    }];
    b.set_countdown(5);
    fx.world.objects.insert(b_id, b);
    fx.world.order.push(b_id);

    let lamp = fx.lamp_mut();
    lamp.set_countdown(5);
    lamp.position = tile_pos;
    lamp.data[1] = 10; // facing > 9 -> `footprint` faults with insets after B is promoted!
    lamp.data[2] = 0;
    lamp.data[3] = -1;
    lamp.data[14] = -1;
    lamp.data[26] = 0;
    lamp.slots = vec![Slot {
        unknown: 0,
        object: b_id as i16,
    }];
    // Grid cell (2, 3, 1) holds LAMP as head, with B behind it in slot 0.
    fx.world.arrays.get_mut(&3).unwrap().data[cell..cell + 2].copy_from_slice(&LAMP.to_le_bytes());
    fx.world.removals = vec![LAMP];

    let mut play = Play::default();
    let u1 = play_update(&mut fx, &mut play);
    assert!(u1.full);
    assert_eq!(u1.removed, vec![LAMP]);
    assert_eq!(u1.errors.len(), 1);
    assert_eq!(u1.errors[0].recovery, Recovery::Parked(None));
    assert!(!fx.world.objects.contains_key(&LAMP));
    assert!(fx.world.objects.contains_key(&b_id));
    // Crucial: B was promoted to the grid cell by `take_off_one`'s `relink`
    // before `move_tree` faulted, and `force_remove_object` MUST NOT erase B!
    let head_bytes: [u8; 2] = fx.world.arrays[&3].data[cell..cell + 2].try_into().unwrap();
    assert_eq!(
        u16::from_le_bytes(head_bytes),
        b_id,
        "surviving neighbor {b_id} must remain grid head after forced removal of {LAMP}"
    );
    assert_eq!(fx.world.objects[&b_id].data[26], 0);
    assert_eq!(fx.world.objects[&b_id].data[14], -1);
}

#[test]
fn play_mode_failed_dynamic_unlink_repairs_surviving_tiles_when_intermediate_missing() {
    use sims_formats::execution::ObjectFunction;
    use sims_formats::saved_objects::Multipart;

    let mut fx = lot(sleeper());
    fx.world.size = 64;
    let empty_grid = || sims_formats::lot::Array {
        width: 64,
        height: 64,
        stride: 2,
        encoding: 0,
        data: vec![0; 64 * 64 * 2],
    };
    fx.world.arrays.insert(3, empty_grid());
    fx.world.arrays.insert(103, empty_grid());
    fx.world.rooms.map = [vec![0; 64 * 64], vec![0; 64 * 64]];
    fx.world.rooms.rooms.insert(
        0,
        sims_sim::rooms::Room {
            outdoors: true,
            ..Default::default()
        },
    );
    let lib = fx.program.libraries.get_mut(&LAMP_FILE).unwrap();
    lib.definitions.get_mut(&128).unwrap()[10] = 1;
    let mut master_def = lib.definitions[&128];
    master_def[11] = -1;
    lib.definitions.insert(129, master_def);
    lib.definition_order.push(129);
    let mut callbacks = vec![
        ObjectFunction {
            condition: 0,
            action: 0,
        };
        9
    ];
    callbacks[1].action = 4096;
    callbacks[8].action = 4096; // valid DYNAMIC_GROUP callback
    lib.callbacks.insert(128, callbacks);

    // Damaged chain: LAMP (A) -> 91 (B, MISSING!) -> 92 (C, live with C.parent = LAMP).
    let mut c = fx.lamp().clone();
    c.id = 92;
    c.position = [-16, -16, 1];
    c.data[2] = 0;
    c.slots.truncate(1);
    c.set_countdown(5);
    c.multipart = Some(Multipart {
        parent: LAMP as i16,
        group: [2, 0, 0],
        offset: [2, 0, 0],
    });
    fx.world.objects.insert(92, c);
    fx.world.order.push(92);
    fx.world.next_parts.insert(LAMP, 91);
    fx.world.next_parts.insert(91, 92);

    let lamp = fx.lamp_mut();
    lamp.set_countdown(5);
    lamp.position = [-16, -16, 1];
    lamp.data[2] = 0;
    lamp.slots.truncate(1);
    lamp.multipart = Some(Multipart {
        parent: 0,
        group: [0, 0, 0],
        offset: [0, 0, 0],
    });
    fx.world.removals = vec![LAMP];

    let mut play = Play::default();
    let u1 = play_update(&mut fx, &mut play);
    assert!(u1.full);
    assert_eq!(u1.removed, vec![LAMP]);
    assert_eq!(u1.errors.len(), 1);
    assert_eq!(u1.errors[0].recovery, Recovery::Reset);
    assert!(!fx.world.objects.contains_key(&LAMP));
    assert!(fx.world.objects.contains_key(&92));
    // B -> C must be cleaned out and C promoted to standalone lead (`parent == 0`).
    assert!(!fx.world.next_parts.contains_key(&91));
    assert!(!fx.world.next_parts.contains_key(&LAMP));
    assert_eq!(fx.world.objects[&92].multipart.as_ref().unwrap().parent, 0);
    let u2 = play_update(&mut fx, &mut play);
    assert!(u2.full && u2.errors.is_empty());
}

#[test]
fn play_mode_cyclic_adjacent_tile_does_not_hang_dynamic_group_removal() {
    use sims_formats::execution::ObjectFunction;
    use sims_formats::saved_objects::{Multipart, Slot};

    let mut fx = lot(sleeper());
    fx.world.size = 64;
    fx.world.arrays.insert(
        3,
        sims_formats::lot::Array {
            width: 64,
            height: 64,
            stride: 2,
            encoding: 0,
            data: vec![0; 64 * 64 * 2],
        },
    );
    fx.program
        .libraries
        .get_mut(&LAMP_FILE)
        .unwrap()
        .definitions
        .get_mut(&128)
        .unwrap()[10] = 1;
    let mut callbacks = vec![
        ObjectFunction {
            condition: 0,
            action: 0,
        };
        9
    ];
    callbacks[1].action = 4096;
    callbacks[8].action = 4096;
    fx.program
        .libraries
        .get_mut(&LAMP_FILE)
        .unwrap()
        .callbacks
        .insert(128, callbacks);

    // Put surviving dynamic-group partner 90 on tile (4, 4, 1) and two
    // cyclically-linked objects (95 <-> 96 in slot 0) on adjacent tile (4, 3, 1).
    let mut partner = fx.lamp().clone();
    partner.id = 90;
    partner.position = [4 * 16 + 8, 4 * 16 + 8, 1];
    partner.data[1] = 0;
    partner.data[2] = 0;
    partner.data[14] = -1;
    partner.data[26] = 0;
    partner.slots = vec![Slot {
        unknown: 0,
        object: 0,
    }];
    partner.set_countdown(5);
    partner.multipart = Some(Multipart {
        parent: LAMP as i16,
        group: [1, 0, 0],
        offset: [1, 0, 0],
    });
    let mut x = fx.lamp().clone();
    x.id = 95;
    x.position = [4 * 16 + 8, 3 * 16 + 8, 1];
    x.data[14] = -1;
    x.data[26] = 0;
    x.slots = vec![Slot {
        unknown: 0,
        object: 96,
    }];
    x.set_countdown(5);
    let mut y = fx.lamp().clone();
    y.id = 96;
    y.position = [4 * 16 + 8, 3 * 16 + 8, 1];
    y.data[14] = 0;
    y.data[26] = 95;
    y.slots = vec![Slot {
        unknown: 0,
        object: 95,
    }];
    y.set_countdown(5);

    fx.world.objects.insert(90, partner);
    fx.world.objects.insert(95, x);
    fx.world.objects.insert(96, y);
    fx.world.order.extend([90, 95, 96]);
    fx.world.next_parts.insert(LAMP, 90);
    let adj_cell = ((4 * 64 + 3) * 2) as usize;
    fx.world.arrays.get_mut(&3).unwrap().data[adj_cell..adj_cell + 2]
        .copy_from_slice(&95u16.to_le_bytes());

    let lamp = fx.lamp_mut();
    lamp.set_countdown(5);
    lamp.position = [-16, -16, 1];
    lamp.data[2] = 0;
    lamp.slots.truncate(1);
    lamp.multipart = Some(Multipart {
        parent: 0,
        group: [0, 0, 0],
        offset: [0, 0, 0],
    });
    fx.world.removals = vec![LAMP];

    // Must terminate immediately (no infinite loop!) and catch the cycle fault.
    let mut play = Play::default();
    let u1 = play_update(&mut fx, &mut play);
    assert!(u1.full);
    assert_eq!(u1.removed, vec![LAMP]);
    assert!(
        u1.errors.iter().any(|e| matches!(
            e.fault.trap,
            Trap::Invalid(ref msg) if msg.contains("loop")
        ) && e.recovery == Recovery::Reset),
        "expected cycle Trap::Invalid with Recovery::Reset, got {:?}",
        u1.errors
    );
    assert!(!fx.world.objects.contains_key(&LAMP));
}

#[test]
fn play_mode_non_modal_choice_dialog_advances_caller_on_next_tick() {
    // LAMP tree:
    //   0: Dialog(kind = 0, flags = 0x80 => non-modal, !informs) -> true: 1, false: ERROR
    //   1: temps[0] := 42 -> 2
    //   2: Sleep(param 0) -> 2
    let non_modal_choice = [0, 0, 0, 0, 0, 0, 0, 0x80];
    let mut fx = lot(tree(vec![
        ins(36, 1, ERROR, non_modal_choice),
        assign(scope::TEMPS, 0, scope::LITERAL, 42, 2),
        ins(0, 2, ERROR, [0, 0, 0, 0, 0, 0, 0, 0]),
    ]));

    let mut play = Play::default();
    // Tick 1: LAMP yields on the non-modal dialog; end_of_update records ONE
    // Unsupported error and marks the record `FIRST` (2) so LAMP can consume it.
    let u1 = play_update(&mut fx, &mut play);
    assert!(u1.full);
    assert_eq!(u1.errors.len(), 1);
    assert_eq!(u1.errors[0].recovery, Recovery::Reset);
    assert_eq!(fx.world.dialogs.records.len(), 1);
    assert_eq!(fx.world.dialogs.records[0].status, 2); // FIRST

    // Tick 2: LAMP consumes `FIRST`, branches true, writes temps[0] = 42, and
    // marks the record `DONE` (cleaned up at end_of_update); zero new errors!
    let u2 = play_update(&mut fx, &mut play);
    assert!(u2.full, "{:?}", u2.errors);
    assert!(u2.errors.is_empty(), "{:?}", u2.errors);
    assert_eq!(fx.lamp().temps[0], 42);
    assert!(fx.world.dialogs.records.is_empty());
}

#[test]
fn play_mode_cleans_orphaned_dialogs_from_held_children_and_removal_callbacks() {
    use sims_formats::execution::ObjectFunction;
    use sims_formats::saved_objects::Slot;

    let mut fx = lot(sleeper());
    fx.world.size = 64;
    let empty_grid = || sims_formats::lot::Array {
        width: 64,
        height: 64,
        stride: 2,
        encoding: 0,
        data: vec![0; 64 * 64 * 2],
    };
    fx.world.arrays.insert(3, empty_grid());
    fx.world.arrays.insert(103, empty_grid());
    fx.world.rooms.map = [vec![0; 64 * 64], vec![0; 64 * 64]];
    fx.world.rooms.rooms.insert(
        0,
        sims_sim::rooms::Room {
            outdoors: true,
            ..Default::default()
        },
    );
    // LAMP's Reset tree (4097) posts an informing dialog owned by LAMP itself
    // (operand 7 bit 0 = 1) and succeeds cleanly (`RETURN_TRUE`).
    fx.add(
        LAMP_FILE,
        4097,
        tree(vec![ins(36, RETURN_TRUE, ERROR, [0, 0, 0, 0, 0, 0, 0, 1])]),
    );
    let callbacks = [0, 4096, 0, 4097].map(|action| ObjectFunction {
        condition: 0,
        action,
    });
    fx.program
        .libraries
        .get_mut(&LAMP_FILE)
        .unwrap()
        .callbacks
        .insert(128, callbacks.to_vec());

    // Held child 91 sits in LAMP's slot 1 and owns an unanswered NEW dialog.
    let mut child = fx.lamp().clone();
    child.id = 91;
    child.position = [-16, -16, 1];
    child.data[2] = LAMP as i16;
    child.data[3] = 0;
    child.data[14] = 1;
    child.data[26] = LAMP as i16;
    child.slots = vec![Slot {
        unknown: 0,
        object: 0,
    }];
    child.set_countdown(5);
    fx.world.objects.insert(91, child);
    fx.world.order.push(91);

    let lamp = fx.lamp_mut();
    lamp.set_countdown(5);
    lamp.position = [-16, -16, 1];
    lamp.data[2] = 0;
    lamp.slots = vec![
        Slot {
            unknown: 0,
            object: 0,
        },
        Slot {
            unknown: 0,
            object: 91,
        },
    ];
    // Pre-existing SHOWN dialog on held child 91 (not LAMP!) plus an already-
    // orphaned dialog on non-existent object 999.
    fx.world.dialogs.records.push(Dialog {
        caller: 91,
        operands: dialog(0, 0),
        status: 1, // SHOWN
    });
    fx.world.dialogs.records.push(Dialog {
        caller: 999,
        operands: dialog(0, 0),
        status: 0, // NEW
    });
    fx.world.removals = vec![LAMP];

    let mut play = Play::default();
    let u1 = play_update(&mut fx, &mut play);
    assert!(u1.full);
    assert!(u1.errors.is_empty(), "{:?}", u1.errors);
    assert!(!fx.world.objects.contains_key(&LAMP));
    assert!(!fx.world.objects.contains_key(&91));
    // All orphaned dialogs (from 999, held child 91, and LAMP's own RESET
    // callback) must be gone so the next tick never prompts for a dead object.
    assert!(
        fx.world.dialogs.records.is_empty(),
        "orphaned dialogs survived: {:?}",
        fx.world.dialogs.records
    );
    let u2 = play_update(&mut fx, &mut play);
    assert!(u2.full && u2.errors.is_empty());
}

#[test]
fn play_mode_reselection_fault_reconciles_global_3_with_selected_sim() {
    use sims_formats::saved_objects::Slot;

    let mut fx = lot(sleeper());
    fx.world.size = 64;
    fx.world.globals[9] = 1; // FAMILY = 1
    fx.world.globals[3] = 0; // SELECTED = 0
    fx.world.families.push(sims_sim::neighbors::Family {
        id: 1,
        house: 1,
        members: vec![SIM_GUID],
        friends: 0,
        stale: false,
    });
    // Put indicator objects 383 and 384 in the world; SIM stands off-lot so
    // `move_indicators` skips head-slot placement and faults on the missing
    // second indicator its date asks for (`person::DATE = 77` below 0,
    // 0x446996) AFTER `select` already set `SIM.selected = true`!
    let mut ind0 = fx.lamp().clone();
    ind0.id = 383;
    ind0.guid = sims_vm::INDICATORS[0];
    ind0.position = [-16, -16, 1];
    ind0.data[2] = 0;
    ind0.slots = vec![Slot {
        unknown: 0,
        object: 0,
    }];
    ind0.set_countdown(5);
    let mut ind1 = ind0.clone();
    ind1.id = 384;
    ind1.guid = sims_vm::INDICATORS[1];
    fx.world.objects.insert(383, ind0);
    fx.world.objects.insert(384, ind1);
    fx.world.order.extend([383, 384]);
    // The second indicator is gone from the lot, but the interface still
    // names it.
    fx.world.indicators = [383, 385];

    let sim = fx.sim_mut();
    sim.selected = false;
    sim.position = [-16, -16, 1];
    sim.person.as_mut().unwrap().data[77] = -1; // a date -> the missing indicator faults

    let mut play = Play::default();
    let u1 = play_update(&mut fx, &mut play);
    assert!(u1.full);
    assert_eq!(u1.errors.len(), 1);
    assert_eq!(u1.errors[0].recovery, Recovery::Reset);
    assert!(fx.world.objects[&SIM].selected);
    // Global 3 MUST be reconciled with `selected(world)` == Some(SIM) so it is
    // not left out of sync at 0!
    assert_eq!(fx.world.globals[3], SIM as i16);

    // Next tick sees SIM selected (resident) and does not re-fault on reselect!
    let u2 = play_update(&mut fx, &mut play);
    assert!(u2.full && u2.errors.is_empty(), "{:?}", u2.errors);
}

#[test]
fn removal_scrubs_route_door_and_seat_candidates_preventing_id_reuse() {
    use sims_formats::execution::ObjectFunction;
    use sims_sim::route::{Candidate, Context, Route, RouteSlot};

    let mut fx = lot(sleeper());
    fx.world.size = 64;
    // Give SIM_FILE a valid Main callback (4096) so SIM restarts at Main in
    // place while Sim 70 (whose definition has Main == 0) is parked instead.
    fx.program
        .libraries
        .get_mut(&SIM_FILE)
        .unwrap()
        .callbacks
        .insert(
            128,
            [0, 4096]
                .map(|action| ObjectFunction {
                    condition: 0,
                    action,
                })
                .to_vec(),
        );
    // LAMP's Reset tree faults so Play Mode exercises `force_remove_object`.
    fx.add(LAMP_FILE, 4097, tree(vec![ins(2, ERROR, ERROR, [0; 8])]));
    let callbacks = [0, 4096, 0, 4097].map(|action| ObjectFunction {
        condition: 0,
        action,
    });
    fx.program
        .libraries
        .get_mut(&LAMP_FILE)
        .unwrap()
        .callbacks
        .insert(128, callbacks.to_vec());

    // Add a surviving on-lot route target (88) and a door (89) to remove
    // alongside faulting seat LAMP (1), ensuring neither holds SIM in its slots.
    let mut target = fx.lamp().clone();
    target.id = 88;
    target.position = [32, 32, 1];
    target.data[2] = 0;
    target.slots.truncate(1);
    target.set_countdown(5);
    let mut door = target.clone();
    door.id = 89;
    door.position = [-16, -16, 1];
    fx.world.objects.insert(88, target);
    fx.world.objects.insert(89, door);
    fx.world.order.extend([88, 89]);

    let lamp = fx.lamp_mut();
    lamp.set_countdown(5);
    lamp.position = [-16, -16, 1];
    lamp.data[2] = 0;
    lamp.slots.truncate(1);

    let seat_cand = Candidate {
        position: [16, 16],
        score: 10,
        word_c: LAMP as i16,
        word_e: None,
    };
    let stand_cand = Candidate {
        position: [32, 32],
        score: 5,
        word_c: 0,
        word_e: None,
    };
    fx.world.routing.routes.insert(
        SIM,
        Route {
            contexts: vec![Context {
                candidates: vec![seat_cand, stand_cand],
                slot: RouteSlot {
                    offset: [0; 3],
                    number: 1,
                    standing: 1,
                    sitting: 1,
                    ground: 0,
                    flags: 0,
                    snap_target: 0,
                    proximity: [0; 3],
                    gradient: 0,
                    facing: 0,
                    resolution: 0,
                },
                target: 88,
                person: SIM,
                chosen: 0, // chose seat LAMP!
                best: 10,
                word_58: 0,
                time: 0,
                saved: [0; 2],
                word_68: i32::from(89u16), // door 89
                start: [0; 3],
                byte_78: true,
                objects: [LAMP, 0],
                byte_84: false,
                result: 0,
                word_8c: 0,
                byte_8e: false,
                word_94: 0,
                limit: 0x30,
                action: 0,
            }],
            path: vec![[0, 0], [16, 16]],
        },
    );
    fx.world.removals = vec![LAMP, 89];

    let mut play = Play::default();
    let u1 = play_update(&mut fx, &mut play);
    assert!(u1.full);
    assert!(!fx.world.objects.contains_key(&LAMP));
    assert!(!fx.world.objects.contains_key(&89));

    // Immediately reuse ID `LAMP` for a brand-new replacement object.
    let mut replacement = fx.world.objects[&88].clone();
    replacement.id = LAMP;
    fx.world.objects.insert(LAMP, replacement);

    let route = &fx.world.routing.routes[&SIM];
    let ctx = &route.contexts[0];
    assert_eq!(ctx.target, 88, "surviving target 88 stays intact");
    assert_eq!(ctx.word_68, 0, "deleted door 89 must be cleared");
    assert_eq!(ctx.objects[0], 0, "deleted objects[0] must be cleared");
    assert!(
        ctx.candidates.iter().all(|c| c.word_c != LAMP as i16),
        "deleted seat candidate must be removed before ID reuse"
    );
    assert_eq!(ctx.candidates.len(), 1);
    assert_eq!(ctx.chosen, -1, "invalidated chosen seat must reset to -1");
    assert_eq!(
        route.path,
        vec![[0, 0], [16, 16]],
        "active path coordinates must stay intact so UNDER_WAY does not fault at 0x45b2f7"
    );

    // Resuming an UNDER_WAY Go To Routing Slot frame after losing the chosen
    // seat must NOT fault with `0x45b2f7 with no path`.
    fx.add(
        SIM_FILE,
        4096,
        tree(vec![
            ins(45, 1, 1, [0, 0, 0, 0, 0, 0, 0, 0]),
            ins(0, 1, ERROR, [0; 8]),
        ]),
    );
    let sim = fx.sim_mut();
    sim.set_countdown(-1);
    sim.stack[0].tree = 4096;
    sim.stack[0].node = 0;
    sim.stack[0].stack_object = 88;
    sim.stack[0].primitive_state = 3; // UNDER_WAY
    let person = sim.person.as_mut().unwrap();
    person.data.fill(0);
    person.data[29] = 1;
    person.data[32] = 1;
    person.active.flags = 0;
    let mut animator = sims_sim::animator::Animator::new(0);
    animator.arrival = 1; // still walking
    fx.world.animators.insert(SIM, animator);
    // Run Go To Routing Slot directly via Strict Mode update: must stay in
    // UNDER_WAY (3) and NOT fault with `0x45b2f7 with no path`!
    let mut strict_fx = fx;
    let u2 = update(&mut strict_fx, &mut answers(&[])).unwrap();
    assert!(u2.full);
    assert!(u2.errors.is_empty());
    assert_eq!(
        strict_fx.world.objects[&SIM].stack[0].primitive_state, 3,
        "UNDER_WAY route must continue cleanly after chosen seat removal"
    );
}

#[test]
fn play_mode_failed_dynamic_unlink_of_non_lead_repairs_missing_intermediate_and_preserves_unrelated_groups()
 {
    use sims_formats::execution::ObjectFunction;
    use sims_formats::saved_objects::Multipart;

    let mut fx = lot(sleeper());
    fx.world.size = 64;
    let empty_grid = || sims_formats::lot::Array {
        width: 64,
        height: 64,
        stride: 2,
        encoding: 0,
        data: vec![0; 64 * 64 * 2],
    };
    fx.world.arrays.insert(3, empty_grid());
    fx.world.arrays.insert(103, empty_grid());
    fx.world.rooms.map = [vec![0; 64 * 64], vec![0; 64 * 64]];
    fx.world.rooms.rooms.insert(
        0,
        sims_sim::rooms::Room {
            outdoors: true,
            ..Default::default()
        },
    );
    let lib = fx.program.libraries.get_mut(&LAMP_FILE).unwrap();
    lib.definitions.get_mut(&128).unwrap()[10] = 1;
    let mut master_def = lib.definitions[&128];
    master_def[11] = -1;
    lib.definitions.insert(129, master_def);
    lib.definition_order.push(129);
    let mut callbacks = vec![
        ObjectFunction {
            condition: 0,
            action: 0,
        };
        9
    ];
    callbacks[1].action = 4096;
    callbacks[8].action = 4096;
    lib.callbacks.insert(128, callbacks);

    // 1. Damaged chain: A (LAMP) -> B (91, MISSING!) -> C (92, non-lead with C.parent = A).
    // 2. Completely UNRELATED damaged tile U (95) whose recorded lead 999 is
    //    also missing; unlinking A or C must NEVER merge U into A/C's group!
    let mut c = fx.lamp().clone();
    c.id = 92;
    c.position = [-16, -16, 1];
    c.data[2] = 0;
    c.slots.truncate(1);
    c.set_countdown(5);
    c.multipart = Some(Multipart {
        parent: LAMP as i16,
        group: [2, 0, 0],
        offset: [2, 0, 0],
    });
    let mut unrelated = c.clone();
    unrelated.id = 95;
    unrelated.multipart = Some(Multipart {
        parent: 999, // unrelated missing lead
        group: [1, 0, 0],
        offset: [1, 0, 0],
    });
    fx.world.objects.insert(92, c);
    fx.world.objects.insert(95, unrelated);
    fx.world.order.extend([92, 95]);
    fx.world.next_parts.insert(LAMP, 91);
    fx.world.next_parts.insert(91, 92);

    let lamp = fx.lamp_mut();
    lamp.set_countdown(5);
    lamp.position = [-16, -16, 1];
    lamp.data[2] = 0;
    lamp.slots.truncate(1);
    lamp.multipart = Some(Multipart {
        parent: 0,
        group: [0, 0, 0],
        offset: [0, 0, 0],
    });
    // Queue non-lead C (92) first!
    fx.world.removals = vec![92];

    let mut play = Play::default();
    let u1 = play_update(&mut fx, &mut play);
    assert!(u1.full);
    assert_eq!(u1.removed, vec![92]);
    assert_eq!(u1.errors.len(), 1);
    assert_eq!(u1.errors[0].recovery, Recovery::Reset);
    assert!(!fx.world.objects.contains_key(&92));
    // Surviving lead A (LAMP) must have its broken A -> 91 -> 92 links completely
    // repaired so A is a clean standalone tile and unrelated tile 95 is untouched!
    assert!(fx.world.objects.contains_key(&LAMP));
    assert!(!fx.world.next_parts.contains_key(&LAMP));
    assert!(!fx.world.next_parts.contains_key(&91));
    assert_eq!(
        fx.world.objects[&LAMP].multipart.as_ref().unwrap().parent,
        0
    );
    assert_eq!(
        fx.world.objects[&95].multipart.as_ref().unwrap().parent,
        999,
        "unrelated multipart tile 95 must never be merged"
    );

    // Now also verify that unlinking a lead with a missing successor does NOT
    // sweep unrelated tile 95 into its surviving partner's group.
    let mut partner = fx.lamp().clone();
    partner.id = 93;
    partner.multipart = Some(Multipart {
        parent: LAMP as i16,
        group: [1, 0, 0],
        offset: [1, 0, 0],
    });
    fx.world.objects.insert(93, partner);
    fx.world.order.push(93);
    fx.world.next_parts.insert(LAMP, 94); // 94 is missing
    fx.world.next_parts.insert(94, 93);
    fx.world.removals = vec![LAMP];

    let u2 = play_update(&mut fx, &mut play);
    assert!(u2.full);
    assert!(!fx.world.objects.contains_key(&LAMP));
    assert_eq!(fx.world.objects[&93].multipart.as_ref().unwrap().parent, 0);
    assert!(!fx.world.next_parts.contains_key(&93));
    assert_eq!(
        fx.world.objects[&95].multipart.as_ref().unwrap().parent,
        999,
        "unrelated tile 95 must not be merged when lead LAMP is force-unlinked"
    );
}

#[test]
fn play_mode_multipart_fallback_preserves_new_object_reusing_deleted_part_id() {
    use sims_formats::execution::ObjectFunction;
    use sims_formats::saved_objects::Multipart;

    let mut fx = lot(sleeper());
    fx.world.size = 64;
    let empty_grid = || sims_formats::lot::Array {
        width: 64,
        height: 64,
        stride: 2,
        encoding: 0,
        data: vec![0; 64 * 64 * 2],
    };
    fx.world.arrays.insert(3, empty_grid());
    fx.world.arrays.insert(103, empty_grid());
    fx.world.rooms.map = [vec![0; 64 * 64], vec![0; 64 * 64]];
    fx.world.rooms.rooms.insert(
        0,
        sims_sim::rooms::Room {
            outdoors: true,
            ..Default::default()
        },
    );

    // Lead tile (LAMP = 1) has a clean no-op Reset tree (returns true) so
    // `delete_one(1)` deletes ID 1 completely.
    // Partner tile (90) uses a distinct GUID (0x4444) whose Reset tree creates
    // a brand-new single-tile object (GUID 0x5555, which the factory allocates
    // at lowest free ID = 1, reusing LAMP's ID!) and THEN faults with ERROR!
    let partner_guid = 0x4444u32;
    let fresh_guid = 0x5555u32;
    let fresh_bytes = fresh_guid.to_le_bytes();
    // Create New Object Instance (opcode 42), mode 6 (out of world, no
    // placement): operands[0..4] = GUID LE, operands[4] = 6.
    let create_fresh = [
        fresh_bytes[0],
        fresh_bytes[1],
        fresh_bytes[2],
        fresh_bytes[3],
        6,
        0,
        0,
        0,
    ];
    // Lead Reset: simple Return True (assign temp 0 = 1 -> RETURN_TRUE).
    fx.add(
        LAMP_FILE,
        4097,
        tree(vec![assign(
            scope::TEMPS,
            0,
            scope::LITERAL,
            1,
            RETURN_TRUE,
        )]),
    );
    // Fresh object's Main tree (4099): set param 0 = 5, then Sleep(param 0)
    // so `factory::create_single`'s first turn yields cleanly!
    fx.add(
        LAMP_FILE,
        4099,
        tree(vec![
            assign(scope::PARAMETERS, 0, scope::LITERAL, 5, 1),
            ins(0, 1, ERROR, [0; 8]),
        ]),
    );
    // Partner Reset (4098): Create 0x5555 (reuses ID 1!) -> then ERROR!
    fx.add(
        LAMP_FILE,
        4098,
        tree(vec![ins(42, ERROR, ERROR, create_fresh)]),
    );

    let lib = fx.program.libraries.get_mut(&LAMP_FILE).unwrap();
    lib.definitions.get_mut(&128).unwrap()[10] = 1;
    let mut master_def = lib.definitions[&128];
    master_def[11] = -1;
    lib.definitions.insert(129, master_def);
    lib.definition_order.push(129);

    let mut partner_def = definition(partner_guid, 4);
    partner_def[10] = 1;
    lib.definitions.insert(130, partner_def);
    lib.definition_order.push(130);

    let mut fresh_def = [0i16; sims_vm::DEFINITION_WORDS];
    fresh_def[9] = 4;
    fresh_def[14] = fresh_guid as u16 as i16;
    fresh_def[15] = (fresh_guid >> 16) as u16 as i16;
    lib.definitions.insert(131, fresh_def);
    lib.definition_order.push(131);

    lib.callbacks.insert(
        128,
        [0, 4096, 0, 4097]
            .map(|action| ObjectFunction {
                condition: 0,
                action,
            })
            .to_vec(),
    );
    lib.callbacks.insert(
        130,
        [0, 4096, 0, 4098]
            .map(|action| ObjectFunction {
                condition: 0,
                action,
            })
            .to_vec(),
    );
    lib.callbacks.insert(
        131,
        [0, 4099]
            .map(|action| ObjectFunction {
                condition: 0,
                action,
            })
            .to_vec(),
    );
    let partner_idx = fx.catalog.objects.len();
    fx.catalog.objects.push(sims_content::Object {
        guid: partner_guid,
        resource: LAMP_FILE,
        chunk: 130,
        kind: 4,
        master: 129,
    });
    fx.catalog.by_guid.insert(partner_guid, partner_idx);
    let fresh_idx = fx.catalog.objects.len();
    fx.catalog.objects.push(sims_content::Object {
        guid: fresh_guid,
        resource: LAMP_FILE,
        chunk: 131,
        kind: 4,
        master: 0,
    });
    fx.catalog.by_guid.insert(fresh_guid, fresh_idx);
    fx.world.types.insert(9, partner_guid);
    fx.world.types.insert(10, fresh_guid);

    let mut partner = fx.lamp().clone();
    partner.id = 90;
    partner.guid = partner_guid;
    partner.position = [-16, -16, 1];
    partner.data[2] = 0;
    partner.slots.truncate(1);
    partner.set_countdown(5);
    partner.multipart = Some(Multipart {
        parent: LAMP as i16,
        group: [1, 0, 0],
        offset: [1, 0, 0],
    });
    fx.world.objects.insert(90, partner);
    fx.world.order.push(90);
    fx.world.next_parts.insert(LAMP, 90);

    let lamp = fx.lamp_mut();
    lamp.set_countdown(5);
    lamp.position = [-16, -16, 1];
    lamp.data[2] = 0;
    lamp.slots.truncate(1);
    lamp.multipart = Some(Multipart {
        parent: 0,
        group: [0, 0, 0],
        offset: [0, 0, 0],
    });
    fx.world.removals = vec![LAMP];

    let mut play = Play::default();
    let u1 = play_update(&mut fx, &mut play);
    assert!(u1.full);
    assert_eq!(u1.errors.len(), 1);
    assert_eq!(u1.errors[0].recovery, Recovery::Parked(None));
    // Partner 90 was force-removed, while the brand-new object 0x5555 that
    // reused deleted lead ID 1 MUST SURVIVE!
    assert!(!fx.world.objects.contains_key(&90));
    assert_eq!(
        fx.world.objects.get(&LAMP).map(|o| o.guid),
        Some(fresh_guid),
        "brand-new object reusing already-deleted part ID {LAMP} must survive fallback; errors={:?}, objects={:?}",
        u1.errors,
        fx.world.objects.keys().collect::<Vec<_>>()
    );
}

#[test]
fn play_mode_force_removal_repairs_use_counts_callback_free_and_parks_sim_without_main() {
    use sims_formats::execution::ObjectFunction;
    use sims_formats::saved_objects::ObjectUse;

    let mut fx = lot(sleeper());
    // Give SIM_FILE a valid Main callback (4096) so SIM restarts at Main in
    // place while Sim 70 (whose definition has Main == 0) is parked instead.
    fx.program
        .libraries
        .get_mut(&SIM_FILE)
        .unwrap()
        .callbacks
        .insert(
            128,
            [0, 4096]
                .map(|action| ObjectFunction {
                    condition: 0,
                    action,
                })
                .to_vec(),
        );
    // LAMP's Reset tree faults so Play Mode exercises `force_remove_object`.
    fx.add(LAMP_FILE, 4097, tree(vec![ins(2, ERROR, ERROR, [0; 8])]));
    let callbacks = [0, 4096, 0, 4097].map(|action| ObjectFunction {
        condition: 0,
        action,
    });
    fx.program
        .libraries
        .get_mut(&LAMP_FILE)
        .unwrap()
        .callbacks
        .insert(128, callbacks.to_vec());

    // Surviving object OTHER (id 80) shares LAMP's faulting Reset tree! If
    // forced removal ever ran callbacks on OTHER, it would fault/mutate state.
    let mut other = fx.lamp().clone();
    other.id = 80;
    other.position = [-16, -16, 1];
    other.data[62] = 1; // USERS == 1 (used by SIM)
    other.set_countdown(5);
    fx.world.objects.insert(80, other);
    fx.world.order.push(80);

    // Surviving Sim `SIM` uses both LAMP and OTHER, and has LAMP on its stack.
    let lamp = fx.lamp_mut();
    lamp.set_countdown(5);
    lamp.position = [-16, -16, 1];
    lamp.data[2] = 0;
    lamp.slots.truncate(1);
    lamp.data[62] = 1;

    let sim = fx.sim_mut();
    sim.set_countdown(5);
    sim.stack[0].stack_object = LAMP as i16;
    sim.person.as_mut().unwrap().object_uses = vec![
        ObjectUse {
            target: LAMP as i16,
            stack_depth: 1,
            unknown: 0,
        },
        ObjectUse {
            target: 80,
            stack_depth: 1,
            unknown: 0,
        },
    ];

    // Second surviving Sim (id 70) whose definition has NO Main tree (callback
    // 1 == 0) and whose stack holds `stack_object = LAMP`.
    let no_main_guid = 0xDEAD_BEEFu32;
    let no_main_file = 99usize;
    let mut no_main_def = fx.program.libraries[&SIM_FILE].definitions[&128];
    no_main_def[9] = 2;
    let mut lib = sims_vm::Library::default();
    lib.definitions.insert(128, no_main_def);
    lib.callbacks.insert(
        128,
        vec![
            ObjectFunction {
                condition: 0,
                action: 0,
            };
            4
        ],
    );
    fx.program.libraries.insert(no_main_file, lib);
    let no_main_idx = fx.catalog.objects.len();
    fx.catalog.objects.push(sims_content::Object {
        guid: no_main_guid,
        resource: no_main_file,
        chunk: 128,
        kind: 2,
        master: 0,
    });
    fx.catalog.by_guid.insert(no_main_guid, no_main_idx);
    let mut sim_no_main = fx.world.objects[&SIM].clone();
    sim_no_main.id = 70;
    sim_no_main.guid = no_main_guid;
    sim_no_main.set_countdown(5);
    sim_no_main.set_flags(1, true); // READY
    sim_no_main.stack[0].stack_object = LAMP as i16;
    sim_no_main.person.as_mut().unwrap().object_uses.clear();
    fx.world.objects.insert(70, sim_no_main);
    fx.world.order.push(70);
    fx.world.people.push(70);

    fx.world.removals = vec![LAMP];

    let mut play = Play::default();
    let u1 = play_update(&mut fx, &mut play);
    assert!(u1.full);
    assert!(!fx.world.objects.contains_key(&LAMP));
    // 1. OTHER's USERS counter was repaired back to 0 without running callbacks.
    assert_eq!(fx.world.objects[&80].data[62], 0);
    assert!(u1.errors.iter().all(|e| e.object != 80));
    // 2. SIM's uses were cleared and its stack restarted cleanly at Main.
    assert!(
        fx.world.objects[&SIM]
            .person
            .as_ref()
            .unwrap()
            .object_uses
            .is_empty()
    );
    assert_eq!(fx.world.objects[&SIM].stack[0].stack_object, 0);
    // 3. Sim 70 (no Main tree) had its stale frame cleared, was parked
    // (READY bit 0 cleared), and recorded a secondary `Recovery::Parked(Some(_))`
    // fault in `update.errors`!
    assert!(fx.world.objects[&70].stack.is_empty());
    assert_eq!(fx.world.objects[&70].flags() & 1, 0);
    assert!(
        u1.errors
            .iter()
            .any(|e| e.object == 70 && matches!(e.recovery, Recovery::Parked(Some(_)))),
        "expected secondary Parked(Some(_)) fault for Sim 70, got {:?}",
        u1.errors
    );
    // Next tick runs cleanly with zero errors.
    let u2 = play_update(&mut fx, &mut play);
    assert!(u2.full && u2.errors.is_empty(), "{:?}", u2.errors);
}
