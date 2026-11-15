//! Animate Sim (opcode 44, 0x58950b), Look Towards (opcode 22,
//! 0x45a98e) and a person's animator tick (0x58dc72). Synthetic; no game
//! data.

use crate::support::*;
use sims_content::{Clip, Event, Profile, Track};
use sims_core::F32;
use sims_sim::animator::{Base, LOADED, Layer, UNSET};
use sims_sim::motives::{Curve, Rates, Tuning};
use sims_vm::{Status, Trap};

/// The clip every test plays: 100 ms over 4 frames, one motion, no
/// events.
const CLIP: &str = "test-clip";
const SKILL: &str = "test-skill";
const HUNDRED: u32 = 0x42c8_0000;
/// The lot tick count the tests start at: the animator's clock reads
/// 50 per tick.
const TICKS: u32 = 10;

/// Animate Sim operands: the clip's number, where its name is listed,
/// the flags, the number of numbered events and the local to store in.
fn animate(id: i16, source: u8, flags: u8, count: i8, local: u8) -> [u8; 8] {
    let [a, b] = id.to_le_bytes();
    [a, b, local, 0, source, flags, count as u8, 0]
}

/// The Sim runs tree 4096 of its file: one Animate Sim, true and false
/// both returning, with one argument.
fn sim_animates(operands: [u8; 8]) -> Fixture {
    let mut fx = Fixture::new();
    fx.world.ticks = TICKS;
    fx.add(
        SIM_FILE,
        4096,
        shaped(
            0x8002,
            0,
            1,
            2,
            vec![ins(44, RETURN_TRUE, RETURN_FALSE, operands)],
        ),
    );
    // Clip number 1 of the global file's adult table (the fixture's Sim
    // is 174 by person word 58).
    fx.program
        .libraries
        .get_mut(&GLOBAL)
        .unwrap()
        .strings
        .insert(156, vec![String::new(), CLIP.into(), String::new()]);
    fx.program.animations.insert(
        "test.far",
        Clip {
            name: CLIP.into(),
            skill: SKILL.into(),
            duration: HUNDRED,
            distance: 0,
            moving: false,
            frames: 4,
            motions: 1,
            tracks: Vec::new(),
            channels: Vec::new(),
            samples: [0, 0],
        },
    );
    fx.program.animations.insert_samples(SKILL);
    fx.world.types.insert(8, SIM_GUID);
    // The animator the load leaves is to read the person's appearance
    // again, which the first clip does. This profile changes only person
    // words 22 and 23.
    fx.program.people.profiles.insert(
        SIM_GUID,
        Profile {
            gender: None,
            age: None,
            skin: None,
            tones: Vec::new(),
            rig: Default::default(),
        },
    );
    fx.thread(SIM, 4096, &[-7]);
    fx.sim_mut().stack[0].locals = vec![0, 0];
    fx
}

fn turn(fx: &mut Fixture) -> Result<Status, sims_vm::Fault> {
    let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    interpreter.turn(&mut fx.world, SIM, 100)
}

fn unsupported(result: Result<Status, sims_vm::Fault>) -> String {
    match result {
        Err(fault) => match fault.trap {
            Trap::Unsupported(message) => message,
            other => panic!("{other:?}"),
        },
        Ok(status) => panic!("{status:?}"),
    }
}

#[test]
fn the_first_clip_reads_the_appearance_again() {
    // 0x5893b3: looking the clip up reads the appearance if the animator
    // says to (+0xd8) and clears the flag.
    // A person's animator starts as LOADED, with the flag set.
    let loaded = LOADED;
    assert!(loaded.profile_stale);
    let mut fx = sim_animates(animate(1, 3, 0x08 | 0x40, 2, 0));
    fx.sim_mut().person.as_mut().unwrap().data[22] = 9;
    turn(&mut fx).unwrap();
    assert!(!fx.world.animators[&SIM].profile_stale);
    assert_eq!(fx.sim().person.as_ref().unwrap().data[22], 0);
    // Without a profile for the person's type, that fails.
    let mut fx = sim_animates(animate(1, 3, 0x08 | 0x40, 2, 0));
    fx.program.people.profiles.clear();
    assert!(unsupported(turn(&mut fx)).contains("no profile was supplied"));
}

#[test]
fn a_clip_starts_in_the_foreground_and_the_thread_yields() {
    let mut fx = sim_animates(animate(1, 3, 0x08 | 0x40, 2, 0));
    assert_eq!(turn(&mut fx).unwrap(), Status::Yielded);
    let a = &fx.world.animators[&SIM];
    assert!(a.playing);
    assert_eq!((a.state, a.seen, a.selection), (0, 0, -1));
    assert!(a.follow_rate && !a.slow);
    // The first foreground layer takes the raised priority, once.
    assert!(!a.promote && a.skin_dirty);
    let layer = a.foreground.as_ref().unwrap();
    assert_eq!(layer.clip, CLIP);
    assert_eq!(
        (layer.priority, layer.interruptible, layer.frames),
        (6, 1, 4)
    );
    assert_eq!((layer.current, layer.previous), (500, 500));
    assert_eq!(layer.duration.to_bits(), HUNDRED);
    assert_eq!(layer.speed.to_bits(), 0x3f80_0000);
    assert_eq!(layer.phase, F32::ZERO);
    // The frame stays on the instruction, to run it again.
    assert_eq!(fx.sim().stack[0].node, 0);
    // While it plays, each run yields.
    assert_eq!(turn(&mut fx).unwrap(), Status::Yielded);
}

#[test]
fn a_slow_clip_starts_on_the_slow_clock() {
    let mut fx = sim_animates(animate(1, 3, 0x80, 0, 0));
    turn(&mut fx).unwrap();
    let a = &fx.world.animators[&SIM];
    assert!(a.slow);
    assert_eq!(a.foreground.as_ref().unwrap().current, 420);
}

#[test]
fn queued_events_are_taken_in_order_and_unexpected_ones_dropped() {
    let mut fx = sim_animates(animate(1, 3, 0, 2, 0));
    turn(&mut fx).unwrap();
    // A numbered event that is not the next expected is dropped.
    fx.world.animators.get_mut(&SIM).unwrap().events = vec![1];
    assert_eq!(turn(&mut fx).unwrap(), Status::Yielded);
    assert!(fx.world.animators[&SIM].events.is_empty());
    // The expected one is stored in parameter 0, with the false branch.
    fx.world.animators.get_mut(&SIM).unwrap().events = vec![0, 150];
    assert_eq!(turn(&mut fx).unwrap(), Status::Returned(false));
    let a = &fx.world.animators[&SIM];
    assert_eq!((a.seen, &a.events[..]), (1, &[150][..]));
    // An event of 100 or more is stored without counting.
    fx.thread(SIM, 4096, &[-7]);
    fx.sim_mut().stack[0].locals = vec![0, 0];
    assert_eq!(turn(&mut fx).unwrap(), Status::Returned(false));
    assert_eq!(fx.world.animators[&SIM].seen, 1);
}

#[test]
fn an_event_can_go_to_a_local() {
    let mut fx = sim_animates(animate(1, 3, 0x20, 1, 1));
    turn(&mut fx).unwrap();
    fx.world.animators.get_mut(&SIM).unwrap().events = vec![0];
    assert_eq!(turn(&mut fx).unwrap(), Status::Returned(false));
    // The thread returned, so its frame is gone; the local was written
    // before.
    let mut fx = sim_animates(animate(1, 3, 0x20, 1, 2));
    turn(&mut fx).unwrap();
    fx.world.animators.get_mut(&SIM).unwrap().events = vec![0];
    assert_eq!(code(turn(&mut fx).map(|_| true)), Some(8));
}

#[test]
fn a_finished_clip_sends_the_events_it_missed_then_ends() {
    // Backward, three numbered events: they count down from 2.
    let mut fx = sim_animates(animate(1, 3, 0x02, 3, 0));
    turn(&mut fx).unwrap();
    let layer = fx.world.animators[&SIM].foreground.clone().unwrap();
    assert_eq!(layer.phase.to_bits(), 0x3f80_0000);
    assert_eq!(layer.speed.to_bits(), 0xbf80_0000);
    fx.world
        .animators
        .get_mut(&SIM)
        .unwrap()
        .foreground
        .as_mut()
        .unwrap()
        .ended = true;
    for expected in [2, 1, 0] {
        fx.thread(SIM, 4096, &[-7]);
        assert_eq!(turn(&mut fx).unwrap(), Status::Returned(false));
        assert_eq!(fx.world.animators[&SIM].seen, 3 - expected);
    }
    // The end (0x589c25): the true branch. The animator stops playing,
    // takes its state from the posture (0x592904: standing gives 3) and
    // puts the base layer back (0x590430); the pickup and person word 19
    // are cleared. The finished layer stays in the foreground slot.
    {
        let sim = fx.sim_mut();
        sim.person.as_mut().unwrap().data[19] = 7;
    }
    fx.world.animators.get_mut(&SIM).unwrap().pickup = 5;
    // Bit 2 of the type's definition word 102 would keep the base layer
    // off; the fixture's word (1102) has it.
    sim_definition(&mut fx)[102] = 0;
    name_standing_clip(&mut fx);
    fx.thread(SIM, 4096, &[-7]);
    assert_eq!(turn(&mut fx).unwrap(), Status::Returned(true));
    let a = &fx.world.animators[&SIM];
    assert!(!a.playing && a.base == Base::Posture);
    assert_eq!((a.state, a.pickup), (3, 0));
    assert!(a.foreground.as_ref().is_some_and(|l| l.finished()));
    assert_eq!(fx.sim().person.as_ref().unwrap().data[19], 0);
    // With bit 2 set, the base layer stays off.
    let mut fx = sim_animates(animate(1, 3, 0, 0, 0));
    turn(&mut fx).unwrap();
    end_clip(&mut fx);
    assert_eq!(turn(&mut fx).unwrap(), Status::Returned(true));
    let a = &fx.world.animators[&SIM];
    assert!(!a.playing && a.base == Base::Empty);
    assert_eq!(a.state, 3);
}

/// The Sim type's definition words.
fn sim_definition(fx: &mut Fixture) -> &mut [i16; sims_vm::DEFINITION_WORDS] {
    fx.program
        .libraries
        .get_mut(&SIM_FILE)
        .unwrap()
        .definitions
        .get_mut(&128)
        .unwrap()
}

#[test]
fn clip_0_stops_the_foreground_clip() {
    // 0x5897aa: the slot is cleared (0x586b63), the state follows the
    // posture (0x592904), and the pickup flag and person word 19 are
    // cleared; whether a clip plays stays.
    let mut fx = sim_animates(animate(0, 3, 0, 0, 0));
    let mut animator = LOADED;
    animator.foreground = Some(sims_sim::animator::Layer::start(
        CLIP,
        F32::from_bits(HUNDRED),
        4,
        true,
        0,
        false,
        0,
    ));
    animator.selection = 5;
    animator.playing = true;
    animator.pickup = 1;
    animator.state = 0;
    fx.world.animators.insert(SIM, animator);
    fx.sim_mut().person.as_mut().unwrap().data[19] = 4;
    fx.sim_mut().person.as_mut().unwrap().data[0] = 0;
    assert_eq!(turn(&mut fx).unwrap(), Status::Returned(true));
    let a = &fx.world.animators[&SIM];
    assert!(a.foreground.is_none() && a.skin_dirty && a.playing);
    assert_eq!((a.selection, a.state, a.pickup), (-1, 3, 0));
    assert_eq!(fx.sim().person.as_ref().unwrap().data[19], 0);
}

/// A layer of the test clip, started at clock 0 with `tracks` motions
/// with events.
fn layer(tracks: usize) -> sims_sim::animator::Layer {
    sims_sim::animator::Layer::start(CLIP, F32::from_bits(HUNDRED), 4, true, tracks, false, 0)
}

