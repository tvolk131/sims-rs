use super::super::{
    DurableWriter, FenceMonitor, InputSequence, ObservedState, Recorder, SceneIdentity, finalize,
    spawn_writer, write_records,
};
use super::*;
use sha2::{Digest, Sha256};
use std::cell::{Cell, RefCell};
use std::io::Write;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

const SOURCE: &[u8] = include_bytes!("../../../native/window_present_macos.m");
const HEADER: &[u8] = include_bytes!("../../../native/window_present_macos.h");
const ANALYZER: &str = include_str!("../../../../../scripts/window_presentation_evidence.py");

fn boundary(seconds: u64) -> ClockBoundary {
    ClockBoundary {
        sample: SleepClock {
            absolute_before: seconds * 1_000_000_000,
            continuous: seconds * 1_000_000_000 + 250,
            absolute_after: seconds * 1_000_000_000 + 500,
            timebase_numer: 1,
            timebase_denom: 1,
        },
        quartz_before_seconds: seconds as f64,
        quartz_after_seconds: seconds as f64 + 0.000001,
    }
}

fn summary() -> Summary {
    Summary {
        abi_version: 1,
        struct_size: std::mem::size_of::<Summary>() as u32,
        session_state: 4,
        capabilities: 3,
        flags: 12,
        begin_host_time: 10.0,
        cutoff_host_time: 11.0,
        drain_end_host_time: 12.0,
        snapshot_host_time: 12.5,
        ..Summary::default()
    }
}

fn event(kind: u32, sequence: u64, host_time: f64) -> Event {
    Event {
        abi_version: 1,
        struct_size: std::mem::size_of::<Event>() as u32,
        kind,
        event_sequence: sequence,
        host_time,
        ..Event::default()
    }
}

fn retain(records: &RefCell<Vec<(String, Value)>>, kind: &str, data: Value) {
    records.borrow_mut().push((kind.to_owned(), data));
}

#[test]
fn native_presentation_abi_matches_the_authored_header() {
    assert_eq!(std::mem::size_of::<Event>(), 192);
    assert_eq!(std::mem::align_of::<Event>(), 8);
    assert_eq!(std::mem::offset_of!(Event, host_time), 104);
    assert_eq!(std::mem::size_of::<Counters>(), 304);
    assert_eq!(std::mem::size_of::<Summary>(), 408);
    assert_eq!(std::mem::offset_of!(Summary, counters), 104);
}

#[test]
fn metadata_matches_frozen_source_and_analyzer_without_binary_claims() {
    assert_eq!(
        format!("{:x}", Sha256::digest(SOURCE)),
        NATIVE_SOURCE_SHA256
    );
    assert_eq!(
        format!("{:x}", Sha256::digest(HEADER)),
        NATIVE_HEADER_SHA256
    );
    assert!(ANALYZER.contains(NATIVE_SOURCE_SHA256));
    assert!(ANALYZER.contains(NATIVE_HEADER_SHA256));
    for declaration in [
        format!("SOURCE_CONTRACT = '{SOURCE_CONTRACT}'"),
        format!("CLOCK_API = '{CLOCK_API}'"),
        format!("CLOCK_DESCRIPTION = '{CLOCK_DESCRIPTION}'"),
        format!("BOUNDARY = '{BOUNDARY}'"),
        format!("MAX_BRACKET_US = {MAX_BRACKET_US}"),
    ] {
        assert!(ANALYZER.contains(&declaration));
    }
    let metadata = library_metadata(
        Path::new("authored-unloaded-observer.dylib"),
        SOURCE,
        HEADER,
    )
    .unwrap();
    assert_eq!(metadata["source_contract"], SOURCE_CONTRACT);
    assert_eq!(metadata["source_only"], true);
    assert_eq!(metadata["source_sha256"], NATIVE_SOURCE_SHA256);
    assert_eq!(metadata["header_sha256"], NATIVE_HEADER_SHA256);
    assert_eq!(metadata["abi_version"], 1);
    assert_eq!(metadata["event_size"], 192);
    assert_eq!(metadata["counters_size"], 304);
    assert_eq!(metadata["summary_size"], 408);
    assert_eq!(metadata["loaded_binary_receipt"], false);
    assert_eq!(metadata["loaded_binary_source_attested"], false);
    assert!(
        metadata["qualifications"]
            .as_object()
            .unwrap()
            .values()
            .all(|value| value.as_bool() == Some(false))
    );
    let serialized = serde_json::to_string(&metadata).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&serialized).unwrap(),
        metadata
    );
}

