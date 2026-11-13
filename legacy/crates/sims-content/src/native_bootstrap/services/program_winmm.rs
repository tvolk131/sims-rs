//! Explicit host kernel-boot elapsed evidence for a Rust-port WINMM profile.
//!
//! The parent must declare this module and separately authenticate/wire the
//! WINMM registry, service, capability, import and caller. `Clock::new` requires
//! an explicit policy; construction performs no native I/O. There is no default,
//! timestamp adoption, production callback, reset, rebase or fallback clock.
//! Only a successful `sample` issues an owner-bound `Sample`. Reports and numeric
//! readings are historical evidence, never authority that can be adopted.
//!
//! On Darwin64 an attempt reads sysctl KERN_BOOTTIME, gettimeofday, then sysctl
//! KERN_BOOTTIME again, with no retry. These are three independent snapshots,
//! not an atomic observation. Equal boot brackets cannot exclude a calendar
//! step and restoration (ABA): a false forward sample can pass, and subsequent
//! nonregression checks cannot detect every such case. Kernel calendar
//! initialization/adjustment and sleep behavior do not establish Windows early
//! boot timing, multimedia resolution or strict Windows parity. No
//! timeBeginPeriod/timeEndPeriod, native caller/ABI/CPU/CRT/constructor
//! qualification or installed-kernel source matching is implemented or claimed.
//! There is deliberately no arbitrary-origin RAW/Mach clock fallback.
//!
//! Clock retains at most the last accepted sample, latest attempt report and
//! first failure report. Callers may retain immutable historical copies. Failed
//! attempts consume an available sequence but never change the accepted sample.
//! A refusal without I/O has no native sequence. Sequence exhaustion refuses
//! before I/O and never wraps. All source controls, including ignored opt-in
//! real-clock controls, are authored UNRUN; parent owns actual qualification.

use std::{ffi::c_long, sync::Arc};

/// No Default: enabling this host profile is an explicit parent decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    Disabled,
    DarwinKernelBootElapsed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    BootBefore,
    Calendar,
    BootAfter,
}

impl Stage {
    fn index(self) -> usize {
        match self {
            Self::BootBefore => 0,
            Self::Calendar => 1,
            Self::BootAfter => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Seconds,
    Microseconds,
}

/// Typed refusal; no error is represented by an elapsed value of zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    Disabled,
    UnsupportedHost,
    SequenceExhausted,
    NativeCallFailed { stage: Stage, return_code: i32, errno: i32 },
    UnexpectedLength { stage: Stage, expected: usize, returned: Option<usize> },
    InvalidScalar { stage: Stage, field: Field, value: i64 },
    BootChanged,
    CalendarBeforeBoot,
    ElapsedRegression { previous_microseconds: u128, observed_microseconds: u128 },
    ForeignSample,
}

/// The provenance is descriptive, not a transferable issuing capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    NoNativeIo,
    DarwinSdkRealApis,
    #[cfg(test)]
    SyntheticControl,
}

/// Declared output fields, including initialized/partially overwritten fields
/// after a failed or short call. Such fields are NOT accepted timing data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimevalScalars {
    seconds: i64,
    microseconds: i32,
}

impl TimevalScalars {
    pub fn seconds(&self) -> i64 { self.seconds }
    pub fn microseconds(&self) -> i32 { self.microseconds }

    fn checked_microseconds(self, stage: Stage) -> Result<u128, Stop> {
        if self.seconds < 0 {
            return Err(Stop::InvalidScalar {
                stage, field: Field::Seconds, value: self.seconds,
            });
        }
        if !(0..1_000_000).contains(&self.microseconds) {
            return Err(Stop::InvalidScalar {
                stage, field: Field::Microseconds, value: i64::from(self.microseconds),
            });
        }
        // Both scalars have been checked before widening. Even i64::MAX seconds
        // plus 999999 usec is below 2^84, so this u128 arithmetic cannot overflow.
        Ok((self.seconds as u128) * 1_000_000 + self.microseconds as u128)
    }
}