#[test]
fn a_clip_starts_in_the_carry_slot_and_repeats() {
    // 0x589683: the clip is looked up (reading the appearance again), the
    // clock slowed by flag 0x80, and the clip started in the carry slot
    // (0x592726), backward with flag 0x02; then the carry mode is 6 and
    // the layer repeats at priority 10. The true branch is taken at once.
    let mut fx = sim_animates(animate(1, 3, 0x10 | 0x80 | 0x02, 2, 0));
    assert_eq!(turn(&mut fx).unwrap(), Status::Returned(true));
    let a = &fx.world.animators[&SIM];
    let carry = a.carry.as_ref().unwrap();
    assert_eq!(carry.clip, CLIP);
    assert_eq!(
        (carry.priority, carry.mode, carry.interruptible),
        (10, 1, -1)
    );
    assert_eq!((carry.current, carry.speed.to_bits()), (420, 0xbf80_0000));
    assert_eq!(carry.phase.to_bits(), 0x3f80_0000);
    assert_eq!((a.carry_mode, a.carry_selection), (6, -1));
    assert!(a.slow && a.skin_dirty && !a.profile_stale && !a.carry_first);
    // Nothing else changes: no foreground clip, nothing playing, and the
    // next foreground layer still takes the raised priority.
    assert!(a.foreground.is_none() && !a.playing && a.promote);
    assert_eq!((a.state, a.selection), (3, -1));
}

#[test]
fn the_carry_layer_goes_before_a_foreground_layer_of_raised_priority() {
    // 0x599128: made at priority 1, the layer goes in before the first
    // layer whose priority is above 1.
    for (priority, first) in [(6, true), (1, false)] {
        let mut fx = sim_animates(animate(1, 3, 0x10, 0, 0));
        let mut foreground = layer(0);
        foreground.priority = priority;
        fx.world.animators.insert(
            SIM,
            sims_sim::animator::Animator {
                foreground: Some(foreground),
                ..LOADED
            },
        );
        turn(&mut fx).unwrap();
        assert_eq!(fx.world.animators[&SIM].carry_first, first);
    }
    // A new foreground layer goes in before the carry layer.
    let mut fx = sim_animates(animate(1, 3, 0, 0, 0));
    fx.world.animators.insert(
        SIM,
        sims_sim::animator::Animator {
            carry: Some(layer(0)),
            carry_mode: 6,
            carry_first: true,
            ..LOADED
        },
    );
    turn(&mut fx).unwrap();
    let a = &fx.world.animators[&SIM];
    assert!(!a.carry_first && a.carry.is_some() && a.foreground.is_some());
}

#[test]
fn clip_0_clears_the_carry_slot_and_a_pickup_clears_it_too() {
    // 0x589693: the slot is cleared (0x592878), the carry mode 0 and the
    // carry selection -1; the true branch is taken.
    let carrying = || sims_sim::animator::Animator {
        carry: Some(layer(0)),
        carry_mode: 6,
        carry_selection: 3,
        ..LOADED
    };
    let mut fx = sim_animates(animate(0, 3, 0x10, 0, 0));
    fx.world.animators.insert(SIM, carrying());
    assert_eq!(turn(&mut fx).unwrap(), Status::Returned(true));
    let a = &fx.world.animators[&SIM];
    assert!(a.carry.is_none() && a.skin_dirty);
    assert_eq!((a.carry_mode, a.carry_selection), (0, -1));
    // A foreground clip that picks something up (flags 0x11) clears the
    // carry slot as it starts (0x589907); the carry mode stays.
    let mut fx = sim_animates(animate(1, 3, 0x11, 0, 0));
    fx.world.animators.insert(SIM, carrying());
    assert_eq!(turn(&mut fx).unwrap(), Status::Yielded);
    let a = &fx.world.animators[&SIM];
    assert!(a.carry.is_none() && a.foreground.is_some());
    assert_eq!((a.carry_mode, a.carry_selection, a.pickup), (6, -1, 1));
    // The reset (0x589220) empties the slot too.
    let mut a = carrying();
    a.reset().unwrap();
    assert!(a.carry.is_none() && a.skin_dirty);
    assert_eq!((a.carry_mode, a.carry_selection), (0, -1));
}

#[test]
fn a_clip_starts_in_the_base_slot_and_repeats() {
    // 0x58957e: the clip is looked up (reading the appearance again), the
    // clock slowed by flag 0x80, and the clip started in the base slot
    // (0x592726), backward with flag 0x02, replacing the posture's clip
    // there. The true branch is taken at once.
    let mut fx = sim_animates(animate(1, 3, 0x01 | 0x80 | 0x02, 2, 0));
    fx.world.animators.insert(
        SIM,
        sims_sim::animator::Animator {
            posture: 0,
            base: Base::Posture,
            carry_selection: 3,
            ..LOADED
        },
    );
    assert_eq!(turn(&mut fx).unwrap(), Status::Returned(true));
    let a = &fx.world.animators[&SIM];
    assert_eq!(
        a.base,
        Base::Script {
            clip: CLIP.into(),
            reverse: true,
            start: a.clock_at(fx.world.ticks as i32),
        }
    );
    assert!(a.slow && a.skin_dirty && !a.profile_stale);
    // Nothing else changes: the posture noted, the carry bookkeeping, no
    // foreground clip, nothing playing, and the next foreground layer
    // still takes the raised priority.
    assert_eq!((a.posture, a.carry_mode, a.carry_selection), (0, 0, 3));
    assert!(a.foreground.is_none() && a.carry.is_none() && !a.playing && a.promote);
    assert_eq!((a.state, a.selection, a.seen), (3, -1, 0));
    // Forward without flag 0x80.
    let mut fx = sim_animates(animate(1, 3, 0x01, 0, 0));
    assert_eq!(turn(&mut fx).unwrap(), Status::Returned(true));
    let a = &fx.world.animators[&SIM];
    assert_eq!(
        a.base,
        Base::Script {
            clip: CLIP.into(),
            reverse: false,
            start: a.clock_at(fx.world.ticks as i32),
        }
    );
    assert!(!a.slow);
}

#[test]
fn clip_0_clears_the_base_slot() {
    // 0x589585: the slot is cleared (0x592878), which marks the skin's
    // layers changed, and the true branch is taken. The posture noted
    // stays, so the next base update starts the posture's clip again.
    for base in [
        Base::Posture,
        Base::Script {
            clip: CLIP.into(),
            reverse: false,
            start: 0,
        },
    ] {
        let mut fx = sim_animates(animate(0, 3, 0x01, 0, 0));
        fx.world.animators.insert(
            SIM,
            sims_sim::animator::Animator {
                posture: 0,
                base,
                carry_selection: 3,
                ..LOADED
            },
        );
        assert_eq!(turn(&mut fx).unwrap(), Status::Returned(true));
        let a = &fx.world.animators[&SIM];
        assert_eq!(a.base, Base::Empty);
        assert!(a.skin_dirty);
        assert_eq!((a.posture, a.state, a.carry_selection), (0, 3, 3));
    }
    // An empty slot stays empty, and nothing is marked.
    let mut fx = sim_animates(animate(0, 3, 0x01, 0, 0));
    assert_eq!(turn(&mut fx).unwrap(), Status::Returned(true));
    let a = &fx.world.animators[&SIM];
    assert!(a.base == Base::Empty && !a.skin_dirty);
}

/// Marks the clip in the foreground slot as ended.
fn end_clip(fx: &mut Fixture) {
    fx.world
        .animators
        .get_mut(&SIM)
        .unwrap()
        .foreground
        .as_mut()
        .unwrap()
        .ended = true;
}

#[test]
fn what_animate_sim_does_not_port_fails_loudly() {
    for (operands, what) in [
        (animate(1, 1, 0, 0, 0), "source 1"),
        (animate(2, 3, 0, 0, 0), "not listed"),
    ] {
        let mut fx = sim_animates(operands);
        let message = unsupported(turn(&mut fx));
        assert!(message.contains(what), "{message}");
    }
    // A clip whose samples are missing, and one defined differently (here
    // with a timed event) in a second file.
    let mut fx = sim_animates(animate(1, 3, 0, 0, 0));
    fx.program.animations = Default::default();
    fx.program.animations.insert(
        "test.far",
        Clip {
            name: CLIP.into(),
            skill: SKILL.into(),
            duration: HUNDRED,
            distance: 0,
            moving: false,
            frames: 4,
            motions: 1,
            tracks: Vec::new(),
            channels: Vec::new(),
            samples: [0, 0],
        },
    );
    assert!(unsupported(turn(&mut fx)).contains("samples"));
    fx.program.animations.insert_samples(SKILL);
    fx.program.animations.insert(
        "other.far",
        Clip {
            name: CLIP.into(),
            skill: SKILL.into(),
            duration: HUNDRED,
            distance: 0,
            moving: false,
            frames: 4,
            motions: 1,
            tracks: vec![track(&[(0, &[("xevt", "1")])])],
            channels: Vec::new(),
            samples: [0, 0],
        },
    );
    assert!(unsupported(turn(&mut fx)).contains("defined differently"));
    // A clip with timed events in the base slot (0x5992b3).
    let mut fx = sim_animates(animate(1, 3, 0x01, 0, 0));
    fx.program.animations = Default::default();
    fx.program.animations.insert(
        "test.far",
        Clip {
            name: CLIP.into(),
            skill: SKILL.into(),
            duration: HUNDRED,
            distance: 0,
            moving: false,
            frames: 4,
            motions: 1,
            tracks: vec![track(&[(0, &[("xevt", "1")])])],
            channels: Vec::new(),
            samples: [0, 0],
        },
    );
    fx.program.animations.insert_samples(SKILL);
    // A clip with timed events plays in the base slot too: its layer is
    // kept, repeating at priority -10, and the skin's update handles its
    // events (0x5992b3).
    assert_eq!(turn(&mut fx).unwrap(), Status::Returned(true));
    let layer = &fx.world.base_layers[&SIM];
    assert_eq!((layer.mode, layer.priority), (1, -10));
    assert!(matches!(&fx.world.animators[&SIM].base, Base::Script { clip, .. } if clip == CLIP));
    // A clip that ends with the person sitting starts posture 1's base
    // clip, whose name this person's word 64 makes the lookup take from
    // the special tables (0x59db44).
    let mut fx = sim_animates(animate(1, 3, 0, 0, 0));
    turn(&mut fx).unwrap();
    end_clip(&mut fx);
    sim_definition(&mut fx)[102] = 0;
    fx.sim_mut().person.as_mut().unwrap().data[0] = 1;
    assert!(unsupported(turn(&mut fx)).contains("0x59db44"));
}

#[test]
fn a_new_clip_replaces_the_one_in_the_foreground() {
    // 0x586b63: the slot is cleared at once (byte 0x6f6bd0 is 0 in a
    // running game), so the skin is redrawn, and the selection is -1.
    let mut fx = sim_animates(animate(1, 3, 0, 0, 0));
    turn(&mut fx).unwrap();
    let a = fx.world.animators.get_mut(&SIM).unwrap();
    a.playing = false;
    a.skin_dirty = false;
    a.selection = 4;
    a.foreground.as_mut().unwrap().current = 1;
    assert_eq!(turn(&mut fx).unwrap(), Status::Yielded);
    let a = &fx.world.animators[&SIM];
    assert!(a.playing && a.skin_dirty);
    assert_eq!(a.selection, -1);
    assert_eq!(a.foreground.as_ref().unwrap().current, 500);
}

#[test]
fn a_parameter_can_name_the_clip_and_a_bad_one_is_error_8() {
    let mut fx = sim_animates(animate(0, 3, 0x04, 0, 0));
    fx.thread(SIM, 4096, &[1]);
    assert_eq!(turn(&mut fx).unwrap(), Status::Yielded);
    let mut fx = sim_animates(animate(3, 3, 0x04, 0, 0));
    assert_eq!(code(turn(&mut fx).map(|_| true)), Some(8));
}

/// Source 0 (0x59dd6d): an object's own tables name the clip. With no
/// parameter and no object in use, the object is the person itself.
#[test]
fn a_clip_can_be_named_by_an_objects_own_table() {
    let own = |fx: &mut Fixture, table: u16| {
        fx.program
            .libraries
            .get_mut(&SIM_FILE)
            .unwrap()
            .strings
            .insert(table, vec![String::new(), CLIP.into()]);
    };
    // An adult reads table 129.
    let mut fx = sim_animates(animate(1, 0, 0, 0, 0));
    own(&mut fx, 129);
    assert_eq!(turn(&mut fx).unwrap(), Status::Yielded);
    assert_eq!(
        fx.world.animators[&SIM].foreground.as_ref().unwrap().clip,
        CLIP
    );
    // A child (age 17 or less, word 58) reads table 130, not 129.
    let mut fx = sim_animates(animate(1, 0, 0, 0, 0));
    fx.sim_mut().person.as_mut().unwrap().data[58] = 8;
    own(&mut fx, 129);
    assert!(unsupported(turn(&mut fx)).contains("string table 130"));
    let mut fx = sim_animates(animate(1, 0, 0, 0, 0));
    fx.sim_mut().person.as_mut().unwrap().data[58] = 8;
    own(&mut fx, 130);
    assert_eq!(turn(&mut fx).unwrap(), Status::Yielded);
}

