//! A person's animator clock and a layer's clock (0x585483, 0x597a88,
//! 0x598636, 0x597c4c). Synthetic; no game data.

use sims_core::{Ext, F32};
use sims_sim::animator::{Advance, Animator, LOADED, Layer, UNSET};

const THOUSAND: F32 = F32::from_bits(0x447a_0000);

#[test]
fn the_clock_runs_fifty_per_tick_or_forty_two_when_slow() {
    assert_eq!(LOADED.per_tick(), 50);
    assert_eq!(LOADED.clock_at(88_445), 4_422_250);
    let slow = sims_sim::animator::Animator {
        slow: true,
        ..LOADED
    };
    assert_eq!(slow.clock_at(10), 420);
}

#[test]
fn a_layer_advances_by_the_elapsed_share_of_its_duration() {
    let mut layer = Layer::start("c", THOUSAND, 30, true, 0, false, 1000);
    assert_eq!(layer.duration, THOUSAND);
    assert_eq!(layer.update(1000).unwrap(), Advance::Events);
    assert_eq!(layer.phase, F32::ZERO);
    layer.commit();
    // 50 of 1000, in single precision: 0.05.
    assert_eq!(layer.update(1050).unwrap(), Advance::Events);
    assert_eq!(layer.phase.to_bits(), 0x3d4c_cccd);
    assert_eq!((layer.event_time, layer.previous_event_time), (50, 0));
    layer.commit();
    assert_eq!(layer.update(1100).unwrap(), Advance::Events);
    assert_eq!(layer.phase.to_bits(), 0x3dcc_cccd);
    assert_eq!(
        (layer.current, layer.previous, layer.event_time),
        (1100, 1050, 100)
    );
    assert!(!layer.ended);
}

#[test]
fn a_clip_played_backward_ends_at_its_start() {
    let mut layer = Layer::start("c", F32::from_bits(0x42c8_0000), 4, true, 0, true, 0);
    assert_eq!(layer.phase.to_bits(), 0x3f80_0000);
    assert_eq!(layer.speed.to_bits(), 0xbf80_0000);
    // The duration is made positive.
    assert_eq!(layer.duration.to_bits(), 0x42c8_0000);
    layer.update(50).unwrap();
    assert_eq!(layer.phase.to_bits(), 0x3f00_0000);
    layer.update(100).unwrap();
    assert!(layer.ended && layer.finished());
    assert_eq!((layer.phase, layer.speed), (F32::ZERO, F32::ZERO));
    assert_eq!(layer.duration.to_bits(), 0x4cbe_bc20);
    // Past the end the phase is held at 0.
    layer.update(150).unwrap();
    assert_eq!(layer.phase, F32::ZERO);
}

#[test]
fn a_layer_with_nothing_bound_keeps_only_its_clock() {
    let mut layer = Layer::start("c", THOUSAND, 30, false, 0, false, 0);
    assert_eq!(layer.update(50).unwrap(), Advance::Unbound);
    assert_eq!((layer.current, layer.previous), (50, 0));
    assert_eq!(layer.phase, F32::ZERO);
    // So does one whose first motion has no frames.
    let mut layer = Layer::start("c", THOUSAND, 0, true, 0, false, 0);
    assert_eq!(layer.update(50).unwrap(), Advance::Unbound);
}

#[test]
fn a_stopped_layer_has_no_duration_and_is_finished() {
    let mut layer = Layer::start("c", THOUSAND, 30, true, 0, false, 0);
    layer.set_speed(F32::ZERO);
    assert_eq!(layer.duration, F32::ZERO);
    assert!(layer.finished());
    layer.update(50).unwrap();
    assert_eq!(layer.phase, F32::ZERO);
}

#[test]
fn a_weightless_layer_sends_no_events() {
    let mut layer = Layer::start("c", THOUSAND, 30, true, 0, false, 0);
    layer.weight = F32::ZERO;
    assert_eq!(layer.update(50).unwrap(), Advance::Silent);
}