#[test]
fn source_metadata_rejects_missing_changed_and_swapped_source() {
    let mut changed_source = SOURCE.to_vec();
    changed_source[0] ^= 1;
    let mut changed_header = HEADER.to_vec();
    changed_header[0] ^= 1;
    for (source, header) in [
        (&[][..], HEADER),
        (SOURCE, &[][..]),
        (changed_source.as_slice(), HEADER),
        (SOURCE, changed_header.as_slice()),
        (HEADER, SOURCE),
    ] {
        let error = library_metadata(Path::new("never-loaded.dylib"), source, header).unwrap_err();
        assert!(error.to_string().contains("pinned source-only contract"));
    }
}

#[test]
fn source_metadata_does_not_initialize_the_opt_in_observer() {
    assert!(OBSERVER.get().is_none());
    library_metadata(Path::new("no-file-or-loader-access.dylib"), SOURCE, HEADER).unwrap();
    assert!(OBSERVER.get().is_none());
}

#[test]
fn injected_quartz_calls_surround_every_mach_sample_in_order() {
    let calls = RefCell::new(Vec::new());
    let quartz_count = Cell::new(0);
    let absolute_count = Cell::new(0);
    let authored = boundary(9);
    let captured = sample_sleep_clock(
        Timebase { numer: 1, denom: 1 },
        || {
            calls.borrow_mut().push("quartz");
            let count = quartz_count.get();
            quartz_count.set(count + 1);
            if count == 0 {
                authored.quartz_before_seconds
            } else {
                authored.quartz_after_seconds
            }
        },
        || {
            calls.borrow_mut().push("absolute");
            let count = absolute_count.get();
            absolute_count.set(count + 1);
            if count == 0 {
                authored.sample.absolute_before
            } else {
                authored.sample.absolute_after
            }
        },
        || {
            calls.borrow_mut().push("continuous");
            authored.sample.continuous
        },
    )
    .unwrap();
    assert_eq!(
        *calls.borrow(),
        ["quartz", "absolute", "continuous", "absolute", "quartz"]
    );
    assert_eq!(quartz_count.get(), 2);
    assert_eq!(absolute_count.get(), 2);
    assert_eq!(
        sleep_clock_record("begin", captured).unwrap(),
        sleep_clock_record("begin", authored).unwrap()
    );
}

#[test]
fn clock_record_serializes_required_fields_and_preserves_large_ticks() {
    let mut authored = boundary(9);
    authored.sample.absolute_before = u64::MAX - 500;
    authored.sample.continuous = u64::MAX - 250;
    authored.sample.absolute_after = u64::MAX;
    let record = sleep_clock_record("end", authored).unwrap();
    assert_eq!(record["schema"], 1);
    assert_eq!(record["phase"], "end");
    assert_eq!(record["clock_api"], CLOCK_API);
    assert_eq!(record["clock"], SLEEP_CLOCK_DESCRIPTION);
    assert_eq!(
        record["quartz_before_seconds"],
        authored.quartz_before_seconds
    );
    assert_eq!(
        record["quartz_after_seconds"],
        authored.quartz_after_seconds
    );
    assert_eq!(
        record["sample"],
        json!({"absolute_before":u64::MAX - 500,"continuous":u64::MAX - 250,
            "absolute_after":u64::MAX,"timebase_numer":1,"timebase_denom":1})
    );
    assert_eq!(record["sample"]["absolute_after"].as_u64(), Some(u64::MAX));
    let serialized = serde_json::to_string(&record).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&serialized).unwrap(), record);
}

