//! Animate Sim (opcode 44, 0x58950b) and a person's animator tick
//! (0x58dc72). Synthetic; no game data.

use crate::support::*;
use sims_content::{Clip, Event, Profile, Track};
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
            distance: 0,
            moving: false,
            frames: 4,
            motions: 1,
            tracks: Vec::new(),
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
    fx.thread(SIM, 4096, &[-7]);
    assert_eq!(turn(&mut fx).unwrap(), Status::Returned(true));
    let a = &fx.world.animators[&SIM];
    assert!(!a.playing && a.base);
    assert_eq!((a.state, a.pickup), (3, 0));
    assert!(a.foreground.as_ref().is_some_and(|l| l.finished()));
    assert_eq!(fx.sim().person.as_ref().unwrap().data[19], 0);
    // With bit 2 set, the base layer stays off.
    let mut fx = sim_animates(animate(1, 3, 0, 0, 0));
    turn(&mut fx).unwrap();
    end_clip(&mut fx);
    assert_eq!(turn(&mut fx).unwrap(), Status::Returned(true));
    let a = &fx.world.animators[&SIM];
    assert!(!a.playing && !a.base);
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
        (animate(1, 3, 0x01, 0, 0), "base slot"),
        (animate(1, 3, 0x10, 0, 0), "carry slot"),
        (animate(1, 0, 0, 0, 0), "source 0"),
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
        },
    );
    assert!(unsupported(turn(&mut fx)).contains("defined differently"));
    // A clip that ends with the person sitting (0x590497).
    let mut fx = sim_animates(animate(1, 3, 0, 0, 0));
    turn(&mut fx).unwrap();
    end_clip(&mut fx);
    sim_definition(&mut fx)[102] = 0;
    fx.sim_mut().person.as_mut().unwrap().data[0] = 1;
    assert!(unsupported(turn(&mut fx)).contains("0x590497"));
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
            .contains("walks to its destination")
    );
    let mut fx = standing();
    fx.world
        .animators
        .insert(SIM, sims_sim::animator::Animator { state: 6, ..LOADED });
    assert!(tick(&mut fx).unwrap_err().to_string().contains("state 6"));
}

/// The standing Sim with a route to follow: the animator as the route's
/// start leaves it (0x589c85: result 1, segment 0, standing on the
/// person's tile and direction), facing direction 6, and a path east
/// along the first coordinate. Global clip table 150 names the walk
/// (clip 0x14, whose distance sets the look-ahead) and the half turn
/// (clip 5).
fn routed() -> Fixture {
    let mut fx = standing();
    fx.sim_mut().data[1] = 6;
    // Data word 17 picks another look-ahead (0x58e6d5).
    fx.sim_mut().data[17] = 0;
    // An adult (word 58) of kind 0 (word 65), with no special clip
    // tables (word 64).
    let person = fx.sim_mut().person.as_mut().unwrap();
    person.data[58] = 20;
    person.data[64] = 0;
    person.data[65] = 0;
    let names = (0..=0x14)
        .map(|i| match i {
            5 => "test-half-turn".to_string(),
            0x13 => "test-stop".to_string(),
            0x14 => "test-walk".to_string(),
            _ => String::new(),
        })
        .collect();
    fx.program
        .libraries
        .get_mut(&GLOBAL)
        .unwrap()
        .strings
        .insert(150, names);
    for (name, distance) in [
        ("test-half-turn", 0),
        ("test-stop", 0x4200_0000),
        ("test-walk", 0x4180_0000),
    ] {
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
            },
        );
    }
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
    // A flat lot of 40 tiles a side with no walls, floors, pools or
    // water, on which the Sim may stand on level 1 (data word 4 bit 0)
    // without a floor (data word 42 flag 2), for the walk's place test.
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
    fx
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

#[test]
fn what_the_route_follow_does_not_port_fails_loudly() {
    // Facing the point ahead, a standing animator walks; but the point,
    // 21.33 away, is farther than the stop clip's stride (1 times 16/3),
    // and stopping short is not ported (0x58ed52).
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
        },
    );
    assert!(
        tick(&mut fx)
            .unwrap_err()
            .to_string()
            .contains("stops short")
    );
    // A step onto a tile where the person does not fit (0x58f682): the
    // lot has no floors, and without data word 42 flag 2 the person may
    // not stand without one.
    let mut fx = routed();
    fx.sim_mut().data[1] = 2;
    fx.world.animators.get_mut(&SIM).unwrap().position[2] = F32::from_f32(2.0);
    fx.world.animators.get_mut(&SIM).unwrap().destination[2] = F32::from_f32(2.0);
    fx.sim_mut().data[42] = 0;
    assert!(tick(&mut fx).unwrap_err().to_string().contains("blocked"));
    // A path whose second segment is its last.
    let mut fx = routed();
    fx.world.routing.routes.get_mut(&SIM).unwrap().path.pop();
    assert!(
        tick(&mut fx)
            .unwrap_err()
            .to_string()
            .contains("last segment")
    );
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