#[test]
fn a_repeating_clip_starts_over_past_its_end() {
    // 0x597dca: end mode 1 keeps the speed and duration and takes the
    // part of the phase past a whole number.
    let hundred = F32::from_bits(0x42c8_0000);
    let mut layer = Layer::start("c", hundred, 4, true, 0, false, 0);
    layer.mode = 1;
    layer.update(80).unwrap();
    assert_eq!(layer.phase.to_bits(), 0x3f4c_cccd);
    assert!(!layer.ended);
    // 0.8 + 0.5 lies halfway between two singles and rounds to the even
    // one, 1.29999995; less its floor, 0.29999995, so the event time
    // truncates to 29.
    layer.update(130).unwrap();
    assert!(layer.ended);
    assert_eq!(layer.phase.to_bits(), 0x3e99_9998);
    assert_eq!(
        (layer.speed, layer.duration),
        (F32::from_bits(0x3f80_0000), hundred)
    );
    assert_eq!(layer.event_time, 29);
    // A phase of exactly 1 is kept, and ends the clip too.
    let mut layer = Layer::start("c", hundred, 4, true, 0, false, 0);
    layer.mode = 1;
    layer.update(100).unwrap();
    assert!(layer.ended);
    assert_eq!(layer.phase, F32::from_bits(0x3f80_0000));
    // The next update goes on from there, past 1 again.
    layer.update(150).unwrap();
    assert!(layer.ended);
    assert_eq!(layer.phase.to_bits(), 0x3f00_0000);
    // Backward, a phase below 0 starts over from the top.
    let mut layer = Layer::start("c", hundred, 4, true, 0, true, 0);
    layer.mode = 1;
    layer.update(125).unwrap();
    assert!(layer.ended);
    assert_eq!(layer.phase.to_bits(), 0x3f40_0000);
}

#[test]
fn what_the_layer_update_does_not_port_fails() {
    let mut layer = Layer::start("c", F32::from_bits(0x42c8_0000), 4, true, 0, false, 0);
    layer.mode = 2;
    assert!(layer.update(200).unwrap_err().contains("end mode 2"));
}

#[test]
fn a_fading_layer_moves_its_weight_towards_the_end_then_stops() {
    // 0x597a3e starts the fade; 0x597e3c steps it at the layer's clock.
    let mut layer = Layer::start("c", THOUSAND, 4, true, 0, false, 0);
    layer.start_fade(0, 200, F32::from_bits(0x3f80_0000), F32::ZERO);
    assert!(layer.fading);
    // A quarter of the way: 1 + (0 - 1) * 0.25.
    layer.update(50).unwrap();
    assert!(layer.fading);
    assert_eq!(layer.weight.to_bits(), 0x3f40_0000);
    // At the length itself the fade is not yet over.
    layer.update(200).unwrap();
    assert!(layer.fading);
    assert_eq!(layer.weight, F32::ZERO);
    // Past it the weight is the end and the fade is over.
    let mut layer = Layer::start("c", THOUSAND, 4, true, 0, false, 0);
    layer.start_fade(
        0,
        200,
        F32::from_bits(0x3f80_0000),
        F32::from_bits(0x3e80_0000),
    );
    layer.update(250).unwrap();
    assert!(!layer.fading);
    assert_eq!(layer.weight.to_bits(), 0x3e80_0000);
    // A fade of no length has no share done (0 over 0 is not a number):
    // the weight stays at the start until the clock passes it.
    let mut layer = Layer::start("c", THOUSAND, 4, true, 0, false, 0);
    layer.start_fade(0, 0, F32::from_bits(0x3f00_0000), F32::ZERO);
    layer.update(0).unwrap();
    assert!(layer.fading);
    assert_eq!(layer.weight.to_bits(), 0x3f00_0000);
}

#[test]
fn a_new_animator_stands_unplaced_with_no_posture() {
    // 0x587d2c and 0x58464f.
    let a = Animator::new(4_422_450);
    assert_eq!(a.clock, 4_422_450);
    assert_eq!(
        (a.rate.to_bits(), a.target.to_bits()),
        (0x3f80_0000, 0x3f80_0000)
    );
    assert_eq!((a.state, a.arrival, a.posture), (3, 0, -1));
    assert_eq!(a.position, [UNSET; 3]);
    assert_eq!(a.destination, [F32::ZERO; 3]);
    assert!(!a.base && a.foreground.is_none() && !a.playing);
    assert_eq!((a.selection, a.carry_selection, a.carry_mode), (-1, -1, 0));
    // Only a reset raises the next foreground clip's priority.
    assert!(!a.promote);
    assert!(a.profile_stale);
}

