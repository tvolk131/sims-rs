//! Animate Sim (opcode 44, 0x58950b) and a person's animator tick
//! (0x58dc72). Synthetic; no game data.

use crate::support::*;
use sims_content::{Clip, Profile};
use sims_core::F32;
use sims_sim::animator::{LOADED, UNSET};
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
            frames: 4,
            motions: 1,
            events: false,
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
fn a_finished_clip_sends_the_events_it_missed_then_its_end_fails_loudly() {
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
    fx.thread(SIM, 4096, &[-7]);
    assert!(unsupported(turn(&mut fx)).contains("0x592904"));
}

#[test]
fn what_animate_sim_does_not_port_fails_loudly() {
    for (operands, what) in [
        (animate(1, 3, 0x01, 0, 0), "base slot"),
        (animate(1, 3, 0x10, 0, 0), "carry slot"),
        (animate(0, 3, 0, 0, 0), "clip 0"),
        (animate(1, 0, 0, 0, 0), "source 0"),
        (animate(2, 3, 0, 0, 0), "not listed"),
    ] {
        let mut fx = sim_animates(operands);
        let message = unsupported(turn(&mut fx));
        assert!(message.contains(what), "{message}");
    }
    // A clip whose samples are missing, and one with timed events.
    let mut fx = sim_animates(animate(1, 3, 0, 0, 0));
    fx.program.animations = Default::default();
    fx.program.animations.insert(
        "test.far",
        Clip {
            name: CLIP.into(),
            skill: SKILL.into(),
            duration: HUNDRED,
            frames: 4,
            motions: 1,
            events: false,
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
            frames: 4,
            motions: 1,
            events: true,
        },
    );
    assert!(unsupported(turn(&mut fx)).contains("defined differently"));
    // Starting a clip over one already in the foreground slot.
    let mut fx = sim_animates(animate(1, 3, 0, 0, 0));
    turn(&mut fx).unwrap();
    fx.world.animators.get_mut(&SIM).unwrap().playing = false;
    assert!(unsupported(turn(&mut fx)).contains("0x586b63"));
}

#[test]
fn a_parameter_can_name_the_clip_and_a_bad_one_is_error_8() {
    let mut fx = sim_animates(animate(0, 3, 0x04, 0, 0));
    fx.thread(SIM, 4096, &[1]);
    assert_eq!(turn(&mut fx).unwrap(), Status::Yielded);
    let mut fx = sim_animates(animate(3, 3, 0x04, 0, 0));
    assert_eq!(code(turn(&mut fx).map(|_| true)), Some(8));
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
    // The thread's next run finds the clip done; ending it is not ported.
    assert!(tick(&mut fx).unwrap_err().to_string().contains("0x592904"));
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

#[test]
fn what_the_animator_tick_does_not_port_fails_loudly() {
    // Something in the Sim's hands.
    let mut fx = standing();
    fx.sim_mut().slots[1].object = 1;
    assert!(tick(&mut fx).unwrap_err().to_string().contains("carry"));
    // A posture whose base clip would start.
    let mut fx = standing();
    fx.program
        .libraries
        .get_mut(&SIM_FILE)
        .unwrap()
        .definitions
        .get_mut(&128)
        .unwrap()[102] = 0;
    fx.sim_mut().person.as_mut().unwrap().data[0] = 1;
    assert!(tick(&mut fx).unwrap_err().to_string().contains("base clip"));
    // The head following something.
    let mut fx = standing();
    fx.sim_mut().person.as_mut().unwrap().data[42] = 1;
    assert!(tick(&mut fx).unwrap_err().to_string().contains("word 42"));
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
            .contains("turn or walk")
    );
    let mut fx = standing();
    fx.world
        .animators
        .insert(SIM, sims_sim::animator::Animator { state: 6, ..LOADED });
    assert!(tick(&mut fx).unwrap_err().to_string().contains("state 6"));
}
