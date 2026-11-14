//! Motive decay, mood and motive changes. Synthetic; no game data.

use sims_core::F32;
use sims_formats::saved_objects::{Interaction, MotiveDelta, Person};
use sims_sim::motives::{Curve, MOOD, ROOM, Rates, Tuning, apply_changes};

fn interaction() -> Interaction {
    Interaction {
        uid: 0,
        caller: 0,
        target: 0,
        icon: 0,
        table_index: 0,
        args: [0; 4],
        priority: 0,
        action: 0,
        attenuation: 0.0,
        flags: 0,
    }
}

fn person(motives: [f32; 16]) -> Person {
    Person {
        animation_events: 0,
        engaged: 0,
        appearance: Default::default(),
        accessories: Vec::new(),
        animations: Default::default(),
        routing_state: 0,
        floats: [0.0; 9],
        previous_motives: motives,
        motives,
        data: vec![0; 256],
        routing_frames: 0,
        active: interaction(),
        last: interaction(),
        queue: Vec::new(),
        object_uses: Vec::new(),
        motive_deltas: Vec::new(),
    }
}

fn tuning() -> Tuning {
    let rates = || Rates::from_lookup(0, |_| None);
    Tuning {
        rates: [rates(), rates(), rates()],
        adult_mood: vec![Curve::parse(7, "(-100;3)(0;1)(100;1)", 8)],
        child_mood: vec![Curve::parse(7, "(-100;1)(100;1)", 8)],
    }
}

#[test]
fn curves_sort_their_points_and_drop_repeats_and_extras() {
    let curve = Curve::parse(7, "(10;1) (0;2) (10;5) junk (20;3)", 3);
    let points: Vec<[f32; 2]> = curve
        .points
        .iter()
        .map(|p| [p[0].to_f32(), p[1].to_f32()])
        .collect();
    assert_eq!(points, vec![[0.0, 2.0], [10.0, 1.0], [20.0, 3.0]]);
    assert_eq!(curve.inverses.len(), 2);
    assert_eq!(curve.inverses[0].to_f32(), 0.1);
    let capped = Curve::parse(7, "(0;0)(1;1)(2;2)", 2);
    assert_eq!(capped.points.len(), 2);
}

#[test]
fn a_curve_is_flat_outside_its_points_and_straight_between() {
    let curve = Curve::parse(7, "(-100;3)(0;1)(100;1)", 8);
    let at = |x: f32| curve.weight(F32::from_f32(x)).to_f32().to_f32();
    assert_eq!(at(-200.0), 3.0);
    assert_eq!(at(-100.0), 3.0);
    assert_eq!(at(-50.0), 2.0);
    assert_eq!(at(50.0), 1.0);
    assert_eq!(at(150.0), 1.0);
    assert_eq!(
        Curve::parse(7, "", 8).weight(F32::ZERO).to_f32().to_f32(),
        0.0
    );
}

#[test]
fn the_default_rates_lose_the_energy_span_over_the_waking_hours() {
    let rates = Rates::from_lookup(0, |_| None);
    assert_eq!(rates.energy_awake.to_f32(), -0.375);
    assert_eq!(rates.energy_asleep.to_f32(), 0.75);
    assert_eq!((rates.wake_hour, rates.sleep_hour), (7, 23));
    let other = Rates::from_lookup(1, |name| (name == "energy span").then_some(60.0));
    assert_eq!(other.energy_awake.to_f32(), -2.0);
}

#[test]
fn an_awake_decay_lowers_the_motives_and_works_out_the_mood() {
    let mut motives = [50.0; 16];
    motives[11] = 10.0;
    motives[7] = 20.0;
    let mut p = person(motives);
    p.previous_motives[7] = 10.0;
    tuning().decay(&mut p, 12);
    let m = p.motives;
    assert_eq!(m[5], 49.625, "energy loses the awake rate");
    // Hunger rose by 10 since the last decay: bladder loses a quarter of
    // that, then its awake rate.
    assert_eq!(m[9], 50.0 - 2.5 - 0.3);
    assert_eq!(m[15], 49.75, "fun loses its awake rate");
    assert!(m[14] < 50.0, "social decays while awake");
    // The previous values are the new motives, with the mood from before.
    assert_eq!(p.previous_motives[MOOD], 50.0);
    assert_eq!(p.previous_motives[9], m[9]);
    // One curve, so the mood is hunger itself.
    assert_eq!(m[MOOD], m[7]);
    assert_eq!(m[ROOM], 50.0);
}

#[test]
fn asleep_social_stays_fun_is_multiplied_and_energy_gains() {
    let mut motives = [50.0; 16];
    motives[11] = -1.0;
    let mut p = person(motives);
    tuning().decay(&mut p, 3);
    assert_eq!(p.motives[14], 50.0);
    assert_eq!(p.motives[15], 25.0);
    assert_eq!(p.motives[5], 50.75);
}

#[test]
fn a_decay_keeps_every_motive_within_100() {
    let mut motives = [150.0; 16];
    motives[7] = -150.0;
    let mut p = person(motives);
    tuning().decay(&mut p, 12);
    assert_eq!(p.previous_motives[0], 100.0);
    assert_eq!(p.previous_motives[7], -100.0);
}

#[test]
fn motive_changes_stop_at_their_value() {
    let mut p = person([0.0; 16]);
    p.motive_deltas = vec![
        MotiveDelta {
            motive: 7,
            delta: 3.0,
            stop_at: 2.0,
        },
        MotiveDelta {
            motive: 8,
            delta: -1.0,
            stop_at: -5.0,
        },
        MotiveDelta {
            motive: 9,
            delta: 1.0,
            stop_at: -1.0,
        },
    ];
    apply_changes(&mut p);
    assert_eq!((p.motives[7], p.motives[8], p.motives[9]), (2.0, -1.0, 0.0));
    assert_eq!(p.motive_deltas.len(), 3, "the changes stay set");
}
