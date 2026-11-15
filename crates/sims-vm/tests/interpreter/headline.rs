//! Set Balloon/Headline (opcode 41, 0x44b724): which object it acts on,
//! that only an object with a headline is touched, and what showing and
//! clearing an icon keep. Synthetic; no game data.

use crate::support::*;
use sims_formats::execution::Slot;
use sims_sim::Headline;
use sims_vm::{Status, Trap};
use std::collections::BTreeMap;

const HEADLINE: u16 = 41;
/// The lamp's SLOT chunk.
const SLOTS: u16 = 300;

/// A SLOT entry of `kind` (1 makes a headline, 3 a routing slot).
fn slot(kind: u16) -> Slot {
    Slot {
        kind,
        offset: [0.0, 0.0, 4.0],
        standing: 0,
        sitting: 0,
        ground: 0,
        flags: 0,
        snap_target: -1,
        proximity: [0; 3],
        max_size: 100,
        unknown: 0,
        gradient: 0.0,
        height: 0,
        facing: -2,
        resolution: 16,
    }
}

/// Gives the lamp a SLOT with entries of these kinds.
fn lamp_slots(fx: &mut Fixture, kinds: &[u16]) {
    fx.words(LAMP_FILE)[20] = SLOTS as i16;
    fx.program
        .libraries
        .get_mut(&LAMP_FILE)
        .unwrap()
        .slots
        .insert(SLOTS, kinds.iter().map(|&k| slot(k)).collect());
}

/// A fixture whose headline sprites are these, by ID: (sprite, frames).
fn with_sprites(sprites: &[(u16, i32)]) -> Fixture {
    let mut fx = Fixture::new();
    fx.program.icon_frames = Some(sprites.iter().copied().collect());
    fx
}

/// The sprites most tests use: kind 5's index 8 (508) has 9 frames, kind
/// 4's (308) 6, and kind 5's index 9 (509) only 2.
fn fixture() -> Fixture {
    with_sprites(&[(508, 9), (308, 6), (509, 2)])
}

/// Set Balloon/Headline: the stack object's when `stack`, an icon of
/// `kind` and `index` for `duration` ticks (0 clears), with `options`.
fn icon(stack: bool, kind: u8, index: u8, duration: i16, options: u16) -> [u8; 8] {
    let [d0, d1] = duration.to_le_bytes();
    let [o0, o1] = options.to_le_bytes();
    [u8::from(stack), 0, index, kind, d0, d1, o0, o1]
}

/// An icon of kind 5 and index 8 for `duration` ticks, backdrop 3.
fn headline(stack: bool, duration: i16) -> [u8; 8] {
    icon(stack, 5, 8, duration, 3)
}

/// Object `me` runs one Set Balloon/Headline as its tree 4096, with the
/// stack object `stack`.
fn run(fx: &mut Fixture, me: u16, stack: i16, operands: [u8; 8]) -> Result<bool, sims_vm::Fault> {
    let file = if me == SIM { SIM_FILE } else { LAMP_FILE };
    fx.add(
        file,
        4096,
        tree(vec![ins(HEADLINE, RETURN_TRUE, RETURN_FALSE, operands)]),
    );
    fx.run_with(me, 4096, stack, &[], 100)
}

fn shown(priority: i32, duration: i32) -> Headline {
    Headline { priority, duration }
}

#[test]
fn an_object_with_no_headline_is_left_alone() {
    let mut fx = fixture();
    lamp_slots(&mut fx, &[3, 0]);
    assert_eq!(
        run(&mut fx, LAMP, SIM as i16, headline(false, -1)),
        Ok(true)
    );
    assert!(fx.world.headlines.is_empty());
}

#[test]
fn a_slot_of_kind_1_gives_an_object_a_headline() {
    let mut fx = fixture();
    lamp_slots(&mut fx, &[3, 0, 1]);
    assert_eq!(
        run(&mut fx, LAMP, SIM as i16, headline(false, 30)),
        Ok(true)
    );
    assert_eq!(fx.world.headlines, BTreeMap::from([(LAMP, shown(1, 30))]));
    // A duration of 0 clears it.
    assert_eq!(run(&mut fx, LAMP, SIM as i16, headline(false, 0)), Ok(true));
    assert!(fx.world.headlines.is_empty());
}

#[test]
fn a_person_has_a_headline() {
    let mut fx = fixture();
    assert_eq!(
        run(&mut fx, SIM, LAMP as i16, headline(false, -1)),
        Ok(true)
    );
    assert_eq!(fx.world.headlines, BTreeMap::from([(SIM, shown(1, -1))]));
}