/// Scalar receipt of one returned native call, never an unpacked byte buffer.
/// Errno is read from the actual thread-local channel before and immediately
/// after the call, without clearing it. A successful call may leave stale errno;
/// only the return code determines whether errno describes a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeCall {
    return_code: i32,
    errno_before: i32,
    errno_after: i32,
    buffer_capacity: Option<usize>,
    returned_length: Option<usize>,
    output: TimevalScalars,
}

impl NativeCall {
    pub fn return_code(&self) -> i32 { self.return_code }
    pub fn errno_before(&self) -> i32 { self.errno_before }
    pub fn errno_after(&self) -> i32 { self.errno_after }
    /// None for gettimeofday, whose ABI has no output-length parameter.
    pub fn buffer_capacity(&self) -> Option<usize> { self.buffer_capacity }
    /// None for gettimeofday; boot sysctl records the returned size_t verbatim.
    pub fn returned_length(&self) -> Option<usize> { self.returned_length }
    /// Declared fields only; rc/length/scalar checks must succeed before use.
    pub fn declared_output(&self) -> TimevalScalars { self.output }

    fn checked_output(self, stage: Stage) -> Result<(TimevalScalars, u128), Stop> {
        if self.return_code != 0 {
            return Err(Stop::NativeCallFailed {
                stage, return_code: self.return_code, errno: self.errno_after,
            });
        }
        if stage != Stage::Calendar {
            let expected = std::mem::size_of::<DarwinTimeval>();
            if self.returned_length != Some(expected) {
                return Err(Stop::UnexpectedLength {
                    stage, expected, returned: self.returned_length,
                });
            }
        }
        Ok((self.output, self.output.checked_microseconds(stage)?))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeOutcome {
    NotEntered,
    /// Set immediately before entering I/O; normal completed reports replace it
    /// with Returned. There is no catch/retry or report guarantee after a panic.
    Entered,
    Returned(NativeCall),
}

/// Accepted value evidence. This copy contains no issuing identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reading {
    elapsed_microseconds: u128,
}

impl Reading {
    pub fn elapsed_microseconds(&self) -> u128 { self.elapsed_microseconds }
    pub fn elapsed_milliseconds(&self) -> u128 { self.elapsed_microseconds / 1_000 }
    /// Floor microseconds to milliseconds, then wrap to the low DWORD bits.
    pub fn value(&self) -> u32 {
        (self.elapsed_milliseconds() & u128::from(u32::MAX)) as u32
    }
}

/// Immutable receipt of a completed attempt. BootBefore/BootAfter always request
/// exactly sysctl([CTL_KERN=1, KERN_BOOTTIME=21], namelen=2, newp=NULL, newlen=0)
/// into one naturally aligned DarwinTimeval. Calendar requests gettimeofday with
/// timezone=NULL. NotEntered distinguishes all skipped calls after a refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Report {
    policy: Policy,
    sequence: Option<u64>,
    provenance: Provenance,
    calls: [NativeOutcome; 3],
    result: Result<Reading, Stop>,
}

impl Report {
    pub fn policy(&self) -> Policy { self.policy }
    pub fn sequence(&self) -> Option<u64> { self.sequence }
    pub fn provenance(&self) -> Provenance { self.provenance }
    pub fn call(&self, stage: Stage) -> NativeOutcome { self.calls[stage.index()] }
    pub fn result(&self) -> Result<Reading, Stop> { self.result }
}

struct Issuer;

/// Authentic historical issuance. Private fields, no deserialization or public
/// construction/adoption. Cloning preserves this issuance and its immutable
/// receipt; matching numeric values from another Clock confer no ownership.
#[derive(Clone)]
pub struct Sample {
    issuer: Arc<Issuer>,
    sequence: u64,
    reading: Reading,
    report: Report,
}

impl Sample {
    pub fn value(&self) -> u32 { self.reading.value() }
    pub fn elapsed_microseconds(&self) -> u128 { self.reading.elapsed_microseconds() }
    pub fn elapsed_milliseconds(&self) -> u128 { self.reading.elapsed_milliseconds() }
    pub fn sequence(&self) -> u64 { self.sequence }
    pub fn report(&self) -> &Report { &self.report }
}

