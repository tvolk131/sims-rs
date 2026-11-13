use super::{RECORDER, Record, emit};
use anyhow::{Result, ensure};
use serde::Serialize;
use serde_json::{Value, json};
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

const SOURCE_CONTRACT: &str = "sims-window-presentation-source-only-v1";
const NATIVE_SOURCE_SHA256: &str =
    "fdb8c118c964c5725168182ac06d5343371136581d1757717b109c6450d10c5c";
const NATIVE_HEADER_SHA256: &str =
    "af8bfedc623d8f6cea9c0aed31dbc1d69a624822296c75f016f3c5133e9c0fdb";
const CLOCK_API: &str = "QuartzCore.CACurrentMediaTime";
const CLOCK_DESCRIPTION: &str = "CACurrentMediaTime host seconds; not UNIX time";
const SLEEP_CLOCK_DESCRIPTION: &str =
    "mach timebase ticks; absolute stops during sleep; continuous includes sleep";
const BOUNDARY: &str = "MTLDrawable.presentedTime from addPresentedHandler";
const MAX_BRACKET_US: u64 = 1000;

#[repr(C)]
#[derive(Default)]
struct Timebase {
    numer: u32,
    denom: u32,
}

#[derive(Clone, Copy, Debug, Serialize)]
struct SleepClock {
    absolute_before: u64,
    continuous: u64,
    absolute_after: u64,
    timebase_numer: u32,
    timebase_denom: u32,
}

#[derive(Clone, Copy, Debug)]
struct ClockBoundary {
    sample: SleepClock,
    quartz_before_seconds: f64,
    quartz_after_seconds: f64,
}

impl ClockBoundary {
    fn validate(&self) -> Result<()> {
        let sample = self.sample;
        ensure!(
            sample.timebase_numer > 0 && sample.timebase_denom > 0,
            "Mach clock timebase unavailable"
        );
        ensure!(
            sample.absolute_before > 0
                && sample.continuous > 0
                && sample.absolute_before <= sample.absolute_after,
            "Invalid or regressing Mach sleep-clock sample"
        );
        ensure!(
            self.quartz_before_seconds.is_finite()
                && self.quartz_after_seconds.is_finite()
                && self.quartz_before_seconds > 0.0
                && self.quartz_before_seconds <= self.quartz_after_seconds,
            "Invalid or regressing QuartzCore clock bracket"
        );
        ensure!(
            (sample.absolute_after - sample.absolute_before) as u128
                * sample.timebase_numer as u128
                <= MAX_BRACKET_US as u128 * sample.timebase_denom as u128 * 1000
                && self.quartz_after_seconds - self.quartz_before_seconds
                    < MAX_BRACKET_US as f64 / 1_000_000.0,
            "Ambiguous Mach/QuartzCore sleep-clock bracket"
        );
        Ok(())
    }
}

unsafe extern "C" {
    fn mach_absolute_time() -> u64;
    fn mach_continuous_time() -> u64;
    fn mach_timebase_info(info: *mut Timebase) -> i32;
}

#[link(name = "QuartzCore", kind = "framework")]
unsafe extern "C" {
    fn CACurrentMediaTime() -> f64;
}

fn sample_sleep_clock(
    timebase: Timebase,
    mut quartz: impl FnMut() -> f64,
    mut absolute: impl FnMut() -> u64,
    continuous: impl FnOnce() -> u64,
) -> Result<ClockBoundary> {
    ensure!(
        timebase.numer > 0 && timebase.denom > 0,
        "Mach clock timebase unavailable"
    );
    let quartz_before_seconds = quartz();
    let absolute_before = absolute();
    let continuous = continuous();
    let absolute_after = absolute();
    let quartz_after_seconds = quartz();
    let boundary = ClockBoundary {
        sample: SleepClock {
            absolute_before,
            continuous,
            absolute_after,
            timebase_numer: timebase.numer,
            timebase_denom: timebase.denom,
        },
        quartz_before_seconds,
        quartz_after_seconds,
    };
    boundary.validate()?;
    Ok(boundary)
}

fn sleep_clock() -> Result<ClockBoundary> {
    let mut timebase = Timebase::default();
    ensure!(
        unsafe { mach_timebase_info(&mut timebase) } == 0
            && timebase.numer > 0
            && timebase.denom > 0,
        "Mach clock timebase unavailable"
    );
    sample_sleep_clock(
        timebase,
        || unsafe { CACurrentMediaTime() },
        || unsafe { mach_absolute_time() },
        || unsafe { mach_continuous_time() },
    )
}

