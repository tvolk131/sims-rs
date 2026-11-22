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
    // Walls with no floor: whether they count (0x4795c5) is not ported.
    let mut fx = before_midnight();
    fx.world.arrays.get_mut(&2).unwrap().data[(4 * 64 + 4) * 14] = 1;
    unsupported(&mut fx, "walls");
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
