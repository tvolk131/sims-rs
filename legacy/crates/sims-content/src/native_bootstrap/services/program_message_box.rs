//! Bounded, private Rust request/decision/teardown protocol.
//!
//! The integrating parent must declare this module. Nothing here enables a
//! USER32 import, creates UI, owns an HWND, pumps messages or qualifies Windows,
//! guest, original-caller, Application/startup/World or playable-alpha behavior.
//! A future qualified UI consumer must call `decide` only for a current explicit
//! UI action, and `acknowledge_teardown` only after genuinely completing its
//! managed-window teardown. This protocol authenticates the consumer's leases;
//! it cannot itself prove that a window existed or was physically removed.
//!
//! Policy: printable ASCII plus CR/LF (no tab), no code-page normalization;
//! NULL caption displays Error, NULL text refuses, empty non-NULL text is valid.
//! The seven encoded button selectors and existing default positions 1..3 are
//! supported. Only APPLMODAL=0, with no additional flags, is accepted. Default
//! selection never acts without an explicit Enter action. Window Close and
//! owner/programmatic loss are typed abandonment, never user IDCANCEL.
//!
//! There is one outstanding call and a never-recycled history of 1..128 issues.
//! Each string is bounded by a configured 1..16KiB (excluding its NUL). Payload
//! and history reservations precede publication. Arc/Mutex allocation follows
//! Rust's ordinary allocation-abort semantics; it is not a recoverable Win32
//! failure. No callback, background worker, blocking wait, native cleanup or
//! unsafe Send/Sync exists. Contention/poison refuses operational access.
//!
//! Drops set actual liveness flags and attempt nonblocking Rust-only cleanup.
//! Contended cleanup is deferred to the next live operation or final Arc drop;
//! poison is recovered only by destruction. Before presentation, loss can
//! discard data or yield a typed unavailable outcome. After presentation,
//! drops never acknowledge teardown, clear the outstanding slot or infer a
//! choice. A surviving Presentation can close without a choice and acknowledge
//! teardown even after its Presenter is gone. Losing that Presentation leaves
//! the slot unresolved for the remaining channel lifetime: no lease recovery,
//! observation adoption or timeout invents physical removal. All storage is
//! bounded and is released when its last actual Rust owner is dropped. An
//! abandoned caller cannot be resumed, even after a prior valid decision.

use crate::native_bootstrap::{
    executable::{Module, ModuleOrigin},
    memory::{Identity, Memory},
};
use std::{
    ffi::CStr,
    sync::{
        Arc, Mutex, MutexGuard, TryLockError,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, ThreadId},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    Disabled,
    PrivateAsciiCrLfV1,
}

/// Explicit private budgets, validated by `pair`; there is no default policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_history: usize,
    pub max_text_bytes: usize,
    pub max_caption_bytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    Disabled,
    InvalidLimits,
    NullText,
    UnsupportedEncoding,
    PayloadLimit,
    UnsupportedFlags,
    UnsupportedButtons,
    AbsentDefault,
    Reservation,
    WrongThread,
    UiNotBound,
    UiAlreadyBound,
    Busy,
    Poisoned,
    ForeignModule,
    ForeignMemory,
    ForeignLease,
    Outstanding,
    HistoryExhausted,
    PresenterGone,
    CallerGone,
    NotReady,
    Stale,
    ButtonNotOffered,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Ok,
    Cancel,
    Abort,
    Retry,
    Ignore,
    Yes,
    No,
    TryAgain,
    Continue,
}

