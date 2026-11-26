//! The walk animator's way to a destination it has not reached (state 3,
//! 0x58df01), its sidestep (0x58f312, state 5), the stop of a person
//! whose word 64 is set (0x58ef8e, state 7), and states 8 and 12.
//! Synthetic; no game data.

use crate::animate::{facing_along, name_clip, routed, standing, tick};
use crate::support::*;
use sims_content::{Channel, Clip};
use sims_core::F32;
use sims_sim::animator::{Animator, LOADED, Layer};

/// 100 milliseconds, as float bits.
const HUNDRED: u32 = 0x42c8_0000;

/// The sidestep clips by sector (0x58fad6).
const SIDESTEPS: [usize; 8] = [15, 13, 11, 12, 14, 16, 18, 17];

/// An adult (word 58) of kind `kind` (word 65) with no pool tables (word
/// 64), placed at (100, 200) facing direction 2, its destination `offset`
/// away, facing direction `facing`. Clip 10 (the turn of none) and the
/// sidestep clips are named; the sidesteps have no root motion.
fn walking_to(kind: i16, offset: [f32; 2], facing: f32) -> Fixture {
    let mut fx = standing();
    let person = fx.sim_mut().person.as_mut().unwrap();
    person.data[58] = 20;
    person.data[64] = 0;
    person.data[65] = kind;
    for selection in [4, 5, 6, 7, 8, 9, 10] {
        name_clip(&mut fx, selection, &format!("test-turn-{selection}"), 0);
    }
    for selection in SIDESTEPS {
        name_clip(&mut fx, selection, &format!("test-side-{selection}"), 0);
    }
    fx.world.animators.insert(
        SIM,
        Animator {
            position: [100.0, 200.0, 2.0].map(F32::from_f32),
            destination: [100.0 + offset[0], 200.0 + offset[1], facing].map(F32::from_f32),
            ..LOADED
        },
    );
    fx
}

/// Names the posture's clip `selection` and gives it a root motion (bone
/// `ROOT`, the clip's first) of four frames whose last position is `root`,
/// in a sample file of its own.
fn root_clip(fx: &mut Fixture, selection: usize, root: [f32; 3]) {
    let name = format!("test-side-{selection}");
    name_clip(fx, selection, &name, 0);
    let skill = format!("{name}-skill");
    // The sample file holds each axis's four values in turn, each a
    // literal (code 255 and the float).
    let mut bytes = Vec::new();
    for value in root {
        for frame in 0..4 {
            bytes.push(255);
            let v = if frame == 3 { value } else { 0.0 };
            bytes.extend(v.to_le_bytes());
        }
    }
    fx.program.animations.insert(
        "walk.far",
        Clip {
            name: name.clone(),
            skill: skill.clone(),
            duration: HUNDRED,
            distance: 0,
            moving: false,
            frames: 4,
            motions: 1,
            tracks: Vec::new(),
            channels: vec![Channel {
                bone: "ROOT".into(),
                translation: Some(0),
                rotation: None,
            }],
            samples: [4, 0],
        },
    );
    fx.program.animations.insert_sample_file(&skill, bytes);
}

fn bits(v: [F32; 3]) -> [u32; 3] {
    v.map(F32::to_bits)
}

fn floats(v: [f32; 3]) -> [u32; 3] {
    v.map(f32::to_bits)
}

/// Ticks until the animator leaves `state`, at most `limit` times.
fn tick_while(fx: &mut Fixture, state: i32, limit: usize) -> usize {
    let mut ticks = 0;
    while fx.world.animators[&SIM].state == state {
        tick(fx).unwrap();
        ticks += 1;
        assert!(ticks <= limit, "still in state {state}");
    }
    ticks
}

#[test]
fn a_destination_nearer_than_the_snap_is_reached_at_once() {
    // 0.35 away, below 0.5 (0x6d07e4): the position takes the
    // destination's (0x58dfcf), and state 3 runs again and finds it
    // arrived. No clip plays.
    let mut fx = walking_to(0, [0.25, 0.25], 2.0);
    tick(&mut fx).unwrap();
    let a = &fx.world.animators[&SIM];
    assert_eq!((a.state, a.arrival, a.selection), (3, 0, -1));
    assert!(a.foreground.is_none());
    assert_eq!(bits(a.position), floats([100.25, 200.25, 2.0]));
    assert_eq!(a.destination, a.position);
}

