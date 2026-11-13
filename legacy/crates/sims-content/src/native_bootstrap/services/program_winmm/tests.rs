//! Authored source controls: ALL UNRUN. Parent must first declare the module.
//! Synthetic I/O is private to cfg(test). Real-clock controls additionally need
//! --ignored and SIMS_WINMM_REAL_CLOCK_CONTROL=1; only parent may execute them.

use super::*;
use std::{collections::VecDeque, mem::size_of};

pub(super) struct ScriptedIo {
    remaining: VecDeque<(Stage, NativeCall)>,
    entered: Vec<Stage>,
}

impl ScriptedIo {
    pub(super) fn read(&mut self, stage: Stage) -> NativeCall {
        let (expected, call) = self.remaining.pop_front().expect("unexpected I/O");
        assert_eq!(stage, expected, "boot/calendar/boot order must be exact");
        self.entered.push(stage);
        call
    }
}

fn scalars(seconds: i64, microseconds: i32) -> TimevalScalars {
    TimevalScalars { seconds, microseconds }
}

fn returned(stage: Stage, output: TimevalScalars) -> NativeCall {
    let length = (stage != Stage::Calendar).then_some(size_of::<DarwinTimeval>());
    NativeCall {
        return_code: 0, errno_before: 37, errno_after: 37,
        buffer_capacity: length, returned_length: length, output,
    }
}

fn triple(boot: TimevalScalars, calendar: TimevalScalars) -> Vec<(Stage, NativeCall)> {
    [Stage::BootBefore, Stage::Calendar, Stage::BootAfter].into_iter()
        .map(|stage| (stage, returned(stage, if stage == Stage::Calendar { calendar } else { boot })))
        .collect()
}

fn elapsed(elapsed_us: u128) -> Vec<(Stage, NativeCall)> {
    let boot = scalars(10, 123);
    let calendar_us = 10_000_123 + elapsed_us;
    let calendar = scalars(
        i64::try_from(calendar_us / 1_000_000).unwrap(),
        i32::try_from(calendar_us % 1_000_000).unwrap(),
    );
    triple(boot, calendar)
}

fn clock(calls: impl IntoIterator<Item = (Stage, NativeCall)>) -> Clock {
    let mut clock = Clock::new(Policy::DarwinKernelBootElapsed);
    clock.scripted_io = Some(ScriptedIo { remaining: calls.into_iter().collect(), entered: vec![] });
    clock
}

fn run(clock: &mut Clock) -> Result<Sample, Stop> {
    // Private synthetic host support; production sample() always checks the host.
    clock.sample_on_host(true)
}

fn entered(clock: &Clock) -> &[Stage] {
    &clock.scripted_io.as_ref().unwrap().entered
}

fn accepted(clock: &Clock) -> Option<(u64, u128, Report)> {
    clock.last_sample().map(|s| (s.sequence(), s.elapsed_microseconds(), *s.report()))
}

fn assert_stopped_at(report: &Report, stage: Stage) {
    for current in [Stage::BootBefore, Stage::Calendar, Stage::BootAfter] {
        if current.index() <= stage.index() {
            assert!(matches!(report.call(current), NativeOutcome::Returned(_)));
        } else {
            assert_eq!(report.call(current), NativeOutcome::NotEntered);
        }
    }
}

#[test]
fn disabled_and_unsupported_refuse_before_io() {
    let mut disabled = Clock::new(Policy::Disabled);
    disabled.scripted_io = Some(ScriptedIo { remaining: VecDeque::new(), entered: vec![] });
    assert_eq!(disabled.sample().unwrap_err(), Stop::Disabled);
    assert!(entered(&disabled).is_empty());
    assert_eq!(disabled.latest_report().unwrap().provenance(), Provenance::NoNativeIo);
    assert_eq!(disabled.latest_report().unwrap().sequence(), None);
    assert_eq!(disabled.latest_report().unwrap().policy(), Policy::Disabled);
    assert_eq!(disabled.latest_report(), disabled.first_failure_report());
    assert!(disabled.last_sample().is_none());

    let mut unsupported = clock([]);
    assert_eq!(unsupported.sample_on_host(false).unwrap_err(), Stop::UnsupportedHost);
    assert!(entered(&unsupported).is_empty());
    let report = unsupported.latest_report().unwrap();
    assert_eq!(report.provenance(), Provenance::NoNativeIo);
    assert_eq!(report.sequence(), None);
    for stage in [Stage::BootBefore, Stage::Calendar, Stage::BootAfter] {
        assert_eq!(report.call(stage), NativeOutcome::NotEntered);
    }
    #[cfg(not(all(target_os = "macos", target_pointer_width = "64")))]
    assert_eq!(unsupported.sample().unwrap_err(), Stop::UnsupportedHost);
}

