//! Add/Change Action String (primitive 50, 0x44cc07) outside a pie menu's
//! listing and autonomy's check trees.

use crate::support::*;
use sims_vm::{Interpreter, Status};

/// The tree each test's object runs as its thread.
const TREE: u16 = 4200;

/// Operands naming string `index` (from 1) of string table 300.
fn action_string(index: u8) -> [u8; 8] {
    let mut operands = [0; 8];
    operands[..2].copy_from_slice(&300u16.to_le_bytes());
    operands[4] = index;
    operands
}

/// A lot where `object`'s thread runs Add/Change Action String on string
/// `index`. Both the Sim's and the lamp's files have table 300 with one
/// string, and the Sim's running action has flags 0x41.
fn named(object: u16, file: usize, index: u8) -> Fixture {
    let mut fx = Fixture::new();
    for resource in [SIM_FILE, LAMP_FILE] {
        fx.program
            .libraries
            .get_mut(&resource)
            .unwrap()
            .strings
            .insert(300, vec!["Eat".to_owned()]);
    }
    fx.add(
        file,
        TREE,
        tree(vec![ins(
            50,
            RETURN_TRUE,
            RETURN_FALSE,
            action_string(index),
        )]),
    );
    fx.sim_mut().person.as_mut().unwrap().active.flags = 0x41;
    fx.thread(object, TREE, &[]);
    fx
}

fn turn(fx: &mut Fixture, object: u16) -> Result<Status, sims_vm::Fault> {
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    interpreter.turn(&mut fx.world, object, 100)
}

fn active_flags(fx: &Fixture) -> u32 {
    fx.sim().person.as_ref().unwrap().active.flags
}

#[test]
fn a_persons_thread_names_its_running_action() {
    // 0x6c4d6c is set in a thread's run, so the string names the running
    // action (0x41f1ae), which sets its flag 0x80 and nothing else.
    let mut fx = named(SIM, SIM_FILE, 1);
    assert_eq!(turn(&mut fx, SIM), Ok(Status::Returned(true)));
    assert_eq!(active_flags(&fx), 0xc1);
}

#[test]
fn another_objects_thread_changes_nothing() {
    let mut fx = named(LAMP, LAMP_FILE, 1);
    let before = format!("{:?}", fx.sim());
    assert_eq!(turn(&mut fx, LAMP), Ok(Status::Returned(true)));
    assert_eq!(format!("{:?}", fx.sim()), before);
}

#[test]
fn a_missing_string_or_a_run_at_once_fails_loudly() {
    // A string the file lacks faults (`0x4704fa` returns null, passed to
    // `strlen` at `0x5b882d`): a script fault, not something unported.
    let mut fx = named(SIM, SIM_FILE, 2);
    let fault = turn(&mut fx, SIM).unwrap_err();
    assert!(fault.to_string().contains("0x4704fa"), "{fault}");
    assert!(!fault.trap.unported(), "{fault}");
    assert_eq!(active_flags(&fx), 0x41);
    // A tree run at once that is not a check tree.
    let mut fx = named(SIM, LAMP_FILE, 1);
    let fault = fx.run_with(LAMP, TREE, SIM as i16, &[], 100).unwrap_err();
    assert!(fault.to_string().contains("0x474a37"), "{fault}");
    assert!(fault.trap.unported(), "{fault}");
    assert_eq!(active_flags(&fx), 0x41);
}