#[test]
fn invalid_timebase_prevents_all_injected_clock_calls() {
    for timebase in [
        Timebase { numer: 0, denom: 1 },
        Timebase { numer: 1, denom: 0 },
        Timebase { numer: 0, denom: 0 },
    ] {
        assert!(
            sample_sleep_clock(
                timebase,
                || panic!("must not sample QuartzCore"),
                || panic!("must not sample Mach absolute"),
                || panic!("must not sample Mach continuous"),
            )
            .is_err()
        );
    }
}

#[test]
fn invalid_samples_never_become_serialized_null_or_filled_clock_evidence() {
    let mutations: [fn(&mut ClockBoundary); 16] = [
        |clock| clock.sample.timebase_numer = 0,
        |clock| clock.sample.timebase_denom = 0,
        |clock| clock.sample.absolute_before = 0,
        |clock| clock.sample.absolute_after = 0,
        |clock| clock.sample.continuous = 0,
        |clock| clock.sample.absolute_after = clock.sample.absolute_before - 1,
        |clock| clock.sample.absolute_after = clock.sample.absolute_before + 1_000_001,
        |clock| clock.quartz_before_seconds = 0.0,
        |clock| clock.quartz_before_seconds = -1.0,
        |clock| clock.quartz_before_seconds = f64::NAN,
        |clock| clock.quartz_before_seconds = f64::INFINITY,
        |clock| clock.quartz_after_seconds = f64::NAN,
        |clock| clock.quartz_after_seconds = f64::INFINITY,
        |clock| clock.quartz_after_seconds = f64::NEG_INFINITY,
        |clock| clock.quartz_after_seconds = clock.quartz_before_seconds - 0.000001,
        |clock| clock.quartz_after_seconds = clock.quartz_before_seconds + 0.002,
    ];
    for mutate in mutations {
        let mut invalid = boundary(9);
        mutate(&mut invalid);
        assert!(invalid.validate().is_err(), "accepted {invalid:?}");
        assert!(sleep_clock_record("begin", invalid).is_err());
    }
    assert!(sleep_clock_record("middle", boundary(9)).is_err());
}

#[test]
fn mach_bracket_limit_uses_wide_integer_timebase_arithmetic() {
    let mut authored = boundary(9);
    authored.sample.absolute_after = authored.sample.absolute_before + 1_000_000;
    assert!(authored.validate().is_ok());
    authored.sample.absolute_after += 1;
    assert!(authored.validate().is_err());
    authored.sample.absolute_before = 1;
    authored.sample.absolute_after = u64::MAX;
    authored.sample.timebase_numer = u32::MAX;
    authored.sample.timebase_denom = u32::MAX;
    assert!(authored.validate().is_err());
    authored.sample.absolute_after = 1;
    assert!(authored.validate().is_ok());
}

#[test]
fn complete_clock_brackets_cover_the_actual_native_snapshot() {
    validate_clock_coverage(boundary(9), boundary(13), &summary()).unwrap();
    let mut native = summary();
    native.snapshot_host_time = 13.5;
    assert!(validate_clock_coverage(boundary(9), boundary(13), &native).is_err());
    assert!(validate_clock_coverage(boundary(10), boundary(13), &summary()).is_err());
    assert!(validate_clock_coverage(boundary(9), boundary(12), &summary()).is_err());
}

#[test]
fn coverage_rejects_timebase_changes_and_cross_sample_regressions() {
    let mut end = boundary(13);
    end.sample.timebase_numer = 2;
    assert!(validate_clock_coverage(boundary(9), end, &summary()).is_err());
    end = boundary(13);
    end.sample.timebase_denom = 2;
    assert!(validate_clock_coverage(boundary(9), end, &summary()).is_err());
    end = boundary(13);
    end.sample.absolute_before = 1;
    end.sample.absolute_after = 2;
    assert!(validate_clock_coverage(boundary(9), end, &summary()).is_err());
    end = boundary(13);
    end.sample.continuous = 1;
    assert!(validate_clock_coverage(boundary(9), end, &summary()).is_err());
}