impl Button {
    /// Documented button ID observation; this does not issue a completed call.
    pub fn id(self) -> i32 {
        match self {
            Self::Ok => 1, Self::Cancel => 2, Self::Abort => 3,
            Self::Retry => 4, Self::Ignore => 5, Self::Yes => 6,
            Self::No => 7, Self::TryAgain => 10, Self::Continue => 11,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Button(Button),
    Enter,
    Escape,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Abandonment {
    Programmatic,
    WindowClose,
    OwnerLost,
    ConsumerUnavailable,
}

/// Read-only result data, not an authentic completed receipt or a Win32 error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    UserChoice(Button),
    Abandoned(Abandonment),
    PresenterUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    Queued,
    Presented,
    AwaitingTeardown,
    Ready,
    Consumed,
}

/// Immutable original bytes and a separately owned presentation string.
pub struct Text {
    // Includes the original terminating NUL; None preserves NULL caption.
    original: Option<Vec<u8>>,
    display: String,
}

impl Text {
    pub fn original_bytes_with_nul(&self) -> Option<&[u8]> {
        self.original.as_deref()
    }
    pub fn display(&self) -> &str { &self.display }

    fn own(input: Option<&CStr>, limit: usize, null_caption: bool) -> Result<Self, Refusal> {
        let bytes = match input {
            Some(value) => value.to_bytes(),
            None if null_caption => b"Error",
            None => return Err(Refusal::NullText),
        };
        if bytes.len() > limit { return Err(Refusal::PayloadLimit); }
        if bytes.iter().any(|b| !matches!(*b, b' '..=b'~' | b'\r' | b'\n')) {
            return Err(Refusal::UnsupportedEncoding);
        }
        let original = if let Some(value) = input {
            let mut copy = Vec::new();
            copy.try_reserve_exact(value.to_bytes_with_nul().len())
                .map_err(|_| Refusal::Reservation)?;
            copy.extend_from_slice(value.to_bytes_with_nul());
            Some(copy)
        } else { None };
        let mut display = String::new();
        display.try_reserve_exact(bytes.len()).map_err(|_| Refusal::Reservation)?;
        // Every admitted byte is one ASCII scalar, including CR/LF.
        display.extend(bytes.iter().map(|b| char::from(*b)));
        Ok(Self { original, display })
    }
}

pub struct Payload {
    text: Text,
    caption: Text,
    flags: u32,
    buttons: &'static [Button],
    default: Button,
}

impl Payload {
    pub fn text(&self) -> &Text { &self.text }
    pub fn caption(&self) -> &Text { &self.caption }
    pub fn flags(&self) -> u32 { self.flags }
    pub fn buttons(&self) -> &[Button] { self.buttons }
    pub fn default_button(&self) -> Button { self.default }

    fn own(text: Option<&CStr>, caption: Option<&CStr>, flags: u32, limits: Limits)
        -> Result<Self, Refusal>
    {
        // These are encoded fields. E.g. source-level OKCANCEL|YESNO is 5,
        // which unambiguously selects RETRYCANCEL in the actual input word.
        if flags & !0x30f != 0 { return Err(Refusal::UnsupportedFlags); }
        let buttons: &'static [Button] = match flags & 0xf {
            0 => &[Button::Ok],
            1 => &[Button::Ok, Button::Cancel],
            2 => &[Button::Abort, Button::Retry, Button::Ignore],
            3 => &[Button::Yes, Button::No, Button::Cancel],
            4 => &[Button::Yes, Button::No],
            5 => &[Button::Retry, Button::Cancel],
            6 => &[Button::Cancel, Button::TryAgain, Button::Continue],
            _ => return Err(Refusal::UnsupportedButtons),
        };
        let default = *buttons.get(((flags >> 8) & 3) as usize)
            .ok_or(Refusal::AbsentDefault)?;
        let text = Text::own(text, limits.max_text_bytes, false)?;
        let caption = Text::own(caption, limits.max_caption_bytes, true)?;
        Ok(Self { text, caption, flags, buttons, default })
    }

    fn action(&self, action: Action) -> Result<Option<Button>, Refusal> {
        match action {
            Action::Enter => Ok(Some(self.default)),
            Action::Button(button) if self.buttons.contains(&button) => Ok(Some(button)),
            Action::Button(_) => Err(Refusal::ButtonNotOffered),
            Action::Escape if self.buttons.contains(&Button::Cancel) => Ok(Some(Button::Cancel)),
            Action::Escape if self.buttons == [Button::Ok] => Ok(Some(Button::Ok)),
            Action::Escape => Ok(None),
        }
    }
}

struct Issuer;
struct RequestIdentity { caller_alive: AtomicBool }
struct PresentationIdentity;
struct CloseIdentity;

struct Authority {
    issuer: Arc<Issuer>,
    request: Arc<RequestIdentity>,
    index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase { Queued, Presented, Closing, Ready, Retired }

struct Record {
    request: Arc<RequestIdentity>,
    phase: Phase,
    payload: Option<Arc<Payload>>,
    presentation: Option<Arc<PresentationIdentity>>,
    close: Option<Arc<CloseIdentity>>,
    proposed: Option<Outcome>,
    ready: Option<Outcome>,
}

impl Record {
    fn retire(&mut self) {
        self.phase = Phase::Retired;
        self.payload = None;
        self.presentation = None;
        self.close = None;
        self.proposed = None;
        self.ready = None;
    }
}

struct State {
    ui_thread: Option<ThreadId>,
    records: Vec<Record>,
    active: Option<usize>,
}

struct Shared {
    issuer: Arc<Issuer>,
    origin: ModuleOrigin,
    memory: Identity,
    caller_thread: ThreadId,
    caller_alive: AtomicBool,
    presenter_alive: AtomicBool,
    limits: Limits,
    state: Mutex<State>,
}

impl Shared {
    fn lock(&self) -> Result<MutexGuard<'_, State>, Refusal> {
        match self.state.try_lock() {
            Ok(state) => Ok(state),
            Err(TryLockError::WouldBlock) => Err(Refusal::Busy),
            Err(TryLockError::Poisoned(_)) => Err(Refusal::Poisoned),
        }
    }