#[test]
fn every_native_failure_retains_rc_errno_length_and_declared_scalars() {
    for stage in [Stage::BootBefore, Stage::Calendar, Stage::BootAfter] {
        let mut calls = elapsed(10_000);
        let failed = &mut calls[stage.index()].1;
        failed.return_code = -1;
        failed.errno_before = 42;
        failed.errno_after = 5;
        failed.returned_length = (stage != Stage::Calendar).then_some(3);
        failed.output = scalars(i64::MIN, i32::MIN);
        let expected_receipt = *failed;
        let mut c = clock(calls);
        assert_eq!(run(&mut c).unwrap_err(), Stop::NativeCallFailed {
            stage, return_code: -1, errno: 5,
        });
        assert_eq!(entered(&c).len(), stage.index() + 1);
        assert!(c.last_sample().is_none());
        let report = c.latest_report().unwrap();
        assert_eq!(report.sequence(), Some(1));
        assert_eq!(report.provenance(), Provenance::SyntheticControl);
        assert_eq!(report.call(stage), NativeOutcome::Returned(expected_receipt));
        assert_eq!(expected_receipt.return_code(), -1);
        assert_eq!(expected_receipt.errno_before(), 42);
        assert_eq!(expected_receipt.errno_after(), 5);
        assert_eq!(expected_receipt.declared_output().seconds(), i64::MIN);
        assert_eq!(expected_receipt.declared_output().microseconds(), i32::MIN);
        assert_stopped_at(report, stage);
    }
}

#[test]
fn every_partial_boot_length_and_oversize_refuses_without_adopting_output() {
    let expected = size_of::<DarwinTimeval>();
    let lengths = (0..expected).map(Some).chain([Some(expected + 1), Some(usize::MAX), None]);
    for returned_length in lengths {
        for stage in [Stage::BootBefore, Stage::BootAfter] {
            let mut calls = elapsed(0);
            calls[stage.index()].1.returned_length = returned_length;
            // Bad fields would cause another error if used before length checks.
            calls[stage.index()].1.output = scalars(-1, -1);
            let mut c = clock(calls);
            assert_eq!(run(&mut c).unwrap_err(), Stop::UnexpectedLength {
                stage, expected, returned: returned_length,
            });
            assert!(c.last_sample().is_none());
            assert_eq!(entered(&c).len(), stage.index() + 1);
            let report = c.latest_report().unwrap();
            assert_stopped_at(report, stage);
            match report.call(stage) {
                NativeOutcome::Returned(call) => {
                    assert_eq!(call.buffer_capacity(), Some(expected));
                    assert_eq!(call.returned_length(), returned_length);
                }
                other => panic!("missing returned call: {other:?}"),
            }
        }
    }
}

#[test]
fn each_stage_rejects_negative_seconds_and_out_of_range_microseconds() {
    for stage in [Stage::BootBefore, Stage::Calendar, Stage::BootAfter] {
        for (output, field, value) in [
            (scalars(-1, 0), Field::Seconds, -1),
            (scalars(i64::MIN, 0), Field::Seconds, i64::MIN),
            (scalars(10, -1), Field::Microseconds, -1),
            (scalars(10, i32::MIN), Field::Microseconds, i64::from(i32::MIN)),
            (scalars(10, 1_000_000), Field::Microseconds, 1_000_000),
            (scalars(10, i32::MAX), Field::Microseconds, i64::from(i32::MAX)),
        ] {
            let mut calls = elapsed(1_000);
            calls[stage.index()].1.output = output;
            let mut c = clock(calls);
            assert_eq!(run(&mut c).unwrap_err(), Stop::InvalidScalar { stage, field, value });
            assert_eq!(entered(&c).len(), stage.index() + 1);
            assert_stopped_at(c.latest_report().unwrap(), stage);
            assert!(c.last_sample().is_none());
        }
    }
}

#[test]
fn changed_boot_seconds_or_microseconds_and_calendar_before_boot_refuse() {
    for changed in [scalars(11, 123), scalars(10, 124)] {
        let mut calls = elapsed(5_000_000);
        calls[2].1.output = changed;
        let mut c = clock(calls);
        assert_eq!(run(&mut c).unwrap_err(), Stop::BootChanged);
        assert_eq!(entered(&c), &[Stage::BootBefore, Stage::Calendar, Stage::BootAfter]);
        assert!(c.last_sample().is_none());
    }
    for calendar in [scalars(9, 999_999), scalars(10, 122)] {
        let mut c = clock(triple(scalars(10, 123), calendar));
        assert_eq!(run(&mut c).unwrap_err(), Stop::CalendarBeforeBoot);
        assert_eq!(entered(&c).len(), 3);
        assert!(c.last_sample().is_none());
    }
}

