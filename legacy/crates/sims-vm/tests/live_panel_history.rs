//! Source-derived arithmetic tests; original code is not executed here.
use sims_vm::interface::live_panel::history::{AverageHistory, Elapsed};

fn restore<T: serde::Serialize + for<'a> serde::Deserialize<'a>>(v: &T) -> T {
    serde_json::from_slice(&serde_json::to_vec(v).unwrap()).unwrap()
}

#[test]
fn history_retains_sample_time_averages_and_rounds_ties_away_from_zero() {
    let mut h = AverageHistory::new(3, 5).unwrap();
    assert_eq!(h.average(), 0);
    for v in [1, 2, 8, -10, -11] {
        h.push(v).unwrap();
        h = restore(&h);
    }
    assert_eq!(h.samples(), [8, -10, -11]);
    // At insertion:11/3 ->4, 0/3 ->0, -13/3 ->-4.
    assert_eq!(h.averages(), [4, 0, -4]);
    assert_eq!(h.trend_limit(), 5);
    for (values, expected) in [
        ([0, 1], 1),
        ([0, -1], -1),
        ([0, 3], 2),
        ([0, -3], -2),
        ([1, -1], 0),
    ] {
        let mut h = AverageHistory::new(10, 5).unwrap();
        for v in values {
            h.push(v).unwrap();
        }
        assert_eq!(h.average(), expected);
    }
}

#[test]
fn history_preserves_wrapping_native_sum_and_rejects_corrupt_restore_before_mutation() {
    let mut h = AverageHistory::new(2, 5).unwrap();
    h.push(i32::MAX).unwrap();
    h.push(i32::MAX).unwrap();
    assert_eq!(h.average(), -1); // native32-bit sum wraps to-2
    h.push(i32::MIN).unwrap();
    assert_eq!(h.average(), -1); // sum=-1, rounded half away from zero
    h.push(0).unwrap();
    assert_eq!(h.average(), i32::MIN / 2);
    let saved = serde_json::to_value(&h).unwrap();
    for (key, value) in [
        ("sum", serde_json::json!(9)),
        ("capacity", serde_json::json!(0)),
        ("averages", serde_json::json!([])),
    ] {
        let mut corrupted = saved.clone();
        corrupted[key] = value;
        let mut h: AverageHistory = serde_json::from_value(corrupted).unwrap();
        let before = serde_json::to_vec(&h).unwrap();
        assert!(h.validate().is_err());
        assert!(h.push(123).is_err());
        assert_eq!(serde_json::to_vec(&h).unwrap(), before);
    }
}

#[test]
fn elapsed_poll_uses_unsigned_compare_and_keeps_zero_interval_semantics() {
    for accumulated in [0, 1, 4, 5, 6, 10, 11, 0x80000000, u32::MAX] {
        for interval in [0, 1, 5, 0x80000000, u32::MAX] {
            let mut timer = Elapsed {
                accumulated,
                interval,
            };
            let expected = interval <= accumulated;
            assert_eq!(timer.consume(), expected);
            assert_eq!(
                timer.accumulated,
                if expected {
                    accumulated - interval
                } else {
                    accumulated
                }
            );
            assert_eq!(timer, restore(&timer));
        }
    }
    let mut timer = Elapsed {
        accumulated: 11,
        interval: 5,
    };
    assert!(timer.consume());
    assert!(timer.consume());
    assert!(!timer.consume());
    assert_eq!(timer.accumulated, 1);
}

#[test]
fn trend_uses_stored_means_and_preserves_signed_clamp_order() {
    let mut h = AverageHistory::new(3, 5).unwrap();
    assert!(h.trend().is_err());
    for v in [0, 100, 100] {
        h.push(v).unwrap();
    }
    assert_eq!(h.trend().unwrap(), 5);
    for v in [-100, -100, -100] {
        h.push(v).unwrap();
    }
    assert_eq!(h.averages(), [33, -33, -100]);
    assert_eq!(h.trend().unwrap(), -5);
    h.push(-100).unwrap();
    h.push(-100).unwrap();
    assert_eq!(h.trend().unwrap(), 0);
    // The native constructor accepts this value. Rust clamp(-limit,limit)
    // would panic and would not reproduce4cb635's ordered comparisons.
    let mut negative = AverageHistory::new(2, -5).unwrap();
    negative.push(0).unwrap();
    assert_eq!(negative.trend().unwrap(), 5);
    negative.push(100).unwrap();
    assert_eq!(negative.trend().unwrap(), -5);
}