fn sleep_clock_record(phase: &str, boundary: ClockBoundary) -> Result<Value> {
    ensure!(
        matches!(phase, "begin" | "end"),
        "Invalid sleep-clock phase"
    );
    boundary.validate()?;
    Ok(json!({"schema":1,"phase":phase,"sample":boundary.sample,
        "clock":SLEEP_CLOCK_DESCRIPTION,"clock_api":CLOCK_API,
        "quartz_before_seconds":boundary.quartz_before_seconds,
        "quartz_after_seconds":boundary.quartz_after_seconds}))
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub(super) struct Event {
    abi_version: u32,
    struct_size: u32,
    kind: u32,
    flags: u32,
    event_sequence: u64,
    acquisition_sequence: u64,
    layer_lifetime_id: u64,
    drawable_id: u64,
    texture_identity: u64,
    draw_context_sequence: u64,
    frame_id: u64,
    state_id: u64,
    bound_frame_id: u64,
    bound_state_id: u64,
    bind_count: u64,
    host_time: f64,
    acquisition_enter_host_time: f64,
    acquisition_call_host_time: f64,
    acquisition_return_host_time: f64,
    presented_host_time: f64,
    callback_delivery_host_time: f64,
    drawable_width: f64,
    drawable_height: f64,
    display_sync_enabled: u32,
    maximum_drawable_count: u32,
    allows_next_drawable_timeout: u32,
    presents_with_transaction: u32,
    platform_notification: u32,
    status: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Serialize)]
struct Counters {
    acquisition_entries: u64,
    acquisition_returns: u64,
    acquisition_null_returns: u64,
    successful_acquisitions: u64,
    acquisition_exceptions: u64,
    handler_registration_attempts: u64,
    handler_registrations: u64,
    handler_registration_failures: u64,
    presented_callbacks: u64,
    positive_presentations: u64,
    zero_presented_timestamps: u64,
    invalid_presented_timestamps: u64,
    duplicate_callbacks: u64,
    callback_lookup_conflicts: u64,
    bind_calls: u64,
    successful_binds: u64,
    duplicate_binds: u64,
    missing_binds: u64,
    bind_lookup_conflicts: u64,
    binding_conflicts: u64,
    missing_draw_contexts: u64,
    texture_lifetime_reuses: u64,
    texture_reuses_before_callback: u64,
    texture_identity_conflicts: u64,
    bookkeeping_overflows: u64,
    overflow_untraced_calls: u64,
    outstanding_acquisitions: u64,
    active_acquisition_calls: u64,
    callbacks_awaited: u64,
    emitted_events: u64,
    draw_context_calls: u64,
    platform_notifications: u64,
    observation_errors: u64,
    unresolved_events: u64,
    late_callbacks: u64,
    late_binds: u64,
    post_callback_bind_attempts: u64,
    reentrant_acquisition_calls: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Serialize)]
struct Summary {
    abi_version: u32,
    struct_size: u32,
    status: i32,
    session_state: u32,
    capabilities: u64,
    flags: u64,
    begin_host_time: f64,
    cutoff_host_time: f64,
    drain_end_host_time: f64,
    snapshot_host_time: f64,
    last_event_sequence: u64,
    last_acquisition_sequence: u64,
    unresolved_acquisitions: u64,
    unbound_acquisitions: u64,
    unresolved_unbound_acquisitions: u64,
    counters: Counters,
}

fn library_metadata(path: &Path, source: &[u8], header: &[u8]) -> Result<Value> {
    ensure!(
        sims_runtime::exploration::digest(source) == NATIVE_SOURCE_SHA256
            && sims_runtime::exploration::digest(header) == NATIVE_HEADER_SHA256,
        "Compile-time presentation source/header differs from the pinned source-only contract"
    );
    ensure!(
        std::mem::size_of::<Event>() == 192
            && std::mem::size_of::<Counters>() == 304
            && std::mem::size_of::<Summary>() == 408,
        "Presentation ABI sizes differ from the pinned source-only contract"
    );
    Ok(
        json!({"path":path,"source_contract":SOURCE_CONTRACT,"source_only":true,
        "source_sha256":NATIVE_SOURCE_SHA256,"header_sha256":NATIVE_HEADER_SHA256,
        "abi_version":1,"event_size":std::mem::size_of::<Event>(),
        "counters_size":std::mem::size_of::<Counters>(),
        "summary_size":std::mem::size_of::<Summary>(),
        "loaded_binary_receipt":false,"loaded_binary_source_attested":false,
        "qualifications":{"build":false,"test":false,"native":false,
            "presentation":false,"latency":false,"alpha":false},
        "scope":"Pinned compile-time native source/header and ABI only; the loaded path is not a binary receipt or dynamic source attestation; native begin is deferred to the first root draw"}),
    )
}