#[test]
fn a_destination_within_the_short_distance_is_stepped_to_sideways() {
    // One unit east, facing the way it should: below 3.1 (0x6d07b0), so
    // a sidestep. The angle 0 is sector 0 (0 + 1/16 of a turn, times 8):
    // clip 15. Its root motion ends at (0.5, 0.25): the clip itself moves
    // the person that far, so the destination goes back by it (cosine 1,
    // sine 0). A distance of 1 or more gives the clip its full weight.
    let mut fx = walking_to(0, [1.0, 0.0], 2.0);
    root_clip(&mut fx, 15, [0.5, 0.25, 7.0]);
    tick(&mut fx).unwrap();
    let a = &fx.world.animators[&SIM];
    assert_eq!((a.state, a.selection), (5, 15));
    assert_eq!(bits(a.motion.turn_from), floats([100.0, 200.0, 2.0]));
    assert_eq!(bits(a.motion.turn_destination), floats([101.0, 200.0, 2.0]));
    assert_eq!(bits(a.destination), floats([100.5, 199.75, 2.0]));
    assert_eq!(bits(a.position), floats([100.0, 200.0, 2.0]));
    let layer = a.foreground.as_ref().unwrap();
    assert_eq!(layer.clip, "test-side-15");
    assert_eq!(layer.weight.to_bits(), 1f32.to_bits());
    assert_eq!(layer.speed, a.rate);
    assert_eq!(layer.mode, 0);
    // The raised priority of the next foreground clip is used up.
    assert_eq!(layer.priority, 6);
    assert!(!a.promote);
    // State 5 eases towards the destination as the clip plays (from the
    // phase the tail left: the clip started at this tick's clock, so the
    // next tick still finds phase 0).
    tick(&mut fx).unwrap();
    assert_eq!(
        bits(fx.world.animators[&SIM].position),
        floats([100.0, 200.0, 2.0])
    );
    tick(&mut fx).unwrap();
    let a = &fx.world.animators[&SIM];
    assert_eq!(a.state, 5);
    let x = a.position[0].to_f32();
    assert!(x > 100.0 && x < 100.5, "{x}");
    // At the clip's end the animator stands where the step was to end:
    // the noted destination, not the moved one; the next clip's priority
    // is not raised again (unlike a turn's end).
    tick_while(&mut fx, 5, 10);
    let a = &fx.world.animators[&SIM];
    assert_eq!((a.state, a.arrival, a.selection), (3, 0, -1));
    assert!(a.foreground.is_none());
    assert_eq!(bits(a.position), floats([101.0, 200.0, 2.0]));
    assert_eq!(a.destination, a.position);
    assert!(!a.promote);
    assert_eq!(fx.sim().data[1], 2);
}

#[test]
fn the_sidestep_clip_follows_the_angle_to_the_destination() {
    // Each eighth of a turn, from east counterclockwise, starting half a
    // sector early (0x58f33a adds 1/16 of a turn).
    let offsets = [
        [2.0, 0.0],
        [2.0, 2.0],
        [0.0, 2.0],
        [-2.0, 2.0],
        [-2.0, 0.0],
        [-2.0, -2.0],
        [0.0, -2.0],
        [2.0, -2.0],
    ];
    for (offset, clip) in offsets.into_iter().zip(SIDESTEPS) {
        let mut fx = walking_to(0, offset, 2.0);
        tick(&mut fx).unwrap();
        let a = &fx.world.animators[&SIM];
        assert_eq!((a.state, a.selection), (5, clip as i32), "{offset:?}");
        // No root motion: the destination stays, and the weight is the
        // distance over a length of 0.
        assert_eq!(a.destination, a.motion.turn_destination);
        assert_eq!(
            a.foreground.as_ref().unwrap().weight.to_bits(),
            1f32.to_bits(),
            "{offset:?}"
        );
    }
    // Just short of an eighth past east is still sector 0; at 1/16 of a
    // turn the next.
    let mut fx = walking_to(0, [2.0, 0.8], 2.0);
    tick(&mut fx).unwrap();
    assert_eq!(fx.world.animators[&SIM].selection, 15);
    let mut fx = walking_to(0, [2.0, 0.9], 2.0);
    tick(&mut fx).unwrap();
    assert_eq!(fx.world.animators[&SIM].selection, 13);
}