#[test]
fn coverage_rejects_invalid_native_abi_and_nonfinite_or_regressing_boundaries() {
    let mutations: [fn(&mut Summary); 11] = [
        |native| native.abi_version = 0,
        |native| native.struct_size = 0,
        |native| native.session_state = 3,
        |native| native.begin_host_time = 0.0,
        |native| native.begin_host_time = f64::NAN,
        |native| native.cutoff_host_time = f64::INFINITY,
        |native| native.drain_end_host_time = f64::NEG_INFINITY,
        |native| native.snapshot_host_time = f64::NAN,
        |native| native.cutoff_host_time = native.begin_host_time,
        |native| native.drain_end_host_time = native.cutoff_host_time - 1.0,
        |native| native.snapshot_host_time = native.drain_end_host_time - 1.0,
    ];
    for mutate in mutations {
        let mut native = summary();
        mutate(&mut native);
        assert!(validate_clock_coverage(boundary(9), boundary(13), &native).is_err());
    }
}

#[test]
fn native_begin_and_platform_callbacks_precede_successful_begin_metadata() {
    let state = AtomicU8::new(1);
    let records = RefCell::new(Vec::new());
    begin_recording(
        &state,
        || {
            assert!(records.borrow().is_empty());
            retain(
                &records,
                "presentation_event",
                serde_json::to_value(event(1, 1, 10.0)).unwrap(),
            );
            let mut platform = event(9, 2, 10.1);
            platform.platform_notification = 6;
            retain(
                &records,
                "presentation_event",
                serde_json::to_value(platform).unwrap(),
            );
            assert_eq!(records.borrow().len(), 2);
            0
        },
        |kind, data| retain(&records, kind, data),
    );
    let records = records.borrow();
    assert_eq!(
        records
            .iter()
            .map(|record| record.0.as_str())
            .collect::<Vec<_>>(),
        [
            "presentation_event",
            "presentation_event",
            "presentation_begin"
        ]
    );
    assert_eq!(records[0].1["event_sequence"], 1);
    assert_eq!(records[1].1["platform_notification"], 6);
    assert_eq!(records[2].1["status"], 0);
    assert_eq!(records[2].1["clock_api"], CLOCK_API);
    assert_eq!(records[2].1["clock"], CLOCK_DESCRIPTION);
    assert_eq!(records[2].1["boundary"], BOUNDARY);
    assert_eq!(state.load(Ordering::Relaxed), 1);
}

#[test]
fn failed_native_begin_is_reported_after_return_without_success_predeclaration() {
    let state = AtomicU8::new(1);
    let returned = Cell::new(false);
    let records = RefCell::new(Vec::new());
    begin_recording(
        &state,
        || {
            assert!(records.borrow().is_empty());
            returned.set(true);
            -7
        },
        |kind, data| {
            assert!(returned.get());
            assert_eq!(state.load(Ordering::Relaxed), 2);
            retain(&records, kind, data);
        },
    );
    assert_eq!(records.borrow().len(), 1);
    assert_eq!(records.borrow()[0].1["status"], -7);
}

#[test]
fn native_drain_events_and_final_snapshot_precede_end_sample_and_metadata() {
    let calls = RefCell::new(Vec::new());
    let records = RefCell::new(Vec::new());
    finish_recording(
        boundary(9),
        |native| {
            calls.borrow_mut().push("native_end");
            for (kind, sequence, time) in [(11, 1, 11.0), (6, 2, 11.5), (13, 3, 12.0)] {
                retain(
                    &records,
                    "presentation_event",
                    serde_json::to_value(event(kind, sequence, time)).unwrap(),
                );
            }
            *native = summary();
            native.last_event_sequence = 3;
            native.counters.emitted_events = 3;
            calls.borrow_mut().push("final_native_snapshot");
            0
        },
        || {
            assert_eq!(records.borrow().len(), 3);
            calls.borrow_mut().push("end_clock_sample");
            Ok(boundary(13))
        },
        |kind, data| {
            calls
                .borrow_mut()
                .push(if kind == "presentation_sleep_clock" {
                    "sleep_record"
                } else {
                    "end_record"
                });
            retain(&records, kind, data);
        },
    );
    assert_eq!(
        *calls.borrow(),
        [
            "native_end",
            "final_native_snapshot",
            "end_clock_sample",
            "sleep_record",
            "end_record"
        ]
    );
    let records = records.borrow();
    assert_eq!(
        records
            .iter()
            .map(|record| record.0.as_str())
            .collect::<Vec<_>>(),
        [
            "presentation_event",
            "presentation_event",
            "presentation_event",
            "presentation_sleep_clock",
            "presentation_end"
        ]
    );
    assert_eq!(records[3].1["phase"], "end");
    assert_eq!(records[4].1["summary"]["snapshot_host_time"], 12.5);
    assert_eq!(records[4].1["summary"]["counters"]["emitted_events"], 3);
}