impl std::fmt::Debug for Sample {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sample")
            .field("sequence", &self.sequence)
            .field("reading", &self.reading)
            .field("report", &self.report)
            .finish_non_exhaustive()
    }
}

/// Non-Clone owner; no public policy mutation, reset or serialized authority.
pub struct Clock {
    policy: Policy,
    issuer: Arc<Issuer>,
    last_sequence: u64,
    last_sample: Option<Sample>,
    latest_report: Option<Report>,
    first_failure_report: Option<Report>,
    #[cfg(test)]
    scripted_io: Option<tests::ScriptedIo>,
}

impl Clock {
    pub fn new(policy: Policy) -> Self {
        Self {
            policy,
            issuer: Arc::new(Issuer),
            last_sequence: 0,
            last_sample: None,
            latest_report: None,
            first_failure_report: None,
            #[cfg(test)]
            scripted_io: None,
        }
    }

    pub fn policy(&self) -> Policy { self.policy }
    pub fn last_sample(&self) -> Option<&Sample> { self.last_sample.as_ref() }
    pub fn latest_report(&self) -> Option<&Report> { self.latest_report.as_ref() }
    pub fn first_failure_report(&self) -> Option<&Report> {
        self.first_failure_report.as_ref()
    }

    /// Validates issuer identity, including historical samples. This does not
    /// adopt evidence, refresh it, assert freshness or perform native I/O.
    pub fn validate(&self, sample: &Sample) -> Result<(), Stop> {
        if Arc::ptr_eq(&self.issuer, &sample.issuer) {
            Ok(())
        } else {
            Err(Stop::ForeignSample)
        }
    }

    /// Disabled refuses before I/O; the enabled profile requires Darwin64.
    /// Stops at the first failure and does not retry or publish a partial sample.
    pub fn sample(&mut self) -> Result<Sample, Stop> {
        self.sample_on_host(cfg!(all(target_os = "macos", target_pointer_width = "64")))
    }

    fn sample_on_host(&mut self, supported: bool) -> Result<Sample, Stop> {
        let mut attempt = Attempt {
            policy: self.policy,
            sequence: None,
            provenance: Provenance::NoNativeIo,
            calls: [NativeOutcome::NotEntered; 3],
        };
        let result = self.observe(&mut attempt, supported);
        let report = Report {
            policy: attempt.policy,
            sequence: attempt.sequence,
            provenance: attempt.provenance,
            calls: attempt.calls,
            result: result.map(|(_, reading)| reading),
        };
        self.latest_report = Some(report);
        match result {
            Ok((sequence, reading)) => {
                let sample = Sample {
                    issuer: Arc::clone(&self.issuer), sequence, reading, report,
                };
                self.last_sample = Some(sample.clone());
                Ok(sample)
            }
            Err(stop) => {
                if self.first_failure_report.is_none() {
                    self.first_failure_report = Some(report);
                }
                Err(stop)
            }
        }
    }

    fn observe(&mut self, attempt: &mut Attempt, supported: bool) -> Result<(u64, Reading), Stop> {
        if self.policy == Policy::Disabled {
            return Err(Stop::Disabled);
        }
        if !supported {
            return Err(Stop::UnsupportedHost);
        }
        let sequence = self.last_sequence.checked_add(1).ok_or(Stop::SequenceExhausted)?;
        self.last_sequence = sequence;
        attempt.sequence = Some(sequence);
        attempt.provenance = Provenance::DarwinSdkRealApis;
        #[cfg(test)]
        if self.scripted_io.is_some() {
            attempt.provenance = Provenance::SyntheticControl;
        }
        let (boot_before, boot_us) = self.read_stage(attempt, Stage::BootBefore)?;
        let (_, calendar_us) = self.read_stage(attempt, Stage::Calendar)?;
        let (boot_after, _) = self.read_stage(attempt, Stage::BootAfter)?;
        if boot_before != boot_after {
            return Err(Stop::BootChanged);
        }
        let elapsed_microseconds = calendar_us.checked_sub(boot_us).ok_or(Stop::CalendarBeforeBoot)?;
        if let Some(previous) = &self.last_sample {
            if elapsed_microseconds < previous.elapsed_microseconds() {
                return Err(Stop::ElapsedRegression {
                    previous_microseconds: previous.elapsed_microseconds(),
                    observed_microseconds: elapsed_microseconds,
                });
            }
        }
        Ok((sequence, Reading { elapsed_microseconds }))
    }