#[test]
fn byte_0_bit_0_acts_on_the_stack_object() {
    let mut fx = fixture();
    // The person has a headline and the lamp none.
    assert_eq!(run(&mut fx, SIM, LAMP as i16, headline(true, 5)), Ok(true));
    assert!(fx.world.headlines.is_empty());
    assert_eq!(run(&mut fx, LAMP, SIM as i16, headline(true, 5)), Ok(true));
    assert_eq!(fx.world.headlines, BTreeMap::from([(SIM, shown(1, 5))]));
    let fault = run(&mut fx, SIM, 999, headline(true, 0)).unwrap_err();
    assert_eq!(fault.trap.code(), Some(0x15), "{fault}");
}

#[test]
fn a_headline_of_a_higher_priority_stays() {
    let mut fx = fixture();
    // Kind 4 has priority 2.
    run(&mut fx, SIM, 0, icon(false, 4, 8, 50, 0)).unwrap();
    assert_eq!(fx.world.headlines[&SIM], shown(2, 50));
    // Neither another kind's icon nor its clear touches it.
    assert_eq!(run(&mut fx, SIM, 0, headline(false, 10)), Ok(true));
    assert_eq!(run(&mut fx, SIM, 0, headline(false, 0)), Ok(true));
    assert_eq!(fx.world.headlines[&SIM], shown(2, 50));
    // The same priority replaces it, and clears it.
    run(&mut fx, SIM, 0, icon(false, 4, 8, 7, 0)).unwrap();
    assert_eq!(fx.world.headlines[&SIM], shown(2, 7));
    run(&mut fx, SIM, 0, icon(false, 4, 8, 0, 0)).unwrap();
    assert!(fx.world.headlines.is_empty());
    // A lower priority is replaced by a higher one.
    run(&mut fx, SIM, 0, headline(false, 10)).unwrap();
    run(&mut fx, SIM, 0, icon(false, 4, 8, 20, 0)).unwrap();
    assert_eq!(fx.world.headlines[&SIM], shown(2, 20));
}

#[test]
fn an_icon_with_no_sprite_or_too_few_frames_shows_nothing() {
    let mut fx = fixture();
    // Index 9 of kind 5 has 2 frames: none in threes. Index 10 has no
    // sprite at all. Kind 11 names no sprite.
    for operands in [
        icon(false, 5, 9, 30, 0),
        icon(false, 5, 10, 30, 0),
        icon(false, 11, 8, 30, 0),
    ] {
        assert_eq!(run(&mut fx, SIM, 0, operands), Ok(true));
        assert!(fx.world.headlines.is_empty());
    }
    // A headline showing still takes the priority (written first).
    run(&mut fx, SIM, 0, headline(false, 40)).unwrap();
    run(&mut fx, SIM, 0, icon(false, 4, 9, 30, 0)).unwrap();
    assert_eq!(fx.world.headlines[&SIM], shown(2, 40));
}

#[test]
fn kind_0_has_its_own_table() {
    // Index 15 is sprite 401; index 19 and above name none.
    let mut fx = with_sprites(&[(401, 3), (219, 3), (19, 3)]);
    run(&mut fx, SIM, 0, icon(false, 0, 15, 6, 0)).unwrap();
    assert_eq!(fx.world.headlines[&SIM], shown(1, 6));
    run(&mut fx, SIM, 0, icon(false, 0, 15, 0, 0)).unwrap();
    run(&mut fx, SIM, 0, icon(false, 0, 19, 6, 0)).unwrap();
    assert!(fx.world.headlines.is_empty());
}

#[test]
fn option_bit_11_counts_the_duration_in_animation_loops() {
    let mut fx = fixture();
    // 9 frames are 3 in the headline's loop; each loop frame lasts 12
    // ticks: 5 loops are 180 ticks.
    run(&mut fx, SIM, 0, icon(false, 5, 8, 5, 0x800)).unwrap();
    assert_eq!(fx.world.headlines[&SIM], shown(1, 180));
    // -1 stays -1.
    run(&mut fx, SIM, 0, icon(false, 5, 8, -1, 0x800)).unwrap();
    assert_eq!(fx.world.headlines[&SIM], shown(1, -1));
    // Other options are drawn, and change nothing kept.
    run(&mut fx, SIM, 0, icon(false, 5, 8, 9, 0x0702)).unwrap();
    assert_eq!(fx.world.headlines[&SIM], shown(1, 9));
}