#[test]
fn a_sidestep_nearer_than_1_is_weighted_by_its_share_of_the_clip() {
    // 0.75 away with a root motion 1 long: weight 0.75.
    let mut fx = walking_to(0, [0.75, 0.0], 2.0);
    root_clip(&mut fx, 15, [0.6, 0.8, 0.0]);
    tick(&mut fx).unwrap();
    let a = &fx.world.animators[&SIM];
    assert_eq!(a.selection, 15);
    let weight = a.foreground.as_ref().unwrap().weight;
    assert_eq!(weight.to_bits(), 0.75f32.to_bits());
    // 0.6 away with one 2 long: 0.3, held to at least 0.5 (0x6d07e0).
    let mut fx = walking_to(0, [0.6, 0.0], 2.0);
    root_clip(&mut fx, 15, [1.2, 1.6, 0.0]);
    tick(&mut fx).unwrap();
    let weight = fx.world.animators[&SIM].foreground.as_ref().unwrap().weight;
    assert_eq!(weight.to_bits(), 0.5f32.to_bits());
    // No root motion: the distance over 0, which stays.
    let mut fx = walking_to(0, [0.75, 0.0], 2.0);
    tick(&mut fx).unwrap();
    let weight = fx.world.animators[&SIM].foreground.as_ref().unwrap().weight;
    assert_eq!(weight.to_bits(), f32::INFINITY.to_bits());
}

#[test]
fn a_root_motion_that_is_not_the_clips_first_fails_loudly() {
    let mut fx = walking_to(0, [1.0, 0.0], 2.0);
    root_clip(&mut fx, 15, [0.5, 0.25, 0.0]);
    let mut clip = fx.program.animations.clip("test-side-15").unwrap().clone();
    clip.channels.insert(
        0,
        Channel {
            bone: "PELVIS".into(),
            translation: None,
            rotation: None,
        },
    );
    fx.program.animations.insert("walk.far", clip);
    let fault = tick(&mut fx).unwrap_err().to_string();
    assert!(fault.contains("0x596b60"), "{fault}");
}

#[test]
fn a_destination_at_least_the_short_distance_away_is_turned_to_with_a_clip() {
    // 4 away facing the way it should: the turn of none, clip 10, which
    // carries the person there as it plays (state 4).
    let mut fx = walking_to(0, [4.0, 0.0], 2.0);
    tick(&mut fx).unwrap();
    let a = &fx.world.animators[&SIM];
    assert_eq!((a.state, a.selection), (4, 10));
    assert_eq!(a.motion.turn_wanted.to_bits(), 0);
    assert_eq!(a.motion.turn_clip.to_bits(), 0);
    assert_eq!(bits(a.motion.turn_destination), floats([104.0, 200.0, 2.0]));
    tick(&mut fx).unwrap();
    tick(&mut fx).unwrap();
    let x = fx.world.animators[&SIM].position[0].to_f32();
    assert!(x > 100.0 && x < 104.0, "{x}");
    tick_while(&mut fx, 4, 10);
    let a = &fx.world.animators[&SIM];
    assert_eq!(a.state, 3);
    assert_eq!(bits(a.position), floats([104.0, 200.0, 2.0]));
    // A turn of a step or more with a destination elsewhere turns with
    // the turn's clip: four eighths back, clip 5.
    let mut fx = walking_to(0, [1.0, 0.0], 6.0);
    tick(&mut fx).unwrap();
    let a = &fx.world.animators[&SIM];
    assert_eq!((a.state, a.selection), (4, 5));
    assert_eq!(a.motion.turn_wanted.to_bits(), (-4f32).to_bits());
}