/// Look Towards (opcode 22, 0x45a98e) in mode 0: the head follows the
/// stack object, through person data words 41 to 45 (0x58a035).
#[test]
fn look_towards_mode_0_sets_the_heads_target() {
    let look = |mode: i16| {
        let mut fx = Fixture::new();
        let [a, b] = mode.to_le_bytes();
        fx.add(
            SIM_FILE,
            4096,
            tree(vec![ins(
                22,
                RETURN_TRUE,
                RETURN_FALSE,
                [a, b, 0, 0, 0, 0, 0, 0],
            )]),
        );
        fx.add(
            LAMP_FILE,
            4096,
            tree(vec![ins(
                22,
                RETURN_TRUE,
                RETURN_FALSE,
                [a, b, 0, 0, 0, 0, 0, 0],
            )]),
        );
        fx
    };
    let mut fx = look(0);
    fx.sim_mut().person.as_mut().unwrap().data[41..46].copy_from_slice(&[9, 9, 9, 9, 9]);
    assert!(fx.run_with(SIM, 4096, LAMP as i16, &[], 10).unwrap());
    let data = &fx.sim().person.as_ref().unwrap().data;
    assert_eq!(data[41..46], [LAMP as i16, 1, 0, 1, 0]);
    // With no stack object, word 42 is 0.
    let mut fx = look(0);
    assert!(fx.run_with(SIM, 4096, 0, &[], 10).unwrap());
    assert_eq!(
        fx.sim().person.as_ref().unwrap().data[41..46],
        [0, 0, 0, 1, 0]
    );
    // Modes 1 to 3 turn the person on the spot (below); any other mode is
    // error 0x27; an object that is not a person fails.
    let fails = |fx: &mut Fixture, object: u16| {
        fx.run_with(object, 4096, LAMP as i16, &[], 10)
            .unwrap_err()
            .to_string()
    };
    assert_eq!(
        code(look(4).run_with(SIM, 4096, LAMP as i16, &[], 10)),
        Some(0x27)
    );
    assert!(fails(&mut look(0), LAMP).contains("not a person"));
}

/// Look Towards (opcode 22) in modes 2 and 3 (0x45a9c1): the person turns
/// on the spot, by a route from its own place to its own place, then to
/// the stack object's place (mode 2) or the place mirrored through its
/// own (mode 3).
#[test]
fn look_towards_modes_2_and_3_turn_on_the_spot() {
    let look = |mode: i16| {
        let mut fx = Fixture::new();
        let [a, b] = mode.to_le_bytes();
        fx.add(
            SIM_FILE,
            4096,
            tree(vec![ins(
                22,
                RETURN_TRUE,
                RETURN_FALSE,
                [a, b, 0, 0, 0, 0, 0, 0],
            )]),
        );
        fx.world.animators.insert(SIM, LOADED);
        fx.sim_mut().position = [100, 200, 1];
        fx.lamp_mut().position = [130, 210, 1];
        fx
    };
    let run =
        |fx: &mut Fixture, stack_object: u16| fx.run_with(SIM, 4096, stack_object as i16, &[], 10);
    // A person sitting or lying (word 0 not 0) does not turn.
    let mut fx = look(2);
    fx.sim_mut().person.as_mut().unwrap().data[0] = 1;
    assert!(!run(&mut fx, LAMP).unwrap());
    assert!(!fx.world.routing.routes.contains_key(&SIM));
    // With no stack object, error 0x15.
    assert_eq!(code(run(&mut look(2), 0)), Some(0x15));
    // Mode 1 turns towards the camera, which is not ported.
    assert!(
        run(&mut look(1), LAMP)
            .unwrap_err()
            .to_string()
            .contains("mode 1")
    );
    // The path holds the second coordinate first. A synchronous tree may
    // not wait for the turn (error 0x44f).
    for (mode, to) in [(2, [210, 130]), (3, [190, 70])] {
        let mut fx = look(mode);
        let result = run(&mut fx, LAMP);
        assert_eq!(
            fx.world.routing.routes[&SIM].path,
            vec![[200, 100], [200, 100], to]
        );
        assert_eq!(code(result), Some(0x44f), "mode {mode}");
    }
}