    fn caller_thread(&self) -> Result<(), Refusal> {
        if self.caller_thread != thread::current().id() { return Err(Refusal::WrongThread); }
        Ok(())
    }

    fn ui_thread(&self, state: &State) -> Result<(), Refusal> {
        match state.ui_thread {
            None => Err(Refusal::UiNotBound),
            Some(id) if id == thread::current().id() => Ok(()),
            Some(_) => Err(Refusal::WrongThread),
        }
    }

    fn record<'a>(&self, state: &'a mut State, key: &Authority) -> Result<&'a mut Record, Refusal> {
        if !Arc::ptr_eq(&self.issuer, &key.issuer) { return Err(Refusal::ForeignLease); }
        let record = state.records.get_mut(key.index).ok_or(Refusal::ForeignLease)?;
        if !Arc::ptr_eq(&record.request, &key.request) { return Err(Refusal::ForeignLease); }
        Ok(record)
    }

    // Never infer teardown from liveness. A presented/closing entry remains
    // active even if every endpoint but this Shared allocation has disappeared.
    fn reconcile(&self, state: &mut State) {
        let Some(index) = state.active else { return };
        let record = &mut state.records[index];
        let caller = self.caller_alive.load(Ordering::Acquire)
            && record.request.caller_alive.load(Ordering::Acquire);
        if matches!(record.phase, Phase::Queued | Phase::Ready) && !caller {
            record.retire();
            state.active = None;
        } else if record.phase == Phase::Queued
            && !self.presenter_alive.load(Ordering::Acquire)
        {
            record.phase = Phase::Ready;
            record.ready = Some(Outcome::PresenterUnavailable);
            record.payload = None;
        }
    }

    fn destruction_cleanup(&self) {
        match self.state.try_lock() {
            Ok(mut state) => self.reconcile(&mut state),
            // Recovery is solely for Rust resource destruction, never an
            // operational reply, user decision or physical teardown claim.
            Err(TryLockError::Poisoned(poison)) => self.reconcile(&mut poison.into_inner()),
            Err(TryLockError::WouldBlock) => (),
        }
    }
}

/// Non-Clone caller, bound to the construction thread and actual origins.
pub struct Caller { shared: Arc<Shared> }
/// Non-Clone UI endpoint. Move it to its UI thread, then bind exactly once.
pub struct Presenter { shared: Arc<Shared> }

pub fn pair(policy: Policy, limits: Limits, module: &Module, memory: &Memory)
    -> Result<(Caller, Presenter), Refusal>
{
    if policy == Policy::Disabled { return Err(Refusal::Disabled); }
    if !(1..=128).contains(&limits.max_history)
        || !(1..=16384).contains(&limits.max_text_bytes)
        || !(1..=16384).contains(&limits.max_caption_bytes)
    { return Err(Refusal::InvalidLimits); }
    let mut records = Vec::new();
    records.try_reserve_exact(limits.max_history).map_err(|_| Refusal::Reservation)?;
    let shared = Arc::new(Shared {
        issuer: Arc::new(Issuer), origin: module.origin(), memory: memory.identity(),
        caller_thread: thread::current().id(), caller_alive: AtomicBool::new(true),
        presenter_alive: AtomicBool::new(true), limits,
        state: Mutex::new(State { ui_thread: None, records, active: None }),
    });
    Ok((Caller { shared: Arc::clone(&shared) }, Presenter { shared }))
}