#[test]
fn post_read_refusals_preserve_accepted_state_and_allow_later_recovery() {
    for changed_boot in [false, true] {
        let mut rejected = triple(scalars(10, 123), scalars(10, 122));
        if changed_boot {
            rejected[2].1.output.microseconds = 124;
        }
        let mut c = clock(elapsed(1_000).into_iter().chain(rejected).chain(elapsed(2_000)));
        let historical = run(&mut c).unwrap();
        let before = accepted(&c);
        let expected = if changed_boot { Stop::BootChanged } else { Stop::CalendarBeforeBoot };
        assert_eq!(run(&mut c).unwrap_err(), expected);
        assert_eq!(accepted(&c), before);
        assert_eq!(c.latest_report().unwrap().result(), Err(expected));
        assert_eq!(run(&mut c).unwrap().sequence(), 3);
        assert_eq!(historical.value(), 1);
        assert_eq!(c.validate(&historical), Ok(()));
    }
}

#[test]
fn fractional_microseconds_subtract_before_millisecond_floor() {
    let mut c = clock(triple(scalars(100, 999_500), scalars(101, 501)));
    let sample = run(&mut c).unwrap();
    assert_eq!(sample.elapsed_microseconds(), 1_001);
    assert_eq!(sample.elapsed_milliseconds(), 1);
    assert_eq!(sample.value(), 1);
    assert_eq!(sample.report().result().unwrap().elapsed_microseconds(), 1_001);
    // Stale errno after rc=0 is retained, and is not treated as a syscall error.
    match sample.report().call(Stage::Calendar) {
        NativeOutcome::Returned(call) => {
            assert_eq!(call.errno_before(), 37);
            assert_eq!(call.errno_after(), 37);
            assert_eq!(call.buffer_capacity(), None);
            assert_eq!(call.returned_length(), None);
        }
        other => panic!("missing calendar receipt: {other:?}"),
    }
}

#[test]
fn maximum_darwin_scalars_widen_to_u128_without_overflow() {
    let maximum = (i64::MAX as u128) * 1_000_000 + 999_999;
    let mut c = clock(triple(scalars(0, 0), scalars(i64::MAX, 999_999)));
    let sample = run(&mut c).unwrap();
    assert_eq!(sample.elapsed_microseconds(), maximum);
    assert_eq!(sample.elapsed_milliseconds(), maximum / 1_000);
    assert_eq!(sample.value(), ((maximum / 1_000) & u128::from(u32::MAX)) as u32);
    let mut near_max = clock(triple(scalars(i64::MAX, 999_998), scalars(i64::MAX, 999_999)));
    assert_eq!(run(&mut near_max).unwrap().elapsed_microseconds(), 1);
}

#[test]
fn valid_zero_repeated_values_and_low32_wrap_are_successes() {
    let wrap_us = (u128::from(u32::MAX) + 1) * 1_000;
    let calls = elapsed(0).into_iter().chain(elapsed(0))
        .chain(elapsed(wrap_us - 1)).chain(elapsed(wrap_us)).chain(elapsed(wrap_us + 1_000));
    let mut c = clock(calls);
    let zero = run(&mut c).unwrap();
    let repeated = run(&mut c).unwrap();
    assert_eq!(zero.value(), 0);
    assert_eq!(zero.elapsed_microseconds(), 0);
    assert_eq!(repeated.value(), 0);
    assert_eq!(repeated.sequence(), 2);
    assert_eq!(run(&mut c).unwrap().value(), u32::MAX);
    assert_eq!(run(&mut c).unwrap().value(), 0);
    assert_eq!(run(&mut c).unwrap().value(), 1);
    assert!(c.first_failure_report().is_none());
    assert_eq!(zero.sequence(), 1);
    assert_eq!(c.validate(&zero), Ok(()));
}