#[test]
fn option_bit_12_adds_my_temp_0_to_the_index() {
    let mut fx = fixture();
    // The lamp runs it on the person: the lamp's temp 0 counts.
    fx.lamp_mut().temps[0] = 1;
    fx.sim_mut().temps[0] = 5;
    run(&mut fx, LAMP, SIM as i16, icon(true, 5, 7, 10, 0x1000)).unwrap();
    assert_eq!(fx.world.headlines[&SIM], shown(1, 10));
    // Without the bit, index 7 has no sprite.
    run(&mut fx, LAMP, SIM as i16, icon(true, 5, 7, 0, 0)).unwrap();
    run(&mut fx, LAMP, SIM as i16, icon(true, 5, 7, 10, 0)).unwrap();
    assert!(fx.world.headlines.is_empty());
}

#[test]
fn what_is_not_ported_fails() {
    // A neighbor's catalog picture (kind 7, mode 1).
    let mut fx = fixture();
    let fault = run(&mut fx, SIM, 0, icon(false, 7, 1, 10, 0)).unwrap_err();
    assert!(
        matches!(&fault.trap, Trap::Unsupported(m) if m.contains("0x44b862")),
        "{fault}"
    );
    // Sprites that were not read.
    let mut fx = Fixture::new();
    let fault = run(&mut fx, SIM, 0, headline(false, 10)).unwrap_err();
    assert!(
        matches!(&fault.trap, Trap::Unsupported(m) if m.contains("Sprites.iff")),
        "{fault}"
    );
    // Clearing needs no sprite.
    assert_eq!(run(&mut fx, SIM, 0, headline(false, 0)), Ok(true));
    // A negative sprite ID.
    let mut fx = fixture();
    fx.sim_mut().temps[0] = -600;
    let fault = run(&mut fx, SIM, 0, icon(false, 5, 0, 10, 0x1000)).unwrap_err();
    assert!(
        matches!(&fault.trap, Trap::Unsupported(m) if m.contains("0x4c8deb")),
        "{fault}"
    );
}

#[test]
fn a_catalog_picture_of_the_stack_object_has_one_frame() {
    // Kind 7, mode 0: no sprite is needed, and the frame count is 1.
    let mut fx = Fixture::new();
    assert_eq!(
        run(&mut fx, SIM, LAMP as i16, icon(false, 7, 0, 10, 0)),
        Ok(true)
    );
    assert_eq!(fx.world.headlines, BTreeMap::from([(SIM, shown(1, 10))]));
    // Option bit 11 counts loops of one frame: 12 ticks each.
    assert_eq!(
        run(&mut fx, SIM, LAMP as i16, icon(false, 7, 0, 5, 0x800)),
        Ok(true)
    );
    assert_eq!(fx.world.headlines[&SIM], shown(1, 60));
    // -1 stays as it is.
    assert_eq!(
        run(&mut fx, SIM, LAMP as i16, icon(false, 7, 0, -1, 0x800)),
        Ok(true)
    );
    assert_eq!(fx.world.headlines[&SIM], shown(1, -1));
}

#[test]
fn a_catalog_picture_of_nothing_on_the_lot_shows_nothing() {
    let mut fx = Fixture::new();
    for stack in [0, -1, 999] {
        assert_eq!(run(&mut fx, SIM, stack, icon(false, 7, 0, 10, 0)), Ok(true));
    }
    assert!(fx.world.headlines.is_empty());
    // Any mode above 2 shows nothing either; the mode is byte 2 without
    // the temp that option bit 12 adds.
    assert_eq!(
        run(&mut fx, SIM, LAMP as i16, icon(false, 7, 3, 10, 0)),
        Ok(true)
    );
    fx.sim_mut().temps[0] = 2;
    assert_eq!(
        run(&mut fx, SIM, LAMP as i16, icon(false, 7, 0, 10, 0x1000)),
        Ok(true)
    );
    assert_eq!(fx.world.headlines, BTreeMap::from([(SIM, shown(1, 10))]));
    // Still showing, a headline of the same priority keeps its priority
    // when the picture is not found.
    assert_eq!(run(&mut fx, SIM, 999, icon(false, 7, 0, 20, 0)), Ok(true));
    assert_eq!(fx.world.headlines[&SIM], shown(1, 10));
}

/// Runs Set Balloon/Headline as the Sim's tree 4096, after an Expression
/// that sets local `local` to `value`.
fn run_with_local(
    fx: &mut Fixture,
    local: u8,
    value: i16,
    operands: [u8; 8],
) -> Result<bool, sims_vm::Fault> {
    fx.add(
        SIM_FILE,
        4096,
        tree(vec![
            assign(scope::LOCALS, i16::from(local), scope::LITERAL, value, 1),
            ins(HEADLINE, RETURN_TRUE, RETURN_FALSE, operands),
        ]),
    );
    fx.run_with(SIM, 4096, 0, &[], 100)
}