/// Retains actual input/heap borrows and exclusive caller access until dropped,
/// including after finish. CStr inputs are copied and need not remain borrowed.
///
/// Compile-time controls for the actual pending-call lifetimes.
/// Module cannot be consumed while the call is retained:
/// ```compile_fail,E0505
/// use sims_content::native_bootstrap::{executable::Module, Memory};
/// use sims_content::native_bootstrap::services::program_message_box::Caller;
/// use std::ffi::CStr;
/// fn retained(mut caller: Caller, module: Module, memory: &Memory, text: &CStr) {
///     let pending = caller.request(&module, memory, Some(text), None, 0).unwrap();
///     drop(module);
///     drop(pending);
/// }
/// ```
/// The actual heap cannot be mutated while retained:
/// ```compile_fail,E0502
/// use sims_content::native_bootstrap::{executable::Module, Memory};
/// use sims_content::native_bootstrap::services::program_message_box::Caller;
/// use std::ffi::CStr;
/// fn retained(mut caller: Caller, module: &Module, mut memory: Memory, text: &CStr) {
///     let pending = caller.request(module, &memory, Some(text), None, 0).unwrap();
///     memory.allocate_owner(16).unwrap();
///     drop(pending);
/// }
/// ```
/// Exclusive caller use cannot be duplicated:
/// ```compile_fail,E0499
/// use sims_content::native_bootstrap::{executable::Module, Memory};
/// use sims_content::native_bootstrap::services::program_message_box::Caller;
/// use std::ffi::CStr;
/// fn exclusive(mut caller: Caller, module: &Module, memory: &Memory, text: &CStr) {
///     let first = caller.request(module, memory, Some(text), None, 0).unwrap();
///     let second = caller.request(module, memory, Some(text), None, 0).unwrap();
///     drop((first, second));
/// }
/// ```
/// Both actual inputs must outlive the pending continuation:
/// ```compile_fail,E0597
/// use sims_content::native_bootstrap::{executable::read, Memory};
/// use sims_content::native_bootstrap::services::program_message_box::Caller;
/// use std::{ffi::CStr, path::Path};
/// fn outlive(mut caller: Caller, text: &CStr) {
///     let pending = {
///         let module = read(Path::new("authored.exe")).unwrap();
///         let memory = Memory::new(8192).unwrap();
///         caller.request(&module, &memory, Some(text), None, 0).unwrap()
///     };
///     drop(pending);
/// }
/// ```
pub struct PendingCall<'c, 'i> {
    caller: &'c mut Caller,
    module: &'i Module,
    memory: &'i Memory,
    key: Authority,
    payload: Arc<Payload>,
    finished: bool,
}

pub struct Presentation {
    shared: Arc<Shared>,
    key: Authority,
    identity: Arc<PresentationIdentity>,
    payload: Arc<Payload>,
}

/// Opaque proposed close, without a public result or adoption constructor.
pub struct CloseIntent {
    shared: Arc<Shared>,
    key: Authority,
    presentation: Arc<PresentationIdentity>,
    identity: Arc<CloseIdentity>,
}

/// Only `PendingCall::finish` issues this receipt. No Clone/Deserialize/adoption.
pub struct Completion {
    key: Authority,
    outcome: Outcome,
}

impl Completion {
    pub fn outcome(&self) -> Outcome { self.outcome }
    pub fn number(&self) -> usize { self.key.index + 1 }
}

impl Caller {
    pub fn request<'c, 'i>(&'c mut self, module: &'i Module, memory: &'i Memory,
        text: Option<&CStr>, caption: Option<&CStr>, flags: u32)
        -> Result<PendingCall<'c, 'i>, Refusal>
    {
        self.shared.caller_thread()?;
        if self.shared.origin != module.origin() { return Err(Refusal::ForeignModule); }
        if !memory.has_identity(&self.shared.memory) { return Err(Refusal::ForeignMemory); }
        let (key, payload) = {
            let mut state = self.shared.lock()?;
            self.shared.reconcile(&mut state);
            if !self.shared.presenter_alive.load(Ordering::Acquire) {
                return Err(Refusal::PresenterGone);
            }
            if state.active.is_some() { return Err(Refusal::Outstanding); }
            if state.records.len() >= self.shared.limits.max_history {
                return Err(Refusal::HistoryExhausted);
            }
            let payload = Arc::new(Payload::own(text, caption, flags, self.shared.limits)?);
            let request = Arc::new(RequestIdentity { caller_alive: AtomicBool::new(true) });
            let index = state.records.len();
            let key = Authority { issuer: Arc::clone(&self.shared.issuer),
                request: Arc::clone(&request), index };
            // History storage was reserved by pair. No fallible operation
            // follows publication and issued indices are never recycled.
            state.records.push(Record { request, phase: Phase::Queued,
                payload: Some(Arc::clone(&payload)), presentation: None,
                close: None, proposed: None, ready: None });
            state.active = Some(index);
            (key, payload)
        };
        Ok(PendingCall { caller: self, module, memory, key, payload, finished: false })
    }