#[test]
fn a_pets_bounds_are_scaled() {
    // A pet (kind above 1) is put at its destination only within 0.5
    // times 0.2 (0x66d72c), and steps sideways only within 3.1 times 1/12
    // (0x6693ec): 0.18 away it steps sideways (sector 1, clip 13), where
    // an adult is put there at once.
    for kind in [2, 3] {
        let mut fx = walking_to(kind, [0.15, 0.1], 2.0);
        tick(&mut fx).unwrap();
        let a = &fx.world.animators[&SIM];
        assert_eq!((a.state, a.selection), (5, 13), "{kind}");
    }
    for kind in [0, 1] {
        let mut fx = walking_to(kind, [0.15, 0.1], 2.0);
        tick(&mut fx).unwrap();
        let a = &fx.world.animators[&SIM];
        assert_eq!((a.state, a.selection), (3, -1), "{kind}");
        assert_eq!(a.position, a.destination);
    }
    // 1 away a pet turns with clip 10, where an adult steps sideways.
    let mut fx = walking_to(2, [1.0, 0.0], 2.0);
    tick(&mut fx).unwrap();
    assert_eq!(fx.world.animators[&SIM].state, 4);
    let mut fx = walking_to(0, [1.0, 0.0], 2.0);
    tick(&mut fx).unwrap();
    assert_eq!(fx.world.animators[&SIM].state, 5);
    // 0.05 away a pet is put at its destination.
    let mut fx = walking_to(2, [0.05, 0.0], 2.0);
    tick(&mut fx).unwrap();
    let a = &fx.world.animators[&SIM];
    assert_eq!((a.state, a.selection), (3, -1));
    assert_eq!(a.position, a.destination);
}

#[test]
fn state_8_only_sets_the_walk_flag() {
    let mut fx = walking_to(0, [1.0, 0.0], 2.0);
    let before = Animator {
        state: 8,
        ..fx.world.animators[&SIM].clone()
    };
    fx.world.animators.insert(SIM, before.clone());
    tick(&mut fx).unwrap();
    let a = &fx.world.animators[&SIM];
    assert_eq!((a.state, a.walking), (8, true));
    assert_eq!(
        (a.position, a.destination),
        (before.position, before.destination)
    );
}

#[test]
fn state_12_waits_for_a_clip_that_may_not_be_interrupted() {
    let held = |interruptible: Option<i32>| {
        let mut fx = walking_to(0, [0.0, 0.0], 2.0);
        let a = fx.world.animators.get_mut(&SIM).unwrap();
        a.state = 12;
        a.walking = true;
        a.foreground = interruptible.map(|i| {
            let mut layer = Layer::start(
                "test-turn-10",
                F32::from_bits(HUNDRED),
                4,
                true,
                0,
                false,
                500,
            );
            layer.interruptible = i;
            layer.mode = 1;
            layer
        });
        tick(&mut fx).unwrap();
        fx.world.animators[&SIM].clone()
    };
    // Not to be interrupted: the clip takes the animator's speed.
    let a = held(Some(0));
    assert_eq!((a.state, a.walking), (12, false));
    assert_eq!(a.foreground.as_ref().unwrap().speed, a.rate);
    // Otherwise, or with no clip, the animator stands (state 3).
    for interruptible in [Some(-1), Some(1), None] {
        let a = held(interruptible);
        assert_eq!((a.state, a.walking), (3, false), "{interruptible:?}");
    }
}

#[test]
fn a_state_beyond_the_table_goes_to_the_tail() {
    for state in [13, -1, 100] {
        let mut fx = walking_to(0, [1.0, 0.0], 2.0);
        let a = fx.world.animators.get_mut(&SIM).unwrap();
        a.state = state;
        let before = a.clone();
        tick(&mut fx).unwrap();
        let a = &fx.world.animators[&SIM];
        assert_eq!(a.state, state);
        assert_eq!(
            (a.position, a.destination),
            (before.position, before.destination)
        );
    }
}