#[test]
fn a_reset_unplaces_the_animator_and_drops_its_posture() {
    // 0x589220: what the person's tick leaves after it starts the
    // posture's clip and places the animator off the lot.
    let off_lot = Ext::from_i32(-16).single().to_f32();
    let mut a = Animator {
        rate: F32::from_bits(0x3f99_999a),
        target: F32::from_bits(0x3fb3_3333),
        position: [off_lot, off_lot, F32::ZERO],
        destination: [off_lot, off_lot, F32::ZERO],
        posture: 0,
        base: true,
        carry_mode: 1,
        state: 0,
        events: vec![3],
        playing: true,
        profile_stale: false,
        ..Animator::new(4_422_450)
    };
    a.reset().unwrap();
    assert_eq!(a.position, [UNSET; 3]);
    // The destination and the speeds stay.
    assert_eq!(a.destination, [off_lot, off_lot, F32::ZERO]);
    assert_eq!(a.rate.to_bits(), 0x3f99_999a);
    assert_eq!(
        (a.posture, a.carry_mode, a.carry_selection, a.state),
        (-1, 0, -1, 3)
    );
    assert!(!a.base && a.skin_dirty);
    assert!(a.events.is_empty() && !a.playing);
    assert!(a.promote && a.profile_stale);
    // The load's state is a new animator reset.
    let mut loaded = Animator::new(0);
    loaded.reset().unwrap();
    assert_eq!(loaded, LOADED);
}

#[test]
fn a_reset_with_a_clip_playing_or_a_route_result_is_refused() {
    let mut a = Animator {
        foreground: Some(Layer::start("c", THOUSAND, 30, true, 0, false, 0)),
        ..LOADED
    };
    assert!(a.reset().unwrap_err().contains("0x592878"));
    let mut a = Animator {
        arrival: 2,
        ..LOADED
    };
    assert!(a.reset().unwrap_err().contains("0x589e2d"));
}

/// A motion's events for [`Layer::due`]: (time, has a property). Index 3
/// has no property; indices 0 and 6 lie outside 0 to 1040 (a clip of
/// 1000 plus 40, 0x66d738).
const EVENTS: [(i32, bool); 7] = [
    (-5, true),
    (0, true),
    (100, true),
    (200, false),
    (300, true),
    (1040, true),
    (1041, true),
];

/// Sets the event times as an update would leave them.
fn at(layer: &mut Layer, previous: i32, now: i32) {
    layer.previous_event_time = previous;
    layer.event_time = now;
}

#[test]
fn forward_events_are_due_up_to_the_event_time_and_wrap_round() {
    // 0x597ff9 to 0x598208, forward.
    let mut layer = Layer::start("c", THOUSAND, 30, true, 1, false, 0);
    assert_eq!(layer.cursors, [0]);
    at(&mut layer, 0, 250);
    // -5 is passed over; 200 has no property; the cursor stops at 300.
    assert_eq!(layer.due(0, &EVENTS).unwrap(), [1, 2]);
    assert_eq!(layer.cursors, [4]);
    at(&mut layer, 250, 1040);
    assert_eq!(layer.due(0, &EVENTS).unwrap(), [4, 5]);
    assert_eq!(layer.cursors, [7]);
    // Nothing more is due at the same time.
    at(&mut layer, 1040, 1040);
    assert!(layer.due(0, &EVENTS).unwrap().is_empty());
    // The event time went back: from the cursor to the end, then from the
    // first event.
    layer.cursors[0] = 4;
    at(&mut layer, 250, 10);
    assert_eq!(layer.due(0, &EVENTS).unwrap(), [4, 5, 1]);
    assert_eq!(layer.cursors, [2]);
    // A motion the layer has no cursor for is refused.
    assert!(layer.due(1, &EVENTS).unwrap_err().contains("no motion 1"));
}

#[test]
fn backward_events_are_due_down_to_the_event_time_and_wrap_round() {
    let mut layer = Layer::start("c", THOUSAND, 30, true, 1, true, 0);
    layer.cursors[0] = 6;
    at(&mut layer, 1040, 100);
    // 1041 is passed over; 100 is due; the cursor stops at 0.
    assert_eq!(layer.due(0, &EVENTS).unwrap(), [5, 4, 2]);
    assert_eq!(layer.cursors, [1]);
    // Going round backward, the cursor steps back before each event, so
    // the one it pointed at is not handled (0x5947bb); then it goes to the
    // last event (0x5947cc).
    at(&mut layer, 100, 500);
    assert_eq!(layer.due(0, &EVENTS).unwrap(), [5]);
    assert_eq!(layer.cursors, [4]);
}
