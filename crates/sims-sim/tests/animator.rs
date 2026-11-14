//! A person's animator clock and a layer's clock (0x585483, 0x597a88,
//! 0x598636, 0x597c4c). Synthetic; no game data.

use sims_core::F32;
use sims_sim::animator::{Advance, LOADED, Layer};

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