#[test]
fn a_string_with_tokens_fails_loudly_before_anything_changes() {
    // The string is formatted (0x44bf1a) before 0x44cc9a looks at where
    // it goes, and a pass can raise an error (native 0x1c for a
    // non-person, 0x44c57b), so a string with "$" fails loudly in a
    // thread's run as well: the person's and another object's.
    for (object, file) in [(SIM, SIM_FILE), (LAMP, LAMP_FILE)] {
        let mut fx = named(object, file, 1);
        for resource in [SIM_FILE, LAMP_FILE] {
            fx.program
                .libraries
                .get_mut(&resource)
                .unwrap()
                .strings
                .insert(300, vec!["$TokenNameLocal:0".to_owned()]);
        }
        let fault = turn(&mut fx, object).unwrap_err();
        assert!(fault.to_string().contains("0x44bf1a"), "{fault}");
        assert_eq!(active_flags(&fx), 0x41);
    }

    // A non-person running `$Job` faults in the career pass (0x44ca27):
    // a genuine script fault (`!unported()`), not an unported feature.
    let mut fx = named(LAMP, LAMP_FILE, 1);
    fx.program
        .libraries
        .get_mut(&LAMP_FILE)
        .unwrap()
        .strings
        .insert(300, vec!["$Job".to_owned()]);
    let fault = turn(&mut fx, LAMP).unwrap_err();
    assert!(fault.to_string().contains("0x44ca27"), "{fault}");
    assert!(!fault.trap.unported(), "{fault}");

    // Even when the formatted text is discarded (thread run), `$TimeLocal`
    // (0x44c90b) and `$Job*` (0x44c9fc, 0x44ca0b) still read in-range
    // locals, so a local left as half of a pointer traps before `NAMED` is
    // set.
    for (text, unknown_local, addr) in [
        ("$TimeLocal:0", 0u32, "0x44c90b"),
        ("$Job:0:1", 0u32, "0x44c9fc"),
        ("$Job:0:1", 1u32, "0x44ca0b"),
        ("$JobOffer:0:1", 1u32, "0x44ca0b"),
        ("$JobDesc:0:1", 0u32, "0x44c9fc"),
    ] {
        let mut fx = named(SIM, SIM_FILE, 1);
        fx.sim_mut().stack[0].locals = vec![0; 4];
        fx.program.career_texts_end = true;
        fx.program
            .libraries
            .get_mut(&SIM_FILE)
            .unwrap()
            .strings
            .insert(300, vec![text.to_owned()]);
        fx.world.frame_storage.insert(
            SIM,
            sims_sim::FrameStorage {
                offsets: vec![0],
                unknown: [(0, unknown_local)].into(),
                ..Default::default()
            },
        );
        let fault = turn(&mut fx, SIM).unwrap_err();
        assert!(
            fault.to_string().contains(addr) && fault.to_string().contains("half of a pointer"),
            "{text} local {unknown_local}: {fault}"
        );
        assert_eq!(active_flags(&fx), 0x41, "{text}");
    }

    // In a thread's run (text dropped), `$Local`, `$Me` (CTSS slot 1), and
    // valid `$TimeLocal` / `$Job` (with `career_texts_end`) succeed and set
    // `NAMED` on a person's active interaction.
    let mut fx = named(SIM, SIM_FILE, 1);
    fx.program.career_texts_end = true;
    let def = fx.catalog.object(SIM_GUID).unwrap();
    let lib = fx.program.libraries.get_mut(&SIM_FILE).unwrap();
    lib.definitions.get_mut(&def.chunk).unwrap()[41] = 100;
    lib.catalog_strings.insert(
        100,
        vec![(1, "Mortimer".to_owned()), (1, "Description".to_owned())],
    );
    lib.strings.insert(
        300,
        vec!["$Me pays $$Local:0 at $TimeLocal:0 ($Job:0:1)".to_owned()],
    );
    assert_eq!(turn(&mut fx, SIM), Ok(Status::Returned(true)));
    assert_eq!(active_flags(&fx), 0xc1);
}

/// A lot whose Sim (selected if `selected`) runs an action with flags
/// 0xc1 and an icon, and queues one with flags 0x84 and an icon and one
/// with flags 0x80 and no icon.
fn listed(selected: bool) -> Fixture {
    let mut fx = Fixture::new();
    let sim = fx.world.objects.get_mut(&SIM).unwrap();
    sim.selected = selected;
    let person = sim.person.as_mut().unwrap();
    person.active.icon = LAMP as i16;
    person.active.flags = 0xc1;
    let mut queued = person.active.clone();
    queued.flags = 0x84;
    let mut iconless = person.active.clone();
    iconless.icon = 0;
    iconless.flags = 0x80;
    person.queue = vec![queued, iconless];
    fx
}

fn flags(fx: &Fixture) -> Vec<u32> {
    let person = fx.sim().person.as_ref().unwrap();
    std::iter::once(&person.active)
        .chain(&person.queue)
        .map(|a| a.flags)
        .collect()
}

fn frame(fx: &mut Fixture, mode: sims_vm::PanelMode) {
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    interpreter.draw_frame_in(&mut fx.world, 1, mode).unwrap();
}

#[test]
fn a_live_mode_frame_clears_the_named_flag_of_the_selected_sims_listed_actions() {
    // The queue panel lists the running action and each queued one with
    // an icon (0x4ea31c, 0x4ea203); listing works the name out (0x41f167),
    // which clears flag 0x80 and nothing else.
    let mut fx = listed(true);
    frame(&mut fx, sims_vm::PanelMode::Live);
    assert_eq!(flags(&fx), [0x41, 0x04, 0x80]);
    // `draw_frame` is a live-mode frame.
    let mut fx = listed(true);
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    interpreter.draw_frame(&mut fx.world, 1).unwrap();
    assert_eq!(flags(&fx), [0x41, 0x04, 0x80]);
}

#[test]
fn a_frame_lists_no_queue_outside_live_mode_or_without_a_selected_sim() {
    use sims_vm::PanelMode::*;
    for mode in [Build, Buy, Options, Camera] {
        let mut fx = listed(true);
        frame(&mut fx, mode);
        assert_eq!(flags(&fx), [0xc1, 0x84, 0x80], "{mode:?}");
    }
    let mut fx = listed(false);
    frame(&mut fx, Live);
    assert_eq!(flags(&fx), [0xc1, 0x84, 0x80]);
}