#[test]
fn full_microsecond_regression_refuses_even_with_equal_or_increasing_dword() {
    let wrap_us = (u128::from(u32::MAX) + 1) * 1_000;
    for (previous, regressed) in [(1_999, 1_998), (wrap_us, wrap_us - 1)] {
        let mut c = clock(elapsed(previous).into_iter().chain(elapsed(regressed)));
        let first = run(&mut c).unwrap();
        let before = accepted(&c);
        assert_eq!(run(&mut c).unwrap_err(), Stop::ElapsedRegression {
            previous_microseconds: previous, observed_microseconds: regressed,
        });
        assert_eq!(accepted(&c), before);
        assert_eq!(c.validate(&first), Ok(()));
    }
}

#[test]
fn every_read_refusal_preserves_last_accepted_sample_and_consumes_sequence() {
    for stage in [Stage::BootBefore, Stage::Calendar, Stage::BootAfter] {
        for kind in 0..3 {
            let mut rejected = elapsed(2_000);
            let call = &mut rejected[stage.index()].1;
            match kind {
                0 => { call.return_code = -1; call.errno_after = 4; }
                1 if stage != Stage::Calendar => { call.returned_length = Some(0); }
                _ => { call.output.microseconds = -1; }
            }
            // Leave no trailing scripted calls after the first expected refusal.
            rejected.truncate(stage.index() + 1);
            let mut c = clock(elapsed(1_000).into_iter().chain(rejected).chain(elapsed(3_000)));
            let first = run(&mut c).unwrap();
            let before = accepted(&c);
            assert!(run(&mut c).is_err());
            assert_eq!(accepted(&c), before);
            assert_eq!(c.latest_report().unwrap().sequence(), Some(2));
            let third = run(&mut c).unwrap();
            assert_eq!(third.sequence(), 3);
            assert_eq!(third.value(), 3);
            assert_eq!(first.value(), 1);
            assert_eq!(first.report().sequence(), Some(1));
        }
    }
}

#[test]
fn foreign_equal_valued_samples_are_rejected_and_history_is_immutable() {
    let mut a = clock(elapsed(7_000).into_iter().chain(elapsed(8_000)));
    let mut b = clock(elapsed(7_000));
    let old = run(&mut a).unwrap();
    let old_report = *old.report();
    let copied = old.clone();
    let foreign = run(&mut b).unwrap();
    assert_eq!(old.value(), foreign.value());
    assert_eq!(old.sequence(), foreign.sequence());
    assert_eq!(a.validate(&foreign), Err(Stop::ForeignSample));
    assert_eq!(b.validate(&old), Err(Stop::ForeignSample));
    let newer = run(&mut a).unwrap();
    assert_eq!(a.validate(&old), Ok(()));
    assert_eq!(a.validate(&copied), Ok(()));
    assert_eq!(*old.report(), old_report);
    assert_eq!(copied.value(), 7);
    assert_eq!(newer.value(), 8);
    drop(a);
    let replacement = clock([]);
    assert_eq!(replacement.validate(&old), Err(Stop::ForeignSample));
}

#[test]
fn latest_and_first_failure_receipts_are_bounded_and_preserved() {
    let mut failure = elapsed(2_000);
    failure[0].1.return_code = -1;
    failure[0].1.errno_after = 5;
    failure.truncate(1);
    let mut changed = elapsed(4_000);
    changed[2].1.output.microseconds += 1;
    let mut c = clock(elapsed(1_000).into_iter().chain(failure).chain(elapsed(3_000)).chain(changed));
    let first_sample = run(&mut c).unwrap();
    assert!(run(&mut c).is_err());
    let first_failure = *c.first_failure_report().unwrap();
    assert_eq!(first_failure.sequence(), Some(2));
    let third_sample = run(&mut c).unwrap();
    assert_eq!(c.latest_report(), Some(third_sample.report()));
    assert_eq!(c.first_failure_report(), Some(&first_failure));
    assert_eq!(run(&mut c).unwrap_err(), Stop::BootChanged);
    assert_eq!(c.latest_report().unwrap().sequence(), Some(4));
    assert_eq!(c.first_failure_report(), Some(&first_failure));
    assert_eq!(c.last_sample().unwrap().sequence(), 3);
    assert_eq!(first_sample.value(), 1);
    assert_eq!(first_sample.report().result().unwrap().value(), 1);
}

