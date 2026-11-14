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
    let mut layer = Layer::start("c", THOUSAND, 30, true, false, 1000);
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
    let mut layer = Layer::start("c", F32::from_bits(0x42c8_0000), 4, true, true, 0);
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
    let mut layer = Layer::start("c", THOUSAND, 30, false, false, 0);
    assert_eq!(layer.update(50).unwrap(), Advance::Unbound);
    assert_eq!((layer.current, layer.previous), (50, 0));
    assert_eq!(layer.phase, F32::ZERO);
    // So does one whose first motion has no frames.
    let mut layer = Layer::start("c", THOUSAND, 0, true, false, 0);
    assert_eq!(layer.update(50).unwrap(), Advance::Unbound);
}

#[test]
fn a_stopped_layer_has_no_duration_and_is_finished() {
    let mut layer = Layer::start("c", THOUSAND, 30, true, false, 0);
    layer.set_speed(F32::ZERO);
    assert_eq!(layer.duration, F32::ZERO);
    assert!(layer.finished());
    layer.update(50).unwrap();
    assert_eq!(layer.phase, F32::ZERO);
}

#[test]
fn a_weightless_layer_sends_no_events() {
    let mut layer = Layer::start("c", THOUSAND, 30, true, false, 0);
    layer.weight = F32::ZERO;
    assert_eq!(layer.update(50).unwrap(), Advance::Silent);
}

#[test]
fn what_the_layer_update_does_not_port_fails() {
    let mut layer = Layer::start("c", F32::from_bits(0x42c8_0000), 4, true, false, 0);
    layer.mode = 1;
    assert!(layer.update(200).unwrap_err().contains("end mode 1"));
    let mut layer = Layer::start("c", THOUSAND, 4, true, false, 0);
    layer.fading = true;
    assert!(layer.update(50).unwrap_err().contains("fading"));
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
        foreground: Some(Layer::start("c", THOUSAND, 30, true, false, 0)),
        ..LOADED
    };
    assert!(a.reset().unwrap_err().contains("0x592878"));
    let mut a = Animator {
        arrival: 2,
        ..LOADED
    };
    assert!(a.reset().unwrap_err().contains("0x589e2d"));
}