fn validate_clock_coverage(
    begin: ClockBoundary,
    end: ClockBoundary,
    summary: &Summary,
) -> Result<()> {
    begin.validate()?;
    end.validate()?;
    ensure!(
        begin.sample.timebase_numer == end.sample.timebase_numer
            && begin.sample.timebase_denom == end.sample.timebase_denom,
        "Mach clock timebase changed during presentation observation"
    );
    ensure!(
        begin.sample.absolute_after <= end.sample.absolute_before
            && begin.sample.continuous <= end.sample.continuous,
        "Regressing presentation sleep-clock boundaries"
    );
    ensure!(
        summary.abi_version == 1
            && summary.struct_size as usize == std::mem::size_of::<Summary>()
            && summary.session_state == 4
            && [
                summary.begin_host_time,
                summary.cutoff_host_time,
                summary.drain_end_host_time,
                summary.snapshot_host_time,
            ]
            .iter()
            .all(|time| time.is_finite() && *time > 0.0)
            && summary.begin_host_time < summary.cutoff_host_time
            && summary.cutoff_host_time <= summary.drain_end_host_time
            && summary.drain_end_host_time <= summary.snapshot_host_time,
        "Invalid native presentation snapshot clock boundaries, ABI or session state"
    );
    ensure!(
        begin.quartz_after_seconds <= summary.begin_host_time
            && summary.snapshot_host_time <= end.quartz_before_seconds,
        "Sleep-clock evidence does not cover the native begin/drain/final snapshot"
    );
    Ok(())
}

fn begin_recording(
    state: &AtomicU8,
    native_begin: impl FnOnce() -> i32,
    mut record: impl FnMut(&str, Value),
) {
    let status = native_begin();
    if status != 0 {
        state.store(2, Ordering::Relaxed);
    }
    record(
        "presentation_begin",
        json!({"status":status,"abi_version":1,
            "boundary":BOUNDARY,"clock":CLOCK_DESCRIPTION,"clock_api":CLOCK_API,
            "scope":"API-reported on-screen presentation, not photons or GPU duration"}),
    );
}

fn finish_recording(
    begin: ClockBoundary,
    native_end: impl FnOnce(&mut Summary) -> i32,
    sample_clock: impl FnOnce() -> Result<ClockBoundary>,
    mut record: impl FnMut(&str, Value),
) {
    let mut summary = Summary::default();
    let status = native_end(&mut summary);
    let clock_record = sample_clock().and_then(|end| {
        validate_clock_coverage(begin, end, &summary)?;
        sleep_clock_record("end", end)
    });
    match clock_record {
        Ok(data) => record("presentation_sleep_clock", data),
        Err(error) => record(
            "presentation_error",
            json!({"operation":"sleep_clock","phase":"end","error":error.to_string()}),
        ),
    }
    record(
        "presentation_end",
        json!({"status":status,"summary":summary,
            "scope":"End-clock sampling follows native end return; drain/snapshot semantics describe pinned source, not binary attestation; later callbacks remain raw events"}),
    );
}

#[derive(Serialize)]
pub(super) struct Packet {
    kind: &'static str,
    elapsed_us: u128,
    dropped_records: u64,
    data: Event,
}

type Callback = unsafe extern "C" fn(*const Event, *mut c_void);
type Begin = unsafe extern "C" fn(u32, u32, Callback, *mut c_void) -> i32;
type DrawContext = unsafe extern "C" fn(u64, u64) -> i32;
type Bind = unsafe extern "C" fn(u64, u64, u64) -> i32;
type End = unsafe extern "C" fn(u32, *mut Summary, u32) -> i32;

struct Observer {
    _handle: usize,
    begin: Begin,
    draw: DrawContext,
    bind: Bind,
    end: End,
    begin_clock: ClockBoundary,
    state: AtomicU8,
}

static OBSERVER: OnceLock<Observer> = OnceLock::new();

unsafe extern "C" {
    fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    fn dlerror() -> *const c_char;
}

fn loader_error() -> String {
    let error = unsafe { dlerror() };
    if error.is_null() {
        "dynamic loader returned no diagnostic".into()
    } else {
        unsafe { CStr::from_ptr(error) }
            .to_string_lossy()
            .into_owned()
    }
}

fn symbol(handle: *mut c_void, name: &CStr) -> Result<*mut c_void> {
    let pointer = unsafe { dlsym(handle, name.as_ptr()) };
    ensure!(
        !pointer.is_null(),
        "presentation observer symbol {:?}: {}",
        name,
        loader_error()
    );
    Ok(pointer)
}