#[test]
fn sequence_overflow_refuses_before_io_without_resetting_accepted_state() {
    let mut c = clock(elapsed(1_000));
    let first = run(&mut c).unwrap();
    let before = accepted(&c);
    c.last_sequence = u64::MAX; // Private overflow control, no production setter.
    assert_eq!(run(&mut c).unwrap_err(), Stop::SequenceExhausted);
    assert_eq!(run(&mut c).unwrap_err(), Stop::SequenceExhausted);
    assert_eq!(entered(&c).len(), 3);
    assert_eq!(accepted(&c), before);
    let report = c.latest_report().unwrap();
    assert_eq!(report.sequence(), None);
    assert_eq!(report.provenance(), Provenance::NoNativeIo);
    assert_eq!(report.result(), Err(Stop::SequenceExhausted));
    for stage in [Stage::BootBefore, Stage::Calendar, Stage::BootAfter] {
        assert_eq!(report.call(stage), NativeOutcome::NotEntered);
    }
    assert_eq!(c.validate(&first), Ok(()));
}

#[test]
fn last_available_sequence_is_issued_once_before_exhaustion() {
    let mut c = clock(elapsed(0));
    c.last_sequence = u64::MAX - 1; // Private boundary setup only.
    let last = run(&mut c).unwrap();
    assert_eq!(last.sequence(), u64::MAX);
    assert_eq!(last.value(), 0);
    assert_eq!(run(&mut c).unwrap_err(), Stop::SequenceExhausted);
    assert_eq!(entered(&c).len(), 3);
    assert_eq!(c.last_sample().unwrap().sequence(), u64::MAX);
    assert_eq!(c.validate(&last), Ok(()));
}

#[test]
fn calendar_aba_limit_equal_brackets_can_accept_false_forward_evidence() {
    // The actual elapsed time might be 1s, while calendar jumps forward to 2s
    // only during the calendar read and restores before BootAfter. The observed
    // boot values are identical. No third snapshot records the restored clock.
    // At a later genuine 3s reading nonregression also passes. This control
    // demonstrates missing atomicity, NOT detection of the hidden calendar ABA.
    let mut c = clock(elapsed(2_000_000).into_iter().chain(elapsed(3_000_000)));
    assert_eq!(run(&mut c).unwrap().value(), 2_000);
    assert_eq!(run(&mut c).unwrap().value(), 3_000);
    assert!(c.first_failure_report().is_none());
}

#[cfg(all(target_os = "macos", target_pointer_width = "64"))]
#[test]
fn darwin64_declared_layout_control() {
    // Source expectation only; parent independently checks SDK and live target.
    assert_eq!(size_of::<c_long>(), 8);
    assert_eq!(size_of::<DarwinTimeval>(), 16);
    assert_eq!(std::mem::align_of::<DarwinTimeval>(), 8);
    assert_eq!(std::mem::offset_of!(DarwinTimeval, seconds), 0);
    assert_eq!(std::mem::offset_of!(DarwinTimeval, microseconds), 8);
}

#[cfg(all(target_os = "macos", target_pointer_width = "64"))]
#[test]
#[ignore = "UNRUN opt-in real Darwin API control; parent qualification required"]
fn real_clock_control_native_receipt_and_owner() {
    assert_eq!(std::env::var("SIMS_WINMM_REAL_CLOCK_CONTROL").as_deref(), Ok("1"));
    let mut c = Clock::new(Policy::DarwinKernelBootElapsed);
    let sample = c.sample().expect("real boot/calendar bracket must validate");
    assert_eq!(c.validate(&sample), Ok(()));
    assert_eq!(sample.report().provenance(), Provenance::DarwinSdkRealApis);
    for stage in [Stage::BootBefore, Stage::Calendar, Stage::BootAfter] {
        match sample.report().call(stage) {
            NativeOutcome::Returned(call) => {
                assert_eq!(call.return_code(), 0);
                let expected = (stage != Stage::Calendar).then_some(size_of::<DarwinTimeval>());
                assert_eq!(call.buffer_capacity(), expected);
                assert_eq!(call.returned_length(), expected);
            }
            other => panic!("missing real native receipt: {other:?}"),
        }
    }
}

#[cfg(all(target_os = "macos", target_pointer_width = "64"))]
#[test]
#[ignore = "UNRUN opt-in real Darwin API control; calendar changes may refuse"]
fn real_clock_control_second_sample_preserves_history() {
    assert_eq!(std::env::var("SIMS_WINMM_REAL_CLOCK_CONTROL").as_deref(), Ok("1"));
    let mut c = Clock::new(Policy::DarwinKernelBootElapsed);
    let first = c.sample().expect("first real attempt must validate");
    let historical = *first.report();
    let second = c.sample().expect("second real attempt must validate");
    assert!(second.elapsed_microseconds() >= first.elapsed_microseconds());
    assert_eq!(second.sequence(), first.sequence() + 1);
    assert_eq!(*first.report(), historical);
    assert_eq!(c.validate(&first), Ok(()));
}