/// The animator in state 7, playing the stop clip backward from its
/// start, from (100, 200) facing 2 to (110, 200) facing 6, stopped for
/// `reason` (+0x288), its route's end `arrival`.
fn stopping(reason: i32, arrival: i32) -> Fixture {
    let mut fx = walking_to(0, [10.0, 0.0], 6.0);
    name_clip(&mut fx, 0x13, "test-stop", 0);
    let a = fx.world.animators.get_mut(&SIM).unwrap();
    a.state = 7;
    a.arrival = arrival;
    a.motion.near_end = reason;
    a.motion.turn_from = a.position;
    a.motion.turn_destination = a.destination;
    a.selection = 0x13;
    a.foreground = Some(Layer::start(
        "test-stop",
        F32::from_bits(HUNDRED),
        4,
        true,
        0,
        true,
        500,
    ));
    fx
}

#[test]
fn state_7_eases_as_the_stop_plays_backward_then_stands() {
    let mut fx = stopping(1, 0);
    // The clip at phase 1 backward: P is 1 less the phase, 0.
    tick(&mut fx).unwrap();
    let a = &fx.world.animators[&SIM];
    assert_eq!(a.state, 7);
    assert_eq!(bits(a.position), floats([100.0, 200.0, 2.0]));
    assert!(!a.walking);
    // The clip's speed is left as it is (-1).
    tick(&mut fx).unwrap();
    let layer = fx.world.animators[&SIM].foreground.as_ref().unwrap();
    assert_eq!(layer.speed.to_bits(), (-1f32).to_bits());
    // Then the turn (4 eighths, put into -4 to 4: -4) and the way there
    // follow P.
    tick(&mut fx).unwrap();
    let a = &fx.world.animators[&SIM];
    assert_eq!(a.state, 7);
    let x = a.position[0].to_f32();
    assert!(x > 100.0 && x < 110.0, "{x}");
    let direction = a.position[2].to_f32();
    assert!(direction < 2.0 && direction > -2.0, "{direction}");
    // At the clip's end the stop ends where it was to (+0x1dc to +0x1e4),
    // the next clip's priority raised again; the route's end -1 (why: 1)
    // is noted as 2 by state 3.
    tick_while(&mut fx, 7, 10);
    let a = &fx.world.animators[&SIM];
    assert_eq!((a.state, a.arrival, a.selection), (3, 2, -1));
    assert!(a.foreground.is_none());
    assert!(a.promote);
    assert_eq!(bits(a.position), floats([110.0, 200.0, 6.0]));
    assert_eq!(fx.sim().data[1], 6);
}

#[test]
fn state_7_notes_why_it_stopped() {
    // A clip that does not move (speed within the epsilon) ends the stop
    // at once. The route's end follows why it stopped: 1, 2 and 3 give
    // -1, 2 and 3, which state 3 then notes as 2; any other leaves it.
    for (reason, arrival, noted) in [(1, 0, 2), (2, 0, 2), (3, 0, 2), (0, 0, 0), (4, 0, 0)] {
        let mut fx = stopping(reason, arrival);
        let a = fx.world.animators.get_mut(&SIM).unwrap();
        let layer = a.foreground.as_mut().unwrap();
        layer.speed = F32::ZERO;
        layer.phase = F32::from_f32(0.5);
        tick(&mut fx).unwrap();
        let a = &fx.world.animators[&SIM];
        assert_eq!((a.state, a.arrival), (3, noted), "{reason}");
        assert_eq!(bits(a.position), floats([110.0, 200.0, 6.0]));
    }
    // A clip that has ended counts as done.
    let mut fx = stopping(1, 0);
    let a = fx.world.animators.get_mut(&SIM).unwrap();
    a.foreground.as_mut().unwrap().ended = true;
    tick(&mut fx).unwrap();
    assert_eq!(fx.world.animators[&SIM].state, 3);
}