    /// Identity comparison only; observation cannot resume or adopt a call.
    pub fn issued(&self, completion: &Completion) -> bool {
        Arc::ptr_eq(&self.shared.issuer, &completion.key.issuer)
    }
}

impl PendingCall<'_, '_> {
    pub fn number(&self) -> usize { self.key.index + 1 }
    pub fn payload(&self) -> &Payload { &self.payload }
    pub fn module(&self) -> &Module { self.module }
    pub fn memory(&self) -> &Memory { self.memory }

    pub fn poll(&self) -> Result<Progress, Refusal> {
        let shared = &self.caller.shared;
        shared.caller_thread()?;
        let mut state = shared.lock()?;
        shared.reconcile(&mut state);
        let record = shared.record(&mut state, &self.key)?;
        Ok(match record.phase {
            Phase::Queued => Progress::Queued, Phase::Presented => Progress::Presented,
            Phase::Closing => Progress::AwaitingTeardown, Phase::Ready => Progress::Ready,
            Phase::Retired => Progress::Consumed,
        })
    }

    /// Before presentation only: a typed abandonment, never a button result.
    pub fn cancel_before_presentation(&mut self) -> Result<(), Refusal> {
        let shared = &self.caller.shared;
        shared.caller_thread()?;
        let mut state = shared.lock()?;
        let record = shared.record(&mut state, &self.key)?;
        if record.phase != Phase::Queued { return Err(Refusal::Stale); }
        record.phase = Phase::Ready;
        record.ready = Some(Outcome::Abandoned(Abandonment::Programmatic));
        record.payload = None;
        Ok(())
    }

    pub fn finish(&mut self) -> Result<Completion, Refusal> {
        let shared = &self.caller.shared;
        shared.caller_thread()?;
        if self.finished { return Err(Refusal::Stale); }
        let mut state = shared.lock()?;
        shared.reconcile(&mut state);
        let record = shared.record(&mut state, &self.key)?;
        if record.phase != Phase::Ready { return Err(Refusal::NotReady); }
        let outcome = record.ready.ok_or(Refusal::NotReady)?;
        let key = Authority { issuer: Arc::clone(&self.key.issuer),
            request: Arc::clone(&self.key.request), index: self.key.index };
        record.retire();
        state.active = None;
        self.finished = true;
        Ok(Completion { key, outcome })
    }
}

impl Presenter {
    pub fn bind_ui_thread(&mut self) -> Result<(), Refusal> {
        let mut state = self.shared.lock()?;
        if state.ui_thread.is_some() { return Err(Refusal::UiAlreadyBound); }
        state.ui_thread = Some(thread::current().id());
        Ok(())
    }

    /// Publication to the actual UI consumer, not evidence of physical display.
    pub fn claim(&mut self) -> Result<Option<Presentation>, Refusal> {
        let mut state = self.shared.lock()?;
        self.shared.ui_thread(&state)?;
        self.shared.reconcile(&mut state);
        let Some(index) = state.active else { return Ok(None) };
        let record = &mut state.records[index];
        if record.phase != Phase::Queued { return Err(Refusal::Stale); }
        let payload = Arc::clone(record.payload.as_ref().ok_or(Refusal::Stale)?);
        let identity = Arc::new(PresentationIdentity);
        let key = Authority { issuer: Arc::clone(&self.shared.issuer),
            request: Arc::clone(&record.request), index };
        record.presentation = Some(Arc::clone(&identity));
        record.phase = Phase::Presented;
        Ok(Some(Presentation { shared: Arc::clone(&self.shared), key, identity, payload }))
    }

    /// Caller of this method must be the qualified current UI event consumer.
    pub fn decide(&mut self, lease: &Presentation, action: Action)
        -> Result<Option<CloseIntent>, Refusal>
    {
        if !Arc::ptr_eq(&self.shared, &lease.shared) { return Err(Refusal::ForeignLease); }
        lease.close(Some(action), None)
    }
}