#[test]
fn failed_end_sampling_retains_native_summary_without_inventing_a_clock() {
    let records = RefCell::new(Vec::new());
    finish_recording(
        boundary(9),
        |native| {
            *native = summary();
            0
        },
        || anyhow::bail!("authored sample failure"),
        |kind, data| retain(&records, kind, data),
    );
    let records = records.borrow();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].0, "presentation_error");
    assert_eq!(records[0].1["phase"], "end");
    assert_eq!(records[0].1["error"], "authored sample failure");
    assert_eq!(records[1].0, "presentation_end");
    assert_eq!(
        records[1].1["summary"],
        serde_json::to_value(summary()).unwrap()
    );
}

#[test]
fn failed_native_end_without_snapshot_does_not_fabricate_clock_evidence() {
    let records = RefCell::new(Vec::new());
    let sampled = Cell::new(false);
    finish_recording(
        boundary(9),
        |_| -8,
        || {
            sampled.set(true);
            Ok(boundary(13))
        },
        |kind, data| retain(&records, kind, data),
    );
    assert!(sampled.get());
    let records = records.borrow();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].0, "presentation_error");
    assert_eq!(records[1].0, "presentation_end");
    assert_eq!(records[1].1["status"], -8);
    assert_eq!(
        records[1].1["summary"],
        serde_json::to_value(Summary::default()).unwrap()
    );
}

#[test]
fn uncovered_final_snapshot_reports_failure_without_retiming_or_filling() {
    let records = RefCell::new(Vec::new());
    finish_recording(
        boundary(9),
        |native| {
            *native = summary();
            native.snapshot_host_time = 14.0;
            0
        },
        || Ok(boundary(13)),
        |kind, data| retain(&records, kind, data),
    );
    let records = records.borrow();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].0, "presentation_error");
    assert!(
        records[0].1["error"]
            .as_str()
            .unwrap()
            .contains("final snapshot")
    );
    assert_eq!(records[1].1["summary"]["snapshot_host_time"], 14.0);
}

#[test]
fn native_failure_status_and_late_accounting_are_not_rewritten() {
    let records = RefCell::new(Vec::new());
    let mut native = summary();
    native.status = -13;
    native.flags = 13;
    native.last_event_sequence = 41;
    native.counters.late_callbacks = 2;
    native.counters.late_binds = 3;
    native.counters.emitted_events = 41;
    native.counters.unresolved_events = 1;
    finish_recording(
        boundary(9),
        |output| {
            *output = native;
            -12
        },
        || Ok(boundary(13)),
        |kind, data| retain(&records, kind, data),
    );
    let records = records.borrow();
    assert_eq!(records[0].0, "presentation_sleep_clock");
    assert_eq!(records[1].1["status"], -12);
    assert_eq!(
        records[1].1["summary"],
        serde_json::to_value(native).unwrap()
    );
}

#[test]
fn late_callback_during_end_sampling_is_retained_without_rewriting_the_snapshot() {
    let records = RefCell::new(Vec::new());
    let mut snapshot = summary();
    snapshot.last_event_sequence = 3;
    snapshot.counters.emitted_events = 3;
    let mut late = event(6, 4, 12.75);
    late.flags = 1 | 2;
    late.callback_delivery_host_time = 12.75;
    finish_recording(
        boundary(9),
        |native| {
            *native = snapshot;
            0
        },
        || {
            retain(
                &records,
                "presentation_event",
                serde_json::to_value(late).unwrap(),
            );
            Ok(boundary(13))
        },
        |kind, data| retain(&records, kind, data),
    );
    let records = records.borrow();
    assert_eq!(records.len(), 3);
    assert_eq!(records[0].0, "presentation_event");
    assert_eq!(records[0].1, serde_json::to_value(late).unwrap());
    assert_eq!(records[1].0, "presentation_sleep_clock");
    assert_eq!(records[2].0, "presentation_end");
    assert_eq!(
        records[2].1["summary"],
        serde_json::to_value(snapshot).unwrap()
    );
}