/// The routed Sim at the end of a short path, near it already (+0x288),
/// with person word 64 set and its walk flag `walking`.
#[test]
fn a_share_just_below_a_whole_turn_steps_with_clip_minus_1() {
    // From (32, 32) to (32.73491, 31.69559): the angle's share of a turn
    // plus 1/16 is just below 0, so its part above the whole number below
    // rounds to 1, and times 8 is sector 8 (0x58f363): clip -1. It has no
    // name (string 0, 0x4704fa), so no displacement, and starts nothing
    // (0x592733): the foreground playing goes on, with the stop's end
    // mode, the animator's speed and the weight (the distance, 0.797, over
    // a length of 0).
    let at = |fx: &mut Fixture| {
        let a = fx.world.animators.get_mut(&SIM).unwrap();
        a.position = [32.0, 32.0, 2.0].map(F32::from_f32);
        a.destination = [0x4202_f08d, 0x41fd_9091, 0x4000_0000].map(F32::from_bits);
    };
    let mut fx = walking_to(0, [0.0, 0.0], 2.0);
    at(&mut fx);
    let playing = Layer::start(
        "test-turn-10",
        F32::from_bits(HUNDRED),
        4,
        true,
        0,
        true,
        500,
    );
    // It repeats (end mode 1) until the step makes it stop at its end.
    let mut playing = playing;
    playing.mode = 1;
    let a = fx.world.animators.get_mut(&SIM).unwrap();
    a.foreground = Some(playing.clone());
    a.selection = 10;
    let destination = a.destination;
    tick(&mut fx).unwrap();
    let a = &fx.world.animators[&SIM];
    assert_eq!((a.state, a.selection), (5, -1));
    assert_eq!(a.destination, destination, "no displacement");
    assert_eq!(a.motion.turn_destination, destination);
    let layer = a.foreground.as_ref().unwrap();
    assert_eq!(layer.clip, "test-turn-10");
    assert_eq!((layer.mode, layer.speed), (0, a.rate));
    assert_eq!(layer.weight.to_bits(), f32::INFINITY.to_bits());
    // A foreground already noted as clip -1 is left as it is by 0x5926d7
    // (0x5926ef): its end mode stays, though the speed and weight are set.
    let mut fx = walking_to(0, [0.0, 0.0], 2.0);
    at(&mut fx);
    let a = fx.world.animators.get_mut(&SIM).unwrap();
    a.foreground = Some(playing);
    a.selection = -1;
    tick(&mut fx).unwrap();
    let a = &fx.world.animators[&SIM];
    assert_eq!((a.state, a.selection), (5, -1));
    let layer = a.foreground.as_ref().unwrap();
    assert_eq!((layer.mode, layer.speed), (1, a.rate));
    assert_eq!(layer.weight.to_bits(), f32::INFINITY.to_bits());
    // With no foreground the original reads through a null pointer.
    let mut fx = walking_to(0, [0.0, 0.0], 2.0);
    at(&mut fx);
    let fault = tick(&mut fx).unwrap_err().to_string();
    assert!(fault.contains("0x58f4d3"), "{fault}");
}

#[test]
fn a_position_that_is_not_a_number_is_left_as_it_is() {
    // 0x58dd8f compares with -999 by `sahf; je`: unordered sets ZF, so a
    // coordinate that is not a number counts as unset and state 3 places
    // the animator (0x592132) and is done. The placing compares within
    // epsilon by `jb` (0x592160), which unordered takes, so it places
    // nothing: the coordinate stays, and no turn or step starts.
    let mut fx = walking_to(0, [1.0, 0.0], 2.0);
    fx.world.animators.get_mut(&SIM).unwrap().position[1] = F32::from_f32(f32::NAN);
    tick(&mut fx).unwrap();
    let a = &fx.world.animators[&SIM];
    assert_eq!((a.state, a.selection), (3, -1));
    assert!(a.position[1].to_f32().is_nan());
    assert_eq!(a.position[0].to_f32(), 100.0);
    assert!(a.foreground.is_none());
}

fn special_at_end(walking: bool) -> Fixture {
    let mut fx = routed();
    fx.world.routing.routes.get_mut(&SIM).unwrap().path.pop();
    fx.sim_mut().data[9] = 1;
    fx.sim_mut().person.as_mut().unwrap().data[64] = 1;
    let a = fx.world.animators.get_mut(&SIM).unwrap();
    a.motion.near_end = 1;
    a.walking = walking;
    fx
}