#[test]
fn a_catalog_picture_of_a_locals_object() {
    // Kind 7, mode 2: the local is operand word 0 shifted right by one;
    // bit 0 still picks the object the headline is on.
    let mut fx = Fixture::new();
    let mut operands = icon(false, 7, 2, 10, 0);
    operands[0] = 3 << 1;
    assert_eq!(run_with_local(&mut fx, 3, LAMP as i16, operands), Ok(true));
    assert_eq!(fx.world.headlines, BTreeMap::from([(SIM, shown(1, 10))]));
    // A local naming nothing on the lot shows nothing.
    let mut fx = Fixture::new();
    assert_eq!(run_with_local(&mut fx, 3, 999, operands), Ok(true));
    assert!(fx.world.headlines.is_empty());
    // A local past the frame's four: native error 0x33.
    let mut fx = Fixture::new();
    operands[0] = 4 << 1;
    let fault = run_with_local(&mut fx, 3, LAMP as i16, operands).unwrap_err();
    assert_eq!(fault.trap.code(), Some(0x33), "{fault}");
}

#[test]
fn a_catalog_picture_of_a_local_left_half_a_pointer_fails_loudly() {
    // The original reads the frame storage's word as it is (0x44b84b).
    let mut fx = Fixture::new();
    let mut storage = sims_sim::FrameStorage::default();
    storage.unknown.insert((0, 3));
    fx.world.frame_storage.insert(SIM, storage);
    let mut operands = icon(false, 7, 2, 10, 0);
    operands[0] = 3 << 1;
    let fault = run(&mut fx, SIM, 0, operands).unwrap_err();
    assert!(
        matches!(&fault.trap, Trap::Unsupported(m) if m.contains("half of a pointer")),
        "{fault}"
    );
    assert!(fx.world.headlines.is_empty());
}

#[test]
fn the_tick_counts_a_positive_duration_down() {
    let mut fx = fixture();
    fx.add(SIM_FILE, 4097, sleeper());
    fx.thread(SIM, 4097, &[100]);
    let tick = |fx: &mut Fixture| {
        let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
        interpreter.tick_object(&mut fx.world, SIM).unwrap();
    };
    fx.world.headlines.insert(SIM, shown(1, 2));
    tick(&mut fx);
    assert_eq!(fx.world.headlines[&SIM], shown(1, 1));
    tick(&mut fx);
    assert!(fx.world.headlines.is_empty());
    // A negative duration shows until it is cleared.
    fx.world.headlines.insert(SIM, shown(2, -1));
    tick(&mut fx);
    assert_eq!(fx.world.headlines[&SIM], shown(2, -1));
}

/// The lamp, with a headline, known to the scheduler and not locked out,
/// with a thread about to sleep `count` turns.
fn asleep(count: i16) -> Fixture {
    let mut fx = fixture();
    lamp_slots(&mut fx, &[1]);
    fx.add(LAMP_FILE, 4097, sleeper());
    fx.lamp_mut().registry_word = Some(0x0001_ffff);
    fx.lamp_mut().data[25] = 0;
    fx.thread(LAMP, 4097, &[count]);
    fx
}

fn turn(fx: &mut Fixture) -> Status {
    let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    interpreter
        .turn(&mut fx.world, LAMP, 100)
        .unwrap_or_else(|f| panic!("{f}"))
}

#[test]
fn an_object_showing_a_headline_counts_its_sleep_by_itself() {
    let mut fx = asleep(5);
    fx.world.headlines.insert(LAMP, shown(1, 30));
    assert_eq!(turn(&mut fx), Status::Yielded);
    assert_eq!(fx.lamp().countdown(), Some(-1));
    assert_eq!(fx.lamp().stack[0].parameters, [4]);
}

#[test]
fn showing_an_icon_wakes_the_object() {
    let mut fx = asleep(9);
    assert_eq!(turn(&mut fx), Status::Yielded);
    assert_eq!(fx.lamp().countdown(), Some(8));
    // Six ticks pass; the person shows an icon on the lamp.
    fx.lamp_mut().set_countdown(2);
    assert_eq!(run(&mut fx, SIM, LAMP as i16, headline(true, 30)), Ok(true));
    assert_eq!(fx.lamp().countdown(), Some(-2));
    assert_eq!(fx.lamp().stack[0].parameters, [2]);
    // Clearing it does not wake it.
    fx.lamp_mut().set_countdown(2);
    assert_eq!(run(&mut fx, SIM, LAMP as i16, headline(true, 0)), Ok(true));
    assert_eq!(fx.lamp().countdown(), Some(2));
}