#[test]
fn packet_serialization_preserves_raw_late_event_identity_and_explicit_drop_count() {
    let mut raw = event(6, u64::MAX, 13.5);
    raw.flags = 1 | 2;
    raw.acquisition_sequence = 37;
    raw.texture_identity = u64::MAX - 1;
    raw.presented_host_time = 11.75;
    raw.callback_delivery_host_time = 13.5;
    raw.status = -9;
    let packet = Packet {
        kind: "presentation_event",
        elapsed_us: 17,
        dropped_records: 5,
        data: raw,
    };
    let serialized = serde_json::to_value(packet).unwrap();
    assert_eq!(serialized["data"], serde_json::to_value(raw).unwrap());
    assert_eq!(serialized["dropped_records"], 5);
    assert_eq!(serialized["elapsed_us"], 17);
    assert_eq!(serialized["data"]["acquisition_enter_host_time"], 0.0);
    assert_eq!(
        serialized["data"]["event_sequence"].as_u64(),
        Some(u64::MAX)
    );
}

fn callback_recorder(
    sender: mpsc::SyncSender<Record>,
    writer_result: mpsc::Receiver<anyhow::Result<()>>,
) -> Recorder {
    Recorder {
        loaded_input: crate::support::window_session::loaded_input::Account::default(),
        start: Instant::now(),
        sender,
        writer_result: Mutex::new(writer_result),
        dropped: AtomicU64::new(0),
        frames: AtomicU64::new(0),
        drawn_state: AtomicU64::new(0),
        state: Mutex::new(ObservedState::default()),
        scene: Mutex::new(SceneIdentity::default()),
        previous_draw: Mutex::new(None),
        observer_fence: Arc::new(FenceMonitor::new()),
        fence: AtomicU64::new(0),
        inputs: InputSequence::default(),
        spans: AtomicU64::new(0),
    }
}

fn callback_queue(capacity: usize) -> (Recorder, mpsc::Receiver<Record>) {
    let (sender, receiver) = mpsc::sync_channel(capacity);
    let (_, result) = mpsc::sync_channel(1);
    (callback_recorder(sender, result), receiver)
}

fn callback_packet(receiver: &mpsc::Receiver<Record>) -> Packet {
    match receiver.try_recv().unwrap() {
        Record::Presentation(packet) => packet,
        _ => panic!("expected a presentation packet"),
    }
}

#[derive(Default)]
struct CallbackOutput {
    bytes: Vec<u8>,
    durable: Vec<u8>,
    operations: Vec<&'static str>,
}

#[derive(Clone, Copy)]
enum CallbackFailure {
    Write,
    Flush,
    Sync,
}

#[derive(Default)]
struct CallbackWriter {
    output: Arc<Mutex<CallbackOutput>>,
    failure: Option<CallbackFailure>,
}

impl Write for CallbackWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if matches!(self.failure, Some(CallbackFailure::Write)) {
            return Err(std::io::Error::other("injected callback write failure"));
        }
        self.output.lock().unwrap().bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.output.lock().unwrap().operations.push("flush");
        if matches!(self.failure, Some(CallbackFailure::Flush)) {
            return Err(std::io::Error::other("injected callback flush failure"));
        }
        Ok(())
    }
}

impl DurableWriter for CallbackWriter {
    fn sync_data(&self) -> std::io::Result<()> {
        let mut output = self.output.lock().unwrap();
        output.operations.push("sync");
        if matches!(self.failure, Some(CallbackFailure::Sync)) {
            return Err(std::io::Error::other("injected callback sync failure"));
        }
        output.durable = output.bytes.clone();
        Ok(())
    }
}