#[test]
fn a_person_whose_word_64_is_set_stops_at_once_when_not_walking() {
    // 0x58e83b, then 0x58ef8e with +0x290 clear: the route's end -1, the
    // destination the position, and state 3 notes the arrival.
    let mut fx = special_at_end(false);
    tick(&mut fx).unwrap();
    let a = &fx.world.animators[&SIM];
    assert_eq!((a.state, a.arrival, a.selection), (3, 2, -1));
    assert!(a.foreground.is_none());
    assert_eq!(bits(a.position), floats([100.0, 200.0, 6.0]));
    assert_eq!(a.destination, a.position);
    assert_eq!(a.motion.turn_destination, a.position);
}

#[test]
fn a_walking_person_whose_word_64_is_set_plays_the_stop_backward() {
    // With +0x290 set the stop clip (0x13) starts backward and state 7
    // carries the person to the end point, (300, 200), facing the path's
    // last point (direction 2).
    let mut fx = special_at_end(true);
    tick(&mut fx).unwrap();
    let a = &fx.world.animators[&SIM];
    assert_eq!((a.state, a.selection, a.walking), (7, 0x13, false));
    assert_eq!(a.motion.progress.to_bits(), 1f32.to_bits());
    assert_eq!(bits(a.destination), floats([300.0, 200.0, 2.0]));
    assert_eq!(bits(a.motion.turn_destination), floats([300.0, 200.0, 2.0]));
    assert_eq!(bits(a.motion.turn_from), floats([100.0, 200.0, 6.0]));
    let layer = a.foreground.as_ref().unwrap();
    assert_eq!(layer.clip, "test-stop");
    assert_eq!(layer.speed.to_bits(), (-1f32).to_bits());
    assert_eq!(layer.mode, 0);
    tick_while(&mut fx, 7, 10);
    let a = &fx.world.animators[&SIM];
    assert_eq!((a.state, a.arrival), (3, 2));
    assert_eq!(bits(a.position), floats([300.0, 200.0, 2.0]));
    assert_eq!(fx.sim().data[1], 2);
}

#[test]
fn a_stop_asked_under_way_for_a_person_whose_word_64_is_set_plays_the_stop_backward() {
    // 0x58ef2f: the step is taken, then why it stops becomes 3 and the
    // stop starts where the step ended.
    let mut walked = facing_along();
    tick(&mut walked).unwrap();
    let stepped = walked.world.animators[&SIM].position;
    let mut fx = facing_along();
    fx.sim_mut().person.as_mut().unwrap().data[64] = 1;
    fx.world.animators.get_mut(&SIM).unwrap().walking = true;
    fx.world.stops_asked.insert(SIM);
    tick(&mut fx).unwrap();
    let a = &fx.world.animators[&SIM];
    assert_eq!((a.state, a.selection, a.motion.near_end), (7, 0x13, 3));
    assert_eq!(a.arrival, 1);
    assert_eq!(a.position, stepped);
    assert_eq!(a.destination, stepped);
    assert_eq!(a.motion.turn_from, stepped);
    assert_eq!(fx.sim().position, walked.sim().position);
    let layer = a.foreground.as_ref().unwrap();
    assert_eq!(layer.clip, "test-stop");
    assert_eq!(layer.speed.to_bits(), (-1f32).to_bits());
    // The stop ends where it started: the route's end 3, noted as 2.
    tick_while(&mut fx, 7, 10);
    let a = &fx.world.animators[&SIM];
    assert_eq!((a.state, a.arrival), (3, 2));
    assert_eq!(a.position, stepped);
}

#[test]
fn a_stop_clip_no_file_defines_leaves_the_foreground_playing() {
    // 0x58f036: the lookup of a name no animation file defines gives a
    // null clip, so nothing starts and state 7 eases on with what plays.
    let mut fx = special_at_end(true);
    fx.program
        .libraries
        .get_mut(&GLOBAL)
        .unwrap()
        .strings
        .get_mut(&150)
        .unwrap()[0x13] = "test-defined-nowhere".into();
    fx.world.animators.get_mut(&SIM).unwrap().foreground = Some(Layer::start(
        "test-walk",
        F32::from_bits(HUNDRED),
        4,
        true,
        0,
        true,
        500,
    ));
    tick(&mut fx).unwrap();
    let a = &fx.world.animators[&SIM];
    assert_eq!((a.state, a.selection), (7, 0x13));
    assert_eq!(a.foreground.as_ref().unwrap().clip, "test-walk");
}