#[test]
fn a_walking_animator_makes_the_thread_wait() {
    let mut fx = sim_animates(animate(1, 3, 0, 0, 0));
    fx.world
        .animators
        .insert(SIM, sims_sim::animator::Animator { state: 5, ..LOADED });
    assert_eq!(turn(&mut fx).unwrap(), Status::Yielded);
    assert!(!fx.world.animators[&SIM].playing);
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

/// The Sim stands on the lot, sleeping, with nothing in its hands; its
/// motives do not decay and it is a visitor, so no lot service is asked.
fn standing() -> Fixture {
    let mut fx = sim_animates(animate(1, 3, 0, 0, 0));
    fx.add(SIM_FILE, 4097, sleeper());
    fx.thread(SIM, 4097, &[100]);
    let sim = fx.sim_mut();
    sim.registry_word = Some(0x0001_ffff);
    sim.position = [100, 200, 1];
    sim.data[1] = 2;
    sim.data[25] = 0;
    sim.data[29] = -5;
    sim.data[34] = 0;
    sim.slots[1].object = 0;
    let person = sim.person.as_mut().unwrap();
    person.data[1] = 0;
    person.data[29] = 1;
    person.data[32] = 1;
    person.data[42] = 0;
    person.data[45] = 2;
    fx
}

fn tick(fx: &mut Fixture) -> Result<bool, sims_vm::Fault> {
    let tuning = tuning();
    let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    let ticks = fx.world.ticks as i32;
    let result = interpreter.tick_person(&mut fx.world, SIM, ticks, &tuning);
    fx.world.ticks += 1;
    result
}

#[test]
fn the_first_tick_places_the_animator_and_its_speed_rises() {
    let mut fx = standing();
    tick(&mut fx).unwrap();
    let a = &fx.world.animators[&SIM];
    assert_eq!((a.clock, a.head_clock, a.skin_clock), (500, 500, 500));
    // Placed on the person's position and direction.
    let bits = |v: [F32; 3]| v.map(F32::to_bits);
    let placed = [0x42c8_0000, 0x4348_0000, 0x4000_0000];
    assert_eq!(bits(a.position), placed);
    assert_eq!(bits(a.destination), placed);
    // The speed wanted is 1.2 plus a fifth; the speed moves up by a fifth
    // (single precision throughout).
    assert_eq!(
        (a.rate.to_bits(), a.target.to_bits()),
        (0x3f99_999a, 0x3fb3_3334)
    );
    assert_eq!((a.state, a.arrival), (3, 0));
    // The head's countdown went down by one.
    assert_eq!(fx.sim().person.as_ref().unwrap().data[45], 1);
    // Standing where it is placed, it has arrived; the speed reaches 1.4.
    tick(&mut fx).unwrap();
    let a = &fx.world.animators[&SIM];
    assert_eq!(a.rate.to_bits(), 0x3fb3_3334);
    assert_eq!((a.state, a.arrival, a.clock), (3, 0, 550));
    // The countdown reached 0, which sets person word 42 to 8.
    let person = fx.sim().person.as_ref().unwrap();
    assert_eq!((person.data[45], person.data[42]), (0, 8));
}

#[test]
fn an_arrival_after_a_route_is_noted_and_directions_wrap() {
    let mut fx = standing();
    let placed = [100, 200].map(|v| F32::from_f32(v as f32));
    fx.world.animators.insert(
        SIM,
        sims_sim::animator::Animator {
            arrival: -1,
            position: [placed[0], placed[1], F32::from_f32(-6.0)],
            destination: [placed[0], placed[1], F32::from_f32(10.0)],
            ..LOADED
        },
    );
    tick(&mut fx).unwrap();
    let a = &fx.world.animators[&SIM];
    assert_eq!(a.arrival, 2);
    assert_eq!(a.position[2].to_bits(), 0x4000_0000);
    assert_eq!(a.destination[2].to_bits(), 0x4000_0000);
}

#[test]
fn a_turn_of_less_than_a_step_faces_the_destination_at_once() {
    // Arrived, an eighth of a direction short of the destination's: less
    // than a step (a quarter over the speed), and within the epsilon of
    // its destination's coordinates. The animator is put at its
    // destination (0x58dfcf), the direction goes to data word 1, and state
    // 3 runs again, which notes the route's end. No clip plays. The person
    // is of kind 0 (word 65), whose bounds are not scaled.
    let mut fx = standing();
    fx.sim_mut().person.as_mut().unwrap().data[65] = 0;
    let [half, quarter] = [0x3f00_0000, 0x3e80_0000].map(F32::from_bits);
    fx.world.animators.insert(
        SIM,
        sims_sim::animator::Animator {
            arrival: -1,
            position: [
                F32::from_bits(0x3f00_0005),
                F32::from_bits(0x3e80_0003),
                F32::from_f32(2.4375),
            ],
            destination: [half, quarter, F32::from_f32(2.5625)],
            ..LOADED
        },
    );
    tick(&mut fx).unwrap();
    let a = &fx.world.animators[&SIM];
    assert_eq!((a.state, a.arrival, a.selection), (3, 2, -1));
    assert!(a.foreground.is_none());
    let bits = |v: [F32; 3]| v.map(F32::to_bits);
    assert_eq!(bits(a.position), [0x3f00_0000, 0x3e80_0000, 0x4024_0000]);
    assert_eq!(bits(a.destination), bits(a.position));
    assert_eq!(fx.sim().data[1], 3);
    // Half a direction is a step at least: a turn with a clip, which this
    // Sim's tables do not name (an adult, word 58, with no special
    // tables, word 64).
    let mut fx = standing();
    let person = fx.sim_mut().person.as_mut().unwrap();
    person.data[58] = 20;
    person.data[64] = 0;
    person.data[65] = 0;
    fx.world.animators.insert(
        SIM,
        sims_sim::animator::Animator {
            position: [half, quarter, F32::from_f32(2.0)],
            destination: [half, quarter, F32::from_f32(2.5)],
            ..LOADED
        },
    );
    assert!(
        tick(&mut fx)
            .unwrap_err()
            .to_string()
            .contains("has no name for its clip 10")
    );
}

#[test]
fn off_the_lot_the_animator_only_places_itself() {
    let mut fx = standing();
    fx.sim_mut().position = [-16, -16, 1];
    tick(&mut fx).unwrap();
    let a = &fx.world.animators[&SIM];
    assert_eq!(a.position[0].to_bits(), 0xc180_0000);
    assert_eq!(a.state, 3);
    assert_ne!(a.position[2], UNSET);
}

#[test]
fn the_clip_advances_with_the_clock_and_stops_at_its_end() {
    let mut fx = standing();
    fx.thread(SIM, 4096, &[-7]);
    fx.sim_mut().stack[0].locals = vec![0, 0];
    // The thread starts the clip; the animator updates it at the same
    // clock, so it has not moved.
    tick(&mut fx).unwrap();
    let layer = fx.world.animators[&SIM].foreground.clone().unwrap();
    assert_eq!(
        (layer.current, layer.previous, layer.phase),
        (500, 500, F32::ZERO)
    );
    // 50 ms of 100: half way, event time 50.
    tick(&mut fx).unwrap();
    let layer = fx.world.animators[&SIM].foreground.clone().unwrap();
    assert_eq!((layer.current, layer.previous), (550, 500));
    assert_eq!(layer.phase.to_bits(), 0x3f00_0000);
    assert_eq!((layer.event_time, layer.previous_event_time), (50, 50));
    assert!(!layer.finished());
    // The end: stopped, with the duration of a stopped clip.
    tick(&mut fx).unwrap();
    let layer = fx.world.animators[&SIM].foreground.clone().unwrap();
    assert!(layer.ended && layer.finished());
    assert_eq!(layer.phase.to_bits(), 0x3f80_0000);
    assert_eq!(
        (layer.speed, layer.duration.to_bits()),
        (F32::ZERO, 0x4cbe_bc20)
    );
    // The thread's next run finds the clip done and ends it: the animator
    // stops playing and stands (state 3). The tree then returns from its
    // only frame, which is not ported.
    let message = tick(&mut fx).unwrap_err().to_string();
    assert!(message.contains("outermost frame"), "{message}");
    let a = &fx.world.animators[&SIM];
    assert!(!a.playing);
    assert_eq!(a.state, 3);
}

#[test]
fn a_clip_that_follows_the_speed_takes_it() {
    let mut fx = standing();
    fx.add(
        SIM_FILE,
        4096,
        shaped(
            0x8002,
            0,
            1,
            2,
            vec![ins(
                44,
                RETURN_TRUE,
                RETURN_FALSE,
                animate(1, 3, 0x40, 0, 0),
            )],
        ),
    );
    fx.thread(SIM, 4096, &[-7]);
    fx.sim_mut().stack[0].locals = vec![0, 0];
    tick(&mut fx).unwrap();
    // State 0 hands the layer the animator's speed: 1.2, so the clip's
    // duration is 100 / 1.2.
    let a = &fx.world.animators[&SIM];
    let layer = a.foreground.as_ref().unwrap();
    assert_eq!(layer.speed, a.rate);
    assert_eq!(layer.speed.to_bits(), 0x3f99_999a);
    assert_eq!(layer.duration.to_bits(), 0x42a6_aaaa);
}

/// One motion's timed events: (time, properties).
fn track(events: &[(i32, &[(&str, &str)])]) -> Track {
    Track {
        bone: "ROOT".into(),
        events: events
            .iter()
            .map(|&(time, pairs)| Event {
                time,
                properties: pairs.iter().map(|&(k, v)| (k.into(), v.into())).collect(),
            })
            .collect(),
    }
}

/// The standing Sim, whose clip has the timed events `tracks`.
fn with_events(tracks: Vec<Track>) -> Fixture {
    let mut fx = standing();
    fx.program.animations.insert(
        "test.far",
        Clip {
            name: CLIP.into(),
            skill: SKILL.into(),
            duration: HUNDRED,
            distance: 0,
            moving: false,
            frames: 4,
            motions: 2,
            tracks,
            channels: Vec::new(),
            samples: [0, 0],
        },
    );
    fx.thread(SIM, 4096, &[-7]);
    fx.sim_mut().stack[0].locals = vec![0, 0];
    fx
}

#[test]
fn a_clips_timed_events_are_handled_as_it_plays() {
    // 0x5912fd: "xevt" queues its number (read as atoi does), the hands
    // and censor go to person words 22, 23 and 30, "interruptible" to the
    // layer; the anchor is not kept, and sounds and other keys do nothing. An event before 0 is
    // passed over.
    let mut fx = with_events(vec![
        track(&[
            (-5, &[("xevt", "9")]),
            (0, &[("xevt", " -12abc"), ("sound", "x")]),
            (30, &[("lefthand", "7"), ("anchor", "1"), ("censor", "2")]),
            (90, &[("xevt", "4")]),
        ]),
        track(&[(50, &[("righthand", "3"), ("interruptible", "0")])]),
    ]);
    // The thread starts the clip; the animator's update at the same clock
    // reaches event time 0.
    tick(&mut fx).unwrap();
    let a = &fx.world.animators[&SIM];
    assert_eq!(a.foreground.as_ref().unwrap().cursors, [2, 0]);
    assert_eq!(a.events, [-12]);
    // Event time 50: the first motion's event at 30, then the second's at
    // 50. (The thread dropped the queued -12 first.)
    tick(&mut fx).unwrap();
    let a = &fx.world.animators[&SIM];
    let layer = a.foreground.as_ref().unwrap();
    assert_eq!(
        (layer.cursors.as_slice(), layer.interruptible),
        (&[3, 1][..], 0)
    );
    let person = &fx.sim().person.as_ref().unwrap().data;
    assert_eq!((person[22], person[23], person[30]), (7, 3, 2));
}

#[test]
fn the_carry_layer_plays_with_the_foreground_in_the_skins_order() {
    // 0x5992b3 updates the skin's layers in its list's order, sorted by
    // priority (0x5991fd), so the events of the layer that comes first
    // are queued first. Of two layers of the same priority, the one put in
    // the list first comes first. The carry layer repeats at its end (end
    // mode 1).
    for (carry_first, queued) in [(false, [1, 2]), (true, [2, 1])] {
        let mut fx = standing();
        fx.program.animations.insert(
            "test.far",
            Clip {
                name: CLIP.into(),
                skill: SKILL.into(),
                duration: HUNDRED,
                distance: 0,
                moving: false,
                frames: 4,
                motions: 1,
                tracks: vec![track(&[(0, &[("xevt", "1")])])],
                channels: Vec::new(),
                samples: [0, 0],
            },
        );
        fx.program.animations.insert(
            "carry.far",
            Clip {
                name: "carry-clip".into(),
                skill: SKILL.into(),
                duration: HUNDRED,
                distance: 0,
                moving: false,
                frames: 4,
                motions: 1,
                tracks: vec![track(&[(0, &[("xevt", "2")])])],
                channels: Vec::new(),
                samples: [0, 0],
            },
        );
        let start = |clip: &str, now| {
            sims_sim::animator::Layer::start(clip, F32::from_bits(HUNDRED), 4, true, 1, false, now)
        };
        let mut carry = start("carry-clip", 500);
        carry.mode = 1;
        carry.priority = 10;
        let mut foreground = start(CLIP, 500);
        foreground.priority = 10;
        fx.world.animators.insert(
            SIM,
            sims_sim::animator::Animator {
                foreground: Some(foreground),
                carry: Some(carry),
                carry_mode: 6,
                carry_first,
                ..LOADED
            },
        );
        tick(&mut fx).unwrap();
        assert_eq!(fx.world.animators[&SIM].events, queued);
        // Two more ticks reach the clips' end: the foreground clip stops,
        // the carry clip goes on.
        tick(&mut fx).unwrap();
        tick(&mut fx).unwrap();
        let a = &fx.world.animators[&SIM];
        assert!(a.foreground.as_ref().unwrap().finished());
        let carry = a.carry.as_ref().unwrap();
        assert_eq!((carry.current, carry.speed.to_bits()), (600, 0x3f80_0000));
        assert_eq!(carry.phase.to_bits(), 0x3f80_0000);
        assert_eq!(a.carry_mode, 6);
    }
}

/// The standing Sim with something in its hands, the global adult
/// tables naming `name` as the posture's clip 0x26, the carry clip.
fn carrying(name: &str) -> Fixture {
    let mut fx = standing();
    fx.sim_mut().slots[1].object = 1;
    fx.sim_mut().person.as_mut().unwrap().data[64] = 0;
    let mut names = vec![String::new(); 0x27];
    names[0x26] = name.into();
    for table in [152, 153] {
        fx.program
            .libraries
            .get_mut(&GLOBAL)
            .unwrap()
            .strings
            .insert(table, names.clone());
    }
    fx
}

#[test]
fn a_person_carrying_starts_the_carry_clip_until_its_hands_are_empty() {
    // 0x5903a3: something in the first contained slot sets the carry mode
    // to 1 and chooses carry clip 0, which starts the posture's clip 0x26
    // in the carry slot (0x5903fc): forward, stopping at its end (end mode
    // 0), at priority 10, read with the appearance read again first.
    let mut fx = carrying(CLIP);
    tick(&mut fx).unwrap();
    let a = &fx.world.animators[&SIM];
    assert_eq!((a.carry_mode, a.carry_selection), (1, 0));
    assert!(!a.profile_stale && !a.carry_first && !a.skin_dirty);
    let carry = a.carry.as_ref().unwrap();
    assert_eq!(carry.clip, CLIP);
    assert_eq!(
        (carry.priority, carry.mode, carry.interruptible),
        (10, 0, -1)
    );
    assert_eq!((carry.current, carry.previous), (500, 500));
    assert_eq!(
        (carry.speed.to_bits(), carry.phase.to_bits()),
        (0x3f80_0000, 0)
    );
    // The carry clip stays chosen: the layer is not started again, and
    // the skin's update takes it through the clip (50 per tick over 100
    // ms) to its end, where it stops.
    tick(&mut fx).unwrap();
    let carry = fx.world.animators[&SIM].carry.clone().unwrap();
    assert_eq!((carry.current, carry.previous), (550, 500));
    assert_eq!(carry.phase.to_bits(), 0x3f00_0000);
    tick(&mut fx).unwrap();
    let a = &fx.world.animators[&SIM];
    let carry = a.carry.as_ref().unwrap();
    assert!(carry.ended && carry.finished());
    assert_eq!(
        (carry.speed.to_bits(), carry.phase.to_bits()),
        (0, 0x3f80_0000)
    );
    assert_eq!((a.carry_mode, a.carry_selection), (1, 0));
    // Empty hands choose -1, which clears the slot at once.
    fx.sim_mut().slots[1].object = 0;
    tick(&mut fx).unwrap();
    let a = &fx.world.animators[&SIM];
    assert_eq!((a.carry_mode, a.carry_selection), (0, -1));
    assert!(a.carry.is_none());
    // While a clip that picks something up plays (+0x1a4), -1 is chosen
    // whatever the hands hold, and the carry mode stays.
    let mut fx = carrying(CLIP);
    tick(&mut fx).unwrap();
    fx.world.animators.get_mut(&SIM).unwrap().pickup = 1;
    tick(&mut fx).unwrap();
    let a = &fx.world.animators[&SIM];
    assert_eq!((a.carry_mode, a.carry_selection), (1, -1));
    assert!(a.carry.is_none());
}

#[test]
fn the_skin_updates_its_layers_by_priority() {
    // The carry clip goes in before a foreground layer of raised priority
    // (0x599128), but the skin's update first sorts its layers by
    // priority (0x5991fd), so the foreground's events come first. A
    // foreground of higher priority than the carry layer's 10 stays after
    // it, and one of the same priority keeps its place in the list.
    for (priority, queued) in [(6, [1, 2]), (11, [2, 1]), (10, [2, 1])] {
        let mut fx = carrying("carry-clip");
        fx.program.animations.insert(
            "test.far",
            Clip {
                name: CLIP.into(),
                skill: SKILL.into(),
                duration: HUNDRED,
                distance: 0,
                moving: false,
                frames: 4,
                motions: 1,
                tracks: vec![track(&[(0, &[("xevt", "1")])])],
                channels: Vec::new(),
                samples: [0, 0],
            },
        );
        fx.program.animations.insert(
            "carry.far",
            Clip {
                name: "carry-clip".into(),
                skill: SKILL.into(),
                duration: HUNDRED,
                distance: 0,
                moving: false,
                frames: 4,
                motions: 1,
                tracks: vec![track(&[(0, &[("xevt", "2")])])],
                channels: Vec::new(),
                samples: [0, 0],
            },
        );
        let mut foreground =
            sims_sim::animator::Layer::start(CLIP, F32::from_bits(HUNDRED), 4, true, 1, false, 450);
        foreground.priority = priority;
        fx.world.animators.insert(
            SIM,
            sims_sim::animator::Animator {
                foreground: Some(foreground),
                profile_stale: false,
                ..LOADED
            },
        );
        tick(&mut fx).unwrap();
        let a = &fx.world.animators[&SIM];
        assert_eq!(a.events, queued, "foreground priority {priority}");
        // The order the layers were put in is kept.
        assert!(a.carry_first);
    }
    // A carry layer put in after a foreground layer whose priority is
    // above its own updates first, whether or not the skin has changed
    // since the last update, whose sort left that order.
    for dirty in [true, false] {
        let queued = [2, 1];
        let mut fx = standing();
        fx.program.animations.insert(
            "test.far",
            Clip {
                name: CLIP.into(),
                skill: SKILL.into(),
                duration: HUNDRED,
                distance: 0,
                moving: false,
                frames: 4,
                motions: 1,
                tracks: vec![track(&[(0, &[("xevt", "1")])])],
                channels: Vec::new(),
                samples: [0, 0],
            },
        );
        fx.program.animations.insert(
            "carry.far",
            Clip {
                name: "carry-clip".into(),
                skill: SKILL.into(),
                duration: HUNDRED,
                distance: 0,
                moving: false,
                frames: 4,
                motions: 1,
                tracks: vec![track(&[(0, &[("xevt", "2")])])],
                channels: Vec::new(),
                samples: [0, 0],
            },
        );
        let start = |clip: &str| {
            sims_sim::animator::Layer::start(clip, F32::from_bits(HUNDRED), 4, true, 1, false, 450)
        };
        let mut foreground = start(CLIP);
        foreground.priority = 11;
        let mut carry = start("carry-clip");
        carry.priority = 10;
        carry.mode = 1;
        fx.world.animators.insert(
            SIM,
            sims_sim::animator::Animator {
                foreground: Some(foreground),
                carry: Some(carry),
                carry_mode: 6,
                skin_dirty: dirty,
                skin_clock: 450,
                profile_stale: false,
                ..LOADED
            },
        );
        tick(&mut fx).unwrap();
        let a = &fx.world.animators[&SIM];
        assert_eq!(a.events, queued, "skin changed: {dirty}");
        assert!(!a.carry_first);
    }
}

#[test]
fn a_carry_clip_not_found_fails_loudly() {
    // The original leaves the slot empty and still notes clip 0 chosen.
    let mut fx = carrying("");
    let message = tick(&mut fx).unwrap_err().to_string();
    assert!(message.contains("0x59db2f"), "{message}");
    let mut fx = carrying("a-clip-no-file-holds");
    let message = tick(&mut fx).unwrap_err().to_string();
    assert!(message.contains("0x59dc32"), "{message}");
}

#[test]
fn a_timed_event_not_ported_fails_loudly() {
    for key in ["dress", "undress"] {
        let mut fx = with_events(vec![track(&[(0, &[(key, "1")])])]);
        let message = tick(&mut fx).unwrap_err().to_string();
        assert!(
            message.contains(key) && message.contains("0x5912fd"),
            "{message}"
        );
    }
}

#[test]
fn what_the_animator_tick_does_not_port_fails_loudly() {
    // The head following something is ported (0x5907c5): with the target
    // an eighth of a turn away the turn moves by its most, 0.2. This
    // person type binds no head anchors, so nothing is measured.
    let mut fx = standing();
    tick(&mut fx).unwrap();
    let person = fx.sim_mut().person.as_mut().unwrap();
    person.data[41..46].copy_from_slice(&[0, 1, 0, 0, 0]);
    fx.world
        .animators
        .get_mut(&SIM)
        .unwrap()
        .head
        .target_heading = F32::from_f32(1.0);
    tick(&mut fx).unwrap();
    assert_eq!(fx.world.animators[&SIM].head.turn.to_bits(), 0xbe4c_cccd);
    // A destination elsewhere, and a state not ported.
    let mut fx = standing();
    let mut animator = LOADED;
    animator.position = [F32::ZERO; 3];
    animator.destination = [F32::from_f32(16.0), F32::ZERO, F32::ZERO];
    fx.world.animators.insert(SIM, animator.clone());
    assert!(
        tick(&mut fx)
            .unwrap_err()
            .to_string()
            .contains("walks to its destination")
    );
    let mut fx = standing();
    fx.world
        .animators
        .insert(SIM, sims_sim::animator::Animator { state: 7, ..LOADED });
    assert!(tick(&mut fx).unwrap_err().to_string().contains("state 7"));
}

/// Names the posture's clip `selection` in the global table 150, which
/// grows to hold it, and adds a clip of that name moving `distance`
/// (float bits; 0 moves nothing) per cycle of 100 milliseconds.
fn name_clip(fx: &mut Fixture, selection: usize, name: &str, distance: u32) {
    let names = fx
        .program
        .libraries
        .get_mut(&GLOBAL)
        .unwrap()
        .strings
        .entry(150)
        .or_default();
    if names.len() <= selection {
        names.resize(selection + 1, String::new());
    }
    names[selection] = name.into();
    fx.program.animations.insert(
        "walk.far",
        Clip {
            name: name.into(),
            skill: SKILL.into(),
            duration: HUNDRED,
            distance,
            moving: distance != 0,
            frames: 4,
            motions: 1,
            tracks: Vec::new(),
            channels: Vec::new(),
            samples: [0, 0],
        },
    );
}

/// The standing Sim with a route to follow: the animator as the route's
/// start leaves it (0x589c85: result 1, segment 0, standing on the
/// person's tile and direction), facing direction 6, and a path east
/// along the first coordinate. Global clip table 150 names the walk
/// (clip 0x14, whose distance sets the look-ahead), the stop from a walk
/// (clip 0x13) and the half turn (clip 5).
fn routed() -> Fixture {
    let mut fx = standing();
    fx.sim_mut().data[1] = 6;
    // Data word 17 clear: the person walks (state 9), and looks ahead by
    // the walk clip's distance, not the run's (0x58e6cb).
    fx.sim_mut().data[17] = 0;
    // An adult (word 58) of kind 0 (word 65), with no special clip
    // tables (word 64).
    let person = fx.sim_mut().person.as_mut().unwrap();
    person.data[58] = 20;
    person.data[64] = 0;
    person.data[65] = 0;
    name_clip(&mut fx, 5, "test-half-turn", 0);
    name_clip(&mut fx, 0x13, "test-stop", 0x4200_0000);
    name_clip(&mut fx, 0x14, "test-walk", 0x4180_0000);
    let at = [100.0, 200.0, 6.0].map(F32::from_f32);
    let mut animator = LOADED;
    animator.position = at;
    animator.destination = at;
    animator.arrival = 1;
    animator.state = 3;
    animator.motion.segment = 0;
    animator.motion.progress = F32::ZERO;
    fx.world.animators.insert(SIM, animator);
    fx.world.routing.routes.insert(
        SIM,
        sims_sim::route::Route {
            contexts: Vec::new(),
            path: vec![[200, 100], [200, 300], [200, 400], [200, 500]],
        },
    );
    flat_lot(&mut fx);
    fx
}

/// A flat lot of 40 tiles a side with no walls, floors, pools or water, on
/// which the Sim may stand on level 1 (data word 4 bit 0) without a floor
/// (data word 42 flag 2), for the walk's place test.
fn flat_lot(fx: &mut Fixture) {
    use crate::rooms::array;
    let world = &mut fx.world;
    world.size = 40;
    world.arrays.insert(0, array(4, 0));
    for (floors, walls, flags) in [(1, 2, 8), (101, 102, 108)] {
        world.arrays.insert(floors, array(2, 0));
        world.arrays.insert(walls, array(14, 0));
        world.arrays.insert(flags, array(1, 0));
    }
    world.arrays.insert(3, array(2, 0));
    world.arrays.insert(103, array(2, 0));
    world.arrays.insert(9, array(1, 0xff));
    world.arrays.insert(10, array(1, 0xff));
    world.rebuild_rooms().unwrap();
    world.indicators = [LAMP, SIM];
    let sim = fx.sim_mut();
    sim.data[4] |= 1;
    sim.data[42] = 2;
}

#[test]
fn a_route_is_followed_and_a_turn_back_is_made_on_the_spot() {
    let mut fx = routed();
    tick(&mut fx).unwrap();
    let a = &fx.world.animators[&SIM];
    // State 3 finds the route's result 1 and follows (state 9). Segment 0
    // has no length, so the animator moves to segment 1 and looks ahead
    // along it from 0 in steps of 1/200, to the first point at least the
    // look-ahead away: half the walk's distance (16), times 16/3, halved,
    // 21.33; squared 455.1, reached 22 units on (progress 0.11, summed in
    // single precision). That point lies in
    // direction 2, 4 eighths back from 6: the animator stands to turn
    // (state 3), and the turn starts the half turn clip (state 4).
    assert_eq!(a.motion.segment, 1);
    assert_eq!(a.state, 4);
    assert_eq!(a.motion.turn_wanted.to_bits(), 0xc080_0000);
    assert_eq!(a.motion.turn_clip.to_bits(), 0xc080_0000);
    assert_eq!(a.motion.turn_destination[2].to_bits(), 0x4000_0000);
    assert_eq!(a.motion.turn_from[2].to_bits(), 0x40c0_0000);
    // The destination's direction is the position's plus the turn less
    // the clip's turn: 6 again.
    assert_eq!(a.destination[2].to_bits(), 0x40c0_0000);
    let layer = a.foreground.as_ref().unwrap();
    assert_eq!((layer.clip.as_str(), layer.priority), ("test-half-turn", 5));
    assert_eq!(layer.speed, a.rate);
    assert_eq!(a.motion.progress.to_bits(), 0x3de1_47b0);
    // State 4 plays the turn.
    tick(&mut fx).unwrap();
    let a = &fx.world.animators[&SIM];
    assert_eq!(a.state, 4);
    assert_eq!(fx.sim().data[1], 6);
    // At the clip's end (0x58e06f) the turn ends where it was to: facing
    // direction 2, the clip cleared and its selection -1, the next
    // clip's priority raised again. The animator stands and, its route
    // result still 1, follows again at once.
    let mut ticks = 0;
    while fx.world.animators[&SIM].state == 4 {
        tick(&mut fx).unwrap();
        ticks += 1;
        assert!(ticks < 10);
    }
    assert_eq!(ticks, 2);
    // The point ahead lies straight ahead, so the animator walks
    // (0x58ec8a): the walk clip (0x14) repeats in the foreground at the
    // animator's speed, 1.4, from phase 0, as nothing moving played
    // before; it takes the raised priority. The step is the clip's
    // distance, 16, times 16/3, over its duration at that speed in ticks
    // of 42 milliseconds (100 / 1.4 / 42): 50.18 units along the first
    // coordinate.
    let a = &fx.world.animators[&SIM];
    assert_eq!((a.state, a.selection, a.walking), (9, 0x14, true));
    let layer = a.foreground.as_ref().unwrap();
    assert_eq!((layer.clip.as_str(), layer.mode), ("test-walk", 1));
    assert_eq!(
        (layer.priority, layer.speed, layer.phase),
        (6, a.rate, F32::ZERO)
    );
    assert_eq!(a.rate.to_bits(), 0x3fb3_3334);
    assert_eq!(
        a.position.map(F32::to_bits),
        [0x4316_2d0f, 0x4348_0000, 0x4000_0000]
    );
    // The person is put at the step's end rounded, on the path, and is
    // seen in the outdoor room 0.
    assert_eq!(fx.sim().position, [150, 200, 1]);
    assert_eq!((fx.sim().data[1], fx.sim().data[29]), (2, 0));
    // The next tick goes on with the same clip, keeping its phase (the
    // clip moves the person): 50 milliseconds of 71.4.
    tick(&mut fx).unwrap();
    let a = &fx.world.animators[&SIM];
    let layer = a.foreground.as_ref().unwrap();
    assert_eq!(layer.phase.to_bits(), 0x3f33_3334);
    assert_eq!(a.position[0].to_bits(), 0x4348_5a1e);
    assert_eq!(fx.sim().position, [200, 200, 1]);
}

#[test]
fn a_step_is_moved_onto_the_path_where_it_is_nearer() {
    // 0x48313e: path points hold the +0xf4 coordinate first.
    let path = [[0, 0], [0, 100], [100, 100], [100, 200]];
    // Heading for point 2: the segment from point 1 to 2 is looked at.
    // (40, 103) is 3 off it; its nearest point there is (40, 100).
    assert_eq!(
        sims_vm::snap_to_path(&path, [40, 103], 2).unwrap(),
        [40, 100]
    );
    // The path's first looked-at point wins when it is nearer.
    assert_eq!(
        sims_vm::snap_to_path(&path, [-5, 100], 2).unwrap(),
        [0, 100]
    );
    // Index 0 looks at the first segment; below 0 at every segment but
    // the last.
    assert_eq!(sims_vm::snap_to_path(&path, [3, 50], 0).unwrap(), [0, 50]);
    assert_eq!(
        sims_vm::snap_to_path(&path, [97, 60], -1).unwrap(),
        [97, 100]
    );
    // Heading for the last point, or past it, the point stays.
    assert_eq!(
        sims_vm::snap_to_path(&path, [97, 160], 3).unwrap(),
        [97, 160]
    );
    // A projection that is not a whole unit rounds half up: on the
    // segment from (0, 0) to (10, 5), (4, 3) projects to t = 0.44,
    // (4.4, 2.2), rounded (4, 2).
    let path = [[0, 0], [10, 5], [20, 5]];
    assert_eq!(sims_vm::snap_to_path(&path, [4, 3], 1).unwrap(), [4, 2]);
}

/// At the end of a path (0x58e740) a person with object data word 9 set
/// keeps walking while it is far from the end point and has not come
/// near it (+0x288 clear). Once near, it stands where it is (0x58e846):
/// the destination becomes the position, +0x288 and +0x28c are set, and
/// the state's code runs again as a standing animator, which stops
/// walking.
#[test]
fn at_the_end_of_its_path_a_person_walks_on_then_stops() {
    // Three points: the second segment is the last; the end point is
    // 100 away, farther than the stopping distance (2 times 1.5).
    let mut fx = routed();
    fx.world.routing.routes.get_mut(&SIM).unwrap().path.pop();
    fx.sim_mut().data[9] = 1;
    assert!(tick(&mut fx).unwrap());
    let a = &fx.world.animators[&SIM];
    assert_eq!((a.motion.segment, a.motion.near_end), (1, 0));
    assert_ne!(a.state, 3);
    // Marked as near the end: it stops on the spot.
    let mut fx = routed();
    fx.world.routing.routes.get_mut(&SIM).unwrap().path.pop();
    fx.sim_mut().data[9] = 1;
    let a = fx.world.animators.get_mut(&SIM).unwrap();
    a.motion.near_end = 1;
    a.walking = true;
    assert!(tick(&mut fx).unwrap());
    // Standing where it is, facing its destination's direction, with the
    // route's end noted (2), it stops walking (0x58f117).
    let a = &fx.world.animators[&SIM];
    assert_eq!((a.state, a.arrival), (3, 2));
    assert_eq!((a.motion.near_end, a.motion.finishing), (1, 1));
    assert_eq!(a.destination, a.position);
    assert_eq!(a.position, [100.0, 200.0, 6.0].map(F32::from_f32));
    assert!(!a.walking);
}

#[test]
fn a_blocked_step_ends_the_route_blocked() {
    // A step onto a tile where the person does not fit (0x58eecb): the
    // lot has no floors, and without data word 42 flag 2 the person may
    // not stand without one. The route ends blocked (+0x188 = 3,
    // 0x58f682); the step is kept, but neither the person nor the
    // animator moves.
    let mut fx = routed();
    fx.sim_mut().data[1] = 2;
    fx.world.animators.get_mut(&SIM).unwrap().position[2] = F32::from_f32(2.0);
    fx.world.animators.get_mut(&SIM).unwrap().destination[2] = F32::from_f32(2.0);
    fx.sim_mut().data[42] = 0;
    let before = fx.world.animators[&SIM].position;
    let placed = fx.world.objects[&SIM].position;
    tick(&mut fx).unwrap();
    let a = &fx.world.animators[&SIM];
    assert_eq!(a.arrival, 3);
    assert!(a.walking);
    assert_eq!(a.position[..2], before[..2]);
    assert_ne!(a.motion.step, [F32::ZERO; 2]);
    assert_eq!(fx.world.objects[&SIM].position, placed);
}

#[test]
fn a_blocked_last_step_at_the_end_of_the_path_ends_the_route_blocked() {
    // Near the end of its path with a clip in the foreground, a person
    // whose object data word 9 is set stands where it is and takes one
    // more step (0x58e846). When that step is blocked (0x58eecb, as
    // above), the route ends blocked (3, 0x58f682) and the state's code
    // does not run again, so the standing animator does not turn the 3
    // into the route's end (2). The stand's writes before the step stay,
    // and neither the person nor the animator moves.
    let mut fx = routed();
    fx.world.routing.routes.get_mut(&SIM).unwrap().path.pop();
    fx.sim_mut().data[9] = 1;
    fx.sim_mut().data[42] = 0;
    let a = fx.world.animators.get_mut(&SIM).unwrap();
    a.position[2] = F32::from_f32(2.0);
    a.destination[2] = F32::from_f32(2.0);
    a.motion.near_end = 1;
    a.motion.step = [16.0, 0.0].map(F32::from_f32);
    a.walking = true;
    a.foreground = Some(sims_sim::animator::Layer::start(
        CLIP,
        F32::from_bits(HUNDRED),
        4,
        true,
        0,
        false,
        0,
    ));
    let before = a.position;
    let placed = fx.world.objects[&SIM].position;
    tick(&mut fx).unwrap();
    let a = &fx.world.animators[&SIM];
    assert_eq!(a.arrival, 3);
    assert_eq!((a.state, a.motion.near_end, a.motion.finishing), (3, 1, 1));
    assert_eq!(a.position, before);
    assert_eq!(a.destination, before);
    assert!(a.walking);
    assert_ne!(a.motion.step, [16.0, 0.0].map(F32::from_f32));
    assert_eq!(fx.world.objects[&SIM].position, placed);
}

#[test]
fn what_the_route_follow_does_not_port_fails_loudly() {
    // Facing the point ahead, a standing animator walks; but the point,
    // 21.33 away, is farther than the stop clip's stride (1 times 16/3),
    // so it starts with the stop clip instead (state 6, 0x58ed52, then
    // 0x58e125), still standing. (With data word 17 1 it goes to state 11
    // instead; see a_run_starts_with_the_stop_from_a_run_then_runs.)
    let mut fx = routed();
    fx.sim_mut().data[1] = 2;
    fx.world.animators.get_mut(&SIM).unwrap().position[2] = F32::from_f32(2.0);
    fx.world.animators.get_mut(&SIM).unwrap().destination[2] = F32::from_f32(2.0);
    fx.program.animations.insert(
        "walk.far",
        Clip {
            name: "test-stop".into(),
            skill: SKILL.into(),
            duration: HUNDRED,
            distance: 0x3f80_0000,
            moving: true,
            frames: 4,
            motions: 1,
            tracks: Vec::new(),
            channels: Vec::new(),
            samples: [0, 0],
        },
    );
    tick(&mut fx).unwrap();
    let a = &fx.world.animators[&SIM];
    assert_eq!((a.state, a.selection, a.walking), (6, 0x13, false));
    // The end of a path for a person whose object data word 9 is clear
    // (0x58e8be) needs the walk's footfalls, which this walk clip has no
    // events for (0x588df8).
    let mut fx = routed();
    fx.world.routing.routes.get_mut(&SIM).unwrap().path.pop();
    fx.sim_mut().data[9] = 0;
    assert!(tick(&mut fx).unwrap_err().to_string().contains("0x588df8"));
    // Stopping at the end of a path for a person whose word 64 is set
    // (0x58e83b).
    let mut fx = routed();
    fx.world.routing.routes.get_mut(&SIM).unwrap().path.pop();
    fx.sim_mut().data[9] = 1;
    fx.sim_mut().person.as_mut().unwrap().data[64] = 1;
    fx.world.animators.get_mut(&SIM).unwrap().motion.near_end = 1;
    assert!(tick(&mut fx).unwrap_err().to_string().contains("0x58e83b"));
    // No walk clip to set the look-ahead.
    let mut fx = routed();
    fx.program
        .libraries
        .get_mut(&GLOBAL)
        .unwrap()
        .strings
        .get_mut(&150)
        .unwrap()[0x14] = String::new();
    assert!(tick(&mut fx).unwrap_err().to_string().contains("clip 20"));
}

/// The routed Sim running: object data word 17 is 1, and table 150 also
/// names the run (clip 0x15), moving 32 per cycle, twice the walk, and
/// the stop from a run (clip 0x16), moving 1.
fn running() -> Fixture {
    let mut fx = routed();
    fx.sim_mut().data[17] = 1;
    name_clip(&mut fx, 0x15, "test-run", 0x4200_0000);
    name_clip(&mut fx, 0x16, "test-stop-run", 0x3f80_0000);
    fx
}

#[test]
fn a_run_looks_ahead_by_the_run_clips_distance() {
    // 0x58e6cb: with data word 17 set the look-ahead is half of +0x1fc,
    // the run clip's (0x15) halved distance times 16/3: 32 halved, times
    // 16/3, halved, 42.67, twice the walk's. Squared 1820.4, it is
    // reached 43 units along segment 1 (progress 0.215, summed in single
    // precision), where the walk's is reached at 0.11. The point lies
    // behind, so the animator turns back as a walk does (state 4).
    let mut fx = running();
    tick(&mut fx).unwrap();
    let a = &fx.world.animators[&SIM];
    assert_eq!((a.motion.segment, a.state), (1, 4));
    let progress = a.motion.progress.to_bits();
    assert_eq!(progress, 0x3e5c_28f1);
    // The walk clip's distance is not read: halving it changes nothing.
    let mut fx = running();
    name_clip(&mut fx, 0x14, "test-walk", 0x4100_0000);
    tick(&mut fx).unwrap();
    assert_eq!(fx.world.animators[&SIM].motion.progress.to_bits(), progress);
}

#[test]
fn a_run_starts_with_the_stop_from_a_run_then_runs() {
    // Facing the point ahead, 42.67 away, a standing animator with data
    // word 17 1 is farther from it than the stride of the stop from a run
    // (+0x204, clip 0x16: 1 times 16/3), so it goes to state 11
    // (0x58ed52). The stop from a walk (clip 0x13, 32 times 16/3) would
    // not have stopped it.
    let mut fx = running();
    fx.sim_mut().data[1] = 2;
    let a = fx.world.animators.get_mut(&SIM).unwrap();
    a.position[2] = F32::from_f32(2.0);
    a.destination[2] = F32::from_f32(2.0);
    tick(&mut fx).unwrap();
    // State 11 (0x58e377) is state 6's code with clip 0x16: it starts
    // that clip to stop at its end, still standing, at the speed wanted
    // in state 11 (1.2 plus a fifth: above state 9, the base is 1), and
    // heads for the clip's distance times 16/3 along direction 2, east:
    // 105.33 (clip 0x13's would be 270.67).
    let a = &fx.world.animators[&SIM];
    assert_eq!((a.state, a.selection, a.walking), (11, 0x16, false));
    let layer = a.foreground.as_ref().unwrap();
    assert_eq!((layer.clip.as_str(), layer.mode), ("test-stop-run", 0));
    assert_eq!((a.rate, a.target), (layer.speed, layer.speed));
    assert_eq!(a.rate.to_bits(), 0x3fb3_3334);
    assert_eq!(a.motion.turn_from[0].to_bits(), 0x42c8_0000);
    assert_eq!(
        a.destination[..2]
            .iter()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>(),
        [0x42d2_aaab, 0x4348_0000]
    );
    // At the clip's end it stands at the destination and goes to state
    // 10 (0x58e494), not 9, whose code runs: it runs on with clip 0x15,
    // repeating.
    let mut ticks = 0;
    while fx.world.animators[&SIM].state == 11 {
        tick(&mut fx).unwrap();
        ticks += 1;
        assert!(ticks < 10);
    }
    let a = &fx.world.animators[&SIM];
    assert_eq!((a.state, a.selection, a.walking), (10, 0x15, true));
    let layer = a.foreground.as_ref().unwrap();
    assert_eq!((layer.clip.as_str(), layer.mode), ("test-run", 1));
    // With data word 17 clear the same walk stops with clip 0x13 in
    // state 6 and ends in state 9, walking with clip 0x14.
    let mut fx = routed();
    fx.sim_mut().data[1] = 2;
    let a = fx.world.animators.get_mut(&SIM).unwrap();
    a.position[2] = F32::from_f32(2.0);
    a.destination[2] = F32::from_f32(2.0);
    name_clip(&mut fx, 0x13, "test-stop", 0x3f80_0000);
    tick(&mut fx).unwrap();
    let a = &fx.world.animators[&SIM];
    assert_eq!((a.state, a.selection), (6, 0x13));
    let mut ticks = 0;
    while fx.world.animators[&SIM].state == 6 {
        tick(&mut fx).unwrap();
        ticks += 1;
        assert!(ticks < 10);
    }
    let a = &fx.world.animators[&SIM];
    assert_eq!((a.state, a.selection, a.walking), (9, 0x14, true));
}

/// The standing Sim of kind `kind` (person word 65), arrived, facing
/// `from` while its destination faces 2.5625, within the epsilon of its
/// destination's coordinates: an adult (word 58) with no special tables
/// (word 64), whose posture names clip 10, the turn of less than half a
/// direction.
fn facing(kind: i16, from: f32) -> Fixture {
    let mut fx = standing();
    let person = fx.sim_mut().person.as_mut().unwrap();
    person.data[58] = 20;
    person.data[64] = 0;
    person.data[65] = kind;
    name_clip(&mut fx, 10, "test-small-turn", 0);
    let [half, quarter] = [0x3f00_0000, 0x3e80_0000].map(F32::from_bits);
    fx.world.animators.insert(
        SIM,
        sims_sim::animator::Animator {
            arrival: -1,
            position: [
                F32::from_bits(0x3f00_0005),
                F32::from_bits(0x3e80_0003),
                F32::from_f32(from),
            ],
            destination: [half, quarter, F32::from_f32(2.5625)],
            ..LOADED
        },
    );
    fx
}

#[test]
fn a_pet_turns_at_once_only_within_a_sixteenth_over_the_speed() {
    // 0x58df67: a person of kind above 1 turns at once within 1 times
    // 1/16 (0x667278) over the speed, not a quarter. An eighth of a
    // direction is within a quarter over the speed for kinds 0 and 1:
    // they face the destination at once and state 3 notes the route's
    // end.
    for kind in [0, 1] {
        let mut fx = facing(kind, 2.4375);
        tick(&mut fx).unwrap();
        let a = &fx.world.animators[&SIM];
        assert_eq!((a.state, a.arrival, a.selection), (3, 2, -1), "{kind}");
        assert_eq!(a.position[2].to_bits(), 0x4024_0000);
    }
    // For kinds 2 and 3 it is not: they turn with clip 10 (state 4).
    for kind in [2, 3] {
        let mut fx = facing(kind, 2.4375);
        tick(&mut fx).unwrap();
        let a = &fx.world.animators[&SIM];
        assert_eq!((a.state, a.selection), (4, 10), "{kind}");
        assert_eq!(a.motion.turn_wanted.to_bits(), 0x3e00_0000);
        let layer = a.foreground.as_ref().unwrap();
        assert_eq!(
            (layer.clip.as_str(), layer.priority),
            ("test-small-turn", 5)
        );
    }
    // A thirty-second of a direction is within the sixteenth over the
    // speed: a pet faces the destination at once too.
    let mut fx = facing(2, 2.53125);
    tick(&mut fx).unwrap();
    let a = &fx.world.animators[&SIM];
    assert_eq!((a.state, a.arrival, a.selection), (3, 2, -1));
    assert_eq!(a.position[2].to_bits(), 0x4024_0000);
}

fn draw(fx: &mut Fixture) -> Result<(), sims_vm::Fault> {
    let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    interpreter.draw_frame(&mut fx.world, 1)
}

/// The standing Sim after its first tick, turned part of the way from
/// direction 2 towards 3 as a turn leaves it.
fn turning() -> Fixture {
    let mut fx = standing();
    tick(&mut fx).unwrap();
    // The fixture's definition word 102 (1102) has bit 3, which makes the
    // body follow person word 8.
    sim_definition(&mut fx)[102] &= !8;
    let a = fx.world.animators.get_mut(&SIM).unwrap();
    assert!(a.undrawn);
    a.position[2] = F32::from_bits(0x4010_0000);
    a.destination[2] = F32::from_bits(0x4040_0000);
    fx
}

#[test]
fn the_first_draw_places_the_animator_again() {
    // 0x590c05: byte +0xdb unsets the position (0x589361), and the
    // placement (0x592132) puts the person's tile and direction into the
    // position and the destination.
    let mut fx = turning();
    fx.sim_mut().data[1] = 5;
    draw(&mut fx).unwrap();
    let a = &fx.world.animators[&SIM];
    assert!(!a.undrawn);
    let bits = |v: [F32; 3]| v.map(F32::to_bits);
    let placed = [0x42c8_0000, 0x4348_0000, 0x40a0_0000];
    assert_eq!(bits(a.position), placed);
    assert_eq!(bits(a.destination), placed);
    // Later draws change nothing.
    let a = fx.world.animators.get_mut(&SIM).unwrap();
    a.position[2] = F32::from_bits(0x4010_0000);
    let before = fx.world.hash();
    draw(&mut fx).unwrap();
    assert_eq!(fx.world.hash(), before);
}

#[test]
fn a_person_the_view_does_not_draw_keeps_its_animator() {
    // 0x4ab56d: tiles 1 to the lot's size minus 2, and the level shown.
    for position in [[8, 200, 1], [100, 16 * 63, 1], [100, 200, 2]] {
        let mut fx = turning();
        fx.sim_mut().position = position;
        fx.sim_mut().person.as_mut().unwrap().data[40] = 0;
        let before = fx.world.hash();
        draw(&mut fx).unwrap();
        assert_eq!(fx.world.hash(), before, "{position:?}");
    }
    // One level up, with person word 40 set, it is drawn.
    let mut fx = turning();
    fx.sim_mut().position = [100, 200, 2];
    fx.sim_mut().person.as_mut().unwrap().data[40] = 1;
    draw(&mut fx).unwrap();
    assert!(!fx.world.animators[&SIM].undrawn);
}

#[test]
fn a_draw_reads_the_appearance_and_clears_the_body_word() {
    // Byte +0xd8 reads the appearance (person words 22 and 23 become 0);
    // with person word 65 bit 1 or 2, word 8 becomes 0 (0x58a2e5).
    let mut fx = turning();
    fx.world.animators.get_mut(&SIM).unwrap().profile_stale = true;
    let person = fx.sim_mut().person.as_mut().unwrap();
    (person.data[22], person.data[8], person.data[65]) = (4, 3, 2);
    draw(&mut fx).unwrap();
    let person = fx.sim().person.as_ref().unwrap();
    assert_eq!((person.data[22], person.data[8]), (0, 0));
    assert!(!fx.world.animators[&SIM].profile_stale);
    // Without those bits word 8 stays.
    let mut fx = turning();
    let person = fx.sim_mut().person.as_mut().unwrap();
    (person.data[8], person.data[65]) = (3, 1);
    draw(&mut fx).unwrap();
    assert_eq!(fx.sim().person.as_ref().unwrap().data[8], 3);
}

#[test]
fn what_a_draw_does_not_port_fails_loudly() {
    let failed = |fx: &mut Fixture| match draw(fx).unwrap_err().trap {
        Trap::Unsupported(message) => message,
        other => panic!("{other:?}"),
    };
    // The skin waits for its update.
    let mut fx = turning();
    fx.world.animators.get_mut(&SIM).unwrap().skin_dirty = true;
    assert!(failed(&mut fx).contains("skin"));
    // Drawn before, and not placed.
    let mut fx = turning();
    let a = fx.world.animators.get_mut(&SIM).unwrap();
    a.undrawn = false;
    a.position[0] = UNSET;
    assert!(failed(&mut fx).contains("+0xda"));
    // A body that follows word 8.
    let mut fx = turning();
    sim_definition(&mut fx)[102] |= 8;
    fx.sim_mut().person.as_mut().unwrap().data[8] = 1;
    assert!(failed(&mut fx).contains("word 8"));
    // The type in the view's second list.
    let mut fx = turning();
    sim_definition(&mut fx)[101] |= 2;
    assert!(failed(&mut fx).contains("second list"));
}

/// The standing Sim sitting (posture 1) with its base layer free to start:
/// definition word 102 clear, person word 64 clear, and an adult, so its
/// clips are named by the global file's adult tables (152 and 153).
/// Names posture 0's base clip (its clip 3) [`CLIP`], in the global
/// table 150, for an adult whose word 64 is clear.
fn name_standing_clip(fx: &mut Fixture) {
    let person = fx.sim_mut().person.as_mut().unwrap();
    person.data[58] = 30;
    person.data[64] = 0;
    fx.program
        .libraries
        .get_mut(&GLOBAL)
        .unwrap()
        .strings
        .insert(
            150,
            vec![String::new(), String::new(), String::new(), CLIP.into()],
        );
}

fn sitting(name: &str) -> Fixture {
    let mut fx = standing();
    fx.program
        .libraries
        .get_mut(&SIM_FILE)
        .unwrap()
        .definitions
        .get_mut(&128)
        .unwrap()[102] = 0;
    let person = fx.sim_mut().person.as_mut().unwrap();
    person.data[0] = 1;
    person.data[58] = 30;
    person.data[64] = 0;
    for table in [152, 153] {
        fx.program
            .libraries
            .get_mut(&GLOBAL)
            .unwrap()
            .strings
            .insert(table, vec![String::new(), name.into()]);
    }
    fx
}

#[test]
fn the_base_update_keeps_a_scripts_clip_until_the_posture_changes() {
    // 0x590430 for a standing Sim whose definition word 102 lets the base
    // layer start, with a clip Animate Sim put in the base slot.
    let script = || Base::Script {
        clip: CLIP.into(),
        reverse: false,
        start: 0,
    };
    let base_after = |kind: i16, noted: i32, flags: i16| {
        let mut fx = standing();
        sim_definition(&mut fx)[102] = flags;
        name_standing_clip(&mut fx);
        fx.sim_mut().person.as_mut().unwrap().data[65] = kind;
        fx.world.animators.insert(
            SIM,
            sims_sim::animator::Animator {
                posture: noted,
                base: script(),
                ..LOADED
            },
        );
        let mut layer = Layer::start(CLIP, F32::from_bits(HUNDRED), 4, true, 0, false, 0);
        layer.mode = 1;
        layer.priority = -10;
        fx.world.base_layers.insert(SIM, layer);
        tick(&mut fx).unwrap();
        let a = &fx.world.animators[&SIM];
        (a.base.clone(), a.posture)
    };
    // A person of kind at most 1 keeps it while the posture is the one
    // noted.
    assert_eq!(base_after(0, 0, 0), (script(), 0));
    assert_eq!(base_after(1, 0, 0), (script(), 0));
    // Any other kind has it replaced by the posture's clip.
    assert_eq!(base_after(2, 0, 0), (Base::Posture, 0));
    // Another posture noted clears it, and the posture's clip starts.
    assert_eq!(base_after(0, 1, 0), (Base::Posture, 0));
    // With bit 2 of word 102 set, it is cleared and nothing starts; with
    // the posture unchanged, it stays whatever the kind.
    assert_eq!(base_after(0, 1, 4), (Base::Empty, 1));
    assert_eq!(base_after(2, 0, 4), (script(), 0));
    // The reset (0x589220) empties the slot too.
    let mut a = sims_sim::animator::Animator {
        base: script(),
        ..LOADED
    };
    a.reset().unwrap();
    assert!(a.base == Base::Empty && a.skin_dirty);
}

#[test]
fn postures_1_and_2_start_their_base_clip_once_it_is_found() {
    // Posture 1's base clip is its clip 1, found without timed events: the
    // base layer starts.
    let mut fx = sitting(CLIP);
    assert!(
        fx.world
            .animators
            .get(&SIM)
            .is_none_or(|a| a.base == Base::Empty)
    );
    tick(&mut fx).unwrap();
    let animator = &fx.world.animators[&SIM];
    assert_eq!((animator.posture, &animator.base), (1, &Base::Posture));
    // A clip with no name starts nothing (0x5904af): the posture is noted
    // and the slot stays empty. A name no file holds fails loudly
    // (0x59dc32). A clip with timed events starts.
    let mut fx = sitting("");
    tick(&mut fx).unwrap();
    let animator = &fx.world.animators[&SIM];
    assert_eq!((animator.posture, &animator.base), (1, &Base::Empty));
    assert!(!fx.world.base_layers.contains_key(&SIM));
    let mut fx = sitting("a-clip-no-file-holds");
    assert!(tick(&mut fx).unwrap_err().to_string().contains("0x59dc32"));
    let mut fx = sitting(CLIP);
    fx.program.animations.insert(
        "test.far",
        Clip {
            name: CLIP.into(),
            skill: SKILL.into(),
            duration: HUNDRED,
            distance: 0,
            moving: false,
            frames: 4,
            motions: 1,
            tracks: vec![track(&[(0, &[("xevt", "1")])])],
            channels: Vec::new(),
            samples: [0, 0],
        },
    );
    tick(&mut fx).unwrap();
    assert_eq!(fx.world.animators[&SIM].base, Base::Posture);
    assert_eq!(fx.world.base_layers[&SIM].clip, CLIP);
    // Posture 2's base clip is its clip 2, not clip 1: with the name at
    // clip 2 the layer starts, and with it at clip 1 only, clip 2 has no
    // name.
    for (names, found) in [(["", "x", CLIP], true), (["", CLIP, ""], false)] {
        let mut fx = sitting("");
        fx.sim_mut().person.as_mut().unwrap().data[0] = 2;
        for table in [152, 153] {
            fx.program
                .libraries
                .get_mut(&GLOBAL)
                .unwrap()
                .strings
                .insert(table, names.iter().map(|n| n.to_string()).collect());
        }
        tick(&mut fx).unwrap();
        let animator = &fx.world.animators[&SIM];
        let base = if found { Base::Posture } else { Base::Empty };
        assert_eq!((animator.posture, &animator.base), (2, &base));
    }
}

#[test]
fn a_seated_animator_clears_a_clip_still_playing_and_stops_walking() {
    // States 1 and 2 (0x58f0eb, 0x58f12e): the foreground clip is cleared
    // and the base layer brought up to date, then the animator stops
    // walking. In state 1, person word 78 holds the clip.
    let seated = |state: i32, hold: i16| {
        let mut fx = standing();
        fx.sim_mut().person.as_mut().unwrap().data[78] = hold;
        fx.world.animators.insert(
            SIM,
            sims_sim::animator::Animator {
                state,
                walking: true,
                foreground: Some(layer(0)),
                ..LOADED
            },
        );
        tick(&mut fx).unwrap();
        let a = &fx.world.animators[&SIM];
        (a.foreground.is_some(), a.walking, a.state)
    };
    assert_eq!(seated(1, 0), (false, false, 1));
    assert_eq!(seated(2, 0), (false, false, 2));
    assert_eq!(seated(1, 1), (true, false, 1));
    assert_eq!(seated(2, 1), (false, false, 2));
}

/// [`standing`] running, in its own tick, a tree of one parameter that
/// takes the person off the lot (Find Location For mode 1 on itself) and
/// then runs `next`.
fn taking_itself_off(next: sims_formats::behavior::Instruction) -> Fixture {
    taking_itself_off_then(vec![next])
}

/// [`taking_itself_off`], then the nodes `then` (from node 1).
fn taking_itself_off_then(then: Vec<sims_formats::behavior::Instruction>) -> Fixture {
    let mut fx = standing();
    let mut nodes = vec![ins(16, 1, 1, [1, 0, 0, 0, 0, 0, 0, 0])];
    nodes.extend(then);
    fx.add(SIM_FILE, 4098, shaped(0x8002, 0, 1, 0, nodes));
    fx.thread(SIM, 4098, &[5]);
    fx.sim_mut().stack[0].stack_object = SIM as i16;
    // The head's countdown (person word 45) has run out: the refresh's
    // head step (0x58da12) then writes no person word.
    fx.sim_mut().person.as_mut().unwrap().data[45] = 0;
    flat_lot(&mut fx);
    fx
}

/// The person plays a clip, in its foreground layer, with one timed event
/// `key` = `value` due at the first tick's clock.
fn playing_an_event(fx: &mut Fixture, key: &str, value: &str) {
    fx.program.animations.insert(
        "test.far",
        Clip {
            name: CLIP.into(),
            skill: SKILL.into(),
            duration: HUNDRED,
            distance: 0,
            moving: false,
            frames: 4,
            motions: 1,
            tracks: vec![track(&[(0, &[(key, value)])])],
            channels: Vec::new(),
            samples: [0, 0],
        },
    );
    let mut foreground = Layer::start(CLIP, F32::from_bits(HUNDRED), 4, true, 1, false, 500);
    foreground.priority = 10;
    fx.world.animators.insert(
        SIM,
        sims_sim::animator::Animator {
            foreground: Some(foreground),
            ..LOADED
        },
    );
}

/// Sleep for the tree's parameter.
fn sleep() -> sims_formats::behavior::Instruction {
    ins(0, RETURN_TRUE, RETURN_FALSE, [0; 8])
}

#[test]
fn a_person_takes_itself_off_the_lot_in_its_own_tick() {
    // 0x433125 refreshes the person's animator (0x58da12) only if the
    // house view drew it, which is not kept. Here the refresh then the
    // tick reach what the tick alone does, so either way it is the same.
    let mut fx = taking_itself_off(sleep());
    let mut alone = standing();
    tick(&mut alone).unwrap();
    tick(&mut fx).unwrap();
    let off = fx.sim();
    assert_ne!(off.position, alone.sim().position);
    assert_eq!(off.data[2], 0);
    let (a, b) = (&fx.world.animators[&SIM], &alone.world.animators[&SIM]);
    assert_eq!(
        (a.clock, a.head_clock, a.skin_clock),
        (b.clock, b.head_clock, b.skin_clock)
    );
}

#[test]
fn after_taking_itself_off_only_what_reads_no_animator_may_run() {
    // Animate Sim reads the animator, which the refresh at the take-off
    // may have written.
    let mut fx = taking_itself_off(ins(44, RETURN_TRUE, RETURN_FALSE, [0; 8]));
    let fault = tick(&mut fx).unwrap_err().to_string();
    assert!(
        fault.contains("0x58da12") && fault.contains("opcode 44"),
        "{fault}"
    );
}

#[test]
fn a_person_holding_something_in_its_hand_taking_itself_off_fails_loudly() {
    // The refresh is taken to read only the head slot (0x58dae5).
    let mut fx = taking_itself_off(sleep());
    fx.sim_mut().slots[1].object = LAMP as i16;
    let fault = tick(&mut fx).unwrap_err().to_string();
    assert!(fault.contains("0x58dae5"), "{fault}");
}

#[test]
fn what_the_take_offs_callbacks_put_in_the_hand_is_checked_after_them() {
    // The person's function 10 (leaving the lot) grabs the lamp into its
    // hand. The hand was empty before the callbacks; the check after them
    // (as the refresh comes after them) still fails.
    let mut fx = taking_itself_off(sleep());
    for slot in &mut fx.lamp_mut().slots {
        slot.object = 0;
    }
    // Its tile slot and the five a person adds (0x45962b), all empty.
    let empty = fx.sim().slots[1].clone();
    fx.sim_mut().slots.resize(6, empty);
    // Live mode (global 19), and the lamp may be carried (data word 4
    // bit 5).
    fx.world.globals[19] = 0;
    fx.lamp_mut().data[4] |= 0x20;
    let library = fx.program.libraries.get_mut(&SIM_FILE).unwrap();
    let table = library.callbacks.get_mut(&128).unwrap();
    table.resize_with(11, || sims_formats::execution::ObjectFunction {
        condition: 0,
        action: 0,
    });
    table[10].action = 4099;
    fx.add(
        SIM_FILE,
        4099,
        tree(vec![
            assign(scope::STACK_OBJECT, 0, scope::LITERAL, LAMP as i16, 1),
            ins(4, RETURN_TRUE, RETURN_FALSE, [0; 8]),
        ]),
    );
    let fault = tick(&mut fx).unwrap_err().to_string();
    assert!(fault.contains("0x58dae5"), "{fault}");
    assert_eq!(fx.sim().slots[1].object, LAMP as i16);
}

#[test]
fn a_tile_of_a_group_in_the_head_slot_taking_itself_off_fails_loudly() {
    let mut fx = taking_itself_off(sleep());
    for slot in &mut fx.lamp_mut().slots {
        slot.object = 0;
    }
    fx.lamp_mut().multipart = Some(sims_formats::saved_objects::Multipart {
        offset: [0; 3],
        group: [0; 3],
        parent: 0,
    });
    fx.sim_mut().slots[2].object = LAMP as i16;
    let fault = tick(&mut fx).unwrap_err().to_string();
    assert!(
        fault.contains("0x58dc08") && fault.contains("the group tile 1"),
        "{fault}"
    );
}

#[test]
fn an_ordinary_object_in_the_head_slot_is_redrawn_as_presentation() {
    // The refresh reads the container entry at begin + 0x48, our slot 2,
    // and redraws what it holds (0x58dae5 to 0x58dc55): for an ordinary
    // object it writes only the object's graphic, as the animator's tick
    // then does again. So the take-off goes on, the lamp leaving the lot
    // with the person.
    let mut fx = taking_itself_off(sleep());
    for slot in &mut fx.lamp_mut().slots {
        slot.object = 0;
    }
    assert!(fx.sim().slots.len() > 2);
    fx.sim_mut().slots[2].object = LAMP as i16;
    tick(&mut fx).unwrap();
    assert_eq!(fx.world.objects[&LAMP].position, fx.sim().position);
    assert_eq!(fx.sim().data[2], 0);
}

#[test]
fn a_person_in_the_head_slot_taking_itself_off_fails_loudly() {
    // A person there has another class (0x667378), whose functions the
    // redraw would call (0x58dc08 to 0x58dc52).
    let mut fx = taking_itself_off(sleep());
    fx.sim_mut().slots[2].object = SIM as i16;
    let fault = tick(&mut fx).unwrap_err().to_string();
    assert!(
        fault.contains("0x58dc08") && fault.contains("the person 2"),
        "{fault}"
    );
}

#[test]
fn a_refresh_whose_event_writes_a_hand_fails_at_the_take_off() {
    // The person's playing clip has a "lefthand" event due at the clock:
    // the refresh at the take-off (0x58da12) writes person word 22 there
    // (0x5914ab), before the thread reads it. The thread copies word 22
    // and then sets it to what the event writes, so at the animator's
    // tick nothing tells the branches apart; the refresh is worked out at
    // the take-off, and fails there.
    let mut fx = taking_itself_off_then(vec![
        assign(scope::TEMPS, 0, 18, 22, 2),
        assign(18, 22, scope::LITERAL, 3, 3),
        sleep(),
    ]);
    playing_an_event(&mut fx, "lefthand", "3");
    fx.sim_mut().person.as_mut().unwrap().data[22] = 0;
    let fault = tick(&mut fx).unwrap_err().to_string();
    assert!(
        fault.contains("0x58da12") && fault.contains("write more than its animator"),
        "{fault}"
    );
    assert_eq!(fx.sim().stack[0].node, 0, "the thread went no further");
}

#[test]
fn a_refresh_whose_event_plays_a_sound_fails_at_the_take_off() {
    // For a selected person, "selectedsound" plays (0x59158c, 0x59175e,
    // 0x57a54c); the sounds of timed events are not modeled, so in the
    // refresh any sound key fails. The tick alone passes over it.
    for key in ["sound", "selectedsound", "deselectedsound", "footstep"] {
        let mut fx = taking_itself_off(sleep());
        playing_an_event(&mut fx, key, "x");
        let fault = tick(&mut fx).unwrap_err().to_string();
        assert!(
            fault.contains("0x58da12") && fault.contains(&format!("{key:?}")),
            "{fault}"
        );
        let mut alone = standing();
        playing_an_event(&mut alone, key, "x");
        tick(&mut alone).unwrap();
    }
}

#[test]
fn the_skins_update_notes_the_bones_its_layers_move() {
    // Every motion with positions of a layer the skin's update samples
    // (0x597c4c, 0x598226); a motion with rotations only moves nothing.
    let mut fx = standing();
    playing_an_event(&mut fx, "xevt", "1");
    let clip = fx.program.animations.clip(CLIP).unwrap().clone();
    let channel = |bone: &str, translation| sims_content::Channel {
        bone: bone.into(),
        translation,
        rotation: Some(0),
    };
    fx.program.animations.insert(
        "test.far",
        Clip {
            channels: vec![channel("ROOT", Some(0)), channel("HEAD", None)],
            ..clip
        },
    );
    tick(&mut fx).unwrap();
    assert_eq!(fx.world.translated_bones[&SIM], ["ROOT".to_owned()].into());
    // A layer with no weight samples nothing.
    let mut fx = standing();
    playing_an_event(&mut fx, "xevt", "1");
    let a = fx.world.animators.get_mut(&SIM).unwrap();
    a.foreground.as_mut().unwrap().weight = F32::ZERO;
    tick(&mut fx).unwrap();
    assert!(!fx.world.translated_bones.contains_key(&SIM));
}

#[test]
fn a_refresh_that_would_count_the_head_down_fails_loudly() {
    // With the head's countdown running, the refresh's head step lowers
    // person word 45, which the rest of the lot update may read: whether
    // the house view drew the person would then matter.
    let mut fx = taking_itself_off(sleep());
    fx.sim_mut().person.as_mut().unwrap().data[45] = 2;
    let fault = tick(&mut fx).unwrap_err().to_string();
    assert!(
        fault.contains("0x58da12") && fault.contains("write more than its animator"),
        "{fault}"
    );
}