pub(super) fn load(path: &Path) -> Result<()> {
    ensure!(
        RECORDER.get().is_some(),
        "presentation observer requires the private window recorder"
    );
    ensure!(
        OBSERVER.get().is_none(),
        "presentation observer already loaded"
    );
    ensure!(
        std::fs::symlink_metadata(path)?.is_file(),
        "presentation observer must be a regular file"
    );
    let canonical = path.canonicalize()?;
    let metadata = library_metadata(
        &canonical,
        include_bytes!("../../native/window_present_macos.m"),
        include_bytes!("../../native/window_present_macos.h"),
    )?;
    let begin_clock = sleep_clock()?;
    let begin_clock_record = sleep_clock_record("begin", begin_clock)?;
    let filename = CString::new(canonical.as_os_str().as_bytes())?;
    let handle = unsafe { dlopen(filename.as_ptr(), 2) };
    ensure!(
        !handle.is_null(),
        "cannot load presentation observer: {}",
        loader_error()
    );
    let observer = unsafe {
        Observer {
            _handle: handle as usize,
            begin: std::mem::transmute::<*mut c_void, Begin>(symbol(
                handle,
                c"sims_window_present_begin",
            )?),
            draw: std::mem::transmute::<*mut c_void, DrawContext>(symbol(
                handle,
                c"sims_window_present_draw_context",
            )?),
            bind: std::mem::transmute::<*mut c_void, Bind>(symbol(
                handle,
                c"sims_window_present_bind",
            )?),
            end: std::mem::transmute::<*mut c_void, End>(symbol(
                handle,
                c"sims_window_present_end",
            )?),
            begin_clock,
            state: AtomicU8::new(0),
        }
    };
    OBSERVER
        .set(observer)
        .map_err(|_| anyhow::anyhow!("presentation observer already loaded"))?;
    emit("presentation_library", metadata);
    emit("presentation_sleep_clock", begin_clock_record);
    Ok(())
}

unsafe extern "C" fn receive(pointer: *const Event, _: *mut c_void) {
    let Some(recorder) = RECORDER.get() else {
        return;
    };
    unsafe { receive_for_recorder(recorder, pointer) };
}

unsafe fn receive_for_recorder(recorder: &super::Recorder, pointer: *const Event) {
    if pointer.is_null() {
        recorder.dropped.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let event = unsafe { *pointer };
    if event.abi_version != 1 || event.struct_size as usize != std::mem::size_of::<Event>() {
        recorder.dropped.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let packet = Packet {
        kind: "presentation_event",
        elapsed_us: recorder.start.elapsed().as_micros(),
        dropped_records: recorder.dropped.load(Ordering::Relaxed),
        data: event,
    };
    if recorder
        .sender
        .try_send(Record::Presentation(packet))
        .is_err()
    {
        recorder.dropped.fetch_add(1, Ordering::Relaxed);
    }
}

pub(super) fn draw_context(frame_id: u64, state_id: u64) {
    let Some(observer) = OBSERVER.get() else {
        return;
    };
    if observer
        .state
        .compare_exchange(0, 1, Ordering::Relaxed, Ordering::Relaxed)
        .is_ok()
    {
        begin_recording(
            &observer.state,
            || unsafe {
                (observer.begin)(
                    1,
                    std::mem::size_of::<Event>() as u32,
                    receive,
                    std::ptr::null_mut(),
                )
            },
            emit,
        );
    }
    if observer.state.load(Ordering::Relaxed) == 1 {
        let status = unsafe { (observer.draw)(frame_id, state_id) };
        if status != 0 {
            emit(
                "presentation_error",
                json!({"operation":"draw_context","status":status,"frame_id":frame_id}),
            );
        }
    }
}

pub(super) fn bind(target: &wgpu::TextureView, frame_id: u64, state_id: u64) {
    let Some(observer) = OBSERVER.get() else {
        return;
    };
    if observer.state.load(Ordering::Relaxed) != 1 {
        return;
    }
    let identity = unsafe { target.texture().as_hal::<wgpu::hal::api::Metal>() }
        .map(|texture| std::ptr::from_ref(&**unsafe { texture.raw_handle() }) as usize as u64);
    if let Some(identity) = identity {
        let status = unsafe { (observer.bind)(identity, frame_id, state_id) };
        if status != 0 {
            emit(
                "presentation_error",
                json!({"operation":"bind","status":status,"frame_id":frame_id}),
            );
        }
    } else {
        emit(
            "presentation_error",
            json!({"operation":"bind","error":"Metal texture identity unavailable","frame_id":frame_id}),
        );
    }
}

pub(super) fn finish() {
    let Some(observer) = OBSERVER.get() else {
        return;
    };
    if observer
        .state
        .compare_exchange(1, 3, Ordering::Relaxed, Ordering::Relaxed)
        .is_err()
    {
        return;
    }
    finish_recording(
        observer.begin_clock,
        |summary| unsafe { (observer.end)(500, summary, std::mem::size_of::<Summary>() as u32) },
        sleep_clock,
        emit,
    );
}

#[cfg(test)]
#[path = "presentation/tests.rs"]
mod tests;