    fn read_stage(&mut self, attempt: &mut Attempt, stage: Stage) -> Result<(TimevalScalars, u128), Stop> {
        attempt.calls[stage.index()] = NativeOutcome::Entered;
        let call = self.native_call(stage);
        attempt.calls[stage.index()] = NativeOutcome::Returned(call);
        call.checked_output(stage)
    }

    fn native_call(&mut self, stage: Stage) -> NativeCall {
        #[cfg(test)]
        if let Some(script) = self.scripted_io.as_mut() {
            return script.read(stage);
        }
        #[cfg(all(target_os = "macos", target_pointer_width = "64"))]
        { native::read(stage) }
        #[cfg(not(all(target_os = "macos", target_pointer_width = "64")))]
        {
            let _ = stage;
            unreachable!("unsupported hosts refuse before native I/O")
        }
    }
}

struct Attempt {
    policy: Policy,
    sequence: Option<u64>,
    provenance: Provenance,
    calls: [NativeOutcome; 3],
}

// Exact admitted Darwin64 SDK ABI: long seconds, signed int32 microseconds,
// natural repr(C) alignment/padding; size_t is usize. Never pack/pun this type.
#[repr(C)]
struct DarwinTimeval {
    seconds: c_long,
    microseconds: i32,
}

#[cfg(all(target_os = "macos", target_pointer_width = "64"))]
mod native {
    use super::{DarwinTimeval, NativeCall, Stage, TimevalScalars};
    use std::{ffi::{c_int, c_uint, c_void}, mem::size_of, ptr};

    // Compared source-only with the permitted MacOSX26.5.sdk sys/sysctl.h,
    // sys/time.h, sys/_types/_timeval.h, scalar declarations and sys/errno.h.
    // Parent must independently qualify actual target layout/linkage before use.
    unsafe extern "C" {
        fn sysctl(name: *mut c_int, namelen: c_uint, oldp: *mut c_void,
            oldlenp: *mut usize, newp: *mut c_void, newlen: usize) -> c_int;
        fn gettimeofday(time: *mut DarwinTimeval, timezone: *mut c_void) -> c_int;
        fn __error() -> *mut c_int;
    }

    pub(super) fn read(stage: Stage) -> NativeCall {
        // Initialized scalar fields remain readable even after partial/error
        // writes, but are never used as time without rc/length/scalar checks.
        // Padding is neither read nor exposed as bytes.
        let mut output = DarwinTimeval { seconds: 0, microseconds: 0 };
        let mut length = size_of::<DarwinTimeval>();
        let mut mib: [c_int; 2] = [1, 21];
        let return_code;
        let errno_before;
        let errno_after;
        // SAFETY: __error supplies this thread's errno slot. All output pointers
        // refer to live, naturally aligned ABI-typed storage of sufficient size.
        // The boot query is read-only with exactly two MIB elements; the calendar
        // call receives NULL timezone. No callbacks or other calls intervene
        // between the actual syscall and its thread-local errno capture.
        unsafe {
            let errno = __error();
            errno_before = *errno;
            return_code = match stage {
                Stage::BootBefore | Stage::BootAfter => sysctl(
                    mib.as_mut_ptr(), 2, (&mut output as *mut DarwinTimeval).cast(),
                    &mut length, ptr::null_mut(), 0,
                ),
                Stage::Calendar => gettimeofday(&mut output, ptr::null_mut()),
            };
            errno_after = *errno;
        }
        let is_boot = stage != Stage::Calendar;
        NativeCall {
            return_code, errno_before, errno_after,
            buffer_capacity: is_boot.then_some(size_of::<DarwinTimeval>()),
            returned_length: is_boot.then_some(length),
            output: TimevalScalars {
                seconds: output.seconds, microseconds: output.microseconds,
            },
        }
    }
}

#[cfg(test)]
mod tests;