impl Presentation {
    pub fn number(&self) -> usize { self.key.index + 1 }
    pub fn payload(&self) -> &Payload { &self.payload }

    fn current<'a>(&self, state: &'a mut State) -> Result<&'a mut Record, Refusal> {
        let record = self.shared.record(state, &self.key)?;
        if !record.presentation.as_ref().is_some_and(|id| Arc::ptr_eq(id, &self.identity)) {
            return Err(Refusal::ForeignLease);
        }
        Ok(record)
    }

    fn close(&self, action: Option<Action>, abandonment: Option<Abandonment>)
        -> Result<Option<CloseIntent>, Refusal>
    {
        let mut state = self.shared.lock()?;
        self.shared.ui_thread(&state)?;
        let record = self.current(&mut state)?;
        if record.phase != Phase::Presented { return Err(Refusal::Stale); }
        let outcome = if let Some(action) = action {
            if !self.shared.presenter_alive.load(Ordering::Acquire) { return Err(Refusal::PresenterGone); }
            if !self.shared.caller_alive.load(Ordering::Acquire)
                || !record.request.caller_alive.load(Ordering::Acquire)
            { return Err(Refusal::CallerGone); }
            let Some(button) = self.payload.action(action)? else { return Ok(None) };
            Outcome::UserChoice(button)
        } else {
            Outcome::Abandoned(abandonment.ok_or(Refusal::Stale)?)
        };
        let identity = Arc::new(CloseIdentity);
        let key = Authority { issuer: Arc::clone(&self.key.issuer),
            request: Arc::clone(&self.key.request), index: self.key.index };
        let intent = CloseIntent { shared: Arc::clone(&self.shared), key,
            presentation: Arc::clone(&self.identity), identity: Arc::clone(&identity) };
        record.close = Some(identity);
        record.proposed = Some(outcome);
        record.phase = Phase::Closing;
        Ok(Some(intent))
    }

    /// A genuine teardown path without a user selection. Also usable by the
    /// retained UI consumer after caller/Presenter loss. This is not an ack.
    pub fn close_without_choice(&mut self, reason: Abandonment) -> Result<CloseIntent, Refusal> {
        self.close(None, Some(reason))?.ok_or(Refusal::Stale)
    }

    /// Caller must actually complete teardown first. Mere lease/endpoint Drop
    /// is never such evidence. Foreign, stale, duplicate and reordered intents
    /// refuse before mutation. Ack after caller loss only retires Rust state.
    pub fn acknowledge_teardown(&mut self, intent: &CloseIntent) -> Result<(), Refusal> {
        if !Arc::ptr_eq(&self.shared, &intent.shared)
            || !Arc::ptr_eq(&self.key.issuer, &intent.key.issuer)
            || !Arc::ptr_eq(&self.key.request, &intent.key.request)
            || self.key.index != intent.key.index
            || !Arc::ptr_eq(&self.identity, &intent.presentation)
        { return Err(Refusal::ForeignLease); }
        let mut state = self.shared.lock()?;
        self.shared.ui_thread(&state)?;
        let record = self.current(&mut state)?;
        if record.phase != Phase::Closing { return Err(Refusal::Stale); }
        if !record.close.as_ref().is_some_and(|id| Arc::ptr_eq(id, &intent.identity)) {
            return Err(Refusal::ForeignLease);
        }
        let outcome = record.proposed.ok_or(Refusal::Stale)?;
        if self.shared.caller_alive.load(Ordering::Acquire)
            && record.request.caller_alive.load(Ordering::Acquire)
        {
            record.phase = Phase::Ready;
            record.ready = Some(outcome);
            record.proposed = None;
            record.payload = None;
        } else {
            record.retire();
            state.active = None;
        }
        Ok(())
    }
}

impl Drop for PendingCall<'_, '_> {
    fn drop(&mut self) {
        self.key.request.caller_alive.store(false, Ordering::Release);
        self.caller.shared.destruction_cleanup();
    }
}

impl Drop for Caller {
    fn drop(&mut self) {
        self.shared.caller_alive.store(false, Ordering::Release);
        self.shared.destruction_cleanup();
    }
}

impl Drop for Presenter {
    fn drop(&mut self) {
        self.shared.presenter_alive.store(false, Ordering::Release);
        self.shared.destruction_cleanup();
    }
}

#[cfg(test)]
mod tests;