fn callback_records(bytes: &[u8]) -> Vec<Value> {
    assert!(bytes.ends_with(b"\n"));
    std::str::from_utf8(bytes)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn callback_dispatch_counts_null_input_without_enqueuing() {
    let (recorder, receiver) = callback_queue(1);
    unsafe { receive_for_recorder(&recorder, std::ptr::null()) };
    assert_eq!(recorder.dropped.load(Ordering::Relaxed), 1);
    assert!(matches!(
        receiver.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
}

#[test]
fn callback_dispatch_counts_each_rejected_abi_or_size_without_enqueuing() {
    let (recorder, receiver) = callback_queue(1);
    for (index, (version, size)) in [
        (0, std::mem::size_of::<Event>() as u32),
        (2, std::mem::size_of::<Event>() as u32),
        (1, std::mem::size_of::<Event>() as u32 - 1),
        (1, std::mem::size_of::<Event>() as u32 + 1),
    ]
    .into_iter()
    .enumerate()
    {
        let mut raw = event(6, 1, 12.0);
        raw.abi_version = version;
        raw.struct_size = size;
        unsafe { receive_for_recorder(&recorder, &raw) };
        assert_eq!(recorder.dropped.load(Ordering::Relaxed), index as u64 + 1);
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
    }
}

#[test]
fn callback_dispatch_copies_raw_late_identity_before_returning() {
    let (recorder, receiver) = callback_queue(1);
    let mut raw = event(6, 7, 12.75);
    raw.flags = 1 | 2;
    raw.acquisition_sequence = 37;
    raw.layer_lifetime_id = 2;
    raw.drawable_id = 5;
    raw.texture_identity = u64::MAX;
    raw.presented_host_time = 11.75;
    raw.callback_delivery_host_time = 12.75;
    let expected = serde_json::to_value(raw).unwrap();
    unsafe { receive_for_recorder(&recorder, &raw) };
    raw.event_sequence = 8;
    let packet = callback_packet(&receiver);
    assert_eq!(serde_json::to_value(packet.data).unwrap(), expected);
    assert_eq!(raw.event_sequence, 8);
    assert_eq!(packet.dropped_records, 0);
    assert_eq!(recorder.dropped.load(Ordering::Relaxed), 0);
    assert_eq!(packet.kind, "presentation_event");
    assert!(packet.elapsed_us <= recorder.start.elapsed().as_micros());
}

#[test]
fn callback_dispatch_full_queue_counts_loss_and_next_packet_captures_it() {
    let (recorder, receiver) = callback_queue(1);
    let first = event(6, 1, 12.0);
    let second = event(6, 2, 12.5);
    let third = event(6, 3, 13.0);
    unsafe {
        receive_for_recorder(&recorder, &first);
        receive_for_recorder(&recorder, &second);
    }
    assert_eq!(recorder.dropped.load(Ordering::Relaxed), 1);
    let retained = callback_packet(&receiver);
    assert_eq!(retained.data.event_sequence, 1);
    assert_eq!(retained.dropped_records, 0);
    unsafe { receive_for_recorder(&recorder, &third) };
    let retained = callback_packet(&receiver);
    assert_eq!(retained.data.event_sequence, 3);
    assert_eq!(retained.dropped_records, 1);
    assert_eq!(recorder.dropped.load(Ordering::Relaxed), 1);
}

#[test]
fn callback_dispatch_disconnected_queue_counts_every_loss() {
    let (recorder, receiver) = callback_queue(1);
    drop(receiver);
    for sequence in 1..=3 {
        let raw = event(6, sequence, 12.0);
        unsafe { receive_for_recorder(&recorder, &raw) };
        assert_eq!(recorder.dropped.load(Ordering::Relaxed), sequence);
    }
}

#[test]
fn callback_packets_reach_writer_and_durable_finalization_in_enqueue_order() {
    let writer = CallbackWriter::default();
    let output = Arc::clone(&writer.output);
    let (sender, result) = spawn_writer(writer, 4).unwrap();
    let recorder = callback_recorder(sender, result);
    let superseded = event(8, 1, 12.5);
    let returned = event(4, 2, 12.25);
    unsafe {
        receive_for_recorder(&recorder, &superseded);
        receive_for_recorder(&recorder, &returned);
    }
    let terminal = json!({"kind":"session_end","dropped_records":0});
    let result = recorder.writer_result.lock().unwrap();
    finalize(
        &recorder.sender,
        &result,
        terminal.clone(),
        Duration::from_secs(2),
    )
    .unwrap();
    result
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .unwrap();
    let output = output.lock().unwrap();
    let records = callback_records(&output.durable);
    assert_eq!(records.len(), 3);
    assert_eq!(
        records[0]["data"],
        serde_json::to_value(superseded).unwrap()
    );
    assert_eq!(records[1]["data"], serde_json::to_value(returned).unwrap());
    assert_eq!(records[2], terminal);
    assert_eq!(output.bytes, output.durable);
    assert_eq!(output.operations.last(), Some(&"sync"));
    assert_eq!(recorder.dropped.load(Ordering::Relaxed), 0);
}

#[test]
fn callback_writer_failures_cannot_produce_durable_acknowledgement() {
    for (failure, expected) in [
        (CallbackFailure::Write, "injected callback write failure"),
        (CallbackFailure::Flush, "injected callback flush failure"),
        (CallbackFailure::Sync, "injected callback sync failure"),
    ] {
        let (recorder, receiver) = callback_queue(2);
        let raw = event(6, 1, 12.0);
        unsafe { receive_for_recorder(&recorder, &raw) };
        let (done, acknowledgement) = mpsc::sync_channel(1);
        recorder
            .sender
            .try_send(Record::Finalize {
                end: json!({"kind":"session_end"}),
                done,
            })
            .unwrap();
        let writer = CallbackWriter {
            failure: Some(failure),
            ..CallbackWriter::default()
        };
        let output = Arc::clone(&writer.output);
        let error = write_records(writer, receiver).unwrap_err();
        assert!(format!("{error:#}").contains(expected));
        assert!(output.lock().unwrap().durable.is_empty());
        assert!(matches!(
            acknowledgement.try_recv(),
            Err(mpsc::TryRecvError::Disconnected)
        ));
        assert_eq!(recorder.dropped.load(Ordering::Relaxed), 0);
    }
}

#[test]
fn callback_queued_after_finalize_is_not_durable_or_counted_as_dropped() {
    let (recorder, receiver) = callback_queue(3);
    let before = event(6, 1, 12.0);
    unsafe { receive_for_recorder(&recorder, &before) };
    let (done, acknowledgement) = mpsc::sync_channel(1);
    recorder
        .sender
        .try_send(Record::Finalize {
            end: json!({"kind":"session_end","dropped_records":0}),
            done,
        })
        .unwrap();
    let mut late = event(6, 2, 13.0);
    late.flags = 1 | 2;
    unsafe { receive_for_recorder(&recorder, &late) };
    let writer = CallbackWriter::default();
    let output = Arc::clone(&writer.output);
    write_records(writer, receiver).unwrap();
    acknowledgement.try_recv().unwrap();
    let output = output.lock().unwrap();
    let records = callback_records(&output.durable);
    assert_eq!(records.len(), 2);
    assert_eq!(records[0]["data"], serde_json::to_value(before).unwrap());
    assert_eq!(records[1]["kind"], "session_end");
    assert_eq!(recorder.dropped.load(Ordering::Relaxed), 0);
}

#[test]
fn callback_after_writer_return_counts_loss_beyond_the_terminal_snapshot() {
    let (recorder, receiver) = callback_queue(1);
    let (done, acknowledgement) = mpsc::sync_channel(1);
    recorder
        .sender
        .try_send(Record::Finalize {
            end: json!({"kind":"session_end","dropped_records":0}),
            done,
        })
        .unwrap();
    let writer = CallbackWriter::default();
    let output = Arc::clone(&writer.output);
    write_records(writer, receiver).unwrap();
    acknowledgement.try_recv().unwrap();
    let mut late = event(6, 1, 13.0);
    late.flags = 1 | 2;
    unsafe { receive_for_recorder(&recorder, &late) };
    let output = output.lock().unwrap();
    let records = callback_records(&output.durable);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["dropped_records"], 0);
    assert_eq!(recorder.dropped.load(Ordering::Relaxed), 1);
}
